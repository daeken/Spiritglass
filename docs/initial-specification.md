# Spiritglass — Initial Project Specification

**Tagline:** A window into everything.

**Status:** Initial implementation specification / MVP

## 1. Overview

Spiritglass is a native macOS application that provides a universal, extensible control plane for monitoring and interacting with arbitrary applications, services, projects, and agents.

Its primary purpose is to consolidate information and actions from many independent systems into a single native interface, with an unobtrusive menu bar indicator showing whether anything requires attention.

Spiritglass consists of three components:

1. **Spiritglass.app:** A native SwiftUI frontend.
2. **spiritglassd:** A Rust supervisor daemon that manages modules, their lifecycle, and shared state.
3. **Modules:** Independent executables communicating with the supervisor through a standardized IPC protocol.

Modules may be written in any programming language capable of reading stdin and writing stdout.

The application should be **local-first, extensible, lightweight, and exceptionally easy for coding agents to extend**.

## 2. Architectural principles

- **Native first.** Dashboard interfaces are rendered dynamically using actual SwiftUI components, not HTML or WebViews.
- **Language agnostic.** Modules are independent processes, not libraries loaded into the supervisor.
- **Strongly typed.** Define a formal protocol schema and provide typed SDKs. Coding agents should receive compile-time errors for invalid interfaces whenever possible.
- **Hot reloadable.** Modules can be replaced or restarted without restarting the supervisor or frontend.
- **Fault tolerant.** A failing or malformed module must not crash the supervisor or frontend.
- **Reactive.** Modules publish state changes; the frontend updates subscribed bindings without requiring complete UI regeneration.
- **Composable.** Modules can expose dashboards, data sources, actions, and alerts through a common interface.
- **Minimal magic.** Favor understandable protocols and explicit lifecycle management over complicated abstractions.

## 3. Components

### 3.1 Spiritglass.app — SwiftUI frontend

Responsibilities:

- Native macOS window with module navigation sidebar.
- Dynamic rendering of module-defined dashboards.
- Local reactive data binding and interaction state.
- Native menus, inputs, tables, charts, navigation, and other controls.
- Menu bar presence and global attention indicator.
- macOS notifications.
- Connection management with `spiritglassd`.
- An overview page aggregating information from installed modules.

The frontend should know nothing about individual modules or their implementations.

**Dynamic rendering:** Implement a declarative, recursive view-tree representation. The frontend receives a serialized view definition and constructs native SwiftUI components through a renderer.

Initial widget types:

- Layout: horizontal stack, vertical stack, grid, scroll container, spacer.
- Display: text, icon, metric, status indicator, progress, divider.
- Collections: list, table.
- Visualization: simple charts using Swift Charts.
- Interaction: button, toggle, text input, picker.
- Composition: card, section, tabs.

Each widget should have a stable ID and support relevant configuration, bindings, and actions.

Rendering should degrade gracefully when encountering unsupported widget types.

Do not implement arbitrary executable UI expressions. Conditional display and formatting should use explicitly defined, typed mechanisms.

A WebView component can be introduced later as an escape hatch, but is not part of the initial implementation.

### 3.2 spiritglassd — Rust supervisor

Responsibilities:

- Discover installed modules from manifests.
- Spawn and supervise module processes.
- Manage IPC and protocol negotiation.
- Track module lifecycle and health.
- Route actions to modules.
- Maintain latest published data source values.
- Distribute state changes to frontend subscribers.
- Aggregate alerts and global attention state.
- Persist relevant state using SQLite.
- Manage module restart and hot replacement.
- Provide diagnostics for failed modules.

The supervisor should run independently of the frontend process, allowing the frontend to disconnect and reconnect without requiring modules to restart.

For MVP, the frontend may launch the supervisor when necessary. Support for launchd-managed startup can follow.

The frontend and supervisor should communicate over a local Unix domain socket, using a clearly versioned protocol.

Do not expose an unauthenticated TCP server.

