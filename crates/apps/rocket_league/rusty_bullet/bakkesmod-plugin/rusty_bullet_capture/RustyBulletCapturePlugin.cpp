#include "RustyBulletCapturePlugin.h"

#include "bakkesmod/wrappers/arraywrapper.h"
#include "bakkesmod/wrappers/GameObject/CarComponent/BoostWrapper.h"
#include "bakkesmod/wrappers/GameObject/PriWrapper.h"

#include <sstream>

BAKKESMOD_PLUGIN(RustyBulletCapturePlugin, "Rusty Bullet capture", "1.0", PLUGINTYPE_FREEPLAY | PLUGINTYPE_SOCCAR)

namespace
{
// Field names/nesting below must match crates/rb_capture_ingest/src/wire.rs
// exactly (see ADR-0005) -- this is the wire format itself, not incidental
// formatting.

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
std::string carJson(int playerId, CarWrapper car)
{
    std::ostringstream out;
    out << "{\"player_id\":" << playerId << ",";
    std::string actor = rbActorJson(car);
    // rbActorJson() returns a full `{...}` object; splice its fields in
    // rather than nesting it, since CarState's wire shape is flat plus
    // `boost_amount`/`input`, not `{"actor": {...}, ...}`.
    out << actor.substr(1, actor.size() - 2);
    out << ",\"boost_amount\":" << (car.GetBoostComponent().GetCurrentBoostAmount() * 100.0f);
    out << ",\"input\":" << inputJson(car.GetInput());
    out << "}";
    return out.str();
}
} // namespace

void RustyBulletCapturePlugin::onLoad()
{
    gameWrapper->HookEventWithCallerPost<CarWrapper>(
        "Function TAGame.Car_TA.SetVehicleInput",
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
}

void RustyBulletCapturePlugin::onUnload()
{
    stopCapture({});
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

void RustyBulletCapturePlugin::onVehicleInput(CarWrapper car, void * /*params*/, std::string /*eventName*/)
{
    if (!capturing_ || car.IsNull())
    {
        return;
    }

    ServerWrapper server = gameWrapper->GetCurrentGameState();
    if (server.IsNull())
    {
        return;
    }

    BallWrapper ball = server.GetBall();
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

    ArrayWrapper<PriWrapper> pris = server.GetPRIs();

    std::ostringstream line;
    line << "{\"timestamp_secs\":" << timestampSecs << ",\"ball\":" << rbActorJson(ball) << ",\"cars\":[";

    bool first = true;
    int nextPlayerId = 0;
    // `player_id` here is just this recording session's PRI iteration
    // order, not a stable cross-session id -- BakkesMod's own PRI/unique-id
    // wrappers exist, but a one-off capture script (see RB-VERIFY-002's
    // Non-goals) never replays a capture against a second session, so a
    // per-session ordinal is all `rb_capture_ingest` needs.
    if (!pris.IsNull())
    {
        for (int i = 0; i < pris.Count(); ++i)
        {
            PriWrapper pri = pris.Get(i);
            if (pri.IsNull())
            {
                continue;
            }
            CarWrapper car = pri.GetCar();
            if (car.IsNull())
            {
                continue;
            }

            if (!first)
            {
                line << ",";
            }
            first = false;
            line << carJson(nextPlayerId, car);
            ++nextPlayerId;
        }
    }

    line << "]}";

    captureFile_ << line.str() << "\n";
    captureFile_.flush();
}
