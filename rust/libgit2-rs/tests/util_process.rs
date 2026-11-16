//! Tests of `unix/process.c`'s port through the exported C ABI. They fork
//! and exec, and resolve programs with `git_fs_path_find_executable` (C
//! until its WP lands), so they need `link-c` and do not run under Miri.

#![cfg(all(feature = "link-c", not(miri)))]
#![allow(clippy::multiple_unsafe_ops_per_block, clippy::borrow_as_ptr)]

use core::ffi::{c_char, c_void};

use ffibox::{CBitfield, CBits};

use libgit2_rs::util::errors::{self, ErrorClass};
use libgit2_rs::util::process::{
    GitProcess, GitProcessOptions, GitProcessResult, ffi, flags, status,
};
use libgit2_rs::util::str::GitStr;
use libgit2_rs::util::test_support;

fn msg() -> String {
    String::from_utf8(errors::last_message().unwrap_or_default()).unwrap()
}

/// Options with the bitfield fields `f` set.
fn opts(f: &[CBits<u32>]) -> GitProcessOptions {
    let mut flags = CBitfield::new(0);
    for &b in f {
        flags.set_flag(b, true);
    }
    GitProcessOptions {
        flags,
        cwd: core::ptr::null_mut(),
    }
}

/// Spawn `args` (C strings), feed `input`, collect stdout, wait.
fn run(
    args: &[&core::ffi::CStr],
    env: &[&core::ffi::CStr],
    mut o: GitProcessOptions,
    input: &[u8],
) -> (Vec<u8>, GitProcessResult) {
    let argv: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
    let envv: Vec<*const c_char> = env.iter().map(|a| a.as_ptr()).collect();
    let mut p: *mut GitProcess = core::ptr::null_mut();
    let mut out = Vec::new();
    let mut r = GitProcessResult::default();
    // SAFETY: valid arrays, options and buffers, as process.h requires.
    unsafe {
        assert_eq!(
            ffi::git_process_new(
                &mut p,
                argv.as_ptr(),
                argv.len(),
                envv.as_ptr(),
                envv.len(),
                &mut o
            ),
            0
        );
        assert_eq!(ffi::git_process_start(p), 0, "{}", msg());
        if o.flags.is_set(flags::CAPTURE_IN) {
            assert_eq!(
                ffi::git_process_write(p, input.as_ptr().cast(), input.len()),
                input.len() as isize
            );
            assert_eq!(ffi::git_process_close_in(p), 0);
        }
        let mut buf = [0u8; 64];
        loop {
            let n = ffi::git_process_read(p, buf.as_mut_ptr().cast(), buf.len());
            assert!(n >= 0);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        assert_eq!(ffi::git_process_wait(&mut r, p), 0);
        ffi::git_process_close(p);
        ffi::git_process_free(p);
    }
    (out, r)
}

#[test]
fn exit_codes_and_pipes() {
    let _g = test_support::init();
    let _l = fd_lock();
    let (_, r) = run(&[c"/bin/false"], &[], opts(&[flags::CAPTURE_OUT]), b"");
    assert_eq!(
        r,
        GitProcessResult {
            status: status::NORMAL,
            exitcode: 1,
            signal: 0
        }
    );
    let (out, r) = run(
        &[c"cat"],
        &[],
        opts(&[flags::CAPTURE_IN, flags::CAPTURE_OUT]),
        b"Hello, world.\r\n",
    );
    assert_eq!(out, b"Hello, world.\r\n");
    assert_eq!(r.exitcode, 0);
    let (out, _) = run(
        &[c"echo \"$0:$1\"", c"a b", c"c"],
        &[],
        opts(&[flags::USE_SHELL, flags::CAPTURE_OUT]),
        b"",
    );
    // `sh -c '<args[0]> "$@"' <args[0]> <args...>`: $0 is args[0]
    assert_eq!(out, b"echo \"$0:$1\":a b a b c\n");
    let (_, r) = run(
        &[c"kill -TERM $$"],
        &[],
        opts(&[flags::USE_SHELL, flags::CAPTURE_OUT]),
        b"",
    );
    assert_eq!(
        r,
        GitProcessResult {
            status: status::ERROR,
            exitcode: 0,
            signal: libc::SIGTERM
        }
    );
}

#[test]
fn environment() {
    let _g = test_support::init();
    let _l = fd_lock();
    let (out, _) = run(
        &[c"/usr/bin/env"],
        &[c"U4_A=1", c"U4_GONE=", c"PATH=/bin"],
        opts(&[flags::CAPTURE_OUT, flags::EXCLUDE_ENV]),
        b"",
    );
    assert_eq!(out, b"U4_A=1\nPATH=/bin\n");
    let (out, _) = run(
        &[c"/usr/bin/env"],
        &[],
        opts(&[flags::CAPTURE_OUT, flags::EXCLUDE_ENV]),
        b"",
    );
    assert_eq!(out, b"");
}

#[test]
fn start_failures() {
    let _g = test_support::init();
    let _l = fd_lock();
    let mut p: *mut GitProcess = core::ptr::null_mut();
    let args = [c"/a/b/z/y/not_found".as_ptr()];
    let mut o = opts(&[flags::CAPTURE_OUT]);
    // SAFETY: valid arguments.
    unsafe {
        assert_eq!(
            ffi::git_process_new(&mut p, args.as_ptr(), 1, core::ptr::null(), 0, &mut o),
            0
        );
        assert_eq!(ffi::git_process_start(p), -1);
        assert_eq!(
            msg(),
            "cannot run /a/b/z/y/not_found: No such file or directory"
        );
        assert_eq!(errors::last_class(), Some(ErrorClass::SSH));
        ffi::git_process_free(p);

        let pwd = [c"/bin/pwd".as_ptr()];
        let mut o = opts(&[flags::CAPTURE_OUT]);
        o.cwd = c"/no/such/dir".as_ptr().cast_mut();
        assert_eq!(
            ffi::git_process_new(&mut p, pwd.as_ptr(), 1, core::ptr::null(), 0, &mut o),
            0
        );
        assert_eq!(ffi::git_process_start(p), -1);
        assert_eq!(msg(), "could not chdir: No such file or directory");
        assert_eq!(errors::last_class(), Some(ErrorClass::OS));
        // the pipes were closed, and are not named any more
        assert_eq!(ffi::git_process_close(p), 0);
        let mut pid = 0;
        assert_eq!(ffi::git_process_id(&mut pid, p), -1);
        assert_eq!(msg(), "process not running");
        ffi::git_process_free(p);
    }
}

#[test]
fn argument_checks_and_messages() {
    let _g = test_support::init();
    let _l = fd_lock();
    let mut p: *mut GitProcess = core::ptr::null_mut();
    let args = [c"/bin/true".as_ptr()];
    let mut c = 0u8;
    // SAFETY: valid (or deliberately NULL) arguments.
    unsafe {
        assert_eq!(
            ffi::git_process_new(
                &mut p,
                args.as_ptr(),
                0,
                core::ptr::null(),
                0,
                core::ptr::null_mut()
            ),
            -1
        );
        assert_eq!(msg(), "invalid argument: 'out && args && args_len > 0'");
        assert!(p.is_null());
        assert_eq!(
            ffi::git_process_new(
                &mut p,
                args.as_ptr(),
                1,
                core::ptr::null(),
                0,
                core::ptr::null_mut()
            ),
            0
        );
        assert_eq!(
            ffi::git_process_read(p, (&raw mut c).cast::<c_void>(), 1),
            -1
        );
        assert_eq!(
            msg(),
            "unrecoverable internal error: 'process->capture_out'"
        );
        assert_eq!(
            ffi::git_process_write(p, (&raw const c).cast::<c_void>(), 1),
            -1
        );
        assert_eq!(msg(), "unrecoverable internal error: 'process->capture_in'");
        assert_eq!(
            ffi::git_process_read(core::ptr::null_mut(), (&raw mut c).cast::<c_void>(), 1),
            -1
        );
        assert_eq!(msg(), "invalid argument: 'process'");
        assert_eq!(ffi::git_process_close_err(p), -1);
        assert_eq!(msg(), "error is not open");
        let mut r = GitProcessResult {
            status: 9,
            exitcode: 9,
            signal: 9,
        };
        assert_eq!(ffi::git_process_wait(&mut r, p), -1);
        assert_eq!(msg(), "process is stopped");
        assert_eq!(r, GitProcessResult::default());
        ffi::git_process_free(p);
    }

    for (r, m) in [
        (
            GitProcessResult {
                status: status::NONE,
                exitcode: 0,
                signal: 0,
            },
            "process not started",
        ),
        (
            GitProcessResult {
                status: status::NORMAL,
                exitcode: 3,
                signal: 0,
            },
            "process exited with code 3",
        ),
        (
            GitProcessResult {
                status: status::ERROR,
                exitcode: 0,
                signal: 9,
            },
            "process exited on signal 9",
        ),
        (
            GitProcessResult {
                status: status::ERROR,
                exitcode: 0,
                signal: 0,
            },
            "unknown error",
        ),
    ] {
        let mut s = GitStr::new();
        let mut r = r;
        // SAFETY: valid string and result.
        assert_eq!(unsafe { ffi::git_process_result_msg(&mut s, &mut r) }, 0);
        assert_eq!(s.as_bytes(), m.as_bytes());
    }
}

#[test]
fn write_to_dead_child_is_epipe() {
    let _g = test_support::init();
    let _l = fd_lock();
    let mut p: *mut GitProcess = core::ptr::null_mut();
    let args = [c"/bin/true".as_ptr()];
    let mut o = opts(&[flags::CAPTURE_IN]);
    // SAFETY: valid arguments.
    unsafe {
        assert_eq!(
            ffi::git_process_new(&mut p, args.as_ptr(), 1, core::ptr::null(), 0, &mut o),
            0
        );
        assert_eq!(ffi::git_process_start(p), 0);
        assert_eq!(ffi::git_process_wait(core::ptr::null_mut(), p), 0);
        // SIGPIPE is blocked and consumed: the write fails, the test lives
        assert_eq!(ffi::git_process_write(p, b"x".as_ptr().cast(), 1), -1);
        assert_eq!(msg(), "could not write to child process: Broken pipe");
        ffi::git_process_close(p);
        ffi::git_process_free(p);
    }
}

// ----- descriptors, buffers and signals (review U4-3, U4-4, U4-6, U4-8) -----

fn open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}

