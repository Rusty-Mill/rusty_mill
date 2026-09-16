//! Best-effort resolution of the process vDSO's `clock_gettime`, so
//! [`crate::time::clock_gettime`] can read the clock in userspace (no syscall
//! trap) the way glibc does.
//!
//! The vDSO is a small ELF image the kernel maps into every process; its base
//! address is published in the ELF auxiliary vector as `AT_SYSINFO_EHDR`. We
//! read that from `/proc/self/auxv` (using this crate's own syscalls — no
//! libc), parse the vDSO's dynamic symbol table, and look up the per-arch
//! `clock_gettime` entry. If anything is missing or looks wrong we return
//! `None` and the caller falls back to the raw syscall — so this is a pure
//! optimization, never a correctness dependency.

use crate::time::Timespec;
use core::sync::atomic::{AtomicUsize, Ordering};

/// vDSO `clock_gettime` ABI: `int f(clockid_t, struct timespec *)`, returning 0
/// on success or a negative value meaning "the vDSO can't serve this; use the
/// syscall".
pub(crate) type ClockGettimeFn = unsafe extern "C" fn(i32, *mut Timespec) -> i32;

#[cfg(target_arch = "x86_64")]
const SYMBOL: &[u8] = b"__vdso_clock_gettime";
#[cfg(target_arch = "aarch64")]
const SYMBOL: &[u8] = b"__kernel_clock_gettime";

/// Expected `e_machine` so we never trust a vDSO of the wrong architecture
/// (e.g. a host image leaking through an emulator).
#[cfg(target_arch = "x86_64")]
const E_MACHINE: u16 = 62; // EM_X86_64
#[cfg(target_arch = "aarch64")]
const E_MACHINE: u16 = 183; // EM_AARCH64

// Cache: 0 = not resolved yet, 1 = resolution failed (never a real code
// address), any other value = the resolved function pointer as `usize`.
static CACHE: AtomicUsize = AtomicUsize::new(0);
const FAILED: usize = 1;

/// The resolved vDSO `clock_gettime`, or `None` if this process has no usable
/// vDSO entry. Resolution is attempted once and cached; racing threads compute
/// the same pointer, so the unsynchronized retry is harmless.
///
/// `Relaxed` is sufficient on both the load and the store: the cached value is
/// a fixed vDSO code address the kernel already mapped and made executable
/// before this process ran any code, not a pointer to data this thread wrote
/// and needs to publish — there is nothing for an `Acquire`/`Release` pair to
/// synchronize here. Skipping the fence keeps every call on the hot path (this
/// is called on every [`crate::time::clock_gettime`]) a plain load.
#[inline]
pub(crate) fn clock_gettime_fn() -> Option<ClockGettimeFn> {
    let cached = CACHE.load(Ordering::Relaxed);
    if cached == FAILED {
        return None;
    }
    if cached != 0 {
        // SAFETY: a non-sentinel cache value was stored from a successful
        // resolve(), which only ever yields a valid ClockGettimeFn pointer.
        return Some(unsafe { core::mem::transmute::<usize, ClockGettimeFn>(cached) });
    }
    let resolved = resolve();
    CACHE.store(resolved.map_or(FAILED, |f| f as usize), Ordering::Relaxed);
    resolved
}

fn resolve() -> Option<ClockGettimeFn> {
    let base = vdso_base()?;
    // SAFETY: `base` is the kernel-published vDSO ELF image; `lookup` reads only
    // within it and validates the ELF header before trusting the layout.
    unsafe { lookup(base) }
}

/// Read `AT_SYSINFO_EHDR` (the vDSO base) from `/proc/self/auxv`.
fn vdso_base() -> Option<*const u8> {
    const AT_NULL: u64 = 0;
    const AT_SYSINFO_EHDR: u64 = 33;

    let fd = crate::fd::open(c"/proc/self/auxv", crate::fd::O_RDONLY, 0).ok()?;
    let mut buf = [0u8; 4096];
    let n = crate::fd::read_all(fd, &mut buf).ok()?;
    let _ = crate::fd::close(fd);

    let mut i = 0;
    while i + 16 <= n {
        let key = u64::from_ne_bytes(buf[i..i + 8].try_into().unwrap());
        let val = u64::from_ne_bytes(buf[i + 8..i + 16].try_into().unwrap());
        if key == AT_SYSINFO_EHDR {
            return if val == 0 {
                None
            } else {
                Some(val as *const u8)
            };
        }
        if key == AT_NULL {
            break;
        }
        i += 16;
    }
    None
}

// ELF64 constants used below.
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const DT_NULL: i64 = 0;
const DT_HASH: i64 = 4;
const DT_STRTAB: i64 = 5;
const DT_SYMTAB: i64 = 6;

