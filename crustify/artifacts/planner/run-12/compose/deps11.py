"""dependencies.json to the schema of Crustify 043a8ae (dependencies-schema):
each library's candidates with one free-text quality_assessment, every entity
the campaign uses of it (c_entities) mapped to what the candidates and std
offer, the plan's recommendation, and the crustify campaign.

Run 10 read the crates, area by area, against the Adopted crates criteria; its
findings are carried here: each candidate's quality_assessment is composed from
its notes, shared state, soundness, advisories and maintenance (HEAD's
rust_equivalent), with the verdict run 10's commit gave it, and each entity's
matches follow the area run 10 assessed it in, with the item the candidate
offers for it. The entities are those of schedule.json's modules'
`depends_on.external` records (schedule.v11.json), each named by the public
header that declares it.

An entity's recommended route is its area's, as run 10 recommended them:
`crate` where a fit candidate covers the area, `std` where the standard library
serves it and no state crosses to the remaining C, `crustify-campaign` (or
`local`, for libpcre2-8) for the rest. libc's decision is the user's, carried
unchanged: no crate adopted, everything not `std` to its crustify campaign.

Writes crustify/artifacts/dependencies.json."""
import json, collections, subprocess, os, sys
R = '/target/libgit2-crustify/crustify/.worktrees/planner/run-12/'
T = R + 'crustify/tmp/planner-run-12/'
OLD = json.loads(subprocess.run(['git', '-C', R, 'show', 'HEAD:crustify/artifacts/dependencies.json'],
                                capture_output=True, check=True).stdout)
SCHED = json.load(open(T + 'schedule.v11.json'))

# ---- the entities each library's users reference, from schedule.json's external records
PUBLIC = [  # an internal glibc header -> the public header that declares what it defines
    ('asm-generic/errno-base.h', 'errno.h'), ('asm-generic/errno.h', 'errno.h'), ('asm-generic/socket.h', 'sys/socket.h'),
    ('bits/byteswap.h', 'byteswap.h'), ('bits/confname.h', 'unistd.h'), ('bits/dirent.h', 'dirent.h'),
    ('bits/fcntl-linux.h', 'fcntl.h'), ('bits/mman-linux.h', 'sys/mman.h'), ('bits/poll.h', 'poll.h'),
    ('bits/posix1_lim.h', 'limits.h'), ('bits/pthreadtypes.h', 'pthread.h'), ('bits/sigaction.h', 'signal.h'),
    ('bits/signum-generic.h', 'signal.h'), ('bits/sigthread.h', 'signal.h'), ('bits/socket.h', 'sys/socket.h'),
    ('bits/socket_type.h', 'sys/socket.h'), ('bits/stat.h', 'sys/stat.h'), ('bits/struct_stat.h', 'sys/stat.h'),
    ('bits/time.h', 'time.h'), ('bits/types/__sigset_t.h', 'signal.h'), ('bits/types/struct_FILE.h', 'stdio.h'),
    ('bits/types/struct_timespec.h', 'time.h'), ('bits/types/struct_timeval.h', 'sys/time.h'),
    ('bits/types/struct_tm.h', 'time.h'), ('bits/waitstatus.h', 'sys/wait.h'), ('sys/poll.h', 'poll.h')]
def header(path):
    h = path.replace('/usr/include/', '').replace('x86_64-linux-gnu/', '')
    return dict(PUBLIC).get(h, h)
FND = ' (foundational)'
USED = collections.defaultdict(lambda: collections.defaultdict(list))   # lib -> (name, kind, header) -> [module]
for m in SCHED['modules']:
    for e in m['depends_on']['external']:
        for it in e['items']:
            k = (it['to']['name'], it['kind'], header(it['to']['defined_in']))
            rec = {'link_unit': m['link_unit'], 'module': m['module'], 'foundational': m['foundational']}
            if rec not in USED[e['library']][k]: USED[e['library']][k].append(rec)

# ---- candidates: run 10's records and verdicts
VERDICT = {  # run 10's commit message: why a crate is, or is not, fit
    'chrono': 'Not fit: coverage. It has no mktime: no normalisation of out-of-range fields and no tm_isdst hint, so the clocks and calendar time area it was assessed for stays with the crustify campaign.',
    'nix': 'Fit for files, directories and file metadata; sockets, name resolution and polling; process identity, users and system information; and errno and error strings.',
    'rustix': 'Not fit: maintenance. One crates.io owner publishes it; it otherwise covers files and sockets.',
    'socket2': 'Fit for sockets, name resolution and polling; nix is recommended there, as it covers the other areas too.',
    'memmap2': 'Not fit: coverage and shared state. Its file-backed constructors are unsafe fns and it cannot adopt a mapping C created.',
    'sha2': 'Fit for the SHA-256 digest.', 'hmac': 'Fit for the NTLM digests, with md-5 and md4.',
    'md-5': 'Fit for the NTLM digests, as hmac\'s MD5.', 'md4': 'Fit for the NTLM digests (MD4).',
    'des': 'Fit for DES-ECB.', 'getrandom': 'Fit for random bytes.',
    'rustls-webpki': 'Not fit: coverage. No subject CN for the CN fallback and no ASN.1 string API.',
    'openssl': 'Not fit: soundness. A small fork fixing bread and X509StoreRef::objects would make it qualify for libssl and for libcrypto\'s X.509 and BIO areas.',
    'rustls': 'Not fit: coverage. No cipher lists, no verify-none-then-report flow, no CN fallback.',
    'native-tls': 'Not fit: soundness, through openssl\'s SslStream, and coverage.',
    'fancy-regex': 'Not fit: coverage. Not PCRE2\'s dialect.', 'regex': 'Not fit: coverage. Not PCRE2\'s dialect.',
    'pcre2': 'Not fit: maintenance. Last released 2025-09-25, by one maintainer.',
    'flate2': 'Fit for zlib streams and CRC-32.',
    'zlib-rs': 'Not fit: soundness unclear. Its unsafe beyond the stable API was not read and no scan was run.',
    'miniz_oxide': 'Fit for zlib streams, which flate2 serves through it.',
    'crc32fast': 'Not fit: maintenance. One maintainer.',
}
DROPPED = {'libz-rs-sys': 'no safe API: every entry point is an unsafe extern "C" fn over a raw z_stream'}
def assessment(c):
    adv = c.get('advisories') or []
    advs = ('No RustSec advisory affects ' + c['version'] + '.') if not adv else ('Advisories: ' + '; '.join(
        a if isinstance(a, str) else json.dumps(a) for a in adv) + '.')
    m = c['maintenance']
    so = c['soundness']
    share = ('It exposes the state it holds to C (raw descriptors or handles).' if c.get('shares_state')
             else 'It keeps its state in Rust: nothing it holds crosses to C.')
    return ' '.join([VERDICT[c['crate']], 'Coverage: ' + c['notes'], 'Shared state: ' + share,
                     f"Soundness ({so['judgement']}): {so['evidence']}", advs,
                     f"Maintenance: last released {m['last_release']}, published by {m['publisher']}; license {c['license']}."])
def candidates(old):
    out = []
    for kind, ck in (('ported', 'ported'), ('bindings', 'bindings')):
        for c in old['rust_equivalent'].get(ck, []):
            if c['crate'] in DROPPED: continue
            out.append({'crate': c['crate'], 'version': c['version'], 'license': c['license'], 'kind': kind,
                        'sys': c.get('sys') if kind == 'bindings' else None, 'quality_assessment': assessment(c)})
    return out

# ---- matches. m(crate, api, match, note); a spec is (route, (crate, api) or None, [matches])
def m(crate, api, match='exact', note=''):
    return {'crate': crate, 'rust_api': api, 'match': match, 'note': note}
def spec(route, rec, *ms): return (route, rec, list(ms))
CC = 'crustify-campaign'
NX = 'nix'
# libc ------------------------------------------------------------------------------------
FD = 'the descriptor crosses to the remaining C as the raw fd of an OwnedFd while C still reads it'
def nixfile(name, nx, rx, sd, sdmatch='differs', sdnote=FD, nxmatch='exact', nxnote=''):
    ms = [m(NX, nx, nxmatch, nxnote) if nx else m(NX, None, 'missing', nxnote)]
    if rx: ms.append(m('rustix', rx))
    if sd: ms.append(m('std', sd, sdmatch, sdnote))
    return spec('crate', (NX, nx), *ms) if nx else spec('std', ('std', sd), *ms)
LIBC = {}
# files, directories and file metadata: nix
for n, nx, rx, sd in [
        ('open', 'nix::fcntl::open', 'rustix::fs::open', 'std::fs::OpenOptions::open'),
        ('read', 'nix::unistd::read', 'rustix::io::read', 'std::io::Read::read'),
        ('write', 'nix::unistd::write', 'rustix::io::write', 'std::io::Write::write'),
        ('pread', 'nix::sys::uio::pread', 'rustix::io::pread', 'std::os::unix::fs::FileExt::read_at'),
        ('pwrite', 'nix::sys::uio::pwrite', 'rustix::io::pwrite', 'std::os::unix::fs::FileExt::write_at'),
        ('lseek', 'nix::unistd::lseek', 'rustix::fs::seek', 'std::io::Seek::seek'),
        ('fsync', 'nix::unistd::fsync', 'rustix::fs::fsync', 'std::fs::File::sync_all'),
        ('fstat', 'nix::sys::stat::fstat', 'rustix::fs::fstat', 'std::fs::File::metadata'),
        ('futimens', 'nix::sys::stat::futimens', 'rustix::fs::futimens', 'std::fs::File::set_times')]:
    LIBC[n] = nixfile(n, nx, rx, sd)
