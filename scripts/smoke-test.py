#!/usr/bin/env python3
"""End-to-end checks against the actual supervisor and module executables."""
import contextlib
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from module_harness import Peer

ROOT = Path(__file__).resolve().parent.parent
DAEMON = ROOT / "target/debug/spiritglassd"
EXAMPLE = ROOT / "target/debug/spiritglass-example"


class Session(Peer):
    def __init__(self, path):
        self.connection = socket.socket(socket.AF_UNIX)
        self.connection.connect(str(path))
        self.reader = self.connection.makefile("rb")
        self.snapshot = None
        super().__init__(self.reader, self.connection.sendall)

    def subscribe(self):
        self.call("initialize", {"protocol_versions": [1]})
        self.snapshot = self.call("session.subscribe")
        return self

    def notify(self, message):
        if message["method"] == "session.snapshot":
            self.snapshot = message["params"]
        elif message["method"] == "state.changed":
            self.snapshot.update(message["params"])

    def wait(self, predicate):
        deadline = time.monotonic() + 8
        while not predicate(self.snapshot):
            self.notify(self.receive(max(0.01, deadline - time.monotonic())))
        return self.snapshot

    def close(self):
        with contextlib.suppress(OSError):
            self.connection.shutdown(socket.SHUT_RDWR)
        self.worker.join(timeout=1)
        self.reader.close()
        self.connection.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()


class Supervisor:
    def __init__(self, fixture=None, directory=None):
        self.directory = directory or tempfile.TemporaryDirectory(prefix="sg-", dir="/tmp")
        self.path = Path(self.directory.name) / "control.sock"
        self.log = tempfile.TemporaryFile()
        command = [str(DAEMON), "--socket", str(self.path), "--module", str(EXAMPLE)]
        if fixture:
            command = [str(DAEMON), "--socket", str(self.path), "--module", sys.executable, "--", str(Path(__file__).resolve()), "--fixture", fixture]
        self.process = subprocess.Popen(command, stdout=self.log, stderr=self.log, start_new_session=True)
        deadline = time.monotonic() + 8
        while True:
            try:
                with socket.socket(socket.AF_UNIX) as connection:
                    connection.connect(str(self.path))
                break
            except OSError:
                if self.process.poll() is not None or time.monotonic() >= deadline:
                    self.log.seek(0)
                    raise RuntimeError(self.log.read().decode())
                time.sleep(0.02)

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()
        self.log.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
        self.directory.cleanup()


