#!/usr/bin/env bash
# Create or update the pull request's single macOS preview comment.
#
# Usage: preview-comment.sh building|success|failure
#        preview-comment.sh --check
#
# building runs at the start of the workflow. success and failure replace that
# comment when the build job finishes. A newer push cancels the previous run;
# the cancelled run does not call this script, so it cannot overwrite the new
# comment.
#
# building and failure keep the last successful build. success records it on
# the second line as a hidden marker; later updates copy that line and show
# the fields again.
#
# Requires GH_TOKEN, GITHUB_REPOSITORY, PR_NUMBER, RUN_URL, RUN_ID, and
# HEAD_SHA. success also requires ARTIFACT and SHA256.
set -euo pipefail

marker="<!-- navidog-preview -->"
# ponytail: values are space-separated and unescaped. Run URLs, ids, SHAs, and
# the artifact name have no spaces; a record that fails the charset check is
# dropped. If a field ever needs spaces, switch this to JSON.
success_prefix="<!-- navidog-preview-success "

PREV_RUN_URL=""
PREV_RUN_ID=""
PREV_HEAD_SHA=""
PREV_ARTIFACT=""
PREV_SHA256=""

clear_previous() {
  PREV_RUN_URL=""
  PREV_RUN_ID=""
  PREV_HEAD_SHA=""
  PREV_ARTIFACT=""
  PREV_SHA256=""
}

have_previous() {
  [ -n "$PREV_RUN_URL" ] &&
    [ -n "$PREV_RUN_ID" ] &&
    [ -n "$PREV_HEAD_SHA" ] &&
    [ -n "$PREV_ARTIFACT" ] &&
    [ -n "$PREV_SHA256" ]
}

# Line 1 stays the comment marker. Line 2 is the hidden success record.
comment_header() {
  if [ -n "${1:-}" ]; then
    printf '%s\n%srun_url=%s run_id=%s head_sha=%s artifact=%s sha256=%s -->' \
      "$marker" "$success_prefix" "$1" "$2" "$3" "$4" "$5"
  else
    printf '%s' "$marker"
  fi
}

comment_header_for_previous() {
  if have_previous; then
    comment_header "$PREV_RUN_URL" "$PREV_RUN_ID" "$PREV_HEAD_SHA" \
      "$PREV_ARTIFACT" "$PREV_SHA256"
  else
    comment_header
  fi
}

commit_link() {
  local head="$1"
  local short="${head:0:7}"
  local repo="${GITHUB_REPOSITORY:?}"
  local commit_url="${GITHUB_SERVER_URL:-https://github.com}/${repo}/commit/${head}"
  printf '[`%s`](%s)' "$short" "$commit_url"
}