LIBC['fcntl'] = spec('crate', (NX, 'nix::fcntl::fcntl'), m(NX, 'nix::fcntl::fcntl'),
                     m('rustix', 'rustix::io::fcntl_getfd', 'differs', 'one function per command (fcntl_getfd/setfd, fs::fcntl_getfl/setfl)'),
                     m('std', None, 'missing', 'std sets FD_CLOEXEC itself and offers no descriptor flags'))
LIBC['close'] = spec('crate', (NX, 'nix::unistd::close'), m(NX, 'nix::unistd::close'),
                     m('rustix', 'std::os::fd::OwnedFd', 'differs', 'closing is dropping the OwnedFd'),
                     m('std', 'std::os::fd::OwnedFd', 'differs', 'closing is dropping the OwnedFd that owns the descriptor'))
PATHS = 'paths are &Path; errno is glibc\'s through std::io::Error::raw_os_error'
for n, nx, nxm, nxn, rx, sd, sdm in [
        ('stat', 'nix::sys::stat::stat', 'exact', '', 'rustix::fs::stat', 'std::fs::metadata', 'differs'),
        ('lstat', 'nix::sys::stat::lstat', 'exact', '', 'rustix::fs::lstat', 'std::fs::symlink_metadata', 'differs'),
        ('chmod', 'nix::sys::stat::fchmodat', 'differs', 'relative to AT_FDCWD', 'rustix::fs::chmod', 'std::fs::set_permissions', 'exact'),
        ('mkdir', 'nix::unistd::mkdir', 'exact', '', 'rustix::fs::mkdir', 'std::os::unix::fs::DirBuilderExt::mode', 'differs'),
        ('rmdir', 'nix::unistd::unlinkat', 'differs', 'with UnlinkatFlags::RemoveDir', 'rustix::fs::rmdir', 'std::fs::remove_dir', 'exact'),
        ('unlink', 'nix::unistd::unlink', 'exact', '', 'rustix::fs::unlink', 'std::fs::remove_file', 'exact'),
        ('link', 'nix::unistd::linkat', 'differs', 'relative to AT_FDCWD', 'rustix::fs::link', 'std::fs::hard_link', 'exact'),
        ('symlink', 'nix::unistd::symlinkat', 'differs', 'relative to AT_FDCWD', 'rustix::fs::symlink', 'std::os::unix::fs::symlink', 'exact'),
        ('readlink', 'nix::fcntl::readlink', 'exact', '', 'rustix::fs::readlink', 'std::fs::read_link', 'exact'),
        ('rename', 'nix::fcntl::renameat', 'differs', 'relative to AT_FDCWD', 'rustix::fs::rename', 'std::fs::rename', 'exact'),
        ('access', 'nix::unistd::access', 'exact', '', 'rustix::fs::access', None, None),
        ('getcwd', 'nix::unistd::getcwd', 'exact', '', 'rustix::process::getcwd', 'std::env::current_dir', 'exact'),
        ('chdir', 'nix::unistd::chdir', 'exact', '', 'rustix::process::chdir', 'std::env::set_current_dir', 'exact'),
        ('utimes', 'nix::sys::stat::utimes', 'exact', '', 'rustix::fs::utimensat', 'std::fs::File::set_times', 'differs')]:
    ms = [m(NX, nx, nxm, nxn), m('rustix', rx)]
    if sd: ms.append(m('std', sd, sdm, PATHS if sdm == 'differs' else ''))
    else: ms.append(m('std', None, 'missing', 'std checks no permissions without opening the file'))
    LIBC[n] = spec('crate', (NX, nx), *ms)
LIBC['remove'] = spec('crate', (NX, 'nix::unistd::unlink'),
                      m(NX, 'nix::unistd::unlink', 'differs', 'remove() also removes an empty directory; repository.c removes a file'),
                      m('std', 'std::fs::remove_file', 'differs', 'remove() also removes an empty directory; repository.c removes a file'))
LIBC['realpath'] = spec('std', ('std', 'std::fs::canonicalize'), m(NX, None, 'missing'), m('rustix', None, 'missing'),
                        m('std', 'std::fs::canonicalize', 'exact'))
for n in ('opendir', 'readdir', 'closedir', '__dirstream', 'dirent'):
    api = {'opendir': 'nix::dir::Dir::open', 'readdir': 'nix::dir::Iter', 'closedir': 'nix::dir::Dir',
           '__dirstream': 'nix::dir::Dir', 'dirent': 'nix::dir::Entry'}[n]
    LIBC[n] = spec('crate', (NX, api), m(NX, api, 'exact' if n != 'closedir' else 'differs',
                                         'closing is dropping the Dir' if n == 'closedir' else ''),
                   m('rustix', 'rustix::fs::Dir'),
                   m('std', 'std::fs::read_dir', 'differs', 'skips . and .., and yields DirEntry rather than a dirent'))
STATN = 'std::fs::Metadata through std::os::unix::fs::MetadataExt, not a struct stat'
for n in ('stat|struct', 'st_mtime', 'st_ctime'):
    LIBC[n] = spec('crate', (NX, 'nix::sys::stat::FileStat'), m(NX, 'nix::sys::stat::FileStat', 'exact', ''),
                   m('rustix', 'rustix::fs::Stat'), m('std', 'std::os::unix::fs::MetadataExt', 'differs', STATN))
OFL = {'O_APPEND': 'O_APPEND', 'O_CLOEXEC': 'O_CLOEXEC', '__O_CLOEXEC': 'O_CLOEXEC', 'O_CREAT': 'O_CREAT', 'O_EXCL': 'O_EXCL',
       'O_FSYNC': 'O_FSYNC', 'O_SYNC': 'O_SYNC', 'O_NONBLOCK': 'O_NONBLOCK', 'O_RDONLY': 'O_RDONLY', 'O_RDWR': 'O_RDWR',
       'O_TRUNC': 'O_TRUNC', 'O_WRONLY': 'O_WRONLY'}
for n, f in OFL.items():
    LIBC[n] = spec('crate', (NX, f'nix::fcntl::OFlag::{f}'), m(NX, f'nix::fcntl::OFlag::{f}'),
                   m('rustix', f'rustix::fs::OFlags::{f[2:]}'),
                   m('std', 'std::fs::OpenOptions', 'differs', 'a builder method rather than a flag; O_CLOEXEC is always set'))
for n, api in (('F_GETFD', 'nix::fcntl::FcntlArg::F_GETFD'), ('F_SETFD', 'nix::fcntl::FcntlArg::F_SETFD'),
               ('F_GETFL', 'nix::fcntl::FcntlArg::F_GETFL'), ('F_SETFL', 'nix::fcntl::FcntlArg::F_SETFL'),
               ('FD_CLOEXEC', 'nix::fcntl::FdFlag::FD_CLOEXEC')):
    LIBC[n] = spec('crate', (NX, api), m(NX, api), m('std', None, 'missing', 'std sets FD_CLOEXEC itself'))
MODE = 'std::fs::FileType\'s is_dir/is_symlink/is_file, and PermissionsExt::mode for the bits'
for n, api in (('S_IFMT', 'nix::sys::stat::SFlag::S_IFMT'), ('__S_IFMT', 'nix::sys::stat::SFlag::S_IFMT'),
               ('S_IFDIR', 'nix::sys::stat::SFlag::S_IFDIR'), ('__S_IFDIR', 'nix::sys::stat::SFlag::S_IFDIR'),
               ('S_IFLNK', 'nix::sys::stat::SFlag::S_IFLNK'), ('__S_IFLNK', 'nix::sys::stat::SFlag::S_IFLNK'),
               ('S_IFREG', 'nix::sys::stat::SFlag::S_IFREG'), ('__S_IFREG', 'nix::sys::stat::SFlag::S_IFREG'),
               ('S_ISGID', 'nix::sys::stat::Mode::S_ISGID'), ('__S_ISGID', 'nix::sys::stat::Mode::S_ISGID'),
               ('S_IXUSR', 'nix::sys::stat::Mode::S_IXUSR'), ('__S_IEXEC', 'nix::sys::stat::Mode::S_IXUSR'),
               ('S_ISDIR', 'nix::sys::stat::SFlag::S_IFDIR'), ('S_ISLNK', 'nix::sys::stat::SFlag::S_IFLNK'),
               ('S_ISREG', 'nix::sys::stat::SFlag::S_IFREG'), ('__S_ISTYPE', 'nix::sys::stat::SFlag::S_IFMT')):
    tst = n in ('S_ISDIR', 'S_ISLNK', 'S_ISREG', '__S_ISTYPE')
    LIBC[n] = spec('crate', (NX, api), m(NX, api, 'differs' if tst else 'exact',
                                         'a test of (st_mode & S_IFMT) against the flag' if tst else ''),
                   m('rustix', 'rustix::fs::FileType'), m('std', 'std::fs::FileType', 'differs', MODE))
