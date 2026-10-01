# Deterministic local stream-json fixture. It never contacts a model provider.
import json
import os
from pathlib import Path
import sys
import time
import urllib.parse
import urllib.request

root = Path(os.environ["WKS_LOCAL_SPAWN_FIXTURE"])
args = sys.argv[1:]
session = args[args.index("--session-id") + 1]
logfile = root / ("received-" + session + ".jsonl")


def record(value):
    with logfile.open("a", encoding="utf-8") as out:
        out.write(json.dumps(value) + "\n")
        out.flush()


def emit(value):
    print(json.dumps(value), flush=True)


config_path = args[args.index("--mcp-config") + 1]
server = json.loads(Path(config_path).read_text())["mcpServers"]["workspacer"]
endpoint = server["url"]
assert urllib.parse.urlparse(endpoint).hostname in ("127.0.0.1", "::1")
assert "HUB_TOKEN" not in os.environ
assert "WKS_MCP_TOKEN" not in os.environ
assert server["headers"]["Authorization"].startswith("Bearer ")
headers = dict(server["headers"])
headers.update({"Content-Type": "application/json", "Accept": "application/json, text/event-stream"})
sequence = 0


def rpc(method, params=None, notification=False):
    global sequence
    sequence += 1
    body = {"jsonrpc": "2.0", "method": method}
    if not notification:
        body["id"] = sequence
    if params is not None:
        body["params"] = params
    req = urllib.request.Request(endpoint, data=json.dumps(body).encode(), headers=headers)
    with urllib.request.urlopen(req, timeout=8) as response:
        if response.headers.get("Mcp-Session-Id"):
            headers["Mcp-Session-Id"] = response.headers["Mcp-Session-Id"]
        if notification:
            response.read()
            return None
        if "text/event-stream" in response.headers.get("Content-Type", ""):
            while True:
                line = response.readline()
                if not line:
                    raise RuntimeError("MCP stream closed before reply")
                if line.startswith(b"data:"):
                    value = json.loads(line[5:].strip())
                    if value.get("id") == sequence:
                        break
        else:
            value = json.load(response)
    assert "error" not in value, value.get("error")
    return value["result"]


initialized = rpc("initialize", {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "local-fixture", "version": "1"}})
headers["MCP-Protocol-Version"] = initialized["protocolVersion"]
rpc("notifications/initialized", notification=True)
config = rpc("tools/call", {"name": "get_config", "arguments": {}})
assert not config.get("isError", False)
record({"ready": True, "pid": os.getpid(), "session": session, "scopedMcpCall": True, "hostCredentialsAbsent": True})
instructions = args[args.index("--append-system-prompt") + 1]
record({"launchInstructions": instructions})
emit({"type": "system", "subtype": "init", "session_id": session, "model": "claude-sonnet-4-6", "tools": [], "mcp_servers": [{"name": "workspacer", "status": "connected"}]})

for line in sys.stdin:
    frame = json.loads(line)
    if frame.get("type") == "control_response" and frame.get("response", {}).get("request_id") == "numeric-question-fixture":
        reply = frame["response"]["response"]
        record({"numericAnswers": reply["updatedInput"]["answers"]})
        emit({"type": "result", "subtype": "success", "is_error": False, "total_cost_usd": 0, "usage": {"input_tokens": 1, "output_tokens": 1}})
        continue
    if frame.get("type") == "control_request":
        emit({"type": "control_response", "response": {"subtype": "success", "request_id": frame["request_id"], "response": {}}})
        continue
    if frame.get("type") != "user":
        continue
    content = frame.get("message", {}).get("content", [])
    prompt = "\n".join(block.get("text", "") for block in content if block.get("type") == "text")
    record({"message": prompt})
    if "spawn-ordinary-child-fixture" in prompt:
        spawned = rpc("tools/call", {"name": "spawn_agent", "arguments": {
            "cwd": str(root / "project"), "provider": "claude", "transport": "stream",
            "profileId": "isolated", "trackTask": False,
            "message": "finish-child-fixture", "label": "Fixture child"
        }})
        assert not spawned.get("isError", False), spawned
        receipt = json.loads(spawned["content"][0]["text"])
        record({"spawnedChild": receipt})
        # Expose the same namespaced tool pair that a provider would publish.
        emit({"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "ordinary-child-spawn", "name": "mcp__workspacer__spawn_agent", "input": {"trackTask": False, "label": "Fixture child"}}
        ]}})
        emit({"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "ordinary-child-spawn", "content": json.dumps(receipt)}
        ]}})
    if "ask-numeric-fixture" in prompt:
        emit({"type": "control_request", "request_id": "numeric-question-fixture", "request": {
            "subtype": "can_use_tool", "tool_name": "AskUserQuestion", "input": {"questions": [
                {"question": "Literal first", "options": [{"label": "One"}, {"label": "Two"}, {"label": "Three"}]},
                {"question": "Literal second", "options": [{"label": "One"}, {"label": "Two"}, {"label": "Three"}]},
                {"question": "Option control", "options": [{"label": "Red"}, {"label": "Blue"}]}
            ]}}})
        continue
    if "finish-child-fixture" in prompt:
        release = root / ("release-" + session)
        deadline = time.monotonic() + 15
        while not release.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        assert release.exists(), "fixture release not received"
        progress = rpc("tools/call", {"name": "report_progress", "arguments": {"note": "fixture milestone"}})
        assert not progress.get("isError", False)
        record({"progressMcpCall": True})
        reply = "child-finished-fixture"
    else:
        reply = "parent-fixture-received"
    emit({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": reply}}})
    emit({"type": "result", "subtype": "success", "is_error": False, "total_cost_usd": 0, "usage": {"input_tokens": 1, "output_tokens": 1}, "modelUsage": {"claude-sonnet-4-6": {"contextWindow": 200000}}})
