#!/usr/bin/env bash
# fetch_release.sh REPO TAG ASSET OUT: one GitHub release asset, retried.
#
# CI lost jobs to release downloads timing out (HTTP 504) before a test ran.
# The asset comes through the API first (`gh release download`), which kept
# answering while github.com's download links did not, then through the
# download link, each retried.
set -u
repo=$1 tag=$2 asset=$3 out=$4
for attempt in 1 2 3 4 5; do
  gh release download "$tag" --repo "$repo" --pattern "$asset" --output "$out" --clobber && exit 0
  echo "fetch_release: $asset through the API failed (attempt $attempt)" >&2
  sleep $((attempt * 5))
done
curl -fsSL --retry 5 --retry-all-errors --retry-delay 5 \
  "https://github.com/$repo/releases/download/$tag/$asset" -o "$out"