for n, api, sd in (('SEEK_SET', 'nix::unistd::Whence::SeekSet', 'std::io::SeekFrom::Start'),
                   ('SEEK_END', 'nix::unistd::Whence::SeekEnd', 'std::io::SeekFrom::End')):
    LIBC[n] = spec('crate', (NX, api), m(NX, api), m('rustix', 'rustix::fs::SeekFrom'), m('std', sd, 'exact'))
for n, api in (('F_OK', 'nix::unistd::AccessFlags::F_OK'), ('R_OK', 'nix::unistd::AccessFlags::R_OK')):
    LIBC[n] = spec('crate', (NX, api), m(NX, api), m('rustix', 'rustix::fs::Access'), m('std', None, 'missing', 'std checks no permissions'))
for n, sd in (('STDIN_FILENO', 'std::io::stdin'), ('STDOUT_FILENO', 'std::io::stdout'), ('STDERR_FILENO', 'std::io::stderr')):
    LIBC[n] = spec('std', ('std', sd), m(NX, None, 'missing', 'libc\'s constant, re-exported'),
                   m('std', sd, 'differs', 'its AsFd gives the descriptor'))
# sockets, name resolution and polling: nix
SK = 'socket2 builds sockets as values (Socket::new, Domain, Type, Protocol) rather than by constants'
for n, nx, s2, sd in [
        ('socket', 'nix::sys::socket::socket', 'socket2::Socket::new', None),
        ('connect', 'nix::sys::socket::connect', 'socket2::Socket::connect', 'std::net::TcpStream::connect'),
        ('send', 'nix::sys::socket::send', 'socket2::Socket::send', 'std::io::Write::write'),
        ('recv', 'nix::sys::socket::recv', 'socket2::Socket::recv', 'std::io::Read::read'),
        ('getsockopt', 'nix::sys::socket::getsockopt', 'socket2::Socket::take_error', None),
        ('setsockopt', 'nix::sys::socket::setsockopt', 'socket2::Socket::set_tcp_keepalive', None)]:
    ms = [m(NX, nx), m('socket2', s2, 'exact' if n in ('socket', 'connect', 'send', 'recv') else 'differs',
                       '' if n in ('socket', 'connect', 'send', 'recv') else 'a method per option: SO_ERROR as take_error, SO_KEEPALIVE and TCP_KEEP* as set_tcp_keepalive')]
    ms.append(m('rustix', 'rustix::net::' + {'socket': 'socket_with', 'getsockopt': 'sockopt::socket_error', 'setsockopt': 'sockopt::set_socket_keepalive'}.get(n, n)))
    if sd: ms.append(m('std', sd, 'differs', 'std connects with blocking resolution and no SO_ERROR poll; socket.c connects non-blocking and polls'))
    LIBC[n] = spec('crate', (NX, nx), *ms)
LIBC['poll'] = spec('crate', (NX, 'nix::poll::poll'), m(NX, 'nix::poll::poll'), m('rustix', 'rustix::event::poll'),
                    m('socket2', None, 'missing', 'only SO_SNDTIMEO/SO_RCVTIMEO timeouts, where socket.c polls on EAGAIN'))
LIBC['pollfd'] = spec('crate', (NX, 'nix::poll::PollFd'), m(NX, 'nix::poll::PollFd'), m('rustix', 'rustix::event::PollFd'))
for n in ('POLLIN', 'POLLOUT', 'POLLERR', 'POLLHUP', 'POLLPRI'):
    LIBC[n] = spec('crate', (NX, f'nix::poll::PollFlags::{n}'), m(NX, f'nix::poll::PollFlags::{n}'), m('rustix', f'rustix::event::PollFlags::{n[4:]}'))
for n, nx, s2 in (('AF_INET', 'nix::sys::socket::AddressFamily::Inet', 'socket2::Domain::IPV4'),
                  ('PF_INET', 'nix::sys::socket::AddressFamily::Inet', 'socket2::Domain::IPV4'),
                  ('AF_INET6', 'nix::sys::socket::AddressFamily::Inet6', 'socket2::Domain::IPV6'),
                  ('PF_INET6', 'nix::sys::socket::AddressFamily::Inet6', 'socket2::Domain::IPV6'),
                  ('AF_UNSPEC', 'nix::sys::socket::AddressFamily::Unspec', None),
                  ('PF_UNSPEC', 'nix::sys::socket::AddressFamily::Unspec', None),
                  ('SOCK_STREAM', 'nix::sys::socket::SockType::Stream', 'socket2::Type::STREAM'),
                  ('SOCK_CLOEXEC', 'nix::sys::socket::SockFlag::SOCK_CLOEXEC', 'socket2::Socket::new'),
                  ('IPPROTO_TCP', 'nix::sys::socket::SockProtocol::Tcp', 'socket2::Protocol::TCP'),
                  ('SOL_SOCKET', 'nix::sys::socket::sockopt', None),
                  ('SO_ERROR', 'nix::sys::socket::sockopt::SocketError', 'socket2::Socket::take_error'),
                  ('SO_KEEPALIVE', 'nix::sys::socket::sockopt::KeepAlive', 'socket2::Socket::set_keepalive'),
                  ('TCP_KEEPIDLE', 'nix::sys::socket::sockopt::TcpKeepIdle', 'socket2::TcpKeepalive::with_time'),
                  ('TCP_KEEPINTVL', 'nix::sys::socket::sockopt::TcpKeepInterval', 'socket2::TcpKeepalive::with_interval'),
                  ('TCP_KEEPCNT', 'nix::sys::socket::sockopt::TcpKeepCount', 'socket2::TcpKeepalive::with_retries')):
    ms = [m(NX, nx, 'differs' if n == 'SOL_SOCKET' else 'exact', 'the level is implied by the typed option' if n == 'SOL_SOCKET' else '')]
    if s2: ms.append(m('socket2', s2, 'differs', SK))
    LIBC[n] = spec('crate', (NX, nx), *ms)
for n, nx, s2 in (('sockaddr', 'nix::sys::socket::SockaddrStorage', 'socket2::SockAddr'),
                  ('in_addr', 'nix::sys::socket::SockaddrIn', 'socket2::SockAddr'),
                  ('in6_addr', 'nix::sys::socket::SockaddrIn6', 'socket2::SockAddr')):
    LIBC[n] = spec('crate', (NX, nx), m(NX, nx), m('socket2', s2),
                   m('std', 'std::net::SocketAddr', 'differs', 'a Rust value, converted at the boundary'))
GAI = 'std resolves through glibc\'s getaddrinfo itself; a failure is an io::Error carrying gai_strerror\'s text, not the EAI_* code'
LIBC['getaddrinfo'] = spec('std', ('std', 'std::net::ToSocketAddrs::to_socket_addrs'), m(NX, None, 'missing'), m('socket2', None, 'missing'),
                           m('std', 'std::net::ToSocketAddrs::to_socket_addrs', 'differs', GAI))
LIBC['freeaddrinfo'] = spec('std', ('std', 'std::vec::IntoIter'), m('std', 'std::vec::IntoIter', 'differs', 'the addresses are an owned iterator, freed when dropped'))
LIBC['addrinfo'] = spec('std', ('std', 'std::net::SocketAddr'), m(NX, None, 'missing'),
                        m('std', 'std::net::SocketAddr', 'differs', 'each result is a SocketAddr; socktype and protocol are the caller\'s'))
LIBC['gai_strerror'] = spec(CC, None, m(NX, None, 'missing'),
                            m('std', 'std::io::Error', 'differs', 'its Display prefixes gai_strerror\'s text; the bare text is the wrapper\'s'))
LIBC['inet_pton'] = spec('std', ('std', 'std::net::IpAddr::from_str'), m(NX, None, 'missing'), m('std', 'std::net::IpAddr::from_str'))
for n, sd in (('htonl', 'u32::to_be'), ('htons', 'u16::to_be'), ('ntohl', 'u32::from_be'), ('ntohs', 'u16::from_be'),
              ('__bswap_16', 'u16::swap_bytes'), ('__bswap_32', 'u32::swap_bytes')):
    LIBC[n] = spec('std', ('std', sd), m('std', sd))
# process identity, users and system information: nix
for n in ('getuid', 'geteuid', 'getgid', 'getpid', 'getppid', 'getpgid', 'getsid'):
    ms = [m(NX, f'nix::unistd::{n}'), m('rustix', f'rustix::process::{n}')]
    if n == 'getpid': ms.append(m('std', 'std::process::id'))
    LIBC[n] = spec('crate', (NX, f'nix::unistd::{n}'), *ms)
for n in ('getpwuid_r', 'passwd', '_SC_GETPW_R_SIZE_MAX'):
    LIBC[n] = spec('crate', (NX, 'nix::unistd::User::from_uid'),
                   m(NX, 'nix::unistd::User::from_uid', 'exact' if n != '_SC_GETPW_R_SIZE_MAX' else 'differs',
                     'from_uid sizes its own buffer' if n == '_SC_GETPW_R_SIZE_MAX' else ''), m('rustix', None, 'missing'))
LIBC['sysconf'] = spec('crate', (NX, 'nix::unistd::sysconf'), m(NX, 'nix::unistd::sysconf'),
                       m('rustix', 'rustix::param::page_size', 'differs', 'page size only'))
