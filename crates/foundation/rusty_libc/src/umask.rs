//! File-mode creation mask.

use crate::arch::nr;
use crate::arch::syscall1;

/// Set the file-mode creation mask to `mask`, returning the previous mask.
///
/// `umask(2)` cannot fail, so there is no `Result`.
pub fn umask(mask: u32) -> u32 {
    // SAFETY: single integer argument; the syscall always succeeds.
    unsafe { syscall1(nr::UMASK, mask as usize) as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_restore() {
        let original = umask(0o022);
        // Setting again returns what we just set.
        let prev = umask(original);
        assert_eq!(prev, 0o022);
        // And we are back to the original mask.
        let now = umask(original);
        assert_eq!(now, original);
    }
}
