# Spiritglass protocol version 1

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

## Frontend ↔ supervisor

| Method | Direction | Parameters/result |
| --- | --- | --- |
| `initialize` | Client request | Same version negotiation as modules |
| `session.subscribe` | Client request | `{}` → complete `Snapshot` |
| `session.snapshot` | Server notification | Complete `Snapshot` when declarations/view change |
| `state.changed` | Server notification | Complete `sources`, `health`, `diagnostic`; no tree |
| `action.invoke` | Client request | Same action payload/result as the module |

`Snapshot` contains `protocol_version`, nullable `module` and `view`, `sources`,
`health`, and nullable `diagnostic`. Health is `starting`, `ready`, or `unavailable`.
Each source entry contains `value`, `status` (`available` or `stale`), `revision`,
and `updated_at_ms` (Unix epoch milliseconds).

Subscribe returns the snapshot at a single point in the update sequence; later
updates follow that response. Re-subscription is allowed. Updates can coalesce,
so revisions need not be consecutive. No update history is guaranteed. Clients
must treat all retained values as stale while disconnected and retrieve a new
snapshot after reconnecting. An individual source without a published value is
loading during startup, otherwise unavailable. State survives client disconnects
and module failure, but not supervisor termination.

The first slice supports one module, so action IDs are scoped to that module's
connection. Multiple-module addressing will need an explicit protocol extension.

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
then kills the process. A protocol failure marks sources stale and requires an
explicit supervisor restart in this first slice. Automatic recovery, replacement
handover semantics, and alerts are reserved for subsequent milestones.
