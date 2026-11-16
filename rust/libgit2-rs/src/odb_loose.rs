//! Port of `src/libgit2/odb_loose.c`: the loose object backend
//! (`objects/xx/yyyy…`, zlib-deflated `"<type> <size>\0<data>"`, and the
//! old pack-like format, read only).
//!
//! The backend is a [`CTable`] over `git_odb_backend` (C's
//! `loose_backend`, whose layout no C outside this file saw): C, and the
//! odb, call its slots. Its state, [`Loose`], is immutable once made, so
//! every slot takes `&Loose` and the backend is used from any thread, as
//! C's was. Its read and write streams are `CTable`s over `git_odb_stream`.
//!
//! Files are written through `git_filebuf` (a temporary file deflated with
//! the system zlib at the backend's compression level, renamed into
//! place), so the bytes on disk, the modes and the fsync behaviour are C's.

use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::ptr::NonNull;
use std::sync::{Mutex, PoisonError};

use ffibox::CTable;

use crate::odb::{
    BackendTable, GIT_OBJECT_DIR_MODE, GIT_OBJECT_FILE_MODE, GIT_ODB_BACKEND_LOOSE_FSYNC,
    GitOdbBackend, GitOdbBackendLooseOptions, GitOdbStream, StreamTable, backend_base, busy,
    error_ambiguous, error_notfound, git_object_size_t, git_object_t, stringn2type, type_is_valid,
};
use crate::oid::{GIT_OID_MINPREFIXLEN, Oid, OidType, git_oid_t, oid_hexsize};
use crate::util::alloc::{self, GitAlloc};
use crate::util::errors::{self, ErrorClass, ErrorCode, Result};
use crate::util::filebuf::{
    GIT_FILEBUF_DEFLATE_SHIFT, GIT_FILEBUF_FSYNC, GIT_FILEBUF_TEMPORARY, GitFilebuf,
};
use crate::util::fs_path;
use crate::util::futils::{
    self, GIT_MKDIR_PATH, GIT_MKDIR_SKIP_LAST, GIT_MKDIR_VERIFY_DIR, MkdirOpts,
};
use crate::util::posix::{self, GitMap};
use crate::util::str::GitStr;
use crate::util::zstream::{GitZstream, ZStream, ZstreamKind, ZstreamPtr};
use crate::{git_assert, git_assert_arg, git_error_set};

/// `MAX_HEADER_LEN`.
const MAX_HEADER_LEN: usize = 64;

/// `Z_BEST_SPEED`.
const Z_BEST_SPEED: c_int = 1;

/// `obj_hdr`: an object's type and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjHdr {
    /// The type.
    pub type_: git_object_t,
    /// The size.
    pub size: usize,
}

/// The loose backend's state (`loose_backend` but for its table).
#[derive(Debug)]
pub struct Loose {
    options: GitOdbBackendLooseOptions,
    oid_hexsize: usize,
    /// The objects directory, with a trailing `/`.
    objects_dir: Vec<u8>,
}

/// `git__fromhex` as a table: a digit's value, or -1.
static FROM_HEX: [i8; 256] = {
    let mut t = [-1i8; 256];
    let mut c = 0;
    while c < 256 {
        t[c] = crate::util::util::fromhex(c as u8) as i8;
        c += 1;
    }
    t
};

/// `(git__fromhex(a) << 4) | git__fromhex(b)`, when both are digits.
#[inline(always)]
fn hex_pair(a: u8, b: u8) -> Option<u8> {
    let v = (i32::from(FROM_HEX[usize::from(a)]) << 4) | i32::from(FROM_HEX[usize::from(b)]);
    (v >= 0).then_some(v as u8)
}

/// The digits of the hex alphabet.
const HEX: &[u8; 16] = b"0123456789abcdef";

impl Loose {
    /// `git_odb__backend_loose`'s state: `objects_dir` with a `/`, the
    /// options normalized.
    pub fn new(objects_dir: &[u8], opts: Option<&GitOdbBackendLooseOptions>) -> Result<Loose> {
        let mut dir = Vec::new();
        dir.try_reserve_exact(objects_dir.len() + 2)
            .map_err(|_| errors::oom())?;
        dir.extend_from_slice(objects_dir);
        // C read `objects_dir[-1]` for an empty directory (rust/DEFECTS.md).
        if dir.last() != Some(&b'/') {
            dir.push(b'/');
        }
        let mut options = opts.copied().unwrap_or_default();
        if options.compression_level < 0 {
            options.compression_level = Z_BEST_SPEED;
        }
        if options.dir_mode == 0 {
            options.dir_mode = GIT_OBJECT_DIR_MODE;
        }
        if options.file_mode == 0 {
            options.file_mode = GIT_OBJECT_FILE_MODE;
        }
        if options.oid_type == 0 {
            options.oid_type = OidType::DEFAULT.raw();
        }
        Ok(Loose {
            oid_hexsize: oid_hexsize(options.oid_type),
            options,
            objects_dir: dir,
        })
    }

    /// The normalized options.
    pub fn options(&self) -> &GitOdbBackendLooseOptions {
        &self.options
    }

    /// The objects directory (with its trailing `/`).
    pub fn objects_dir(&self) -> &[u8] {
        &self.objects_dir
    }

    fn oid_type(&self) -> git_oid_t {
        self.options.oid_type
    }

    /// `object_file_name`: `<objects>/xx/yyyy…` for `id`, with the
    /// backend's hex size (C's buffer arithmetic uses it, the digits come
    /// from `id`).
    fn object_file_name(&self, name: &mut GitStr, id: &Oid) -> Result<()> {
        name.set(&self.objects_dir)?;
        fs_path::to_dir(name)?;
        let n = self.oid_hexsize;
        let mut path = [0u8; crate::oid::GIT_OID_MAX_HEXSIZE + 1];
        let raw = id.raw_id();
        let mut o = 0;
        for i in 0..n {
            if i == 2 {
                path[o] = b'/';
                o += 1;
            }
            let b = raw[i / 2];
            path[o] = HEX[usize::from(if i % 2 == 0 { b >> 4 } else { b & 15 })];
            o += 1;
        }
        if n < 2 {
            // An unknown type: C overflowed its path buffer here
            // (rust/DEFECTS.md).
            return Err(git_error_set!(ErrorClass::INVALID, "unknown oid type"));
        }
        name.put(&path[..o])
    }

