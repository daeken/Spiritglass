#!/usr/bin/env python3
"""Exercise installation, isolated failures, and generation handover with real processes."""
import concurrent.futures
import contextlib
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
from module_harness import Peer

ROOT = Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "scripts/fixtures/lifecycle_module.py"


def atomic(path, value):
    temporary = path.with_suffix(".new")
    temporary.write_text(json.dumps(value))
    temporary.replace(path)


class Client(Peer):
    def __init__(self, path):
        self.connection = socket.socket(socket.AF_UNIX)
        self.connection.connect(str(path))
        self.reader = self.connection.makefile("rb")
        self.snapshot = {"modules": {}}
        super().__init__(self.reader, self.connection.sendall)
        self.call("initialize", {"protocol_versions": [2]})
        self.snapshot = self.call("session.subscribe")

    def notify(self, message):
        if message["method"] == "session.snapshot":
            self.snapshot = message["params"]
        else:
            assert message["method"] == "state.changed", message
            for identifier, update in message["params"]["modules"].items():
                self.snapshot["modules"][identifier]["state"].update(update)

    def wait(self, predicate, timeout=12):
        deadline = time.monotonic() + timeout
        while not predicate(self.snapshot):
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AssertionError(json.dumps(self.snapshot, indent=2))
            self.notify(self.receive(remaining))
        return self.snapshot

    def ready(self, identifier):
        self.wait(lambda s: s["modules"].get(identifier, {}).get("phase") == "ready")
        return self.snapshot["modules"][identifier]

    def action(self, identifier, action="increment", **options):
        return self.call("action.invoke", {"module_id": identifier, "action": action, "arguments": {}}, **options)

    def control(self, identifier, method):
        return self.call(f"module.{method}", {"module_id": identifier})

    def __enter__(self):
        return self

    def __exit__(self, *_):
        with contextlib.suppress(OSError):
            self.connection.shutdown(socket.SHUT_RDWR)
        self.worker.join(timeout=1)
        self.reader.close()
        self.connection.close()


class World:
    def __init__(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="sg-life-", dir="/tmp")
        self.root = Path(self.temporary.name)
        self.modules = self.root / "modules"
        self.modules.mkdir()
        self.socket = self.root / "control.sock"
        self.process = None
        self.log = tempfile.TemporaryFile()

    def install(self, identifier, **settings):
        folder = self.modules / identifier
        folder.mkdir(exist_ok=True)
        config = folder / "deployment.json"
        atomic(config, {"id": identifier, **settings})
        manifest = {"id": identifier, "name": identifier, "executable": sys.executable,
                    "args": [str(FIXTURE), str(config)], "watch": ["deployment.json"],
                    "lifecycle": settings.get("lifecycle", "always"), "enabled": settings.get("enabled", True)}
        if "stale_after_ms" in settings:
            manifest["stale_after_ms"] = settings["stale_after_ms"]
        atomic(folder / "module.json", manifest)
        return config

    def deploy(self, identifier, **settings):
        config = self.modules / identifier / "deployment.json"
        atomic(config, {"id": identifier, **settings})

    def start(self):
        self.process = subprocess.Popen([str(ROOT / "target/debug/spiritglassd"), "--socket", str(self.socket), "--modules-dir", str(self.modules)], stdout=self.log, stderr=self.log, start_new_session=True)
        deadline = time.monotonic() + 8
        while True:
            try:
                with socket.socket(socket.AF_UNIX) as connection:
                    connection.connect(str(self.socket))
                return self
            except OSError:
                if self.process.poll() is not None or time.monotonic() >= deadline:
                    self.log.seek(0)
                    raise RuntimeError(self.log.read().decode())
                time.sleep(0.02)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        if self.process and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait()
        self.log.close()
        self.temporary.cleanup()


