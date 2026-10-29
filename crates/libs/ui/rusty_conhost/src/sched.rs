//! The engine's turn order (design §7.2) and admission arithmetic (§7.4)
//! as pure functions, so the fairness and capacity rules are tested
//! without a driver.
//!
//! One turn: take the control lane, handle a child exit, take at most one
//! slice of terminal input, submit a READ_IO if admission allows, then
//! wait. The wait reports at most one packet, because only one READ_IO is
//! ever in flight. So between any two packets the control lane and the
//! child's exit are served, and input never starves either.

use platform::term::WinSize;

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
}

/// What one turn does, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Turn {
    pub control: Control,
    pub child_exited: bool,
    /// Bytes to take from the input lane and hand to the console.
    pub input: usize,
    pub submit_read: bool,
    /// Work is left that a later turn can do at once, so the wait must
    /// not block.
    pub poll: bool,
}

/// Plan one turn.
pub(crate) fn turn(v: &View) -> Turn {
    let input = if v.output <= INPUT_PAUSE_ABOVE {
        v.input.min(INPUT_SLICE)
    } else {
        0
    };
    Turn {
        control: v.control,
        child_exited: v.exited,
        input,
        submit_read: !v.closed && !v.read_in_flight && admits_read(v.waiting, v.parked),
        poll: input < v.input && v.output <= INPUT_PAUSE_ABOVE,
    }
}

/// Whether another packet may be accepted: fewer than [`MAX_WAITING`]
/// calls wait, and the most variable input one packet can bring still
/// fits under [`PARKED_CAP`]. A packet that then arrives always fits.
pub(crate) fn admits_read(waiting: usize, parked: usize) -> bool {
    waiting < MAX_WAITING && parked + PACKET_INPUT_CAP <= PARKED_CAP
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
    }

    impl Sim {
        fn new() -> Sim {
            Sim {
                view: View::default(),
                packets: 0,
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
            assert!(t.poll, "turn {k}: input is left");
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
        assert!(!t.poll, "nothing can be done until the reader drains");
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
        assert!(admits_read(0, PARKED_CAP - PACKET_INPUT_CAP));
        assert!(!admits_read(0, PARKED_CAP - PACKET_INPUT_CAP + 1));

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
}
