# Spiritglass wire protocols

Module stdio uses version **1**. The frontend/supervisor socket uses version **2**.
The envelopes and module SDK remain compatible with the original release.

## Framing and envelopes

Each line is one UTF-8 JSON-RPC 2.0 object, without pretty printing. Maximum line
size is 1,048,576 bytes excluding LF. Stdout is exclusively protocol traffic;
stderr is for logs. Arrays of batch requests are unsupported. IDs are unsigned
64-bit integers or strings. Requests use non-null IDs. Notifications omit IDs.
Request/notification parameters may be omitted (meaning `{}`), but all current
methods use named object parameters. Extra fields in method payloads are rejected.

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol_versions":[1]}}
{"jsonrpc":"2.0","id":1,"result":{"protocol_version":1}}
```

Success contains `result`, including `null` where applicable. Failure contains
`error: {"code": integer, "message": string}`. A response cannot contain both.
Malformed envelopes use a null error response ID when no valid request ID exists.
There are no cancellation, binary transfer, or module-initiated host APIs yet.

[`protocol/v1.schema.json`](../protocol/v1.schema.json) contains `$defs` for every
envelope, method parameter/result, dashboard node, and data declaration. The root
is a catalog, not a wire message: validate an envelope against `$defs/Message`,
then validate its method-specific payload. Runtime semantic checks additionally
verify binding paths/types, uniqueness, version support, and size/complexity limits.

The multi-module client and manifest schema is protocol/v2.schema.json. The v1
catalog also retains the historical single-module client shapes for reference.

## Supervisor → module

The supervisor performs these requests in order:

| Method | Parameters | Result |
| --- | --- | --- |
| `initialize` | `{"protocol_versions":[1]}` | `{"protocol_version":1}` |
| `module.describe` | `{}` | `ModuleDescription` |
| `view.get` | `{"view_id":"main"}` | `Widget` |
| `action.invoke` | `{"action":"increment","arguments":{}}` | Module-defined JSON value |
| `shutdown` | `{}` | `null`, then exit |

Initialize once per connection. A compatible version must be selected from those
offered. `module.describe` returns `id`, `name`, `description`, `view_id`, `sources`,
and `actions`. IDs are 1–128 ASCII alphanumerics, dots, underscores, or hyphens.
IDs must be unique within their respective declaration or view namespace.

```json
{
  "id":"dev.example.counter",
  "name":"Counter",
  "description":"An independent module",
  "view_id":"main",
  "sources":[{"id":"overview","schema":{"type":"object","properties":{"count":{"type":"integer"}}}}],
  "actions":[{"id":"increment","arguments":{"type":"object","properties":{}}}]
}
```

`DataType` supports string, number, integer, boolean, array (`items`), and object
(`properties`). All object properties are required, and additional properties
are rejected. No implicit coercions occur. Schema validation supports nesting to
32 levels. Nullability, unions, and optional object fields are future extensions.

The module begins publishing after responding to `view.get`:

```json
{"jsonrpc":"2.0","method":"state.publish","params":{"source":"overview","value":{"count":1}}}
```

Every publication is a complete source value, not a JSON Patch. The supervisor
validates it before replacing the retained value. The aggregate snapshot must
also fit the message limit. For action completion, publish new source state before
responding to the action. Multiple source publications are not an atomic transaction.

## Dashboard definitions

Widgets are tagged by `type`, have stable `id`s, and are restricted to 1,000 nodes
and depth 32. The first renderer supports:

| Type | Fields beyond `id` and `type` |
| --- | --- |
| `v_stack`, `h_stack` | `children: Widget[]` |
| `card` | `title: string`, `children: Widget[]` |
| `text` | `value: Property<string>` |
| `metric` | `label: string`, `value: Property<number>`, optional `unit: string` |
| `status` | `label: string`, `value: Property<Status>` |
| `button` | `label: string`, `action: string`, `arguments: JSON` |
| `divider` | — |

`Status` is `clear`, `notice`, `urgent`, `emergency`, or `unknown`. Unknown bound
strings display as unknown. Unavailable/stale sources never display as live clear.

Properties are either literals or bindings:

```json
{"type":"metric","id":"count","label":"Count","value":{"kind":"binding","source":"overview","path":["count"]},"unit":null}
```

`{"kind":"literal","value":123}` supplies a fixed value. Binding paths are arrays
of exact object keys or decimal array indices, relative to the declared source.
There is no expression evaluation, implicit dot syntax, or formatting language.
Bindings must exist in the declared schema and match the property's type. An
integer source can supply a numeric metric. Literal types are checked by decoding.

## Frontend ↔ supervisor (version 2)

The client offers {"protocol_versions":[2]} on initialize. Version 1 clients
receive an explicit compatibility error; version 1 modules continue working.

| Method | Direction | Parameters/result |
| --- | --- | --- |
| initialize | Client request | Protocol version 2 negotiation |
| session.subscribe | Client request | {} → complete SessionSnapshot |
| session.snapshot | Server notification | Complete session on membership/metadata/generation change |
| state.changed | Server notification | All module source maps, without trees |
| action.invoke | Client request | module_id, action, arguments → generation and value |
| module.reload | Client request | module_id → accepted |
| module.activate | Client request | module_id → accepted |
| module.deactivate | Client request | module_id → accepted |

A session snapshot contains protocol_version (2), modules (keyed by stable ID),
and discovery diagnostics. Each ManagedModule contains:

- id, name, lifecycle, enabled.
- phase: disabled, dormant, starting, ready, restarting, or unavailable.
- generation: the current process-generation identifier; 0 before first promotion.
- pid, restart_count, reloading, nullable reload_error, bounded history.
- state: the retained module Snapshot, with protocol_version 1, nullable module
  declaration and view, sources, health, and nullable diagnostic.

The module-state wrapper preserves the v1 data/view vocabulary. Each source has
value, status (available/stale), revision, and updated_at_ms. Revisions are scoped
to a generation; use generation plus revision to distinguish replacement data.
Generation identifiers are monotonic within one daemon, not business versions.

The shared [session fixture](../protocol/fixtures/session.snapshot.json) is a
complete real-process example decoded by both Rust and Swift tests.

Source-only notifications have this shape:

```json
{"jsonrpc":"2.0","method":"state.changed","params":{"modules":{"dev.example.counter":{"sources":{},"health":"ready","diagnostic":null}}}}
```

Each included source map is complete. All currently installed modules are present.
A client validates the module key set before atomically applying updates. A
state-only update never fetches, decodes, or replaces a view tree.

Explicitly addressed actions use:

```json
{"jsonrpc":"2.0","id":8,"method":"action.invoke","params":{"module_id":"dev.example.counter","action":"increment","arguments":{}}}
{"jsonrpc":"2.0","id":8,"result":{"generation":4,"active_generation":4,"value":{"count":1}}}
```

Action IDs can be identical in different modules. The supervisor validates the
active generation's action declaration. Calls accepted before promotion finish
on their original process; stale-generation publications cannot overwrite current
data. Completion reports its original generation and active_generation at response
time (null if no active process remains), even if the client's metadata update
has not arrived yet. No automatic replay occurs.

Lifecycle controls return {"accepted":true} when accepted, not when validation
or shutdown has finished. Watch snapshots for the resulting phase/generation.
Reload also activates an enabled on-demand module. Deactivate is valid only for
on-demand modules. Disabled modules reject controls until their manifest enables
them.

Subscribe returns a snapshot at a single point in the update sequence, followed
by later notifications. Re-subscription is allowed. Updates may coalesce;
revisions need not be consecutive. Reconnection restores current state without
requiring republication. All retained values are stale while disconnected.

## Manifests

ModuleManifest requires id, name, and executable. Optional fields are args,
working_directory, environment, lifecycle (always by default), enabled (true),
watch (additional deployed paths), and stale_after_ms. Paths are relative to the
manifest folder; there is no shell expansion. Unknown fields are rejected.

The manifest and module declaration IDs must match. Disabled/dormant modules are
visible but do not monitor. Source freshness is optional and must be between
1000 and 86400000 milliseconds. See [module management](modules.md) for discovery,
atomic deployment, recovery, and replacement semantics.

## Errors and deadlines

| Code | Meaning |
| --- | --- |
| `-32700` | Invalid JSON |
| `-32600` | Invalid envelope |
| `-32601` | Unsupported method |
| `-32602` | Invalid parameters, undeclared action, or wrong argument shape |
| `-32001` | No compatible protocol version |
| `-32002` | Invalid initialization/lifecycle order |
| `-32003` | Unavailable/busy module or uncertain action completion |

Module calls time out after five seconds. Client action waits have a six-second
limit including queue time. Queued requests can expire without being executed.
An accepted action may still complete after a client disconnect or timeout; never
automatically replay it. No exactly-once or cross-session deduplication is promised.

Graceful shutdown allows one second for the response and another for process exit,
then kills the process. A protocol failure marks sources stale and triggers
automatic recovery with backoff. Failed candidates preserve their live predecessor.
Persisted alerts remain a subsequent milestone.
