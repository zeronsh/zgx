"""Local servers for the web view probes' certificate and credentials cases.

`https()` serves pages over TLS with a self-signed certificate for
127.0.0.1 (fixtures/selfsigned-127.0.0.1.pem, test-only), so the only thing
wrong is its issuer: a page that loads posts `{"tls": "<path>"}` to the app.
`basic_auth()` serves over HTTP behind Basic credentials for realm `zgx`
(user `zgx`, password `secret`): with them a page posts `{"auth": "ok"}`,
without them the refusal's page posts `{"auth": "refused"}`. Each runs on a
thread of its own and returns its base URL; each request's path is kept in
the server's `paths`.
"""
import base64
import http.server
import pathlib
import ssl
import threading

FIXTURE = pathlib.Path(__file__).resolve().parent / "fixtures" / "selfsigned-127.0.0.1.pem"
CREDENTIALS = "Basic " + base64.b64encode(b"zgx:secret").decode()


def page(message):
    return (f"<!doctype html><title>trust</title><body><script>"
            f"window.zgx && window.zgx.postMessage({message});</script>").encode()


def serve(handler, context=None):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    server.paths = []
    if context is not None:
        server.socket = context.wrap_socket(server.socket, server_side=True)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    scheme = "https" if context is not None else "http"
    return server, f"{scheme}://127.0.0.1:{server.server_address[1]}"


class Quiet(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send(self, status, body, headers=()):
        self.send_response(status)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        for name, value in headers:
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(body)


def https():
    class Handler(Quiet):
        def do_GET(self):
            self.server.paths.append(self.path)
            self.send(200, page('{"tls": "%s"}' % self.path))

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(FIXTURE)
    return serve(Handler, context)


def basic_auth():
    class Handler(Quiet):
        def do_GET(self):
            self.server.paths.append(self.path)
            if self.headers.get("Authorization") == CREDENTIALS:
                self.send(200, page('{"auth": "ok"}'))
            else:
                self.send(401, page('{"auth": "refused"}'), [("WWW-Authenticate", 'Basic realm="zgx"')])

    return serve(Handler)