class EndToEnd(unittest.TestCase):
    def test_rust_sdk_handles_bad_messages_and_graceful_shutdown(self):
        process = subprocess.Popen([str(EXAMPLE)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        try:
            peer = Peer(process.stdout, lambda data: (process.stdin.write(data), process.stdin.flush()))
            peer.send_bytes(b'not JSON\n')
            self.assertEqual(peer.receive()["error"]["code"], -32700)
            self.assertEqual(peer.call("module.describe", allow_error=True)["error"]["code"], -32002)
            self.assertEqual(peer.call("initialize", {"protocol_versions": [1]}), {"protocol_version": 1})
            description = peer.call("module.describe")
            self.assertEqual(description["id"], "dev.spiritglass.example")
            peer.call("view.get", {"view_id": description["view_id"]})
            self.assertEqual(peer.call("action.invoke", {"action": "increment", "arguments": {}}), {"count": 1})
            published = [message["params"]["value"] for message in peer.notifications if message["method"] == "state.publish"]
            self.assertTrue(any(value["count"] == 1 for value in published))
            peer.call("shutdown")
            self.assertEqual(process.wait(timeout=3), 0)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            process.stdin.close()
            process.stdout.close()

    def test_live_state_actions_validation_and_reconnection(self):
        with Supervisor() as supervisor:
            with Session(supervisor.path).subscribe() as client:
                snapshot = client.wait(lambda s: s["health"] == "ready" and "overview" in s["sources"])
                first_revision = snapshot["sources"]["overview"]["revision"]
                client.wait(lambda s: s["sources"]["overview"]["revision"] > first_revision)
                for count in range(1, 6):
                    self.assertEqual(client.call("action.invoke", {"action": "increment", "arguments": {}}, request_id=f"action-{count}"), {"count": count})
                client.wait(lambda s: s["sources"]["overview"]["value"]["count"] == 5)
                self.assertEqual(client.snapshot["sources"]["overview"]["value"]["status"], "notice")
                error = client.call("action.invoke", {"action": "increment", "arguments": {"unexpected": True}}, allow_error=True)
                self.assertEqual(error["error"]["code"], -32602)
                client.send_bytes(b'{"jsonrpc":"1.0","id":99,"method":"x"}\n')
                response = client.receive()
                while "method" in response:
                    client.notify(response)
                    response = client.receive()
                self.assertEqual(response["error"]["code"], -32600)
            with Session(supervisor.path).subscribe() as reconnect:
                self.assertEqual(reconnect.snapshot["sources"]["overview"]["value"]["count"], 5)
                self.assertEqual(reconnect.call("action.invoke", {"action": "reset", "arguments": {}}), {"count": 0})

    def test_protocol_negotiation(self):
        with Supervisor() as supervisor, Session(supervisor.path) as client:
            self.assertEqual(client.call("session.subscribe", allow_error=True)["error"]["code"], -32002)
            self.assertEqual(client.call("initialize", {"protocol_versions": [999]}, allow_error=True)["error"]["code"], -32001)
            self.assertEqual(client.call("initialize", {"protocol_versions": [1]}), {"protocol_version": 1})

    def test_crash_and_invalid_state_retain_stale_values(self):
        for fixture in ("crash", "bad-state", "bad-json-after-start"):
            with self.subTest(fixture=fixture), Supervisor(fixture) as supervisor, Session(supervisor.path).subscribe() as client:
                client.wait(lambda s: "sample" in s["sources"])
                response = client.call("action.invoke", {"action": "fail", "arguments": {}}, allow_error=True)
                self.assertIn("error", response)
                snapshot = client.wait(lambda s: s["health"] == "unavailable")
                self.assertEqual(snapshot["sources"]["sample"]["value"], 7)
                self.assertEqual(snapshot["sources"]["sample"]["status"], "stale")
                self.assertTrue(snapshot["diagnostic"])
                with Session(supervisor.path).subscribe() as reconnect:
                    self.assertEqual(reconnect.snapshot["health"], "unavailable")
                    self.assertEqual(reconnect.snapshot["sources"]["sample"]["status"], "stale")

    def test_bad_startup_does_not_take_down_supervisor(self):
        for fixture in ("bad-json", "incompatible"):
            with self.subTest(fixture=fixture), Supervisor(fixture) as supervisor, Session(supervisor.path).subscribe() as client:
                snapshot = client.wait(lambda s: s["health"] == "unavailable")
                self.assertIsNone(snapshot["view"])
                self.assertTrue(snapshot["diagnostic"])

    def test_live_socket_is_exclusive_and_stale_socket_recovers(self):
        directory = tempfile.TemporaryDirectory(prefix="sg-", dir="/tmp")
        try:
            with Supervisor(directory=directory) as supervisor:
                conflict = subprocess.run([str(DAEMON), "--socket", str(supervisor.path), "--module", str(EXAMPLE)], capture_output=True, timeout=3)
                self.assertNotEqual(conflict.returncode, 0)
                self.assertEqual(supervisor.path.stat().st_mode & 0o777, 0o600)
                os.killpg(supervisor.process.pid, signal.SIGKILL)
                supervisor.process.wait()
                self.assertTrue(supervisor.path.exists())
                with Supervisor(directory=directory) as replacement, Session(replacement.path).subscribe() as client:
                    client.wait(lambda s: s["health"] == "ready")
                self.assertFalse(supervisor.path.exists())
        finally:
            directory.cleanup()


def fixture(mode):
    if mode == "bad-json":
        print("not JSON", flush=True)
        time.sleep(10)
        return
    for line in sys.stdin:
        message = json.loads(line)
        method = message["method"]
        if method == "initialize":
            result = {"protocol_version": 999 if mode == "incompatible" else 1}
        elif method == "module.describe":
            result = {"id": "fixture", "name": "Fixture", "description": "Fault injection", "view_id": "main", "sources": [{"id": "sample", "schema": {"type": "integer"}}], "actions": [{"id": "fail", "arguments": {"type": "object", "properties": {}}}]}
        elif method == "view.get":
            result = {"type": "metric", "id": "value", "label": "Sample", "value": {"kind": "binding", "source": "sample", "path": []}, "unit": None}
        elif method == "action.invoke":
            if mode == "crash":
                os._exit(7)
            if mode == "bad-json-after-start":
                print("not JSON", flush=True)
            else:
                print(json.dumps({"jsonrpc": "2.0", "method": "state.publish", "params": {"source": "sample", "value": "invalid integer"}}), flush=True)
            continue
        elif method == "shutdown":
            result = None
        else:
            raise AssertionError(method)
        print(json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}), flush=True)
        if method == "view.get":
            print(json.dumps({"jsonrpc": "2.0", "method": "state.publish", "params": {"source": "sample", "value": 7}}), flush=True)
        if method == "shutdown":
            return


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--fixture":
        fixture(sys.argv[2])
    else:
        unittest.main(verbosity=2)
