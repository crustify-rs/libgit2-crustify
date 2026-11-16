//! Cross-cutting parts of the safe Rust API (PORTING.md §5): the library's
//! global options ([`opts`]).

pub mod error;
#[macro_use]
pub(crate) mod flags;
pub mod opts;

/// A user value (a backend) whose drop never unwinds: a panic in its
/// `Drop` is caught and its payload leaked (S-3). C releases backends from
/// `free` slots, which cannot unwind (a panic there would abort).
pub(crate) struct NoUnwindDrop<T>(core::mem::ManuallyDrop<T>);

impl<T> NoUnwindDrop<T> {
    pub(crate) fn new(v: T) -> NoUnwindDrop<T> {
        NoUnwindDrop(core::mem::ManuallyDrop::new(v))
    }
}

impl<T> core::ops::Deref for NoUnwindDrop<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> Drop for NoUnwindDrop<T> {
    fn drop(&mut self) {
        // SAFETY: the value is taken once, here, and never used after (the
        // wrapper is being dropped).
        let v = unsafe { core::mem::ManuallyDrop::take(&mut self.0) };
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(v))) {
            // The payload's own drop could panic again: leak it.
            core::mem::forget(payload);
        }
    }
}
