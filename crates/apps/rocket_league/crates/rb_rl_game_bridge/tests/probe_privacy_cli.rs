//! Exercise the live runner's analysis flag with synthetic data and the actual probe binary.
#![allow(clippy::unwrap_used)]

#[test]
fn no_names_suppresses_names_in_successful_and_failed_analysis_output() {
    use rb_rl_game_bridge::{encode_game_packet, GameBundle, GameEvent};
    let path = std::env::temp_dir().join(format!("rb-probe-privacy-{}.bin", std::process::id()));
    let bundle = GameBundle {
        app_seconds: 0.0,
        delta_physics_frames: 1,
        events: vec![GameEvent::PlayerSpawn {
            id: 38,
            command_id: None,
            name: "SYNTHETIC_PRIVATE_NAME".into(),
            team: 0,
        }],
    };
    let mut bytes = rb_rlbot_wire::frame::frame(&encode_game_packet(&bundle).unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let probe = |hidden: bool| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_rb_rl_game_probe"));
        command.arg("--analyze").arg(&path);
        if hidden {
            command.arg("--no-names");
        }
        command.output().unwrap()
    };
    let shown = probe(false);
    assert!(shown.status.success());
    assert!(String::from_utf8_lossy(&shown.stdout).contains("SYNTHETIC_PRIVATE_NAME"));
    for truncated in [false, true] {
        if truncated {
            bytes.push(0); // A partial next header fails after the spawn has been printed.
            std::fs::write(&path, &bytes).unwrap();
        }
        let hidden = probe(true);
        assert_eq!(hidden.status.success(), !truncated);
        let stdout = String::from_utf8_lossy(&hidden.stdout);
        assert!(stdout.contains("PlayerSpawn: actor 38 command_id None team 0"));
        assert!(!stdout.contains("SYNTHETIC_PRIVATE_NAME"));
        assert!(!String::from_utf8_lossy(&hidden.stderr).contains("SYNTHETIC_PRIVATE_NAME"));
    }
    std::fs::remove_file(path).unwrap();
}
