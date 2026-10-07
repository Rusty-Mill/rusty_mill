#include "RustyBulletCapturePlugin.h"

#include "bakkesmod/wrappers/arraywrapper.h"
#include "bakkesmod/wrappers/GameObject/CarComponent/BoostWrapper.h"

#include <sstream>
#include <system_error>

// `PLUGINTYPE` (bakkesmodsdk.h) has no flag for a regular local/offline
// match at all -- only FREEPLAY, CUSTOM_TRAINING, SPECTATOR, BOTAI, REPLAY,
// THREADED, THREADEDUNLOAD exist. `PLUGINTYPE_FREEPLAY` is this plugin's
// primary use case (see README); it doesn't gate loading during a normal
// match, since there's no bit for one to begin with.
BAKKESMOD_PLUGIN(RustyBulletCapturePlugin, "Rusty Bullet capture", "1.4", PLUGINTYPE_FREEPLAY)

namespace
{
// Field names/nesting below must match crates/rb_capture_ingest/src/wire.rs
// exactly (see ADR-0005) -- this is the wire format itself, not incidental
// formatting.

// A freeplay reset (Backspace) destroys and respawns actors. A destroyed
// actor keeps its memory until garbage collection, but its `bDeleteMe`
// flag is set; reading physics state through it crashed the game
// (`Launch.log` call stack inside this plugin, 2026-10-03). Only live
// actors are read.
bool isLive(ActorWrapper actor)
{
    return !actor.IsNull() && actor.GetbDeleteMe() == 0;
}

// The first live ball, or a null wrapper when there is none (mid-reset).
BallWrapper liveBall(ServerWrapper server)
{
    ArrayWrapper<BallWrapper> balls = server.GetGameBalls();
    if (!balls.IsNull())
    {
        for (int i = 0; i < balls.Count(); ++i)
        {
            BallWrapper ball = balls.Get(i);
            if (isLive(ball))
            {
                return ball;
            }
        }
    }
    return BallWrapper(0);
}

std::string vectorJson(Vector v)
{
    std::ostringstream out;
    out << "{\"x\":" << v.X << ",\"y\":" << v.Y << ",\"z\":" << v.Z << "}";
    return out.str();
}

std::string quatJson(Quat q)
{
    std::ostringstream out;
    out << "{\"x\":" << q.X << ",\"y\":" << q.Y << ",\"z\":" << q.Z << ",\"w\":" << q.W << "}";
    return out.str();
}

std::string rbActorJson(RBActorWrapper actor)
{
    RBState state = actor.GetRBState();
    std::ostringstream out;
    out << "{\"position\":" << vectorJson(state.Location) << ",\"rotation\":" << quatJson(state.Quaternion)
        << ",\"velocity\":" << vectorJson(state.LinearVelocity)
        << ",\"angular_velocity\":" << vectorJson(state.AngularVelocity) << "}";
    return out.str();
}

// `rb_domain::ControllerInput::{throttle,steer}` are plain `f32`, and
// `pitch`/`yaw`/`roll` are always `Some` for a capture (see ADR-0005 and
// `CarState.input`'s doc comment) -- unlike a replay, a live capture always
// has the analog stick values, so no field here is ever omitted.
std::string inputJson(ControllerInput input)
{
    std::ostringstream out;
    out << "{\"throttle\":" << input.Throttle << ",\"steer\":" << input.Steer << ",\"pitch\":" << input.Pitch
        << ",\"yaw\":" << input.Yaw << ",\"roll\":" << input.Roll << ",\"jump\":" << (input.Jump ? "true" : "false")
        << ",\"boost\":" << (input.HoldingBoost ? "true" : "false")
        << ",\"handbrake\":" << (input.Handbrake ? "true" : "false") << "}";
    return out.str();
}

// `rb_domain::CarState::boost_amount` is 0-100 (see
// `rb_replay_ingest::convert::boost_raw_to_percent`'s doc comment) --
// `BoostWrapper::GetCurrentBoostAmount()` is the 0.0-1.0 fraction, so scale
// it here to keep both ingestion adapters in the same unit.
std::string carJson(int playerId, CarWrapper car, const ControllerInput &input)
{
    std::ostringstream out;
    out << "{\"player_id\":" << playerId << ",";
    std::string actor = rbActorJson(car);
    // rbActorJson() returns a full `{...}` object; splice its fields in
    // rather than nesting it, since CarState's wire shape is flat plus
    // `boost_amount`/`input`, not `{"actor": {...}, ...}`.
    out << actor.substr(1, actor.size() - 2);
    // A car mid-spawn, mid-reset or demolished can have no boost component;
    // reading through a null wrapper crashes the game. Record 0 instead.
    BoostWrapper boost = car.GetBoostComponent();
    float boostAmount = boost.IsNull() ? 0.0f : boost.GetCurrentBoostAmount() * 100.0f;
    out << ",\"boost_amount\":" << boostAmount;
    out << ",\"input\":" << inputJson(input);
    out << "}";
    return out.str();
}
} // namespace

