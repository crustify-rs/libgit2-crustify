//! The system PCRE2 (`libpcre2-8`), which [`crate::util::regexp`] binds
//! (this configuration's `GIT_REGEX_PCRE2`): compiled patterns and match
//! data as ffibox owners over PCRE2's own allocation and free routines.

use core::ffi::{CStr, c_int, c_void};

use ffibox::{CBox, CRefSeam, define_ctype, impl_cdrop};

/// `PCRE2_CASELESS`.
pub const PCRE2_CASELESS: u32 = 0x0000_0008;
/// `PCRE2_ERROR_NOMATCH`.
pub const PCRE2_ERROR_NOMATCH: c_int = -1;
/// `PCRE2_UNSET`.
pub const PCRE2_UNSET: usize = usize::MAX;

/// The C types, opaque.
pub mod raw {
    /// `pcre2_code` (8-bit).
    #[repr(C)]
    pub struct pcre2_code_8 {
        _opaque: [u8; 0],
    }
    /// `pcre2_match_data` (8-bit).
    #[repr(C)]
    pub struct pcre2_match_data_8 {
        _opaque: [u8; 0],
    }
}
use raw::{pcre2_code_8, pcre2_match_data_8};

unsafe extern "C" {
    fn pcre2_compile_8(
        pattern: *const u8,
        length: usize,
        options: u32,
        errorcode: *mut c_int,
        erroroffset: *mut usize,
        ccontext: *mut c_void,
    ) -> *mut pcre2_code_8;
    fn pcre2_code_free_8(code: *mut pcre2_code_8);
    fn pcre2_get_error_message_8(errorcode: c_int, buffer: *mut u8, bufflen: usize) -> c_int;
    fn pcre2_match_data_create_8(ovecsize: u32, gcontext: *mut c_void) -> *mut pcre2_match_data_8;
    fn pcre2_match_data_free_8(match_data: *mut pcre2_match_data_8);
    fn pcre2_match_8(
        code: *const pcre2_code_8,
        subject: *const u8,
        length: usize,
        startoffset: usize,
        options: u32,
        match_data: *mut pcre2_match_data_8,
        mcontext: *mut c_void,
    ) -> c_int;
    fn pcre2_get_ovector_pointer_8(match_data: *mut pcre2_match_data_8) -> *mut usize;
    fn pcre2_get_ovector_count_8(match_data: *mut pcre2_match_data_8) -> u32;
}

define_ctype!(Code, CodeRef, CodeMut, pcre2_code_8);
define_ctype!(MatchData, MatchDataRef, MatchDataMut, pcre2_match_data_8);

// SAFETY: a compiled pattern is immutable once compiled; PCRE2 documents
// matching with one pattern from several threads at once (each with its
// own match data) as safe, and freeing it from any thread.
unsafe impl Send for Code {}
// SAFETY: as above: shared access only matches.
unsafe impl Sync for Code {}
// SAFETY: match data belongs to one owner, and PCRE2 frees it from any
// thread.
unsafe impl Send for MatchData {}

/// The policy: `pcre2_code_free`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CodeFree;
impl_cdrop!(CodeFree, Code, pcre2_code_free_8);

/// The policy: `pcre2_match_data_free`.
#[derive(Clone, Copy, Debug, Default)]
pub struct MatchDataFree;
impl_cdrop!(MatchDataFree, MatchData, pcre2_match_data_free_8);

/// A compiled pattern.
pub type CodeBox = CBox<Code, CodeFree>;
/// Match data.
pub type MatchDataBox = CBox<MatchData, MatchDataFree>;

/// `PCRE2_ZERO_TERMINATED`.
const PCRE2_ZERO_TERMINATED: usize = usize::MAX;

/// `pcre2_compile(pattern, PCRE2_ZERO_TERMINATED, options, …)`: the
/// compiled pattern, or PCRE2's error code. `None` is a NULL pattern, which
/// PCRE2 rejects with an error of its own.
pub fn compile(pattern: Option<&CStr>, options: u32) -> Result<CodeBox, c_int> {
    let mut error: c_int = 0;
    let mut offset: usize = 0;
    let ptr = pattern.map_or(core::ptr::null(), |p| p.as_ptr().cast::<u8>());
    // SAFETY: `ptr` is NULL (which PCRE2 checks for) or a NUL-terminated
    // pattern, readable for the call (PCRE2 copies what it needs into the
    // compiled code); the out-parameters are locals; a NULL context selects
    // the defaults.
    let code = unsafe {
        pcre2_compile_8(
            ptr,
            PCRE2_ZERO_TERMINATED,
            options,
            &raw mut error,
            &raw mut offset,
            core::ptr::null_mut(),
        )
    };
    // SAFETY: `pcre2_compile` returns an owned pattern (freed with
    // `pcre2_code_free`) or NULL.
    unsafe { CodeBox::from_c(code) }.ok_or(error)
}

/// `pcre2_get_error_message` into `buf`: the message (truncated to fit;
/// PCRE2 NUL-terminates it), without its NUL.
pub fn error_message(error: c_int, buf: &mut [u8]) -> &[u8] {
    // SAFETY: `buf` is writable for its length.
    unsafe { pcre2_get_error_message_8(error, buf.as_mut_ptr(), buf.len()) };
    CStr::from_bytes_until_nul(buf).map_or(&[], CStr::to_bytes)
}

/// `pcre2_match_data_create(pairs, NULL)`; `None` when PCRE2 cannot
/// allocate.
pub fn match_data_create(pairs: u32) -> Option<MatchDataBox> {
    // SAFETY: a NULL context selects `malloc`.
    let md = unsafe { pcre2_match_data_create_8(pairs, core::ptr::null_mut()) };
    // SAFETY: an owned match data block (freed with
    // `pcre2_match_data_free`) or NULL.
    unsafe { MatchDataBox::from_c(md) }
}

/// `pcre2_match(code, subject, len, 0, 0, md, NULL)`: the number of pairs
/// set (0 when the match data is too small), or a negative error.
pub fn pcre2_match(code: CodeRef<'_>, subject: &[u8], md: &mut MatchDataBox) -> c_int {
    // SAFETY: `code` is a live compiled pattern (the handle's borrow);
    // `subject` is readable for its length; `md` is exclusively ours for
    // the call; PCRE2 keeps neither pointer.
    unsafe {
        pcre2_match_8(
            code.as_ptr(),
            subject.as_ptr(),
            subject.len(),
            0,
            0,
            md.as_c_ptr(),
            core::ptr::null_mut(),
        )
    }
}

/// The ovector of `md`: two offsets per pair.
pub fn ovector(md: &MatchDataBox) -> &[usize] {
    let p = md.as_c_ptr();
    // SAFETY: `md` is a live match data block; the accessor only reads it.
    let ptr = unsafe { pcre2_get_ovector_pointer_8(p) };
    // SAFETY: as above.
    let pairs = unsafe { pcre2_get_ovector_count_8(p) };
    // SAFETY: PCRE2's ovector has `2 * pairs` offsets, owned by `md` and
    // written only by `pcre2_match`, which needs `&mut md`.
    unsafe { core::slice::from_raw_parts(ptr, 2 * pairs as usize) }
}

/// Borrow a compiled pattern C holds.
///
/// # Safety
///
/// A non-null `code` is a live compiled pattern that is not freed for
/// `'a`.
pub unsafe fn code_ref<'a>(code: *mut pcre2_code_8) -> Option<CodeRef<'a>> {
    // SAFETY: the caller's contract.
    unsafe { CodeRef::from_ptr(code) }
}
