//! `cli_opt_status_fprint`'s export against C's, on a stream whose first write takes three bytes
//! and fails with `EINTR`, as a `write` a signal interrupts part way does: C's `fprintf` fails,
//! and `cli_opt_status_fprint` returns -1 having written those three bytes. The export, called
//! through its raw binding as C calls it, must do the same. Exits 1 when it does not.

use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr;
use std::process::ExitCode;

use libgit2::sys::git2 as sys;

/// What the stream's writes were given, and whether the next one fails.
struct Sink {
    bytes: Vec<u8>,
    fail: bool,
}

/// `cookie_io_functions_t`.
#[repr(C)]
struct CookieIo {
    read: Option<unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> isize>,
    write: Option<unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> isize>,
    seek: Option<unsafe extern "C" fn(*mut c_void, *mut i64, c_int) -> c_int>,
    close: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
}

unsafe extern "C" {
    fn fopencookie(cookie: *mut c_void, mode: *const c_char, io: CookieIo) -> *mut sys::FILE;
    fn setvbuf(stream: *mut sys::FILE, buf: *mut c_char, mode: c_int, size: usize) -> c_int;
    fn fclose(stream: *mut sys::FILE) -> c_int;
    fn __errno_location() -> *mut c_int;
    /// C's `cli_opt_status_fprint`, compiled under this name by `build.rs`.
    fn c_cli_opt_status_fprint(
        file: *mut sys::FILE,
        command: *const c_char,
        opt: *const sys::cli_opt,
    ) -> c_int;
}

const EINTR: c_int = 4;
const IONBF: c_int = 2;

unsafe extern "C" fn take(cookie: *mut c_void, buf: *const c_char, size: usize) -> isize {
    // SAFETY: the cookie is the `Sink` `run` gave the stream, which only the stream reaches
    // while it is open; `buf` holds `size` bytes.
    let (sink, bytes) = unsafe {
        (
            &mut *cookie.cast::<Sink>(),
            core::slice::from_raw_parts(buf.cast::<u8>(), size),
        )
    };
    let taken = if sink.fail { size.min(3) } else { size };
    sink.bytes.extend_from_slice(&bytes[..taken]);
    if taken < size {
        sink.fail = false;
        // SAFETY: the calling thread's `errno`.
        unsafe { *__errno_location() = EINTR };
    }
    taken as isize
}

/// Writes `opt` through `fprint` to a fresh interrupted stream: its return, `errno` after it,
/// and the bytes the stream took.
fn run(
    fprint: unsafe extern "C" fn(*mut sys::FILE, *const c_char, *const sys::cli_opt) -> c_int,
    command: &CStr,
    opt: &sys::cli_opt,
) -> (c_int, c_int, Vec<u8>) {
    let sink = Box::into_raw(Box::new(Sink {
        bytes: Vec::new(),
        fail: true,
    }));
    let io = CookieIo {
        read: None,
        write: Some(take),
        seek: None,
        close: None,
    };
    // SAFETY: the sink stays put until it is taken back below; the stream is closed first.
    unsafe {
        let file = fopencookie(sink.cast(), c"w".as_ptr(), io);
        assert!(!file.is_null());
        assert_eq!(setvbuf(file, ptr::null_mut(), IONBF, 0), 0);
        let rc = fprint(file, command.as_ptr(), opt);
        let errno = *__errno_location();
        fclose(file);
        (rc, errno, Box::from_raw(sink).bytes)
    }
}

fn main() -> ExitCode {
    let opt = sys::cli_opt {
        status: sys::CLI_OPT_STATUS_UNKNOWN_OPTION,
        arg: c"--nope".as_ptr().cast_mut(),
        ..Default::default()
    };
    let c = run(c_cli_opt_status_fprint, c"git2", &opt);
    let rust = run(sys::cli_opt_status_fprint, c"git2", &opt);
    let show = |(rc, errno, bytes): &(c_int, c_int, Vec<u8>)| {
        format!(
            "returned {rc}, errno {errno}, wrote {:?}",
            String::from_utf8_lossy(bytes)
        )
    };
    println!("C:      {}", show(&c));
    println!("export: {}", show(&rust));
    let failed = |r: &(c_int, c_int, Vec<u8>)| (r.0 < 0, r.1, r.2.clone());
    if failed(&c) == failed(&rust) {
        ExitCode::SUCCESS
    } else {
        println!("the export departs from C");
        ExitCode::FAILURE
    }
}
