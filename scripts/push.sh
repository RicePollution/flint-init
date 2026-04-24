#!/usr/bin/env bash
# scripts/push.sh — stage, commit, tag (auto-incremented semver), and push.
#
# Usage:
#   ./scripts/push.sh "commit message"           # bump patch (default)
#   ./scripts/push.sh "commit message" minor     # bump minor, reset patch
#   ./scripts/push.sh "commit message" major     # bump major, reset minor + patch

set -euo pipefail

if [ $# -lt 1 ]; then
    echo "usage: $0 <commit message> [major|minor|patch]" >&2
    exit 1
fi

MSG="$1"
BUMP="${2:-patch}"

# Validate bump type
case "$BUMP" in
    major|minor|patch) ;;
    *) echo "error: bump must be major, minor, or patch (got '$BUMP')" >&2; exit 1 ;;
esac

# Find the most recent semver tag, stripping any non-numeric suffix (e.g. -stage2)
LAST_TAG="$(git tag --sort=-version:refname | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)"

if [ -z "$LAST_TAG" ]; then
    MAJOR=0; MINOR=1; PATCH=0
else
    # Strip leading 'v' and any suffix after the third numeric component
    VERSION="${LAST_TAG#v}"
    VERSION="${VERSION%%-*}"   # drop -stage2 style suffixes
    IFS='.' read -r MAJOR MINOR PATCH <<< "$VERSION"
fi

case "$BUMP" in
    major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
    minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
    patch) PATCH=$((PATCH + 1)) ;;
esac

NEW_TAG="v${MAJOR}.${MINOR}.${PATCH}"

# Stage everything tracked + new files (excluding gitignored paths)
git add -A

# Check there's actually something to commit
if git diff --cached --quiet; then
    echo "nothing staged to commit" >&2
    exit 1
fi

git commit -m "$MSG

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"

git tag "$NEW_TAG"
git push --tags origin HEAD

echo "pushed $NEW_TAG"
