//! A child forked from a thread that holds a thread-local value, while another thread makes
//! and disposes of keys, runs its thread-local destructors at `exit`, as libgit2's
//! `git_process_start` does in its child when `execve` fails. Each child is given two seconds
//! to end; the first that does not makes this exit 1.

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use libgit2::libgit2::src::util::thread::TlsKey;
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, fork};

fn main() -> ExitCode {
    let stop = Arc::new(AtomicBool::new(false));
    let churn = thread::spawn({
        let stop = Arc::clone(&stop);
        move || {
            while !stop.load(Ordering::Relaxed) {
                TlsKey::<u8>::new(None).unwrap().dispose().unwrap();
            }
        }
    });
    let key = TlsKey::<u8>::new(None).unwrap();
    key.set(Some(1)).unwrap();

    let forks = 100;
    for n in 1..=forks {
        // SAFETY: the child calls only `exit`, which runs the thread's destructors.
        match unsafe { fork() }.unwrap() {
            // SAFETY: as above.
            ForkResult::Child => unsafe { nix::libc::exit(0) },
            ForkResult::Parent { child } => {
                let start = Instant::now();
                while waitpid(child, Some(WaitPidFlag::WNOHANG)).unwrap() == WaitStatus::StillAlive {
                    if start.elapsed() > Duration::from_secs(2) {
                        let wchan = std::fs::read_to_string(format!("/proc/{child}/wchan"));
                        let syscall = std::fs::read_to_string(format!("/proc/{child}/syscall"));
                        kill(child, Signal::SIGKILL).unwrap();
                        waitpid(child, None).unwrap();
                        eprintln!(
                            "fork {n} of {forks}: the child did not end within 2 s, blocked in {} \
                             (syscall {})",
                            wchan.unwrap_or_default(),
                            syscall.unwrap_or_default().split_whitespace().next().unwrap_or("?"),
                        );
                        return ExitCode::FAILURE;
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    churn.join().unwrap();
    println!("{forks} forked children ended");
    ExitCode::SUCCESS
}
