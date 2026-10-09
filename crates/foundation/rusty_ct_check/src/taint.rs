//! Valgrind client requests that mark memory secret (undefined) or public
//! (defined). Natively these are no-ops; they only act under `valgrind`.
//! Implemented for x86-64 only; on other targets every function does nothing,
//! so a taint run there silently checks nothing: see [`SUPPORTED`].
//!
//! Pitfall: if the optimiser knows a secret's value (a constant in the test),
//! it may fold the load away and memcheck sees nothing. Pass test secrets
//! through `core::hint::black_box` before use.

/// Whether this build can actually taint memory for valgrind.
pub const SUPPORTED: bool = cfg!(target_arch = "x86_64");

const MAKE_MEM_UNDEFINED: usize = 0x4d43_0001;
const MAKE_MEM_DEFINED: usize = 0x4d43_0002;

/// Marks `bytes` as secret: under memcheck, a branch or memory address that
/// depends on them is reported.
pub fn mark_secret(bytes: &[u8]) {
    request(MAKE_MEM_UNDEFINED, bytes);
}

/// Marks `bytes` public again. Call on a value that is *meant* to be revealed
/// (a ciphertext, a signature) before branching on it.
pub fn declassify(bytes: &[u8]) {
    request(MAKE_MEM_DEFINED, bytes);
}

#[cfg(target_arch = "x86_64")]
#[allow(unsafe_code)]
fn request(code: usize, bytes: &[u8]) {
    let args: [usize; 6] = [code, bytes.as_ptr() as usize, bytes.len(), 0, 0, 0];
    let mut result: usize = 0;
    // SAFETY: the instruction sequence is valgrind's documented client-request
    // preamble; it rotates `rdi` by 64 bits in total and exchanges `rbx` with
    // itself, so it has no architectural effect outside valgrind. `args` is a
    // live local array whose address is passed in `rax`.
    unsafe {
        core::arch::asm!(
            "rol rdi, 3", "rol rdi, 13", "rol rdi, 61", "rol rdi, 51",
            "xchg rbx, rbx",
            inout("rdx") result,
            in("rax") args.as_ptr(),
            options(nostack),
        );
    }
    core::hint::black_box(result);
}

#[cfg(not(target_arch = "x86_64"))]
fn request(_code: usize, _bytes: &[u8]) {}
