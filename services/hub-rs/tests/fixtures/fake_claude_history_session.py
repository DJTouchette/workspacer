# Deterministic stream-json agent for the hub's conversation-history tests.
# Never contacts a model provider. Speaks the shapes observed live on Claude
# Code 2.1.299: user frames are replayed (`isReplay`) with their uuid, a turn
# streams thinking then text, `/compact` brackets a `compact_boundary`, and
# `rewind_files` answers a dry run with a result and a real one with the CLI's
# no-checkpoint error. Every user frame and the spawn argv/env are recorded.
import json
import os
from pathlib import Path
import sys

root = Path(os.environ["WKS_HISTORY_FIXTURE"])
args = sys.argv[1:]
session = args[args.index("--session-id") + 1]
logfile = root / ("received-" + session + ".jsonl")
(root / ("argv-" + session + ".json")).write_text(json.dumps({
    "argv": args,
    "checkpointing": os.environ.get("CLAUDE_CODE_ENABLE_SDK_FILE_CHECKPOINTING"),
}))


def record(value):
    with logfile.open("a", encoding="utf-8") as out:
        out.write(json.dumps(value) + "\n")
        out.flush()


transcript = Path(os.environ["HOME"]) / ".claude" / "projects" / os.getcwd().rstrip("/").replace("/", "-") / (session + ".jsonl")
transcript.parent.mkdir(parents=True, exist_ok=True)
last_entry = [None]


def chain(row):
    # Transcript rows the way Claude Code chains them (uuid / parentUuid).
    row["parentUuid"] = last_entry[0]
    last_entry[0] = row["uuid"]
    with transcript.open("a", encoding="utf-8") as out:
        out.write(json.dumps(row) + "\n")


def emit(value):
    print(json.dumps(value), flush=True)


def result():
    emit({"type": "result", "subtype": "success", "is_error": False, "total_cost_usd": 0,
          "usage": {"input_tokens": 1, "output_tokens": 1},
          "modelUsage": {"claude-sonnet-4-6": {"contextWindow": 200000}}})


emit({"type": "system", "subtype": "init", "session_id": session, "model": "claude-sonnet-4-6",
      "claude_code_version": "2.1.299", "tools": [], "mcp_servers": [], "slash_commands": ["compact"]})
for line in sys.stdin:
    frame = json.loads(line)
    if frame.get("type") == "control_request":
        request = frame.get("request", {})
        rid = frame["request_id"]
        if request.get("subtype") == "rewind_files":
            record({"rewind": request})
            if request.get("dry_run"):
                emit({"type": "control_response", "response": {"subtype": "success", "request_id": rid,
                      "response": {"canRewind": True, "filesChanged": ["a.txt"], "insertions": 0, "deletions": 1}}})
            else:
                emit({"type": "control_response", "response": {"subtype": "error", "request_id": rid,
                      "error": "No file checkpoint found for this message."}})
            continue
        emit({"type": "control_response", "response": {"subtype": "success", "request_id": rid, "response": {}}})
        continue
    if frame.get("type") != "user":
        continue
    content = frame.get("message", {}).get("content", [])
    if isinstance(content, str):
        prompt = content
    else:
        prompt = "\n".join(block.get("text", "") for block in content if block.get("type") == "text")
    record({"message": prompt, "uuid": frame.get("uuid")})
    chain({"type": "user", "uuid": frame.get("uuid"), "message": {"role": "user", "content": prompt}})
    if not prompt.startswith("/compact"):
        chain({"type": "assistant", "uuid": "a-" + frame.get("uuid"),
               "message": {"id": "msg_1", "role": "assistant", "content": [{"type": "text", "text": "done"}]}})
    emit({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": prompt}]},
          "parent_tool_use_id": None, "uuid": frame.get("uuid"), "isReplay": True})
    if prompt.startswith("/compact"):
        emit({"type": "system", "subtype": "status", "status": "compacting"})
        emit({"type": "system", "subtype": "status", "status": None, "compact_result": "success"})
        emit({"type": "system", "subtype": "compact_boundary",
              "compact_metadata": {"trigger": "manual", "pre_tokens": 25000, "post_tokens": 2764}})
        result()
        continue
    emit({"type": "stream_event", "event": {"type": "message_start", "message": {"id": "msg_1", "model": "claude-sonnet-4-6"}}})
    emit({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0,
          "delta": {"type": "thinking_delta", "thinking": "Weighing the request."}}})
    emit({"type": "stream_event", "event": {"type": "content_block_delta", "index": 1,
          "delta": {"type": "text_delta", "text": "done"}}})
    emit({"type": "assistant", "message": {"id": "msg_1", "model": "claude-sonnet-4-6", "content": [
        {"type": "thinking", "thinking": "Weighing the request.", "signature": "s"}]}})
    result()