    /// `object_mkdir`.
    fn object_mkdir(&self, name: &GitStr) -> Result<()> {
        let rel = name.as_bytes().get(self.objects_dir.len()..).unwrap_or(&[]);
        futils::mkdir_relative(
            rel,
            Some(&self.objects_dir),
            self.options.dir_mode,
            GIT_MKDIR_PATH | GIT_MKDIR_SKIP_LAST | GIT_MKDIR_VERIFY_DIR,
            &mut MkdirOpts::default(),
        )
    }

    /// `locate_object`: the path of `id`, or `NOT_FOUND` (no message).
    fn locate_object(&self, loc: &mut GitStr, id: &Oid) -> Result<()> {
        self.object_file_name(loc, id)?;
        if !path_exists(loc) {
            return Err(ErrorCode::NOT_FOUND);
        }
        Ok(())
    }

    /// The not-found error of a full id.
    fn notfound(&self, id: &Oid) -> ErrorCode {
        error_notfound(b"no matching loose object", Some(id), self.oid_hexsize)
    }

    /// `locate_object_short_oid`: the path and id of the one object whose
    /// id starts with the first `len` digits of `short_oid`.
    fn locate_object_short_oid(
        &self,
        loc: &mut GitStr,
        short_oid: &Oid,
        len: usize,
    ) -> Result<Oid> {
        let hexsize = self.oid_hexsize;
        let alloc_len = crate::util::integer::alloc_add(self.objects_dir.len(), hexsize)?;
        let alloc_len = crate::util::integer::alloc_add(alloc_len, 3)?;
        loc.grow(alloc_len)?;
        loc.set(&self.objects_dir)?;
        fs_path::to_dir(loc)?;
        let dir_len = loc.len();

        let short_hex = short_oid.hex();
        let mut short = [0u8; crate::oid::GIT_OID_MAX_HEXSIZE];
        short[..short_hex.as_bytes().len()].copy_from_slice(short_hex.as_bytes());

        loc.put(&short[..3])?;
        let n = loc.len();
        loc.as_mut_bytes()[n - 1] = b'/';

        if !path_isdir(loc) {
            return Err(error_notfound(
                b"no matching loose object for prefix",
                Some(short_oid),
                len,
            ));
        }

        let entry_dir_len = loc.len();
        let mut found = 0u32;
        let mut res = [0u8; crate::oid::GIT_OID_MAX_HEXSIZE];
        let cmp_len = len.saturating_sub(2).min(short.len() - 2);
        let r = fs_path::direach(loc, 0, &mut |path: &mut GitStr| {
            let p = path.as_bytes();
            if p.len().wrapping_sub(entry_dir_len) != hexsize.wrapping_sub(2) {
                return 0;
            }
            if !path_isdir(path) {
                let name = &path.as_bytes()[entry_dir_len..];
                if name.get(..cmp_len) == Some(&short[2..2 + cmp_len]) {
                    if found == 0 {
                        res[0] = short[0];
                        res[1] = short[1];
                        res[2..hexsize].copy_from_slice(&name[..hexsize - 2]);
                    }
                    found += 1;
                }
            }
            if found > 1 {
                return ErrorCode::AMBIGUOUS.raw();
            }
            0
        });
        if let Err(e) = r
            && e != ErrorCode::AMBIGUOUS
        {
            return Err(e);
        }
        if found == 0 {
            return Err(error_notfound(
                b"no matching loose object for prefix",
                Some(short_oid),
                len,
            ));
        }
        if found > 1 {
            return Err(error_ambiguous(b"multiple matches in loose objects"));
        }
        let kind = OidType::from_raw(self.oid_type()).ok_or(ErrorCode::GENERIC)?;
        let res_oid = Oid::from_prefix(&res[..kind.hexsize()], kind)?;

        loc.truncate(dir_len);
        self.object_file_name(loc, &res_oid)?;
        Ok(res_oid)
    }

    /// `filebuf_flags`.
    fn filebuf_flags(&self) -> c_int {
        let mut flags =
            GIT_FILEBUF_TEMPORARY | (self.options.compression_level << GIT_FILEBUF_DEFLATE_SHIFT);
        if self.options.flags & GIT_ODB_BACKEND_LOOSE_FSYNC != 0
            || crate::sys::pending::repository_fsync_gitdir()
        {
            flags |= GIT_FILEBUF_FSYNC;
        }
        flags
    }

    /// The temporary file's path, `<objects>/tmp_object`.
    fn tmp_path(&self) -> Result<GitStr> {
        let mut p = GitStr::new();
        p.joinpath(&self.objects_dir, b"tmp_object")?;
        Ok(p)
    }

    // -- the backend's operations ------------------------------------------

    /// `loose_backend__read_header`.
    pub fn read_header(&self, oid: &Oid) -> Result<ObjHdr> {
        let mut path = GitStr::new();
        if self.locate_object(&mut path, oid).is_err() {
            return Err(self.notfound(oid));
        }
        read_header_loose(&path)
    }

    /// `loose_backend__read`: the data (`git__allocator`, `size + 1`
    /// bytes, NUL-terminated), its size and type.
    pub fn read(&self, oid: &Oid) -> Result<RawData> {
        let mut path = GitStr::new();
        if self.locate_object(&mut path, oid).is_err() {
            return Err(self.notfound(oid));
        }
        read_loose(&path)
    }

    /// `loose_backend__read_prefix`.
    /// The full id goes to `out_oid` as soon as it is known (C wrote it
    /// before reading the object, so it is set when the read fails).
    pub fn read_prefix(
        &self,
        short_oid: &Oid,
        len: usize,
        out_oid: &mut Option<Oid>,
    ) -> Result<RawData> {
        git_assert_arg!(
            len >= GIT_OID_MINPREFIXLEN && len <= self.oid_hexsize,
            "len >= 4 && len <= backend->oid_hexsize"
        );
        if len == self.oid_hexsize {
            let raw = self.read(short_oid)?;
            *out_oid = Some(*short_oid);
            return Ok(raw);
        }
        let mut path = GitStr::new();
        let id = self.locate_object_short_oid(&mut path, short_oid, len)?;
        *out_oid = Some(id);
        read_loose(&path)
    }

