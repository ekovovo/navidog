#!/usr/bin/env bash
# Run the whole workspace and keep going after a failure, then publish each
# panic as a GitHub Actions annotation so the summary lists every one.
set -uo pipefail

log="$(mktemp)"
cargo test --workspace --no-fail-fast --color never >"$log" 2>&1
status=$?
cat "$log"

python3 - "$log" <<'PY'
import re
import sys

lines = open(sys.argv[1], errors="replace").read().splitlines()
pat = re.compile(r"panicked at ([^:\s]+):(\d+):(\d+)")
for index, line in enumerate(lines):
    match = pat.search(line)
    if not match:
        continue
    extra = []
    for nxt in lines[index + 1 : index + 5]:
        if nxt.startswith(("note:", "failures:", "thread ")):
            break
        stripped = nxt.strip()
        if stripped:
            extra.append(stripped)
    message = " | ".join(extra) or "test panicked"
    for old, new in (("%", "%25"), ("\r", ""), ("\n", "%0A")):
        message = message.replace(old, new)
    file, line_no, col = match.group(1), match.group(2), match.group(3)
    print(f"::error file={file},line={line_no},col={col}::{message}", flush=True)
PY

rm -f "$log"
exit "$status"
