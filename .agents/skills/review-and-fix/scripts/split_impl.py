"""Split a Rust module file into <name>/mod.rs plus child modules, moving code only.

Usage (from the repository root): python3 split_impl.py <spec.json>

spec:
  {"source": "crates/.../viewer.rs",          # becomes crates/.../viewer/mod.rs (git mv)
   "type": "TableViewer",                      # methods are taken from `impl <type> {` blocks
   "modules": {
     "<child>": {"doc": "module doc line",
                 "methods": ["name", ...],     # moved into `impl <type>` in the child, made pub(super)
                 "items": [["start line", "end line"], ...]}}}  # top-level code moved verbatim

A method spans from its signature (with the comments and attributes right above it) to the first
line that is exactly four spaces and `}`, which in rustfmt output can only be the method's end.
An item spans from the first line equal to the start marker to the next line equal to the end
marker, plus the `///` and `#[` lines right above it.
"""
import json
import os
import re
import subprocess
import sys

spec = json.load(open(sys.argv[1]))
src_path = spec["source"]
ty = spec["type"]
if not os.path.exists(src_path):
    sys.exit(f"{src_path} not found; run from the repository root")
lines = open(src_path).read().split("\n")


def method_spans():
    spans = {}
    in_impl = False
    i = 0
    while i < len(lines):
        line = lines[i]
        if line == f"impl {ty} {{":
            in_impl = True
        elif in_impl and line == "}":
            in_impl = False
        m = re.match(r"^    (pub(\([a-z]+\))? )?fn (\w+)", line)
        if in_impl and m:
            name = m.group(3)
            start = i
            while start > 0 and re.match(r"^    (//|#\[)", lines[start - 1]):
                start -= 1
            end = i
            while lines[end] != "    }":
                end += 1
            if name in spans:
                sys.exit(f"method {name} is defined twice")
            spans[name] = (start, end)
            i = end
        i += 1
    return spans


spans = method_spans()
removed = set()
outputs = {}
for module, cfg in spec["modules"].items():
    chunks = []
    for name in cfg.get("methods", []):
        if name not in spans:
            sys.exit(f"no method {name} in impl {ty}")
        start, end = spans[name]
        body = lines[start : end + 1]
        for k, line in enumerate(body):
            if re.match(r"^    fn \w+", line):
                body[k] = line.replace("    fn ", "    pub(super) fn ", 1)
                break
        chunks.append("\n".join(body))
        removed.update(range(start, end + 1))
    items = []
    for start_marker, end_marker in cfg.get("items", []):
        if start_marker not in lines:
            sys.exit(f"no line {start_marker!r}")
        start = lines.index(start_marker)
        end = start
        while lines[end] != end_marker:
            end += 1
        while start > 0 and re.match(r"^(///|#\[)", lines[start - 1]):
            start -= 1
        items.append("\n".join(lines[start : end + 1]))
        removed.update(range(start, end + 1))
    text = f"//! {cfg['doc']}\n\nuse super::*;\n"
    if chunks:
        text += f"\nimpl {ty} {{\n" + "\n\n".join(chunks) + "\n}\n"
    for item in items:
        text += "\n" + item + "\n"
    outputs[module] = text

kept = "\n".join(line for k, line in enumerate(lines) if k not in removed)
# Impl blocks whose methods all moved
kept = re.sub(rf"\nimpl {ty} \{{\n\s*\}}\n", "\n", kept)
# Declare the children after the last top-level `use`
parts = kept.split("\n")
idx = max(i for i, line in enumerate(parts) if line.startswith("use "))
while not parts[idx].rstrip().endswith(";"):
    idx += 1
decls = "\n".join(f"mod {m};" for m in sorted(spec["modules"]))
parts.insert(idx + 1, "\n" + decls)

base = os.path.splitext(src_path)[0]
os.makedirs(base, exist_ok=True)
subprocess.run(["git", "mv", src_path, os.path.join(base, "mod.rs")], check=True)
open(os.path.join(base, "mod.rs"), "w").write("\n".join(parts))
for module, text in outputs.items():
    open(os.path.join(base, f"{module}.rs"), "w").write(text)
print(f"moved {len(removed)} lines into {', '.join(outputs)}; now run cargo fmt --all")
