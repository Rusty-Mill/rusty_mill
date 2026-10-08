#pragma once

#include "bakkesmod/plugin/bakkesmodplugin.h"
#include "bakkesmod/plugin/bakkesmodsdk.h"
#include "bakkesmod/wrappers/GameWrapper.h"
#include "bakkesmod/wrappers/GameEvent/ServerWrapper.h"
#include "bakkesmod/wrappers/GameObject/BallWrapper.h"
#include "bakkesmod/wrappers/GameObject/CarWrapper.h"

#include <cstdint>
#include <filesystem>
#include <fstream>
#include <memory>
#include <string>
#include <unordered_map>
#include <vector>

// Records a JSON-Lines capture file matching ADR-0005 / RB-VERIFY-002-FR-001:
// one line per physics tick, `{"timestamp_secs", "ball", "cars"}`, readable
// by `rb_capture_ingest` without any changes on the Rust side. Built,
// loaded, and run against a real Rocket League + BakkesMod install -- see
// this directory's README.md and RB-VERIFY-002's spec Change history.
class RustyBulletCapturePlugin : public BakkesMod::Plugin::BakkesModPlugin
{
public:
    void onLoad() override;
    void onUnload() override;

private:
    // Hooked once per car per physics tick (`Function TAGame.Car_TA.SetVehicleInput`,
    // post). Multiple cars can fire this in the same tick; `lastPhysicsFrame_`
    // dedupes so exactly one capture line is written per tick regardless of
    // car count.
    void onVehicleInput(CarWrapper car, void *params, std::string eventName);

    void startCapture(std::vector<std::string> args);
    void stopCapture(std::vector<std::string> args);

    // Job-file trigger (1.4): once a second, looks for `job.json` in
    // `<BakkesMod data>/rusty_bullet_capture/`, runs it through the same
    // `startCapture`/`stopCapture` as the console commands, then deletes it
    // (the delete is the runner's acknowledgement). Also rewrites
    // `heartbeat.txt` there every poll. `alive` outlives the plugin inside
    // the pending timeout so a poll after unload does nothing.
    void pollJobs(std::shared_ptr<bool> alive);
    void runJob(const std::string &text);
    std::filesystem::path jobDir() const;

    // Builds and appends one capture-file line from the current server/ball
    // state. Does nothing if `capturing_` is false or either wrapper is null.
    void writeFrame(ServerWrapper server, BallWrapper ball);

    std::ofstream captureFile_;
    bool capturing_ = false;
    int lastPhysicsFrame_ = -1;
    bool haveStartTime_ = false;
    float startPhysicsTime_ = 0.0f;
    // Last timestamp written, so a ball respawn can't make time jump back.
    float lastTimestampSecs_ = -1.0f;
    // The input each car's `SetVehicleInput` was called with most recently,
    // keyed by the car actor's address; what the capture records as `input`
    // (1.3; `CarWrapper::GetInput()` is not updated for an RLBot-driven car).
    std::unordered_map<std::uintptr_t, ControllerInput> lastInputs_;
    // Ticks left to log the input argument against `GetInput()` after a
    // `rb_capture_start`, for diagnosis.
    int debugTicksLeft_ = 0;
    std::shared_ptr<bool> alive_ = std::make_shared<bool>(true);
};
