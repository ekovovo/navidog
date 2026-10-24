#!/usr/bin/env bash
# Attach one or more files to the release for the current tag.
#
# The release must already exist: prepare_vars creates the draft for a tag and
# the release job creates nightly before uploading. `--clobber` replaces an
# asset of the same name, so re-running a job does not fail with 422.
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

# Resolve the release by tag directly. An earlier version listed /releases and
# created a draft when the tag was missing; that list can briefly lag behind a
# release created moments earlier, which left a duplicate nightly draft.
gh release upload "$tag" "$@" -R "$repo" --clobber
