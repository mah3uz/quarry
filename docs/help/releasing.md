---
title: 'Releasing'
description: 'How a quarry release is made - the changelog, `just ship`, the GitHub Release, the AUR packages and the Homebrew formula.'
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
- `gh` is signed in, and the AUR accepts your SSH key,
- the Homebrew tap (`mah3uz/homebrew-tap` on GitHub) exists,
- `wrangler` is logged in (`npx wrangler login` in `docs/`), for the docs site.

Then it:

1. Runs `just check` (clippy and the tests).
2. Sets the version in `Cargo.toml`, `Cargo.lock`, both PKGBUILDs and the Homebrew formula, moves `## Unreleased` under
   `## 0.2.0 - <date>` in the changelog, and commits `Version 0.2.0`.
3. **Asks before going further.** Answer anything but `y` and nothing is pushed or published; undo
   the commit with `git reset --hard HEAD~1`.
4. Tags `v0.2.0` and pushes `main` and the tag.
5. Runs `packaging/release.sh`: builds `quarry-0.2.0-x86_64-unknown-linux-gnu.tar.gz` (binary,
   README, licence) from the tag's sources into `dist/`, with a `.sha256`, and fills in both
   PKGBUILDs' checksums and `.SRCINFO` files and the formula's checksum.
6. Creates the GitHub Release `v0.2.0` with the tarball, its checksum, and the changelog section as
   notes (`just release-notes 0.2.0`, with a compare link to the previous release).
7. Commits the checksums as `Release 0.2.0` and pushes.
8. Publishes `quarry-sql` and `quarry-sql-bin` to the AUR (`just aur`).
9. Publishes the formula to the Homebrew tap (`just brew`).
10. Builds the docs site and deploys it (`just docs-deploy`), so what it says about installing and
    the new version goes live with the release.

If a step after the tag fails, `ship` says so; finish the remaining steps by hand, in order:
`just release`, then `gh release create …` as `packaging/release.sh` prints, then commit and push,
then `just aur`, then `just brew`, then `just docs-deploy`.

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

## Homebrew formula

`packaging/homebrew/quarry.rb` builds quarry from the tag's source tarball with Cargo, on macOS and
Linux, and installs shell completion. It's published to a tap, the GitHub repository
`mah3uz/homebrew-tap`, which has to exist before the first release that uses it:

```sh
gh repo create mah3uz/homebrew-tap --public
```

`just brew` copies the formula to `Formula/quarry.rb` in the tap through a throwaway clone in
`dist/tap`. It refuses when the formula isn't at the current version or its checksum isn't the
source tarball's.

## Other recipes

| Recipe | Does |
|---|---|
| `just tarball` | The release tarball from the working tree, for testing |
| `just release` | Step 5 on its own, after a tag is pushed |
| `just srcinfo` | Regenerate both `.SRCINFO` files after editing a PKGBUILD |
