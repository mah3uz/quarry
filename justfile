# quarry development tasks. Run `just` to see them all.

set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments

version := `sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1`
target := `rustc -vV | sed -n 's/^host: //p'`
pkgver := `sed -n 's/^pkgver=//p' packaging/aur/quarry/PKGBUILD`

[private]
default:
    @just --list

# Run quarry from source, e.g. `just run demo.db -e 'select 1'`
[group('dev')]
run *args:
    cargo run -- "$@"

# Open the TUI from source, e.g. `just tui postgres://me@localhost/app`
[group('dev')]
tui *args:
    cargo run -- --tui "$@"

# Debug build
[group('dev')]
build:
    cargo build

# Run the tests (Postgres/MySQL tests use QUARRY_TEST_PG / QUARRY_TEST_MYSQL and skip if unreachable)
[group('check')]
test *args:
    cargo test "$@"

# Run the slow, ignored timing tests in release mode
[group('check')]
test-slow:
    cargo test --release -- --ignored

# Clippy with warnings as errors
[group('check')]
lint:
    cargo clippy --all-targets -- -D warnings

# Everything CI should run: lint, then tests
[group('check')]
check: lint test

# Build the Arch package from HEAD and install it with pacman
[group('install')]
install: package
    sudo pacman -U dist/quarry-{{pkgver}}-*-x86_64.pkg.tar.zst

# Remove the installed package (settings and history are kept)
[group('install')]
uninstall:
    sudo pacman -R quarry

# Install the binary into ~/.cargo/bin with Cargo instead (for systems without pacman)
[group('install')]
install-cargo:
    cargo install --path . --locked

# Remove the binary installed with install-cargo
[group('install')]
uninstall-cargo:
    cargo uninstall quarry

# Build the release tarball (binary, README, LICENSE) from the working tree into dist/
[group('release')]
tarball:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release --locked
    name="quarry-{{version}}-{{target}}"
    rm -rf "dist/$name" && mkdir -p "dist/$name"
    cp target/release/quarry README.md LICENSE "dist/$name/"
    tar -C dist -czf "dist/$name.tar.gz" "$name"
    rm -rf "dist/$name"
    (cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
    echo "dist/$name.tar.gz"

# Build the Arch package from the committed HEAD into dist/, as the AUR build does from a tag
[group('release')]
package:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "$(git status --porcelain)" ]; then
      echo "note: packaging HEAD; uncommitted changes are left out" >&2
    fi
    rm -rf dist/build && mkdir -p dist/build
    git archive --prefix=quarry-{{pkgver}}/ -o dist/build/quarry-{{pkgver}}.tar.gz HEAD
    cp packaging/aur/quarry/PKGBUILD dist/build/
    (cd dist/build && makepkg -f --noconfirm --skipchecksums)
    mv dist/build/quarry-{{pkgver}}-*-x86_64.pkg.tar.zst dist/
    rm -rf dist/build
    ls dist/quarry-{{pkgver}}-*-x86_64.pkg.tar.zst

# After pushing tag v<version>: the release tarball, AUR checksums and .SRCINFO
[group('release')]
release:
    packaging/release.sh

# One release's section of CHANGELOG.md, with its compare link; `Unreleased` shows what's coming
[group('release')]
release-notes v:
    #!/usr/bin/env bash
    set -euo pipefail
    url=$(sed -n 's/^repository = "\(.*\)"/\1/p' Cargo.toml)
    awk -v v={{v}} -v url="$url/" '
      $1 == "##" && found { prev = $2; exit }
      $1 == "##" && $2 == v { found = 1; i = index($0, " - "); if (i) stamp = substr($0, i + 3); next }
      found { lines[++n] = $0 }
      END {
        if (!found) { print "release-notes: no \"## " v "\" in CHANGELOG.md" > "/dev/stderr"; exit 1 }
        first = 1; while (first <= n && lines[first] == "") first++
        while (n >= first && lines[n] == "") n--
        if (stamp != "" && first <= n) print "_Released " stamp "_\n"
        for (i = first; i <= n; i++) print lines[i]
        if (v == "Unreleased" || first > n) exit
        print ""
        print "**Full Changelog**: " url (prev ? "compare/v" prev "...v" v : "commits/v" v)
      }
    ' CHANGELOG.md