### 3.3 Modules — Independent executables

Each module is a standalone executable launched by the supervisor.

A module can:

- Declare its identity, capabilities, and dashboard views.
- Register typed data sources.
- Publish snapshots and incremental state changes.
- Define and handle actions.
- Create, update, and resolve alerts.
- Request supported host capabilities through the protocol.

A module must not need to know anything about SwiftUI or the supervisor's implementation language.

Modules should support lifecycle policies such as `always` and `on-demand`. Additional scheduling policies may be added later.

Modules are trusted local executables for MVP. OS-level sandboxing and detailed permission management are out of scope.

## 4. Module protocol

Use **JSON-RPC 2.0 over newline-delimited JSON on stdin/stdout** for the initial protocol.

- Stdout is reserved exclusively for protocol messages.
- Stderr is available for logging and diagnostics.
- Messages support asynchronous requests, responses, and notifications.
- Every protocol interaction must be validated.
- Protocol versions must support compatibility negotiation.
- Large payloads and binary transfer mechanisms are deferred.

Define a canonical, machine-readable schema for the wire protocol and UI definitions.

Initial interactions:

| Method | Purpose |
|---|---|
| `initialize` | Supervisor initializes module and negotiates protocol |
| `module.describe` | Retrieve metadata and capabilities |
| `view.get` | Retrieve a declarative dashboard tree |
| `state.publish` | Publish source state changes |
| `action.invoke` | Invoke an action with arguments |
| `alert.upsert` | Create or update an alert |
| `alert.resolve` | Resolve an existing alert |
| `shutdown` | Request graceful module termination |

Exact message shapes are implementation decisions, but must be documented and tested.

Use stable module IDs, source IDs, widget IDs, action IDs, and alert IDs.

The frontend/supervisor connection should support initial snapshots followed by incremental updates. Reconnection must restore the current state without requiring modules to republish everything.

## 5. Data binding

Modules should declare named data sources with associated schemas.

UI widgets can bind their properties to values from these sources.

For example, a ScoutGhost metric widget might bind its displayed value to `overview.activeAlliances`.

Requirements:

- Support typed primitives, arrays, and structured objects.
- Validate bindings against source schemas where possible.
- Support reactive updates without rebuilding the whole view tree.
- Preserve local UI state when unrelated data changes.
- Support two-way input bindings and explicit action invocation.
- Display loading, unavailable, stale, and error states.

The native client owns presentation state such as selection, scrolling, expansion, and editing.

Modules own business data and business logic.

The SDK should make invalid bindings difficult to express and should provide useful compiler diagnostics.

## 6. Alerts and attention management

Spiritglass has four attention severities:

| Severity | Indicator | Meaning |
|---|---|---|
| Clear | Green | Everything is operating normally |
| Notice | Yellow | Something needs attention eventually |
| Urgent | Red | Something needs attention soon |
| Emergency | Black | Immediate attention required |

Unknown or stale monitoring state must be represented separately, rather than silently appearing green.

Alerts should have stable identities, severity, title, description, timestamps, optional associated actions, and resolution state.

The supervisor should:

- Persist active alerts across module restarts.
- Deduplicate updates to existing alerts.
- Calculate global attention state from active alerts.
- Emit notification events for meaningful transitions.
- Support acknowledgment and snoozing without confusing them with resolution.
- Avoid repeatedly notifying about unchanged conditions.

The native application displays the global indicator in the menu bar and provides a popover listing the issues contributing to it.

Emergency state should be visually distinctive even on dark menu bars.

The supervisor should retain enough information to show the last known state of an unavailable module, clearly marked stale.

## 7. Module lifecycle and hot replacement

The supervisor discovers modules through manifests containing their identity, executable path, arguments, and lifecycle configuration.

For MVP, support:

- Module launch and initialization.
- Graceful shutdown.
- Crash detection and restart with backoff.
- Disabled and unavailable module states.
- Explicit module reload.
- Automatic reload when a deployed executable changes.

