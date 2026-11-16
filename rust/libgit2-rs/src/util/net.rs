//! Port of `src/util/net.c`: URL parsing and manipulation (`git_net_url`),
//! SCP-style paths, redirects, `no_proxy`-style patterns and certificate
//! host-name matching.
//!
//! [`GitNetUrl`] is `git_net_url` (kind 3, mirrored): the transports and
//! `remote.c`, which stay C, embed it by value, read and set its fields and
//! free them with `git__free`. Its strings are therefore owned C strings in
//! libgit2's allocator (`Option<GitCStr>`, a nullable `char *`). The parsers
//! are safe Rust over byte slices; the exports in [`ffi`] are shims over
//! them, and [`Url`] is the safe API.
//!
//! Inputs are read up to their first NUL byte, as the C reads its
//! NUL-terminated arguments.
//!
//! Where the C dereferences a NULL field (`git_net_url_is_ipv6` without a
//! host, a pattern match without a port, a redirect to a location without a
//! scheme…), the Rust gives a defined answer instead: see `rust/DEFECTS.md`.

use core::ffi::{c_char, c_int, c_uint};
use core::mem;

use crate::util::alloc::{self, GitCStr};
use crate::util::errors::{self, Bytes, Error, ErrorClass, ErrorCode, Result};
use crate::util::str::GitStr;
use crate::util::util::{memzero, strlcmp};
use crate::{git_assert, git_assert_arg, git_error_set};

const DEFAULT_PORT_HTTP: &[u8] = b"80";
const DEFAULT_PORT_HTTPS: &[u8] = b"443";
const DEFAULT_PORT_GIT: &[u8] = b"9418";
const DEFAULT_PORT_SSH: &[u8] = b"22";

/// `git_net_url`, mirrored. Every string is NULL or an owned allocation of
/// `git__allocator`; dropping the value disposes it as
/// `git_net_url_dispose` (zeroing the credentials first).
#[repr(C)]
#[derive(Default)]
pub struct GitNetUrl {
    /// `scheme`, lowercased by the parsers.
    pub scheme: Option<GitCStr>,
    /// `host` (percent-decoded; an IPv6 address without its brackets,
    /// except from the SCP parser, which keeps them).
    pub host: Option<GitCStr>,
    /// `port`, given or the scheme's default.
    pub port: Option<GitCStr>,
    /// `path`.
    pub path: Option<GitCStr>,
    /// `query` (percent-decoded), without the `?`.
    pub query: Option<GitCStr>,
    /// `fragment` (percent-decoded), without the `#`.
    pub fragment: Option<GitCStr>,
    /// `username` (percent-decoded).
    pub username: Option<GitCStr>,
    /// `password` (percent-decoded).
    pub password: Option<GitCStr>,
    /// `port_specified`: 1 when the port was given.
    pub port_specified: c_uint,
}

impl Drop for GitNetUrl {
    fn drop(&mut self) {
        self.dispose();
    }
}

impl core::fmt::Debug for GitNetUrl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = |v: &Option<GitCStr>| v.as_ref().map(|c| Bytes(c.as_bytes()).to_string());
        f.debug_struct("GitNetUrl")
            .field("scheme", &s(&self.scheme))
            .field("host", &s(&self.host))
            .field("port", &s(&self.port))
            .field("path", &s(&self.path))
            .field("query", &s(&self.query))
            .field("fragment", &s(&self.fragment))
            .field("username", &s(&self.username))
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("port_specified", &self.port_specified)
            .finish()
    }
}

/// The bytes of an optional field (`None` for NULL).
#[inline]
fn field(f: &Option<GitCStr>) -> Option<&[u8]> {
    f.as_ref().map(|s| s.as_bytes())
}

/// The bytes of `s` before its first NUL: what C sees of a string.
#[inline]
pub(crate) fn c_prefix(s: &[u8]) -> &[u8] {
    match memchr::memchr(0, s) {
        Some(n) => &s[..n],
        None => s,
    }
}

/// `git__strdup` of `s` (without NUL bytes) as an owned string;
/// `GIT_ERROR_CHECK_ALLOC` on failure.
fn dup(s: &[u8]) -> Result<GitCStr> {
    // SAFETY: `substrdup` returns NULL or a fresh NUL-terminated allocation
    // of `git__allocator`, now owned here.
    unsafe { GitCStr::from_raw(alloc::substrdup(s)) }.ok_or_else(errors::oom)
}

/// Zero the bytes of an owned string in place (`git__memzero`).
fn zero_cstr(s: &mut GitCStr) {
    let len = s.len();
    let p = s.as_ptr().cast_mut().cast::<u8>();
    // SAFETY: `s` exclusively owns the `len + 1` bytes at `p` (we hold
    // `&mut` to it, and no view of them is alive), so they may be written;
    // the NUL at `len` is kept, so the string stays NUL-terminated.
    memzero(unsafe { core::slice::from_raw_parts_mut(p, len) });
}

/// Shorten an owned string in place to `n` bytes (`s[n] = '\0'`); nothing
/// when `n` is not within it.
fn truncate_cstr(s: &mut GitCStr, n: usize) {
    if n < s.len() {
        let p = s.as_ptr().cast_mut();
        // SAFETY: `n < strlen`, so `p + n` is within the allocation that `s`
        // exclusively owns (we hold `&mut` to it); a NUL earlier keeps it a
        // NUL-terminated string.
        unsafe { p.wrapping_add(n).write(0) };
    }
}

/// ASCII `tolower`, as `git__tolower` on the C locale.
#[inline]
fn tolower(c: u8) -> u8 {
    c.to_ascii_lowercase()
}

#[inline]
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `git_net_hostname_matches_cert`: whether `hostname` matches the
/// certificate name `pattern` (RFC 2818: `*` matches within one label).
pub fn hostname_matches_cert(hostname: &[u8], pattern: &[u8]) -> bool {
    let (hostname, pattern) = (c_prefix(hostname), c_prefix(pattern));
    let (mut h, mut p) = (0usize, 0usize);
    loop {
        let c = tolower(at(pattern, p));
        p += 1;

        if c == 0 {
            return h >= hostname.len();
        }

        if c == b'*' {
            let c = at(pattern, p);

            // '*' at the end matches everything left
            if c == 0 {
                return true;
            }

            // Move towards the next matching char; wildcards do not
            // cross subdomains. (The C recurses, in tail position.)
            loop {
                if h >= hostname.len() {
                    return false;
                }
                let hc = tolower(hostname[h]);
                if hc == c || hc == b'.' {
                    break;
                }
                h += 1;
            }
            continue;
        }

        if c != tolower(at(hostname, h)) {
            return false;
        }
        h += 1;
    }
}

