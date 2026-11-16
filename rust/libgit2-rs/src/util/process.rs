//! Port of `src/util/unix/process.c` (with `src/util/process.h`): spawning
//! a child with `fork` and `execve`, with optional pipes to its standard
//! streams; used by the CLI and the `ssh_exec` transport.
//!
//! Types:
//! - `git_process` is opaque (kind 1): its body was private to process.c,
//!   and C only passes the pointer. It is a `Box<GitProcess>`.
//! - `git_process_options` and `git_process_result` are mirrored (kind 3):
//!   C allocates them (on its stack) and fills or reads them.
//!
//! The child, between `fork` and `execve`, runs only async-signal-safe
//! system calls on data prepared before the `fork` (no allocation, no lock,
//! no panic path): see [`child_exec`]. File descriptors are handled as the
//! C handles them (which descriptors are closed and duplicated where, and
//! `SIGPIPE` being blocked around writes); the differences are fixes of
//! defects (rust/DEFECTS.md): the child leaves with `_exit`, the parent
//! closes its status pipe and waits for the right child when `exec` failed.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::ffi::CString;

use crate::util::errors::{self, Error, ErrorClass, ErrorCode, Result};
use crate::util::fs_path;
use crate::util::net::c_prefix;
use crate::util::str::GitStr;
use crate::{git_assert, git_assert_arg, git_error_set};

/// `git_process_result_status`.
pub mod status {
    use core::ffi::c_int;
    /// `GIT_PROCESS_STATUS_NONE`: not started.
    pub const NONE: c_int = 0;
    /// `GIT_PROCESS_STATUS_NORMAL`: exited.
    pub const NORMAL: c_int = 1;
    /// `GIT_PROCESS_STATUS_ERROR`: killed by a signal, or otherwise.
    pub const ERROR: c_int = 2;
}

/// `git_process_options`, mirrored. The bitfield is one `unsigned int`
/// (x86-64 SysV: the first declared is the least significant bit):
/// bit 0 `use_shell`, 1 `capture_in`, 2 `capture_out`, 3 `capture_err`,
/// 4 `exclude_env`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GitProcessOptions {
    /// The bitfield.
    pub flags: c_uint,
    /// `cwd`: the directory to run in, or NULL.
    pub cwd: *mut c_char,
}

/// Bit masks of [`GitProcessOptions::flags`].
pub mod flags {
    use core::ffi::c_uint;
    /// `use_shell`.
    pub const USE_SHELL: c_uint = 1 << 0;
    /// `capture_in`.
    pub const CAPTURE_IN: c_uint = 1 << 1;
    /// `capture_out`.
    pub const CAPTURE_OUT: c_uint = 1 << 2;
    /// `capture_err`.
    pub const CAPTURE_ERR: c_uint = 1 << 3;
    /// `exclude_env`.
    pub const EXCLUDE_ENV: c_uint = 1 << 4;
}

/// `git_process_result`, mirrored.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GitProcessResult {
    /// One of [`status`].
    pub status: c_int,
    /// The exit code, for `NORMAL`.
    pub exitcode: c_int,
    /// The signal that ended it, for `ERROR`.
    pub signal: c_int,
}

/// What `git_process_new` reads of its options.
#[derive(Clone, Debug, Default)]
pub struct ProcessOptions {
    /// Run `args[0]` with `/bin/sh -c '<args[0]> "$@"' <args...>`.
    pub use_shell: bool,
    /// A pipe to the child's standard input ([`GitProcess::write`]).
    pub capture_in: bool,
    /// A pipe from the child's standard output ([`GitProcess::read`]).
    pub capture_out: bool,
    /// A pipe from the child's standard error ([`GitProcess::read_err`]).
    pub capture_err: bool,
    /// Do not inherit this process's environment.
    pub exclude_env: bool,
    /// The directory to run in.
    pub cwd: Option<Vec<u8>>,
}

/// A child process (`git_process`), not started until
/// [`start`](Self::start).
pub struct GitProcess {
    /// The arguments; `args[0]` is the program (resolved by `start`).
    args: Vec<CString>,
    /// The environment, or `None` for an empty one (C passes NULL).
    env: Option<Vec<CString>>,
    cwd: Option<CString>,
    use_shell: bool,
    capture_in: bool,
    capture_out: bool,
    capture_err: bool,
    pid: libc::pid_t,
    child_in: c_int,
    child_out: c_int,
    child_err: c_int,
}

/// A copy of `s` (up to a NUL) as a `CString`, or the out-of-memory error.
fn try_cstring(s: &[u8]) -> Result<CString> {
    let s = c_prefix(s);
    let mut v = Vec::new();
    v.try_reserve_exact(s.len() + 1)
        .map_err(|_| errors::oom())?;
    v.extend_from_slice(s);
    // No NUL inside (`c_prefix`), and room for the terminator.
    Ok(CString::new(v).unwrap_or_default())
}

/// `is_delete_env`: `NAME=` (an `=` and nothing after).
fn is_delete_env(env: &[u8]) -> bool {
    match memchr::memchr(b'=', env) {
        Some(i) => i + 1 == env.len(),
        None => false,
    }
}

/// `git_strlist_contains_key(strings, key, '=')`: `key` has an `=`, and
/// some string starts with what precedes it (so `FOOBAR=x` matches the key
/// of `FOO=y`, as in C).
fn strlist_contains_key(strings: &[&[u8]], key: &[u8]) -> bool {
    let Some(n) = memchr::memchr(b'=', key) else {
        return false;
    };
    strings.iter().any(|s| c_prefix(s).starts_with(&key[..n]))
}

