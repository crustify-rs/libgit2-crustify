//! Port of `src/util/errors.c`: the thread-local error state, and the error
//! types of the Rust port (PORTING.md §4.4).
//!
//! The C model is kept: a failure is a non-zero `int` code
//! ([`ErrorCode`]; `GIT_ERROR` = -1, `GIT_ENOTFOUND`, …), and its message and
//! class live in the calling thread's error state, which `git_error_last`
//! returns. A Rust function returns [`Result<T>`], whose error is the `Copy`
//! code, with the message already set in the thread state. The safe Rust API
//! turns that into a rich [`Error`] (code, class, message) at its boundary.
//!
//! Setting an error:
//!
//! ```ignore
//! use libgit2_rs::__test::util::errors::{self, ErrorClass, ErrorCode, Result};
//! use libgit2_rs::git_error_set;
//!
//! fn parse(s: &[u8]) -> Result<u32> {
//!     if s.is_empty() {
//!         // `git_error_set(GIT_ERROR_INVALID, "empty input of %d bytes", 0)`
//!         return Err(git_error_set!(ErrorClass::INVALID, "empty input of {} bytes", 0));
//!     }
//!     if s[0] == b'x' {
//!         // Paths and other bytes go in byte for byte, UTF-8 or not:
//!         return Err(git_error_set!(ErrorClass::INVALID, "bad '{}'", errors::Bytes(s)));
//!     }
//!     Ok(s.len() as u32)
//! }
//! assert_eq!(parse(b""), Err(ErrorCode::GENERIC));
//! assert_eq!(errors::last_message().unwrap(), b"empty input of 0 bytes");
//! ```
//!
//! # Where the state lives
//!
//! C kept it in a `git_tlsdata` slot allocated with `git__allocator`; the
//! port keeps it in a Rust `thread_local!`, in Rust's allocator (see
//! `rust/DIVERGENCES.md`): it is reached only through this module, and C only
//! reads the `git_error` that `git_error_last` returns, until the thread's
//! next error call, as before.

use core::cell::{Cell, RefCell, UnsafeCell};
use core::ffi::{CStr, c_char, c_int};
use core::fmt;

use super::runtime;

/// The result type of the port's internal functions: the error is the
/// [`ErrorCode`], whose message is in the thread's error state.
pub type Result<T, E = ErrorCode> = core::result::Result<T, E>;

// ---------------------------------------------------------------------------
// Codes and classes
// ---------------------------------------------------------------------------

/// A libgit2 status code that is not success (`git_error_code`, or another
/// non-zero `int` a callback returned).
///
/// `Copy`, one `int` wide: the message that goes with it is in the thread's
/// error state, as in C.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
#[must_use]
pub struct ErrorCode(c_int);

