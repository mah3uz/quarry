#!/usr/bin/env bash
# Run after pushing the tag v<version>: builds the release tarball from the tag's sources and fills both
# AUR PKGBUILDs' checksums and .SRCINFO files, and the Homebrew formula's checksum. It uploads and pushes nothing.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
aur=$root/packaging/aur
ver=$(sed -n 's/^pkgver=//p' "$aur/quarry-sql/PKGBUILD")
url=$(sed -n "s/^url='\(.*\)'/\1/p" "$aur/quarry-sql/PKGBUILD")
target=x86_64-unknown-linux-gnu
name=quarry-$ver-$target
dist=$root/dist
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

[[ $(rustc -vV | sed -n 's/^host: //p') == "$target" ]] || { echo "release.sh builds $target; this machine isn't one" >&2; exit 1; }

set_sum() {
  sed -i "s/^sha256sums=('[^']*')/sha256sums=('$2')/" "$1"
}

echo "==> quarry $ver: checksum of the v$ver source tarball"
curl -fsSL "$url/archive/refs/tags/v$ver.tar.gz" -o "$work/source.tar.gz"
source_sum=$(sha256sum "$work/source.tar.gz" | cut -d' ' -f1)
set_sum "$aur/quarry-sql/PKGBUILD" "$source_sum"
sed -i "s/^  sha256 \"[^\"]*\"/  sha256 \"$source_sum\"/" "$root/packaging/homebrew/quarry.rb"

echo "==> building $name from the tag"
tar -xzf "$work/source.tar.gz" -C "$work"
# A target dir of its own, kept between releases, so later builds are incremental.
(cd "$work/quarry-$ver" && CARGO_TARGET_DIR="$root/target/release-build" cargo build --release --locked)
mkdir -p "$work/$name" "$dist"
cp "$root/target/release-build/release/quarry" "$work/quarry-$ver"/{README.md,LICENSE} "$work/$name/"
tar -C "$work" -czf "$dist/$name.tar.gz" "$name"
(cd "$dist" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")

echo "==> quarry-sql-bin: checksum of the release asset"
set_sum "$aur/quarry-sql-bin/PKGBUILD" "$(sha256sum "$dist/$name.tar.gz" | cut -d' ' -f1)"

for pkg in quarry-sql quarry-sql-bin; do
  (cd "$aur/$pkg" && makepkg --printsrcinfo > .SRCINFO)
done

# `just ship` runs the next steps itself.
[[ -n ${QUARRY_SHIP:-} ]] && exit 0

cat <<NEXT

Done: $dist/$name.tar.gz
Next, by hand:
  gh release create v$ver "$dist/$name.tar.gz" "$dist/$name.tar.gz.sha256" --title "v$ver" --notes-file <(just release-notes $ver)
  git commit -am "Release $ver" && git push
  just aur
  just brew
  just docs-deploy
NEXT
