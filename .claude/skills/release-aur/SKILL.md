# Release a new version on AUR

## Step 1 — Bump the version in this repo

Edit `flexo/Cargo.toml`: change the `version` field under `[package]` to the new version string.

Do **not** manually edit `flexo/Cargo.lock`. Instead, run this from the `flexo/` directory to regenerate it:

```bash
cargo check
```

Commit both files with a message like `chore: bump flexo version to X.Y.Z`, then push to the master branch.

## Step 2 — Update the AUR package

The AUR package is `flexo-git` on AUR, checked out locally via SSH. Make sure the version bump from Step 1 is already committed and pushed before continuing.

From the local checkout of the AUR package, run:

```bash
makepkg -od
makepkg --printsrcinfo > .SRCINFO
```

`makepkg -od` downloads sources and resolves the new `pkgver` (the git-describe string), which updates `PKGBUILD` in place. The second command regenerates `.SRCINFO` to match.

Verify both `PKGBUILD` and `.SRCINFO` show the new version, then commit and push.