for n, api in (('_SC_NPROCESSORS_ONLN', 'nix::unistd::SysconfVar::_NPROCESSORS_ONLN'), ('_SC_PAGESIZE', 'nix::unistd::SysconfVar::PAGE_SIZE'),
               ('_SC_PAGE_SIZE', 'nix::unistd::SysconfVar::PAGE_SIZE')):
    ms = [m(NX, api), m('rustix', 'rustix::param::page_size' if 'PAGE' in n else None, 'exact' if 'PAGE' in n else 'missing')]
    if 'NPROC' in n: ms.append(m('std', 'std::thread::available_parallelism', 'differs', 'honours the affinity mask and cgroup quota'))
    LIBC[n] = spec('crate', (NX, api), *ms)
LIBC['getloadavg'] = spec('crate', (NX, 'nix::sys::sysinfo::SysInfo::load_average'),
                          m(NX, 'nix::sys::sysinfo::SysInfo::load_average', 'differs', 'through sysinfo(2), at 1/65536 precision; rand.c only seeds from it'),
                          m('rustix', 'rustix::system::sysinfo', 'differs', 'raw loads'))
LIBC['getenv'] = spec('std', ('std', 'std::env::var_os'), m('std', 'std::env::var_os', 'exact'))
# errno and error strings: nix
ERRC = {'EACCES', 'EAGAIN', 'EBUSY', 'EEXIST', 'EINTR', 'EINVAL', 'ENOENT', 'ENOMEM', 'ENOTDIR', 'EPIPE', 'ERANGE',
        'EINPROGRESS', 'ENAMETOOLONG', 'ENOTEMPTY', 'ETIMEDOUT', 'EWOULDBLOCK'}
for n in ERRC:
    LIBC[n] = spec('crate', (NX, f'nix::errno::Errno::{n}'), m(NX, f'nix::errno::Errno::{n}'),
                   m('rustix', f'rustix::io::Errno::{n[1:]}'),
                   m('std', 'std::io::ErrorKind', 'differs', 'a kind, not the code; raw_os_error gives the code'))
for n in ('errno', '__errno_location'):
    LIBC[n] = spec('crate', (NX, 'nix::errno::Errno::last'),
                   m(NX, 'nix::errno::Errno::last', 'exact', ''),
                   m('std', 'std::io::Error::last_os_error', 'differs', 'reads errno but cannot set it, which C callers of ported functions read'))
LIBC['strerror'] = spec(CC, None, m(NX, 'nix::errno::Errno::desc', 'differs', 'nix\'s own table: EAGAIN gives \'Try again\', glibc \'Resource temporarily unavailable\''),
                        m('std', 'std::io::Error::from_raw_os_error', 'differs', 'glibc\'s text, with \' (os error N)\' appended'))
LIBC['perror'] = spec(CC, None, m('std', 'std::eprintln', 'differs', 'writes through Rust\'s stderr, not the FILE* the cli\'s C also writes'))
# processes and signals
PROC = 'std::process::Command forks, sets up the pipes and the working directory and execs in one call'
for n, nx, sd in (('fork', 'nix::unistd::fork', 'std::process::Command::spawn'), ('execve', 'nix::unistd::execve', 'std::process::Command::spawn'),
                  ('pipe', 'nix::unistd::pipe', 'std::process::Stdio::piped'), ('dup2', 'nix::unistd::dup2', 'std::process::Stdio'),
                  ('waitpid', 'nix::sys::wait::waitpid', 'std::process::Child::wait')):
    LIBC[n] = spec('std', ('std', sd), m(NX, nx, 'differs' if n == 'fork' else 'exact', 'an unsafe fn' if n == 'fork' else ''),
                   m('std', sd, 'differs', PROC))
for n, sd in (('WEXITSTATUS', 'std::process::ExitStatus::code'), ('__WEXITSTATUS', 'std::process::ExitStatus::code'),
              ('WIFEXITED', 'std::process::ExitStatus::code'), ('__WIFEXITED', 'std::process::ExitStatus::code'),
              ('WIFSIGNALED', 'std::os::unix::process::ExitStatusExt::signal'), ('__WIFSIGNALED', 'std::os::unix::process::ExitStatusExt::signal'),
              ('WTERMSIG', 'std::os::unix::process::ExitStatusExt::signal'), ('__WTERMSIG', 'std::os::unix::process::ExitStatusExt::signal')):
    LIBC[n] = spec('std', ('std', sd), m(NX, 'nix::sys::wait::WaitStatus'), m('std', sd, 'differs', 'an Option rather than a test and a field'))
SIG = 'signal masks stay the crustify campaign\'s (run 10): sighandler.c and process.c share the process\'s mask with the remaining C'
for n, nx in (('sigaddset', 'nix::sys::signal::SigSet::add'), ('sigemptyset', 'nix::sys::signal::SigSet::empty'),
              ('sigismember', 'nix::sys::signal::SigSet::contains'), ('sigwait', 'nix::sys::signal::SigSet::wait'),
              ('pthread_sigmask', 'nix::sys::signal::pthread_sigmask'), ('__sigset_t', 'nix::sys::signal::SigSet'),
              ('SIG_BLOCK', 'nix::sys::signal::SigmaskHow::SIG_BLOCK'), ('SIG_SETMASK', 'nix::sys::signal::SigmaskHow::SIG_SETMASK'),
              ('SIGINT', 'nix::sys::signal::Signal::SIGINT'), ('SIGPIPE', 'nix::sys::signal::Signal::SIGPIPE'),
              ('SIG_DFL', 'nix::sys::signal::SigHandler::SigDfl'), ('signal', 'nix::sys::signal::signal'),
              ('SIG_ERR', None), ('sigpending', None)):
    LIBC[n] = spec(CC, None, m(NX, nx, 'differs' if n == 'signal' else ('missing' if nx is None else 'exact'),
                               'an unsafe fn' if n == 'signal' else ''),
                   m('std', None, 'missing', SIG))
for n, sd in (('exit', 'std::process::exit'), ('abort', 'std::process::abort')):
    LIBC[n] = spec('std', ('std', sd), m('std', sd))
# clocks and calendar time: std for the clocks; calendar conversion to the crustify campaign
for n, sd, note in (('time', 'std::time::SystemTime::now', ''), ('gettimeofday', 'std::time::SystemTime::now', ''),
                    ('clock_gettime', 'std::time::Instant::now', ''), ('CLOCK_MONOTONIC', 'std::time::Instant', ''),
                    ('difftime', 'std::time::Duration', 'a Duration between two times rather than a double'),
                    ('timespec', 'std::time::Duration', 'a Duration since the epoch'),
                    ('timeval', 'std::time::Duration', 'a Duration since the epoch')):
    LIBC[n] = spec('std', ('std', sd), m('std', sd, 'differs' if note else 'exact', note),
                   m('chrono', 'chrono::Utc::now', 'differs', 'a DateTime') if n in ('time', 'gettimeofday') else
                   m(NX, 'nix::time::clock_gettime') if n == 'clock_gettime' else m('std', sd, 'differs' if note else 'exact', note))
    LIBC[n] = (LIBC[n][0], LIBC[n][1], [x for i, x in enumerate(LIBC[n][2]) if x not in LIBC[n][2][:i]])
CAL = 'no mktime: no normalisation of out-of-range fields (date.c:564) and no tm_isdst hint (signature.c:172)'
for n, ch in (('gmtime_r', 'chrono::DateTime::from_timestamp'), ('localtime_r', 'chrono::Local::timestamp_opt'),
              ('mktime', 'chrono::Local::from_local_datetime'), ('tm', 'chrono::NaiveDateTime')):
    LIBC[n] = spec(CC, None, m('chrono', ch, 'differs', CAL), m('std', None, 'missing', 'std has no calendar'))
# memory maps
for n, nx in (('mmap', 'nix::sys::mman::mmap'), ('munmap', 'nix::sys::mman::munmap'), ('MAP_PRIVATE', 'nix::sys::mman::MapFlags::MAP_PRIVATE'),
              ('MAP_SHARED', 'nix::sys::mman::MapFlags::MAP_SHARED'), ('PROT_WRITE', 'nix::sys::mman::ProtFlags::PROT_WRITE'),
              ('MAP_FAILED', None)):
    LIBC[n] = spec(CC, None, m('memmap2', 'memmap2::MmapOptions::map' if n in ('mmap', 'MAP_SHARED', 'MAP_PRIVATE', 'PROT_WRITE') else
                               ('memmap2::Mmap' if n == 'munmap' else None), 'missing' if n == 'MAP_FAILED' else 'differs',
                               'an unsafe constructor; cannot adopt or unmap a git_map C created' if n != 'MAP_FAILED' else 'an io::Error'),
                   m(NX, nx, 'missing' if nx is None else ('differs' if n in ('mmap', 'munmap') else 'exact'),
                     'an unsafe fn' if n in ('mmap', 'munmap') else ''))