#[inline]
fn is_valid_scheme_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.'
}

/// `git_net_str_is_url`: whether `s` starts with `<scheme>://`.
pub fn str_is_url(s: &[u8]) -> bool {
    let s = c_prefix(s);
    for (i, &c) in s.iter().enumerate() {
        if c == b':' && at(s, i + 1) == b'/' && at(s, i + 2) == b'/' {
            return true;
        }
        if !is_valid_scheme_char(c) {
            break;
        }
    }
    false
}

fn default_port_for_scheme(scheme: &[u8]) -> Option<&'static [u8]> {
    match scheme {
        b"http" => Some(DEFAULT_PORT_HTTP),
        b"https" => Some(DEFAULT_PORT_HTTPS),
        b"git" => Some(DEFAULT_PORT_GIT),
        b"ssh" | b"ssh+git" | b"git+ssh" => Some(DEFAULT_PORT_SSH),
        _ => None,
    }
}

/// `is_ssh_scheme`: `strncasecmp(scheme, "ssh", scheme_len) == 0 || …`, so
/// (as in C) any case-insensitive prefix of `ssh`, `ssh+git` or `git+ssh`
/// counts, e.g. `g` or `git`.
fn is_ssh_scheme(scheme: &[u8]) -> bool {
    if scheme.is_empty() {
        return false;
    }
    let prefix_of =
        |lit: &[u8]| scheme.len() <= lit.len() && scheme.eq_ignore_ascii_case(&lit[..scheme.len()]);
    prefix_of(b"ssh") || prefix_of(b"ssh+git") || prefix_of(b"git+ssh")
}

/// `git_net_url_parser`: the pieces of the URL, as slices of the input.
#[derive(Default)]
struct Parser<'a> {
    hierarchical: bool,
    scheme: &'a [u8],
    user: &'a [u8],
    password: &'a [u8],
    host: &'a [u8],
    port: &'a [u8],
    path: &'a [u8],
    query: &'a [u8],
    fragment: &'a [u8],
}

fn url_invalid(message: &str) -> ErrorCode {
    let _ = git_error_set!(ErrorClass::NET, "invalid url: {}", message);
    ErrorCode::INVALID_SPEC
}

#[inline]
fn is_hex_or_colon(c: u8) -> bool {
    c.is_ascii_hexdigit() || c == b':'
}

/// `url_parse_authority`: `[user[:password]@](host|[ipv6])[:port]`, walked
/// backwards so that `@` may appear in an SSH user name.
fn parse_authority<'a>(parser: &mut Parser<'a>, authority: &'a [u8]) -> Result<()> {
    #[derive(PartialEq, Eq, Clone, Copy)]
    enum State {
        HostPort,
        Host,
        Ipv6,
        HostEnd,
        UserPass,
        User,
    }

    let len = authority.len();
    if len == 0 {
        return Ok(());
    }

    let hostport_end = len;
    let mut host_end: Option<usize> = None;
    let mut userpass_end = 0usize;
    let mut user_end: Option<usize> = None;
    let mut state = State::HostPort;

    let mut i = len;
    while i > 0 && user_end.is_none() {
        i -= 1;
        let c = authority[i];
        let mut host_case = false;

        match state {
            State::HostPort => {
                if c == b':' {
                    parser.port = &authority[i + 1..hostport_end];
                    host_end = Some(i);
                    state = State::Host;
                } else {
                    // Only digits so far: a host or a port. A non-digit
                    // means a host; either way, look for the '@'.
                    if !c.is_ascii_digit() {
                        host_end = Some(hostport_end);
                        state = State::Host;
                    }
                    host_case = true;
                }
            }
            State::Host => host_case = true,
            State::Ipv6 => {
                if c == b'[' {
                    // `host_end` is set on entering Ipv6.
                    let end = host_end.unwrap_or(hostport_end);
                    parser.host = &authority[i + 1..end];
                    state = State::HostEnd;
                } else if !is_hex_or_colon(c) {
                    return Err(url_invalid("malformed hostname"));
                }
            }
            State::HostEnd => {
                if c == b'@' {
                    userpass_end = i;
                    state = State::UserPass;
                } else {
                    return Err(url_invalid("malformed hostname"));
                }
            }
            State::UserPass => {
                if c == b'@' && !is_ssh_scheme(parser.scheme) {
                    return Err(url_invalid("malformed hostname"));
                }
                if c == b':' {
                    parser.password = &authority[i + 1..userpass_end];
                    user_end = Some(i);
                    state = State::User;
                }
            }
            // The loop stops once `user_end` is set.
            State::User => {}
        }

        if host_case {
            if c == b']' && host_end == Some(i + 1) {
                host_end = Some(i);
                state = State::Ipv6;
            } else if c == b'@' {
                let end = host_end.unwrap_or(hostport_end);
                parser.host = &authority[i + 1..end];
                userpass_end = i;
                state = State::UserPass;
            } else if c == b'[' || c == b']' || c == b':' {
                return Err(url_invalid("malformed hostname"));
            }
        }
    }

    match state {
        State::HostPort => parser.host = &authority[..hostport_end],
        State::Host => parser.host = &authority[..host_end.unwrap_or(hostport_end)],
        State::Ipv6 => return Err(url_invalid("malformed hostname")),
        State::HostEnd => {}
        State::UserPass => parser.user = &authority[..userpass_end],
        State::User => parser.user = &authority[..user_end.unwrap_or(0)],
    }
    Ok(())
}

/// `url_parse_path`: `path[?query][#fragment]`.
fn parse_path<'a>(parser: &mut Parser<'a>, path: &'a [u8]) {
    parser.path = path;
    match memchr::memchr2(b'?', b'#', path) {
        None => {}
        Some(q) if path[q] == b'?' => {
            parser.path = &path[..q];
            let rest = &path[q + 1..];
            match memchr::memchr(b'#', rest) {
                Some(h) => {
                    parser.query = &rest[..h];
                    parser.fragment = &rest[h + 1..];
                }
                None => parser.query = rest,
            }
        }
        Some(h) => {
            parser.path = &path[..h];
            parser.fragment = &path[h + 1..];
        }
    }
}

/// `git_str_detach` of a string: `None` unless it was written.
#[inline]
fn detach(s: &mut GitStr) -> Option<GitCStr> {
    s.detach()
}

