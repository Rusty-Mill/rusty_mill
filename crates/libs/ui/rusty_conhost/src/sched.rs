//! The engine's turn order (design §7.2) and admission arithmetic (§7.4)
//! as pure functions, so the fairness and capacity rules are tested
//! without a driver.
//!
//! One turn: take the control lane, handle a child exit, take at most one
//! slice of terminal input, submit a READ_IO if admission allows, then
//! wait. The wait reports at most one packet, because only one READ_IO is
//! ever in flight. So between any two packets the control lane and the
//! child's exit are served, and input never starves either. The CTRL_CLOSE
//! grace deadline (§7.3) is checked first on every turn, so it is served
//! on the first turn at or after it, whatever else is ready.

use std::time::{Duration, Instant};

use platform::term::WinSize;

/// How long clients have to exit after CTRL_CLOSE before the server closes.
pub(crate) const GRACE: Duration = Duration::from_secs(2);

/// Terminal input taken per turn.
pub(crate) const INPUT_SLICE: usize = 16 * 1024;
/// Input is taken only while the output queue holds at most this much:
/// the 64 KiB client budget plus the 16 KiB host reserve.
pub(crate) const INPUT_PAUSE_ABOVE: usize = 80 * 1024;
/// Calls waiting in the host, parked writes included.
pub(crate) const MAX_WAITING: usize = 64;
/// Payload retained by parked calls, reserved before a READ_IO is sent.
pub(crate) const PARKED_CAP: usize = 1024 * 1024;
/// Variable input read from one packet.
pub(crate) const PACKET_INPUT_CAP: usize = 64 * 1024;
/// The most of an API's fixed descriptor a reply writes back: every
/// struct served fits (the largest is `CONSOLE_SCREENBUFFERINFO_MSG`), so
/// an oversized descriptor costs nothing parked.
pub(crate) const WRITE_BACK_CAP: usize = 92;
/// The most one held call retains: its text, at most the packet's input
/// (a title's OSC is capped well below it), plus its write-back.
pub(crate) const MAX_RETAINED_PER_CALL: usize = PACKET_INPUT_CAP + WRITE_BACK_CAP;

/// The control lane: coalesced flags that never need capacity. Setting a
/// flag that is already set changes nothing; the latest size wins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Control {
    pub hang_up: bool,
    pub output_drained: bool,
    pub resize: Option<WinSize>,
}

impl Control {
    /// Every flag set since the last call, leaving the lane clear.
    pub fn take(&mut self) -> Control {
        std::mem::take(self)
    }
}

/// What the engine sees at the start of a turn.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct View {
    /// The control flags taken for this turn.
    pub control: Control,
    /// A wait reported the child's exit and the console hasn't seen it.
    pub exited: bool,
    /// Bytes in the output queue.
    pub output: usize,
    /// Bytes waiting in the input lane.
    pub input: usize,
    pub read_in_flight: bool,
    /// Calls the console holds: parked writes, and later waiting reads.
    pub waiting: usize,
    /// Payload bytes those calls retain.
    pub parked: usize,
    /// The server is closed: nothing more is read from the driver.
    pub closed: bool,
    /// Waiting reads stopped at a call's reply budget with input left.
    pub reads_ready: bool,
    /// Parked writes stopped at the budget on the host's own answers.
    pub writes_ready: bool,
}

/// What one turn does, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Turn {
    pub control: Control,
    pub child_exited: bool,
    /// Serve the waiting reads the last call left, before new input.
    pub serve_reads: bool,
    /// Bytes to take from the input lane and hand to the console.
    pub input: usize,
    pub submit_read: bool,
    /// Go on with the parked writes the last pump left, as a drain would.
    pub pump_writes: bool,
}

/// Plan one turn.
pub(crate) fn turn(v: &View) -> Turn {
    // Reads owed queued input go first, so the queue stays bounded.
    let input = if v.output <= INPUT_PAUSE_ABOVE && !v.reads_ready {
        v.input.min(INPUT_SLICE)
    } else {
        0
    };
    Turn {
        control: v.control,
        child_exited: v.exited,
        serve_reads: v.reads_ready,
        input,
        submit_read: read_admitted(v),
        pump_writes: v.writes_ready,
    }
}