# threads and locks
LIBC['pthread_create'] = spec('std', ('std', 'std::thread::spawn'), m('std', 'std::thread::spawn', 'differs', 'a closure rather than a C start routine and its argument'))
LIBC['pthread_join'] = spec('std', ('std', 'std::thread::JoinHandle::join'), m('std', 'std::thread::JoinHandle::join', 'differs', 'the result is the closure\'s'))
LOCK = 'the lock lives in C structs (git_mutex, git_rwlock, git_cond) that remaining C locks; a std lock cannot be embedded in C memory'
for n in ('pthread_mutex_init', 'pthread_mutex_destroy', 'pthread_mutex_lock', 'pthread_mutex_unlock', 'pthread_mutex_t',
          'pthread_mutexattr_t', 'pthread_rwlock_init', 'pthread_rwlock_destroy', 'pthread_rwlock_rdlock', 'pthread_rwlock_wrlock',
          'pthread_rwlock_unlock', 'pthread_rwlock_t', 'pthread_rwlockattr_t', 'pthread_cond_init', 'pthread_cond_destroy',
          'pthread_cond_signal', 'pthread_cond_wait', 'pthread_cond_t', 'pthread_condattr_t', 'pthread_attr_t'):
    sd = ('std::sync::Mutex' if 'mutex' in n else 'std::sync::RwLock' if 'rwlock' in n else
          'std::sync::Condvar' if 'cond' in n else 'std::thread::Builder')
    LIBC[n] = spec(CC, None, m('std', sd, 'differs', LOCK if n != 'pthread_attr_t' else 'the attributes are the builder\'s'))
TLS = 'thread.c\'s git_tlsdata keys are created at run time with destructors, and their values are C pointers; thread_local! is static'
for n in ('pthread_key_create', 'pthread_key_delete', 'pthread_getspecific', 'pthread_setspecific'):
    LIBC[n] = spec(CC, None, m('std', 'std::thread_local', 'differs', TLS))
# memory allocation: across the boundary, the crustify campaign's
MEM = 'memory handed to or freed by the remaining C must come from, and go back to, C\'s allocator'
for n, sd in (('malloc', 'std::alloc::alloc'), ('calloc', 'std::alloc::alloc_zeroed'), ('realloc', 'std::alloc::realloc'), ('free', 'std::alloc::dealloc')):
    LIBC[n] = spec(CC, None, m('std', sd, 'differs', MEM + '; std needs the Layout at free'))
LIBC['alloca'] = spec('std', ('std', 'alloc::vec::Vec'), m('std', 'alloc::vec::Vec', 'differs', 'a heap Vec for opt.c\'s stack array; nothing crosses to C'))
# strings and memory: std
for n, sd, note in (('memchr', 'core::slice::<[u8]>::iter().position', ''), ('memcmp', 'core::cmp::Ord::cmp', 'slices compare lexicographically, as memcmp does'),
                    ('memcpy', 'core::slice::<[T]>::copy_from_slice', ''), ('memmove', 'core::slice::<[T]>::copy_within', ''),
                    ('memset', 'core::slice::<[T]>::fill', ''), ('strchr', 'core::slice::<[u8]>::iter().position', 'on the bytes before the NUL'),
                    ('strrchr', 'core::slice::<[u8]>::iter().rposition', 'on the bytes before the NUL'),
                    ('strcmp', 'core::ffi::CStr::cmp', ''), ('strncmp', 'core::cmp::Ord::cmp', 'on the prefixes, stopping at a NUL'),
                    ('strcpy', 'core::slice::<[T]>::copy_from_slice', 'with the NUL'), ('strncpy', 'core::slice::<[T]>::copy_from_slice', 'zero-padding the rest'),
                    ('strlen', 'core::ffi::CStr::count_bytes', ''), ('strnlen', 'core::slice::<[u8]>::iter().position', 'bounded by the slice'),
                    ('strspn', 'core::slice::<[u8]>::iter().position', 'the first byte not in the set'),
                    ('strcspn', 'core::slice::<[u8]>::iter().position', 'the first byte in the set'),
                    ('strstr', 'core::slice::<[u8]>::windows', 'a search over windows; the memchr crate is left out'),
                    ('strcasecmp', 'core::slice::<[u8]>::eq_ignore_ascii_case', 'ASCII folding, as the C locale does'),
                    ('strncasecmp', 'core::slice::<[u8]>::eq_ignore_ascii_case', 'ASCII folding on the prefixes'),
                    ('strdup', 'alloc::ffi::CString::from', 'ntlm.c\'s copies stay in its own Rust once ported')):
    LIBC[n] = spec('std', ('std', sd), m('std', sd, 'differs' if note else 'exact', note))
CT = 'ASCII classification; libgit2 runs in the C locale, where glibc\'s tables agree'
for n in ('isalnum', 'isalpha', 'isascii', 'isblank', 'iscntrl', 'isdigit', 'isgraph', 'islower', 'isprint', 'ispunct',
          'isspace', 'isupper', 'isxdigit', 'tolower', 'toupper', '__isascii', '__isctype', '__tobody',
          '__ctype_b_loc', '__ctype_tolower_loc', '__ctype_toupper_loc'):
    base = n.strip('_')
    sd = {'isascii': 'u8::is_ascii', 'isblank': 'u8::is_ascii_whitespace', 'isspace': 'u8::is_ascii_whitespace',
          'isxdigit': 'u8::is_ascii_hexdigit', 'isdigit': 'u8::is_ascii_digit', 'isalnum': 'u8::is_ascii_alphanumeric',
          'isalpha': 'u8::is_ascii_alphabetic', 'iscntrl': 'u8::is_ascii_control', 'isgraph': 'u8::is_ascii_graphic',
          'islower': 'u8::is_ascii_lowercase', 'isupper': 'u8::is_ascii_uppercase', 'ispunct': 'u8::is_ascii_punctuation',
          'isprint': 'u8::is_ascii_graphic', 'tolower': 'u8::to_ascii_lowercase', 'toupper': 'u8::to_ascii_uppercase'}.get(base,
          'u8::to_ascii_lowercase' if 'tolower' in n else 'u8::to_ascii_uppercase' if 'toupper' in n else 'u8::is_ascii')
    note = CT + ('; isspace includes \\v, which is_ascii_whitespace does not' if base == 'isspace' else
                 '; isblank is space and tab only' if base == 'isblank' else '; isprint includes the space' if base == 'isprint' else '')
    LIBC[n] = spec('std', ('std', sd), m('std', sd, 'differs', note))
# FILE* streams and C formatting: the crustify campaign's
FILE = 'the cli\'s remaining C writes the same FILE* streams; Rust\'s std::io handles buffer separately'
for n, sd in (('fflush', 'std::io::Write::flush'), ('fileno', 'std::os::fd::AsFd::as_fd'), ('fprintf', 'std::write'),
              ('fputc', 'std::io::Write::write_all'), ('fwrite', 'std::io::Write::write_all'), ('printf', 'std::print'),
              ('vfprintf', 'std::write'), ('stderr', 'std::io::stderr'), ('stdin', 'std::io::stdin'), ('stdout', 'std::io::stdout'),
              ('_IO_FILE', 'std::io::Stdout')):
    LIBC[n] = spec(CC, None, m('std', sd, 'differs', FILE))
for n in ('snprintf', 'vsnprintf'):
    LIBC[n] = spec(CC, None, m('std', 'std::format', 'differs', 'Rust formatting, not printf\'s: the format strings and va_lists come from the remaining C (git_str_printf, git_error_set)'))
# conversions, sorting and randomness
for n, sd in (('strtol', 'i64::from_str_radix'), ('strtoul', 'u64::from_str_radix'), ('strtoull', 'u64::from_str_radix')):
    LIBC[n] = spec('std', ('std', sd), m('std', sd, 'differs', 'no end pointer, no leading whitespace or base prefix: the caller scans the digits'))
LIBC['qsort'] = spec('std', ('std', 'core::slice::<[T]>::sort_unstable_by'), m('std', 'core::slice::<[T]>::sort_unstable_by', 'differs', 'a closure comparator'))
LIBC['qsort_r'] = spec('std', ('std', 'core::slice::<[T]>::sort_unstable_by'), m('std', 'core::slice::<[T]>::sort_unstable_by', 'differs', 'a closure comparator, which calls the C comparator and payload util.c\'s callers pass'))
LIBC['rand'] = spec(CC, None, m('std', None, 'missing', 'std has no PRNG; reftable\'s system.c only seeds a retry jitter'))
LIBC['getentropy'] = spec(CC, None, m(NX, None, 'missing'), m('rustix', 'rustix::rand::getrandom', 'differs', 'getrandom(2) rather than getentropy'),
                          m('std', None, 'missing', 'no public entropy API'))
# integer limits, assertions
for n, sd in (('INT32_MAX', 'i32::MAX'), ('INT64_MAX', 'i64::MAX'), ('INT64_MIN', 'i64::MIN'), ('SIZE_MAX', 'usize::MAX'),
              ('UINT16_MAX', 'u16::MAX'), ('UINT32_MAX', 'u32::MAX'), ('UINT64_MAX', 'u64::MAX'), ('SSIZE_MAX', 'isize::MAX'),
              ('INT64_C', 'i64'), ('UINT64_C', 'u64'), ('__INT64_C', 'i64'), ('__UINT64_C', 'u64'), ('MIN', 'core::cmp::min')):
    LIBC[n] = spec('std', ('std', sd), m('std', sd, 'exact', ''))
for n in ('assert', '__ASSERT_VOID_CAST'):
    LIBC[n] = spec('std', ('std', 'core::debug_assert'), m('std', 'core::debug_assert', 'exact' if n == 'assert' else 'differs',
                                                            '' if n == 'assert' else 'part of assert\'s expansion'))

