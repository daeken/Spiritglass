# Architecture and first-slice decisions

```text
Spiritglass.app                  spiritglassd                   Module executable
SwiftUI + local UI state  <----> retained validated state <----> business data/logic
                         Unix socket                    stdin/stdout
                         JSON-RPC 2.0 NDJSON             JSON-RPC 2.0 NDJSON
```

## Boundaries

The protocol crate contains no process management or async runtime. Serde types
are the source of truth for the wire models; Schemars generates a language-neutral
schema. Validation checks declared source shapes, binding paths/types, widget
identities, action arguments, and view complexity.

The SDK runs one module on stdio. Its synchronous trait keeps the example small;
`tick` and action handlers must return promptly. Long-running and concurrent work
will need an asynchronous publisher API in a later SDK revision.

The supervisor owns the module process and a latest-state snapshot. It listens
before initializing the module, so clients can see startup and failure diagnostics.
Declarations and the view become visible together after validation. Published
sources get a supervisor timestamp and increasing revision. On failure, the daemon
retains data and marks every published source stale. Startup failure does not make
the socket unavailable.

The frontend knows no example module IDs or source field names. It renders a
recursive view tree using stable IDs, resolves properties from retained sources,
and routes button actions by ID. It owns sidebar selection and presentation state.
Unknown components render a fallback without preventing their siblings from
rendering. Missing values, stale values, and unavailable connections are visible.

## Transport and state

Both transports use JSON-RPC 2.0 over newline-delimited UTF-8 JSON, capped at 1 MiB
per message. There is no TCP listener. The socket directory is private, the socket
is mode 600, and a held file lock prevents multiple supervisors from owning it.
A stale socket left after an unclean exit is recovered; a regular file at the
socket path is preserved.

Clients negotiate protocol version 1, request an initial snapshot, then receive
updates. `state.changed` contains the complete latest source map and health but no
view tree. That lets slow clients coalesce updates safely. Reconnecting clients
get a fresh complete snapshot without asking the module to republish anything.
This is in-memory retention across frontend reconnects, not persistence across
supervisor restarts.

Only the module task writes business state. A bounded action queue serializes
module calls. Actions are forwarded at most once per received request, with no
automatic replay after a disconnect. Clients must not assume a timed-out or
disconnected action did not execute. Queued actions whose response receiver has
already expired are discarded before execution. Duplicate request IDs do not
provide deduplication across sessions.

Module handshake calls and actions have five-second deadlines. Malformed module
messages, invalid source values, mismatched response IDs, timeouts, and process
exit make the module unavailable and terminate its process. A module's ordinary
JSON-RPC action error is returned to the caller without marking it unavailable.
Malformed client messages receive an error while the connection can continue;
oversized frames disconnect that client. Slow client writes time out. Client
connections and pending action queues are bounded.

The app reconnects automatically. It marks retained values stale while disconnected,
disables action buttons, and never automatically retries an action. A separate
serial socket reader feeds ordered events to the main actor. SwiftUI renders and
updates only on the main actor.

## Deliberate scope

One executable is supplied explicitly to the daemon. There is no discovery,
automatic restart, or executable watcher yet. Restart the daemon to restart the
module. Initialization is cancelable; SIGINT/SIGTERM request graceful module
shutdown, then force termination if necessary.

The app is a Swift package, so it builds and opens in Xcode without committing an
Xcode project. A script packages and signs a local `.app`. This is a development
bundle, not a notarized release or an installed launch agent. The app does not
automatically start the supervisor; the demo launcher starts both independently.

The first renderer supports vertical/horizontal stacks, text, metrics, status,
cards, buttons, and dividers. The app's menu bar lamp summarizes all declared
status components, using their existing typed bindings instead of module-specific
source fields. The highest severity wins. Stale or unknown monitoring gets a
question mark; stale clear never produces green, while known warnings remain
visible with an incomplete-monitoring badge. The popover lists contributing
statuses and their freshness. Closing the dashboard leaves monitoring running;
Open Spiritglass restores it and Quit exits the app.

This is a frontend summary of status components. Persisted alert aggregation,
monitoring freshness policies, and notifications are still future work. No alert
state is inferred from the example counter, and there are no protocol changes.

The app icon source lives in `apps/macos/Resources/AppIcon.png`. The packaging
script generates the standard Retina/non-Retina icon sizes and a native `.icns`
resource before signing the app. Its source prompt and provenance are beside it.
The build refreshes the bundle's modification time and local app registration;
the app also explicitly sets its running Dock icon from the bundled resource at
launch, so an earlier development build's generic icon does not linger.

There are no executable expressions, WebViews, networking endpoints, or dynamic
libraries. Rust UI literals have typed properties; bindings are checked at module
initialization. Compile-time field-path generation and editable controls belong to
the extensibility phase.
