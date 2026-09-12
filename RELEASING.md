# Releasing TermLoom

A release is a git tag. Pushing `v<version>` builds three binaries, attaches
them to a GitHub Release with checksums, and makes
`curl -fsSL https://termloom.ghassen.io/install.sh | sh` install that version.

## One-time setup

### 1. GitHub Pages

The install script and landing page are served from Pages.

1. Repository **Settings → Pages → Build and deployment → Source**: choose
   **GitHub Actions**. Until this is set, the `Site` workflow fails on its
   first step with *"Get Pages site failed … verify that the repository has
   Pages enabled"* — that is the switch it is asking for, not a broken
   workflow. Re-run the workflow afterwards.
2. Push to `main` once; the `Site` workflow publishes `site/` plus a copy of
   `install.sh`.
3. Confirm `https://ghassenbrg.github.io/termloom/install.sh` serves the script.

### 2. The custom domain

`site/CNAME` already claims `termloom.ghassen.io`. At the DNS host for
`ghassen.io` add:

```
Type   Name       Value
CNAME  termloom   ghassenbrg.github.io.
```

Then in **Settings → Pages** set the custom domain to `termloom.ghassen.io`
and tick **Enforce HTTPS** once the certificate is issued (a few minutes).

Verify:

```bash
dig +short termloom.ghassen.io
curl -fsSL https://termloom.ghassen.io/install.sh | head -5
```

With the apex domain on another host, this subdomain is independent of it.

## Cutting a release

1. Update `CHANGELOG.md` — the section heading must be `## <version> — <date>`,
   because the workflow copies that section into the release notes.
2. Set the version in `Cargo.toml`, then `cargo check` so `Cargo.lock` follows.
3. Run the full gate on the pinned toolchain:

   ```bash
   cargo +1.90 fmt --all -- --check
   cargo +1.90 clippy --all-targets --all-features -- -D warnings
   cargo +1.90 test --all-targets --locked
   ```

4. Commit, tag and push:

   ```bash
   git commit -am "Release 0.1.0"
   git tag -a v0.1.0 -m "TermLoom 0.1.0"
   git push origin main v0.1.0
   ```

5. Watch it: `gh run watch` — the `Release` workflow builds
   `aarch64-apple-darwin`, `x86_64-apple-darwin` and
   `x86_64-unknown-linux-gnu`, smoke-tests each native binary, writes
   `SHA256SUMS` and publishes the release.

The tag must exist before the release is published (`--verify-tag`), so never
delete and re-push a tag that has already shipped; cut a new patch version
instead.

## Verifying the result

```bash
curl -fsSL https://termloom.ghassen.io/install.sh | sh
termloom --version
```

On a machine that already has TermLoom, the installer replaces the binary
atomically, so a running instance keeps working until it exits.

Check the checksums independently:

```bash
gh release download v0.1.0 --pattern 'SHA256SUMS' --pattern '*.tar.gz'
sha256sum -c SHA256SUMS
```

## Re-running a failed publish

The build matrix is re-runnable from the Actions tab. If the tag is already
pushed and only the publish step failed, use the manual trigger:

```bash
gh workflow run Release -f tag=v0.1.0
```

## Installing without the script

```bash
cargo install --git https://github.com/ghassenbrg/termloom --locked
```

or download a tarball from the release page, verify it and copy the binary
onto `PATH`.