macro_rules! codes {
    ($($(#[$m:meta])* $name:ident = $v:expr, $c:literal;)*) => {
        impl ErrorCode {
            $($(#[$m])* #[doc = concat!("`", $c, "`")] pub const $name: ErrorCode = ErrorCode($v);)*

            /// The C name of a known code.
            pub const fn name(self) -> Option<&'static str> {
                match self.0 {
                    $($v => Some($c),)*
                    _ => None,
                }
            }
        }

        impl crate::api::error::ErrorCode {
            $($(#[$m])* #[doc = concat!("`", $c, "`")] pub const $name: Self = Self($v);)*

            /// The C name of a known code (`"GIT_ENOTFOUND"`).
            pub const fn name(self) -> Option<&'static str> {
                match self.0 {
                    $($v => Some($c),)*
                    _ => None,
                }
            }
        }
    };
}

codes! {
    GENERIC = -1, "GIT_ERROR";
    NOT_FOUND = -3, "GIT_ENOTFOUND";
    EXISTS = -4, "GIT_EEXISTS";
    AMBIGUOUS = -5, "GIT_EAMBIGUOUS";
    BUFS = -6, "GIT_EBUFS";
    USER = -7, "GIT_EUSER";
    BARE_REPO = -8, "GIT_EBAREREPO";
    UNBORN_BRANCH = -9, "GIT_EUNBORNBRANCH";
    UNMERGED = -10, "GIT_EUNMERGED";
    NON_FAST_FORWARD = -11, "GIT_ENONFASTFORWARD";
    INVALID_SPEC = -12, "GIT_EINVALIDSPEC";
    CONFLICT = -13, "GIT_ECONFLICT";
    LOCKED = -14, "GIT_ELOCKED";
    MODIFIED = -15, "GIT_EMODIFIED";
    AUTH = -16, "GIT_EAUTH";
    CERTIFICATE = -17, "GIT_ECERTIFICATE";
    APPLIED = -18, "GIT_EAPPLIED";
    PEEL = -19, "GIT_EPEEL";
    EOF = -20, "GIT_EEOF";
    INVALID = -21, "GIT_EINVALID";
    UNCOMMITTED = -22, "GIT_EUNCOMMITTED";
    DIRECTORY = -23, "GIT_EDIRECTORY";
    MERGE_CONFLICT = -24, "GIT_EMERGECONFLICT";
    PASSTHROUGH = -30, "GIT_PASSTHROUGH";
    ITER_OVER = -31, "GIT_ITEROVER";
    RETRY = -32, "GIT_RETRY";
    MISMATCH = -33, "GIT_EMISMATCH";
    INDEX_DIRTY = -34, "GIT_EINDEXDIRTY";
    APPLY_FAIL = -35, "GIT_EAPPLYFAIL";
    OWNER = -36, "GIT_EOWNER";
    TIMEOUT = -37, "GIT_TIMEOUT";
    UNCHANGED = -38, "GIT_EUNCHANGED";
    NOT_SUPPORTED = -39, "GIT_ENOTSUPPORTED";
    READ_ONLY = -40, "GIT_EREADONLY";
}

impl ErrorCode {
    /// The code for a non-zero C status; `None` for 0 (success).
    #[inline]
    pub const fn from_raw(code: c_int) -> Option<ErrorCode> {
        if code == 0 {
            None
        } else {
            Some(ErrorCode(code))
        }
    }

    /// The C status.
    #[inline]
    pub const fn raw(self) -> c_int {
        self.0
    }

    /// A C status as a `Result`: `Ok(code)` for a non-negative one, the code
    /// as the error for a negative one (libgit2's usual convention).
    #[inline]
    pub const fn check(code: c_int) -> Result<c_int> {
        if code < 0 {
            Err(ErrorCode(code))
        } else {
            Ok(code)
        }
    }

    /// A C status as a `Result`: `Ok(())` for 0, the code as the error for
    /// any other value (the convention of callbacks, where a positive value
    /// also stops the operation).
    #[inline]
    pub const fn check_zero(code: c_int) -> Result<()> {
        if code == 0 {
            Ok(())
        } else {
            Err(ErrorCode(code))
        }
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

/// An error class (`git_error_t`). Any `int` is representable, since C
/// callers may set any class through `git_error_set_str`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ErrorClass(pub c_int);

macro_rules! classes {
    ($($name:ident = $v:expr, $c:literal;)*) => {
        impl ErrorClass {
            $(#[doc = concat!("`", $c, "`")] pub const $name: ErrorClass = ErrorClass($v);)*

            /// The C name of a known class.
            pub const fn name(self) -> Option<&'static str> {
                match self.0 {
                    $($v => Some($c),)*
                    _ => None,
                }
            }
        }

        impl crate::api::error::ErrorClass {
            $(#[doc = concat!("`", $c, "`")] pub const $name: Self = Self($v);)*

            /// The C name of a known class (`"GIT_ERROR_ODB"`).
            pub const fn name(self) -> Option<&'static str> {
                match self.0 {
                    $($v => Some($c),)*
                    _ => None,
                }
            }
        }
    };
}

classes! {
    NONE = 0, "GIT_ERROR_NONE";
    NOMEMORY = 1, "GIT_ERROR_NOMEMORY";
    OS = 2, "GIT_ERROR_OS";
    INVALID = 3, "GIT_ERROR_INVALID";
    REFERENCE = 4, "GIT_ERROR_REFERENCE";
    ZLIB = 5, "GIT_ERROR_ZLIB";
    REPOSITORY = 6, "GIT_ERROR_REPOSITORY";
    CONFIG = 7, "GIT_ERROR_CONFIG";
    REGEX = 8, "GIT_ERROR_REGEX";
    ODB = 9, "GIT_ERROR_ODB";
    INDEX = 10, "GIT_ERROR_INDEX";
    OBJECT = 11, "GIT_ERROR_OBJECT";
    NET = 12, "GIT_ERROR_NET";
    TAG = 13, "GIT_ERROR_TAG";
    TREE = 14, "GIT_ERROR_TREE";
    INDEXER = 15, "GIT_ERROR_INDEXER";
    SSL = 16, "GIT_ERROR_SSL";
    SUBMODULE = 17, "GIT_ERROR_SUBMODULE";
    THREAD = 18, "GIT_ERROR_THREAD";
    STASH = 19, "GIT_ERROR_STASH";
    CHECKOUT = 20, "GIT_ERROR_CHECKOUT";
    FETCHHEAD = 21, "GIT_ERROR_FETCHHEAD";
    MERGE = 22, "GIT_ERROR_MERGE";
    SSH = 23, "GIT_ERROR_SSH";
    FILTER = 24, "GIT_ERROR_FILTER";
    REVERT = 25, "GIT_ERROR_REVERT";
    CALLBACK = 26, "GIT_ERROR_CALLBACK";
    CHERRYPICK = 27, "GIT_ERROR_CHERRYPICK";
    DESCRIBE = 28, "GIT_ERROR_DESCRIBE";
    REBASE = 29, "GIT_ERROR_REBASE";
    FILESYSTEM = 30, "GIT_ERROR_FILESYSTEM";
    PATCH = 31, "GIT_ERROR_PATCH";
    WORKTREE = 32, "GIT_ERROR_WORKTREE";
    SHA = 33, "GIT_ERROR_SHA";
    HTTP = 34, "GIT_ERROR_HTTP";
    INTERNAL = 35, "GIT_ERROR_INTERNAL";
    GRAFTS = 36, "GIT_ERROR_GRAFTS";
}

impl fmt::Debug for ErrorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => f.write_str(n),
            None => write!(f, "ErrorClass({})", self.0),
        }
    }
}

// ---------------------------------------------------------------------------
// The C layout, and the static errors
// ---------------------------------------------------------------------------

/// C's `git_error` (`include/git2/errors.h`): mirrored (kind 3).
#[repr(C)]
#[derive(Debug)]
pub struct GitError {
    /// The message, NUL-terminated.
    pub message: *mut c_char,
    /// The class (`git_error_t`).
    pub klass: c_int,
}

/// A `git_error` in static memory, which C reads and never writes.
#[repr(transparent)]
pub struct StaticError(GitError);

// SAFETY: a `StaticError` is immutable: its message points to a string
// literal and nothing writes either field (C receives it as `const
// git_error *`), so sharing it between threads is sharing read-only data.
unsafe impl Sync for StaticError {}

impl StaticError {
    const fn new(message: &'static CStr, klass: ErrorClass) -> StaticError {
        StaticError(GitError {
            message: message.as_ptr().cast_mut(),
            klass: klass.0,
        })
    }

    fn as_ptr(&'static self) -> *const GitError {
        &raw const self.0
    }

    fn message(&'static self) -> &'static CStr {
        // SAFETY: `message` was made from a `&'static CStr` in `new` and is
        // never written.
        unsafe { CStr::from_ptr(self.0.message) }
    }
}

static OOM_ERROR: StaticError = StaticError::new(c"Out of memory", ErrorClass::NOMEMORY);
static UNINITIALIZED_ERROR: StaticError =
    StaticError::new(c"library has not been initialized", ErrorClass::INVALID);
static TLSDATA_ERROR: StaticError = StaticError::new(
    c"thread-local data initialization failure",
    ErrorClass::THREAD,
);
static NO_ERROR: StaticError = StaticError::new(c"no error", ErrorClass::NONE);

fn static_error(p: *const GitError) -> Option<&'static StaticError> {
    [&OOM_ERROR, &UNINITIALIZED_ERROR, &TLSDATA_ERROR, &NO_ERROR]
        .into_iter()
        .find(|e| core::ptr::eq(e.as_ptr(), p))
}

// ---------------------------------------------------------------------------
// The thread state
// ---------------------------------------------------------------------------

/// What `last` points at in C.
#[derive(Clone, Copy)]
enum Last {
    /// `NULL`: no error.
    None,
    /// `&threadstate->error`: the message buffer.
    Own,
    /// One of the static errors.
    Static(&'static StaticError),
}

struct ThreadState {
    /// The message, NUL-terminated whenever `last` is `Own`.
    message: RefCell<Vec<u8>>,
    /// The `git_error` handed to C. Written only inside this module's calls;
    /// C reads it between them, on this thread.
    error: UnsafeCell<GitError>,
    last: Cell<Last>,
    /// Set while a message is being formatted into `message` (see [`Bytes`]).
    formatting: Cell<bool>,
}

thread_local! {
    static STATE: ThreadState = const {
        ThreadState {
            message: RefCell::new(Vec::new()),
            error: UnsafeCell::new(GitError { message: core::ptr::null_mut(), klass: 0 }),
            last: Cell::new(Last::None),
            formatting: Cell::new(false),
        }
    };
}

/// Run `f` on this thread's state; `None` once the thread's locals are
/// being destroyed (C's "thread-local data initialization failure").
#[inline]
fn with_state<R>(f: impl FnOnce(&ThreadState) -> R) -> Option<R> {
    STATE.try_with(f).ok()
}

impl ThreadState {
    /// Point `last` at the message buffer, with class `class`.
    fn commit_own(&self, class: c_int, message: &mut Vec<u8>) {
        message.push(0);
        let error = self.error.get();
        // SAFETY: `error` is this thread's; no reference to it exists (C
        // only reads it between calls, and this module forms none), so the
        // write does not alias.
        unsafe {
            error.write(GitError {
                message: message.as_mut_ptr().cast(),
                klass: class,
            })
        };
        self.last.set(Last::Own);
    }

    /// Whether `p` points into this thread's message buffer (so that a C
    /// caller passed `git_error_last()->message` back in).
    fn owns_message_ptr(&self, p: *const c_char) -> bool {
        match self.message.try_borrow() {
            Ok(m) => {
                let start = m.as_ptr() as usize;
                m.capacity() > 0 && (p as usize) >= start && (p as usize) < start + m.capacity()
            }
            // Mutably borrowed: we are inside a formatting call.
            Err(_) => false,
        }
    }

    /// Replace the message with what `fill` writes, as class `class`; on
    /// allocation failure the error becomes the out-of-memory one.
    fn set_with(&self, class: c_int, fill: impl FnOnce(&mut Vec<u8>) -> bool) {
        if self.formatting.get() {
            // Re-entered from an argument's `Display`: the outer call sets it.
            return;
        }
        let Ok(mut message) = self.message.try_borrow_mut() else {
            // Re-entered while formatting a message: the outer call sets it.
            return;
        };
        // Consistent for C while the buffer changes: no error until commit.
        self.last.set(Last::None);
        message.clear();
        if fill(&mut message) && message.try_reserve(1).is_ok() {
            self.commit_own(class, &mut message);
        } else {
            message.clear();
            self.last.set(Last::Static(&OOM_ERROR));
        }
    }
}

/// Append `bytes` to `v`, failing (instead of aborting) when the allocation
/// fails.
#[inline]
fn try_extend(v: &mut Vec<u8>, bytes: &[u8]) -> bool {
    if v.try_reserve(bytes.len()).is_err() {
        return false;
    }
    v.extend_from_slice(bytes);
    true
}

// ---------------------------------------------------------------------------
// Setting errors
// ---------------------------------------------------------------------------

/// Set this thread's error to `message` with class `class`
/// (`git_error_set_str`), and return [`ErrorCode::GENERIC`].
pub fn set_str(class: ErrorClass, message: impl AsRef<[u8]>) -> ErrorCode {
    set_bytes_raw(class.0, message.as_ref());
    ErrorCode::GENERIC
}

fn set_bytes_raw(class: c_int, message: &[u8]) {
    with_state(|s| s.set_with(class, |m| try_extend(m, message)));
}

/// Set this thread's error from formatting arguments, and return
/// [`ErrorCode::GENERIC`]. Use [`git_error_set!`](crate::git_error_set).
///
/// For class [`ErrorClass::OS`], `": "` and the message of the current
/// `errno` follow (when `errno` is not 0, which is then reset to 0), as C's
/// `git_error_set` does.
pub fn set_fmt(class: ErrorClass, args: fmt::Arguments<'_>) -> ErrorCode {
    let os_errno = if class == ErrorClass::OS { errno() } else { 0 };
    set_fmt_raw(class.0, Some(args), os_errno);
    ErrorCode::GENERIC
}

/// Writes formatted pieces into the thread's message buffer.
struct MessageWriter<'a>(&'a ThreadState);

impl fmt::Write for MessageWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let Ok(mut m) = self.0.message.try_borrow_mut() else {
            return Err(fmt::Error);
        };
        if try_extend(&mut m, s.as_bytes()) {
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

fn set_fmt_raw(class: c_int, args: Option<fmt::Arguments<'_>>, os_errno: c_int) {
    with_state(|s| {
        if s.formatting.get() {
            // Re-entered from an argument's `Display`: leave the outer one be.
            return;
        }
        match s.message.try_borrow_mut() {
            Ok(mut m) => {
                // While the buffer is rewritten (and an argument's `Display`
                // may panic), C sees no error rather than a stale pointer.
                s.last.set(Last::None);
                m.clear();
            }
            Err(_) => return,
        }
        let mut ok = true;
        if let Some(args) = args {
            /// Resets the flag even if an argument's `Display` panics.
            struct Formatting<'a>(&'a Cell<bool>);
            impl Drop for Formatting<'_> {
                fn drop(&mut self) {
                    self.0.set(false);
                }
            }
            s.formatting.set(true);
            let _flag = Formatting(&s.formatting);
            ok = fmt::write(&mut MessageWriter(s), args).is_ok();
        }
        let Ok(mut m) = s.message.try_borrow_mut() else {
            return;
        };
        if ok && class == ErrorClass::OS.0 {
            if args.is_some() {
                ok = try_extend(&mut m, b": ");
            }
            if ok && os_errno != 0 {
                ok = push_strerror(&mut m, os_errno);
                set_errno(0);
            }
        }
        if ok && m.try_reserve(1).is_ok() {
            s.commit_own(class, &mut m);
        } else {
            m.clear();
            s.last.set(Last::Static(&OOM_ERROR));
        }
    });
}

/// Append `strerror(code)`.
fn push_strerror(m: &mut Vec<u8>, code: c_int) -> bool {
    let mut buf = [0u8; 256];
    // SAFETY: `buf` is valid for writes of its length; XSI `strerror_r`
    // writes a NUL-terminated message of at most that many bytes.
    let r = unsafe { libc::strerror_r(code, buf.as_mut_ptr().cast(), buf.len()) };
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    if r != 0 && len == 0 {
        return try_extend(m, format!("Unknown error {code}").as_bytes());
    }
    try_extend(m, &buf[..len])
}

/// Bytes to format into an error message byte for byte, whether or not they
/// are UTF-8, as C's `%s` does with a path.
///
/// Inside [`git_error_set!`](crate::git_error_set) the bytes go into the
/// message unchanged (width and precision flags are ignored). Displayed
/// anywhere else, invalid UTF-8 is replaced, as by `String::from_utf8_lossy`.
#[derive(Clone, Copy)]
pub struct Bytes<'a>(pub &'a [u8]);

impl fmt::Display for Bytes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Ok(s) = core::str::from_utf8(self.0) {
            return f.write_str(s);
        }
        let raw = with_state(|s| {
            if !s.formatting.get() {
                return None;
            }
            let mut m = s.message.try_borrow_mut().ok()?;
            Some(try_extend(&mut m, self.0))
        })
        .flatten();
        match raw {
            Some(true) => Ok(()),
            Some(false) => Err(fmt::Error),
            None => f.write_str(&String::from_utf8_lossy(self.0)),
        }
    }
}

