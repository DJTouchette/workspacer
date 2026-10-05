# Deterministic stream-json agent for automatic-title tests. Never contacts a
# model provider: every user turn is answered with a fixed sentence.
import json
import sys

args = sys.argv[1:]
session = args[args.index("--session-id") + 1]


def emit(value):
    print(json.dumps(value), flush=True)


emit({"type": "system", "subtype": "init", "session_id": session, "model": "claude-sonnet-4-6", "tools": [], "mcp_servers": []})
for line in sys.stdin:
    frame = json.loads(line)
    if frame.get("type") == "control_request":
        emit({"type": "control_response", "response": {"subtype": "success", "request_id": frame["request_id"], "response": {}}})
        continue
    if frame.get("type") != "user":
        continue
    emit({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {
        "type": "text_delta", "text": "I traced the redirect to the session cookie check."}}})
    emit({"type": "result", "subtype": "success", "is_error": False, "total_cost_usd": 0,
          "usage": {"input_tokens": 1, "output_tokens": 1}, "modelUsage": {"claude-sonnet-4-6": {"contextWindow": 200000}}})