/// `setup_args`.
fn setup_args(args: &[&[u8]], use_shell: bool) -> Result<Vec<CString>> {
    let mut out = Vec::new();
    out.try_reserve(args.len() + 3).map_err(|_| errors::oom())?;
    if use_shell {
        let first = c_prefix(args[0]);
        let mut cmd = Vec::new();
        cmd.try_reserve_exact(first.len() + 6)
            .map_err(|_| errors::oom())?;
        cmd.extend_from_slice(first);
        cmd.extend_from_slice(b" \"$@\"");
        out.push(try_cstring(b"/bin/sh")?);
        out.push(try_cstring(b"-c")?);
        out.push(try_cstring(&cmd)?);
    }
    for a in args {
        out.push(try_cstring(a)?);
    }
    Ok(out)
}

unsafe extern "C" {
    static environ: *const *const c_char;
}

/// This process's environment, as `environ` lists it.
///
/// The caller must not let the environment be changed meanwhile (the
/// header's contract for `git_process_new`).
fn environ_entries() -> Vec<&'static [u8]> {
    let mut v = Vec::new();
    // SAFETY: `environ` is the C library's, read (not written) here.
    let mut p = unsafe { environ };
    if p.is_null() {
        return v;
    }
    loop {
        // SAFETY: `environ` is a NULL-terminated array of NUL-terminated
        // strings, which (per the contract) nothing changes meanwhile.
        let s = unsafe { *p };
        if s.is_null() {
            break;
        }
        // SAFETY: as above; the slice is used before `git_process_new`
        // returns.
        v.push(unsafe { CStr::from_ptr(s) }.to_bytes());
        // SAFETY: not past the terminating NULL.
        p = unsafe { p.add(1) };
    }
    v
}

/// `merge_env`: `env` (less its `NAME=` entries), then, unless
/// `exclude_env`, the inherited entries whose name `env` does not set;
/// `None` when that is nothing.
fn merge_env(env: &[&[u8]], exclude_env: bool) -> Result<Option<Vec<CString>>> {
    let inherited = if exclude_env {
        Vec::new()
    } else {
        environ_entries()
    };
    let mut merged = Vec::new();
    merged
        .try_reserve(env.len() + inherited.len())
        .map_err(|_| errors::oom())?;
    for e in env {
        if is_delete_env(c_prefix(e)) {
            continue;
        }
        merged.push(try_cstring(e)?);
    }
    for kv in inherited {
        if strlist_contains_key(env, kv) {
            continue;
        }
        merged.push(try_cstring(kv)?);
    }
    Ok(if merged.is_empty() {
        None
    } else {
        Some(merged)
    })
}

/// `pipe(fds)`.
fn sys_pipe(fds: &mut [c_int; 2]) -> c_int {
    // SAFETY: `pipe` writes two descriptors into the array.
    unsafe { libc::pipe(fds.as_mut_ptr()) }
}

/// `CLOSE_FD`.
fn close_fd(fd: &mut c_int) {
    if *fd >= 0 {
        // SAFETY: a descriptor this process opened and still owns.
        unsafe { libc::close(*fd) };
        *fd = -1;
    }
}

/// `try_read_status`: read up to `buf.len()` bytes, stopping at EOF.
fn try_read_status(fd: c_int, buf: &mut [u8]) -> Result<usize> {
    let mut read_len = 0;
    while read_len < buf.len() {
        let rest = &mut buf[read_len..];
        // SAFETY: `rest` is writable for its length.
        let ret = unsafe { libc::read(fd, rest.as_mut_ptr().cast(), rest.len()) };
        if ret < 0 {
            let e = errors::errno();
            if e != libc::EAGAIN && e != libc::EINTR {
                let _ = git_error_set!(ErrorClass::OS, "could not read child status");
                return Err(ErrorCode::GENERIC);
            }
            // C adds the -1 to its count here (rust/DEFECTS.md); retry.
            continue;
        }
        if ret == 0 {
            break;
        }
        read_len += ret as usize;
    }
    Ok(read_len)
}

const STATUS_LEN: usize = 3 * core::mem::size_of::<c_int>();

/// `read_status`: 0 when the child's `exec` succeeded (immediate EOF), or
/// the failure it reported (with the OS error set).
fn read_status(fd: c_int) -> Result<c_int> {
    let mut buffer = [0u8; STATUS_LEN];
    let read_len = try_read_status(fd, &mut buffer)?;

    // Immediate EOF indicates the exec succeeded.
    if read_len == 0 {
        return Ok(0);
    }
    if read_len < STATUS_LEN {
        let _ = git_error_set!(ErrorClass::INVALID, "child status truncated");
        return Err(ErrorCode::GENERIC);
    }
    let word = |i: usize| {
        let mut w = [0u8; 4];
        w.copy_from_slice(&buffer[i * 4..i * 4 + 4]);
        c_int::from_ne_bytes(w)
    };
    let (fn_error, os_error, fn_len) = (word(0), word(1), word(2));

    let mut name = [0u8; 128];
    let mut name_len = 0;
    if fn_len > 0 {
        let want = (fn_len as usize).min(name.len() - 1);
        name_len = try_read_status(fd, &mut name[..want])?;
    }

    if fn_error != 0 {
        errors::set_errno(os_error);
        let fn_name: &[u8] = if name_len > 0 {
            c_prefix(&name[..name_len])
        } else {
            b""
        };
        let fn_name = if fn_name.is_empty() {
            b"(unknown)" as &[u8]
        } else {
            fn_name
        };
        let _ = git_error_set!(ErrorClass::OS, "could not {}", errors::Bytes(fn_name));
    }
    Ok(fn_error)
}

