# Spiritglass

**A window into everything.**

A local, native macOS control plane for independent modules. Modules describe
their dashboards and publish business data; Spiritglass turns that description
into SwiftUI controls.

Spiritglass now discovers installed modules, runs them independently, recovers
from crashes, and replaces running versions only after their replacements pass
validation. The native menu bar summarizes every active module.

## Run it

Requires macOS 14+, Xcode with Swift 6+, Python 3, and current stable Rust.

```sh
./scripts/run-demo.sh
```

This builds an ad hoc signed app at .build/Spiritglass.app and atomically
installs two ordinary modules in ~/.spiritglass/modules:

- **Hello, Spiritglass:** live uptime, counter actions, and an example status.
- **Project watch:** read-only Git branch, changed/staged/untracked files, local
  ahead/behind counts, and the latest commit for this project.

Open either dashboard from the sidebar. Try Reload to validate and promote a new
process while the old one keeps running. Rebuilding and redeploying a module
triggers the same handover automatically. Unchanged example binaries are not
redeployed, so deploying one does not reset the other.

The lamp uses green/clear, yellow/notice, red/urgent, and a white-rimmed black
emergency indicator. Unknown or stale monitoring has a question mark and never
silently appears green. Click it for contributing statuses, open the dashboard,
or quit. Closing the window keeps monitoring running. Show Status (⇧⌘S) and
Open Dashboard (⌘0) are also in the app menu.

The supervisor survives frontend disconnects. The launcher prints its PID and
stop command. Logs are in ~/.spiritglass/run/demo.log and app.log. If upgrading
from the first single-module version, stop that old supervisor once before
running the new launcher.

## Install a module

Create a folder containing its independently built executable and module.json:

```json
{
  "id": "dev.example.my-module",
  "name": "My module",
  "executable": "module",
  "args": [],
  "lifecycle": "always",
  "enabled": true,
  "stale_after_ms": 5000
}
```

Put the folder inside ~/.spiritglass/modules. Its declaration must return the
same ID as its manifest. The dashboard appears without changing or rebuilding
the frontend or supervisor. Flat JSON manifests are supported too.

Paths are relative to the manifest folder. Use an absolute executable path for
an interpreter and supply its script in args; add script/config/DLL paths to
watch for automatic reload. There is no shell expansion. Set working_directory
or environment explicitly when needed.

lifecycle can be always or on_demand. An on-demand module starts when selected
or activated and stays running until stopped. Set enabled to false to disable a
module. Optional stale_after_ms marks quiet source values stale; it does not
assume that an idle process is healthy.

Deploy executables and manifests with atomic rename. A partially written or bad
candidate must not evict the healthy process. Removing the installed manifest
uninstalls its module. See [module management](docs/modules.md) for the full loop.

## Develop and inspect

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --workspace
python3 scripts/install-examples.py
cargo run -p spiritglassd
```

In another terminal:

```sh
swift run --package-path apps/macos Spiritglass
```

Both use ~/.spiritglass/run/control.sock. SPIRITGLASS_SOCKET selects another
socket; its parent must be private (mode 700). The daemon accepts --modules-dir.
The demo launcher also accepts SPIRITGLASS_MODULES_DIR.

Headless controls:

```sh
python3 scripts/control.py list
python3 scripts/control.py reload dev.spiritglass.project
python3 scripts/control.py action dev.spiritglass.example increment
```

Start with [the Rust example](examples/rust-module/src/main.rs) or the
[project monitor](modules/project-monitor/src/main.rs). An SDK is optional:
any executable can speak the [module protocol](docs/protocol.md) on stdin/stdout.

```sh
python3 scripts/module_harness.py --action increment target/debug/spiritglass-example
```

## Verify

```sh
./scripts/check.sh
```

Checks include Rust formatting/linting, schemas and shared Rust/Swift wire
fixtures, real-process crash recovery, malformed modules, reconnects, atomic
deployment, failed replacements, on-demand lifecycle, freshness, action
namespaces, in-flight handover, and native binding/attention behavior.

Regenerate both protocol schemas after changing wire types:

```sh
cargo run -p spiritglass-protocol --bin export-schema
```

## Project map

| Location | Responsibility |
| --- | --- |
| apps/macos/ | Native app, recursive renderer, socket client, status popover |
| crates/spiritglassd/ | Discovery, process generations, routing, retained state |
| crates/spiritglass-protocol/ | Module v1 and client v2 wire models and validation |
| crates/spiritglass-sdk/ | Rust module runtime and typed UI properties |
| examples/rust-module/ | Counter and status example |
| modules/project-monitor/ | Independent, read-only Git project monitor |
| protocol/ | Generated schemas and shared wire fixtures |
| scripts/ | Build, atomic deployment, controls, and verification |

The [initial specification](docs/initial-specification.md),
[architecture](docs/architecture.md), and [roadmap](docs/roadmap.md) describe the
larger project. Persisted alerts, acknowledgment/snoozing, notifications, richer
widgets, editable bindings, and a C# SDK/example remain future work.