/// Whether a READ_IO can be sent now: none is in flight and admission
/// allows another packet.
fn read_admitted(v: &View) -> bool {
    !v.closed && !v.read_in_flight && admits_read(v.waiting, v.parked)
}

/// Whether the engine's wait must not block: work is left that the next
/// turn can do at once. Asked of the state after the turn's events, not
/// of its plan: an input slice can leave reads ready, stop at a write
/// that ended in ESC with input left, or complete calls and so reopen
/// admission with no READ_IO armed; no wake announces any of them.
pub(crate) fn must_poll(v: &View) -> bool {
    v.reads_ready
        || v.writes_ready
        || (v.input > 0 && v.output <= INPUT_PAUSE_ABOVE)
        || read_admitted(v)
}

/// Whether another packet may be accepted: fewer than [`MAX_WAITING`]
/// calls wait, and the most one call can retain ([`MAX_RETAINED_PER_CALL`]) still
/// fits under [`PARKED_CAP`]. A packet that then arrives always fits.
pub(crate) fn admits_read(waiting: usize, parked: usize) -> bool {
    waiting < MAX_WAITING && parked + MAX_RETAINED_PER_CALL <= PARKED_CAP
}

/// The CTRL_CLOSE grace deadline: absolute, armed at most once, and never
/// moved by a later hang-up, child exit, wake or packet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Grace {
    #[default]
    Unarmed,
    Armed(Instant),
    /// The deadline passed and was served.
    Spent,
}

impl Grace {
    /// Arm the deadline at `now` + [`GRACE`], unless it was armed before.
    pub fn arm(&mut self, now: Instant) {
        if *self == Grace::Unarmed {
            *self = Grace::Armed(now + GRACE);
        }
    }

    /// Whether the deadline passed by `now`; true on one call only.
    pub fn expire(&mut self, now: Instant) -> bool {
        match *self {
            Grace::Armed(deadline) if now >= deadline => {
                *self = Grace::Spent;
                true
            }
            _ => false,
        }
    }

