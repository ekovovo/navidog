#!/usr/bin/env bash
# Attach one file to the release for the current tag.
#
# The release is normally pre-created by prepare_vars. Creating a draft here
# is only a fallback for a partial run. `--clobber` replaces an asset of the
# same name, so re-running a job does not fail with 422.
#
# Usage: upload_asset.sh <FILE> [TOKEN]
set -euo pipefail

if [ $# -lt 1 ]; then
    echo "Usage: upload_asset.sh <FILE> [TOKEN]"
    exit 1
fi

file_path=$1
if [ -n "${2:-}" ]; then
    export GH_TOKEN="$2"
fi

tag="${GITHUB_REF_NAME:?}"
repo="${GITHUB_REPOSITORY:?}"
echo "Uploading $file_path to $repo@$tag"

# Any release (published or draft) that claims this tag. `gh release view`
# only resolves published tags, so a pre-created draft would look missing
# and every platform job would race to create another.
release_ids="$(
    gh api --paginate "repos/${repo}/releases" \
        -q ".[] | select(.tag_name == \"${tag}\") | .id" 2>/dev/null || true
)"
if [ -z "${release_ids}" ]; then
    echo "No release for $tag; creating draft..."
    gh release create "$tag" -R "$repo" --draft --title "$tag" --notes "" || true
else
    echo "Found release for $tag (id(s): $(echo "$release_ids" | tr '\n' ' '))"
fi

gh release upload "$tag" "$file_path" -R "$repo" --clobber