namespace
{
const char *VEHICLE_INPUT_EVENT = "Function TAGame.Car_TA.SetVehicleInput";

// Pulls the string value of `"key": "..."` out of a flat JSON object,
// undoing the escapes a path needs (`\`, `\"`, `\/`). Job files are tiny
// and written by `rb_run_tapes`, so this is not a general JSON parser.
bool jsonStringField(const std::string &text, const std::string &key, std::string &out)
{
    size_t at = text.find("\"" + key + "\"");
    if (at == std::string::npos)
    {
        return false;
    }
    size_t quote = text.find('"', text.find(':', at) + 1);
    if (quote == std::string::npos)
    {
        return false;
    }
    out.clear();
    for (size_t i = quote + 1; i < text.size(); ++i)
    {
        if (text[i] == '"')
        {
            return true;
        }
        if (text[i] == '\\' && i + 1 < text.size())
        {
            ++i;
        }
        out += text[i];
    }
    return false;
}

// True when the object has `"key": true`.
bool jsonTrueField(const std::string &text, const std::string &key)
{
    size_t at = text.find("\"" + key + "\"");
    if (at == std::string::npos)
    {
        return false;
    }
    size_t colon = text.find(':', at);
    return colon != std::string::npos && text.find("true", colon) == text.find_first_not_of(" \t\r\n", colon + 1);
}
} // namespace

void RustyBulletCapturePlugin::onLoad()
{
    gameWrapper->HookEventWithCallerPost<CarWrapper>(
        VEHICLE_INPUT_EVENT,
        [this](CarWrapper car, void *params, std::string eventName) { onVehicleInput(car, params, eventName); });

    cvarManager->registerNotifier(
        "rb_capture_start",
        [this](std::vector<std::string> args) { startCapture(args); },
        "Start recording a Rusty Bullet capture file: rb_capture_start <path.jsonl>",
        PERMISSION_ALL);

    cvarManager->registerNotifier(
        "rb_capture_stop",
        [this](std::vector<std::string> args) { stopCapture(args); },
        "Stop the current Rusty Bullet capture recording, if any",
        PERMISSION_ALL);

    pollJobs(alive_);
}

void RustyBulletCapturePlugin::onUnload()
{
    *alive_ = false;
    // Remove the per-tick hook before this plugin's memory goes away: a
    // hook left behind calls into a freed `this` on the next tick.
    gameWrapper->UnhookEventPost(VEHICLE_INPUT_EVENT);
    stopCapture({});
}

std::filesystem::path RustyBulletCapturePlugin::jobDir() const
{
    return gameWrapper->GetDataFolder() / "rusty_bullet_capture";
}

void RustyBulletCapturePlugin::pollJobs(std::shared_ptr<bool> alive)
{
    if (!*alive)
    {
        return;
    }

    std::error_code ec;
    std::filesystem::path dir = jobDir();
    std::filesystem::create_directories(dir, ec);
    std::filesystem::path job = dir / "job.json";
    if (std::filesystem::exists(job, ec))
    {
        std::ifstream in(job);
        std::stringstream text;
        text << in.rdbuf();
        in.close();
        runJob(text.str());
        std::filesystem::remove(job, ec);
        if (ec)
        {
            cvarManager->log("rusty_bullet_capture: could not delete the job file: " + ec.message());
        }
    }

    // `capturing=` lets the runner confirm a start took effect.
    std::ofstream beat(dir / "heartbeat.txt", std::ios::out | std::ios::trunc);
    beat << "version=1.4\ncapturing=" << (capturing_ ? 1 : 0) << "\n";

    gameWrapper->SetTimeout([this, alive](GameWrapper *) { pollJobs(alive); }, 1.0f);
}

void RustyBulletCapturePlugin::runJob(const std::string &text)
{
    std::string path;
    if (jsonStringField(text, "start", path))
    {
        startCapture({"rb_capture_start", path});
    }
    else if (jsonTrueField(text, "stop"))
    {
        stopCapture({});
    }
    else
    {
        cvarManager->log("rusty_bullet_capture: unrecognised job file: " + text);
    }
}

void RustyBulletCapturePlugin::startCapture(std::vector<std::string> args)
{
    std::string path = args.size() > 1 ? args[1] : "rusty_bullet_capture.jsonl";

    if (captureFile_.is_open())
    {
        captureFile_.close();
    }

    captureFile_.open(path, std::ios::out | std::ios::trunc);
    if (!captureFile_.is_open())
    {
        cvarManager->log("rusty_bullet_capture: failed to open '" + path + "' for writing");
        return;
    }

    capturing_ = true;
    lastPhysicsFrame_ = -1;
    haveStartTime_ = false;
    lastTimestampSecs_ = -1.0f;
    lastInputs_.clear();
    // Log the two input sources for the first few ticks of every recording,
    // so a capture whose inputs read all-zero can be diagnosed from
    // bakkesmod.log without rebuilding.
    debugTicksLeft_ = 5;
    cvarManager->log("rusty_bullet_capture: recording to '" + path + "'");
}

