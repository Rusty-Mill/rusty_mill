//! The outcome label is objective (derived from goals), and censored states —
//! within a horizon of match end with no further goal — are dropped, not faked.

use replay_analyzer::model::Event;
use replay_value::dataset::next_goal_label;

fn goal(t: f32, team: i32) -> Event {
    Event::Goal {
        t,
        scorer: None,
        team: Some(team),
    }
}

#[test]
fn label_is_objective_and_censors_correctly() {
    // Goals: team 0 at t=5, team 1 at t=30. Match ends at 60. Horizon 10 s.
    let evs = vec![goal(5.0, 0), goal(30.0, 1)];
    let (h, end) = (10.0, 60.0);

    // Next goal (t=5, team 0) is within the horizon of t=0.
    assert_eq!(next_goal_label(&evs, 0.0, 0, h, end), Some(1.0));
    assert_eq!(next_goal_label(&evs, 0.0, 1, h, end), Some(0.0));

    // From t=5 the next goal is t=30 (>10 s away): nobody scores within horizon.
    assert_eq!(next_goal_label(&evs, 5.0, 0, h, end), Some(0.0));
    assert_eq!(next_goal_label(&evs, 5.0, 1, h, end), Some(0.0));

    // Approaching the second goal, team 1 scores within the horizon.
    assert_eq!(next_goal_label(&evs, 22.0, 1, h, end), Some(1.0));
    assert_eq!(next_goal_label(&evs, 22.0, 0, h, end), Some(0.0));

    // After the last goal, a full horizon of match remains with no goal -> 0.
    assert_eq!(next_goal_label(&evs, 45.0, 0, h, end), Some(0.0));

    // ...but within a horizon of match end with no further goal -> censored.
    assert_eq!(next_goal_label(&evs, 55.0, 0, h, end), None);
}
