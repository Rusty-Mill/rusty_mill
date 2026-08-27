//! `sysinfo(2)`: system-wide uptime, load averages, and memory/swap totals in
//! one syscall.
//!
//! No confirmed `rush` consumer need drove this; it was filed and implemented
//! at the user's explicit request during the parity-loop sweep against
//! `libc`, same "no known consumer need" bar Round 4 applied to
//! `flock`/`chroot`/`sendfile` before declining them (see `REVIEW.md`).

use crate::arch::nr;
use crate::arch::{from_ret, syscall1, Errno};

/// Shift applied to [`Sysinfo::loads`]'s fixed-point load-average values —
/// divide by `1 << SI_LOAD_SHIFT` (as a float) to get the familiar
/// `uptime`/`w`-style decimal load average.
pub const SI_LOAD_SHIFT: u32 = 16;

/// System-wide info (kernel `struct sysinfo`, 112 bytes on both 64-bit
/// targets this crate supports).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sysinfo {
    /// Seconds since boot.
    pub uptime: i64,
    /// 1, 5, and 15 minute load averages, as `SI_LOAD_SHIFT`-fixed-point
    /// values.
    pub loads: [u64; 3],
    /// Total usable main memory size, in [`Sysinfo::mem_unit`]-sized units.
    pub totalram: u64,
    /// Available memory size, in [`Sysinfo::mem_unit`]-sized units.
    pub freeram: u64,
    /// Amount of shared memory, in [`Sysinfo::mem_unit`]-sized units.
    pub sharedram: u64,
    /// Memory used by buffers, in [`Sysinfo::mem_unit`]-sized units.
    pub bufferram: u64,
    /// Total swap space size, in [`Sysinfo::mem_unit`]-sized units.
    pub totalswap: u64,
    /// Swap space still available, in [`Sysinfo::mem_unit`]-sized units.
    pub freeswap: u64,
    /// Number of current processes.
    pub procs: u16,
    __pad: u16,
    /// Total high memory size, in [`Sysinfo::mem_unit`]-sized units.
    pub totalhigh: u64,
    /// Available high memory size, in [`Sysinfo::mem_unit`]-sized units.
    pub freehigh: u64,
    /// Memory unit size in bytes — multiply the `*ram`/`*swap`/`*high`
    /// fields by this to get bytes.
    pub mem_unit: u32,
}

const _: () = assert!(core::mem::size_of::<Sysinfo>() == 112);
const _: () = assert!(core::mem::offset_of!(Sysinfo, loads) == 8);
const _: () = assert!(core::mem::offset_of!(Sysinfo, totalram) == 32);
const _: () = assert!(core::mem::offset_of!(Sysinfo, freeswap) == 72);
const _: () = assert!(core::mem::offset_of!(Sysinfo, procs) == 80);
const _: () = assert!(core::mem::offset_of!(Sysinfo, totalhigh) == 88);
const _: () = assert!(core::mem::offset_of!(Sysinfo, freehigh) == 96);
const _: () = assert!(core::mem::offset_of!(Sysinfo, mem_unit) == 104);

/// Fetch system-wide uptime, load averages, and memory/swap totals.
/// Cannot fail on Linux — `sysinfo(2)` has no documented error for a valid
/// out-pointer, which this call always supplies.
pub fn sysinfo() -> Result<Sysinfo, Errno> {
    let mut buf = Sysinfo::default();
    // SAFETY: `buf` is a valid, exclusively borrowed `struct sysinfo` the
    // kernel writes in place.
    let ret = unsafe { syscall1(nr::SYSINFO, &mut buf as *mut Sysinfo as usize) };
    from_ret(ret)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sysinfo_reports_sane_values() {
        let info = sysinfo().expect("sysinfo");
        assert!(info.uptime >= 0);
        assert!(info.totalram > 0);
        assert!(info.freeram <= info.totalram);
        assert!(info.mem_unit >= 1);
        assert!(info.procs >= 1);
    }

    #[test]
    fn load_averages_convert_to_a_reasonable_decimal() {
        let info = sysinfo().expect("sysinfo");
        for raw in info.loads {
            let load = raw as f64 / (1u64 << SI_LOAD_SHIFT) as f64;
            // A CI box under real load could exceed this, but a wildly
            // wrong shift/offset would blow way past it.
            assert!((0.0..1000.0).contains(&load));
        }
    }
}
