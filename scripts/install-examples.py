#!/usr/bin/env python3
"""Atomically deploy the example and read-only Git monitor as ordinary modules."""
import argparse
import filecmp
import json
import os
from pathlib import Path
import shutil
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def replace(path, writer):
    descriptor, temporary = tempfile.mkstemp(prefix=".deploy-", dir=path.parent)
    os.close(descriptor)
    temporary = Path(temporary)
    try:
        writer(temporary)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--modules-dir", type=Path, default=Path.home() / ".spiritglass/modules")
    parser.add_argument("--project", type=Path, default=ROOT)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/debug")
    options = parser.parse_args()
    destination = options.modules_dir.expanduser().resolve()
    destination.mkdir(mode=0o700, parents=True, exist_ok=True)
    for identifier, name, binary, arguments in [
        ("dev.spiritglass.example", "Hello, Spiritglass", "spiritglass-example", []),
        ("dev.spiritglass.project", "Project watch", "spiritglass-project-monitor", [str(options.project.expanduser().resolve())]),
    ]:
        source = options.bin_dir / binary
        if not source.is_file():
            parser.error(f"{source} is missing; build the Rust workspace first")
        folder = destination / identifier
        folder.mkdir(mode=0o700, exist_ok=True)
        manifest_path = folder / "module.json"
        if manifest_path.exists() and json.loads(manifest_path.read_text())["id"] != identifier:
            parser.error(f"{folder} belongs to a different module")
        deployed = folder / "module"
        if not deployed.is_file() or not filecmp.cmp(source, deployed, shallow=False):
            replace(deployed, lambda path: shutil.copy2(source, path))
        manifest = {"id": identifier, "name": name, "executable": "module", "args": arguments,
                    "lifecycle": "always", "enabled": True, "stale_after_ms": 5000}
        if not manifest_path.exists() or json.loads(manifest_path.read_text()) != manifest:
            replace(manifest_path, lambda path: path.write_text(json.dumps(manifest, indent=2) + "\n"))
        print(f"Installed {name}: {folder}")


if __name__ == "__main__":
    main()