# libcrypto -------------------------------------------------------------------------------
LIBCRYPTO = {}
for n, api in (('SHA256_Init', 'sha2::Sha256::new'), ('SHA256_Update', 'sha2::Digest::update'),
               ('SHA256_Final', 'sha2::Digest::finalize'), ('SHA256state_st', 'sha2::Sha256')):
    LIBCRYPTO[n] = spec('crate', ('sha2', api), m('sha2', api, 'exact' if n != 'SHA256state_st' else 'differs',
                                                  'not a C-layout SHA256_CTX: git_hash_ctx embeds it by value until hash.c\'s port/layout' if n == 'SHA256state_st' else ''),
                        m('openssl', 'openssl::sha::Sha256', 'exact'))
for n, api in (('HMAC_CTX_new', 'hmac::Mac::new_from_slice'), ('HMAC_CTX_reset', 'hmac::Mac::new_from_slice'),
               ('HMAC_CTX_free', 'hmac::Hmac'), ('HMAC_Init_ex', 'hmac::Mac::new_from_slice'), ('HMAC_Update', 'hmac::Mac::update'),
               ('HMAC_Final', 'hmac::Mac::finalize'), ('hmac_ctx_st', 'hmac::Hmac'), ('engine_st', None)):
    note = {'HMAC_CTX_new': 'the context is created with its key', 'HMAC_CTX_reset': 'a re-init is a new value',
            'HMAC_CTX_free': 'dropped', 'engine_st': 'HMAC_Init_ex is passed a NULL engine'}.get(n, '')
    LIBCRYPTO[n] = spec('crate', ('hmac', api) if api else None, m('hmac', api, 'missing' if api is None else ('differs' if note else 'exact'), note),
                        m('openssl', 'openssl::sign::Signer', 'differs', 'PKey::hmac with Signer'))
    if api is None: LIBCRYPTO[n] = ('crate', ('hmac', 'hmac::Mac::new_from_slice'), LIBCRYPTO[n][2])
for n in ('EVP_md5', 'evp_md_st'):
    LIBCRYPTO[n] = spec('crate', ('md-5', 'md5::Md5'), m('md-5', 'md5::Md5', 'differs', 'a type parameter of Hmac<Md5> rather than a value'),
                        m('openssl', 'openssl::hash::MessageDigest::md5'))
LIBCRYPTO['MD4'] = spec('crate', ('md4', 'md4::Md4::digest'), m('md4', 'md4::Md4::digest'),
                        m('openssl', 'openssl::md::Md::fetch', 'differs', 'through EVP, which needs OpenSSL 3\'s legacy provider'))
for n, api in (('DES_set_key', 'des::Des::new_from_slice'), ('DES_ecb_encrypt', 'cipher::BlockEncrypt::encrypt_block'),
               ('DES_ks', 'des::Des'), ('DES_ENCRYPT', 'cipher::BlockEncrypt')):
    LIBCRYPTO[n] = spec('crate', ('des', api), m('des', api, 'exact' if n in ('DES_set_key', 'DES_ecb_encrypt') else 'differs',
                                                '' if n in ('DES_set_key', 'DES_ecb_encrypt') else 'a type or trait rather than a schedule or a flag'),
                        m('openssl', 'openssl::symm::Cipher::des_ecb', 'differs', 'through EVP, which needs OpenSSL 3\'s legacy provider'))
LIBCRYPTO['RAND_bytes'] = spec('crate', ('getrandom', 'getrandom::fill'),
                               m('getrandom', 'getrandom::fill', 'differs', 'the kernel\'s CSPRNG rather than OpenSSL\'s DRBG; a getrandom::Error on failure'),
                               m('openssl', 'openssl::rand::rand_bytes'))
X509 = 'the certificate comes from the remaining C TLS stream (SSL_get1_peer_certificate) and from git_libgit2_opts\' X509*'
for n, api in (('X509_free', 'openssl::x509::X509'), ('X509_get_subject_name', 'openssl::x509::X509Ref::subject_name'),
               ('X509_NAME_get_index_by_NID', 'openssl::x509::X509NameRef::entries_by_nid'),
               ('X509_NAME_get_entry', 'openssl::x509::X509NameRef::entries_by_nid'), ('X509_NAME_ENTRY_get_data', 'openssl::x509::X509NameEntryRef::data'),
               ('X509_get_ext_d2i', 'openssl::x509::X509Ref::subject_alt_names'), ('i2d_X509', 'openssl::x509::X509Ref::to_der'),
               ('X509_STORE_add_cert', 'openssl::x509::store::X509StoreBuilderRef::add_cert'), ('X509_V_OK', 'openssl::x509::X509VerifyResult::OK'),
               ('ASN1_STRING_get0_data', 'openssl::asn1::Asn1StringRef::as_slice'), ('ASN1_STRING_length', 'openssl::asn1::Asn1StringRef::len'),
               ('ASN1_STRING_to_UTF8', 'openssl::asn1::Asn1StringRef::as_utf8'), ('ASN1_STRING_type', None),
               ('V_ASN1_UTF8STRING', None), ('asn1_string_st', 'openssl::asn1::Asn1String'),
               ('GENERAL_NAMES_free', 'openssl::stack::Stack'), ('ossl_check_const_GENERAL_NAME_sk_type', 'openssl::stack::StackRef'),
               ('sk_GENERAL_NAME_num', 'openssl::stack::StackRef::len'), ('sk_GENERAL_NAME_value', 'openssl::stack::StackRef::get'),
               ('OPENSSL_sk_num', 'openssl::stack::StackRef::len'), ('OPENSSL_sk_value', 'openssl::stack::StackRef::get'),
               ('GEN_DNS', 'openssl::x509::GeneralNameRef::dnsname'), ('GEN_IPADD', 'openssl::x509::GeneralNameRef::ipaddress'),
               ('GENERAL_NAME_st', 'openssl::x509::GeneralName'), ('stack_st_GENERAL_NAME', 'openssl::stack::Stack'),
               ('NID_commonName', 'openssl::nid::Nid::COMMONNAME'), ('NID_subject_alt_name', 'openssl::x509::X509Ref::subject_alt_names'),
               ('x509_st', 'openssl::x509::X509'), ('x509_store_st', 'openssl::x509::store::X509Store'),
               ('X509_name_st', 'openssl::x509::X509Name'), ('X509_name_entry_st', 'openssl::x509::X509NameEntry')):
    ms = [m('openssl', api, 'missing' if api is None else 'exact', 'no ASN1_STRING_type accessor' if api is None else '')]
    if n in ('GEN_DNS', 'GEN_IPADD', 'X509_get_ext_d2i', 'NID_subject_alt_name', 'x509_st', 'i2d_X509'):
        ms.append(m('rustls-webpki', 'webpki::EndEntityCert', 'differs', 'verifies SAN DNS and IP names rather than listing them; DER only'))
    if n in ('NID_commonName', 'X509_NAME_get_index_by_NID', 'X509_NAME_get_entry', 'X509_NAME_ENTRY_get_data', 'X509_get_subject_name'):
        ms.append(m('rustls-webpki', None, 'missing', 'no subject CN, which the CN fallback reads'))
    LIBCRYPTO[n] = spec(CC, None, *ms)
BIO = 'the BIO reads and writes libgit2\'s git_stream, which the remaining C TLS stream (streams/openssl.c) drives'
for n in ('BIO_get_data', 'BIO_get_new_index', 'BIO_meth_free', 'BIO_meth_new', 'BIO_meth_set_create', 'BIO_meth_set_ctrl',
          'BIO_meth_set_destroy', 'BIO_meth_set_gets', 'BIO_meth_set_puts', 'BIO_meth_set_read', 'BIO_meth_set_write', 'BIO_new',
          'BIO_set_data', 'BIO_set_init', 'BIO_CTRL_FLUSH', 'BIO_TYPE_SOURCE_SINK', 'bio_method_st', 'bio_st'):
    LIBCRYPTO[n] = spec(CC, None, m('openssl', 'openssl::ssl::SslStream', 'differs', 'no BIO_meth_* API: SslStream<S: Read + Write> builds its own BIO method; ' + BIO))
for n, api, note in (('CRYPTO_malloc', None, 'internal to the crate'), ('CRYPTO_free', None, 'internal to the crate'),
                     ('OPENSSL_malloc', None, 'internal to the crate'), ('OPENSSL_free', None, 'internal to the crate'),
                     ('ERR_get_error', 'openssl::error::ErrorStack::get', ''), ('ERR_error_string', 'openssl::error::Error', 'its Display adds file:line'),
                     ('ERR_error_string_n', 'openssl::error::Error', 'its Display adds file:line'),
                     ('ERR_lib_error_string', 'openssl::error::Error::library', ''),
                     ('OPENSSL_FILE', None, 'part of OPENSSL_malloc\'s expansion'), ('OPENSSL_LINE', None, 'part of OPENSSL_malloc\'s expansion'),
                     ('ossl_init_settings_st', None, 'initialisation is implicit'), ('ssl_ctx_st', 'openssl::ssl::SslContext', 'libssl\'s object, declared in libcrypto\'s types.h'),
                     ('ssl_st', 'openssl::ssl::Ssl', 'libssl\'s object, declared in libcrypto\'s types.h')):
    LIBCRYPTO[n] = spec(CC, None, m('openssl', api, 'missing' if api is None else ('differs' if note else 'exact'), note))
