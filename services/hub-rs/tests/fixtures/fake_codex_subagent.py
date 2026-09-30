# Inert local Codex app-server fixture. No model or external network access.
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import struct
import sys
import threading
import time
import urllib.parse

root = Path(os.environ["WKS_LOCAL_SPAWN_FIXTURE"])
if "--version" in sys.argv:
    print("codex-fixture 1.0")
    sys.exit(0)
endpoint = urllib.parse.urlparse(sys.argv[sys.argv.index("--listen") + 1])
assert endpoint.hostname == "127.0.0.1"
assert "HUB_TOKEN" not in os.environ
assert "WKS_MCP_TOKEN" not in os.environ
parent = "fixture-codex-parent"
child = "fixture-codex-child"
with (root / "codex-processes.jsonl").open("a") as output:
    output.write(json.dumps({"pid": os.getpid()}) + "\n")


class Server(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def do_GET(self):
        if self.path == "/readyz":
            self.send_response(200)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        key = self.headers["Sec-WebSocket-Key"]
        digest = hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()
        self.send_response(101)
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", base64.b64encode(digest).decode())
        self.end_headers()
        writing = threading.Lock()
        stopped = threading.Event()

        def send(value):
            body = json.dumps(value).encode()
            length = bytes([len(body)]) if len(body) < 126 else b"\x7e" + struct.pack("!H", len(body))
            with writing:
                self.connection.sendall(b"\x81" + length + body)

        def exact(count):
            data = b""
            while len(data) < count:
                chunk = self.rfile.read(count - len(data))
                if not chunk:
                    raise EOFError()
                data += chunk
            return data

        def expose():
            deadline = time.monotonic() + 30
            while not stopped.is_set() and time.monotonic() < deadline:
                if (root / "expose-codex-child").exists():
                    try:
                        send({"method": "thread/started", "params": {"thread": {"id": child, "parentThreadId": parent}}})
                    except OSError:
                        pass
                    return
                stopped.wait(0.01)

        exposing = None
        try:
            while True:
                first, second = exact(2)
                opcode = first & 15
                length = second & 127
                if length == 126:
                    length = struct.unpack("!H", exact(2))[0]
                elif length == 127:
                    length = struct.unpack("!Q", exact(8))[0]
                assert length <= 1024 * 1024
                mask = exact(4) if second & 128 else b""
                payload = exact(length)
                if mask:
                    payload = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
                if opcode == 8:
                    return
                if opcode != 1:
                    continue
                request = json.loads(payload)
                method = request.get("method")
                if "id" not in request:
                    continue
                result = {"thread": {"id": parent}} if method in ("thread/start", "thread/resume") else {}
                send({"jsonrpc": "2.0", "id": request["id"], "result": result})
                if method == "turn/start":
                    send({"method": "turn/started", "params": {"threadId": parent}})
                    send({"method": "turn/completed", "params": {"threadId": parent, "turn": {"status": "completed"}}})
                    if exposing is None:
                        exposing = threading.Thread(target=expose, daemon=True)
                        exposing.start()
        except (EOFError, OSError):
            pass
        finally:
            stopped.set()
            if exposing is not None:
                exposing.join(timeout=1)
            threading.Thread(target=self.server.shutdown, daemon=True).start()


server = http.server.ThreadingHTTPServer(("127.0.0.1", endpoint.port), Server)
parent_pid = os.getppid()
finished = threading.Event()


def parent_watch():
    while not finished.wait(0.1):
        if os.getppid() != parent_pid:
            server.shutdown()
            return


threading.Thread(target=parent_watch, daemon=True).start()
try:
    server.serve_forever()
finally:
    finished.set()
    server.server_close()
