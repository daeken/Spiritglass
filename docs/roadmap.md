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

## 2. Reliability and module management

Add manifests, discovery, module addressing and navigation, always/on-demand
lifecycle, crash recovery with bounded backoff, executable watching, explicit reload,
and diagnostics history. Define initialization freshness and module heartbeat policy.

For replacement, start and validate a candidate while the old process continues.
Only promote a candidate after protocol, declarations, view, and initial required
sources are healthy. Route new actions to the promoted process, let already
accepted actions finish on their original process, and reject ambiguous retries.
Candidate state remains isolated until promotion. Failed builds and failed candidates
leave the old process operational. These semantics are planned, not implemented.

Acceptance: independently rebuilt modules can replace their running versions, and
crashes do not affect unrelated modules or the app.

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