fn decoded(s: &[u8]) -> Result<Option<GitCStr>> {
    if s.is_empty() {
        return Ok(None);
    }
    let mut g = GitStr::new();
    g.decode_percent(s)?;
    Ok(detach(&mut g))
}

fn copied(s: &[u8]) -> Result<Option<GitCStr>> {
    let mut g = GitStr::new();
    g.put(s)?;
    Ok(detach(&mut g))
}

/// `url_parse_finalize`: decode the pieces into `url`, which is only
/// written when every piece succeeded.
fn parse_finalize(url: &mut GitNetUrl, parser: &Parser<'_>) -> Result<()> {
    let mut scheme = None;
    if !parser.scheme.is_empty() {
        let mut s = copied(parser.scheme)?;
        if let Some(s) = s.as_mut() {
            lowercase_cstr(s);
        }
        scheme = s;
    }

    let user = decoded(parser.user)?;
    let password = decoded(parser.password)?;
    let host = decoded(parser.host)?;

    let mut port_specified = 0;
    let port = if !parser.port.is_empty() {
        port_specified = 1;
        copied(parser.port)?
    } else if let Some(d) = scheme
        .as_ref()
        .and_then(|s| default_port_for_scheme(s.as_bytes()))
    {
        copied(d)?
    } else {
        None
    };

    let path = if !parser.path.is_empty() {
        copied(parser.path)?
    } else if parser.hierarchical {
        copied(b"/")?
    } else {
        None
    };

    let query = decoded(parser.query)?;
    let fragment = decoded(parser.fragment)?;

    url.scheme = scheme;
    url.host = host;
    url.port = port;
    url.path = path;
    url.query = query;
    url.fragment = fragment;
    url.username = user;
    url.password = password;
    url.port_specified = port_specified;
    Ok(())
}

fn lowercase_cstr(s: &mut GitCStr) {
    let len = s.len();
    let p = s.as_ptr().cast_mut().cast::<u8>();
    // SAFETY: as in `zero_cstr`: `s` exclusively owns these `len` bytes,
    // and lowercasing never writes a NUL.
    unsafe { core::slice::from_raw_parts_mut(p, len) }.make_ascii_lowercase();
}

/// `git_net_url_parse`: parse `given` into `url`, which must be empty (the
/// export clears it first, as C's `memset`). On failure `url` stays empty.
pub fn parse(url: &mut GitNetUrl, given: &[u8]) -> Result<()> {
    #[derive(PartialEq, Eq, Clone, Copy)]
    enum State {
        SchemeStart,
        Scheme,
        AuthorityStart,
        Authority,
        PathStart,
        Path,
    }

    let given = c_prefix(given);
    let mut parser = Parser::default();
    let mut authority_start = 0usize;
    let mut authority_len = 0usize;
    let mut path_start = 0usize;
    let mut state = State::SchemeStart;
    let mut scheme_start = 0usize;

    let mut i = 0usize;
    while i < given.len() {
        let c = given[i];
        // `continue 'again` re-dispatches the same character (C's
        // fall-through).
        let mut st = state;
        loop {
            match st {
                State::SchemeStart => {
                    scheme_start = i;
                    state = State::Scheme;
                    st = State::Scheme;
                    continue;
                }
                State::Scheme => {
                    if c == b':' {
                        parser.scheme = &given[scheme_start..i];
                        if !parser.scheme.is_empty()
                            && at(given, i + 1) == b'/'
                            && at(given, i + 2) == b'/'
                        {
                            i += 2;
                            parser.hierarchical = true;
                            state = State::AuthorityStart;
                        } else {
                            state = State::PathStart;
                        }
                    } else if !is_valid_scheme_char(c) {
                        // an illegal scheme character: a relative path
                        path_start = 0;
                        state = State::Path;
                    }
                }
                State::AuthorityStart => {
                    authority_start = i;
                    state = State::Authority;
                    st = State::Authority;
                    continue;
                }
                State::Authority => {
                    if c == b'/' {
                        authority_len = i - authority_start;
                        st = State::PathStart;
                        continue;
                    }
                }
                State::PathStart => {
                    path_start = i;
                    state = State::Path;
                }
                State::Path => {}
            }
            break;
        }
        i += 1;
    }
    let end = given.len();

    let mut path_len = 0usize;
    match state {
        State::SchemeStart => {
            // The empty string: C's `GIT_ASSERT(!"unhandled state")`.
            git_assert!(false, "!\"unhandled state\"");
        }
        State::Scheme => {
            // never saw a ':': a relative path, not a bare scheme
            path_start = 0;
            path_len = end;
        }
        State::AuthorityStart | State::PathStart => {}
        State::Authority => authority_len = end - authority_start,
        State::Path => path_len = end - path_start,
    }

    if authority_len > 0 {
        parse_authority(
            &mut parser,
            &given[authority_start..authority_start + authority_len],
        )?;
    }
    if path_len > 0 {
        parse_path(&mut parser, &given[path_start..path_start + path_len]);
    }
    parse_finalize(url, &parser)
}

/// `git_net_url_parse_http`: a URL, or an HTTP endpoint without a scheme
/// (`localhost`, `localhost:8080/path`, `user@host`).
pub fn parse_http(url: &mut GitNetUrl, given: &[u8]) -> Result<()> {
    let given = c_prefix(given);
    if str_is_url(given) {
        return parse(url, given);
    }

    let (authority, path) = match memchr::memchr(b'/', given) {
        Some(p) => (&given[..p], &given[p..]),
        None => (given, &given[given.len()..]),
    };

    let mut parser = Parser {
        scheme: b"http",
        hierarchical: true,
        ..Parser::default()
    };
    if !authority.is_empty() {
        parse_authority(&mut parser, authority)?;
    }
    if !path.is_empty() {
        parse_path(&mut parser, path);
    }
    parse_finalize(url, &parser)
}

fn scp_invalid(message: &str) -> ErrorCode {
    let _ = git_error_set!(ErrorClass::NET, "invalid scp-style path: {}", message);
    ErrorCode::INVALID_SPEC
}

/// `is_ipv6`: `[` then hex digits and colons up to `]`, with at least two
/// colons.
fn is_ipv6(s: &[u8]) -> bool {
    if s.first() != Some(&b'[') {
        return false;
    }
    let mut colons = 0usize;
    for &c in &s[1..] {
        if c == b':' {
            colons += 1;
        }
        if c == b']' {
            return colons > 1;
        }
        if !is_hex_or_colon(c) {
            return false;
        }
    }
    false
}

