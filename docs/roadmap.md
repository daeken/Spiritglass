# Implementation milestones

## 1. End-to-end vertical slice — implemented

- Rust workspace with separate protocol, SDK, supervisor, and example.
- Versioned and validated JSON-RPC over module stdio and a private Unix socket.
- SwiftUI dashboard built from a module-defined native view tree.
- Live source updates, button actions, and an example status indicator.
- Spirit-box app icon and a persistent menu bar status lamp with a status popover.
- Retained snapshots on client reconnect and visibly stale data after failure.
- Generated schema, shared Rust/Swift wire fixture, module harness, integration tests.

Acceptance: the live counter and status change without rebuilding or restarting
the frontend. Closing the app leaves the supervisor and module running.

## 2. Reliability and module management — implemented

- Folder/flat manifests, discovery, and up to 32 independent modules.
- Module-addressed actions, dynamic native navigation, and all-module attention.
- Always/on-demand/disabled lifecycle, start/stop/reload controls, and headless tools.
- Crash recovery with bounded backoff and retained stale values.
- Deployed-file watching, private candidate validation, and safe hot replacement.
- Predictable in-flight action draining with generation-tagged completion.
- Optional per-source freshness and bounded diagnostic history.
- An independent read-only Git project monitor and atomic deployment tooling.
- Real-process tests for corrupt executables, malformed candidates, ID/binding
  errors, missing initial data, namespace isolation, recovery, and handover.

Acceptance: install two modules without changing the frontend or supervisor;
rebuild and replace either with the app open; a bad candidate or crash leaves
unrelated modules operational. Business state continuity belongs to each module.

On-demand modules stop explicitly; idle scheduling is not implemented. State
and diagnostics remain in memory until the supervisor exits.

## 3. Attention system

The native menu bar presentation is already implemented for dashboard status
components, including worst-severity aggregation and unknown/stale state.

Add alert upsert/resolve, stable identities, SQLite retention, clear/notice/urgent/
emergency aggregation, unknown/stale monitoring state, acknowledgment, snoozing,
transition deduplication, alert-backed menu bar presentation, and native notifications.

Acceptance: an alert changes the menu bar, survives module restart, and resolves
without restarting the app. Acknowledgment and snoozing do not imply resolution.

## 4. Extensibility

Add typed field-path helpers, two-way controls with local editing state, remaining
layout and collection widgets, Swift Charts, conditional/formatting vocabularies,
an asynchronous SDK publisher, and an independently built C# example.

Acceptance: installing the C# module adds a native dashboard without rebuilding
the supervisor or app.