    /// `loose_backend__exists`.
    pub fn exists(&self, oid: &Oid) -> bool {
        let mut path = GitStr::new();
        self.locate_object(&mut path, oid).is_ok()
    }

    /// `loose_backend__exists_prefix`.
    pub fn exists_prefix(&self, short_id: &Oid, len: usize) -> Result<Oid> {
        git_assert_arg!(len >= GIT_OID_MINPREFIXLEN, "len >= 4");
        let mut path = GitStr::new();
        self.locate_object_short_oid(&mut path, short_id, len)
    }

    /// `loose_backend__foreach`: `cb` for each object, in directory order;
    /// a non-zero return stops the walk and is returned.
    pub fn foreach(&self, cb: &mut dyn FnMut(&Oid) -> c_int) -> c_int {
        let mut buf = GitStr::new();
        let r = buf
            .sets(&self.objects_dir)
            .and_then(|()| fs_path::to_dir(&mut buf));
        if r.is_err() || buf.is_oom() {
            return -1;
        }
        let dir_len = buf.len();
        let r = fs_path::direach(&mut buf, 0, &mut |path: &mut GitStr| {
            if !path_isdir(path) {
                return 0;
            }
            let r = fs_path::direach(path, 0, &mut |path: &mut GitStr| {
                let Some(oid) = self.filename_to_oid(&path.as_bytes()[dir_len..]) else {
                    return 0;
                };
                errors::set_after_callback(cb(&oid), "git_odb_foreach")
            });
            match r {
                Ok(()) => 0,
                Err(e) => e.raw(),
            }
        });
        match r {
            Ok(()) => 0,
            Err(e) => e.raw(),
        }
    }

    /// `filename_to_oid`: the id of `"xx/yyyy…"`.
    fn filename_to_oid(&self, name: &[u8]) -> Option<Oid> {
        let hexsize = self.oid_hexsize;
        if hexsize < 2 || name.len() != hexsize + 1 || name[2] != b'/' {
            return None;
        }
        let mut id = [0u8; crate::oid::GIT_OID_MAX_SIZE];
        let (first, rest) = id.split_at_mut(1);
        first[0] = hex_pair(name[0], name[1])?;
        let digits = &name[3..];
        for (out, pair) in rest.iter_mut().zip(digits.as_chunks::<2>().0) {
            *out = hex_pair(pair[0], pair[1])?;
        }
        Some(Oid::from_raw_parts(self.oid_type() as u8, id))
    }

    /// `loose_backend__write`.
    pub fn write(&self, oid: &Oid, data: &[u8], type_: git_object_t) -> Result<()> {
        let mut header = [0u8; MAX_HEADER_LEN];
        let header_len = crate::odb::format_object_header(&mut header, data.len() as u64, type_)?;
        let mut final_path = self.tmp_path().map_err(|_| ErrorCode::GENERIC)?;
        let mut fbuf = OpenFilebuf::new();
        // SAFETY: the filebuf is boxed: it does not move until cleaned up
        // (by the commit or `OpenFilebuf`'s drop).
        if unsafe {
            fbuf.0.open(
                final_path.as_bytes(),
                self.filebuf_flags(),
                self.options.file_mode,
            )
        }
        .is_err()
        {
            return Err(ErrorCode::GENERIC);
        }
        // C ignores these results: a failed write fails the commit.
        let _ = fbuf.0.write(&header[..header_len]);
        let _ = fbuf.0.write(data);
        if self.object_file_name(&mut final_path, oid).is_err()
            || self.object_mkdir(&final_path).is_err()
            || commit_at(&mut fbuf.0, &final_path).is_err()
        {
            return Err(ErrorCode::GENERIC);
        }
        Ok(())
    }

    /// `loose_backend__freshen`.
    pub fn freshen(&self, oid: &Oid) -> Result<()> {
        let mut path = GitStr::new();
        if self.object_file_name(&mut path, oid).is_err() {
            return Err(ErrorCode::GENERIC);
        }
        futils::touch(c_path(&path)?, None)
    }
}

/// A `GitFilebuf` on the heap (zlib keeps its address), cleaned up on drop.
struct OpenFilebuf(Box<GitFilebuf>);

impl OpenFilebuf {
    fn new() -> OpenFilebuf {
        OpenFilebuf(Box::default())
    }
}

impl Drop for OpenFilebuf {
    fn drop(&mut self) {
        // After a commit (which cleans up), this does nothing.
        self.0.cleanup();
    }
}

/// `git_filebuf_commit_at(fbuf, path)`.
fn commit_at(fbuf: &mut GitFilebuf, path: &GitStr) -> Result<()> {
    fbuf.commit_at(c_path(path)?)
}

/// The string as a C path.
fn c_path(p: &GitStr) -> Result<&CStr> {
    p.as_c_str().ok_or(ErrorCode::GENERIC)
}

fn path_exists(p: &GitStr) -> bool {
    p.as_c_str().is_some_and(fs_path::exists)
}

fn path_isdir(p: &GitStr) -> bool {
    p.as_c_str().is_some_and(fs_path::isdir)
}

/// Object data in `git__allocator` memory, as backends return it.
pub type RawData = (ffibox::CVec<u8, GitAlloc>, usize, git_object_t);

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

fn invalid_header() -> ErrorCode {
    git_error_set!(
        ErrorClass::OBJECT,
        "failed to parse loose object: invalid header"
    )
}

/// `parse_header_packlike`: the header and its length.
pub(crate) fn parse_header_packlike(data: &[u8]) -> Result<(ObjHdr, usize)> {
    let Some(&first) = data.first() else {
        return Err(invalid_header());
    };
    let mut used = 1;
    let mut c = u64::from(first);
    let type_ = ((c >> 4) & 7) as git_object_t;
    let mut size = c & 15;
    let mut shift = 4u32;
    while c & 0x80 != 0 {
        if data.len() <= used {
            return Err(invalid_header());
        }
        if usize::BITS <= shift {
            return Err(invalid_header());
        }
        c = u64::from(data[used]);
        used += 1;
        size = size.wrapping_add((c & 0x7f) << shift);
        shift += 7;
    }
    Ok((
        ObjHdr {
            type_,
            size: size as usize,
        },
        used,
    ))
}