/// `has_at`: an `@` before the first `:`.
fn has_at(s: &[u8]) -> bool {
    for &c in s {
        if c == b'@' {
            return true;
        }
        if c == b':' {
            break;
        }
    }
    false
}

/// `git_net_url_parse_scp`: `[user@]host:path` (and `[host:port]:path`,
/// `[ipv6]:path`) as an `ssh` URL. `url` must be empty; on failure it keeps
/// what was set before the failure, as C.
pub fn parse_scp(url: &mut GitNetUrl, given: &[u8]) -> Result<()> {
    #[derive(PartialEq, Eq, Clone, Copy)]
    enum State {
        None,
        User,
        HostStart,
        Host,
        HostEnd,
        Ipv6,
        Ipv6End,
        PortStart,
        Port,
        PortEnd,
        PathStart,
    }

    let given = c_prefix(given);
    let (mut user, mut host, mut port) = (0usize, 0usize, 0usize);
    let (mut user_len, mut host_len, mut port_len) = (0usize, 0usize, 0usize);
    let mut path: Option<usize> = None;
    let mut bracket: u16 = 0;
    let mut state = State::None;

    let mut i = 0usize;
    while i < given.len() && path.is_none() {
        let c = given[i];
        match state {
            State::None => match c {
                b'@' => return Err(scp_invalid("unexpected '@'")),
                b':' => return Err(scp_invalid("unexpected ':'")),
                b'[' => {
                    if is_ipv6(&given[i..]) {
                        state = State::Ipv6;
                        host = i;
                    } else {
                        let b = bracket;
                        bracket = bracket.wrapping_add(1);
                        if b > 1 {
                            return Err(scp_invalid("unexpected '['"));
                        }
                    }
                }
                _ => {
                    if has_at(&given[i..]) {
                        state = State::User;
                        user = i;
                    } else {
                        state = State::Host;
                        host = i;
                    }
                }
            },
            State::User => {
                if c == b'@' {
                    user_len = i - user;
                    state = State::HostStart;
                }
            }
            State::HostStart => {
                state = if c == b'[' { State::Ipv6 } else { State::Host };
                host = i;
            }
            State::Host => {
                if c == b':' {
                    host_len = i - host;
                    state = if bracket != 0 {
                        State::PortStart
                    } else {
                        State::PathStart
                    };
                } else if c == b']' {
                    let b = bracket;
                    bracket = bracket.wrapping_sub(1);
                    if b == 0 {
                        return Err(scp_invalid("unexpected ']'"));
                    }
                    host_len = i - host;
                    state = State::HostEnd;
                }
            }
            State::HostEnd => {
                if c != b':' {
                    return Err(scp_invalid("unexpected character after hostname"));
                }
                state = State::PathStart;
            }
            State::Ipv6 => {
                if c == b']' {
                    state = State::Ipv6End;
                }
            }
            State::Ipv6End => {
                if c != b':' {
                    return Err(scp_invalid("unexpected character after ipv6 address"));
                }
                host_len = i - host;
                state = if bracket != 0 {
                    State::PortStart
                } else {
                    State::PathStart
                };
            }
            State::PortStart => {
                port = i;
                state = State::Port;
            }
            State::Port => {
                if c == b']' {
                    let b = bracket;
                    bracket = bracket.wrapping_sub(1);
                    if b == 0 {
                        return Err(scp_invalid("unexpected ']'"));
                    }
                    port_len = i - port;
                    state = State::PortEnd;
                }
            }
            State::PortEnd => {
                if c != b':' {
                    return Err(scp_invalid("unexpected character after ipv6 address"));
                }
                state = State::PathStart;
            }
            State::PathStart => path = Some(i),
        }
        i += 1;
    }

    let Some(path) = path else {
        return Err(scp_invalid("path is required"));
    };

    url.scheme = Some(dup(b"ssh")?);
    if user_len > 0 {
        url.username = Some(dup(&given[user..user + user_len])?);
    }
    git_assert!(host_len > 0, "host_len");
    url.host = Some(dup(&given[host..host + host_len])?);
    if port_len > 0 {
        url.port_specified = 1;
        url.port = Some(dup(&given[port..port + port_len])?);
    } else {
        url.port = Some(dup(DEFAULT_PORT_SSH)?);
    }
    url.path = Some(dup(&given[path..])?);
    Ok(())
}

/// `git_net_url_parse_standard_or_scp`.
pub fn parse_standard_or_scp(url: &mut GitNetUrl, given: &[u8]) -> Result<()> {
    if str_is_url(given) {
        parse(url, given)
    } else {
        parse_scp(url, given)
    }
}

/// A NULL `%s` as glibc's printf shows it.
fn or_null(f: Option<&[u8]>) -> &[u8] {
    f.unwrap_or(b"(null)")
}

/// Replace `slot` without freeing what it held, as C's assignment does.
fn overwrite(slot: &mut Option<GitCStr>, value: GitCStr) {
    mem::forget(slot.replace(value));
}

impl GitNetUrl {
    /// An empty URL (`GIT_NET_URL_INIT`).
    pub const fn new() -> GitNetUrl {
        GitNetUrl {
            scheme: None,
            host: None,
            port: None,
            path: None,
            query: None,
            fragment: None,
            username: None,
            password: None,
            port_specified: 0,
        }
    }

    /// `git_net_url_dispose`: zero the credentials, free every string and
    /// leave the URL empty (`port_specified` is kept, as C).
    pub fn dispose(&mut self) {
        if let Some(u) = self.username.as_mut() {
            zero_cstr(u);
        }
        if let Some(p) = self.password.as_mut() {
            zero_cstr(p);
        }
        self.scheme = None;
        self.host = None;
        self.port = None;
        self.path = None;
        self.query = None;
        self.fragment = None;
        self.username = None;
        self.password = None;
    }

    /// `git_net_url_dup`: copy `src`'s strings (but not its fragment or
    /// `port_specified`, as C) into `self`, overwriting without freeing the
    /// fields `src` has. On failure, the fields copied so far stay.
    pub fn dup_from(&mut self, src: &GitNetUrl) -> Result<()> {
        for (to, from) in [
            (&mut self.scheme, &src.scheme),
            (&mut self.host, &src.host),
            (&mut self.port, &src.port),
            (&mut self.path, &src.path),
            (&mut self.query, &src.query),
            (&mut self.username, &src.username),
            (&mut self.password, &src.password),
        ] {
            if let Some(s) = from {
                overwrite(to, dup(s.as_bytes())?);
            }
        }
        Ok(())
    }