last_success_suffix() {
  have_previous || return 0
  local repo="${GITHUB_REPOSITORY:?}"
  local commit
  commit="$(commit_link "$PREV_HEAD_SHA")"
  cat <<EOF

### Last successful build

- **Run:** ${PREV_RUN_URL}
- **Commit:** ${commit}
- **Artifact:** \`${PREV_ARTIFACT}\` (expires in 7 days)
- **CLI:** \`gh run download ${PREV_RUN_ID} -R ${repo} -n ${PREV_ARTIFACT}\`
- **SHA-256:** \`${PREV_SHA256}\`
EOF
}

parse_marker_line() {
  local line="$1"
  local rest="${line#*"$success_prefix"}"
  rest="${rest% -->}"
  local pair key value
  clear_previous
  for pair in $rest; do
    key="${pair%%=*}"
    value="${pair#*=}"
    case "$key" in
      run_url) PREV_RUN_URL="$value" ;;
      run_id) PREV_RUN_ID="$value" ;;
      head_sha) PREV_HEAD_SHA="$value" ;;
      artifact) PREV_ARTIFACT="$value" ;;
      sha256) PREV_SHA256="$value" ;;
    esac
  done
  [[ "$PREV_RUN_URL" =~ ^https://[A-Za-z0-9./_-]+$ ]] || { clear_previous; return 1; }
  [[ "$PREV_RUN_ID" =~ ^[0-9]+$ ]] || { clear_previous; return 1; }
  [[ "$PREV_HEAD_SHA" =~ ^[0-9a-f]{40}$ ]] || { clear_previous; return 1; }
  [[ "$PREV_ARTIFACT" =~ ^[A-Za-z0-9._-]+$ ]] || { clear_previous; return 1; }
  [[ "$PREV_SHA256" =~ ^[0-9a-f]+$ ]] || { clear_previous; return 1; }
  have_previous
}

load_success_from_body() {
  local body="$1"
  local line
  clear_previous
  line="$(printf '%s\n' "$body" | grep -m1 -F "$success_prefix" || true)"
  if [ -n "$line" ]; then
    parse_marker_line "$line"
    return
  fi
  return 1
}

render() {
  local status="$1"
  local head="${HEAD_SHA:?}"
  local repo="${GITHUB_REPOSITORY:?}"
  local run_url="${RUN_URL:?}"
  local run_id="${RUN_ID:?}"
  local commit header body suffix
  commit="$(commit_link "$head")"
  case "$status" in
    building)
      header="$(comment_header_for_previous)"
      suffix="$(last_success_suffix)"
      body="$(cat <<EOF
${header}
### macOS preview build

Building ${commit}...

- **Run:** ${run_url}
- **Status:** in progress

This comment is updated when the build finishes. A newer push replaces this run.${suffix}
EOF
)"
      ;;
    success)
      local artifact="${ARTIFACT:?}"
      local sha256="${SHA256:?}"
      header="$(comment_header "$run_url" "$run_id" "$head" "$artifact" "$sha256")"
      body="$(cat <<EOF
${header}
### macOS preview build

- **Run:** ${run_url}
- **Commit:** ${commit}
- **Artifact:** \`${artifact}\` (expires in 7 days)
- **CLI:** \`gh run download ${run_id} -R ${repo} -n ${artifact}\`
- **SHA-256:** \`${sha256}\`

The app is ad-hoc signed and not notarized. Open the DMG, move \`Navidog.app\`
to \`~/Applications\` (create it if needed), then allow it in **System Settings
→ Privacy & Security → Open Anyway** if macOS blocks the first launch.

This build comes from pull request code; review the diff before running it.
EOF
)"
      ;;
    failure)
      header="$(comment_header_for_previous)"
      suffix="$(last_success_suffix)"
      body="$(cat <<EOF
${header}
### macOS preview build

The build of ${commit} failed.

- **Run:** ${run_url}

See the run log for the error.${suffix}
EOF
)"
      ;;
    *)
      echo "Usage: preview-comment.sh building|success|failure" >&2
      exit 2
      ;;
  esac
  printf '%s\n' "$body"
}

# Update the newest matching comment. A pull request should have one; if an
# older duplicate is still there, the bottom one is the one readers see.
find_comment_id() {
  local repo="${GITHUB_REPOSITORY:?}"
  local pr="${PR_NUMBER:?}"
  gh api --paginate "repos/${repo}/issues/${pr}/comments" \
    --jq ".[] | select(.user.login == \"github-actions[bot]\" and ((.body // \"\") | startswith(\"${marker}\"))) | .id" \
    | tail -n 1
}

comment_body() {
  local id="$1"
  local repo="${GITHUB_REPOSITORY:?}"
  gh api "repos/${repo}/issues/comments/${id}" --jq .body
}

upsert() {
  local id="$1"
  local body="$2"
  local repo="${GITHUB_REPOSITORY:?}"
  local pr="${PR_NUMBER:?}"
  if [ -n "$id" ]; then
    gh api -X PATCH "repos/${repo}/issues/comments/${id}" -f body="$body" >/dev/null
  else
    gh api -X POST "repos/${repo}/issues/${pr}/comments" -f body="$body" >/dev/null
  fi
}

if [ "${1:-}" = "--check" ]; then
  export GITHUB_REPOSITORY="example/navidog"
  export GITHUB_SERVER_URL="https://github.com"
  export RUN_URL="https://github.com/example/navidog/actions/runs/1"
  export RUN_ID="1"
  export HEAD_SHA="0123456789abcdef0123456789abcdef01234567"
  export ARTIFACT="Navidog-preview-macos-aarch64"
  export SHA256="deadbeef"
  for status in building success failure; do
    body="$(render "$status")"
    first="${body%%$'\n'*}"
    if [ "$first" != "$marker" ]; then
      echo "$status: marker is not the first line" >&2
      exit 1
    fi
  done
  building="$(render building)"
  success="$(render success)"
  failure="$(render failure)"
  printf '%s' "$building" | grep -q "in progress"
  printf '%s' "$success" | grep -q "Navidog-preview-macos-aarch64"
  printf '%s' "$success" | grep -q "deadbeef"
  printf '%s' "$failure" | grep -q "failed"
  printf '%s' "$building" | grep -q "0123456"
  second="${success#*$'\n'}"
  second="${second%%$'\n'*}"
  case "$second" in
    "<!-- navidog-preview-success "*) ;;
    *)
      echo "success: hidden marker is not the second line" >&2
      exit 1
      ;;
  esac
  printf '%s' "$second" | grep -q "run_id=1"
  printf '%s' "$second" | grep -q "sha256=deadbeef"
  printf '%s' "$second" | grep -q "artifact=Navidog-preview-macos-aarch64"
  if printf '%s' "$success" | grep -q "Last successful"; then
    echo "success: duplicated the last-success section" >&2
    exit 1
  fi
  if printf '%s' "$building" | grep -q "Last successful"; then
    echo "building: invented a last success" >&2
    exit 1
  fi
  if printf '%s' "$failure" | grep -q "Last successful"; then
    echo "failure: invented a last success" >&2
    exit 1
  fi

  # A later run keeps the success recorded by the marker.
  export RUN_URL="https://github.com/example/navidog/actions/runs/42"
  export RUN_ID="42"
  export HEAD_SHA="abcdef0123456789abcdef0123456789abcdef01"
  export SHA256="cafebabe"
  previous="$(render success)"
  load_success_from_body "$previous"
  [ "$PREV_RUN_ID" = "42" ]
  [ "$PREV_SHA256" = "cafebabe" ]
  [ "$PREV_ARTIFACT" = "Navidog-preview-macos-aarch64" ]
  [ "$PREV_HEAD_SHA" = "abcdef0123456789abcdef0123456789abcdef01" ]
  [ "$PREV_RUN_URL" = "https://github.com/example/navidog/actions/runs/42" ]

  export RUN_URL="https://github.com/example/navidog/actions/runs/99"
  export RUN_ID="99"
  export HEAD_SHA="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  building="$(render building)"
  failure="$(render failure)"
  for body in "$building" "$failure"; do
    printf '%s' "$body" | grep -q "Last successful"
    printf '%s' "$body" | grep -q "cafebabe"
    printf '%s' "$body" | grep -q "run_id=42"
    printf '%s' "$body" | grep -q "actions/runs/99"
    second="${body#*$'\n'}"
    second="${second%%$'\n'*}"
    case "$second" in
      "<!-- navidog-preview-success "*) ;;
      *)
        echo "kept comment: hidden marker is not the second line" >&2
        exit 1
        ;;
    esac
  done
  printf '%s' "$building" | grep -q "aaaaaaa"
  printf '%s' "$building" | grep -q "abcdef0"
  printf '%s' "$failure" | grep -q "failed"

  # The next update must keep the same success, not the in-progress run.
  load_success_from_body "$building"
  [ "$PREV_RUN_ID" = "42" ]
  [ "$PREV_SHA256" = "cafebabe" ]
  load_success_from_body "$success"
  [ "$PREV_RUN_ID" = "1" ]
  [ "$PREV_SHA256" = "deadbeef" ]

  clear_previous
  if load_success_from_body "$(render building)"; then
    echo "building comment without a success record parsed as success" >&2
    exit 1
  fi
  if load_success_from_body "$(render failure)"; then
    echo "failure comment without a success record parsed as success" >&2
    exit 1
  fi
  [ -z "$PREV_RUN_ID" ]

  poison='<!-- navidog-preview-success run_url=https://github.com/example/navidog/actions/runs/42 run_id=42 head_sha=abcdef0123456789abcdef0123456789abcdef01 artifact=ok;rm sha256=cafebabe -->'
  if load_success_from_body "$poison"; then
    echo "poisoned artifact was accepted" >&2
    exit 1
  fi
  [ -z "$PREV_ARTIFACT" ]
  echo "ok"
  exit 0
fi

status="${1:?Usage: preview-comment.sh building|success|failure}"
id="$(find_comment_id)"
if [ "$status" != "success" ] && [ -n "$id" ]; then
  # Fetch outside the `||` so a failed read stops before the comment is replaced.
  existing="$(comment_body "$id")"
  load_success_from_body "$existing" || true
fi
upsert "$id" "$(render "$status")"
