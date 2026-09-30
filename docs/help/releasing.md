---
title: 'Releasing'
description: 'How a quarry release is made - the changelog, `just ship`, the GitHub Release and the AUR packages.'
---

# Releasing

Releases are made from a maintainer's machine with `just`, the GitHub CLI (`gh`), `makepkg`, and an
SSH key registered with your [AUR account](https://aur.archlinux.org/account). Nothing runs in CI.

## While you work

Add a line to `CHANGELOG.md` under `## Unreleased` for anything a user would notice, written for
users: what changed and how to use it, not how it was built. Group entries under `###` headings when
there are several. That section becomes the release's GitHub Release notes, word for word.

`just release-notes Unreleased` shows what the next release will say.

## Shipping a release

```sh
just ship 0.2.0
```

`ship` stops at the first thing that isn't ready. It checks that:

- the version looks like `0.2.0` and is newer than the last release tag,
- `CHANGELOG.md` has something under `## Unreleased`,
- you're on an up-to-date, clean `main`, and the tag doesn't exist yet,
- `gh` is signed in, and the AUR accepts your SSH key.

Then it:

1. Runs `just check` (clippy and the tests).
2. Sets the version in `Cargo.toml`, `Cargo.lock` and both PKGBUILDs, moves `## Unreleased` under
   `## 0.2.0 - <date>` in the changelog, and commits `Version 0.2.0`.
3. **Asks before going further.** Answer anything but `y` and nothing is pushed or published; undo
   the commit with `git reset --hard HEAD~1`.
4. Tags `v0.2.0` and pushes `main` and the tag.
5. Runs `packaging/release.sh`: builds `quarry-0.2.0-x86_64-unknown-linux-gnu.tar.gz` (binary,
   README, licence) from the tag's sources into `dist/`, with a `.sha256`, and fills in both
   PKGBUILDs' checksums and `.SRCINFO` files.
6. Creates the GitHub Release `v0.2.0` with the tarball, its checksum, and the changelog section as
   notes (`just release-notes 0.2.0`, with a compare link to the previous release).
7. Commits the checksums as `Release 0.2.0` and pushes.
8. Publishes `quarry-sql` and `quarry-sql-bin` to the AUR (`just aur`).

If a step after the tag fails, `ship` says so; finish the remaining steps by hand, in order:
`just release`, then `gh release create …` as `packaging/release.sh` prints, then commit and push,
then `just aur`.

## AUR packages

`packaging/aur/` holds the two AUR packages. Both install the `quarry` command with shell completion,
and conflict with the AUR's unrelated `quarry` package, which also ships `/usr/bin/quarry`.

| Package | Installs |
|---|---|
| `quarry-sql` | Builds from the tag's source tarball, and runs the unit, CLI and driver tests |
| `quarry-sql-bin` | The prebuilt tarball from the GitHub Release |

`just aur` publishes both through throwaway clones in `dist/aur`. It refuses when a checksum is
missing, a `.SRCINFO` is stale, or the GitHub Release has no tarball yet. To try the source package
locally, `just package` builds it from the committed `HEAD` into `dist/`, and `just install`
installs it.

## Other recipes

| Recipe | Does |
|---|---|
| `just tarball` | The release tarball from the working tree, for testing |
| `just release` | Step 5 on its own, after a tag is pushed |
| `just srcinfo` | Regenerate both `.SRCINFO` files after editing a PKGBUILD |
