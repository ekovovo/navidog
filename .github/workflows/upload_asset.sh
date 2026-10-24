#!/usr/bin/env bash
# Attach one or more files to the release for the current tag.
#
# The release is normally pre-created by prepare_vars or by the release job.
# Creating a draft here is only a fallback. `--clobber` replaces an asset of
# the same name, so re-running a job does not fail with 422.
#
# Usage: upload_asset.sh <FILE>...
# The release tag is GITHUB_REF_NAME, unless RELEASE_TAG is set. Nightly builds
# run on main, so they set RELEASE_TAG=nightly.
set -euo pipefail

if [ $# -lt 1 ]; then
    echo "Usage: upload_asset.sh <FILE>..."
    exit 1
fi

tag="${RELEASE_TAG:-${GITHUB_REF_NAME:?}}"
repo="${GITHUB_REPOSITORY:?}"
echo "Uploading $* to $repo@$tag"

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

gh release upload "$tag" "$@" -R "$repo" --clobber
