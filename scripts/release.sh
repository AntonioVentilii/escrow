#!/usr/bin/env bash
#
# Cut a new release of the escrow canister.
#
# Usage:
#   scripts/release.sh <version>            # e.g. scripts/release.sh 0.0.3
#   scripts/release.sh <version> --dry-run  # validate + show diff, do not push
#
# Via npm (note the `--` separator, required to forward the version arg):
#   npm run release -- <version>
#   npm run release -- <version> --dry-run
#
# What it does:
#   1. Validates the version (semver-ish) and the current repo state.
#   2. Bumps the workspace version in Cargo.toml, package.json, and
#      package-lock.json.
#   3. Refreshes Cargo.lock (offline if possible, otherwise online).
#   4. Commits the bump, tags `v<version>`, and pushes branch + tag.
#
# The push of the tag fires `.github/workflows/release.yml`, which builds the
# wasm, gathers the candid, and publishes the GitHub Release with the assets.
#
set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

err() {
  echo "error: $*" >&2
  exit 1
}

if [[ $# -lt 1 ]]; then
  err "missing <version> arg. usage: scripts/release.sh <version> [--dry-run]"
fi

VERSION="$1"
DRY_RUN=0
if [[ "${2:-}" == "--dry-run" ]]; then
  DRY_RUN=1
fi

# Semver-ish: major.minor.patch with optional -prerelease (e.g. 0.0.3, 1.2.3-rc.1)
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  err "version '$VERSION' is not in MAJOR.MINOR.PATCH[-PRERELEASE] form"
fi

TAG="v$VERSION"

if ! git diff --quiet || ! git diff --cached --quiet; then
  err "working tree has uncommitted changes; commit or stash them first"
fi

CURRENT_BRANCH="$(git symbolic-ref --short -q HEAD || true)"
if [[ -z "$CURRENT_BRANCH" ]]; then
  err "HEAD is detached; check out a branch before releasing"
fi
if [[ "$CURRENT_BRANCH" != "main" ]]; then
  echo "warning: not on 'main' (currently on '$CURRENT_BRANCH'). Continuing anyway." >&2
fi

git fetch --tags --quiet
if git rev-parse "$TAG" >/dev/null 2>&1; then
  err "tag '$TAG' already exists"
fi

CURRENT_VERSION="$(awk -F'"' '/^version[[:space:]]*=/ { print $2; exit }' Cargo.toml)"
echo "Bumping version: $CURRENT_VERSION -> $VERSION"

# Cargo.toml: only the [workspace.package] version line (the first `version = "..."`).
python3 - "$VERSION" <<'PY'
import re, sys
new = sys.argv[1]
path = "Cargo.toml"
with open(path) as f:
    content = f.read()
pattern = re.compile(r'(\[workspace\.package\][^\[]*?version\s*=\s*")([^"]+)(")', re.DOTALL)
new_content, n = pattern.subn(lambda m: m.group(1) + new + m.group(3), content, count=1)
if n != 1:
    sys.exit("error: failed to locate [workspace.package] version in Cargo.toml")
with open(path, "w") as f:
    f.write(new_content)
PY

# package.json + package-lock.json in one shot.
npm version --no-git-tag-version "$VERSION" >/dev/null

# Refresh Cargo.lock for the new workspace version.
# `cargo check` (without --locked) will update the lockfile; prefer offline so
# the script doesn't need network for routine bumps. Fall back to online if the
# offline cache is missing anything.
if ! cargo check --workspace --offline >/dev/null 2>&1; then
  cargo check --workspace >/dev/null
fi
# Sanity check: with the refreshed lockfile, a --locked check must succeed.
cargo check --workspace --locked --offline >/dev/null

echo
echo "Diff:"
git --no-pager diff --stat
echo

if [[ "$DRY_RUN" -eq 1 ]]; then
  echo "Dry run: leaving changes in working tree, no commit/tag/push."
  exit 0
fi

git add Cargo.toml Cargo.lock package.json package-lock.json
git commit -m "chore: bump version to $VERSION"
git tag "$TAG"
git push origin "$CURRENT_BRANCH"
git push origin "$TAG"

echo
echo "Released $TAG. The Release workflow will build and publish the GitHub Release."
echo "Track it: gh run list --workflow=release.yml --limit 1"