/// `parse_header`: `"<type> <size>\0"`; the header and its length.
pub(crate) fn parse_header(data: &[u8]) -> Result<(ObjHdr, usize)> {
    let Some(typename_len) = memchr::memchr(b' ', data) else {
        return Err(invalid_header());
    };
    let type_ = stringn2type(&data[..typename_len]);
    let size_idx = typename_len + 1;
    let Some(size_len) = memchr::memchr(0, &data[size_idx..]) else {
        return Err(invalid_header());
    };
    let i = size_idx + size_len;
    let size = match crate::util::util::strntol64(&data[size_idx..i], 10) {
        Ok((n, _)) if n >= 0 => n,
        _ => return Err(invalid_header()),
    };
    Ok((
        ObjHdr {
            type_,
            size: size as usize,
        },
        i + 1,
    ))
}

/// `is_zlib_compressed_data`.
pub(crate) fn is_zlib_compressed_data(data: &[u8]) -> bool {
    if data.len() < 2 {
        return false;
    }
    let w = (u32::from(data[0]) << 8) + u32::from(data[1]);
    (data[0] & 0x8F) == 0x08 && w % 31 == 0
}

/// A zeroed `git__allocator` buffer of `n` bytes.
fn calloc_buf(n: usize) -> Result<ffibox::CVec<u8, GitAlloc>> {
    let p = alloc::calloc(1, n).cast::<u8>();
    if p.is_null() {
        return Err(ErrorCode::GENERIC);
    }
    // SAFETY: a fresh zeroed `git__allocator` allocation of `n` bytes, whose
    // ownership passes to the vector.
    unsafe { ffibox::CVec::from_raw_parts(p, n) }.ok_or(ErrorCode::GENERIC)
}

/// `read_loose_packlike`.
fn read_loose_packlike(obj: &[u8]) -> Result<RawData> {
    let (hdr, head_len) = parse_header_packlike(obj)?;
    if !type_is_valid(hdr.type_) || head_len > obj.len() {
        return Err(git_error_set!(
            ErrorClass::ODB,
            "failed to inflate loose object"
        ));
    }
    let data = &obj[head_len..];
    let alloc_size = crate::util::integer::alloc_add(hdr.size, 1)?;
    let mut body = GitStr::with_capacity(alloc_size)?;
    crate::util::zstream::inflate_buf(&mut body, data)?;
    // C hands out `hdr.size` bytes whatever was inflated: the rest of a
    // short body was uninitialized there, zero here (rust/DEFECTS.md).
    if body.len() < hdr.size {
        body.putcn(0, hdr.size - body.len())?;
    }
    // The string's own buffer (C's `git_str_detach`): `len + 1 >=
    // hdr.size + 1` initialized bytes, NUL-terminated.
    let n = body.len() + 1;
    let p = body.detach().ok_or(ErrorCode::GENERIC)?.into_raw();
    // SAFETY: a `git__allocator` buffer of at least `n` initialized bytes
    // (the contents and the NUL), whose ownership passes to the vector.
    let out =
        unsafe { ffibox::CVec::from_raw_parts(p.cast::<u8>(), n) }.ok_or(ErrorCode::GENERIC)?;
    Ok((out, hdr.size, hdr.type_))
}

/// `read_loose_standard`.
fn read_loose_standard(obj: &[u8]) -> Result<RawData> {
    let mut zs = ZStream::new_bulk(ZstreamKind::Inflate)?;
    zs.set_input(obj);
    let mut head = [0u8; MAX_HEADER_LEN];
    let mut decompressed = zs.get_output(&mut head)?;
    let (hdr, head_len) = parse_header(&head[..decompressed])?;
    git_assert!(head_len <= decompressed, "head_len <= decompressed");
    if !type_is_valid(hdr.type_) {
        return Err(git_error_set!(
            ErrorClass::ODB,
            "failed to inflate disk object"
        ));
    }
    let alloc_size = crate::util::integer::alloc_add(hdr.size, 1)?;
    let mut body = calloc_buf(alloc_size)?;
    let b = body.as_mut_slice();
    decompressed -= head_len;
    if decompressed > hdr.size {
        return Err(git_error_set!(
            ErrorClass::ODB,
            "malformed object: body was longer than specified in header"
        ));
    }
    b[..decompressed].copy_from_slice(&head[head_len..head_len + decompressed]);
    let got = zs.get_output(&mut b[decompressed..hdr.size])?;
    if !zs.done() {
        return Err(git_error_set!(
            ErrorClass::ZLIB,
            "failed to finish zlib inflation: stream aborted prematurely"
        ));
    }
    // Short bodies are zero-padded (the buffer is zeroed), and terminated.
    b[decompressed + got] = 0;
    Ok((body, hdr.size, hdr.type_))
}

/// `read_loose`.
fn read_loose(loc: &GitStr) -> Result<RawData> {
    if loc.is_oom() {
        return Err(ErrorCode::GENERIC);
    }
    let mut obj = GitStr::new();
    futils::readbuffer(&mut obj, c_path(loc)?)?;
    let data = obj.as_bytes();
    if is_zlib_compressed_data(data) {
        read_loose_standard(data)
    } else {
        read_loose_packlike(data)
    }
}

/// `read_header_loose_standard`.
fn read_header_loose_standard(data: &[u8]) -> Result<ObjHdr> {
    let mut zs = ZStream::new(ZstreamKind::Inflate)?;
    zs.set_input(data);
    let mut inflated = [0u8; MAX_HEADER_LEN];
    let n = zs.get_output_chunk(&mut inflated)?;
    parse_header(&inflated[..n]).map(|(h, _)| h)
}