/// Every test of this file holds it: the descriptor counts must not see
/// another test's pipes.
static FD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn fd_lock() -> std::sync::MutexGuard<'static, ()> {
    FD_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn no_descriptor_leaks() {
    let _g = test_support::init();
    let _l = fd_lock();
    let all = libgit2_rs::ProcessOptions {
        capture_in: true,
        capture_out: true,
        capture_err: true,
        ..Default::default()
    };
    let before = open_fds();
    for _ in 0..10 {
        // the safe API: new, start, wait, drop
        let mut p = libgit2_rs::Process::new(&[b"/bin/true"], &[], &all).unwrap();
        p.start().unwrap();
        assert_eq!(p.wait().unwrap(), libgit2_rs::ProcessStatus::Exited(0));
        drop(p);
        // the C ABI: new, start, wait, free (without git_process_close)
        let args = [c"/bin/true".as_ptr()];
        let mut o = opts(&[flags::CAPTURE_IN, flags::CAPTURE_OUT, flags::CAPTURE_ERR]);
        let mut p: *mut GitProcess = core::ptr::null_mut();
        // SAFETY: valid arguments.
        unsafe {
            assert_eq!(
                ffi::git_process_new(&mut p, args.as_ptr(), 1, core::ptr::null(), 0, &mut o),
                0
            );
            assert_eq!(ffi::git_process_start(p), 0);
            assert_eq!(ffi::git_process_wait(core::ptr::null_mut(), p), 0);
            ffi::git_process_free(p);
        }
        // failed starts: not found, and a failing chdir after the fork
        let mut p = libgit2_rs::Process::new(&[b"/no/such/program"], &[], &all).unwrap();
        assert!(p.start().is_err());
        drop(p);
        let cwd = libgit2_rs::ProcessOptions {
            cwd: Some(b"/no/such/dir".to_vec()),
            ..all.clone()
        };
        let mut p = libgit2_rs::Process::new(&[b"/bin/true"], &[], &cwd).unwrap();
        assert!(p.start().is_err());
        drop(p);
    }
    assert_eq!(open_fds(), before, "descriptors leaked");
}