impl fmt::Debug for Bytes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&String::from_utf8_lossy(self.0), f)
    }
}

/// Set the thread's error to the out-of-memory one (`git_error_set_oom`),
/// and return [`ErrorCode::GENERIC`], as `GIT_ERROR_CHECK_ALLOC` returns -1.
#[inline]
pub fn oom() -> ErrorCode {
    set_oom();
    ErrorCode::GENERIC
}

/// Set the thread's error to the out-of-memory one (`git_error_set_oom`).
pub fn set_oom() {
    with_state(|s| s.last.set(Last::Static(&OOM_ERROR)));
}

/// Clear the thread's error (`git_error_clear`), and `errno`.
pub fn clear() {
    with_state(|s| {
        if !matches!(s.last.get(), Last::None) {
            s.set_with(0, |_| true);
            s.last.set(Last::None);
        }
    });
    set_errno(0);
}

/// Whether the thread has an error set (`git_error_exists`).
pub fn exists() -> bool {
    with_state(|s| !matches!(s.last.get(), Last::None)).unwrap_or(true)
}

/// The thread's error class and message, as `git_error_last` would give
/// them in an initialized library (`no error` with class 0 when there is
/// none).
pub fn last() -> (ErrorClass, Vec<u8>) {
    let r = with_state(|s| match s.last.get() {
        Last::None => (ErrorClass::NONE, NO_ERROR.message().to_bytes().to_vec()),
        Last::Static(e) => (ErrorClass(e.0.klass), e.message().to_bytes().to_vec()),
        Last::Own => {
            let m = s.message.borrow();
            // SAFETY: `error` is this thread's and only this module writes it,
            // which it is not doing now.
            let klass = unsafe { (*s.error.get()).klass };
            (ErrorClass(klass), m[..m.len().saturating_sub(1)].to_vec())
        }
    });
    r.unwrap_or_else(|| {
        (
            ErrorClass(TLSDATA_ERROR.0.klass),
            TLSDATA_ERROR.message().to_bytes().to_vec(),
        )
    })
}

