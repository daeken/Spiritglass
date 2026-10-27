# Architecture

Spiritglass.app owns native presentation. spiritglassd owns discovery, process
generations, routing, and retained state. Independent modules own business data
and logic. The frontend contains no module-specific source fields or identities.

## Protocol boundaries

Module stdin/stdout stays on JSON-RPC protocol version 1. Existing modules and
the Rust SDK remain compatible. The Unix socket now negotiates client protocol
version 2 for module collections, addressing, and lifecycle controls.

The protocol crate is independent of async/process management. Its Serde models
generate v1.schema.json and v2.schema.json. Validation covers envelope shape,
identities, data schemas, view complexity, binding types, and action arguments.
Shared wire fixtures are decoded by Rust and Swift tests.

Both transports use bounded newline-delimited UTF-8 JSON with a 1 MiB frame
limit. There is no TCP server. The socket directory is private, its file is mode
600, and a file lock prevents duplicate supervisors. Stale sockets are recovered.

## Supervision

The daemon separates three responsibilities:

- discovery resolves and validates installed manifests and deployed-file fingerprints.
- registry owns the module collection, desired lifecycle, generation routing,
  recovery deadlines, latest public state, and bounded diagnostics.
- process/module implement one stdio connection and one process generation.

Each generation has its own process, bounded action queue, retained source state,
and stop channel. Modules and candidates initialize and publish independently.
A module failure never tears down the registry or unrelated module tasks.

Discovery polls every 500 ms. Atomic manifest edits update configuration, and
deployed executables or extra watch paths trigger replacement after one stable
observation interval. Invalid manifests preserve known live installations.
Directory scan failure never masquerades as uninstalling all modules.

always modules run automatically. on_demand modules start on activation and stop
on explicit deactivation. Disabled/dormant modules do not contribute monitoring.
Automatic idle shutdown is intentionally absent.

## Validation and handover

Candidates negotiate version 1, return a matching module ID, declare valid data
and actions, provide a valid dashboard, and publish valid initial values for every
source. They remain isolated until all checks pass. Optional source freshness
and aggregate payload budgets apply before promotion.

Promotion changes routing and the public module snapshot in the same registry
turn. The previous generation drains already-accepted actions and shuts down.
Its later updates cannot overwrite current state. Calls return the generation
on which they executed. A failed candidate leaves the old process operational
and exposes a replacement error.

The frontend keeps its connection, navigation, and stable module/widget identity
through replacement. Modules own business-state continuity; the example counter
resets with a new process. Deploying a module does not rebuild the app or daemon.

Native executables must be deployed with atomic rename. The supervisor protects
a running generation from failed candidates; it cannot prevent OS failures if
another program overwrites the running executable's existing mapped inode.

## Recovery and freshness

Crashes, malformed module messages, invalid values, mismatched RPC responses,
and timed-out actions mark retained data stale. Recovery uses exponential
backoff from 250 ms to 30 seconds, with streak reset after 30 healthy seconds.

A candidate has five seconds per initialization call and five seconds after
view retrieval to publish all required initial sources. Actions have five-second
module deadlines. Client waits include queue time and expire after six seconds.
Accepted actions are not replayed; disconnection can leave completion uncertain.

Retiring generations drain accepted calls for up to six seconds, then allow
one second for shutdown response and one for exit before forced termination.
Daemon shutdown is bounded and stops every generation, including retired ones.

Optional stale_after_ms is per-source publication freshness. Expired values
become stale without assuming the entire process is dead. Fresh valid data
restores availability. Raw modules configure a freshness policy appropriate to
their publication schedule; there is no invented heartbeat.

## State and native presentation

The initial client snapshot contains every installed module, including disabled,
dormant, starting, or failed modules. Source-only updates carry complete source
maps without dashboard trees. Metadata, membership, and generation changes carry
a new session snapshot. Updates can coalesce safely; reconnect retrieves current
state without asking modules to republish.

The Swift client validates a complete update before applying it on the main actor.
Its socket worker uses bounded ordered events on dedicated I/O queues. Selection,
view identity, and pending action identities are scoped by module. In-flight
completion from a prior generation is reported without retrying the action.

The renderer supports stacks, cards, text, metrics, status, buttons, and dividers.
Unknown components render a fallback. Unknown, stale, and missing data never
silently appear live and clear. No executable expressions or WebViews exist.

The menu bar aggregates status components from all active modules. Highest
severity wins; unknown monitoring stays separate. Known stale warnings remain
visible with a question-mark badge, while stale clear cannot produce green.
Replacement failures and discovery problems contribute notices. Disabled and
dormant modules are excluded.

The popover remains available when the dashboard closes. The Dock icon or Open
Spiritglass reopens the existing window. Show Status and Open Dashboard also
have keyboard commands.

## Integration and tooling

The independent project monitor runs Git inspection on a background worker with
bounded command time/output and a coalesced refresh channel. It only reads local
state, disables optional Git index writes and external fsmonitor integration,
and never fetches. Inspection failures produce Unknown with cached values.

The example installer uses atomic sibling-file replacement and skips unchanged
binaries/manifests. scripts/control.py provides headless module management;
module_harness.py speaks the unchanged module v1 protocol directly.

The Swift package builds without an Xcode project. Packaging creates standard
Retina/non-Retina icon sizes, an .icns resource, and an ad hoc signed .app.
The build refreshes local app registration; launch explicitly updates the Dock
icon. Sources and the generation prompt are in apps/macos/Resources.

## Remaining boundaries

State and diagnostics are in memory across frontend reconnects and module
restarts, not supervisor restarts. Persisted alerts, acknowledgment/snoozing,
notifications, and SQLite retention remain the attention milestone. A launchd
service, untrusted-module sandboxing, remote access, async SDK publishers,
editable controls, and broader widgets remain outside this implementation.