/// What the child needs between `fork` and `execve`, prepared before the
/// `fork`: plain values and pointers into the parent's live allocations,
/// which the child's copy of memory still holds.
struct ChildPlan {
    status_read: c_int,
    status_write: c_int,
    in_: Option<(c_int, c_int)>,
    out: Option<(c_int, c_int)>,
    err: Option<(c_int, c_int)>,
    cwd: *const c_char,
    path: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
}

/// `try_write_status`: write all of `len` bytes at `buf` unless `write`
/// fails or makes no progress. Async-signal-safe.
fn child_write_all(fd: c_int, buf: *const u8, len: usize) -> bool {
    let mut written = 0usize;
    while written < len {
        // SAFETY: `buf` has `len` readable bytes (a local array or a
        // static string); `write` is async-signal-safe.
        let ret = unsafe { libc::write(fd, buf.wrapping_add(written).cast(), len - written) };
        if ret <= 0 {
            break;
        }
        written += ret as usize;
    }
    written == len
}

/// `write_status`. Async-signal-safe: a stack buffer and `write`.
fn child_write_status(fd: c_int, name: &'static [u8], error: c_int, os_error: c_int) {
    let fn_len = name.len() as c_int;
    let (e, o, l) = (
        error.to_ne_bytes(),
        os_error.to_ne_bytes(),
        fn_len.to_ne_bytes(),
    );
    // Constant indices only: no bounds check, so no panic path.
    let buffer: [u8; STATUS_LEN] = [
        e[0], e[1], e[2], e[3], o[0], o[1], o[2], o[3], l[0], l[1], l[2], l[3],
    ];
    // Do our best effort to write all the status.
    if !child_write_all(fd, buffer.as_ptr(), STATUS_LEN) {
        return;
    }
    if !name.is_empty() {
        child_write_all(fd, name.as_ptr(), name.len());
    }
}

/// The child: arrange its descriptors, change directory, `execve`; on
/// failure, report through the status pipe and `_exit(0)`.
///
/// Only async-signal-safe calls (`close`, `dup2`, `chdir`, `execve`,
/// `write`, `_exit`, reading `errno`) on the prepared plan: no allocation,
/// no lock, and no operation that can panic.
fn child_exec(plan: &ChildPlan) -> ! {
    // Close the opposing side of the pipes
    if plan.status_read >= 0 {
        // SAFETY: closing a descriptor (async-signal-safe).
        unsafe { libc::close(plan.status_read) };
    }
    for (stream, target, ours_is_read) in [
        (plan.in_, libc::STDIN_FILENO, true),
        (plan.out, libc::STDOUT_FILENO, false),
        (plan.err, libc::STDERR_FILENO, false),
    ] {
        let Some((r, w)) = stream else { continue };
        // The child keeps the end the parent does not, as C's CLOSE_FD.
        let (keep, other) = if ours_is_read { (r, w) } else { (w, r) };
        if other >= 0 {
            // SAFETY: as above.
            unsafe { libc::close(other) };
        }
        // SAFETY: duplicating a descriptor (async-signal-safe).
        unsafe { libc::dup2(keep, target) };
    }

    if !plan.cwd.is_null() {
        // SAFETY: `cwd` is a NUL-terminated string the parent keeps alive
        // (in this copy of its memory); `chdir` is async-signal-safe.
        let error = unsafe { libc::chdir(plan.cwd) };
        if error < 0 {
            child_write_status(plan.status_write, b"chdir", error, child_errno());
            // SAFETY: `_exit` is async-signal-safe and always sound.
            unsafe { libc::_exit(0) };
        }
    }

    // Exec the process and write the results back if the call fails; if it
    // succeeds, CLOEXEC closes the status pipe.
    // SAFETY: `path` and the strings of `argv` / `envp` are NUL-terminated,
    // the arrays NULL-terminated (or `envp` NULL), all alive in this copy
    // of the parent's memory; `execve` is async-signal-safe.
    let error = unsafe { libc::execve(plan.path, plan.argv, plan.envp) };
    child_write_status(plan.status_write, b"execve", error, child_errno());
    // SAFETY: as above.
    unsafe { libc::_exit(0) }
}

/// `errno`, async-signal-safely.
fn child_errno() -> c_int {
    // SAFETY: `__errno_location` has no preconditions, and is
    // async-signal-safe.
    let p = unsafe { libc::__errno_location() };
    // SAFETY: it points to this thread's `errno`, valid and readable.
    unsafe { *p }
}

impl GitProcess {
    /// `git_process_new`: a process to run `args` (`args[0]` is looked up
    /// in `PATH` unless it contains a `/`), with `env` added to (or, with
    /// `exclude_env`, instead of) this process's environment; `NAME=`
    /// entries remove a variable.
    ///
    /// The environment must not change during the call.
    pub fn new(args: &[&[u8]], env: &[&[u8]], opts: Option<&ProcessOptions>) -> Result<GitProcess> {
        git_assert_arg!(!args.is_empty(), "out && args && args_len > 0");
        let use_shell = opts.is_some_and(|o| o.use_shell);
        let argv = setup_args(args, use_shell)?;
        let envv = merge_env(env, opts.is_some_and(|o| o.exclude_env))?;

        let mut p = GitProcess {
            args: argv,
            env: envv,
            cwd: None,
            use_shell: false,
            capture_in: false,
            capture_out: false,
            capture_err: false,
            pid: 0,
            child_in: -1,
            child_out: -1,
            child_err: -1,
        };
        if let Some(o) = opts {
            p.use_shell = o.use_shell;
            p.capture_in = o.capture_in;
            p.capture_out = o.capture_out;
            p.capture_err = o.capture_err;
            if let Some(cwd) = &o.cwd {
                p.cwd = Some(try_cstring(cwd)?);
            }
        }
        Ok(p)
    }

