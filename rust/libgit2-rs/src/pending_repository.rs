//! PENDING(l-repo): a minimal stand-in for l-repo's safe `Repository`, so
//! that the safe API of commits and trees (which needs a repository to look
//! objects up and to write them) can be written and tested now. It opens a
//! repository through the C symbols and frees it on drop; the integration
//! step replaces it with l-repo's type, which has the same name, so that the
//! users of [`Repository`] (`commit::api`, `tree::api`) only change their
//! `use`.

use core::ptr::NonNull;
use std::ffi::CString;
use std::path::Path;

use crate::sys::pending::l_commit::{self as pend, git_repository};
use crate::util::errors::{Error, ErrorClass, ErrorCode};

/// An open repository.
pub struct Repository {
    raw: NonNull<git_repository>,
}

impl Repository {
    /// `git_repository_open`.
    pub fn open(path: impl AsRef<Path>) -> Result<Repository, Error> {
        use std::os::unix::ffi::OsStrExt;
        let path = CString::new(path.as_ref().as_os_str().as_bytes()).map_err(|_| {
            Error::new(
                ErrorCode::INVALID,
                ErrorClass::INVALID,
                "path contains a NUL byte",
            )
        })?;
        let mut raw = core::ptr::null_mut();
        // SAFETY: `raw` is writable; `path` NUL-terminated.
        let e = unsafe { pend::git_repository_open(&raw mut raw, path.as_ptr()) };
        if e < 0 {
            return Err(Error::from(crate::commit::ec(e)));
        }
        NonNull::new(raw)
            .map(|raw| Repository { raw })
            .ok_or_else(|| Error::from(ErrorCode::GENERIC))
    }

    /// The C repository.
    pub(crate) fn raw(&self) -> *mut git_repository {
        self.raw.as_ptr()
    }
}

impl Drop for Repository {
    fn drop(&mut self) {
        // SAFETY: the repository this value opened, freed once; the objects
        // borrowed from it (lifetime `'r`) are gone.
        unsafe { pend::git_repository_free(self.raw.as_ptr()) }
    }
}