    /// `git_net_url_joinpath`: `self` becomes `one` with `two` appended to
    /// its path (one `/` between them), and `two`'s query (if any) as the
    /// query; `one`'s query and fragment are dropped.
    pub fn joinpath(&mut self, one: &GitNetUrl, two: &[u8]) -> Result<()> {
        self.dispose();
        let mut two = c_prefix(two);

        let mut two_len = match memchr::memchr(b'?', two) {
            Some(q) => {
                let query = &two[q + 1..];
                if !query.is_empty() {
                    self.query = Some(dup(query)?);
                }
                q
            }
            None => two.len(),
        };

        // Strip all trailing `/`s from the first path
        let one_path = field(&one.path).unwrap_or(b"");
        let mut one_len = one_path.len();
        while one_len > 0 && one_path[one_len - 1] == b'/' {
            one_len -= 1;
        }

        // Strip all leading `/`s from the second path
        while two.first() == Some(&b'/') {
            two = &two[1..];
            two_len -= 1;
        }

        let mut path = GitStr::new();
        let _ = path.put(&one_path[..one_len]);
        let _ = path.putc(b'/');
        let _ = path.put(&two[..two_len]);
        if path.is_oom() {
            return Err(ErrorCode::GENERIC);
        }
        self.path = path.detach();

        for (to, from) in [
            (&mut self.scheme, &one.scheme),
            (&mut self.host, &one.host),
            (&mut self.port, &one.port),
            (&mut self.username, &one.username),
            (&mut self.password, &one.password),
        ] {
            if let Some(s) = from {
                *to = Some(dup(s.as_bytes())?);
            }
        }
        Ok(())
    }

    /// `remove_service_suffix`: drop `service_suffix` (e.g.
    /// `/info/refs?service=git-upload-pack`) from the end of the path,
    /// whether or not the server kept its query, leaving at least `/`.
    fn remove_service_suffix(&mut self, service_suffix: &[u8]) {
        let service_query = memchr::memchr(b'?', service_suffix);
        let full_suffix_len = service_suffix.len();
        let suffix_len = service_query.unwrap_or(full_suffix_len);
        // C: `strlen(url->path)` (NULL: see DEFECTS.md).
        let Some(path) = field(&self.path) else {
            return;
        };
        let path_len = path.len();
        let mut truncate: isize = -1;

        // A redirect without query parameters, like "/newloc/info/refs"
        if suffix_len > 0 && path_len >= suffix_len {
            let suffix_offset = path_len - suffix_len;
            if path[suffix_offset..] == service_suffix[..suffix_len]
                && service_query
                    .is_none_or(|q| field(&self.query) == Some(&service_suffix[q + 1..]))
            {
                truncate = suffix_offset as isize;
            }
        }

        // A redirect with query parameters, like
        // "/newloc/info/refs?service=git-upload-pack"
        if truncate < 0 && path.ends_with(service_suffix) {
            truncate = (path_len - full_suffix_len) as isize;
        }

        // Ensure we leave a minimum of '/' as the path
        if truncate == 0 {
            truncate += 1;
        }

        if truncate > 0 {
            if let Some(p) = self.path.as_mut() {
                truncate_cstr(p, truncate as usize);
            }
            self.query = None;
        }
    }

    /// `git_net_url_apply_redirect`: follow `location` (an absolute path,
    /// or a URL on the same scheme or `https`, and on the same host unless
    /// `allow_offsite`), then remove `service_suffix`.
    pub fn apply_redirect(
        &mut self,
        location: &[u8],
        allow_offsite: bool,
        service_suffix: Option<&[u8]>,
    ) -> Result<()> {
        let location = c_prefix(location);
        if location.first() == Some(&b'/') {
            self.path = None;
            self.path = Some(dup(location)?);
        } else {
            let mut tmp = GitNetUrl::new();
            parse(&mut tmp, location)?;

            // Validate that this is a legal redirection
            if let Some(orig) = field(&self.scheme) {
                let to = field(&tmp.scheme);
                if to != Some(orig) && to != Some(b"https") {
                    let _ = git_error_set!(
                        ErrorClass::NET,
                        "cannot redirect from '{}' to '{}'",
                        Bytes(orig),
                        Bytes(or_null(to))
                    );
                    return Err(ErrorCode::GENERIC);
                }
            }
            if let Some(orig) = field(&self.host) {
                let to = field(&tmp.host);
                if !allow_offsite && to.is_none_or(|t| crate::util::util::strcasecmp(orig, t) != 0)
                {
                    let _ = git_error_set!(
                        ErrorClass::NET,
                        "cannot redirect from '{}' to '{}'",
                        Bytes(orig),
                        Bytes(or_null(to))
                    );
                    return Err(ErrorCode::GENERIC);
                }
            }
            mem::swap(self, &mut tmp);
        }

        if let Some(suffix) = service_suffix {
            self.remove_service_suffix(c_prefix(suffix));
        }
        Ok(())
    }

    /// `git_net_url_valid`: it has a host, a port and a path.
    pub fn is_valid(&self) -> bool {
        self.host.is_some() && self.port.is_some() && self.path.is_some()
    }

    /// `git_net_url_is_default_port`.
    pub fn is_default_port(&self) -> bool {
        match field(&self.scheme).and_then(default_port_for_scheme) {
            Some(d) => field(&self.port) == Some(d),
            None => false,
        }
    }

    /// `git_net_url_is_ipv6`: the host contains a `:`.
    pub fn is_ipv6(&self) -> bool {
        field(&self.host).is_some_and(|h| h.contains(&b':'))
    }

    /// `git_net_url_fmt`: append the URL to `buf` (credentials included,
    /// the port only when not the default, no fragment).
    pub fn fmt(&self, buf: &mut GitStr) -> Result<()> {
        git_assert_arg!(self.scheme.is_some(), "url->scheme");
        git_assert_arg!(self.host.is_some(), "url->host");

        let _ = buf.puts(field(&self.scheme).unwrap_or(b""));
        let _ = buf.puts(b"://");
        if let Some(u) = field(&self.username) {
            let _ = buf.puts(u);
            if let Some(p) = field(&self.password) {
                let _ = buf.puts(b":");
                let _ = buf.puts(p);
            }
            let _ = buf.putc(b'@');
        }
        let _ = buf.puts(field(&self.host).unwrap_or(b""));
        if let Some(port) = field(&self.port)
            && !self.is_default_port()
        {
            let _ = buf.putc(b':');
            let _ = buf.puts(port);
        }
        self.fmt_path_query(buf)
    }

