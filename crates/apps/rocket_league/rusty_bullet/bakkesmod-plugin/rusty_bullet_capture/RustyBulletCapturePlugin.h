#pragma once

#include "bakkesmod/plugin/bakkesmodplugin.h"
#include "bakkesmod/plugin/bakkesmodsdk.h"
#include "bakkesmod/wrappers/GameWrapper.h"
#include "bakkesmod/wrappers/GameEvent/ServerWrapper.h"
#include "bakkesmod/wrappers/GameObject/BallWrapper.h"
#include "bakkesmod/wrappers/GameObject/CarWrapper.h"

#include <fstream>
#include <string>
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

    // Builds and appends one capture-file line from the current server/ball
    // state. Does nothing if `capturing_` is false or either wrapper is null.
    void writeFrame(ServerWrapper server, BallWrapper ball);

    std::ofstream captureFile_;
    bool capturing_ = false;
    int lastPhysicsFrame_ = -1;
    bool haveStartTime_ = false;
    float startPhysicsTime_ = 0.0f;
};
