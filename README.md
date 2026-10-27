# Spiritglass

**A window into everything.**

A local, native macOS dashboard for independent modules. Modules describe their UI
and publish business data; Spiritglass turns that description into SwiftUI controls.

The first vertical slice is implemented: a Rust supervisor, a Rust module SDK, a
live example module, and a native macOS application. The app can disconnect and
reconnect without restarting the module.

## Run the demo

Requires macOS 14+, Xcode with Swift 6+, Python 3, and a current stable Rust toolchain.

```sh
./scripts/run-demo.sh
```

The script builds the workspace, creates an ad hoc signed app at
`.build/Spiritglass.app`, starts the supervisor, and opens the dashboard. Try
**Send a little signal** five times to change the example status, or **Reset**.
The uptime updates every second. Quit and reopen the app to see that the module
continues running.

The system menu bar shows the highest severity among the dashboard's status
components: green for clear, yellow for notice, red for urgent, and a black lamp
with a white rim and exclamation mark for emergency. A question mark indicates
unknown or stale monitoring. Click the lamp to inspect the contributing statuses,
open the dashboard, or quit. Closing the dashboard leaves the app monitoring in
the menu bar; use Quit to exit the app.
The Spiritglass app menu also provides **Show Status** (⇧⌘S) and **Open Dashboard**
(⌘0). Clicking the Dock icon restores a closed dashboard.

The supervisor keeps running after the app closes. The script prints its PID and
the command to stop it. Logs are in `~/.spiritglass/run/demo.log` and `app.log`.
Run the script again to reuse the existing supervisor; stop it first to run a
newly rebuilt module. Automatic hot replacement is a later milestone.

For separate development sessions:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --workspace
cargo run -p spiritglassd -- --module "$PWD/target/debug/spiritglass-example"
```

In another terminal:

```sh
swift run --package-path apps/macos Spiritglass
```

Both use `~/.spiritglass/run/control.sock`. Set `SPIRITGLASS_SOCKET` for a separate
session. Its parent must be a private directory (mode 700). The daemon also accepts
`--socket PATH`, and module arguments after `--`.

## Work on a module

Start with [`examples/rust-module`](examples/rust-module/src/main.rs). Implement
the SDK's `Module` trait: declarations, view, current state, and action handler.
The dashboard tree is fetched once. Source updates change bound values without
fetching or decoding a new tree. Stable widget IDs preserve SwiftUI identity.

Exercise a module without the app or supervisor:

```sh
python3 scripts/module_harness.py --action increment target/debug/spiritglass-example
```

An SDK is optional. Any executable that speaks the [documented protocol](docs/protocol.md)
on stdin/stdout can be a module. Stderr is for diagnostics.

## Check the project

```sh
./scripts/check.sh
```

This checks Rust formatting and linting, protocol/schema compatibility, socket
lifecycle, live actions, reconnection, failed modules, malformed messages, and Swift
binding behavior against a shared Rust-generated snapshot. CI runs the same checks.

Regenerate the checked-in schema after changing wire types:

```sh
cargo run -p spiritglass-protocol --bin export-schema
```

## Project map

| Location | Responsibility |
| --- | --- |
| `apps/macos/` | SwiftUI app, native renderer, Unix socket client |
| `crates/spiritglassd/` | Process supervision, validated routing, retained state |
| `crates/spiritglass-protocol/` | Wire models, validation, schema exporter |
| `crates/spiritglass-sdk/` | Rust module runtime and typed UI properties |
| `examples/rust-module/` | Live uptime, counter actions, example status |
| `protocol/` | JSON Schema and shared wire fixture |
| `scripts/` | Build, launch, module harness, integration checks |

The [initial specification](docs/initial-specification.md),
[architecture decisions](docs/architecture.md), and [roadmap](docs/roadmap.md)
describe the larger project. Manifest discovery, multiple modules, automatic
restart/hot replacement, persisted alert aggregation and notifications, richer
widgets, two-way inputs, and C# interoperability remain future work.
