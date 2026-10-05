# Deterministic `codex exec --json` stand-in for automatic-title tests. Records
# argv and stdin for each call in titler-calls.jsonl beside its bin/ folder;
# never contacts a model provider. A model id containing "reject" fails the way
# the real CLI does for an unknown model.
import json
from pathlib import Path
import sys

root = Path(__file__).resolve().parent.parent
prompt = sys.stdin.read()
args = sys.argv[1:]
with (root / "titler-calls.jsonl").open("a", encoding="utf-8") as out:
    out.write(json.dumps({"argv": args, "stdin": prompt}) + "\n")
model = args[args.index("--model") + 1] if "--model" in args else ""
if "reject" in model:
    print("error: unknown model '" + model + "'", file=sys.stderr)
    sys.exit(1)
print(json.dumps({"type": "thread.started", "thread_id": "fixture"}))
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": "Title: Fix the login redirect loop."}}))
print(json.dumps({"type": "turn.completed"}))
