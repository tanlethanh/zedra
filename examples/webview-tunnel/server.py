#!/usr/bin/env python3
"""Multi-port localhost web app for exercising the Zedra in-app webview tunnel.

Runs three loopback servers so a single page proves the tunnel forwards real
TCP streams across ports, not just simple HTTP:

  http://localhost:5173   frontend page (the URL you open in Zedra)
  http://localhost:5174   backend API: JSON (/api/info) + SSE stream (/api/stream)
  ws://localhost:5175      WebSocket echo

The page also probes the optional JS bridge (window.zedra /
window.webkit.messageHandlers.zedra) and exposes window.zedraSetStatus, so it
lights up extra when opened by a webview configured with messaging/eval.

Pure standard library. No dependencies. Python 3.8+.
"""

import base64
import hashlib
import json
import socket
import struct
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

FRONTEND_PORT = 5173
BACKEND_PORT = 5174
WS_PORT = 5175

PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<title>Zedra Tunnel Test</title>
<style>
  :root { color-scheme: dark; }
  * { box-sizing: border-box; }
  body { margin: 0; font: 16px/1.5 -apple-system, system-ui, sans-serif;
         background: #0e0c0c; color: #eaeaea; padding: 20px; }
  h1 { font-size: 20px; margin: 0 0 4px; }
  .sub { color: #888; font-size: 13px; margin-bottom: 20px; }
  .card { background: #161616; border: 1px solid #2c2c2c; border-radius: 12px;
          padding: 16px; margin-bottom: 14px; }
  .card h2 { font-size: 14px; margin: 0 0 10px; color: #98c379; letter-spacing: .3px; }
  .row { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
  button { font: 600 15px system-ui; padding: 10px 14px; border-radius: 10px;
           border: 1px solid #2c2c2c; background: #222; color: #eaeaea; }
  button:active { background: #333; }
  pre { background: #0a0a0a; border-radius: 8px; padding: 10px; margin: 8px 0 0;
        overflow: auto; font-size: 12px; color: #cfcfcf; max-height: 160px; }
  .pill { display: inline-block; padding: 2px 8px; border-radius: 999px;
          font-size: 12px; font-weight: 600; }
  .ok { background: #1f3d1f; color: #98c379; }
  .bad { background: #3d1f1f; color: #e06c75; }
  .wait { background: #3a341c; color: #e5c07b; }
  a { color: #61afef; }
  input { font: 15px system-ui; padding: 9px 11px; border-radius: 9px;
          border: 1px solid #2c2c2c; background: #0a0a0a; color: #eaeaea; flex: 1; min-width: 120px; }
</style>
</head>
<body>
  <h1>Zedra Tunnel Test</h1>
  <div class="sub" id="origin"></div>

  <div class="card">
    <h2>STATUS</h2>
    <div id="status">Running checks…</div>
  </div>

  <div class="card">
    <h2>BACKEND API (port 5174)</h2>
    <div class="row"><span class="pill wait" id="api-pill">pending</span>
      <button onclick="callApi()">Call /api/info</button></div>
    <pre id="api-out">—</pre>
  </div>

  <div class="card">
    <h2>SERVER-SENT EVENTS (port 5174)</h2>
    <div class="row"><span class="pill wait" id="sse-pill">pending</span>
      <span id="sse-count">0 ticks</span></div>
    <pre id="sse-out">—</pre>
  </div>

  <div class="card">
    <h2>WEBSOCKET ECHO (port 5175)</h2>
    <div class="row"><span class="pill wait" id="ws-pill">connecting</span></div>
    <div class="row" style="margin-top:8px">
      <input id="ws-input" value="hello from webview" />
      <button onclick="wsSend()">Send</button>
    </div>
    <pre id="ws-out">—</pre>
  </div>

  <div class="card">
    <h2>NATIVE JS BRIDGE (optional)</h2>
    <div class="row"><span class="pill wait" id="bridge-pill">probing</span>
      <button onclick="post('button tapped')">Post to Rust</button></div>
    <div class="sub" style="margin-top:8px" id="bridge-note"></div>
  </div>

  <div class="card">
    <h2>NAVIGATION</h2>
    <div class="row">
      <a href="/page2">Internal link (allowed)</a>
      <a href="https://example.com/blocked">External https (blocked if intercepting)</a>
    </div>
  </div>

<script>
const $ = (id) => document.getElementById(id);
$("origin").textContent = location.href;
function setPill(id, cls, text){ const p=$(id); p.className="pill "+cls; p.textContent=text; }

// --- Backend JSON API ---
async function callApi(){
  setPill("api-pill","wait","calling…");
  try{
    const r = await fetch("http://localhost:5174/api/info");
    const j = await r.json();
    $("api-out").textContent = JSON.stringify(j, null, 2);
    setPill("api-pill","ok","ok");
  }catch(e){ $("api-out").textContent = String(e); setPill("api-pill","bad","failed"); }
}

// --- Server-sent events ---
let sseN = 0;
try{
  const es = new EventSource("http://localhost:5174/api/stream");
  es.onopen = () => setPill("sse-pill","ok","streaming");
  es.onerror = () => setPill("sse-pill","bad","error");
  es.onmessage = (ev) => {
    sseN++; $("sse-count").textContent = sseN + " ticks";
    $("sse-out").textContent = ev.data + "\\n" + $("sse-out").textContent.split("\\n").slice(0,5).join("\\n");
  };
}catch(e){ setPill("sse-pill","bad","unsupported"); }

// --- WebSocket echo ---
let ws;
try{
  ws = new WebSocket("ws://localhost:5175");
  ws.onopen = () => { setPill("ws-pill","ok","connected"); ws.send("ping at "+Date.now()); };
  ws.onclose = () => setPill("ws-pill","bad","closed");
  ws.onerror = () => setPill("ws-pill","bad","error");
  ws.onmessage = (ev) => { $("ws-out").textContent = "echo: " + ev.data + "\\n" + $("ws-out").textContent; };
}catch(e){ setPill("ws-pill","bad","unsupported"); }
function wsSend(){ if(ws && ws.readyState===1) ws.send($("ws-input").value); }

// --- Optional native JS bridge (works when the webview wires messaging) ---
function bridge(){
  if (window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.zedra)
    return (m) => window.webkit.messageHandlers.zedra.postMessage(m);
  if (window.zedra && window.zedra.postMessage)
    return (m) => window.zedra.postMessage(m);
  return null;
}
function post(m){ const b = bridge(); if (b) b(m); }
window.zedraSetStatus = (s) => { $("status").textContent = s; };
(function(){
  const b = bridge();
  if (b){ setPill("bridge-pill","ok","present"); $("bridge-note").textContent =
      "Bridge detected. Posting messages to Rust; window.zedraSetStatus can be called from eval_js.";
    post("page loaded");
  } else { setPill("bridge-pill","wait","absent");
    $("bridge-note").textContent =
      "No native bridge (expected for the plain tunnel). Open via a webview wired with on_message to light this up.";
  }
})();

// Overall status
setTimeout(()=>{ if($("status").textContent==="Running checks…")
  $("status").textContent = "Page loaded over the tunnel. Check the cards below."; }, 300);
</script>
</body>
</html>"""

PAGE2 = """<!doctype html><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<body style="font:16px system-ui;background:#0e0c0c;color:#eaeaea;padding:24px">
<h1>Page 2</h1><p>Internal navigation worked.</p>
<p><a style="color:#61afef" href="/">&larr; Back</a></p>"""


class FrontendHandler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        body = PAGE2 if self.path.startswith("/page2") else PAGE
        data = body.encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)


class BackendHandler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _cors(self):
        self.send_header("Access-Control-Allow-Origin", "*")

    def do_GET(self):
        if self.path.startswith("/api/info"):
            payload = json.dumps({
                "service": "zedra-tunnel-test-backend",
                "port": BACKEND_PORT,
                "time": time.strftime("%H:%M:%S"),
                "message": "Reached a second host localhost port through the tunnel.",
            }).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self._cors()
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return
        if self.path.startswith("/api/stream"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Connection", "keep-alive")
            self._cors()
            self.end_headers()
            try:
                n = 0
                while True:
                    n += 1
                    msg = f"data: tick {n} @ {time.strftime('%H:%M:%S')}\n\n"
                    self.wfile.write(msg.encode("utf-8"))
                    self.wfile.flush()
                    time.sleep(1)
            except (BrokenPipeError, ConnectionResetError):
                return
        self.send_response(404)
        self._cors()
        self.end_headers()


# --- Minimal RFC 6455 WebSocket echo (text frames + ping/close) ---
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def ws_accept(key: str) -> str:
    digest = hashlib.sha1((key + WS_GUID).encode()).digest()
    return base64.b64encode(digest).decode()


def ws_handshake(conn) -> bool:
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = conn.recv(1024)
        if not chunk:
            return False
        data += chunk
    key = None
    for line in data.decode("latin1").split("\r\n"):
        if line.lower().startswith("sec-websocket-key:"):
            key = line.split(":", 1)[1].strip()
    if not key:
        return False
    resp = (
        "HTTP/1.1 101 Switching Protocols\r\n"
        "Upgrade: websocket\r\nConnection: Upgrade\r\n"
        f"Sec-WebSocket-Accept: {ws_accept(key)}\r\n\r\n"
    )
    conn.sendall(resp.encode())
    return True


def ws_send_text(conn, text: str):
    payload = text.encode("utf-8")
    header = bytearray([0x81])  # FIN + text opcode
    n = len(payload)
    if n < 126:
        header.append(n)
    elif n < 65536:
        header.append(126)
        header += struct.pack(">H", n)
    else:
        header.append(127)
        header += struct.pack(">Q", n)
    conn.sendall(bytes(header) + payload)


def ws_read_frame(conn):
    hdr = conn.recv(2)
    if len(hdr) < 2:
        return None, None
    opcode = hdr[0] & 0x0F
    masked = hdr[1] & 0x80
    length = hdr[1] & 0x7F
    if length == 126:
        length = struct.unpack(">H", conn.recv(2))[0]
    elif length == 127:
        length = struct.unpack(">Q", conn.recv(8))[0]
    mask = conn.recv(4) if masked else b""
    data = b""
    while len(data) < length:
        chunk = conn.recv(length - len(data))
        if not chunk:
            break
        data += chunk
    if masked:
        data = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
    return opcode, data


def ws_client(conn, addr):
    try:
        if not ws_handshake(conn):
            return
        ws_send_text(conn, "connected to host WebSocket echo")
        while True:
            opcode, data = ws_read_frame(conn)
            if opcode is None or opcode == 0x8:  # closed
                break
            if opcode == 0x9:  # ping -> pong
                conn.sendall(bytes([0x8A, 0]))
                continue
            if opcode == 0x1:  # text
                ws_send_text(conn, data.decode("utf-8", "replace"))
    except OSError:
        pass
    finally:
        conn.close()


def run_ws_server():
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", WS_PORT))
    srv.listen(16)
    while True:
        conn, addr = srv.accept()
        threading.Thread(target=ws_client, args=(conn, addr), daemon=True).start()


def main():
    frontend = ThreadingHTTPServer(("127.0.0.1", FRONTEND_PORT), FrontendHandler)
    backend = ThreadingHTTPServer(("127.0.0.1", BACKEND_PORT), BackendHandler)
    threading.Thread(target=backend.serve_forever, daemon=True).start()
    threading.Thread(target=run_ws_server, daemon=True).start()
    print(f"frontend : http://localhost:{FRONTEND_PORT}")
    print(f"backend  : http://localhost:{BACKEND_PORT}/api/info  (+ /api/stream SSE)")
    print(f"websocket: ws://localhost:{WS_PORT}")
    print()
    print(f"Open  http://localhost:{FRONTEND_PORT}  from a Zedra terminal and tap it.")
    print("Ctrl-C to stop.")
    try:
        frontend.serve_forever()
    except KeyboardInterrupt:
        print("\nstopped")
        sys.exit(0)


if __name__ == "__main__":
    main()