#[test]
fn read_into_uninitialized_buffers() {
    let _g = test_support::init();
    let _l = fd_lock();
    let args = [c"echo hello; echo oops >&2".as_ptr()];
    let mut o = opts(&[flags::USE_SHELL, flags::CAPTURE_OUT, flags::CAPTURE_ERR]);
    let mut p: *mut GitProcess = core::ptr::null_mut();
    let mut buf = core::mem::MaybeUninit::<[u8; 64]>::uninit();
    // SAFETY: valid arguments; the buffer is writable (uninitialized), and
    // only the bytes `read` reports are read back.
    unsafe {
        assert_eq!(
            ffi::git_process_new(&mut p, args.as_ptr(), 1, core::ptr::null(), 0, &mut o),
            0
        );
        assert_eq!(ffi::git_process_start(p), 0);
        let n = ffi::git_process_read(p, buf.as_mut_ptr().cast(), 64);
        assert_eq!(&buf.assume_init_ref()[..n as usize], b"hello\n");
        let mut ebuf = core::mem::MaybeUninit::<[u8; 64]>::uninit();
        let n = ffi::git_process_read_err(p, ebuf.as_mut_ptr().cast(), 64);
        assert_eq!(&ebuf.assume_init_ref()[..n as usize], b"oops\n");
        assert_eq!(ffi::git_process_wait(core::ptr::null_mut(), p), 0);
        ffi::git_process_free(p);
    }
}