# libssl -----------------------------------------------------------------------------------
LIBSSL = {}
TLSN = 'streams/openssl.c keeps a live git__ssl_ctx that git_libgit2_opts changes; the crate builds an immutable context'
for n, api, rl in (('OPENSSL_init_ssl', None, None), ('TLS_method', 'openssl::ssl::SslMethod::tls', 'rustls::ClientConfig::builder'),
                   ('SSLv23_method', 'openssl::ssl::SslMethod::tls', 'rustls::ClientConfig::builder'),
                   ('SSL_CTX_new', 'openssl::ssl::SslContextBuilder::new', 'rustls::ClientConfig::builder'),
                   ('SSL_CTX_free', 'openssl::ssl::SslContext', 'rustls::ClientConfig'),
                   ('SSL_CTX_ctrl', 'openssl::ssl::SslContextBuilder::set_mode', None),
                   ('SSL_CTX_set_mode', 'openssl::ssl::SslContextBuilder::set_mode', None),
                   ('SSL_CTRL_MODE', 'openssl::ssl::SslContextBuilder::set_mode', None),
                   ('SSL_MODE_AUTO_RETRY', 'openssl::ssl::SslMode::AUTO_RETRY', None),
                   ('SSL_CTX_set_options', 'openssl::ssl::SslContextBuilder::set_options', 'rustls::ClientConfig::builder_with_protocol_versions'),
                   ('SSL_OP_BIT', 'openssl::ssl::SslOptions', None),
                   ('SSL_OP_NO_COMPRESSION', 'openssl::ssl::SslOptions::NO_COMPRESSION', None),
                   ('SSL_OP_NO_SSLv2', 'openssl::ssl::SslOptions::NO_SSLV2', 'rustls::version::TLS12'),
                   ('SSL_OP_NO_SSLv3', 'openssl::ssl::SslOptions::NO_SSLV3', 'rustls::version::TLS12'),
                   ('SSL_OP_NO_TLSv1', 'openssl::ssl::SslOptions::NO_TLSV1', 'rustls::version::TLS12'),
                   ('SSL_OP_NO_TLSv1_1', 'openssl::ssl::SslOptions::NO_TLSV1_1', 'rustls::version::TLS12'),
                   ('SSL_CTX_set_cipher_list', 'openssl::ssl::SslContextBuilder::set_cipher_list', None),
                   ('SSL_CTX_set_default_verify_paths', 'openssl::ssl::SslContextBuilder::set_default_verify_paths', None),
                   ('SSL_CTX_load_verify_locations', 'openssl::ssl::SslContextBuilder::load_verify_locations', None),
                   ('SSL_CTX_get_cert_store', 'openssl::ssl::SslContextBuilder::cert_store_mut', 'rustls::RootCertStore'),
                   ('SSL_CTX_set_verify', 'openssl::ssl::SslContextBuilder::set_verify', 'rustls::client::danger::ServerCertVerifier'),
                   ('SSL_VERIFY_NONE', 'openssl::ssl::SslVerifyMode::NONE', 'rustls::client::danger::ServerCertVerifier'),
                   ('SSL_new', 'openssl::ssl::Ssl::new', 'rustls::ClientConnection::new'),
                   ('SSL_free', 'openssl::ssl::Ssl', 'rustls::ClientConnection'),
                   ('SSL_set_bio', 'openssl::ssl::SslStream::new', 'rustls::StreamOwned'),
                   ('SSL_ctrl', 'openssl::ssl::SslRef::set_hostname', 'rustls::pki_types::ServerName'),
                   ('SSL_set_tlsext_host_name', 'openssl::ssl::SslRef::set_hostname', 'rustls::pki_types::ServerName'),
                   ('SSL_CTRL_SET_TLSEXT_HOSTNAME', 'openssl::ssl::SslRef::set_hostname', 'rustls::pki_types::ServerName'),
                   ('TLSEXT_NAMETYPE_host_name', 'openssl::ssl::SslRef::set_hostname', 'rustls::pki_types::ServerName'),
                   ('SSL_connect', 'openssl::ssl::SslStream::connect', 'rustls::ClientConnection::complete_io'),
                   ('SSL_read', 'openssl::ssl::SslStream::ssl_read', 'rustls::StreamOwned::read'),
                   ('SSL_write', 'openssl::ssl::SslStream::ssl_write', 'rustls::StreamOwned::write'),
                   ('SSL_shutdown', 'openssl::ssl::SslStream::shutdown', 'rustls::ClientConnection::send_close_notify'),
                   ('SSL_get_error', 'openssl::ssl::Error::code', 'rustls::Error'),
                   ('SSL_get_verify_result', 'openssl::ssl::SslRef::verify_result', None),
                   ('SSL_get1_peer_certificate', 'openssl::ssl::SslRef::peer_certificate', 'rustls::ClientConnection::peer_certificates'),
                   ('SSL_get_peer_certificate', 'openssl::ssl::SslRef::peer_certificate', 'rustls::ClientConnection::peer_certificates'),
                   ('SSL_ERROR_NONE', 'openssl::ssl::ErrorCode', None), ('SSL_ERROR_SSL', 'openssl::ssl::ErrorCode::SSL', 'rustls::Error'),
                   ('SSL_ERROR_SYSCALL', 'openssl::ssl::ErrorCode::SYSCALL', None), ('SSL_ERROR_WANT_ACCEPT', 'openssl::ssl::ErrorCode', None),
                   ('SSL_ERROR_WANT_CONNECT', 'openssl::ssl::ErrorCode', None), ('SSL_ERROR_WANT_READ', 'openssl::ssl::ErrorCode::WANT_READ', None),
                   ('SSL_ERROR_WANT_WRITE', 'openssl::ssl::ErrorCode::WANT_WRITE', None),
                   ('SSL_ERROR_WANT_X509_LOOKUP', 'openssl::ssl::ErrorCode::WANT_X509_LOOKUP', None),
                   ('SSL_ERROR_ZERO_RETURN', 'openssl::ssl::ErrorCode::ZERO_RETURN', None)):
    ms = [m('openssl', api, 'missing' if api is None else 'differs', 'initialisation is implicit' if api is None else TLSN)]
    ms.append(m('rustls', rl, 'differs', 'TLS 1.2+ over a Read + Write; no cipher lists, verify-none-then-report needs a custom verifier, webpki\'s rules')
              if rl else m('rustls', None, 'missing', 'no OpenSSL cipher lists, verify paths, modes or SSL_get_error codes'))
    LIBSSL[n] = spec(CC, None, *ms)
# libpcre2-8 --------------------------------------------------------------------------------
LIBPCRE = {}
PC = 'regexp.c alone uses it; the compiled code and its match data never leave the module'
for n, api, rg in (('pcre2_compile_8', 'pcre2::bytes::RegexBuilder::build', 'regex::bytes::RegexBuilder::build'),
                   ('pcre2_compile', 'pcre2::bytes::RegexBuilder::build', 'regex::bytes::RegexBuilder::build'),
                   ('pcre2_code_free_8', 'pcre2::bytes::Regex', 'regex::bytes::Regex'), ('pcre2_code_free', 'pcre2::bytes::Regex', 'regex::bytes::Regex'),
                   ('pcre2_match_8', 'pcre2::bytes::Regex::captures_read', 'regex::bytes::Regex::captures_read'),
                   ('pcre2_match', 'pcre2::bytes::Regex::captures_read', 'regex::bytes::Regex::captures_read'),
                   ('pcre2_match_data_create_8', 'pcre2::bytes::CaptureLocations', 'regex::bytes::CaptureLocations'),
                   ('pcre2_match_data_create', 'pcre2::bytes::CaptureLocations', 'regex::bytes::CaptureLocations'),
                   ('pcre2_match_data_free_8', 'pcre2::bytes::CaptureLocations', 'regex::bytes::CaptureLocations'),
                   ('pcre2_match_data_free', 'pcre2::bytes::CaptureLocations', 'regex::bytes::CaptureLocations'),
                   ('pcre2_get_ovector_pointer_8', 'pcre2::bytes::CaptureLocations::get', 'regex::bytes::CaptureLocations::get'),
                   ('pcre2_get_ovector_pointer', 'pcre2::bytes::CaptureLocations::get', 'regex::bytes::CaptureLocations::get'),
                   ('pcre2_get_error_message_8', 'pcre2::Error', 'regex::Error'), ('pcre2_get_error_message', 'pcre2::Error', 'regex::Error'),
                   ('PCRE2_CASELESS', 'pcre2::bytes::RegexBuilder::caseless', 'regex::bytes::RegexBuilder::case_insensitive'),
                   ('PCRE2_ERROR_NOMATCH', 'pcre2::bytes::Regex::captures_read', 'regex::bytes::Regex::captures_read'),
                   ('PCRE2_UNSET', 'pcre2::bytes::CaptureLocations::get', 'regex::bytes::CaptureLocations::get'),
                   ('PCRE2_ZERO_TERMINATED', 'pcre2::bytes::RegexBuilder::build', 'regex::bytes::RegexBuilder::build'),
                   ('PCRE2_GLUE', None, None), ('PCRE2_JOIN', None, None), ('PCRE2_SUFFIX', None, None),
                   ('pcre2_real_code_8', 'pcre2::bytes::Regex', 'regex::bytes::Regex'),
                   ('pcre2_real_compile_context_8', None, None), ('pcre2_real_general_context_8', None, None),
                   ('pcre2_real_match_context_8', None, None), ('pcre2_real_match_data_8', 'pcre2::bytes::CaptureLocations', 'regex::bytes::CaptureLocations')):
    if api is None:
        note = ('part of the code-unit-width name mangling of pcre2.h\'s macros' if n.startswith('PCRE2_') else
                'regexp.c passes NULL contexts')
        LIBPCRE[n] = spec('local', None, m('pcre2', None, 'missing', note))
        continue
    ms = [m('pcre2', api, 'differs', 'patterns are &str, so raw non-UTF-8 pattern bytes must be escaped; Display prefixes the error message'
            if n.startswith(('pcre2_compile', 'pcre2_get_error', 'PCRE2_ZERO')) else 'the same engine and options'),
          m('regex', rg, 'differs', 'another dialect: no backreferences, lookaround or \\Q..\\E; `$` only at the end'),
          m('fancy-regex', 'fancy_regex::Regex', 'differs', 'another dialect over regex-automata')]
    LIBPCRE[n] = spec('local', None, *ms)