**Replacement behavior:**

1. Keep the existing module running.
2. Start the replacement process.
3. Validate its protocol and initialization.
4. Once healthy, switch routing and subscriptions.
5. Gracefully terminate the previous process.

Failed replacement must leave the previous working process operational.

State changes and in-flight actions should be handled predictably during a replacement. Document the chosen semantics.

A failed build should never trigger removal of a healthy running module.

## 8. SDKs and developer experience

The protocol must remain usable without an SDK.

Build a small Rust SDK first, using Serde and typed builders for module declarations, UI trees, data sources, and action handlers.

Demonstrate interoperability with a second language, preferably C#, even if its initial implementation uses the raw protocol.

TypeScript SDK support can follow.

The project should provide:

- Formal protocol schemas.
- Clear module implementation examples.
- Validation and compatibility tests.
- Useful supervisor diagnostics.
- A straightforward module development loop.
- A test harness capable of exercising modules without running the macOS frontend.

Avoid unnecessary build-system coupling. Modules should compile and deploy independently.

## 9. Initial repository structure

Suggested layout:

- `apps/macos/` — Spiritglass SwiftUI application.
- `crates/spiritglassd/` — Rust supervisor.
- `crates/spiritglass-protocol/` — Protocol models and validation.
- `crates/spiritglass-sdk/` — Rust module SDK.
- `protocol/` — Language-neutral schema definitions.
- `examples/rust-module/` — Example Rust module.
- `examples/dotnet-module/` — Example C# module.
- `docs/` — Architecture, protocol, and module authoring guides.

The implementation may adjust this structure when justified.

## 10. MVP implementation plan

### Phase 1 — End-to-end vertical slice

Implement:

- Rust supervisor launching a single Rust module.
- JSON-RPC communication over stdio.
- SwiftUI frontend connecting to the supervisor.
- Runtime-defined native dashboard.
- One reactive data source.
- One button invoking a module action.
- One live status indicator.

**Acceptance:** Updating the module's state changes a displayed native SwiftUI widget without rebuilding or restarting the frontend.

### Phase 2 — Reliability and module management

Implement module manifests, discovery, multiple modules, graceful shutdown, crash recovery, reconnection, diagnostics, and hot replacement.

**Acceptance:** A module can be rebuilt and replaced while the application remains open; crashes do not affect unrelated modules.

### Phase 3 — Attention system

Implement alert state, SQLite persistence, attention aggregation, menu bar visualization, and macOS notifications.

**Acceptance:** An alert published by a module updates the menu bar, survives a module restart, and can be resolved without restarting the application.

### Phase 4 — Extensibility

Expand the widget library, introduce typed binding helpers, implement a C# sample module, and document module creation.

**Acceptance:** An independently built C# module can be installed without changing or recompiling the frontend or supervisor.

## 11. Explicit non-goals for MVP

- Remote access or cloud synchronization.
- Multi-user authentication.
- Untrusted module sandboxing.
- Plugin marketplaces or automatic downloads.
- Embedding arbitrary SwiftUI code from modules.
- Complex visual dashboard editors.
- A comprehensive custom styling language.
- Arbitrary web application embedding.
- Building every possible widget before the first functioning dashboard.

## 12. Implementation guidance for coding agents

**Prioritize working software and a clean protocol over framework completeness.**

Start with the smallest possible end-to-end implementation and expand iteratively.

Keep protocol definitions, module SDKs, supervisor logic, and native rendering cleanly separated.

Write tests for protocol compatibility, module restarts, bad messages, and reconnect behavior.

Make reasonable engineering decisions autonomously. Document important tradeoffs rather than blocking on minor uncertainties.

The end goal is an application where an agent can create a standalone module, compile it, install it, and immediately have new native dashboards and capabilities appear in Spiritglass—without touching the SwiftUI frontend.

**Spiritglass should be boring infrastructure enabling extraordinarily interesting integrations.**