//! The error of the safe API: [`Error`] (its [`ErrorCode`], [`ErrorClass`]
//! and message), the one error type of every public `Result`.
//!
//! The port's internals keep C's model (a `Copy` code, the message in the
//! thread's error state; `crate::util::errors`). The safe API captures the
//! class and message into an `Error` where the internal error is made into
//! one, at its boundary, so an `Error` never depends on the thread's state
//! afterwards. The internal code type is not public, so no public
//! conversion reads the thread's state.

use std::borrow::Cow;
use std::fmt;

use crate::util::errors as internal;

/// A libgit2 error code (`git_error_code`): what kind of failure, for
/// matching (`e.code() == ErrorCode::NOT_FOUND`).
///
/// The constants are libgit2's codes. Codes a C callback or backend
/// returned that libgit2 does not name are kept as they were.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErrorCode(pub(crate) i32);

/// An error class (`git_error_t`): which subsystem failed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ErrorClass(pub(crate) i32);

impl ErrorCode {
    pub(crate) const fn from_internal(c: internal::ErrorCode) -> ErrorCode {
        ErrorCode(c.raw())
    }

    /// The internal code (never 0).
    pub(crate) const fn internal(self) -> internal::ErrorCode {
        match internal::ErrorCode::from_raw(self.0) {
            Some(c) => c,
            None => internal::ErrorCode::GENERIC,
        }
    }
}

impl ErrorClass {
    pub(crate) const fn from_internal(c: internal::ErrorClass) -> ErrorClass {
        ErrorClass(c.0)
    }

    pub(crate) const fn internal(self) -> internal::ErrorClass {
        internal::ErrorClass(self.0)
    }
}

impl From<internal::ErrorCode> for ErrorCode {
    fn from(c: internal::ErrorCode) -> ErrorCode {
        ErrorCode::from_internal(c)
    }
}

impl From<internal::ErrorClass> for ErrorClass {
    fn from(c: internal::ErrorClass) -> ErrorClass {
        ErrorClass::from_internal(c)
    }
}

impl fmt::Debug for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => write!(f, "{n}({})", self.0),
            None => write!(f, "ErrorCode({})", self.0),
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl fmt::Debug for ErrorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "ErrorClass({})", self.0),
        }
    }
}

/// An error of the safe API: the code, the class and the message (bytes,
/// as libgit2 wrote them; paths in them need not be UTF-8).
#[derive(Clone, PartialEq, Eq)]
pub struct Error {
    code: ErrorCode,
    class: ErrorClass,
    message: Box<[u8]>,
}

impl Error {
    /// An error from its parts (for backends and callbacks that fail).
    pub fn new(code: ErrorCode, class: ErrorClass, message: impl Into<Vec<u8>>) -> Error {
        Error {
            code,
            class,
            message: message.into().into_boxed_slice(),
        }
    }

    /// An error from the port's internal parts.
    pub(crate) fn with(
        code: internal::ErrorCode,
        class: internal::ErrorClass,
        message: impl Into<Vec<u8>>,
    ) -> Error {
        Error::new(code.into(), class.into(), message)
    }

    /// The internal error `code` with the class and message of this
    /// thread's error state, captured now (the state is left as it is).
    pub(crate) fn last(code: internal::ErrorCode) -> Error {
        let (class, message) = internal::last();
        Error::with(code, class, message)
    }

    /// The error with which a Rust panic was caught (class `INTERNAL`).
    pub(crate) fn panicked() -> Error {
        Error::with(
            internal::ErrorCode::GENERIC,
            internal::ErrorClass::INTERNAL,
            "unrecoverable internal error: a Rust panic was caught",
        )
    }

    /// The code.
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// The class.
    pub fn class(&self) -> ErrorClass {
        self.class
    }

    /// The message, as bytes.
    pub fn message_bytes(&self) -> &[u8] {
        &self.message
    }

    /// The message, with invalid UTF-8 replaced.
    pub fn message(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.message)
    }

    /// The internal code (for the C side of a backend or callback).
    pub(crate) fn raw_code(&self) -> internal::ErrorCode {
        self.code.internal()
    }

    /// The internal class.
    pub(crate) fn raw_class(&self) -> internal::ErrorClass {
        self.class.internal()
    }

    /// The code to hand to C for an error that user code (a backend or a
    /// callback) returned: a failure status, never success and never
    /// `GIT_ITEROVER` (which would read as "done" to C's loops; S-6g).
    pub(crate) fn user_code(&self) -> internal::ErrorCode {
        if self.code.0 >= 0 || self.code == ErrorCode::ITER_OVER {
            internal::ErrorCode::GENERIC
        } else {
            self.raw_code()
        }
    }

    /// Make this error, which user code returned, the thread's error (class
    /// and message) and return its [`user_code`](Self::user_code), as C
    /// reports a failure.
    pub(crate) fn set_last(&self) -> internal::ErrorCode {
        let _ = internal::set_str(self.raw_class(), self.message_bytes());
        self.user_code()
    }
}

/// Internal results (`?` in the safe API): the message is captured from the
/// thread's state when the conversion runs, which is where the internal
/// error is made. The internal code type is crate-private, so this
/// conversion is not reachable from outside the crate.
impl From<internal::ErrorCode> for Error {
    fn from(code: internal::ErrorCode) -> Error {
        Error::last(code)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error")
            .field("code", &self.code)
            .field("class", &self.class)
            .field("message", &self.message())
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for Error {}

/// The result of the safe API.
pub type Result<T, E = Error> = core::result::Result<T, E>;