#[inline]
unsafe fn rd_u16(p: *const u8, off: usize) -> u16 {
    unsafe { core::ptr::read_unaligned(p.add(off) as *const u16) }
}
#[inline]
unsafe fn rd_u32(p: *const u8, off: usize) -> u32 {
    unsafe { core::ptr::read_unaligned(p.add(off) as *const u32) }
}
#[inline]
unsafe fn rd_u64(p: *const u8, off: usize) -> u64 {
    unsafe { core::ptr::read_unaligned(p.add(off) as *const u64) }
}
#[inline]
unsafe fn rd_i64(p: *const u8, off: usize) -> i64 {
    unsafe { core::ptr::read_unaligned(p.add(off) as *const i64) }
}

/// Runtime address of virtual address `vaddr` in the loaded image.
#[inline]
fn reloc(base: *const u8, load_off: isize, vaddr: u64) -> *const u8 {
    base.wrapping_offset(load_off.wrapping_add(vaddr as isize))
}

/// Parse the vDSO ELF at `base` and return the `clock_gettime` entry.
///
/// # Safety
/// `base` must point at a valid, mapped ELF image (the kernel's vDSO).
unsafe fn lookup(base: *const u8) -> Option<ClockGettimeFn> {
    unsafe {
        // Validate the ELF header before trusting any offsets: magic, 64-bit
        // class, and matching machine.
        if rd_u32(base, 0) != u32::from_ne_bytes(*b"\x7fELF") {
            return None;
        }
        if *base.add(4) != 2 {
            return None; // not ELFCLASS64
        }
        if rd_u16(base, 18) != E_MACHINE {
            return None;
        }
        let e_phoff = rd_u64(base, 32) as usize;
        let e_phentsize = rd_u16(base, 54) as usize;
        let e_phnum = rd_u16(base, 56) as usize;
        if e_phentsize < 56 {
            return None; // sizeof(Elf64_Phdr)
        }

        // First PT_LOAD gives the load bias; PT_DYNAMIC locates the dynamic tbl.
        let mut load_off: Option<isize> = None;
        let mut dynamic: Option<*const u8> = None;
        for i in 0..e_phnum {
            let ph = base.add(e_phoff + i * e_phentsize);
            let p_type = rd_u32(ph, 0);
            let p_offset = rd_u64(ph, 8);
            let p_vaddr = rd_u64(ph, 16);
            if p_type == PT_LOAD && load_off.is_none() {
                load_off = Some((p_offset as isize).wrapping_sub(p_vaddr as isize));
            } else if p_type == PT_DYNAMIC {
                dynamic = Some(base.add(p_offset as usize));
            }
        }
        let load_off = load_off?;
        let mut d = dynamic?;

        // Dynamic table -> string table, symbol table, SysV hash table.
        let mut strtab: Option<*const u8> = None;
        let mut symtab: Option<*const u8> = None;
        let mut hash: Option<*const u8> = None;
        loop {
            let tag = rd_i64(d, 0);
            if tag == DT_NULL {
                break;
            }
            let val = rd_u64(d, 8);
            match tag {
                DT_STRTAB => strtab = Some(reloc(base, load_off, val)),
                DT_SYMTAB => symtab = Some(reloc(base, load_off, val)),
                DT_HASH => hash = Some(reloc(base, load_off, val)),
                _ => {}
            }
            d = d.add(16); // sizeof(Elf64_Dyn)
        }
        let strtab = strtab?;
        let symtab = symtab?;
        // The SysV hash table's second word (`nchain`) is the symbol count.
        let nsyms = rd_u32(hash?, 4) as usize;

        // Linear scan — the vDSO exports only a handful of symbols.
        for i in 0..nsyms {
            let sym = symtab.add(i * 24); // sizeof(Elf64_Sym)
            let st_name = rd_u32(sym, 0) as usize;
            let st_value = rd_u64(sym, 8);
            if st_name == 0 || st_value == 0 {
                continue;
            }
            if cstr_eq(strtab.add(st_name), SYMBOL) {
                let addr = reloc(base, load_off, st_value);
                return Some(core::mem::transmute::<*const u8, ClockGettimeFn>(addr));
            }
        }
        None
    }
}

/// True if the NUL-terminated C string at `p` equals `target` (which contains
/// no NUL).
///
/// # Safety
/// `p` must point at a NUL-terminated byte sequence readable through its NUL.
unsafe fn cstr_eq(p: *const u8, target: &[u8]) -> bool {
    unsafe {
        let mut i = 0;
        while i < target.len() {
            if *p.add(i) != target[i] {
                return false;
            }
            i += 1;
        }
        *p.add(target.len()) == 0
    }
}