    /// `git_net_url_fmt_path`: append the path (or `/`) and the query.
    pub fn fmt_path(&self, buf: &mut GitStr) -> Result<()> {
        self.fmt_path_query(buf)
    }

    fn fmt_path_query(&self, buf: &mut GitStr) -> Result<()> {
        let _ = buf.puts(field(&self.path).unwrap_or(b"/"));
        if let Some(q) = field(&self.query) {
            let _ = buf.putc(b'?');
            let _ = buf.puts(q);
        }
        if buf.is_oom() {
            Err(ErrorCode::GENERIC)
        } else {
            Ok(())
        }
    }

    /// `git_net_url_matches_pattern`: `*`, `host`, `.domain`, `*.domain`,
    /// each optionally `:port` (a `no_proxy` entry).
    pub fn matches_pattern(&self, pattern: &[u8]) -> bool {
        self.matches_one(c_prefix(pattern))
    }

    fn matches_one(&self, pattern: &[u8]) -> bool {
        let pattern_len = pattern.len();
        let wildcard = if pattern_len == 0 {
            return false;
        } else if pattern == b"*" {
            return true;
        } else if pattern_len > 1 && pattern[0] == b'*' && pattern[1] == b'.' {
            2
        } else if pattern[0] == b'.' {
            1
        } else {
            0
        };

        let mut domain = &pattern[wildcard..];
        let mut port: &[u8] = b"";
        if let Some(colon) = memchr::memchr(b':', domain) {
            port = &domain[colon + 1..];
            domain = &domain[..colon];
        }

        // C reads NULL host and port fields; they are "" here (DEFECTS.md).
        let host = field(&self.host).unwrap_or(b"");

        // A pattern's port *must* match if it's specified
        if !port.is_empty() && strlcmp(field(&self.port).unwrap_or(b""), port) != 0 {
            return false;
        }

        // No wildcard? Host must match exactly.
        if wildcard == 0 {
            return strlcmp(host, domain) == 0;
        }

        // Wildcard: ensure there's (at least) a suffix match
        let (host_len, domain_len) = (host.len(), domain.len());
        if host_len < domain_len || &host[host_len - domain_len..] != domain {
            return false;
        }

        // The pattern is *.domain and the host is simply domain
        if host_len == domain_len {
            return true;
        }

        // The pattern is *.domain and the host is foo.domain
        host[host_len - domain_len - 1] == b'.'
    }

    /// `git_net_url_matches_pattern_list`: any of the comma-separated
    /// patterns matches.
    pub fn matches_pattern_list(&self, pattern_list: &[u8]) -> bool {
        let list = c_prefix(pattern_list);
        if list.is_empty() {
            return false;
        }
        list.split(|&c| c == b',').any(|p| self.matches_one(p))
    }
}

// ---------------------------------------------------------------------------
// The safe API
// ---------------------------------------------------------------------------

/// A parsed URL (`git_net_url`).
///
/// Strings are bytes; inputs are read up to their first NUL byte.
#[derive(Debug, Default)]
pub struct Url(GitNetUrl);

/// What a URL parser fills.
fn parsed(
    f: fn(&mut GitNetUrl, &[u8]) -> Result<()>,
    s: &[u8],
) -> core::result::Result<Url, Error> {
    let mut u = GitNetUrl::new();
    f(&mut u, s).map_err(Error::from)?;
    Ok(Url(u))
}

impl Url {
    /// `git_net_url_parse`: a URL (`scheme://[user[:pass]@]host[:port]/path?query#frag`,
    /// `scheme:path`) or a relative path.
    pub fn parse(s: &[u8]) -> core::result::Result<Url, Error> {
        parsed(parse, s)
    }

    /// `git_net_url_parse_scp`: an SCP-style path (`[user@]host:path`).
    pub fn parse_scp(s: &[u8]) -> core::result::Result<Url, Error> {
        parsed(parse_scp, s)
    }

    /// `git_net_url_parse_standard_or_scp`.
    pub fn parse_standard_or_scp(s: &[u8]) -> core::result::Result<Url, Error> {
        parsed(parse_standard_or_scp, s)
    }

    /// `git_net_url_parse_http`: a URL, or an HTTP endpoint without a
    /// scheme (`host[:port][/path]`).
    pub fn parse_http(s: &[u8]) -> core::result::Result<Url, Error> {
        parsed(parse_http, s)
    }

    /// The scheme (lowercase).
    pub fn scheme(&self) -> Option<&[u8]> {
        field(&self.0.scheme)
    }
    /// The host.
    pub fn host(&self) -> Option<&[u8]> {
        field(&self.0.host)
    }
    /// The port, given or the scheme's default.
    pub fn port(&self) -> Option<&[u8]> {
        field(&self.0.port)
    }
    /// The path.
    pub fn path(&self) -> Option<&[u8]> {
        field(&self.0.path)
    }
    /// The query, without `?`.
    pub fn query(&self) -> Option<&[u8]> {
        field(&self.0.query)
    }
    /// The fragment, without `#`.
    pub fn fragment(&self) -> Option<&[u8]> {
        field(&self.0.fragment)
    }
    /// The user name.
    pub fn username(&self) -> Option<&[u8]> {
        field(&self.0.username)
    }
    /// The password.
    pub fn password(&self) -> Option<&[u8]> {
        field(&self.0.password)
    }
    /// Whether the port was given.
    pub fn port_specified(&self) -> bool {
        self.0.port_specified != 0
    }

    /// Set (or clear) the user name and password.
    pub fn set_credentials(
        &mut self,
        username: Option<&[u8]>,
        password: Option<&[u8]>,
    ) -> core::result::Result<(), Error> {
        let u = username
            .map(|u| dup(c_prefix(u)))
            .transpose()
            .map_err(Error::from)?;
        let p = password
            .map(|p| dup(c_prefix(p)))
            .transpose()
            .map_err(Error::from)?;
        if let Some(old) = self.0.username.as_mut() {
            zero_cstr(old);
        }
        if let Some(old) = self.0.password.as_mut() {
            zero_cstr(old);
        }
        self.0.username = u;
        self.0.password = p;
        Ok(())
    }

    /// A copy (`git_net_url_dup`: without the fragment and
    /// `port_specified`, as C).
    pub fn try_clone(&self) -> core::result::Result<Url, Error> {
        let mut u = GitNetUrl::new();
        u.dup_from(&self.0).map_err(Error::from)?;
        Ok(Url(u))
    }

