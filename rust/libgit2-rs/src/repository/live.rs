//! Which repositories are live, for holders of a repository pointer that
//! take no reference to it.
//!
//! C's file config backend keeps the `repo` it is opened with (for
//! `includeIf.gitdir:` / `onbranch:` conditions) without a reference, and
//! reads it on every refresh. A config outlives its repository whenever its
//! user keeps a reference (`git_repository_config` then
//! `git_repository_free`, or `git_config_add_file_ondisk` with a
//! repository freed first); C then reads freed memory (rust/DEFECTS.md).
//!
//! Every repository is registered here from [`GitRepository::alloc`] to
//! the start of [`GitRepository::free`], under a serial number. A holder
//! keeps the pointer and its [`Token`], and reaches the repository only
//! through [`with_live`], which lends it for a closure if it is still the
//! registered repository of that serial. [`unregister`] (the first step of
//! the free) waits until no closure holds it: so a lent repository is live
//! for the whole closure.

use std::sync::{Condvar, Mutex, MutexGuard};

use super::GitRepository;

/// A registered repository: its address, its serial, the closures that
/// hold it, and whether its free has started.
struct Entry {
    addr: usize,
    serial: u64,
    pins: usize,
    dying: bool,
}

struct Registry {
    entries: Vec<Entry>,
    next: u64,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    entries: Vec::new(),
    next: 1,
});
static UNPINNED: Condvar = Condvar::new();

fn lock() -> MutexGuard<'static, Registry> {
    // Nothing below panics while holding the lock but the allocator's OOM,
    // which aborts; poisoning would leave the data consistent anyway.
    REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

/// A repository's identity at the time a holder kept its pointer: an
/// address and a serial, so a later repository at the same address is not
/// taken for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    addr: usize,
    serial: u64,
}

impl Token {
    /// No repository.
    pub(crate) const NONE: Token = Token { addr: 0, serial: 0 };

    /// The serial, to keep in an atomic (0 for none).
    pub(crate) fn serial(self) -> u64 {
        self.serial
    }

    /// The token of `repo` with the kept `serial`.
    pub(crate) fn from_parts(repo: *const GitRepository, serial: u64) -> Token {
        Token {
            addr: repo.addr(),
            serial,
        }
    }
}

/// Register a newly allocated repository.
pub(crate) fn register(repo: *const GitRepository) {
    let mut r = lock();
    let serial = r.next;
    r.next += 1;
    r.entries.push(Entry {
        addr: repo.addr(),
        serial,
        pins: 0,
        dying: false,
    });
}

/// Unregister `repo` before it is freed: no new [`with_live`] lends it,
/// and this waits until none still does.
pub(crate) fn unregister(repo: *const GitRepository) {
    let addr = repo.addr();
    let mut r = lock();
    let Some(i) = r.entries.iter().position(|e| e.addr == addr && !e.dying) else {
        return;
    };
    r.entries[i].dying = true;
    let serial = r.entries[i].serial;
    loop {
        let Some(i) = r
            .entries
            .iter()
            .position(|e| e.addr == addr && e.serial == serial)
        else {
            return;
        };
        if r.entries[i].pins == 0 {
            r.entries.swap_remove(i);
            return;
        }
        r = UNPINNED.wait(r).unwrap_or_else(|e| e.into_inner());
    }
}

/// The token of the live repository `repo` (NONE for NULL or an
/// unregistered one).
pub(crate) fn token(repo: *const GitRepository) -> Token {
    if repo.is_null() {
        return Token::NONE;
    }
    let r = lock();
    r.entries
        .iter()
        .find(|e| e.addr == repo.addr() && !e.dying)
        .map_or(Token::NONE, |e| Token {
            addr: e.addr,
            serial: e.serial,
        })
}

/// Unpins on drop (also when the closure panics).
struct Pin {
    addr: usize,
    serial: u64,
}

impl Drop for Pin {
    fn drop(&mut self) {
        let mut r = lock();
        if let Some(e) = r
            .entries
            .iter_mut()
            .find(|e| e.addr == self.addr && e.serial == self.serial)
        {
            e.pins -= 1;
        }
        drop(r);
        UNPINNED.notify_all();
    }
}

/// Run `f` with the repository `repo` if it is still the live repository
/// of `tok` (its free not started), and with `None` otherwise. The
/// repository's free waits for `f` to return.
pub(crate) fn with_live<R>(
    repo: *const GitRepository,
    tok: Token,
    f: impl FnOnce(Option<&GitRepository>) -> R,
) -> R {
    if repo.is_null() || tok.serial == 0 || repo.addr() != tok.addr {
        return f(None);
    }
    let pinned = {
        let mut r = lock();
        match r
            .entries
            .iter_mut()
            .find(|e| e.addr == tok.addr && e.serial == tok.serial && !e.dying)
        {
            Some(e) => {
                e.pins += 1;
                true
            }
            None => false,
        }
    };
    if !pinned {
        return f(None);
    }
    let _pin = Pin {
        addr: tok.addr,
        serial: tok.serial,
    };
    // SAFETY: `repo` is the registered repository of `tok`: it was
    // allocated by `GitRepository::alloc` (which registers it) and its
    // `free` has not passed `unregister`, which waits for this pin, so it
    // stays allocated and initialized until `_pin` drops after `f`.
    // `GitRepository` is only read through `&` (interior-mutable) and is
    // `Sync`.
    f(Some(unsafe { &*repo }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lent_only_while_registered() {
        let fake = core::ptr::without_provenance::<GitRepository>(0x1000);
        // Tokens of an unregistered address are NONE; nothing is lent.
        assert_eq!(token(fake), Token::NONE);
        assert!(with_live(fake, Token::NONE, |r| r.is_none()));
        assert!(with_live(fake, Token::from_parts(fake, 7), |r| r.is_none()));
    }

    #[test]
    fn registration_and_serials() {
        // Only the address is used: nothing dereferences it.
        let b = Box::new(0u64);
        let p = core::ptr::from_ref::<u64>(&b).cast::<GitRepository>();
        register(p);
        let t1 = token(p);
        assert_ne!(t1, Token::NONE);
        unregister(p);
        assert_eq!(token(p), Token::NONE);
        // The same address again: a new serial, so the old token is stale.
        register(p);
        let t2 = token(p);
        assert_ne!(t2, t1);
        unregister(p);
        // `with_live` of a stale token lends nothing (and so does not
        // dereference the fake pointer).
        assert!(with_live(p, t1, |r| r.is_none()));
    }

    #[test]
    #[cfg_attr(miri, ignore)] // the repository's free reaches C
    fn a_real_repository_is_lent_until_freed() {
        let _g = crate::util::test_support::init();
        let repo = GitRepository::alloc();
        assert!(!repo.is_null());
        let tok = token(repo);
        assert_ne!(tok, Token::NONE);
        assert!(with_live(repo, tok, |r| r.is_some()));
        // SAFETY: the repository just allocated, used by nothing else.
        unsafe { GitRepository::free(repo) };
        // (The address is not dereferenced: the serial is gone.)
        assert!(with_live(repo, tok, |r| r.is_none()));
    }
}
