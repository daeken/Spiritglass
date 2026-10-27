#!/bin/bash
set -euo pipefail
project_dir="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --manifest-path "$project_dir/Cargo.toml" --workspace
"$project_dir/scripts/build-app.sh"
runtime_dir="$HOME/.spiritglass/run"
mkdir -p -m 700 "$runtime_dir"
chmod 700 "$runtime_dir"
export SPIRITGLASS_SOCKET="${SPIRITGLASS_SOCKET:-$runtime_dir/control.sock}"
log_path="$runtime_dir/demo.log"
if ! python3 - "$SPIRITGLASS_SOCKET" <<'PY'
import socket, sys
with socket.socket(socket.AF_UNIX) as connection:
    try: connection.connect(sys.argv[1])
    except OSError: sys.exit(1)
PY
then
    supervisor_pid="$(python3 - "$project_dir" "$SPIRITGLASS_SOCKET" "$log_path" <<'PY'
from pathlib import Path
import subprocess, sys
root = Path(sys.argv[1])
with open(sys.argv[3], 'w') as log:
    process = subprocess.Popen([str(root / 'target/debug/spiritglassd'), '--socket', sys.argv[2],
        '--module', str(root / 'target/debug/spiritglass-example')], stdin=subprocess.DEVNULL,
        stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
print(process.pid)
PY
)"
    if ! python3 - "$SPIRITGLASS_SOCKET" "$supervisor_pid" <<'PY'
import os, socket, sys, time
for _ in range(100):
    try:
        with socket.socket(socket.AF_UNIX) as connection:
            connection.connect(sys.argv[1])
        break
    except OSError:
        try: os.kill(int(sys.argv[2]), 0)
        except OSError: sys.exit(1)
        time.sleep(0.05)
else:
    sys.exit(1)
PY
    then
        cat "$log_path"
        exit 1
    fi
    printf 'Supervisor started (PID %s). Stop it with: kill %s\n' "$supervisor_pid" "$supervisor_pid"
else
    printf 'Using the supervisor already listening at %s\n' "$SPIRITGLASS_SOCKET"
fi
printf 'Supervisor log: %s\n' "$log_path"
open --env "SPIRITGLASS_SOCKET=$SPIRITGLASS_SOCKET" \
    --stdout "$runtime_dir/app.log" --stderr "$runtime_dir/app.log" \
    "$project_dir/.build/Spiritglass.app"
printf 'Spiritglass opened. Closing it leaves the supervisor running.\n'