# The whole release: checks, version bump, tag and GitHub Release, e.g. `just ship 0.1.0`
[group('release')]
ship v:
    #!/usr/bin/env bash
    set -euo pipefail
    v={{v}}
    fail() { echo "ship: $*" >&2; exit 1; }
    last=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)
    last=${last#v}
    last=${last:-0.0.0}

    [[ $v =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "version must look like 0.2.1, not $v"
    [[ $v != "$last" && $(printf '%s\n%s\n' "$last" "$v" | sort -V | tail -1) == "$v" ]] || fail "$v is not newer than the last release ($last)"
    [[ -n $(just release-notes Unreleased) ]] || fail "CHANGELOG.md has nothing under ## Unreleased"
    [[ $(git branch --show-current) == main ]] || fail "switch to main first"
    [[ -z $(git status --porcelain) ]] || fail "commit or stash your changes first"
    git fetch -q origin
    [[ $(git rev-list --count HEAD..origin/main) == 0 ]] || fail "main is behind origin/main; pull first"
    ! git rev-parse -q --verify "refs/tags/v$v" >/dev/null || fail "tag v$v already exists"
    ! git ls-remote --exit-code --tags origin "v$v" >/dev/null || fail "tag v$v already exists on origin"
    gh auth status >/dev/null 2>&1 || fail "gh is not logged in; run gh auth login"

    echo "==> lint and tests"
    just check

    echo "==> version $v (last release: $last)"
    sed -i "0,/^version = \".*\"/s//version = \"$v\"/" Cargo.toml
    for p in packaging/aur/quarry/PKGBUILD packaging/aur/quarry-bin/PKGBUILD; do
      sed -i "s/^pkgver=.*/pkgver=$v/; s/^pkgrel=.*/pkgrel=1/; s/^sha256sums=.*/sha256sums=('SKIP')/" "$p"
    done
    cargo update --workspace -q
    sed -i "s/^## Unreleased$/## Unreleased\n\n## $v - $(date '+%F %H:%M %:z')/" CHANGELOG.md
    git commit -q -am "Version $v"

    # Everything after this is public and can't be taken back.
    read -rp "Push v$v to origin and publish the GitHub Release? [y/N] " answer
    if [[ $answer != [yY] ]]; then
      echo "Stopped before pushing. To undo the version commit: git reset --hard HEAD~1"
      exit 1
    fi
    trap 'echo "ship: stopped; finish the remaining steps by hand (docs: Help > Releasing)" >&2' ERR

    echo "==> tag and push"
    git tag "v$v"
    git push origin main "v$v"

    echo "==> release tarball and checksums"
    QUARRY_SHIP=1 packaging/release.sh

    echo "==> GitHub Release"
    asset="dist/quarry-$v-x86_64-unknown-linux-gnu.tar.gz"
    just release-notes "$v" > "dist/notes-$v.md"
    gh release create "v$v" "$asset" "$asset.sha256" --title "v$v" --notes-file "dist/notes-$v.md"

    echo "==> commit the checksums"
    git commit -q -am "Release $v"
    git push origin main
    echo "Released $v. packaging/aur is ready for the AUR; publish it with 'just aur' when you decide to."

# Regenerate both AUR packages' .SRCINFO
[group('release')]
srcinfo:
    @for p in quarry quarry-bin; do (cd packaging/aur/$p && makepkg --printsrcinfo > .SRCINFO); done

# Publish packaging/aur to the AUR, through throwaway clones in dist/aur (not part of `ship`)
[group('release')]
aur:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "$(git status --porcelain packaging/aur)" ]; then
      echo "commit packaging/aur first" >&2
      exit 1
    fi
    url=$(sed -n "s/^url='\(.*\)'/\1/p" packaging/aur/quarry/PKGBUILD)
    if ! curl -fsIL -o /dev/null "$url/releases/download/v{{pkgver}}/quarry-{{pkgver}}-x86_64-unknown-linux-gnu.tar.gz"; then
      echo "the v{{pkgver}} GitHub Release has no tarball yet, which quarry-bin downloads" >&2
      exit 1
    fi
    for pkg in quarry quarry-bin; do
      src=packaging/aur/$pkg
      if grep -q "^sha256sums=('SKIP')" "$src/PKGBUILD"; then
        echo "$pkg: no checksum; run just release first" >&2
        exit 1
      fi
      if ! diff -q <(cd "$src" && makepkg --printsrcinfo) "$src/.SRCINFO" >/dev/null; then
        echo "$pkg: .SRCINFO is stale; run just srcinfo and commit" >&2
        exit 1
      fi
      dir=dist/aur/$pkg
      rm -rf "$dir"
      git clone -q "ssh://aur@aur.archlinux.org/$pkg.git" "$dir" 2>/dev/null
      cp "$src"/{PKGBUILD,.SRCINFO} "$dir"/
      git -C "$dir" add PKGBUILD .SRCINFO
      if git -C "$dir" diff --cached --quiet; then
        echo "$pkg: already up to date"
      else
        rel=$(sed -n 's/^pkgrel=//p' "$src/PKGBUILD")
        git -C "$dir" commit -q -m "Update to {{pkgver}}-$rel"
        # The AUR only accepts master, whatever init.defaultBranch named the clone's branch.
        git -C "$dir" push -q origin HEAD:master
        echo "$pkg: pushed {{pkgver}}-$rel"
      fi
      rm -rf "$dir"
    done

# Regenerate logo.svg, banner.svg and the docs site's copies
[group('art')]
art:
    python3 scripts/gen_art.py

[private]
docs-deps:
    [ -d docs/node_modules ] || npm --prefix docs ci

# Preview the docs site with live reload at http://localhost:4321
[group('docs')]
docs: docs-deps
    npm --prefix docs run dev

# Build the docs site into docs/dist
[group('docs')]
docs-build: docs-deps
    npm --prefix docs run build

# Serve the built docs site
[group('docs')]
docs-preview: docs-build
    npm --prefix docs run preview

# Remove build output (Rust, release files and docs)
[group('clean')]
clean:
    cargo clean
    rm -rf dist docs/dist docs/.astro
