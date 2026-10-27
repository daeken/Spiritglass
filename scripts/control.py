#!/usr/bin/env python3
"""Inspect and control installed modules without opening the native app."""
import argparse
import json
import os
from pathlib import Path
import socket
from module_harness import Peer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", default=os.environ.get("SPIRITGLASS_SOCKET", str(Path.home() / ".spiritglass/run/control.sock")))
    parser.add_argument("command", choices=["list", "reload", "activate", "deactivate", "action"])
    parser.add_argument("module_id", nargs="?")
    parser.add_argument("action", nargs="?")
    parser.add_argument("--arguments", default="{}", help="JSON action arguments")
    options = parser.parse_args()
    if options.command != "list" and not options.module_id:
        parser.error("this command needs a module ID")
    if options.command == "action" and not options.action:
        parser.error("action needs an action ID")
    with socket.socket(socket.AF_UNIX) as connection:
        connection.connect(options.socket)
        reader = connection.makefile("rb")
        try:
            peer = Peer(reader, connection.sendall)
            peer.call("initialize", {"protocol_versions": [2]})
            if options.command == "list":
                result = peer.call("session.subscribe")
            elif options.command == "action":
                result = peer.call("action.invoke", {"module_id": options.module_id, "action": options.action, "arguments": json.loads(options.arguments)})
            else:
                result = peer.call(f"module.{options.command}", {"module_id": options.module_id})
            print(json.dumps(result, indent=2))
        finally:
            connection.shutdown(socket.SHUT_RDWR)
            peer.worker.join(timeout=1)
            reader.close()


if __name__ == "__main__":
    main()
