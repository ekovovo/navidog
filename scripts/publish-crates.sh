#!/usr/bin/env bash
# Publish workspace crates to crates.io in dependency order.
#
#   scripts/publish-crates.sh                 # dry-run every publishable crate
#   scripts/publish-crates.sh --execute       # upload
#   scripts/publish-crates.sh --plan          # print order and exit
#   scripts/publish-crates.sh navidog-util    # one crate (still a dry-run)
#
# Crates with `publish = false` are skipped (xtask). A version already on
# crates.io is skipped. `cargo publish` waits for the index after each upload
# before this script starts the next crate.
#
# A full dry-run stops at the first crate whose workspace dependencies are not
# on crates.io yet. `--execute` publishes those first. `--no-verify` only
# checks that each package can be packed.
#
# Workspace path dependencies must set `version` (see the root Cargo.toml).
# The [patch.crates-io] override is not part of the published crate, so the
# verify build uses the crates.io copy of those dependencies.
set -euo pipefail
cd "$(dirname "$0")/.."

mode=dry-run
no_verify=false
allow_dirty=false
self_check=false
selected=()

usage() {
  sed -n '2,/^set -euo pipefail$/p' "$0" | sed '$d; s/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --plan) mode=plan; shift ;;
    --dry-run) mode=dry-run; shift ;;
    --execute) mode=execute; shift ;;
    --no-verify) no_verify=true; shift ;;
    --allow-dirty) allow_dirty=true; shift ;;
    --self-check) self_check=true; shift ;;
    -h|--help) usage; exit 0 ;;
    --) shift; break ;;
    -*) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    *) selected+=("$1"); shift ;;
  esac
done

# The planner is embedded so --self-check exercises the same order logic.
# python3 -c keeps stdin free for `cargo metadata`.
plan_py() {
  cat <<'PY'
import json, os, sys

class PlanError(Exception):
    pass

def req_ok(req, version):
    req = req.strip()
    return req in (version, f"^{version}", f"={version}")

def allowed(publish):
    # null: crates.io. []: publish = false. A list names the allowed registries.
    return publish is None or "crates-io" in publish

def plan(packages, members, selected):
    ws = [p for p in packages if p["id"] in members]
    by_name = {p["name"]: p for p in ws}
    if selected:
        missing = [n for n in selected if n not in by_name]
        if missing:
            raise PlanError("unknown crate: " + ", ".join(missing))
        refused = [n for n in selected if not allowed(by_name[n].get("publish"))]
        if refused:
            raise PlanError("publish = false: " + ", ".join(refused))
        wanted = list(dict.fromkeys(selected))
    else:
        wanted = [p["name"] for p in ws if allowed(p.get("publish"))]
    if not wanted:
        raise PlanError("no crates to publish")
    wanted_set = set(wanted)

    deps = {}
    for name in wanted:
        internal = []
        for dep in by_name[name]["dependencies"]:
            if dep.get("kind") == "dev" or not dep.get("path"):
                continue
            dep_name = dep["name"]
            if dep_name in by_name:
                version = by_name[dep_name]["version"]
                if not req_ok(dep["req"], version):
                    raise PlanError(
                        f"{name} depends on {dep_name} {version} with requirement "
                        f"{dep['req']!r}; set version in workspace.dependencies"
                    )
                if dep_name in wanted_set:
                    internal.append(dep_name)
            elif dep["req"].strip() == "*":
                raise PlanError(f"{name} path dependency {dep_name} has no version")
        deps[name] = internal

    memo = {}

    def depth(name, stack):
        if name in memo:
            return memo[name]
        if name in stack:
            raise PlanError(f"dependency cycle involving {name}")
        others = deps[name]
        value = 0 if not others else 1 + max(depth(n, stack | {name}) for n in others)
        memo[name] = value
        return value

    ordered = sorted(wanted, key=lambda name: (depth(name, frozenset()), name))
    return [(name, by_name[name]["version"]) for name in ordered]

def pkg(name, publish, deps):
    dependencies = []
    for item in deps:
        dep_name, req = item[0], item[1]
        kind = item[2] if len(item) > 2 else None
        dependencies.append({"name": dep_name, "req": req, "kind": kind, "path": dep_name})
    return {
        "id": name,
        "name": name,
        "version": "0.1.0",
        "publish": publish,
        "dependencies": dependencies,
    }

def self_check():
    members = {"a", "b", "c"}
    packages = [
        pkg("a", None, []),
        pkg("b", None, [("a", "^0.1.0"), ("a", "*", "dev")]),
        pkg("c", [], []),
    ]
    assert plan(packages, members, []) == [("a", "0.1.0"), ("b", "0.1.0")]
    assert plan(packages, members, ["b"]) == [("b", "0.1.0")]
    try:
        plan(
            [pkg("a", None, []), pkg("b", None, [("a", "*")])],
            {"a", "b"},
            [],
        )
    except PlanError as error:
        assert "workspace.dependencies" in str(error), error
    else:
        raise PlanError("a '*' requirement should fail")
    print("ok")

def main():
    try:
        if os.environ.get("NAVIDOG_PUBLISH_SELF_CHECK") == "1":
            self_check()
            return
        selected = [s for s in os.environ.get("NAVIDOG_PUBLISH", "").split("\n") if s]
        meta = json.load(sys.stdin)
        for name, version in plan(meta["packages"], set(meta["workspace_members"]), selected):
            print(f"{name}\t{version}")
    except PlanError as error:
        print(error, file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()
PY
}

if [[ "$self_check" == true ]]; then
  NAVIDOG_PUBLISH_SELF_CHECK=1 python3 -c "$(plan_py)"
  exit 0
fi

names=""
if ((${#selected[@]} > 0)); then
  names=$(printf '%s\n' "${selected[@]}")
fi

order=$(
  export NAVIDOG_PUBLISH="$names"
  cargo metadata --format-version 1 --no-deps | python3 -c "$(plan_py)"
)

if [[ "$mode" == plan ]]; then
  printf '%s\n' "$order"
  exit 0
fi

cargo_args=()
if [[ "$mode" == dry-run ]]; then
  cargo_args+=(--dry-run)
fi
if [[ "$no_verify" == true ]]; then
  cargo_args+=(--no-verify)
fi
if [[ "$allow_dirty" == true ]]; then
  cargo_args+=(--allow-dirty)
fi

# 200: this version is already on crates.io. 404: not published yet.
on_crates_io() {
  local name=$1 version=$2 http
  http=$(curl -sS -o /dev/null -w '%{http_code}' \
    -A 'navidog-publish (https://github.com/xhofe/navidog)' \
    "https://crates.io/api/v1/crates/${name}/${version}")
  case "$http" in
    200) return 0 ;;
    404) return 1 ;;
    *) echo "crates.io lookup for ${name} ${version} returned HTTP ${http}" >&2; exit 1 ;;
  esac
}

while IFS=$'\t' read -r name version; do
  [[ -n "$name" ]] || continue
  if on_crates_io "$name" "$version"; then
    echo "==> ${name} ${version} already on crates.io, skip"
    continue
  fi
  if [[ "$mode" == execute ]]; then
    echo "==> publishing ${name} ${version}"
  else
    echo "==> dry-run ${name} ${version}"
  fi
  cargo publish -p "$name" "${cargo_args[@]+"${cargo_args[@]}"}"
done <<EOF
$order
EOF