    /// `git_process_new_from_cmdline`: run `cmdline` with `/bin/sh`.
    pub fn from_cmdline(
        cmdline: &[u8],
        env: &[&[u8]],
        opts: Option<&ProcessOptions>,
    ) -> Result<GitProcess> {
        let mut merged = opts.cloned().unwrap_or_default();
        merged.use_shell = true;
        GitProcess::new(&[cmdline], env, Some(&merged))
    }

    /// `resolve_path`: the full path of `args[0]` (unless the shell runs).
    fn resolve_path(&mut self) -> Result<()> {
        if self.use_shell {
            return Ok(());
        }
        let mut full_path = GitStr::new();
        let error = match fs_path::find_executable(&mut full_path, self.args[0].to_bytes()) {
            Ok(()) => 0,
            Err(e) => e.raw(),
        };
        if error == ErrorCode::NOT_FOUND.raw() {
            let _ = git_error_set!(
                ErrorClass::SSH,
                "cannot run {}: No such file or directory",
                errors::Bytes(self.args[0].to_bytes())
            );
            return Err(ErrorCode::GENERIC);
        }
        ErrorCode::check_zero(error)?;
        self.args[0] = try_cstring(full_path.as_bytes())?;
        Ok(())
    }

    /// `git_process_start`: fork and exec, and wait until the child has
    /// exec'd (or report why it could not).
    pub fn start(&mut self) -> Result<()> {
        let mut in_ = [-1 as c_int; 2];
        let mut out = [-1 as c_int; 2];
        let mut err = [-1 as c_int; 2];
        let mut status = [-1 as c_int; 2];

        let r = self.start_with(&mut in_, &mut out, &mut err, &mut status);
        if r.is_err() {
            for fd in in_
                .iter_mut()
                .chain(&mut out)
                .chain(&mut err)
                .chain(&mut status)
            {
                close_fd(fd);
            }
        }
        r
    }

    fn start_with(
        &mut self,
        in_: &mut [c_int; 2],
        out: &mut [c_int; 2],
        err: &mut [c_int; 2],
        status: &mut [c_int; 2],
    ) -> Result<()> {
        // Locate the path (unless we're letting the shell do it for us)
        self.resolve_path().map_err(|_| ErrorCode::GENERIC)?;

        // The pointer arrays for execve, built before the fork.
        let mut argv: Vec<*const c_char> = Vec::new();
        argv.try_reserve_exact(self.args.len() + 1)
            .map_err(|_| errors::oom())?;
        argv.extend(self.args.iter().map(|a| a.as_ptr()));
        argv.push(core::ptr::null());
        let mut envp: Vec<*const c_char> = Vec::new();
        if let Some(env) = &self.env {
            envp.try_reserve_exact(env.len() + 1)
                .map_err(|_| errors::oom())?;
            envp.extend(env.iter().map(|e| e.as_ptr()));
            envp.push(core::ptr::null());
        }

        // Set up the pipes to read from/write to the process
        if (self.capture_in && sys_pipe(in_) < 0)
            || (self.capture_out && sys_pipe(out) < 0)
            || (self.capture_err && sys_pipe(err) < 0)
        {
            let _ = git_error_set!(ErrorClass::OS, "could not create pipe");
            return Err(ErrorCode::GENERIC);
        }

        // Set up a self-pipe for status from the forked process.
        let ok = sys_pipe(status) >= 0 && {
            // SAFETY: an open descriptor.
            let fdflags = unsafe { libc::fcntl(status[1], libc::F_GETFD) };
            // SAFETY: as above.
            fdflags >= 0
                && unsafe { libc::fcntl(status[1], libc::F_SETFD, fdflags | libc::FD_CLOEXEC) } >= 0
        };
        if !ok {
            let _ = git_error_set!(ErrorClass::OS, "could not create pipe");
            return Err(ErrorCode::GENERIC);
        }

        let plan = ChildPlan {
            status_read: status[0],
            status_write: status[1],
            in_: self.capture_in.then_some((in_[0], in_[1])),
            out: self.capture_out.then_some((out[0], out[1])),
            err: self.capture_err.then_some((err[0], err[1])),
            cwd: self.cwd.as_ref().map_or(core::ptr::null(), |c| c.as_ptr()),
            path: self.args[0].as_ptr(),
            argv: argv.as_ptr(),
            envp: if self.env.is_some() {
                envp.as_ptr()
            } else {
                core::ptr::null()
            },
        };

        // SAFETY: the child only runs `child_exec`, which is
        // async-signal-safe and never returns.
        let pid = unsafe { libc::fork() };
        match pid {
            -1 => {
                let _ = git_error_set!(ErrorClass::OS, "could not fork");
                Err(ErrorCode::GENERIC)
            }
            0 => child_exec(&plan),
            _ => {
                // Close the opposing side of the pipes
                close_fd(&mut status[1]);
                if self.capture_in {
                    close_fd(&mut in_[0]);
                    self.child_in = in_[1];
                }
                if self.capture_out {
                    close_fd(&mut out[1]);
                    self.child_out = out[0];
                }
                if self.capture_err {
                    close_fd(&mut err[1]);
                    self.child_err = err[0];
                }

                // Try to read the status
                let r = read_status(status[0]);
                // C keeps the status pipe open for the process's life
                // (rust/DEFECTS.md); it has served its purpose.
                close_fd(&mut status[0]);
                if !matches!(r, Ok(n) if n >= 0) {
                    // C waits on pid 0 here (rust/DEFECTS.md).
                    let mut state = 0;
                    // SAFETY: our child; `state` is writable.
                    unsafe { libc::waitpid(pid, &raw mut state, 0) };
                    // C leaves these naming the descriptors it closes.
                    self.child_in = -1;
                    self.child_out = -1;
                    self.child_err = -1;
                    return Err(ErrorCode::GENERIC);
                }
                drop((argv, envp));
                self.pid = pid;
                Ok(())
            }
        }
    }