    /// `git_net_url_joinpath`: this URL with `path` (which may carry a
    /// `?query`) appended to its path.
    pub fn join_path(&self, path: &[u8]) -> core::result::Result<Url, Error> {
        let mut u = GitNetUrl::new();
        u.joinpath(&self.0, path).map_err(Error::from)?;
        Ok(Url(u))
    }

    /// `git_net_url_apply_redirect`.
    pub fn apply_redirect(
        &mut self,
        location: &[u8],
        allow_offsite: bool,
        service_suffix: Option<&[u8]>,
    ) -> core::result::Result<(), Error> {
        self.0
            .apply_redirect(location, allow_offsite, service_suffix)
            .map_err(Error::from)
    }

    /// `git_net_url_valid`.
    pub fn is_valid(&self) -> bool {
        self.0.is_valid()
    }
    /// `git_net_url_is_default_port`.
    pub fn is_default_port(&self) -> bool {
        self.0.is_default_port()
    }
    /// `git_net_url_is_ipv6`.
    pub fn is_ipv6(&self) -> bool {
        self.0.is_ipv6()
    }
    /// `git_net_url_matches_pattern`.
    pub fn matches_pattern(&self, pattern: &[u8]) -> bool {
        self.0.matches_pattern(pattern)
    }
    /// `git_net_url_matches_pattern_list`.
    pub fn matches_pattern_list(&self, patterns: &[u8]) -> bool {
        self.0.matches_pattern_list(patterns)
    }

    /// `git_net_url_fmt`: the URL as text. Needs a scheme and a host.
    pub fn to_bytes(&self) -> core::result::Result<Vec<u8>, Error> {
        let mut b = GitStr::new();
        self.0.fmt(&mut b).map_err(Error::from)?;
        Ok(b.as_bytes().to_vec())
    }

    /// `git_net_url_fmt_path`: the path (or `/`) and the query.
    pub fn path_and_query(&self) -> core::result::Result<Vec<u8>, Error> {
        let mut b = GitStr::new();
        self.0.fmt_path(&mut b).map_err(Error::from)?;
        Ok(b.as_bytes().to_vec())
    }
}

/// `git_net_str_is_url`.
pub fn is_url(s: &[u8]) -> bool {
    str_is_url(s)
}

// ---------------------------------------------------------------------------
// The C ABI
// ---------------------------------------------------------------------------

/// The exports of `net.c`.
///
/// Every `git_net_url *` argument must point to an initialized
/// `git_net_url` (fields NULL or strings of `git__allocator` that the URL
/// owns), except the output of the parsers, which may be uninitialized
/// (they clear it first, as C's `memset`). No other thread may use the
/// URLs during the call, and string arguments must not point into the
/// strings of a URL the call writes or frees.
pub mod ffi {
    use super::*;
    use crate::util::export::{barrier, cstr_bytes, status};

    /// The export prologue of the parsers: clear `url` (C's `memset`) and
    /// run `f` on it.
    ///
    /// # Safety
    ///
    /// `url` is valid for writes of a `git_net_url` and not otherwise in
    /// use; `given` is a NUL-terminated string not within `*url`.
    unsafe fn parse_with(
        url: *mut GitNetUrl,
        given: *const c_char,
        f: fn(&mut GitNetUrl, &[u8]) -> Result<()>,
    ) -> c_int {
        barrier(|| {
            // SAFETY: the caller passes a writable `git_net_url`; its old
            // contents are overwritten unread, as `memset` does.
            unsafe { url.write(GitNetUrl::new()) };
            // SAFETY: just initialized; the caller gives us exclusive use.
            let url = unsafe { &mut *url };
            // SAFETY: a NUL-terminated string, not written during the call.
            let given = unsafe { cstr_bytes(given) };
            status(f(url, given))
        })
    }