/// `read_header_loose`: the header from the first 1024 bytes of the file.
fn read_header_loose(loc: &GitStr) -> Result<ObjHdr> {
    if loc.is_oom() {
        return Err(ErrorCode::GENERIC);
    }
    let fd = futils::open_ro(c_path(loc)?)?;
    let r = (|| {
        let mut obj = [0u8; 1024];
        let n = posix::read(fd, &mut obj).map_err(|_| ErrorCode::GENERIC)?;
        let obj = &obj[..n];
        let hdr = if is_zlib_compressed_data(obj) {
            read_header_loose_standard(obj)?
        } else {
            parse_header_packlike(obj)?.0
        };
        if !type_is_valid(hdr.type_) {
            return Err(git_error_set!(
                ErrorClass::ZLIB,
                "failed to read loose object header"
            ));
        }
        Ok(hdr)
    })();
    // SAFETY: the descriptor opened above, closed once.
    let _ = unsafe { posix::close(fd) };
    r
}

// ---------------------------------------------------------------------------
// Streams
// ---------------------------------------------------------------------------

/// `loose_writestream`: the temporary file being written.
struct LooseWriteStream {
    fbuf: Mutex<OpenFilebuf>,
    backend: std::sync::Arc<Loose>,
}

// SAFETY: a filebuf owns its buffers, descriptor and paths (`Send`); the
// mutex makes the state `Sync`.
unsafe impl Send for OpenFilebuf {}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each slot of a stream the stream, first, and
    // calls `free` on every stream once; `read` is NULL (write only).
    unsafe impl CVtable<LooseWriteStream> for StreamTable {}
    base = GitOdbStream { mode: crate::odb::GIT_STREAM_WRONLY, ..GitOdbStream::default() };
    /// `loose_backend__writestream_write`.
    fn write(@stream, data: *const c_char, len: usize) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseWriteStream| {
            let mut f = s.fbuf.lock().unwrap_or_else(PoisonError::into_inner);
            // SAFETY: C's contract: `data` is readable for `len` bytes.
            let data = unsafe { crate::util::export::bytes(data, len) };
            crate::util::export::status(f.0.write(data))
        },
    }
    /// `loose_backend__writestream_finalize`.
    fn finalize_write(@stream, oid: *const Oid) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseWriteStream| {
            let mut f = s.fbuf.lock().unwrap_or_else(PoisonError::into_inner);
            let mut final_path = GitStr::new();
            // SAFETY: C's contract: a readable id.
            let oid = unsafe { &*oid };
            if s.backend.object_file_name(&mut final_path, oid).is_err()
                || s.backend.object_mkdir(&final_path).is_err()
            {
                return -1;
            }
            crate::util::export::status(commit_at(&mut f.0, &final_path))
        },
    }
    release free;
}

/// `loose_readstream`: the mapped file, the inflate stream reading it, and
/// the start of the data that came with the header.
struct ReadState {
    map: GitMap,
    zstream: GitZstream,
    start: [u8; MAX_HEADER_LEN],
    start_len: usize,
    start_read: usize,
}

// SAFETY: the state owns its mapping and its inflate state (zlib-rs's, not
// tied to a thread); the stream's input points into the mapping.
unsafe impl Send for ReadState {}

impl ReadState {
    /// The inflate stream, reached through one raw pointer (zstream's
    /// rule), re-derived for each use from the state, which the stream's
    /// table holds in place (it never moves).
    fn zs(&mut self) -> ZstreamPtr<'_> {
        // SAFETY: the stream is valid (zeroed or initialized), reached only
        // through this pointer while `self` is borrowed, from the thread
        // holding the state's lock, and never moved (the state lives in
        // the table's allocation).
        unsafe { ZstreamPtr::new(&raw mut self.zstream) }
    }
}

impl Drop for ReadState {
    fn drop(&mut self) {
        // `git_futils_mmap_free`, `git_zstream_free`.
        self.zs().free();
        // SAFETY: the mapping is this state's (or empty); the stream that
        // read it is freed.
        unsafe { posix::munmap(&mut self.map) };
    }
}

struct LooseReadStream {
    state: Mutex<ReadState>,
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each slot of a stream the stream, first, and
    // calls `free` on every stream once; the write slots are NULL (read
    // only), which C's contract allows.
    unsafe impl CVtable<LooseReadStream> for StreamTable {}
    base = GitOdbStream::default();
    /// `loose_backend__readstream_read`.
    fn read(@stream, buffer: *mut c_char, buffer_len: usize) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseReadStream| {
            let mut st = s.state.lock().unwrap_or_else(PoisonError::into_inner);
            let st = &mut *st;
            let mut buffer = buffer.cast::<u8>();
            let mut buffer_len = buffer_len.min(c_int::MAX as usize);
            let mut total = 0usize;
            let start_remain = st.start_len - st.start_read;
            if start_remain > 0 && buffer_len > 0 {
                let chunk = start_remain.min(buffer_len);
                // SAFETY: C's contract: `buffer` is writable for
                // `buffer_len` bytes; `start` is the state's.
                unsafe { core::ptr::copy_nonoverlapping(st.start[st.start_read..].as_ptr(), buffer, chunk) };
                buffer = buffer.wrapping_add(chunk);
                st.start_read += chunk;
                total += chunk;
                buffer_len -= chunk;
            }
            if buffer_len > 0 {
                let mut chunk = buffer_len;
                // SAFETY: the input is the mapping, alive in the state and
                // never written; `buffer` is writable for `chunk` bytes
                // (C's contract) and distinct from both.
                if let Err(e) = unsafe { st.zs().get_output(buffer, &mut chunk) } {
                    return e.raw();
                }
                total += chunk;
            }
            total as c_int
        },
    }
    release free;
}