    /// `git_process_id`.
    pub fn id(&self) -> Result<libc::pid_t> {
        if self.pid == 0 {
            let _ = git_error_set!(ErrorClass::INVALID, "process not running");
            return Err(ErrorCode::GENERIC);
        }
        Ok(self.pid)
    }

    fn read_fd(fd: c_int, buf: &mut [u8]) -> Result<usize> {
        let count = buf.len().min(isize::MAX as usize);
        // SAFETY: `buf` is writable for `count` bytes.
        let ret = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), count) };
        if ret < 0 {
            let _ = git_error_set!(ErrorClass::OS, "could not read from child process");
            return Err(ErrorCode::GENERIC);
        }
        Ok(ret as usize)
    }

    /// `git_process_read`: from the child's standard output.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        git_assert!(self.capture_out, "process->capture_out");
        Self::read_fd(self.child_out, buf)
    }

    /// `git_process_read_err`: from the child's standard error.
    pub fn read_err(&mut self, buf: &mut [u8]) -> Result<usize> {
        git_assert!(self.capture_err, "process->capture_err");
        Self::read_fd(self.child_err, buf)
    }

    /// `git_process_write`: to the child's standard input, with `SIGPIPE`
    /// blocked for this thread meanwhile (a closed pipe is `EPIPE`).
    pub fn write(&mut self, buf: &[u8]) -> Result<usize> {
        git_assert!(self.capture_in, "process->capture_in");
        let count = buf.len().min(isize::MAX as usize);
        let saved = disable_signals()?;
        // SAFETY: `buf` is readable for `count` bytes.
        let ret = unsafe { libc::write(self.child_in, buf.as_ptr().cast(), count) };
        if ret < 0 {
            let _ = git_error_set!(ErrorClass::OS, "could not write to child process");
        }
        restore_signals(&saved)?;
        if ret < 0 {
            Err(ErrorCode::GENERIC)
        } else {
            Ok(ret as usize)
        }
    }

    /// `git_process_close_in`.
    pub fn close_in(&mut self) -> Result<()> {
        if !self.capture_in {
            let _ = git_error_set!(ErrorClass::INVALID, "input is not open");
            return Err(ErrorCode::GENERIC);
        }
        close_fd(&mut self.child_in);
        Ok(())
    }

    /// `git_process_close_out`.
    pub fn close_out(&mut self) -> Result<()> {
        if !self.capture_out {
            let _ = git_error_set!(ErrorClass::INVALID, "output is not open");
            return Err(ErrorCode::GENERIC);
        }
        close_fd(&mut self.child_out);
        Ok(())
    }

    /// `git_process_close_err`.
    pub fn close_err(&mut self) -> Result<()> {
        if !self.capture_err {
            let _ = git_error_set!(ErrorClass::INVALID, "error is not open");
            return Err(ErrorCode::GENERIC);
        }
        close_fd(&mut self.child_err);
        Ok(())
    }

    /// `git_process_close`: close the pipes (without waiting).
    pub fn close(&mut self) {
        close_fd(&mut self.child_in);
        close_fd(&mut self.child_out);
        close_fd(&mut self.child_err);
    }

    /// `git_process_wait`: wait for the child to end.
    pub fn wait(&mut self) -> Result<GitProcessResult> {
        if self.pid == 0 {
            let _ = git_error_set!(ErrorClass::INVALID, "process is stopped");
            return Err(ErrorCode::GENERIC);
        }
        let mut state = 0;
        // SAFETY: `state` is writable.
        if unsafe { libc::waitpid(self.pid, &raw mut state, 0) } < 0 {
            let _ = git_error_set!(ErrorClass::OS, "could not wait for child");
            return Err(ErrorCode::GENERIC);
        }
        self.pid = 0;
        let mut result = GitProcessResult::default();
        if libc::WIFEXITED(state) {
            result.status = status::NORMAL;
            result.exitcode = libc::WEXITSTATUS(state);
        } else if libc::WIFSIGNALED(state) {
            result.status = status::ERROR;
            result.signal = libc::WTERMSIG(state);
        } else {
            result.status = status::ERROR;
        }
        Ok(result)
    }
}

impl Drop for GitProcess {
    /// `git_process_free`: close the pipes if the child was not waited for
    /// (as C: after `wait`, the pipes are the caller's to close).
    fn drop(&mut self) {
        if self.pid != 0 {
            self.close();
        }
    }
}

/// A signal set holding only `SIGPIPE`.
fn sigpipe_set() -> libc::sigset_t {
    // SAFETY: `sigset_t` is plain data; it is initialized by `sigemptyset`.
    let mut mask: libc::sigset_t = unsafe { core::mem::zeroed() };
    // SAFETY: a valid set to write.
    unsafe { libc::sigemptyset(&raw mut mask) };
    // SAFETY: an initialized set and a valid signal.
    unsafe { libc::sigaddset(&raw mut mask, libc::SIGPIPE) };
    mask
}

/// `disable_signals` (threads): block `SIGPIPE` for this thread, returning
/// the previous mask.
fn disable_signals() -> Result<libc::sigset_t> {
    let mask = sigpipe_set();
    // SAFETY: plain data, written by `pthread_sigmask`.
    let mut saved: libc::sigset_t = unsafe { core::mem::zeroed() };
    // pthread reports failure with a positive value, which the C's `< 0`
    // never sees; so here.
    // SAFETY: valid sets.
    if unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &raw const mask, &raw mut saved) } < 0 {
        let _ = git_error_set!(ErrorClass::OS, "could not configure signal mask");
        return Err(ErrorCode::GENERIC);
    }
    Ok(saved)
}

