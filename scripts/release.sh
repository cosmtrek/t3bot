#!/bin/sh
# Cut a release: bump Cargo.toml, test, commit, tag vX.Y.Z and push main + the tag.
# The tag push triggers .github/workflows/release.yml, which builds and publishes it.
#
#   scripts/release.sh 0.2.0
#   scripts/release.sh 0.2.0 --dry-run     # everything except commit/tag/push
set -eu
cd "$(dirname "$0")/.."

die() { echo "release: $*" >&2; exit 1; }

[ $# -ge 1 ] || die "usage: scripts/release.sh X.Y.Z [--dry-run]"
v=${1#v}
dry=0
[ "${2:-}" = --dry-run ] && dry=1
echo "$v" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || die "version must be X.Y.Z, got $1"

cur=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
newer=$(printf '%s\n%s\n' "$cur" "$v" | sort -V | tail -n1)
[ "$v" != "$cur" ] && [ "$newer" = "$v" ] || die "$v must be greater than the current version $cur"

[ "$(git rev-parse --abbrev-ref HEAD)" = main ] || die "not on main"
[ -z "$(git status --porcelain --untracked-files=no)" ] || die "working tree has uncommitted changes"
git fetch -q origin main --tags
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || die "main is not in sync with origin/main"
git rev-parse -q --verify "refs/tags/v$v" >/dev/null && die "tag v$v already exists"

sed -i "0,/^version = \"$cur\"$/s//version = \"$v\"/" Cargo.toml
cargo check -q            # updates Cargo.lock
cargo test -q --locked
echo "Cargo.toml: $cur → $v"

if [ "$dry" = 1 ]; then
  git checkout -q -- Cargo.toml Cargo.lock
  echo "dry run: reverted; would commit, tag v$v and push"
  exit 0
fi

git commit -q -m "release: v$v" Cargo.toml Cargo.lock
git tag -a "v$v" -m "t3bot v$v"
git push --atomic origin main "v$v"
echo "pushed v$v; follow the build: gh run watch \$(gh run list -w release -L1 --json databaseId --jq '.[0].databaseId')"