/// The thread's error message, or `None` when no error is set.
pub fn last_message() -> Option<Vec<u8>> {
    if !exists() {
        return None;
    }
    Some(last().1)
}

/// The thread's error class, or `None` when no error is set.
pub fn last_class() -> Option<ErrorClass> {
    if !exists() {
        return None;
    }
    Some(last().0)
}

/// Set the error for an argument check that failed (`GIT_ASSERT_ARG`):
/// `invalid argument: '<expr>'`, class `GIT_ERROR_INVALID`.
#[cold]
pub fn assert_arg_failed(expr: &str) -> ErrorCode {
    set_fmt(
        ErrorClass::INVALID,
        format_args!("invalid argument: '{expr}'"),
    )
}

/// Set the error for an internal check that failed (`GIT_ASSERT`):
/// `unrecoverable internal error: '<expr>'`, class `GIT_ERROR_INTERNAL`.
#[cold]
pub fn assert_failed(expr: &str) -> ErrorCode {
    set_fmt(
        ErrorClass::INTERNAL,
        format_args!("unrecoverable internal error: '{expr}'"),
    )
}

/// Set the error after a callback returned non-zero
/// (`git_error_set_after_callback_function`): if no error is set, set
/// `"<action> callback returned <code>"` with class `GIT_ERROR_CALLBACK`.
/// Returns `code`.
pub fn set_after_callback(code: c_int, action: &str) -> c_int {
    if code != 0 && !exists() {
        let _ = set_fmt(
            ErrorClass::CALLBACK,
            format_args!("{action} callback returned {code}"),
        );
    }
    code
}