/// `restore_signals` (threads): consume a `SIGPIPE` raised meanwhile, then
/// restore the mask.
fn restore_signals(saved: &libc::sigset_t) -> Result<()> {
    let mask = sigpipe_set();
    // SAFETY: plain data, written by `sigpending`.
    let mut pending: libc::sigset_t = unsafe { core::mem::zeroed() };
    // SAFETY: a valid set to write.
    if unsafe { libc::sigpending(&raw mut pending) } < 0 {
        let _ = git_error_set!(ErrorClass::OS, "could not examine pending signals");
        return Err(ErrorCode::GENERIC);
    }
    let mut signal = 0;
    // SAFETY: an initialized set.
    let is_pending = unsafe { libc::sigismember(&raw const pending, libc::SIGPIPE) } == 1;
    // SAFETY: a valid set; `signal` is writable. SIGPIPE is pending and
    // blocked, so `sigwait` returns at once.
    if is_pending && unsafe { libc::sigwait(&raw const mask, &raw mut signal) } < 0 {
        let _ = git_error_set!(
            ErrorClass::OS,
            "could not wait for (blocking) signal delivery"
        );
        return Err(ErrorCode::GENERIC);
    }
    // SAFETY: a valid set.
    if unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, saved, core::ptr::null_mut()) } < 0 {
        let _ = git_error_set!(ErrorClass::OS, "could not configure signal mask");
        return Err(ErrorCode::GENERIC);
    }
    Ok(())
}

/// `git_process_result_msg`: append a description of `result` to `out`.
pub fn result_msg(out: &mut GitStr, result: &GitProcessResult) -> Result<()> {
    match result.status {
        status::NONE => out.puts(b"process not started"),
        status::NORMAL => out.printf(
            "process exited with code %d".len(),
            format_args!("process exited with code {}", result.exitcode),
        ),
        _ if result.signal != 0 => out.printf(
            "process exited on signal %d".len(),
            format_args!("process exited on signal {}", result.signal),
        ),
        _ => out.puts(b"unknown error"),
    }
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// How a process ended ([`GitProcessResult`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessStatus {
    /// It was not started.
    NotStarted,
    /// It exited with this code.
    Exited(i32),
    /// It was killed by this signal.
    Signaled(i32),
    /// Some other end.
    Other,
}

impl From<GitProcessResult> for ProcessStatus {
    fn from(r: GitProcessResult) -> ProcessStatus {
        match r.status {
            status::NONE => ProcessStatus::NotStarted,
            status::NORMAL => ProcessStatus::Exited(r.exitcode),
            _ if r.signal != 0 => ProcessStatus::Signaled(r.signal),
            _ => ProcessStatus::Other,
        }
    }
}

/// `git_process_result_msg` of a result, as text.
pub fn describe(result: &GitProcessResult) -> String {
    let mut s = GitStr::new();
    let _ = result_msg(&mut s, result);
    String::from_utf8_lossy(s.as_bytes()).into_owned()
}

/// A child process (`git_process`): the safe API.
///
/// Dropping it closes its pipes unless it was waited for (as
/// `git_process_free`), but does not wait for it.
pub struct Process(GitProcess);

impl Process {
    /// `git_process_new`: a process to run `args` (`args[0]` is looked up
    /// in `PATH` unless it contains a `/`, or run by `/bin/sh` with
    /// `use_shell`), with `env` added to (or, with `exclude_env`, instead
    /// of) this process's environment; `NAME=` entries remove a variable.
    /// The environment must not be changed meanwhile.
    pub fn new(
        args: &[&[u8]],
        env: &[&[u8]],
        opts: &ProcessOptions,
    ) -> core::result::Result<Process, Error> {
        GitProcess::new(args, env, Some(opts))
            .map(Process)
            .map_err(Error::from)
    }

    /// `git_process_new_from_cmdline`: run `cmdline` with `/bin/sh`.
    pub fn from_cmdline(
        cmdline: &[u8],
        env: &[&[u8]],
        opts: &ProcessOptions,
    ) -> core::result::Result<Process, Error> {
        GitProcess::from_cmdline(cmdline, env, Some(opts))
            .map(Process)
            .map_err(Error::from)
    }

    /// `git_process_start`.
    pub fn start(&mut self) -> core::result::Result<(), Error> {
        self.0.start().map_err(Error::from)
    }

    /// `git_process_id`.
    pub fn id(&self) -> core::result::Result<i32, Error> {
        self.0.id().map_err(Error::from)
    }

    /// `git_process_read`: from the child's standard output (0 at EOF).
    pub fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, Error> {
        self.0.read(buf).map_err(Error::from)
    }

    /// `git_process_read_err`: from the child's standard error.
    pub fn read_err(&mut self, buf: &mut [u8]) -> core::result::Result<usize, Error> {
        self.0.read_err(buf).map_err(Error::from)
    }

    /// `git_process_write`: to the child's standard input.
    pub fn write(&mut self, buf: &[u8]) -> core::result::Result<usize, Error> {
        self.0.write(buf).map_err(Error::from)
    }

    /// `git_process_close_in`.
    pub fn close_in(&mut self) -> core::result::Result<(), Error> {
        self.0.close_in().map_err(Error::from)
    }

    /// `git_process_close_out`.
    pub fn close_out(&mut self) -> core::result::Result<(), Error> {
        self.0.close_out().map_err(Error::from)
    }

