# Local fixture: receives real managed launch/input, never contacts a provider.
import json
import os
from pathlib import Path
import sys

root = Path(os.environ["WKS_NO_MCP_FIXTURE"])
args = sys.argv[1:]
if "--version" in args:
    print("fixture-claude 1.0")
    sys.exit(0)
session = args[args.index("--session-id") + 1]
assert "HUB_TOKEN" not in os.environ
assert "WKS_MCP_TOKEN" not in os.environ
servers = {}
if "--mcp-config" in args:
    servers = json.loads(Path(args[args.index("--mcp-config") + 1]).read_text())["mcpServers"]
    assert set(servers) == {"custom"}
assert "workspacer" not in servers
assert not any("with access to the local workspacer MCP facade" in arg for arg in args)
assert any("profile-without-facade" in arg for arg in args)

def record(value):
    with (root / ("received-" + session + ".jsonl")).open("a", encoding="utf-8") as out:
        out.write(json.dumps(value) + "\n")

def emit(value):
    print(json.dumps(value), flush=True)

record({"ready": True, "pid": os.getpid(), "session": session, "serverNames": list(servers), "args": args})
emit({"type": "system", "subtype": "init", "session_id": session, "model": "claude-sonnet-4-6", "tools": [], "mcp_servers": []})
for line in sys.stdin:
    frame = json.loads(line)
    if frame.get("type") == "control_request":
        emit({"type": "control_response", "response": {"subtype": "success", "request_id": frame["request_id"], "response": {}}})
        continue
    if frame.get("type") != "user":
        continue
    content = frame.get("message", {}).get("content", [])
    prompt = content if isinstance(content, str) else "\n".join(block.get("text", "") for block in content if block.get("type") == "text")
    record({"message": prompt})
    reply = 'no-mcp-finished\n```wks-result\n{"ok":true}\n```'
    emit({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": reply}}})
    emit({"type": "result", "subtype": "success", "is_error": False, "total_cost_usd": 0, "usage": {"input_tokens": 1, "output_tokens": 1}, "modelUsage": {"claude-sonnet-4-6": {"contextWindow": 200000}}})
