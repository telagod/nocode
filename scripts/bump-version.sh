#!/usr/bin/env bash
set -euo pipefail

# Usage: ./scripts/bump-version.sh <new-version>
# Example: ./scripts/bump-version.sh 0.2.0
#
# Updates version in:
#   - Cargo.toml (workspace.package.version)
#   - npm/nocode/package.json (version + optionalDependencies)
#   - npm/nocode-linux-x64/package.json
#   - npm/nocode-darwin-x64/package.json
#   - npm/nocode-darwin-arm64/package.json
#   - npm/nocode-win32-x64/package.json (if exists)

NEW_VERSION="${1:?Usage: bump-version.sh <new-version>}"

# Validate semver format
if ! echo "$NEW_VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$'; then
    echo "error: invalid semver: $NEW_VERSION"
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

OLD_VERSION=$(grep -m1 'version = ' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
echo "bumping: $OLD_VERSION -> $NEW_VERSION"

# 1. Cargo.toml workspace version
sed -i "s/^version = \"$OLD_VERSION\"/version = \"$NEW_VERSION\"/" Cargo.toml
echo "  updated Cargo.toml"

# 2. npm packages
for pkg_json in npm/*/package.json; do
    # Update package version
    sed -i "s/\"version\": \"$OLD_VERSION\"/\"version\": \"$NEW_VERSION\"/" "$pkg_json"
    # Update optionalDependencies versions (main package)
    sed -i "s/\"@telagod\/nocode-[^\"]*\": \"$OLD_VERSION\"/\0/" "$pkg_json"
    sed -i "s/\": \"$OLD_VERSION\"/\": \"$NEW_VERSION\"/g" "$pkg_json"
    echo "  updated $pkg_json"
done

echo ""
echo "done. verify with:"
echo "  grep version Cargo.toml | head -1"
echo "  grep version npm/*/package.json"
echo ""
echo "next steps:"
echo "  git add -A && git commit -m 'chore: bump version to $NEW_VERSION'"
echo "  git tag v$NEW_VERSION && git push origin main --tags"
