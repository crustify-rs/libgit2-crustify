//! Port of `src/util/rand.c`: libgit2's pseudo-random numbers, xoshiro256**
//! seeded through splitmix64 from the system's entropy (`getentropy`, this
//! configuration's `GIT_RAND_GETENTROPY`), falling back to the time and
//! process state.
//!
//! The state is a `std::sync::Mutex` (C's `git_mutex`); C never sees it.
//! The same seed gives the same sequence as the C.

use core::ffi::c_int;
use std::sync::Mutex;

use super::errors::{ErrorClass, Result};
use crate::git_error_set;

static STATE: Mutex<[u64; 4]> = Mutex::new([0; 4]);

/// splitmix64: expands the 64-bit seed into xoshiro's 256 bits.
#[inline]
fn splitmix64(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// xoshiro256** 1.0: the next number, advancing `s`.
#[inline]
fn xoshiro256ss(s: &mut [u64; 4]) -> u64 {
    let result = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
    let t = s[1] << 17;
    s[2] ^= s[0];
    s[3] ^= s[1];
    s[1] ^= s[2];
    s[0] ^= s[3];
    s[2] ^= t;
    s[3] = s[3].rotate_left(45);
    result
}

/// The state that `seed` gives.
fn seeded(seed: u64) -> [u64; 4] {
    let mut mixer = seed;
    [
        splitmix64(&mut mixer),
        splitmix64(&mut mixer),
        splitmix64(&mut mixer),
        splitmix64(&mut mixer),
    ]
}

/// `git_rand_seed`: restart the sequence from `seed`.
pub fn seed(seed: u64) {
    let s = seeded(seed);
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = s;
}

/// `git_rand_next`: the next number of the sequence.
pub fn next() -> u64 {
    xoshiro256ss(&mut STATE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// `getseed`: 64 bits from `getentropy`, else from the time and the
/// process's state (as the C's fallback).
#[cfg(not(miri))]
fn getseed() -> Result<u64> {
    let mut seed = [0u8; 8];
    // SAFETY: `seed` is writable for its 8 bytes.
    if unsafe { libc::getentropy(seed.as_mut_ptr().cast(), seed.len()) } == 0 {
        return Ok(u64::from_ne_bytes(seed));
    }

    let mut tv = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    // SAFETY: `tv` is writable; the time zone may be NULL.
    if unsafe { libc::gettimeofday(&raw mut tv, core::ptr::null_mut()) } < 0 {
        return Err(git_error_set!(
            ErrorClass::OS,
            "could get time for random seed"
        ));
    }
    let mut s: u64 = 0;
    s |= (tv.tv_usec as u64) << 40;
    s |= tv.tv_sec as u64;
    // `getpid()` and `getppid()` (as `u32`: the same bits for the positive
    // `pid_t`s they return).
    let pid = std::process::id();
    let ppid = std::os::unix::process::parent_id();
    // SAFETY: these process queries have no preconditions.
    let pgid = unsafe { libc::getpgid(0) };
    // SAFETY: as above.
    let sid = unsafe { libc::getsid(0) };
    // SAFETY: as above.
    let uid = unsafe { libc::getuid() };
    // SAFETY: as above.
    let gid = unsafe { libc::getgid() };
    s ^= (pid as u64) << 48;
    s ^= (ppid as u64) << 32;
    s ^= (pgid as u64) << 28;
    s ^= (sid as u64) << 16;
    s ^= (uid as u64) << 8;
    s ^= gid as u64;

    let mut load = [0f64; 3];
    // SAFETY: `load` is writable for 3 doubles.
    unsafe { libc::getloadavg(load.as_mut_ptr(), 3) };
    s ^= load[0].to_bits() >> 36;
    s ^= load[1].to_bits();
    s ^= load[2].to_bits() >> 16;

    s ^= super::util::time_monotonic();

    // Mix in the addresses of some variables.
    s ^= ((&raw const seed) as usize as u64) << 32;
    // SAFETY: `__errno_location` has no preconditions.
    s ^= unsafe { libc::__errno_location() } as usize as u64;
    Ok(s)
}

/// Under Miri (no entropy source nor clock under isolation), a constant.
#[cfg(miri)]
fn getseed() -> Result<u64> {
    Ok(0x9e37_79b9_7f4a_7c15)
}

/// `git_rand_global_init`: seed from the system.
pub fn global_init() -> Result<()> {
    let s = getseed()?;
    if s == 0 {
        return Err(git_error_set!(
            ErrorClass::INTERNAL,
            "failed to generate random seed"
        ));
    }
    seed(s);
    Ok(())
}

pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, status};

    /// `git_rand_global_init`.
    ///
    /// # Safety
    ///
    /// None (an init function of libgit2's runtime).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_rand_global_init() -> c_int {
        barrier(|| status(global_init()))
    }

    /// `git_rand_seed`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_rand_seed(s: u64) {
        barrier(|| seed(s))
    }

    /// `git_rand_next`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_rand_next() -> u64 {
        barrier(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_sequence() {
        // splitmix64 from 0, as the reference implementation gives it.
        let mut x = 0;
        assert_eq!(splitmix64(&mut x), 0xe220_a839_7b1d_cdaf);
        // The generator alone, on a fixed state (no global, so tests in
        // parallel do not interfere).
        let mut s = seeded(42);
        let a = xoshiro256ss(&mut s);
        let b = xoshiro256ss(&mut s);
        let mut s2 = seeded(42);
        assert_eq!((a, b), (xoshiro256ss(&mut s2), xoshiro256ss(&mut s2)));
        assert_ne!(a, b);
    }

    #[test]
    fn seeding() {
        assert!(global_init().is_ok());
        let _ = next();
    }
}
