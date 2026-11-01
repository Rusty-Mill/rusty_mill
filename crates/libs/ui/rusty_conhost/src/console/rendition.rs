//! Spotting the VT that resets the rendition (SGR 0, DECSTR, RIS) in the
//! bytes the screen model advances over, across write boundaries.
//! conhost clears the gridlines of a legacy attribute only there (its
//! `SetDefaultRenditionAttributes`); no other SGR touches them.

/// The most parameter bytes kept for one CSI; a longer one is not an SGR
/// reset anyone writes, and is skipped.
const PARAMS_MAX: usize = 64;

#[derive(Default)]
enum State {
    #[default]
    Ground,
    Esc,
    /// Inside a CSI: its parameter bytes, its intermediates, and whether
    /// it is private or overlong, which rules it out.
    Csi {
        params: Vec<u8>,
        bang: bool,
        other: bool,
    },
}

/// A byte-at-a-time watch for rendition resets.
#[derive(Default)]
pub(super) struct ResetWatch {
    state: State,
}

impl ResetWatch {
    /// Advance over `vt`; whether it held a reset.
    pub(super) fn scan(&mut self, vt: &[u8]) -> bool {
        let mut reset = false;
        for &b in vt {
            reset |= self.step(b);
        }
        reset
    }

    fn step(&mut self, b: u8) -> bool {
        // CAN and SUB end any sequence; ESC starts a new one.
        if b == 0x18 || b == 0x1A {
            self.state = State::Ground;
            return false;
        }
        if b == 0x1B {
            self.state = State::Esc;
            return false;
        }
        match &mut self.state {
            State::Ground => false,
            State::Esc => {
                self.state = if b == b'[' {
                    State::Csi {
                        params: Vec::new(),
                        bang: false,
                        other: false,
                    }
                } else {
                    State::Ground
                };
                // RIS.
                b == b'c'
            }
            State::Csi {
                params,
                bang,
                other,
            } => match b {
                // C0 inside a CSI is acted on in place.
                0x00..=0x1F => false,
                b'0'..=b'9' | b';' | b':' => {
                    if params.len() < PARAMS_MAX {
                        params.push(b);
                    } else {
                        *other = true;
                    }
                    false
                }
                b'!' => {
                    *bang = true;
                    false
                }
                0x40..=0x7E => {
                    let reset = match (b, *bang, *other) {
                        (b'm', false, false) => sgr_resets(params),
                        // DECSTR.
                        (b'p', true, false) => params.is_empty(),
                        _ => false,
                    };
                    self.state = State::Ground;
                    reset
                }
                // Private markers, other intermediates, DEL.
                _ => {
                    *other = true;
                    false
                }
            },
        }
    }
}

/// Whether SGR parameters hold a reset: an empty or 0 parameter, not
/// counting the arguments of an extended colour (38, 48, 58).
fn sgr_resets(params: &[u8]) -> bool {
    let list: Vec<&[u8]> = params.split(|&b| b == b';').collect();
    let mut i = 0;
    while i < list.len() {
        let p = list[i];
        // A colon parameter carries its own arguments.
        let head = p.split(|&b| b == b':').next().unwrap_or(&[]);
        let n: u32 = std::str::from_utf8(head)
            .ok()
            .and_then(|s| {
                if s.is_empty() {
                    Some(0)
                } else {
                    s.parse().ok()
                }
            })
            .unwrap_or(u32::MAX);
        if n == 0 && !p.contains(&b':') {
            return true;
        }
        i += match n {
            38 | 48 | 58 if !p.contains(&b':') => match list.get(i + 1) {
                Some(&b"5") => 3,
                Some(&b"2") => 5,
                _ => 1,
            },
            _ => 1,
        };
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resets(vt: &[u8]) -> bool {
        ResetWatch::default().scan(vt)
    }

    #[test]
    fn sgr_zero_and_empty_reset() {
        assert!(resets(b"\x1b[m"));
        assert!(resets(b"\x1b[0m"));
        assert!(resets(b"\x1b[31;0m"));
        assert!(resets(b"\x1b[;31m"));
        assert!(resets(b"\x1bc"), "RIS");
        assert!(resets(b"\x1b[!p"), "DECSTR");
    }

    #[test]
    fn other_sgr_and_colour_arguments_do_not() {
        assert!(!resets(b"\x1b[31m"));
        assert!(!resets(b"\x1b[24m"));
        assert!(!resets(b"\x1b[38;5;0m"));
        assert!(!resets(b"\x1b[48;2;0;0;0m"));
        assert!(!resets(b"\x1b[38:2:0:0:0m"));
        assert!(!resets(b"\x1b[4:0m"), "a colon 0 is an underline style");
        assert!(!resets(b"\x1b[?0m"), "private");
        assert!(!resets(b"\x1b[0K"));
        assert!(!resets(b"text 0m"));
    }

    #[test]
    fn a_reset_split_across_writes_is_seen() {
        let mut w = ResetWatch::default();
        assert!(!w.scan(b"ab\x1b["));
        assert!(!w.scan(b"0"));
        assert!(w.scan(b"m"));
    }

    #[test]
    fn cancelled_and_overlong_sequences_are_not_resets() {
        assert!(!resets(b"\x1b[0\x18m"));
        let long = format!("\x1b[{}0m", "1;".repeat(PARAMS_MAX));
        assert!(!resets(long.as_bytes()));
    }
}
