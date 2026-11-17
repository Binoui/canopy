#!/usr/bin/env python3
"""Fixture site for verify-canopy runs. Any host, any path.

/p/<Title_with_underscores> answers a small article page titled from the path.
Listens on [::] so *.localhost (which resolves to ::1) and 127.0.0.1 both work.
Usage: server.py PORT (0 picks a free port). Prints the bound port on its first line.
"""
import html
import socket
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        if self.path.startswith('/favicon'):
            self.send_response(404)
            self.end_headers()
            return
        title = unquote(self.path.rsplit('/', 1)[-1]).replace('_', ' ') or 'Home'
        host = (self.headers.get('Host') or 'site').split(':')[0]
        links = ''.join(
            f'<li><a href="/p/{html.escape(title.replace(" ", "_"))}_{n}">A linked page number {n}</a></li>'
            for n in range(1, 9)
        )
        body = f"""<!doctype html><meta charset="utf-8"><title>{html.escape(title)}</title>
<style>body{{font:16px/1.6 system-ui,sans-serif;margin:0;background:#f6f4ee;color:#23221f}}
header{{padding:14px 28px;border-bottom:1px solid #dedbd2;background:#fff;font-weight:600}}
main{{max-width:720px;margin:36px auto;padding:0 24px}}h1{{font:600 34px/1.2 Georgia,serif}}</style>
<header>{html.escape(host)}</header><main><h1>{html.escape(title)}</h1>
<p>A fixture page. Its only job is to give the chrome above it something to sit on.
The tree strip, the tabs and the zoomed tree are what this run looks at.</p><ul>{links}</ul>
<p>{'Filler paragraph for scrolling. ' * 60}</p></main>"""
        data = body.encode()
        self.send_response(200)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)


class DualStack(ThreadingHTTPServer):
    address_family = socket.AF_INET6

    def server_bind(self):
        self.socket.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
        super().server_bind()


if __name__ == '__main__':
    server = DualStack(('::', int(sys.argv[1])), Handler)
    print(server.server_address[1], flush=True)
    server.serve_forever()