/// The parent's ends of the pipes are close-on-exec: a child spawned
/// meanwhile does not hold them, so closing our stdin pipe gives EOF.
#[test]
fn pipes_do_not_leak_into_other_children() {
    let _g = test_support::init();
    let _l = fd_lock();
    let io = libgit2_rs::ProcessOptions {
        capture_in: true,
        capture_out: true,
        ..Default::default()
    };
    let mut cat = libgit2_rs::Process::new(&[b"/bin/cat"], &[], &io).unwrap();
    cat.start().unwrap();
    // a second child, started while `cat`'s pipes are open, lives on
    let mut sleeper =
        libgit2_rs::Process::new(&[b"/bin/sleep", b"30"], &[], &Default::default()).unwrap();
    sleeper.start().unwrap();
    assert_eq!(cat.write(b"x").unwrap(), 1);
    cat.close_in().unwrap();
    let mut b = [0u8; 8];
    assert_eq!(cat.read(&mut b).unwrap(), 1);
    // EOF although `sleeper` still runs: it did not inherit our write end
    assert_eq!(cat.read(&mut b).unwrap(), 0);
    assert_eq!(cat.wait().unwrap(), libgit2_rs::ProcessStatus::Exited(0));
    let pid = sleeper.id().unwrap();
    // SAFETY: our child.
    unsafe { libc::kill(pid, libc::SIGKILL) };
    assert_eq!(
        sleeper.wait().unwrap(),
        libgit2_rs::ProcessStatus::Signaled(libc::SIGKILL)
    );
}

/// SIGPIPE's disposition in the child, from `/proc/self/status`.
fn child_ignores_sigpipe(out: &[u8]) -> bool {
    let s = String::from_utf8_lossy(out);
    let line = s.lines().find(|l| l.starts_with("SigIgn:")).unwrap();
    let mask = u64::from_str_radix(line["SigIgn:".len()..].trim(), 16).unwrap();
    mask & (1 << (libc::SIGPIPE - 1)) != 0
}

#[test]
fn sigpipe_disposition() {
    let _g = test_support::init();
    let _l = fd_lock();
    // The test harness (a Rust program) ignores SIGPIPE.
    let (out, _) = run(
        &[c"/bin/cat", c"/proc/self/status"],
        &[],
        opts(&[flags::CAPTURE_OUT]),
        b"",
    );
    assert!(
        child_ignores_sigpipe(&out),
        "the C ABI keeps the inherited disposition"
    );

    let o = libgit2_rs::ProcessOptions {
        capture_out: true,
        ..Default::default()
    };
    let mut p = libgit2_rs::Process::new(&[b"/bin/cat", b"/proc/self/status"], &[], &o).unwrap();
    p.start().unwrap();
    let mut out = Vec::new();
    let mut b = [0u8; 256];
    loop {
        let n = p.read(&mut b).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&b[..n]);
    }
    p.wait().unwrap();
    assert!(
        !child_ignores_sigpipe(&out),
        "the safe API restores SIGPIPE"
    );
}