class Lifecycle(unittest.TestCase):
    def test_read_only_project_monitor_and_nonblocking_refresh(self):
        with World() as world:
            project = world.root / "project"; project.mkdir()
            subprocess.run(["git", "init", "-q", "-b", "main", str(project)], check=True)
            (project / "README").write_text("A project\n")
            subprocess.run(["git", "-C", str(project), "add", "README"], check=True)
            subprocess.run(["git", "-C", str(project), "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-q", "-m", "Initial"], check=True)
            (project / "scratch").write_text("Work in progress\n")
            index_timestamp = (project / ".git/index").stat().st_mtime_ns
            folder = world.modules / "project"; folder.mkdir()
            atomic(folder / "module.json", {"id":"dev.spiritglass.project", "name":"Project", "executable":str(ROOT / "target/debug/spiritglass-project-monitor"), "args":[str(project)]})
            world.start()
            with Client(world.socket) as client:
                client.ready("dev.spiritglass.project")
                client.wait(lambda s: s["modules"]["dev.spiritglass.project"]["state"]["sources"]["project"]["value"]["changed"] == 1)
                value = client.snapshot["modules"]["dev.spiritglass.project"]["state"]["sources"]["project"]["value"]
                self.assertEqual(value["status"], "notice")
                self.assertEqual(value["branch"], "Branch · main")
                self.assertEqual((project / ".git/index").stat().st_mtime_ns, index_timestamp)
                (project / "another").write_text("More work\n")
                self.assertTrue(client.action("dev.spiritglass.project", "refresh")["value"]["refresh_requested"])
                client.wait(lambda s: s["modules"]["dev.spiritglass.project"]["state"]["sources"]["project"]["value"]["changed"] == 2)

    def test_install_remove_and_action_namespaces(self):
        with World() as world:
            world.install("one")
            world.start()
            with Client(world.socket) as client:
                first = client.ready("one")
                world.install("two")
                client.ready("two")
                self.assertEqual(client.action("one")["value"]["count"], 1)
                self.assertEqual(client.action("one")["value"]["count"], 2)
                self.assertEqual(client.action("two")["value"]["count"], 1)
                shutil.rmtree(world.modules / "two")
                client.wait(lambda s: "two" not in s["modules"])
                self.assertEqual(client.snapshot["modules"]["one"]["pid"], first["pid"])
                with Client(world.socket) as reconnect:
                    self.assertEqual(reconnect.snapshot["modules"]["one"]["state"]["sources"]["sample"]["value"]["count"], 2)

    def test_failed_candidates_preserve_live_process_and_routing(self):
        with World() as world:
            world.install("one")
            world.start()
            with Client(world.socket) as client:
                first = client.ready("one")
                for mode in ["bad-json", "wrong-id", "bad-view", "bad-state", "missing-initial"]:
                    with self.subTest(mode=mode):
                        world.deploy("one", mode=mode)
                        prior = client.snapshot["modules"]["one"].get("reload_error")
                        client.control("one", "reload")
                        client.wait(lambda s: not s["modules"]["one"]["reloading"] and s["modules"]["one"]["reload_error"] and s["modules"]["one"]["reload_error"] != prior)
                        current = client.snapshot["modules"]["one"]
                        self.assertEqual(current["pid"], first["pid"])
                        self.assertEqual(current["phase"], "ready")
                        self.assertEqual(client.action("one")["value"]["version"], "one")
                world.deploy("one", version="two")
                client.wait(lambda s: s["modules"]["one"]["generation"] != first["generation"])
                self.assertIsNone(client.snapshot["modules"]["one"]["reload_error"])
                self.assertEqual(client.action("one")["value"]["version"], "two")

    def test_crash_recovery_isolated_from_other_module(self):
        with World() as world:
            world.install("one"); world.install("two"); world.start()
            with Client(world.socket) as client:
                old = client.ready("one"); other = client.ready("two")
                self.assertIn("error", client.action("one", "crash", allow_error=True))
                client.wait(lambda s: s["modules"]["one"]["phase"] == "ready" and s["modules"]["one"]["generation"] != old["generation"])
                self.assertGreater(client.snapshot["modules"]["one"]["restart_count"], 0)
                self.assertEqual(client.snapshot["modules"]["two"]["pid"], other["pid"])
                self.assertEqual(client.action("two")["value"]["count"], 1)

    def test_on_demand_and_disabled_lifecycle(self):
        with World() as world:
            world.install("lazy", lifecycle="on_demand")
            world.install("disabled", enabled=False)
            world.start()
            with Client(world.socket) as client:
                client.wait(lambda s: "disabled" in s["modules"] and "lazy" in s["modules"])
                self.assertEqual(client.snapshot["modules"]["lazy"]["phase"], "dormant")
                self.assertEqual(client.snapshot["modules"]["disabled"]["phase"], "disabled")
                self.assertIn("error", client.call("module.activate", {"module_id": "disabled"}, allow_error=True))
                client.control("lazy", "activate"); client.ready("lazy")
                client.control("lazy", "deactivate")
                client.wait(lambda s: s["modules"]["lazy"]["phase"] == "dormant")
                self.assertEqual(client.snapshot["modules"]["lazy"]["state"]["sources"]["sample"]["status"], "stale")

    def test_invalid_manifests_never_remove_live_modules(self):
        with World() as world:
            world.install("one"); world.start()
            with Client(world.socket) as client:
                first = client.ready("one")
                path = world.modules / "one/module.json"
                saved = path.read_text()
                path.write_text("{invalid")
                client.wait(lambda s: bool(s["diagnostics"]))
                self.assertEqual(client.snapshot["modules"]["one"]["pid"], first["pid"])
                self.assertEqual(client.action("one")["value"]["count"], 1)
                path.write_text(saved)
                client.wait(lambda s: not s["diagnostics"])
                duplicate = world.modules / "duplicate.json"
                duplicate.write_text(saved)
                client.wait(lambda s: bool(s["diagnostics"]))
                self.assertEqual(client.snapshot["modules"]["one"]["pid"], first["pid"])

    def test_optional_source_freshness(self):
        with World() as world:
            world.install("one", stale_after_ms=1000); world.start()
            with Client(world.socket) as client:
                first = client.ready("one")
                client.wait(lambda s: s["modules"]["one"]["state"]["sources"]["sample"]["status"] == "stale")
                self.assertEqual(client.snapshot["modules"]["one"]["pid"], first["pid"])
                client.action("one")
                client.wait(lambda s: s["modules"]["one"]["state"]["sources"]["sample"]["status"] == "available")

    def test_inflight_actions_finish_on_original_generation(self):
        with World() as world:
            config = world.install("one"); world.start()
            with Client(world.socket) as first, Client(world.socket) as second:
                old = first.ready("one"); second.ready("one")
                world.deploy("one", version="two")
                with concurrent.futures.ThreadPoolExecutor() as executor:
                    action = executor.submit(first.action, "one", "slow")
                    deadline = time.monotonic() + 3
                    while not config.with_suffix(".inflight").exists():
                        self.assertLess(time.monotonic(), deadline)
                        time.sleep(0.01)
                    second.control("one", "reload")
                    second.wait(lambda s: s["modules"]["one"]["generation"] != old["generation"])
                    result = action.result(timeout=5)
                self.assertEqual(result["generation"], old["generation"])
                self.assertEqual(result["active_generation"], second.snapshot["modules"]["one"]["generation"])
                self.assertEqual(result["value"]["version"], "one")
                self.assertEqual(second.snapshot["modules"]["one"]["state"]["sources"]["sample"]["value"]["version"], "two")

    def test_atomic_native_executable_replacement(self):
        with World() as world:
            folder = world.modules / "native"; folder.mkdir()
            deployed = folder / "module"
            source = ROOT / "target/debug/spiritglass-example"
            shutil.copy2(source, deployed)
            atomic(folder / "module.json", {"id":"dev.spiritglass.example", "name":"Native", "executable":"module"})
            world.start()
            with Client(world.socket) as client:
                old = client.ready("dev.spiritglass.example")
                broken = folder / "broken"; broken.write_bytes(b"a failed build is not an executable")
                broken.chmod(0o755); broken.replace(deployed)
                client.wait(lambda s: bool(s["modules"]["dev.spiritglass.example"]["reload_error"]))
                self.assertEqual(client.snapshot["modules"]["dev.spiritglass.example"]["pid"], old["pid"])
                self.assertEqual(client.action("dev.spiritglass.example")["value"]["count"], 1)
                restored = folder / "restored"; shutil.copy2(source, restored); restored.replace(deployed)
                client.wait(lambda s: s["modules"]["dev.spiritglass.example"]["generation"] != old["generation"])
                self.assertIsNone(client.snapshot["modules"]["dev.spiritglass.example"]["reload_error"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