impl Loose {
    /// `loose_backend__writestream`.
    fn writestream(
        self: &std::sync::Arc<Self>,
        length: git_object_size_t,
        type_: git_object_t,
    ) -> Result<NonNull<GitOdbStream>> {
        let mut hdr = [0u8; MAX_HEADER_LEN];
        let hdrlen = crate::odb::format_object_header(&mut hdr, length, type_)?;
        let t = CTable::<StreamTable, LooseWriteStream>::new(LooseWriteStream {
            fbuf: Mutex::new(OpenFilebuf::new()),
            backend: self.clone(),
        });
        let mut t = t;
        let ok = {
            let f = t
                .state_mut()
                .fbuf
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner);
            self.tmp_path().is_ok_and(|p| {
                // SAFETY: the filebuf is boxed in the stream's state: it does
                // not move until cleaned up (commit or the stream's release).
                unsafe {
                    f.0.open(p.as_bytes(), self.filebuf_flags(), self.options.file_mode)
                }
                .is_ok()
                    && f.0.write(&hdr[..hdrlen]).is_ok()
            })
        };
        if !ok {
            return Err(ErrorCode::GENERIC);
        }
        Ok(t.into_c())
    }

    /// `loose_backend__readstream`: the stream, the size and the type.
    fn readstream(&self, oid: &Oid) -> Result<(NonNull<GitOdbStream>, usize, git_object_t)> {
        let mut path = GitStr::new();
        if self.locate_object(&mut path, oid).is_err() {
            return Err(self.notfound(oid));
        }
        // `git__malloc` (8-aligned): `init_raw` writes it in place.
        let ctx = alloc::malloc(size_of::<crate::util::hash::HashCtx>())
            .cast::<crate::util::hash::HashCtx>();
        if ctx.is_null() {
            return Err(ErrorCode::GENERIC);
        }
        let t = CTable::<StreamTable, LooseReadStream>::new(LooseReadStream {
            state: Mutex::new(ReadState {
                map: GitMap::default(),
                zstream: GitZstream::new(),
                start: [0; MAX_HEADER_LEN],
                start_len: 0,
                start_read: 0,
            }),
        });
        // Freed unless handed to the stream.
        struct Ctx(*mut crate::util::hash::HashCtx);
        impl Drop for Ctx {
            fn drop(&mut self) {
                // SAFETY: a context of ours (`git__allocator`), freed once.
                unsafe { alloc::free(self.0.cast()) }
            }
        }
        let ctx = Ctx(ctx);
        // SAFETY: a fresh context of ours, written in place.
        unsafe {
            crate::util::hash::HashCtx::init_raw(ctx.0, crate::odb::oid_algorithm(self.oid_type()))
        }?;
        let mut t = t;
        let hdr = {
            let st = t
                .state_mut()
                .state
                .get_mut()
                .unwrap_or_else(PoisonError::into_inner);
            st.map = futils::mmap_ro_file(c_path(&path)?)?;
            // In place: the stream does not move until freed.
            st.zs().init_bulk(ZstreamKind::Inflate as c_int)?;
            // SAFETY: the mapping's bytes, read only, alive while the state
            // is (the stream's input points into them until it is freed).
            let data = unsafe { map_bytes(st.map.data, st.map.len) };
            if is_zlib_compressed_data(data) {
                readstream_standard(st, data)?
            } else {
                readstream_packlike(st, data)?
            }
        };
        let p = t.into_c();
        let ctx = core::mem::ManuallyDrop::new(ctx).0;
        // SAFETY: the fresh table's fields, before C sees it.
        unsafe { (*p.as_ptr()).hash_ctx = ctx.cast::<c_void>() };
        Ok((p, hdr.size, hdr.type_))
    }
}

/// The bytes of a read-only mapping.
///
/// # Safety
///
/// `data` and `len` are a live private read-only mapping (or empty), not
/// unmapped while the slice lives.
unsafe fn map_bytes<'a>(data: *const c_void, len: usize) -> &'a [u8] {
    // SAFETY: the caller's contract; nothing writes a private read-only
    // mapping.
    unsafe { crate::util::export::bytes(data.cast(), len) }
}

/// `loose_backend__readstream_packlike`.
fn readstream_packlike(st: &mut ReadState, data: &[u8]) -> Result<ObjHdr> {
    let (hdr, head_len) = parse_header_packlike(data)?;
    if !type_is_valid(hdr.type_) {
        return Err(git_error_set!(
            ErrorClass::ODB,
            "failed to inflate loose object"
        ));
    }
    let rest = &data[head_len..];
    st.zs().set_input(rest.as_ptr().cast(), rest.len());
    Ok(hdr)
}

/// `loose_backend__readstream_standard`.
fn readstream_standard(st: &mut ReadState, data: &[u8]) -> Result<ObjHdr> {
    st.zs().set_input(data.as_ptr().cast(), data.len());
    let mut head = [0u8; MAX_HEADER_LEN];
    let mut init = head.len();
    // SAFETY: the input is the mapping (alive, unwritten); `head` a local.
    unsafe { st.zs().get_output(head.as_mut_ptr(), &mut init) }?;
    let (hdr, head_len) = parse_header(&head[..init])?;
    if !type_is_valid(hdr.type_) {
        return Err(git_error_set!(
            ErrorClass::ODB,
            "failed to inflate disk object"
        ));
    }
    if init > head_len {
        st.start_len = init - head_len;
        st.start[..st.start_len].copy_from_slice(&head[head_len..init]);
    }
    Ok(hdr)
}

// ---------------------------------------------------------------------------
// The backend table
// ---------------------------------------------------------------------------

/// The backend's state behind its table: the configuration, shared with
/// its write streams.
pub struct LooseBackend(std::sync::Arc<Loose>);

/// Hand out `raw` through C's out-parameters.
///
/// # Safety
///
/// The pointers are writable.
unsafe fn write_raw(
    buffer_p: *mut *mut c_void,
    len_p: *mut usize,
    type_p: *mut git_object_t,
    raw: RawData,
) {
    let (buf, len, t) = raw;
    let (p, _) = buf.into_raw_parts();
    // SAFETY: the caller's contract.
    unsafe { buffer_p.write(p.cast()) };
    // SAFETY: as above.
    unsafe { len_p.write(len) };
    // SAFETY: as above.
    unsafe { type_p.write(t) };
}

fn code(r: Result<()>) -> c_int {
    crate::util::export::status(r)
}

