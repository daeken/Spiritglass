#!/usr/bin/env python3
"""Exercise a module over its real stdio protocol, without the macOS app."""
import argparse
import json
import queue
import subprocess
import threading
import time

MAX_MESSAGE_BYTES = 1_048_576


class Peer:
    def __init__(self, reader, send):
        self.send_bytes = send
        self.messages = queue.Queue(maxsize=128)
        self.next_id = 1
        self.notifications = []

        def read():
            try:
                while line := reader.readline(MAX_MESSAGE_BYTES + 2):
                    if len(line.rstrip(b"\n")) > MAX_MESSAGE_BYTES:
                        raise ValueError("oversized protocol message")
                    self.messages.put(json.loads(line), timeout=5)
            except Exception as error:
                self.messages.put(error, timeout=5)
            finally:
                self.messages.put(None, timeout=5)

        self.worker = threading.Thread(target=read, daemon=True)
        self.worker.start()

    def send(self, message):
        self.send_bytes(json.dumps(message, separators=(",", ":")).encode() + b"\n")

    def receive(self, timeout=8):
        message = self.messages.get(timeout=timeout)
        if message is None:
            raise EOFError("protocol connection closed")
        if isinstance(message, Exception):
            raise message
        assert message.get("jsonrpc") == "2.0", message
        return message

    def notify(self, message):
        self.notifications.append(message)

    def call(self, method, params=None, *, request_id=None, allow_error=False):
        identifier = self.next_id if request_id is None else request_id
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": identifier, "method": method, "params": {} if params is None else params})
        deadline = time.monotonic() + 8
        while True:
            message = self.receive(max(0.01, deadline - time.monotonic()))
            if "method" in message:
                self.notify(message)
                continue
            assert message["id"] == identifier, message
            if allow_error:
                return message
            if "error" in message:
                raise RuntimeError(message["error"])
            return message["result"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--action", help="Optional action ID to invoke with empty arguments")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    options = parser.parse_args()
    if not options.command:
        parser.error("supply a module executable and optional arguments")
    process = subprocess.Popen(options.command, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    try:
        peer = Peer(process.stdout, lambda data: (process.stdin.write(data), process.stdin.flush()))
        assert peer.call("initialize", {"protocol_versions": [1]}) == {"protocol_version": 1}
        description = peer.call("module.describe")
        view = peer.call("view.get", {"view_id": description["view_id"]})
        print(json.dumps({"module": description, "view": view}, indent=2))
        if options.action:
            result = peer.call("action.invoke", {"action": options.action, "arguments": {}})
            print(json.dumps({"action_result": result, "notifications": peer.notifications}, indent=2))
        peer.call("shutdown")
        assert process.wait(timeout=3) == 0
        print("Module handshake, view retrieval, and graceful shutdown passed.")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


if __name__ == "__main__":
    main()