# libz ------------------------------------------------------------------------------------------
LIBZ = {}
FL = 'flate2'
for n, api, zr in (('deflateInit_', 'flate2::Compress::new', 'zlib_rs::deflate::Deflate::new'), ('deflateInit', 'flate2::Compress::new', 'zlib_rs::deflate::Deflate::new'),
                   ('deflate', 'flate2::Compress::compress', 'zlib_rs::deflate::Deflate::compress'),
                   ('deflateReset', 'flate2::Compress::reset', 'zlib_rs::deflate::Deflate::reset'),
                   ('deflateEnd', 'flate2::Compress', 'zlib_rs::deflate::Deflate'),
                   ('inflateInit_', 'flate2::Decompress::new', 'zlib_rs::inflate::Inflate::new'), ('inflateInit', 'flate2::Decompress::new', 'zlib_rs::inflate::Inflate::new'),
                   ('inflate', 'flate2::Decompress::decompress', 'zlib_rs::inflate::Inflate::decompress'),
                   ('inflateReset', 'flate2::Decompress::reset', 'zlib_rs::inflate::Inflate::reset'),
                   ('inflateEnd', 'flate2::Decompress', 'zlib_rs::inflate::Inflate'),
                   ('z_stream_s', 'flate2::Compress', 'zlib_rs::deflate::Deflate'),
                   ('ZLIB_VERSION', 'flate2::Compress::new', None),
                   ('Z_OK', 'flate2::Status::Ok', None), ('Z_STREAM_END', 'flate2::Status::StreamEnd', None),
                   ('Z_BUF_ERROR', 'flate2::Status::BufError', None), ('Z_STREAM_ERROR', 'flate2::CompressError', None),
                   ('Z_MEM_ERROR', None, None), ('Z_FINISH', 'flate2::FlushCompress::Finish', None),
                   ('Z_NO_FLUSH', 'flate2::FlushCompress::None', None), ('Z_BEST_SPEED', 'flate2::Compression::fast', None),
                   ('Z_DEFAULT_COMPRESSION', 'flate2::Compression::default', None), ('Z_NULL', None, None)):
    note = {'deflateEnd': 'dropped', 'inflateEnd': 'dropped', 'z_stream_s': 'Compress and Decompress hold the stream; no z_stream is exposed',
            'ZLIB_VERSION': 'the version check is the crate\'s', 'Z_STREAM_ERROR': 'a bad level is not rejected (filebuf.c:317-324 checks it)',
            'Z_MEM_ERROR': 'an allocation failure aborts', 'Z_NULL': 'no pointers: slices',
            'inflate': 'with the miniz backend a first Finish call whose output does not fit fails the stream, so zstream.c\'s port passes FlushDecompress::None',
            'Z_FINISH': 'FlushDecompress::Finish on the inflate side'}.get(n, '')
    ms = [m(FL, api, 'missing' if api is None else ('differs' if note else 'exact'), note)]
    if zr: ms.append(m('zlib-rs', zr, 'exact' if n not in ('deflateEnd', 'inflateEnd', 'z_stream_s') else 'differs', note if n in ('deflateEnd', 'inflateEnd', 'z_stream_s') else ''))
    if n.startswith(('deflate', 'inflate')) and not n.endswith(('Init_', 'Init')):
        ms.append(m('miniz_oxide', 'miniz_oxide::' + ('deflate::stream::deflate' if n.startswith('deflate') else 'inflate::stream::inflate'), 'differs', 'its own stream state and flush values'))
    LIBZ[n] = spec('crate', (FL, api if api else 'flate2::Compress'), *ms)
LIBZ['crc32'] = spec('crate', (FL, 'flate2::Crc::update'), m(FL, 'flate2::Crc::update'), m('zlib-rs', 'zlib_rs::crc32::crc32'),
                     m('crc32fast', 'crc32fast::Hasher::update'))
LIBZ['deflateBound'] = spec('crate', (FL, 'flate2::Compress::compress_vec'),
                            m(FL, None, 'missing', 'used only to size one output buffer for a single Z_FINISH call (block.c:174-198); a compress_vec loop until StreamEnd replaces it'),
                            m('zlib-rs', 'zlib_rs::deflate::compress_bound'))

TABLES = {'libc': LIBC, 'libcrypto': LIBCRYPTO, 'libssl': LIBSSL, 'libpcre2-8': LIBPCRE, 'libz': LIBZ}
def lookup(lib, name, kind):
    t = TABLES[lib]
    if lib == 'libc' and name == 'stat' and kind == 'struct': return t['stat|struct']
    return t[name]
missing = []
libs = []
for old in OLD['libraries']:
    lib = old['name']
    cands = candidates(old)
    cnames = {c['crate'] for c in cands} | {'std'}
    ents = []
    for (name, kind, hdr), users in sorted(USED[lib].items(), key=lambda p: (p[0][2], p[0][0], p[0][1])):
        try:
            route, rec, ms = lookup(lib, name, kind)
        except KeyError:
            missing.append((lib, name, kind)); continue
        for x in ms: assert x['crate'] in cnames, (lib, name, x['crate'])
        ents.append({'c_name': name, 'kind': kind, 'header': hdr, 'used_by': users, 'matches': ms,
                     'recommended': {'route': route, 'crate': rec[0] if rec and route in ('std', 'crate') else None,
                                     'rust_api': rec[1] if rec and route in ('std', 'crate') else None},
                     'confirmation': None})
    crates = sorted({e['recommended']['crate'] for e in ents if e['recommended']['route'] == 'crate'})
    left = {e['recommended']['route'] for e in ents if e['recommended']['route'] in ('local', 'crustify-campaign')}
    assert len(left) <= 1, (lib, left)
    unc = left.pop() if left else None
    rec = {'adopt': [{'crate': c, 'fork': None} for c in crates], 'uncovered': unc,
           'uncovered_sys': 'pcre2-sys' if unc == 'local' else None}
    dec = None
    if old['decision'] is not None:          # the user's: libc's {adopt: [], remainder: wrapper}
        assert old['decision'] == {'adopt': [], 'remainder': 'wrapper', 'remainder_sys': None}, old['decision']
        dec = {'adopt': [], 'uncovered': 'crustify-campaign', 'uncovered_sys': None, 'overrides': []}
    takes_cc = any(e['recommended']['route'] == 'crustify-campaign' for e in ents) or (dec and dec['uncovered'] == 'crustify-campaign')
    cc = {'repository': old['wrapper']['repository'], 'pin': old['wrapper']['pin']} if takes_cc else None
    libs.append({'name': lib, 'resolved': old['resolved'], 'source': old['source'], 'linked_from': old['linked_from'],
                 'candidates': cands, 'c_entities': ents, 'recommendation': rec, 'decision': dec, 'crustify_campaign': cc})
if missing:
    raise SystemExit(f'entities with no match record: {missing}')
COMMENT = ("The libgit2 port campaign's out-of-tree libraries, in the schema of Crustify 043a8ae, as planner run 11 "
           "brought them to it from run 10's assessments: each library's candidate crates, each with one quality_assessment "
           "composed from run 10's reading of it against the Adopted crates criteria (coverage, shared state, soundness, "
           "advisories, maintenance) and its verdict; every entity the campaign uses of the library (c_entities), from the "
           "external records of schedule.json's modules, named by the public header that declares it, with what each "
           "candidate and std offers for it and the plan's recommended route, by the area run 10 assessed it in; and the "
           "recommendation that follows. libc's decision is the user's, carried unchanged: no crate adopted, what std does "
           "not serve through its crustify campaign. The other decisions are the user's to make from the recommendations. "
           "libz-rs-sys, assessed by run 10, is no candidate: it has no safe API. Planner run 11's composition is "
           "crustify/artifacts/planner/run-11/compose/ (deps11.py).")
doc = {'_comment': COMMENT, 'version': OLD['version'], 'libraries': libs}
open(R + 'crustify/artifacts/dependencies.json', 'w').write(json.dumps(doc, indent=1, ensure_ascii=False) + '\n')
for l in libs:
    c = collections.Counter(e['recommended']['route'] for e in l['c_entities'])
    print(l['name'], len(l['c_entities']), dict(c), 'adopt', [a['crate'] for a in l['recommendation']['adopt']],
          'uncovered', l['recommendation']['uncovered'], 'decision', l['decision'] is not None, 'cc', l['crustify_campaign'])