    /// When the engine's wait must return: at once when work is left
    /// (`poll`), else at the deadline, if one is armed.
    pub fn until(&self, poll: bool, now: Instant) -> Option<Instant> {
        match *self {
            _ if poll => Some(now),
            Grace::Armed(deadline) => Some(deadline),
            Grace::Unarmed | Grace::Spent => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scripted engine: every turn follows [`turn`], the wait always
    /// returns a packet when one is in flight, and the console's side is
    /// reduced to the counters `turn` reads.
    struct Sim {
        view: View,
        packets: usize,
        /// The last turn's wait was a poll.
        polled: bool,
    }

    impl Sim {
        fn new() -> Sim {
            Sim {
                view: View::default(),
                packets: 0,
                polled: false,
            }
        }

        /// Run one turn. `ready_io` says whether the wait returns the
        /// in-flight read. Returns the plan the turn followed.
        fn step(&mut self, ready_io: bool) -> Turn {
            let t = turn(&self.view);
            self.view.control = Control::default();
            self.view.exited = false;
            self.view.input -= t.input;
            if t.submit_read {
                self.view.read_in_flight = true;
            }
            self.polled = must_poll(&self.view);
            if ready_io && self.view.read_in_flight {
                self.view.read_in_flight = false;
                self.packets += 1;
            }
            t
        }
    }

    #[test]
    fn control_and_exit_are_served_on_the_turn_they_appear_under_always_ready_io() {
        let mut sim = Sim::new();
        for k in 0..100 {
            if k == 37 {
                sim.view.control.hang_up = true;
            }
            if k == 61 {
                sim.view.exited = true;
            }
            let t = sim.step(true);
            assert_eq!(t.control.hang_up, k == 37, "turn {k}");
            assert_eq!(t.child_exited, k == 61, "turn {k}");
            assert!(t.submit_read, "turn {k}: one packet per turn");
        }
        assert_eq!(sim.packets, 100);
    }

    #[test]
    fn continuous_input_is_taken_a_slice_at_a_time_without_starving_packets() {
        let mut sim = Sim::new();
        for k in 0..50 {
            sim.view.input = 64 * 1024;
            if k == 20 {
                sim.view.control.output_drained = true;
            }
            let t = sim.step(true);
            assert_eq!(t.input, INPUT_SLICE, "turn {k}");
            assert!(t.submit_read, "turn {k}");
            assert!(sim.polled, "turn {k}: input is left");
            assert_eq!(t.control.output_drained, k == 20, "turn {k}");
        }
        assert_eq!(sim.packets, 50);
    }

    #[test]
    fn control_behind_queued_input_is_handled_before_that_input() {
        let mut sim = Sim::new();
        sim.view.input = 64 * 1024;
        sim.view.control.hang_up = true;
        sim.view.control.resize = Some(WinSize {
            rows: 30,
            cols: 100,
        });
        let t = sim.step(false);
        // The turn's control is applied before its input slice, so none of
        // the queued input precedes it.
        assert!(t.control.hang_up);
        assert_eq!(
            t.control.resize,
            Some(WinSize {
                rows: 30,
                cols: 100
            })
        );
        assert_eq!(t.input, INPUT_SLICE);
    }

    #[test]
    fn input_pauses_while_the_output_queue_is_past_its_reserve() {
        let mut sim = Sim::new();
        sim.view.input = 64 * 1024;
        sim.view.output = INPUT_PAUSE_ABOVE + 1;
        sim.view.control.hang_up = true;
        let t = sim.step(true);
        assert_eq!(t.input, 0);
        assert!(!sim.polled, "nothing can be done until the reader drains");
        assert!(t.control.hang_up, "control is never paused");

        sim.view.output = INPUT_PAUSE_ABOVE;
        sim.view.control.output_drained = true;
        let t = sim.step(true);
        assert!(t.control.output_drained);
        assert_eq!(
            t.input, INPUT_SLICE,
            "input resumes on the turn the drain is seen"
        );
    }

    #[test]
    fn admission_holds_reads_back_at_the_waiting_and_parked_caps() {
        assert!(admits_read(MAX_WAITING - 1, 0));
        assert!(!admits_read(MAX_WAITING, 0));
        assert!(admits_read(0, PARKED_CAP - MAX_RETAINED_PER_CALL));
        assert!(!admits_read(0, PARKED_CAP - MAX_RETAINED_PER_CALL + 1));

        let v = View {
            waiting: MAX_WAITING,
            ..View::default()
        };
        assert!(!turn(&v).submit_read);
        let v = View {
            read_in_flight: true,
            ..View::default()
        };
        assert!(!turn(&v).submit_read, "one READ_IO at a time");
        let v = View {
            closed: true,
            ..View::default()
        };
        assert!(!turn(&v).submit_read);
    }

    #[test]
    fn coalesced_control_takes_every_flag_once() {
        // Two hang-ups and two resizes landed before the engine looked.
        let mut lane = Control {
            hang_up: true,
            resize: Some(WinSize { rows: 1, cols: 1 }),
            ..Control::default()
        };
        lane.hang_up = true;
        lane.resize = Some(WinSize { rows: 2, cols: 3 });
        let taken = lane.take();
        assert!(taken.hang_up);
        assert_eq!(taken.resize, Some(WinSize { rows: 2, cols: 3 }));
        assert_eq!(lane, Control::default());
    }

    #[test]
    fn the_wait_polls_on_what_the_turn_left_not_on_its_plan() {
        // The planned slice took the whole lane: the plan saw no work left.
        let before = View {
            input: 10,
            ..View::default()
        };
        assert_eq!(turn(&before).input, 10);
        // Handling it left reads ready, or a cut at an ESC write end left
        // input queued.
        let after = View {
            reads_ready: true,
            ..View::default()
        };
        assert!(must_poll(&after));
        assert!(must_poll(&View {
            input: 1,
            read_in_flight: true,
            ..View::default()
        }));
        assert!(!must_poll(&View {
            read_in_flight: true,
            ..View::default()
        }));
    }

    #[test]
    fn the_wait_polls_when_admission_reopens_with_no_read_armed() {
        let full = View {
            waiting: MAX_WAITING,
            ..View::default()
        };
        assert!(!turn(&full).submit_read && !must_poll(&full));
        let freed = View {
            waiting: MAX_WAITING - 1,
            ..full
        };
        assert!(must_poll(&freed), "a call completed: arm the read");
        let parked = View {
            waiting: 1,
            parked: PARKED_CAP - MAX_RETAINED_PER_CALL + 1,
            ..View::default()
        };
        assert!(!must_poll(&parked));
        let parked = View {
            parked: PARKED_CAP - MAX_RETAINED_PER_CALL,
            ..parked
        };
        assert!(must_poll(&parked), "parked bytes fell to the boundary");
        assert!(!must_poll(&View {
            read_in_flight: true,
            ..parked
        }));
    }

    #[test]
    fn writes_cut_on_answers_pump_again_without_a_wake() {
        let cut = View {
            writes_ready: true,
            read_in_flight: true,
            ..View::default()
        };
        assert!(turn(&cut).pump_writes && must_poll(&cut));
        let idle = View {
            writes_ready: false,
            ..cut
        };
        assert!(!turn(&idle).pump_writes && !must_poll(&idle));
    }

    #[test]
    fn reads_owed_input_are_served_before_new_input_and_after_control() {
        let v = View {
            control: Control {
                hang_up: true,
                ..Control::default()
            },
            input: 100,
            reads_ready: true,
            ..View::default()
        };
        let t = turn(&v);
        assert!(t.control.hang_up && t.serve_reads && must_poll(&v));
        assert_eq!(t.input, 0, "input waits while reads are owed the queue");
        let t = turn(&View {
            reads_ready: false,
            ..v
        });
        assert!(!t.serve_reads);
        assert_eq!(t.input, 100);
    }

    #[test]
    fn the_grace_expires_on_the_first_turn_at_or_after_it_under_always_ready_io() {
        let start = Instant::now();
        let mut sim = Sim::new();
        let mut grace = Grace::default();
        let mut expired_at = None;
        // A turn every millisecond, each serving a packet, hang-ups and
        // child exits re-arriving throughout.
        for ms in 0..3_000u64 {
            let now = start + Duration::from_millis(ms);
            if grace.expire(now) {
                assert!(expired_at.is_none(), "expires once");
                expired_at = Some(ms);
            }
            sim.view.control.hang_up = ms % 7 == 0;
            sim.view.exited = ms % 11 == 0;
            let t = sim.step(true);
            if t.control.hang_up || t.child_exited {
                grace.arm(now);
            }
            assert!(t.submit_read, "{ms} ms: one packet per turn");
        }
        assert_eq!(expired_at, Some(2_000), "armed at 0 ms, never moved");
        assert_eq!(grace, Grace::Spent);
        grace.arm(start + Duration::from_secs(5));
        assert_eq!(grace, Grace::Spent, "a spent deadline is not re-armed");
    }

    #[test]
    fn the_grace_bounds_an_idle_wait_and_polling_comes_first() {
        let now = Instant::now();
        let mut grace = Grace::default();
        assert_eq!(grace.until(false, now), None, "nothing to wait for");
        assert_eq!(grace.until(true, now), Some(now));
        grace.arm(now);
        let deadline = now + GRACE;
        assert_eq!(grace.until(false, now), Some(deadline));
        assert_eq!(grace.until(true, now), Some(now), "work left: poll");
        let late = now + Duration::from_millis(1_500);
        grace.arm(late);
        assert_eq!(grace.until(false, late), Some(deadline), "absolute");
        assert!(!grace.expire(deadline - Duration::from_nanos(1)));
        assert!(grace.expire(deadline + Duration::from_secs(9)));
        assert_eq!(grace.until(false, deadline), None);
    }
}