#[cold]
pub(crate) fn set_panic_error() {
    let _ = set_str(
        ErrorClass::INTERNAL,
        "unrecoverable internal error: a Rust panic was caught",
    );
}

/// `errno` (`git_error_system_last`).
#[inline]
pub fn errno() -> c_int {
    // SAFETY: `__errno_location` has no preconditions.
    let p = unsafe { libc::__errno_location() };
    // SAFETY: it returns this thread's `errno`, valid for the thread's
    // lifetime.
    unsafe { *p }
}

/// Set `errno` (`git_error_system_set`).
#[inline]
pub fn set_errno(code: c_int) {
    // SAFETY: `__errno_location` has no preconditions.
    let p = unsafe { libc::__errno_location() };
    // SAFETY: it returns this thread's `errno`, valid and aligned for the
    // thread's lifetime, which only this thread accesses.
    unsafe { *p = code };
}

// ---------------------------------------------------------------------------
// Saving and restoring
// ---------------------------------------------------------------------------

/// The thread's error, saved to restore later (`git_error_save`).
pub struct SavedError(Saved);

enum Saved {
    None,
    Static(&'static StaticError),
    Own(c_int, Vec<u8>),
}

/// Save the thread's error, to [`restore`] it later. The state is left as
/// it is.
pub fn save() -> SavedError {
    let s = with_state(|s| match s.last.get() {
        Last::None => Saved::None,
        Last::Static(e) => Saved::Static(e),
        Last::Own => {
            let m = s.message.borrow();
            // SAFETY: `error` is this thread's (a thread-local) and only this
            // module writes it, which it is not doing now: the read does not
            // race or alias a write.
            let klass = unsafe { (*s.error.get()).klass };
            let bytes = &m[..m.len().saturating_sub(1)];
            let mut v = Vec::new();
            if v.try_reserve_exact(bytes.len()).is_err() {
                // As C's `git_error_save` when its copy fails.
                return Saved::Static(&OOM_ERROR);
            }
            v.extend_from_slice(bytes);
            Saved::Own(klass, v)
        }
    });
    SavedError(s.unwrap_or(Saved::Static(&TLSDATA_ERROR)))
}

/// Make `saved` the thread's error again (`git_error_restore`).
pub fn restore(saved: SavedError) {
    match saved.0 {
        Saved::None => {
            with_state(|s| s.last.set(Last::Static(&NO_ERROR)));
        }
        Saved::Static(e) => {
            with_state(|s| s.last.set(Last::Static(e)));
        }
        Saved::Own(klass, msg) => set_bytes_raw(klass, &msg),
    }
}

/// Clear the calling thread's state at library shutdown (C's
/// `git_error_global_shutdown`, which freed this thread's state).
extern "C" fn global_shutdown() {
    with_state(|s| {
        if let Ok(mut m) = s.message.try_borrow_mut() {
            *m = Vec::new();
        }
        s.last.set(Last::None);
    });
}

/// `git_error_global_init`: register the shutdown of the error state.
pub fn global_init() -> Result<()> {
    runtime::shutdown_register(global_shutdown)
}

// ---------------------------------------------------------------------------
// The rich error of the safe API
// ---------------------------------------------------------------------------

/// The safe API's error (`crate::api::error`), named here for the
/// internals that build one.
pub use crate::api::error::Error;

// ---------------------------------------------------------------------------
// Macros
// ---------------------------------------------------------------------------

/// Set the thread's error from a format string (`git_error_set`), and
/// evaluate to [`ErrorCode::GENERIC`](crate::util::errors::ErrorCode::GENERIC).
///
/// Format with Rust's syntax; pass byte strings that must go in unchanged
/// (paths) as [`Bytes`](crate::util::errors::Bytes). For
/// [`ErrorClass::OS`](crate::util::errors::ErrorClass::OS) the `errno`
/// message is appended as C does.
///
/// ```ignore
/// // (crate-internal: the unit tests run it)
/// # use libgit2_rs::{git_error_set, util::errors::{ErrorClass, Result}};
/// fn f(n: usize) -> Result<()> {
///     Err(git_error_set!(ErrorClass::INVALID, "too many: {n}"))
/// }
/// ```
#[macro_export]
#[doc(hidden)] // crate-internal: expands to crate-private paths, unusable outside
macro_rules! git_error_set {
    ($class:expr, $($arg:tt)+) => {
        $crate::util::errors::set_fmt($class, ::core::format_args!($($arg)+))
    };
}

/// `GIT_ASSERT_ARG(expr)`: if `cond` is false, set `invalid argument:
/// '<text>'` (class `GIT_ERROR_INVALID`) and return.
///
/// `text` is the C expression, as the C's message spells it. With two
/// arguments the function returns `Err(ErrorCode::GENERIC)`; with a third
/// it returns that value (`GIT_ASSERT_ARG_WITH_RETVAL`).
///
/// ```ignore
/// // (crate-internal: the unit tests run it)
/// # use libgit2_rs::{git_assert_arg, util::errors::Result};
/// fn f(p: Option<&u8>) -> Result<u8> {
///     git_assert_arg!(p.is_some(), "p");
///     Ok(*p.unwrap())
/// }
/// ```
#[macro_export]
#[doc(hidden)] // crate-internal: expands to crate-private paths, unusable outside
macro_rules! git_assert_arg {
    ($cond:expr, $text:literal) => {
        if !$cond {
            return ::core::result::Result::Err($crate::util::errors::assert_arg_failed($text));
        }
    };
    ($cond:expr, $text:literal, $ret:expr) => {
        if !$cond {
            let _ = $crate::util::errors::assert_arg_failed($text);
            return $ret;
        }
    };
}

/// `GIT_ASSERT(expr)`: if `cond` is false, set `unrecoverable internal
/// error: '<text>'` (class `GIT_ERROR_INTERNAL`) and return, as
/// [`git_assert_arg!`](crate::git_assert_arg).
#[macro_export]
#[doc(hidden)] // crate-internal: expands to crate-private paths, unusable outside
macro_rules! git_assert {
    ($cond:expr, $text:literal) => {
        if !$cond {
            return ::core::result::Result::Err($crate::util::errors::assert_failed($text));
        }
    };
    ($cond:expr, $text:literal, $ret:expr) => {
        if !$cond {
            let _ = $crate::util::errors::assert_failed($text);
            return $ret;
        }
    };
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `errors.c`. `git_error_set` and `git_error_vset` are
/// variadic: they are C shims in `src/util/rust_shims.c`, which format with
/// `vsnprintf` and call [`git_error__set_formatted`].
pub mod ffi {
    use super::*;
    use crate::util::alloc;
    use crate::util::cabi::Out;
    use crate::util::export::{barrier, cstr};

    /// Hand C the `git_error` to show: the thread's, or a static one.
    fn last_ptr() -> *const GitError {
        if runtime::init_count() == 0 {
            return UNINITIALIZED_ERROR.as_ptr();
        }
        with_state(|s| match s.last.get() {
            Last::None => NO_ERROR.as_ptr(),
            Last::Static(e) => e.as_ptr(),
            Last::Own => s.error.get().cast_const(),
        })
        .unwrap_or_else(|| TLSDATA_ERROR.as_ptr())
    }

    /// `git_error_last`: the last error on this thread.
    ///
    /// # Safety
    ///
    /// None to call. The pointee stays valid, and unchanged, until the
    /// thread's next call into libgit2 that may set or clear an error; the
    /// caller must not write it.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_last() -> *const GitError {
        barrier(last_ptr)
    }

    /// `git_error_clear`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_clear() {
        barrier(clear)
    }