ffibox::impl_cvtable! {
    // SAFETY: libgit2 passes each of a backend's slots the backend in the
    // place marked `@`, and calls `free` once; the slots left NULL
    // (`refresh`, `writepack`, `writemidx`) are optional in C's contract.
    unsafe impl CVtable<LooseBackend> for BackendTable {}
    base = backend_base();
    /// `loose_backend__read`.
    fn read(buffer_p: *mut *mut c_void, len_p: *mut usize, type_p: *mut git_object_t, @backend, oid: *const Oid) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseBackend| {
            git_assert_arg!(!oid.is_null(), "oid", -1);
            // SAFETY: C's contract: a readable id.
            match s.0.read(unsafe { &*oid }) {
                Ok(raw) => {
                    // SAFETY: C's contract: writable out-parameters.
                    unsafe { write_raw(buffer_p, len_p, type_p, raw) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `loose_backend__read_prefix`.
    fn read_prefix(
        out_oid: *mut Oid,
        buffer_p: *mut *mut c_void,
        len_p: *mut usize,
        type_p: *mut git_object_t,
        @backend,
        short_oid: *const Oid,
        len: usize,
    ) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseBackend| {
            if !(len >= GIT_OID_MINPREFIXLEN && len <= s.0.oid_hexsize) {
                return errors::assert_arg_failed("len >= 4 && len <= backend->oid_hexsize").raw();
            }
            // SAFETY: C's contract: a readable id (or NULL, refused).
            let Some(short_oid) = (unsafe { short_oid.as_ref() }) else {
                let text = if len == s.0.oid_hexsize { "oid" } else { "backend && short_oid" };
                return errors::assert_arg_failed(text).raw();
            };
            let mut id = None;
            let r = s.0.read_prefix(short_oid, len, &mut id);
            if let Some(id) = id {
                // SAFETY: C's contract: writable out-parameters.
                unsafe { out_oid.write(id) };
            }
            match r {
                Ok(raw) => {
                    // SAFETY: as above.
                    unsafe { write_raw(buffer_p, len_p, type_p, raw) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `loose_backend__read_header`.
    fn read_header(len_p: *mut usize, type_p: *mut git_object_t, @backend, oid: *const Oid) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseBackend| {
            git_assert_arg!(!oid.is_null(), "oid", -1);
            // SAFETY: C's contract: a readable id.
            match s.0.read_header(unsafe { &*oid }) {
                Ok(h) => {
                    // SAFETY: C's contract: writable out-parameters.
                    unsafe { len_p.write(h.size) };
                    // SAFETY: as above.
                    unsafe { type_p.write(h.type_) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `loose_backend__write`.
    fn write(@backend, oid: *const Oid, data: *const c_void, len: usize, type_: git_object_t) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseBackend| {
            if data.is_null() && len != 0 {
                // C read through NULL (rust/DEFECTS.md).
                return errors::assert_arg_failed("data").raw();
            }
            // SAFETY: C's contract: a readable id.
            let oid = unsafe { &*oid };
            // SAFETY: C's contract: `len` readable bytes.
            let data = unsafe { crate::util::export::bytes(data.cast(), len) };
            code(s.0.write(oid, data, type_))
        },
    }
    /// `loose_backend__writestream`.
    fn writestream(stream_out: *mut *mut GitOdbStream, @backend, length: git_object_size_t, type_: git_object_t) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseBackend| {
            // SAFETY: C's contract: writable.
            unsafe { stream_out.write(core::ptr::null_mut()) };
            match s.0.writestream(length, type_) {
                Ok(st) => {
                    // SAFETY: the table this slot runs on, live; its field,
                    // before C sees the stream; and C's out-parameter.
                    unsafe { (*st.as_ptr()).backend = backend };
                    // SAFETY: as above.
                    unsafe { stream_out.write(st.as_ptr()) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `loose_backend__readstream`.
    fn readstream(stream_out: *mut *mut GitOdbStream, len_out: *mut usize, type_out: *mut git_object_t, @backend, oid: *const Oid) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseBackend| {
            git_assert_arg!(!stream_out.is_null(), "stream_out", -1);
            git_assert_arg!(!len_out.is_null(), "len_out", -1);
            git_assert_arg!(!type_out.is_null(), "type_out", -1);
            git_assert_arg!(!oid.is_null(), "oid", -1);
            // SAFETY: C's contract: writable out-parameters.
            unsafe { stream_out.write(core::ptr::null_mut()) };
            // SAFETY: as above.
            unsafe { len_out.write(0) };
            // SAFETY: as above.
            unsafe { type_out.write(crate::odb::GIT_OBJECT_INVALID) };
            // SAFETY: C's contract: a readable id.
            match s.0.readstream(unsafe { &*oid }) {
                Ok((st, len, t)) => {
                    // SAFETY: as in `writestream`.
                    unsafe { (*st.as_ptr()).backend = backend };
                    // SAFETY: as above.
                    unsafe { stream_out.write(st.as_ptr()) };
                    // SAFETY: as above.
                    unsafe { len_out.write(len) };
                    // SAFETY: as above.
                    unsafe { type_out.write(t) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `loose_backend__exists`.
    fn exists(@backend, oid: *const Oid) -> c_int {
        on panic: 0,
        on busy: 0,
        |s: &LooseBackend| {
            git_assert_arg!(!oid.is_null(), "oid", -1);
            // SAFETY: C's contract: a readable id.
            c_int::from(s.0.exists(unsafe { &*oid }))
        },
    }
    /// `loose_backend__exists_prefix`.
    fn exists_prefix(out: *mut Oid, @backend, short_id: *const Oid, len: usize) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseBackend| {
            git_assert_arg!(!out.is_null(), "out", -1);
            git_assert_arg!(!short_id.is_null(), "short_id", -1);
            // SAFETY: C's contract: a readable id.
            match s.0.exists_prefix(unsafe { &*short_id }, len) {
                Ok(id) => {
                    // SAFETY: C's contract: writable.
                    unsafe { out.write(id) };
                    0
                }
                Err(e) => e.raw(),
            }
        },
    }
    /// `loose_backend__foreach`.
    fn foreach(@backend, cb: crate::odb::ForeachCb, data: *mut c_void) -> c_int {
        on panic: -1,
        on busy: busy(),
        |s: &LooseBackend| {
            let Some(cb) = cb else {
                return errors::assert_arg_failed("cb").raw();
            };
            // SAFETY: the caller's callback, with its payload.
            s.0.foreach(&mut |id| unsafe { cb(id, data) })
        },
    }
    /// `loose_backend__freshen`.
    fn freshen(@backend, oid: *const Oid) -> c_int {
        on panic: -1,
        on busy: busy(),
        // SAFETY: C's contract: a readable id.
        |s: &LooseBackend| code(s.0.freshen(unsafe { &*oid })),
    }
    release free;
}

/// `git_odb__backend_loose`: a new loose backend table (owned by the
/// caller, freed through its `free` slot).
pub fn new_backend(
    objects_dir: &[u8],
    opts: Option<&GitOdbBackendLooseOptions>,
) -> Result<NonNull<GitOdbBackend>> {
    let loose = Loose::new(objects_dir, opts)?;
    Ok(
        CTable::<BackendTable, LooseBackend>::new(LooseBackend(std::sync::Arc::new(loose)))
            .into_c(),
    )
}

/// The C ABI of `odb_loose.c`.
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, cstr_bytes};

    /// `git_odb_backend_loose_options_init`.
    ///
    /// # Safety
    ///
    /// `opts` is NULL or writable.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_odb_backend_loose_options_init(
        opts: *mut GitOdbBackendLooseOptions,
        version: c_uint,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!opts.is_null(), "opts", -1);
            if let Err(e) =
                crate::odb::check_version(Some(version), 1, "git_odb_backend_loose_options")
            {
                return e.raw();
            }
            // SAFETY: the caller's contract.
            unsafe { opts.write(GitOdbBackendLooseOptions::default()) };
            0
        })
    }

    /// `git_odb__backend_loose`.
    ///
    /// # Safety
    ///
    /// `backend_out` is writable; `objects_dir` is a NUL-terminated
    /// string; `opts` is NULL or a readable options struct.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_odb__backend_loose(
        backend_out: *mut *mut GitOdbBackend,
        objects_dir: *const c_char,
        opts: *mut GitOdbBackendLooseOptions,
    ) -> c_int {
        barrier(|| {
            git_assert_arg!(!backend_out.is_null(), "backend_out", -1);
            git_assert_arg!(!objects_dir.is_null(), "objects_dir", -1);
            // SAFETY: the caller's contract.
            let dir = unsafe { cstr_bytes(objects_dir) };
            // SAFETY: the caller's contract.
            let opts = unsafe { opts.as_ref() };
            match new_backend(dir, opts) {
                Ok(b) => {
                    // SAFETY: the caller's contract.
                    unsafe { backend_out.write(b.as_ptr()) };
                    0
                }
                Err(e) => e.raw(),
            }
        })
    }

    /// `git_odb_backend_loose`.
    ///
    /// # Safety
    ///
    /// As [`git_odb__backend_loose`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_odb_backend_loose(
        backend_out: *mut *mut GitOdbBackend,
        objects_dir: *const c_char,
        opts: *mut GitOdbBackendLooseOptions,
    ) -> c_int {
        // SAFETY: forwarded.
        unsafe { git_odb__backend_loose(backend_out, objects_dir, opts) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packlike_headers() {
        let _g = crate::util::test_support::init();
        assert_eq!(
            parse_header_packlike(&[0x35]).unwrap(),
            (ObjHdr { type_: 3, size: 5 }, 1)
        );
        assert_eq!(
            parse_header_packlike(&[0x95, 0x01]).unwrap(),
            (
                ObjHdr {
                    type_: 1,
                    size: 5 + 16
                },
                2
            )
        );
        assert!(parse_header_packlike(&[]).is_err());
        assert!(parse_header_packlike(&[0x95]).is_err());
        // Ten continuation bytes run past 64 bits of shift.
        let mut long = vec![0xffu8; 12];
        long.push(0);
        assert!(parse_header_packlike(&long).is_err());
    }

    #[test]
    fn standard_headers() {
        let _g = crate::util::test_support::init();
        assert_eq!(
            parse_header(b"blob 12\0rest").unwrap(),
            (ObjHdr { type_: 3, size: 12 }, 8)
        );
        // `git_object_stringn2type` matches a prefix.
        assert_eq!(parse_header(b"commitx 1\0").unwrap().0.type_, 1);
        assert_eq!(parse_header(b"bogus 1\0").unwrap().0.type_, -1);
        assert!(parse_header(b"blob 12").is_err());
        assert!(parse_header(b"blob12\0").is_err());
        assert!(parse_header(b"blob -1\0").is_err());
        assert!(parse_header(b"blob x\0").is_err());
        let e = parse_header(b"").unwrap_err();
        assert_eq!(e, ErrorCode::GENERIC);
        assert_eq!(
            crate::util::errors::last().1,
            b"failed to parse loose object: invalid header".to_vec()
        );
    }

    #[test]
    fn zlib_detection() {
        assert!(is_zlib_compressed_data(&[0x78, 0x01]));
        assert!(is_zlib_compressed_data(&[0x78, 0x9c]));
        assert!(!is_zlib_compressed_data(&[0x78]));
        assert!(!is_zlib_compressed_data(&[0x35, 0x00]));
    }

    #[test]
    fn file_names() {
        let _g = crate::util::test_support::init();
        let l = Loose::new(b"/tmp/objs", None).unwrap();
        assert_eq!(l.objects_dir(), b"/tmp/objs/");
        let id = Oid::from_hex(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391", OidType::Sha1).unwrap();
        let mut s = GitStr::new();
        l.object_file_name(&mut s, &id).unwrap();
        assert_eq!(
            s.as_bytes(),
            b"/tmp/objs/e6/9de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
        assert_eq!(
            l.filename_to_oid(b"e6/9de29bb2d1d6434b8b29ae775ad8c2e48c5391"),
            Some(id)
        );
        assert_eq!(
            l.filename_to_oid(b"e6/9de29bb2d1d6434b8b29ae775ad8c2e48c539x"),
            None
        );
        assert_eq!(
            l.filename_to_oid(b"e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"),
            None
        );
        assert_eq!(l.options().compression_level, Z_BEST_SPEED);
        assert_eq!(l.options().dir_mode, 0o777);
        assert_eq!(l.options().file_mode, 0o444);
    }
}
