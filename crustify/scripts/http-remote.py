#!/usr/bin/env python3
"""A smart-HTTP Git remote on localhost, for the C that drives libgit2's HTTP client.

    http-remote.py <root> <port file> [--auth USER:PASS] [--log FILE]

Serves the repositories under <root> through `git http-backend`, upload-pack and
receive-pack both, over HTTP/1.1 with persistent connections, so that libgit2's HTTP
client (src/libgit2/transports/httpclient.c over httpparser.c and llhttp) reads several
responses on one connection. It listens on 127.0.0.1 at a port the system picks, written to
<port file> once it accepts connections, and runs until killed.

The first component of a request's path picks how the response's body is framed, the rest
naming the repository under <root>, so that one remote serves every framing llhttp parses:

- `/length/<repo>`: a `Content-Length` body;
- `/chunked/<repo>`: `Transfer-Encoding: chunked`, in chunks of 1, 7, 4096 and 65536 bytes
  in turn, so that a chunk's size line, its data and its end fall across reads;
- `/chunked-ext/<repo>`: chunked as above, every size line carrying chunk extensions
  (`;a=b;flag;q="x y"`), which llhttp's chunk-extension callbacks are called for;
- `/close/<repo>`: no length and no chunking, the body ending with the connection
  (`Connection: close`).
- `/malformed/<repo>`: a response whose header block llhttp refuses, a space in a header's
  name, so that the client reports the parser's error and its reason.

With `--auth`, every request without that user's Basic credentials is answered 401 with a
`WWW-Authenticate: Basic` challenge, a body and the connection kept, before the backend
runs. A path that names no repository is the backend's 404. Requests are logged to
`--log`, one line each, method, path and status, for the caller to compare.

Crustify's campaign check of a module whose exports libgit2's HTTP client reaches, at its
switch from C (crustify/evolution/module-switch-procedure): run it with `python3 -I`.
"""

import base64
import os
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CHUNKS = (1, 7, 4096, 65536)
FRAMINGS = ("length", "chunked", "chunked-ext", "close", "malformed")


def main(argv):
    args = list(argv)
    auth = log = None
    while "--auth" in args or "--log" in args:
        for flag in ("--auth", "--log"):
            if flag in args:
                i = args.index(flag)
                value = args[i + 1]
                del args[i : i + 2]
                if flag == "--auth":
                    auth = value
                else:
                    log = value
    if len(args) != 2:
        sys.exit(__doc__.split("\n\n")[1])
    root, port_file = os.path.realpath(args[0]), args[1]
    log_lock = threading.Lock()
    log_file = open(log, "a", buffering=1) if log else None
    expected = "Basic " + base64.b64encode(auth.encode()).decode() if auth else None

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, fmt, *a):
            pass

        def record(self, status):
            if log_file:
                with log_lock:
                    log_file.write(f"{self.command} {self.path} {status}\n")

        def read_body(self):
            if self.headers.get("Transfer-Encoding", "").lower() == "chunked":
                body = bytearray()
                while True:
                    size = int(self.rfile.readline().split(b";")[0].strip(), 16)
                    if size == 0:
                        while self.rfile.readline() not in (b"\r\n", b"\n", b""):
                            pass
                        return bytes(body)
                    body += self.rfile.read(size)
                    self.rfile.readline()
            return self.rfile.read(int(self.headers.get("Content-Length") or 0))

        def send(self, status, headers, body, framing):
            self.send_response(status)
            for k, v in headers:
                self.send_header(k, v)
            if framing == "malformed":
                self.wfile.write(b"HTTP/1.1 200 OK\r\nBad Header: x\r\nContent-Length: 0\r\n\r\n")
                self.wfile.flush()
                self.close_connection = True
                return
            if framing == "length":
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            elif framing == "close":
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(body)
                self.close_connection = True
            else:
                self.send_header("Transfer-Encoding", "chunked")
                self.end_headers()
                ext = ';a=b;flag;q="x y"' if framing == "chunked-ext" else ""
                pos, i = 0, 0
                while pos < len(body):
                    n = CHUNKS[i % len(CHUNKS)]
                    piece = body[pos : pos + n]
                    self.wfile.write(b"%x%s\r\n%s\r\n" % (len(piece), ext.encode(), piece))
                    pos, i = pos + n, i + 1
                self.wfile.write(b"0%s\r\n\r\n" % ext.encode())
            self.wfile.flush()

        def serve(self):
            body = self.read_body() if self.command == "POST" else b""
            path, _, query = self.path.partition("?")
            framing, _, rest = path.lstrip("/").partition("/")
            if framing not in FRAMINGS:
                framing, rest = "length", path.lstrip("/")
            if expected and self.headers.get("Authorization") != expected:
                self.record(401)
                self.send(
                    401,
                    [("WWW-Authenticate", 'Basic realm="crustify"'),
                     ("Content-Type", "text/plain")],
                    b"authentication required\n",
                    framing,
                )
                return
            env = {
                "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
                "GIT_PROJECT_ROOT": root,
                "GIT_HTTP_EXPORT_ALL": "1",
                "PATH_INFO": "/" + rest,
                "QUERY_STRING": query,
                "REQUEST_METHOD": self.command,
                "CONTENT_TYPE": self.headers.get("Content-Type", ""),
                "CONTENT_LENGTH": str(len(body)),
                "REMOTE_USER": auth.split(":")[0] if auth else "anonymous",
                "REMOTE_ADDR": "127.0.0.1",
                "SERVER_PROTOCOL": "HTTP/1.1",
                "GIT_CONFIG_COUNT": "1",
                "GIT_CONFIG_KEY_0": "http.receivepack",
                "GIT_CONFIG_VALUE_0": "true",
            }
            if self.headers.get("Git-Protocol"):
                env["GIT_PROTOCOL"] = self.headers["Git-Protocol"]
            out = subprocess.run(
                ["git", "http-backend"], input=body, env=env, capture_output=True
            ).stdout
            head, _, payload = out.partition(b"\r\n\r\n")
            if not _:
                head, _, payload = out.partition(b"\n\n")
            status, headers = 200, []
            for line in head.decode("latin-1").splitlines():
                k, _, v = line.partition(":")
                if k.lower() == "status":
                    status = int(v.split()[0])
                elif k:
                    headers.append((k, v.strip()))
            self.record(status)
            self.send(status, headers, payload, framing)

        do_GET = do_POST = serve

    class Server(ThreadingHTTPServer):
        daemon_threads = True

        def handle_error(self, request, client_address):
            # A client that gives up on a request, as libgit2 does on an error, resets or
            # closes its connection: logged, not printed.
            exc = sys.exc_info()[1]
            if log_file:
                with log_lock:
                    log_file.write(f"error {type(exc).__name__}\n")

    server = Server(("127.0.0.1", 0), Handler)
    tmp = port_file + ".tmp"
    with open(tmp, "w") as f:
        f.write(f"{server.server_address[1]}\n")
    os.replace(tmp, port_file)
    server.serve_forever()


if __name__ == "__main__":
    main(sys.argv[1:])