    /// `git_error_set_str`.
    ///
    /// # Safety
    ///
    /// `string` is NULL or a NUL-terminated string, readable for the call;
    /// it may be this thread's `git_error_last()->message` (so it stays a
    /// raw pointer: the call writes that buffer).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_set_str(error_class: c_int, string: *const c_char) -> c_int {
        barrier(|| {
            if string.is_null() {
                let _ = assert_arg_failed("string");
                return -1;
            }
            let ok = with_state(|st| {
                if st.owns_message_ptr(string) {
                    // `git_error_last()->message` passed back: C clears its
                    // buffer first (`ptr[0] = '\0'`), so the message
                    // becomes empty. No reference to the buffer is formed.
                    st.set_with(error_class, |_| true);
                } else {
                    // SAFETY: `string` is non-null (checked) and
                    // NUL-terminated (this export's contract), and not this
                    // thread's message buffer (checked), so nothing writes
                    // it while it is borrowed.
                    let s = unsafe { CStr::from_ptr(string) };
                    st.set_with(error_class, |m| try_extend(m, s.to_bytes()));
                }
                matches!(st.last.get(), Last::Own)
            });
            if ok == Some(true) { 0 } else { -1 }
        })
    }

    /// `git_error_set_oom`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_set_oom() {
        barrier(set_oom)
    }

    /// The Rust half of the `git_error_vset` shim: set the error to the
    /// `len` bytes at `message` (already formatted; NULL when the format was
    /// NULL) with class `error_class`; for `GIT_ERROR_OS`, append `": "` (if
    /// there was a format) and `strerror(os_errno)` (if `os_errno` is not 0,
    /// resetting `errno` to 0), as C's `git_error_vset` does.
    ///
    /// # Safety
    ///
    /// `message` is NULL, or valid for reads of `len` bytes for the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error__set_formatted(
        error_class: c_int,
        message: *const c_char,
        len: usize,
        os_errno: c_int,
    ) {
        barrier(|| {
            if message.is_null() {
                set_fmt_raw(error_class, None, os_errno);
            } else {
                // SAFETY: the caller passes `len` readable bytes at `message`.
                let bytes = unsafe { crate::util::export::bytes(message, len) };
                set_fmt_raw(
                    error_class,
                    Some(format_args!("{}", Bytes(bytes))),
                    os_errno,
                );
            }
        })
    }

    /// `git_error_exists`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_exists() -> bool {
        barrier(exists)
    }

    /// `git_error_global_init`.
    ///
    /// # Safety
    ///
    /// Called from library initialization, under the runtime's lock.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_global_init() -> c_int {
        barrier(|| crate::util::export::status(global_init()))
    }

    /// `git_error_system_last`: `errno`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_system_last() -> c_int {
        errno()
    }

    /// `git_error_system_set`: set `errno`.
    ///
    /// # Safety
    ///
    /// None.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_system_set(code: c_int) {
        set_errno(code)
    }

    /// `git_error_save`: a copy of the thread's error, in `git__allocator`'s
    /// memory, or one of the static errors.
    ///
    /// # Safety
    ///
    /// `out` is valid for a write of a pointer. The error stored there is
    /// given to `git_error_restore` or `git_error_free` exactly once.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_save(mut out: Out<'_, *mut GitError>) -> c_int {
        barrier(|| {
            let (ptr, ret) = save_c();
            out.write(ptr.cast_mut());
            ret
        })
    }

    fn save_c() -> (*const GitError, c_int) {
        let saved = with_state(|s| match s.last.get() {
            Last::None => Err(NO_ERROR.as_ptr()),
            Last::Static(e) => Err(e.as_ptr()),
            Last::Own => {
                // SAFETY: `error` is this thread's (a thread-local) and only this
                // module writes it, which it is not doing now: the read does
                // not race or alias a write.
                let klass = unsafe { (*s.error.get()).klass };
                // The copy goes straight into `git__allocator`'s memory
                // (fallible, as C's `git__strdup`); setting the OOM error
                // only touches `last`, not the borrowed buffer.
                let m = s.message.borrow();
                Ok((klass, alloc::substrdup(&m[..m.len().saturating_sub(1)])))
            }
        });
        let (klass, copy) = match saved {
            None => return (TLSDATA_ERROR.as_ptr(), -1),
            Some(Err(p)) => return (p, 0),
            Some(Ok(v)) => v,
        };
        if copy.is_null() {
            return (OOM_ERROR.as_ptr(), -1);
        }
        let dup = alloc::malloc(core::mem::size_of::<GitError>()).cast::<GitError>();
        if dup.is_null() {
            // SAFETY: `copy` was just allocated by `git__allocator` and is
            // not used again. (C leaks here.)
            unsafe { alloc::free(copy.cast()) };
            return (OOM_ERROR.as_ptr(), -1);
        }
        // SAFETY: `dup` is a fresh allocation of a `GitError`, aligned by the
        // allocator's guarantee (8 bytes).
        unsafe {
            dup.write(GitError {
                message: copy.cast(),
                klass,
            })
        };
        (dup, 0)
    }

    /// `git_error_restore`: make `error` the thread's error, and free it.
    ///
    /// # Safety
    ///
    /// `error` is NULL or a value `git_error_save` returned and that was not
    /// yet restored or freed.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_restore(error: *mut GitError) -> c_int {
        barrier(|| {
            if error.is_null() {
                let _ = assert_arg_failed("error");
                return -1;
            }
            if let Some(st) = static_error(error)
                && with_state(|s| s.last.set(Last::Static(st))).is_some()
            {
                return 0;
            }
            // SAFETY: `error` is non-null (checked), not static (checked),
            // so a live `git_error` from `git_error_save` (this export's
            // contract): aligned and initialized.
            let GitError { message, klass } = unsafe { error.read() };
            if with_state(|s| s.owns_message_ptr(message)) == Some(true) {
                // Not a saved copy (outside the contract): C would empty the
                // message; do so without borrowing the buffer.
                with_state(|s| s.set_with(klass, |_| true));
            } else {
                // SAFETY: a saved error's message is NULL or NUL-terminated,
                // in its own allocation (`save_c`), which nothing writes
                // until it is freed below.
                match unsafe { cstr(message) } {
                    Some(m) => set_bytes_raw(klass, m.to_bytes()),
                    None => {
                        with_state(|s| s.set_with(klass, |_| true));
                    }
                }
            }
            // SAFETY: `error` is a live, non-static saved error, freed once
            // here and not used after (this export's contract).
            unsafe { free_saved(error) };
            0
        })
    }

    /// Free a saved error and its message.
    ///
    /// # Safety
    ///
    /// `error` is a value `git_error_save` returned (not a static error)
    /// that was not yet restored or freed, and is not used after.
    unsafe fn free_saved(error: *mut GitError) {
        // SAFETY: a live `git_error` from `git_error_save` (this fn's
        // contract): aligned and initialized.
        let message = unsafe { (*error).message };
        // SAFETY: `git_error_save` allocated the message with
        // `git__allocator`; it is freed once.
        unsafe { alloc::free(message.cast()) };
        // SAFETY: and the struct, likewise, after its last read.
        unsafe { alloc::free(error.cast()) };
    }

    /// `git_error_free`.
    ///
    /// # Safety
    ///
    /// `error` is NULL, a static error, or a value `git_error_save`
    /// returned that was not yet restored or freed.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_error_free(error: *mut GitError) {
        barrier(|| {
            if error.is_null() || static_error(error).is_some() {
                return;
            }
            // SAFETY: `error` is neither NULL nor static (checked), so a
            // value of `git_error_save` not yet restored or freed, given up
            // by the caller (this export's contract).
            unsafe { free_saved(error) };
        })
    }
}