void RustyBulletCapturePlugin::stopCapture(std::vector<std::string> /*args*/)
{
    if (!capturing_)
    {
        return;
    }

    capturing_ = false;
    captureFile_.close();
    cvarManager->log("rusty_bullet_capture: stopped recording");
}

void RustyBulletCapturePlugin::onVehicleInput(CarWrapper car, void *params, std::string /*eventName*/)
{
    if (!capturing_ || !isLive(car))
    {
        return;
    }

    // `SetVehicleInput(ControllerInput& NewInput)`: `params` is the input
    // the game is applying to this car this tick, whoever produced it.
    // `CarWrapper::GetInput()` stayed all-zero for an RLBot-driven car in
    // every tape-bot capture of 2026-10-06 (RB-RESEARCH-O008) while the car
    // visibly jumped and dodged, so the argument is the record, with
    // `GetInput()` only as the fallback for a null argument.
    ControllerInput input = params != nullptr ? *static_cast<ControllerInput *>(params) : car.GetInput();
    lastInputs_[car.memory_address] = input;
    if (debugTicksLeft_ > 0)
    {
        --debugTicksLeft_;
        ControllerInput stored = car.GetInput();
        cvarManager->log("rusty_bullet_capture: input argument " + inputJson(input) + " | CarWrapper::GetInput() " +
                         inputJson(stored));
    }

    ServerWrapper server = gameWrapper->GetCurrentGameState();
    if (server.IsNull())
    {
        return;
    }

    // Mid-reset there may be no live ball for a tick: skip it.
    BallWrapper ball = liveBall(server);
    if (ball.IsNull())
    {
        return;
    }

    // `SetVehicleInput` fires once per car per physics tick; every car's
    // firing this tick sees the same `GetPhysicsFrame()`, so only the first
    // one to arrive actually writes a line -- this is what keeps a match
    // with N cars from producing N near-duplicate lines per tick.
    int physicsFrame = ball.GetPhysicsFrame();
    if (physicsFrame == lastPhysicsFrame_)
    {
        return;
    }
    lastPhysicsFrame_ = physicsFrame;

    writeFrame(server, ball);
}

void RustyBulletCapturePlugin::writeFrame(ServerWrapper server, BallWrapper ball)
{
    if (!haveStartTime_)
    {
        // First frame of this recording: treat its own physics time as
        // t=0, matching `PhysicsFrame::timestamp_secs`'s doc comment
        // ("seconds since the start of the capture/replay, not a
        // wall-clock time").
        startPhysicsTime_ = ball.GetPhysicsTime();
        haveStartTime_ = true;
    }
    float timestampSecs = ball.GetPhysicsTime() - startPhysicsTime_;
    // A respawned ball restarts its physics clock: rebase so timestamps
    // keep increasing by one tick instead of jumping back.
    if (timestampSecs <= lastTimestampSecs_)
    {
        timestampSecs = lastTimestampSecs_ + 1.0f / 120.0f;
        startPhysicsTime_ = ball.GetPhysicsTime() - timestampSecs;
    }
    lastTimestampSecs_ = timestampSecs;

    // `server.GetPRIs()` + `PriWrapper::GetCar()` looked like the natural way
    // to enumerate cars, but a real capture proved it wrong: in freeplay the
    // PRI's `Car` back-reference never gets updated to the live-driven pawn
    // (PRI exists for scoreboard/stat tracking, which freeplay has none of),
    // so every line recorded the same frozen spawn-point transform with
    // all-zero input while the ball moved for real. `GameEventWrapper::GetCars()`
    // (inherited via `ServerWrapper` -> `TeamGameEventWrapper`) is the game's
    // own live list of spawned car actors -- the same source cameras/
    // scoreboards use -- and reflects real movement.
    ArrayWrapper<CarWrapper> cars = server.GetCars();

    std::ostringstream line;
    line << "{\"timestamp_secs\":" << timestampSecs << ",\"ball\":" << rbActorJson(ball) << ",\"cars\":[";

    bool first = true;
    int nextPlayerId = 0;
    // `player_id` here is just this recording session's car iteration order,
    // not a stable cross-session id -- BakkesMod's own PRI/unique-id
    // wrappers exist, but a one-off capture script (see RB-VERIFY-002's
    // Non-goals) never replays a capture against a second session, so a
    // per-session ordinal is all `rb_capture_ingest` needs.
    if (!cars.IsNull())
    {
        for (int i = 0; i < cars.Count(); ++i)
        {
            CarWrapper car = cars.Get(i);
            if (!isLive(car))
            {
                continue;
            }

            if (!first)
            {
                line << ",";
            }
            first = false;
            auto hooked = lastInputs_.find(car.memory_address);
            ControllerInput input = hooked != lastInputs_.end() ? hooked->second : car.GetInput();
            line << carJson(nextPlayerId, car, input);
            ++nextPlayerId;
        }
    }

    line << "]}";

    captureFile_ << line.str() << "\n";
    captureFile_.flush();
}