    /// `git_process_close_err`.
    pub fn close_err(&mut self) -> core::result::Result<(), Error> {
        self.0.close_err().map_err(Error::from)
    }

    /// `git_process_close`: close every pipe.
    pub fn close(&mut self) {
        self.0.close();
    }

    /// `git_process_wait`.
    pub fn wait(&mut self) -> core::result::Result<ProcessStatus, Error> {
        self.0.wait().map(ProcessStatus::from).map_err(Error::from)
    }
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `unix/process.c`.
///
/// A `git_process *` is one `git_process_new` returned, not yet freed, used
/// by one thread at a time.
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, cstr_bytes, status as to_status};

    /// The strings of a C array (`len` NUL-terminated strings, or NULL).
    ///
    /// # Safety
    ///
    /// When `len > 0` and `p` is not NULL, `p` has `len` valid string
    /// pointers whose strings outlive the result.
    unsafe fn strings<'a>(p: *const *const c_char, len: usize) -> Result<Vec<&'a [u8]>> {
        let mut v = Vec::new();
        if p.is_null() {
            return Ok(v);
        }
        v.try_reserve_exact(len).map_err(|_| errors::oom())?;
        for i in 0..len {
            // SAFETY: the caller's contract: `len` pointers at `p`.
            let s = unsafe { *p.wrapping_add(i) };
            // SAFETY: a NUL-terminated string.
            v.push(unsafe { cstr_bytes(s) });
        }
        Ok(v)
    }

    fn options(o: &GitProcessOptions) -> ProcessOptions {
        ProcessOptions {
            use_shell: o.flags & flags::USE_SHELL != 0,
            capture_in: o.flags & flags::CAPTURE_IN != 0,
            capture_out: o.flags & flags::CAPTURE_OUT != 0,
            capture_err: o.flags & flags::CAPTURE_ERR != 0,
            exclude_env: o.flags & flags::EXCLUDE_ENV != 0,
            // SAFETY: `cwd` is NULL or a NUL-terminated string (the
            // options' contract).
            cwd: (!o.cwd.is_null()).then(|| unsafe { cstr_bytes(o.cwd) }.to_vec()),
        }
    }

    /// The process behind a C pointer.
    ///
    /// # Safety
    ///
    /// `p` is a live process in this thread's exclusive use.
    unsafe fn process<'a>(p: *mut GitProcess) -> &'a mut GitProcess {
        // SAFETY: the caller's contract.
        unsafe { &mut *p }
    }

    /// `git_process_new`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `args` is NULL or has `args_len` valid
    /// string pointers; `env` is NULL or has `env_len`; `opts` is NULL or
    /// valid (with `cwd` NULL or a string). The environment does not change
    /// during the call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_new(
        out: *mut *mut GitProcess,
        args: *const *const c_char,
        args_len: usize,
        env: *const *const c_char,
        env_len: usize,
        opts: *mut GitProcessOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(
                !out.is_null() && !args.is_null() && args_len > 0,
                "out && args && args_len > 0",
                -1
            );
            // SAFETY: `out` is writable.
            unsafe { out.write(core::ptr::null_mut()) };
            // SAFETY: the caller's contract.
            let opts = unsafe { opts.as_ref() }.map(options);
            // SAFETY: the caller's contract.
            let args = match unsafe { strings(args, args_len) } {
                Ok(a) => a,
                Err(e) => return e.raw(),
            };
            // SAFETY: the caller's contract.
            let env = match unsafe { strings(env, env_len) } {
                Ok(e) => e,
                Err(e) => return e.raw(),
            };
            match GitProcess::new(&args, &env, opts.as_ref()) {
                Ok(p) => {
                    // SAFETY: `out` is writable.
                    unsafe { out.write(Box::into_raw(Box::new(p))) };
                    0
                }
                Err(_) => -1,
            }
        })
    }

    /// `git_process_new_from_cmdline`.
    ///
    /// # Safety
    ///
    /// As [`git_process_new`], with `cmdline` a string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_new_from_cmdline(
        out: *mut *mut GitProcess,
        cmdline: *const c_char,
        env: *const *const c_char,
        env_len: usize,
        opts: *mut GitProcessOptions,
    ) -> c_int {
        let mut merged = GitProcessOptions {
            flags: 0,
            cwd: core::ptr::null_mut(),
        };
        if !opts.is_null() {
            // SAFETY: valid options.
            merged = unsafe { opts.read() };
        }
        merged.flags |= flags::USE_SHELL;
        let args = [cmdline];
        // SAFETY: forwarded; `args` is one string pointer.
        unsafe { git_process_new(out, args.as_ptr(), 1, env, env_len, &raw mut merged) }
    }

    /// `git_process_start`.
    ///
    /// # Safety
    ///
    /// `process` is a live process.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_start(process: *mut GitProcess) -> c_int {
        // SAFETY: the caller's contract.
        barrier(|| to_status(unsafe { super::ffi::process(process) }.start()))
    }

    /// `git_process_id`.
    ///
    /// # Safety
    ///
    /// `out` is NULL or writable; `process` NULL or a live process.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_id(
        out: *mut libc::pid_t,
        process: *mut GitProcess,
    ) -> c_int {
        barrier(|| {
            git_assert!(!out.is_null() && !process.is_null(), "out && process", -1);
            // SAFETY: the caller's contract.
            match unsafe { super::ffi::process(process) }.id() {
                Ok(pid) => {
                    // SAFETY: writable.
                    unsafe { out.write(pid) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    fn ssize(r: Result<usize>) -> isize {
        match r {
            Ok(n) => n as isize,
            Err(e) => e.raw() as isize,
        }
    }

    /// `git_process_read`.
    ///
    /// # Safety
    ///
    /// `process` is NULL or a live process; `buf` is writable for `count`
    /// bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_read(
        process: *mut GitProcess,
        buf: *mut c_void,
        count: usize,
    ) -> isize {
        barrier(|| {
            git_assert_arg!(!process.is_null(), "process", -1);
            // SAFETY: the caller's contract.
            let p = unsafe { super::ffi::process(process) };
            git_assert!(p.capture_out, "process->capture_out", -1);
            let count = count.min(isize::MAX as usize);
            // SAFETY: writable for `count` bytes, and not otherwise in use.
            let buf = unsafe { slice_mut(buf, count) };
            ssize(GitProcess::read_fd(p.child_out, buf))
        })
    }

    /// `git_process_read_err`.
    ///
    /// # Safety
    ///
    /// As [`git_process_read`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_read_err(
        process: *mut GitProcess,
        buf: *mut c_void,
        count: usize,
    ) -> isize {
        barrier(|| {
            git_assert_arg!(!process.is_null(), "process", -1);
            // SAFETY: the caller's contract.
            let p = unsafe { super::ffi::process(process) };
            git_assert!(p.capture_err, "process->capture_err", -1);
            let count = count.min(isize::MAX as usize);
            // SAFETY: as in `git_process_read`.
            let buf = unsafe { slice_mut(buf, count) };
            ssize(GitProcess::read_fd(p.child_err, buf))
        })
    }

    /// `count` bytes at `buf` (any pointer when `count` is 0).
    ///
    /// # Safety
    ///
    /// When `count > 0`, `buf` is writable for `count` bytes, unaliased.
    unsafe fn slice_mut<'a>(buf: *mut c_void, count: usize) -> &'a mut [u8] {
        if count == 0 || buf.is_null() {
            return &mut [];
        }
        // SAFETY: the caller's contract.
        unsafe { core::slice::from_raw_parts_mut(buf.cast::<u8>(), count) }
    }

    /// `git_process_write`.
    ///
    /// # Safety
    ///
    /// `process` is NULL or a live process; `buf` is readable for `count`
    /// bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_write(
        process: *mut GitProcess,
        buf: *const c_void,
        count: usize,
    ) -> isize {
        barrier(|| {
            git_assert_arg!(!process.is_null(), "process", -1);
            // SAFETY: the caller's contract.
            let p = unsafe { super::ffi::process(process) };
            git_assert!(p.capture_in, "process->capture_in", -1);
            let count = count.min(isize::MAX as usize);
            // SAFETY: readable for `count` bytes.
            let buf = unsafe { crate::util::export::bytes(buf.cast(), count) };
            ssize(p.write(buf))
        })
    }

    /// `git_process_wait`.
    ///
    /// # Safety
    ///
    /// `result` is NULL or writable; `process` a live process.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_wait(
        result: *mut GitProcessResult,
        process: *mut GitProcess,
    ) -> c_int {
        barrier(|| {
            // SAFETY: NULL or writable.
            unsafe { crate::util::export::write_opt(result, GitProcessResult::default()) };
            // SAFETY: the caller's contract.
            match unsafe { super::ffi::process(process) }.wait() {
                Ok(r) => {
                    // SAFETY: NULL or writable.
                    unsafe { crate::util::export::write_opt(result, r) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_process_close_in`.
    ///
    /// # Safety
    ///
    /// `process` is a live process.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_close_in(process: *mut GitProcess) -> c_int {
        // SAFETY: the caller's contract.
        barrier(|| to_status(unsafe { super::ffi::process(process) }.close_in()))
    }

    /// `git_process_close_out`.
    ///
    /// # Safety
    ///
    /// `process` is a live process.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_close_out(process: *mut GitProcess) -> c_int {
        // SAFETY: the caller's contract.
        barrier(|| to_status(unsafe { super::ffi::process(process) }.close_out()))
    }

    /// `git_process_close_err`.
    ///
    /// # Safety
    ///
    /// `process` is a live process.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_close_err(process: *mut GitProcess) -> c_int {
        // SAFETY: the caller's contract.
        barrier(|| to_status(unsafe { super::ffi::process(process) }.close_err()))
    }

    /// `git_process_close`.
    ///
    /// # Safety
    ///
    /// `process` is a live process.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_close(process: *mut GitProcess) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract.
            unsafe { super::ffi::process(process) }.close();
            0
        })
    }

    /// `git_process_result_msg`.
    ///
    /// # Safety
    ///
    /// `out` is a valid `git_str`, `result` a valid result.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_result_msg(
        out: *mut GitStr,
        result: *mut GitProcessResult,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract; distinct objects.
            let out = unsafe { &mut *out };
            // SAFETY: as above.
            let result = unsafe { &*result };
            to_status(result_msg(out, result))
        })
    }

    /// `git_process_free`.
    ///
    /// # Safety
    ///
    /// `process` is NULL or a live process, not used again.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_process_free(process: *mut GitProcess) {
        barrier(|| {
            if !process.is_null() {
                // SAFETY: from `Box::into_raw` in `git_process_new`, freed once.
                drop(unsafe { Box::from_raw(process) });
            }
        })
    }
}

crate::c_layout_test! {
    headers: ["process.h"],
    types: [
        GitProcessResult => "git_process_result" { status, exitcode, signal },
    ],
    asserts: [
        "sizeof(git_process_options) == 16",
        "_Alignof(git_process_options) == 8",
        "offsetof(git_process_options, cwd) == 8",
        "sizeof(git_process_result_status) == 4",
        "GIT_PROCESS_STATUS_NONE == 0 && GIT_PROCESS_STATUS_NORMAL == 1 && GIT_PROCESS_STATUS_ERROR == 2",
    ],
}