    /// `git_net_hostname_matches_cert`.
    ///
    /// # Safety
    ///
    /// Both are NUL-terminated strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_hostname_matches_cert(
        hostname: *const c_char,
        pattern: *const c_char,
    ) -> bool {
        barrier(|| {
            // SAFETY: the caller passes NUL-terminated strings.
            let h = unsafe { cstr_bytes(hostname) };
            // SAFETY: as above.
            let p = unsafe { cstr_bytes(pattern) };
            hostname_matches_cert(h, p)
        })
    }

    /// `git_net_str_is_url`.
    ///
    /// # Safety
    ///
    /// `str` is a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_str_is_url(str: *const c_char) -> bool {
        // SAFETY: the caller passes a NUL-terminated string.
        barrier(|| str_is_url(unsafe { cstr_bytes(str) }))
    }

    /// `git_net_url_dup`.
    ///
    /// # Safety
    ///
    /// `out` and `in` are initialized URLs (see the module); they may be
    /// the same.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_dup(out: *mut GitNetUrl, in_: *mut GitNetUrl) -> c_int {
        barrier(|| {
            if core::ptr::eq(out, in_) {
                // Copy each string onto itself, leaking the original as C
                // does: read them through a bitwise snapshot that is never
                // dropped (the originals stay allocated: nothing frees them).
                // SAFETY: `in_` is a valid, initialized URL.
                let snapshot = mem::ManuallyDrop::new(unsafe { in_.read() });
                // SAFETY: exclusive use for the call; `snapshot` is a
                // separate value.
                let out = unsafe { &mut *out };
                return status(out.dup_from(&snapshot));
            }
            // SAFETY: distinct, initialized URLs in exclusive use.
            let out = unsafe { &mut *out };
            // SAFETY: as above.
            let src = unsafe { &*in_ };
            status(out.dup_from(src))
        })
    }

    /// `git_net_url_parse`.
    ///
    /// # Safety
    ///
    /// `url` is writable (its contents are overwritten unread); `str` is
    /// a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_parse(url: *mut GitNetUrl, str: *const c_char) -> c_int {
        // SAFETY: forwarded.
        unsafe { parse_with(url, str, parse) }
    }

    /// `git_net_url_parse_scp`.
    ///
    /// # Safety
    ///
    /// As [`git_net_url_parse`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_parse_scp(
        url: *mut GitNetUrl,
        str: *const c_char,
    ) -> c_int {
        // SAFETY: forwarded.
        unsafe { parse_with(url, str, parse_scp) }
    }

    /// `git_net_url_parse_standard_or_scp`.
    ///
    /// # Safety
    ///
    /// As [`git_net_url_parse`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_parse_standard_or_scp(
        url: *mut GitNetUrl,
        str: *const c_char,
    ) -> c_int {
        // SAFETY: forwarded. (The C cleared `url` only in the parser it
        // chose; both clear it.)
        unsafe { parse_with(url, str, parse_standard_or_scp) }
    }

    /// `git_net_url_parse_http`.
    ///
    /// # Safety
    ///
    /// As [`git_net_url_parse`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_parse_http(
        url: *mut GitNetUrl,
        str: *const c_char,
    ) -> c_int {
        // SAFETY: forwarded.
        unsafe { parse_with(url, str, parse_http) }
    }

    /// `git_net_url_joinpath`.
    ///
    /// # Safety
    ///
    /// `out` and `one` are initialized URLs (they may be the same: `out`
    /// is disposed first, so `one` is then empty, as in C); `path` is a
    /// NUL-terminated string not within `out`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_joinpath(
        out: *mut GitNetUrl,
        one: *mut GitNetUrl,
        path: *const c_char,
    ) -> c_int {
        barrier(|| {
            // SAFETY: a NUL-terminated string that the call does not free.
            let path = unsafe { cstr_bytes(path) };
            if core::ptr::eq(out, one) {
                // SAFETY: an initialized URL in exclusive use.
                let out = unsafe { &mut *out };
                out.dispose();
                return status(out.joinpath(&GitNetUrl::new(), path));
            }
            // SAFETY: distinct initialized URLs in exclusive use.
            let out = unsafe { &mut *out };
            // SAFETY: as above.
            let one = unsafe { &*one };
            status(out.joinpath(one, path))
        })
    }

    /// `git_net_url_apply_redirect`.
    ///
    /// # Safety
    ///
    /// `url` is NULL or an initialized URL; `redirect_location` is NULL or
    /// a NUL-terminated string, as is `service_suffix`; neither points into
    /// `url`'s strings.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_apply_redirect(
        url: *mut GitNetUrl,
        redirect_location: *const c_char,
        allow_offsite: bool,
        service_suffix: *const c_char,
    ) -> c_int {
        barrier(|| {
            git_assert!(!url.is_null(), "url", -1);
            git_assert!(!redirect_location.is_null(), "redirect_location", -1);
            // SAFETY: the caller's contract (checked non-null above).
            let url = unsafe { &mut *url };
            // SAFETY: a NUL-terminated string not within `url`.
            let loc = unsafe { cstr_bytes(redirect_location) };
            let suffix = if service_suffix.is_null() {
                None
            } else {
                // SAFETY: a non-null NUL-terminated string.
                Some(unsafe { cstr_bytes(service_suffix) })
            };
            status(url.apply_redirect(loc, allow_offsite, suffix))
        })
    }

    /// `git_net_url_valid`.
    ///
    /// # Safety
    ///
    /// `url` is an initialized URL.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_valid(url: *mut GitNetUrl) -> bool {
        // SAFETY: the caller's contract.
        barrier(|| unsafe { &*url }.is_valid())
    }

    /// `git_net_url_is_default_port`.
    ///
    /// # Safety
    ///
    /// `url` is an initialized URL.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_is_default_port(url: *mut GitNetUrl) -> bool {
        // SAFETY: the caller's contract.
        barrier(|| unsafe { &*url }.is_default_port())
    }

    /// `git_net_url_is_ipv6`.
    ///
    /// # Safety
    ///
    /// `url` is an initialized URL.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_is_ipv6(url: *mut GitNetUrl) -> bool {
        // SAFETY: the caller's contract.
        barrier(|| unsafe { &*url }.is_ipv6())
    }

    /// `git_net_url_swap`.
    ///
    /// # Safety
    ///
    /// `a` and `b` are valid for reads and writes of a `git_net_url` (they
    /// may be the same).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_swap(a: *mut GitNetUrl, b: *mut GitNetUrl) {
        // SAFETY: both valid; `ptr::swap` allows them to be equal. The
        // bytes are exchanged as C's `memcpy`s do, whatever they hold.
        barrier(|| unsafe { core::ptr::swap(a, b) })
    }

    /// `git_net_url_fmt`.
    ///
    /// # Safety
    ///
    /// `buf` is a valid `git_str`; `url` is NULL or an initialized URL
    /// whose strings are not within `buf`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_fmt(buf: *mut GitStr, url: *mut GitNetUrl) -> c_int {
        barrier(|| {
            git_assert_arg!(!url.is_null(), "url", -1);
            // SAFETY: the caller's contract; the two do not overlap.
            let buf = unsafe { &mut *buf };
            // SAFETY: as above.
            let url = unsafe { &*url };
            status(url.fmt(buf))
        })
    }

    /// `git_net_url_fmt_path`.
    ///
    /// # Safety
    ///
    /// As [`git_net_url_fmt`], with `url` not NULL.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_fmt_path(buf: *mut GitStr, url: *mut GitNetUrl) -> c_int {
        barrier(|| {
            // SAFETY: the caller's contract; the two do not overlap.
            let buf = unsafe { &mut *buf };
            // SAFETY: as above.
            let url = unsafe { &*url };
            status(url.fmt_path(buf))
        })
    }

    /// `git_net_url_matches_pattern`.
    ///
    /// # Safety
    ///
    /// `url` is an initialized URL; `pattern` a NUL-terminated string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_matches_pattern(
        url: *mut GitNetUrl,
        pattern: *const c_char,
    ) -> bool {
        barrier(|| {
            // SAFETY: the caller's contract.
            let url = unsafe { &*url };
            // SAFETY: a NUL-terminated string.
            let pattern = unsafe { cstr_bytes(pattern) };
            url.matches_pattern(pattern)
        })
    }

    /// `git_net_url_matches_pattern_list`.
    ///
    /// # Safety
    ///
    /// `url` is an initialized URL; `pattern_list` NULL or a NUL-terminated
    /// string.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_matches_pattern_list(
        url: *mut GitNetUrl,
        pattern_list: *const c_char,
    ) -> bool {
        barrier(|| {
            // SAFETY: the caller's contract.
            let url = unsafe { &*url };
            // SAFETY: NULL or a NUL-terminated string (NULL reads as empty,
            // as C's loop condition).
            let list = unsafe { cstr_bytes(pattern_list) };
            url.matches_pattern_list(list)
        })
    }

    /// `git_net_url_dispose`.
    ///
    /// # Safety
    ///
    /// `url` is an initialized URL.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn git_net_url_dispose(url: *mut GitNetUrl) {
        // SAFETY: the caller's contract.
        barrier(|| unsafe { &mut *url }.dispose())
    }
}

crate::c_layout_test! {
    headers: ["net.h"],
    types: [
        GitNetUrl => "git_net_url" {
            scheme, host, port, path, query, fragment, username, password, port_specified
        },
    ],
}
