#!/bin/bash
# Rebuild the rolling nightly: a workflow_dispatch of publish.yml on main.
# Pushes to main publish the nightly automatically; use this to rebuild the
# current main without a new commit.
#
#   scripts/nightly.sh            ask, then start the run
#   scripts/nightly.sh --watch    follow it to the end
#   scripts/nightly.sh --yes      no confirmation
#   scripts/nightly.sh --force    continue past a failed check
set -euo pipefail
cd "$(dirname "$0")/.."

WORKFLOW=publish.yml
watch=false
assume_yes=false
force=false

for arg in "$@"; do
  case "$arg" in
    --watch) watch=true ;;
    --yes | -y) assume_yes=true ;;
    --force) force=true ;;
    -h | --help)
      sed -n '2,9p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "unknown argument: $arg (see --help)" >&2
      exit 2
      ;;
  esac
done

problem() {
  echo "✗ $1" >&2
  if [ "$force" = true ]; then
    echo "  --force: continuing anyway" >&2
  else
    echo "  (--force to go ahead regardless)" >&2
    exit 1
  fi
}

command -v gh >/dev/null 2>&1 || { echo "gh (GitHub CLI) is required" >&2; exit 1; }
gh auth status >/dev/null 2>&1 || { echo "gh is not signed in: gh auth login" >&2; exit 1; }

git fetch --quiet origin main
remote_sha=$(git rev-parse origin/main)
echo "nightly builds origin/main: $(git log -1 --format='%h %s' "$remote_sha")"

ahead=$(git rev-list --count origin/main..HEAD 2>/dev/null || echo 0)
dirty=$(git status --porcelain | wc -l | tr -d ' ')
if [ "$ahead" != 0 ]; then
  echo "! $ahead local commit(s) are not pushed — they will not be in this nightly:"
  git log --format='    %h %s' origin/main..HEAD
fi
if [ "$dirty" != 0 ]; then
  echo "! $dirty uncommitted file(s) — they will not be in this nightly:"
  git status --porcelain | sed 's/^/    /'
fi

running=$(gh run list --workflow "$WORKFLOW" --json status \
  -q '[.[] | select(.status == "in_progress" or .status == "queued")] | length')
if [ "$running" != 0 ]; then
  problem "$running publish run(s) already in progress or queued"
fi

if [ "$assume_yes" != true ]; then
  printf "Trigger the nightly? [y/N] "
  read -r answer
  case "$answer" in
    y | Y | yes) ;;
    *)
      echo "not triggered"
      exit 0
      ;;
  esac
fi

latest() {
  gh run list --workflow "$WORKFLOW" --event workflow_dispatch --limit 1 \
    --json databaseId -q '.[0].databaseId // 0'
}
before=$(latest)
gh workflow run "$WORKFLOW" --ref main

run_id=$before
for _ in $(seq 1 30); do
  sleep 2
  run_id=$(latest)
  [ "$run_id" != "$before" ] && break
done
if [ "$run_id" = "$before" ]; then
  echo "triggered, but the run has not shown up yet: gh run list --workflow $WORKFLOW"
  exit 0
fi

url=$(gh run view "$run_id" --json url -q .url)
echo "started: $url"
if [ "$watch" = true ]; then
  gh run watch "$run_id" --exit-status
fi
