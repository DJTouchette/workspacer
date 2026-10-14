# Deterministic stream-json agent for hub message-queue tests. Never contacts a
# model provider. Every user turn is recorded; a prompt containing
# "hold:<name>" keeps its turn open until the test creates release-<name>, and
# a prompt containing "ask-question" parks an AskUserQuestion until answered.
import json
import os
from pathlib import Path
import sys
import time

root = Path(os.environ["WKS_QUEUE_FIXTURE"])
args = sys.argv[1:]
session = args[args.index("--session-id") + 1]
logfile = root / ("received-" + session + ".jsonl")


def record(value):
    with logfile.open("a", encoding="utf-8") as out:
        out.write(json.dumps(value) + "\n")
        out.flush()


def emit(value):
    print(json.dumps(value), flush=True)


current = {"uuid": None}


def finish(reply):
    emit({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": reply}}})
    emit({"type": "result", "subtype": "success", "is_error": False, "total_cost_usd": 0, "fast_mode_state": "off",
          "usage": {"input_tokens": 1, "output_tokens": 1}, "modelUsage": {"claude-sonnet-4-6": {"contextWindow": 200000}}})
    if current["uuid"]:
        emit({"type": "command_lifecycle", "command_uuid": current["uuid"], "state": "completed"})


# A CLI new enough for `priority` / `origin` on user frames, so the test can see
# how each send was delivered.
emit({"type": "system", "subtype": "init", "session_id": session, "model": "claude-sonnet-4-6", "claude_code_version": "2.1.299", "tools": [], "mcp_servers": []})
for line in sys.stdin:
    frame = json.loads(line)
    if frame.get("type") == "control_response" and frame.get("response", {}).get("request_id") == "queue-question":
        record({"answered": True})
        finish("answered")
        continue
    if frame.get("type") == "control_request":
        response = {}
        if frame.get("request", {}).get("subtype") == "mcp_status":
            response = {"mcpServers": [{"name": "fixture-mcp", "status": "connected"}]}
        emit({"type": "control_response", "response": {"subtype": "success", "request_id": frame["request_id"], "response": response}})
        continue
    if frame.get("type") != "user":
        continue
    content = frame.get("message", {}).get("content", [])
    if isinstance(content, str):
        prompt = content
    else:
        prompt = "\n".join(block.get("text", "") for block in content if block.get("type") == "text")
    record({"message": prompt, "priority": frame.get("priority"), "origin": frame.get("origin")})
    # CLI 2.1.299's delivery receipts, keyed by the uuid the driver stamped.
    current["uuid"] = frame.get("uuid")
    if current["uuid"]:
        emit({"type": "command_lifecycle", "command_uuid": current["uuid"], "state": "started"})
    if "ask-question" in prompt:
        emit({"type": "control_request", "request_id": "queue-question", "request": {
            "subtype": "can_use_tool", "tool_name": "AskUserQuestion", "input": {"questions": [
                {"question": "Continue?", "options": [{"label": "Yes"}, {"label": "No"}]}
            ]}}})
        continue
    if "hold:" in prompt:
        name = prompt.split("hold:", 1)[1].split()[0]
        release = root / ("release-" + name)
        deadline = time.monotonic() + 60
        while not release.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
    finish("done")
