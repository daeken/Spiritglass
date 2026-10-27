# Module development and deployment

## Discovery and identity

The supervisor watches ~/.spiritglass/modules by default. Pass --modules-dir
for another installation. It reads module.json inside each immediate child
directory, plus flat *.json manifests at the root. It supports up to 32 modules.

Each manifest declares a unique, stable ID. module.describe must return the
same ID. Dashboard widget and source IDs are scoped to that module; actions
carry an explicit module_id on the frontend connection.

A manifest contains id, name, executable, and optional args, working_directory,
environment, lifecycle, enabled, watch, and stale_after_ms. Paths are resolved
relative to the manifest folder. Unknown fields, duplicate IDs, and invalid
manifests produce visible installation diagnostics. Invalid edits retain an
already-running module rather than removing it.

The executable is an explicit path, not a shell command. For Python or .NET,
specify the interpreter's absolute path, put the script or DLL in args, and
include that file in watch. Modules inherit the supervisor's environment, with
manifest environment entries applied on top. These are trusted local processes.

## Development loop

1. Build the module independently.
2. Copy the new executable to a temporary sibling of the deployed executable.
3. Preserve executable permissions, then atomically rename it over the deployed file.
4. Wait for the replacement to become live, or inspect its failure in the dashboard.

The supervisor polls every 500 ms and waits for an unchanged file fingerprint
across two observations before automatically starting a candidate. Fingerprints
include file identity, size, modification time, and mode for the executable and
all extra watch paths. Explicit Reload starts validation immediately.

The example installer implements atomic deployment and skips unchanged files:

```sh
cargo build -p spiritglass-example
python3 scripts/install-examples.py
```

It never rebuilds the app or supervisor, and refuses to overwrite a folder
whose manifest belongs to a different module.

## Replacement contract

The currently active process continues publishing and handling actions while
the candidate negotiates module protocol version 1. The candidate must provide:

- A valid declaration with the expected module ID.
- A valid view, including unique widget IDs and schema-compatible bindings.
- An initial valid value for every declared source, within five seconds of view retrieval.
- Initial values inside the configured freshness budget, if one is specified.
- A snapshot within the shared session payload budget.

Candidate values remain private until all checks pass. Promotion switches the
latest snapshot and the route for new actions together. The previous process
then drains actions accepted before promotion and shuts down. Candidate failure
preserves the original process, snapshot, and routing. Its diagnostic stays
visible until a successful replacement clears it.

Calls accepted before promotion complete on the original process and return that
generation number. Their old source values cannot overwrite the new generation.
Retirement allows up to six seconds for accepted calls, then graceful shutdown.
Queued calls whose response has already expired are skipped. Actions are never
replayed automatically, and an interrupted action can have unknown completion.

A replacement starts with its own business state. Modules that need continuity
must persist or recover that state themselves. The example counter intentionally
resets when replaced; frontend presentation state and connection remain intact.

An executable removed, left incomplete, or replaced by an invalid build does not
cause the supervisor to terminate a healthy process. Atomic deployment is
essential: modifying an executable's existing inode in place can invalidate its
running memory mappings at the OS level, beyond the supervisor's control.

## Lifecycle and recovery

always modules start automatically. on_demand modules remain dormant until
selected or activated, then keep running until explicitly deactivated. There is
no automatic idle timeout. Disabled modules do not start or contribute monitoring.

Failures retain source values marked stale and restart with exponential backoff,
from 250 ms to a 30-second maximum. A process that has stayed healthy for 30
seconds resets the failure streak. Explicit reload bypasses backoff. One module's
crash never restarts unrelated modules.

Removing a valid installed manifest stops and removes its module. An unreadable
installation directory does not uninstall everything; the registry keeps the
current installation and reports the discovery error.

Optional stale_after_ms (1 second to 24 hours) applies per source publication.
Quiet sources become stale while the process can remain alive. A fresh valid
publication restores availability. Modules choose this policy to match their
publication frequency; no implicit heartbeat is invented for raw-protocol modules.

Diagnostics retain the latest eight bounded entries per module, including
generation changes, failures, and rate-limited stderr. Discovery errors are shown
separately. These are in memory; persistent history and alerts are later work.

## Project watch

The project monitor takes a repository path as its first argument. It inspects
local Git data on a background worker with bounded command time and output.
Refresh actions request another inspection without blocking the protocol loop.
It never fetches, pushes, stages, commits, or updates the index.

Ahead/behind reflect the last fetched upstream. Missing upstream tracking is
reported explicitly. Local changes, unpushed commits, or a behind branch produce
Notice. Inspection failures produce Unknown and retain cached values. This is a
project status display, not a persisted alert system.

## Controls and limits

```sh
python3 scripts/control.py list
python3 scripts/control.py reload MODULE_ID
python3 scripts/control.py activate MODULE_ID
python3 scripts/control.py deactivate MODULE_ID
python3 scripts/control.py action MODULE_ID ACTION_ID --arguments '{}'
```

There are at most 32 installed modules, three process generations per installation
slot across the registry, 32 queued actions per generation, 64 client connections,
and 1 MiB per protocol frame. Session source/view data reserves 128 KiB of that
budget for bounded health and diagnostic growth. Oversized candidate or source
updates are rejected without damaging another module's state.
