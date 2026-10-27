"""Raw-protocol fault fixture. Each process reads its deployment exactly once."""
import json
import os
from pathlib import Path
import sys
import time

config_path = Path(sys.argv[1])
config = json.loads(config_path.read_text())
identifier = config["id"]
version = config.get("version", "one")
count = 0
mode = config.get("mode", "normal")


def send(value):
    print(json.dumps(value), flush=True)


def publish():
    value = {"version": version, "count": count, "status": "clear", "pid": os.getpid()}
    if mode == "bad-state":
        value["count"] = "invalid integer"
    send({"jsonrpc": "2.0", "method": "state.publish", "params": {"source": "sample", "value": value}})


for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    if method == "initialize":
        time.sleep(config.get("startup_delay", 0))
        if mode == "bad-json":
            print("bad protocol message", flush=True)
            continue
        result = {"protocol_version": 1}
    elif method == "module.describe":
        result = {"id": "wrong-id" if mode == "wrong-id" else identifier, "name": identifier, "description": f"Version {version}", "view_id": "main",
                  "sources": [{"id": "sample", "schema": {"type": "object", "properties": {"version": {"type": "string"}, "count": {"type": "integer"}, "status": {"type": "string"}, "pid": {"type": "integer"}}}}],
                  "actions": [{"id": name, "arguments": {"type": "object", "properties": {}}} for name in ["increment", "crash", "slow"]]}
    elif method == "view.get":
        result = {"type": "status", "id": "status", "label": "Fixture status", "value": {"kind": "binding", "source": "sample", "path": ["missing" if mode == "bad-view" else "status"]}}
    elif method == "action.invoke":
        action = request["params"]["action"]
        if action == "crash":
            os._exit(7)
        if action == "slow":
            config_path.with_suffix(".inflight").write_text(str(os.getpid()))
            time.sleep(0.8)
        count += 1
        publish()
        result = {"version": version, "count": count}
    elif method == "shutdown":
        result = None
    else:
        raise AssertionError(method)
    send({"jsonrpc": "2.0", "id": request["id"], "result": result})
    if method == "view.get" and mode != "missing-initial":
        publish()
    if method == "shutdown":
        break
