//! Flood limits — stage 4b-vi.
//!
//! Some records cost a peer almost nothing to send and cost this side a full
//! decrypt and a trip through the state machine each: empty application-data
//! records, warning alerts, `ChangeCipherSpec` in TLS 1.3, and `KeyUpdate`s
//! (each one a key derivation). A peer that sends them without end keeps a
//! connection spinning indefinitely while delivering nothing, and BoringSSL's
//! test suite (BoGo) checks that every stack stops it.
//!
//! The limits are BoringSSL's, and they are *consecutive* counts: real data
//! resets them, so a long-lived connection is not penalised for how many
//! `KeyUpdate`s or empty records it has seen in total, only for a run of them
//! with nothing real in between.

/// Consecutive empty records (and, in TLS 1.3, `ChangeCipherSpec` records).
pub const MAX_EMPTY_RECORDS: u32 = 32;
/// Warning alerts, counted separately from empty records so that neither can
/// hide behind the other.
pub const MAX_WARNING_ALERTS: u32 = 4;
/// Consecutive `KeyUpdate` messages.
pub const MAX_KEY_UPDATES: u32 = 32;

/// Which limit a peer exceeded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flood {
    /// More than [`MAX_EMPTY_RECORDS`] in a row.
    EmptyRecords,
    /// More than [`MAX_WARNING_ALERTS`] without real data between.
    WarningAlerts,
    /// More than [`MAX_KEY_UPDATES`] in a row.
    KeyUpdates,
}

impl core::fmt::Display for Flood {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::EmptyRecords => "too many empty records in a row",
            Self::WarningAlerts => "too many warning alerts",
            Self::KeyUpdates => "too many KeyUpdate messages in a row",
        })
    }
}

/// The running counts for one connection or handshake.
#[derive(Clone, Copy, Debug, Default)]
pub struct Noise {
    empty: u32,
    warnings: u32,
    key_updates: u32,
}

impl Noise {
    /// No noise seen yet; `const` so a `const fn` constructor can hold one.
    pub const fn new() -> Self {
        Self {
            empty: 0,
            warnings: 0,
            key_updates: 0,
        }
    }

    /// An empty record (or a TLS 1.3 `ChangeCipherSpec`) arrived.
    pub fn empty_record(&mut self) -> Result<(), Flood> {
        self.empty += 1;
        if self.empty > MAX_EMPTY_RECORDS {
            return Err(Flood::EmptyRecords);
        }
        Ok(())
    }

    /// A warning alert that is being ignored arrived.
    pub fn warning(&mut self) -> Result<(), Flood> {
        self.warnings += 1;
        if self.warnings > MAX_WARNING_ALERTS {
            return Err(Flood::WarningAlerts);
        }
        Ok(())
    }

    /// A `KeyUpdate` arrived.
    pub fn key_update(&mut self) -> Result<(), Flood> {
        self.key_updates += 1;
        if self.key_updates > MAX_KEY_UPDATES {
            return Err(Flood::KeyUpdates);
        }
        Ok(())
    }

    /// Real data arrived: a record that carried application bytes or
    /// handshake content. Resets every run.
    pub fn data(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_limit_allows_exactly_its_maximum_and_data_resets_it() {
        let mut noise = Noise::default();
        for _ in 0..MAX_EMPTY_RECORDS {
            assert_eq!(noise.empty_record(), Ok(()));
        }
        assert_eq!(noise.empty_record(), Err(Flood::EmptyRecords));

        let mut noise = Noise::default();
        for _ in 0..MAX_WARNING_ALERTS {
            assert_eq!(noise.warning(), Ok(()));
        }
        assert_eq!(noise.warning(), Err(Flood::WarningAlerts));

        let mut noise = Noise::default();
        for _ in 0..MAX_KEY_UPDATES {
            assert_eq!(noise.key_update(), Ok(()));
        }
        assert_eq!(noise.key_update(), Err(Flood::KeyUpdates));

        let mut noise = Noise::default();
        for _ in 0..MAX_EMPTY_RECORDS {
            noise.empty_record().expect("within the limit");
        }
        noise.data();
        assert_eq!(noise.empty_record(), Ok(()));
    }

    #[test]
    fn the_counters_are_independent() {
        let mut noise = Noise::default();
        for _ in 0..MAX_EMPTY_RECORDS {
            noise.empty_record().expect("within the limit");
        }
        for _ in 0..MAX_WARNING_ALERTS {
            noise.warning().expect("within the limit");
        }
        assert_eq!(noise.key_update(), Ok(()));
    }
}