crate::c_layout_test! {
    headers: ["git2/errors.h"],
    types: [
        GitError => "git_error" { message, klass },
        StaticError => "git_error" {},
    ],
    asserts: ["GIT_ERROR_GRAFTS == 36", "GIT_EREADONLY == -40"],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last_pair() -> (c_int, Vec<u8>) {
        let (class, message) = last();
        (class.0, message)
    }

    #[test]
    fn assert_macros() {
        let _g = crate::util::test_support::init();
        fn f(x: Option<u8>) -> Result<u8> {
            git_assert_arg!(x.is_some(), "x");
            git_assert!(x != Some(0), "x != 0");
            Ok(x.unwrap())
        }
        assert_eq!(f(None), Err(ErrorCode::GENERIC));
        assert_eq!(last_pair(), (3, b"invalid argument: 'x'".to_vec()));
        assert_eq!(f(Some(0)), Err(ErrorCode::GENERIC));
        assert_eq!(
            last_pair(),
            (35, b"unrecoverable internal error: 'x != 0'".to_vec())
        );
        assert_eq!(f(Some(3)), Ok(3));
        fn g(x: c_int) -> c_int {
            git_assert_arg!(x > 0, "x > 0", -7);
            0
        }
        assert_eq!(g(0), -7);
    }
}
