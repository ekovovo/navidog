"""Check that a module split moved code only.

Usage: python3 verify_move.py <old file> <new module dir> <Type> [<Type> ...]

Compares the non-blank lines of the old file with all `.rs` files in the new directory, as
multisets, after dropping what a split adds: module doc lines, `use super::*;`, `mod x;`,
`use child::Type;`, `impl <Type> {` wrappers, lone `}`, and `pub(super)` on functions, structs and
fields. Prints the lines found on one side only; signatures that rustfmt rewrapped after
`pub(super)` made them longer are the only expected differences.
"""
import collections
import glob
import re
import sys

old_path, new_dir, *types = sys.argv[1:]
wrappers = {f"impl {t} {{" for t in types}


def norm(path):
    out = collections.Counter()
    for line in open(path).read().split("\n"):
        s = line.rstrip()
        if not s.strip() or s.startswith("//! ") or s in wrappers or s in ("use super::*;", "}"):
            continue
        if re.match(r"^mod \w+;$", s) or re.match(r"^use \w+::\w+;$", s):
            continue
        s = re.sub(r"pub\(super\) (fn|struct) ", r"\1 ", s)
        s = re.sub(r"^    pub\(super\) (\w+):", r"    \1:", s)
        out[s] += 1
    return out


old = norm(old_path)
new = collections.Counter()
for path in sorted(glob.glob(f"{new_dir}/*.rs")):
    new += norm(path)
only_old, only_new = old - new, new - old
print("only in old:", sorted(only_old.elements()))
print("only in new:", sorted(only_new.elements()))
sys.exit(0 if not only_old and not only_new else 1)
