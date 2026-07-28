# Releasing AnchorKit

This page documents the version-tag format, release-notes convention, and the
dry-run procedure for the automated WASM release workflow
(`.github/workflows/release.yml`).

---

## How the workflow is triggered

The `release.yml` workflow fires on any tag that matches either of:

```
v<major>.<minor>.<patch>          # production release  – e.g. v0.2.0
v<major>.<minor>.<patch>-<pre>    # pre-release / dry-run – e.g. v0.2.0-rc.1
```

Tags that do not start with `v` are ignored by the workflow.

---

## Version-tag format

AnchorKit tags follow [Semantic Versioning 2.0.0](https://semver.org):

| Component | Meaning |
|---|---|
| `v` prefix | Required; the workflow pattern matches `v*` tags only. |
| `MAJOR` | Incremented on breaking contract ABI changes. |
| `MINOR` | Incremented on backward-compatible feature additions. |
| `PATCH` | Incremented on backward-compatible bug fixes only. |
| `-<pre>` (optional) | Pre-release qualifier; any string after the first `-`. |

### Examples

| Tag | Kind | GitHub release flag |
|---|---|---|
| `v0.2.0` | Production | release |
| `v0.2.0-rc.1` | Release candidate (pre-release) | pre-release |
| `v0.2.0-beta` | Beta (pre-release) | pre-release |
| `v0.2.0-alpha.2` | Alpha (pre-release) | pre-release |

Tags with a `-` qualifier are automatically marked as **pre-release** on
GitHub so they do not appear as the "latest" release for end users.

---

## Release artifacts

Each release (production and pre-release) publishes exactly two files:

| File | Description |
|---|---|
| `anchorkit-<tag>.wasm` | The optimised Soroban contract binary, built with `opt-level=z`, `lto=true`, `strip=true`, `codegen-units=1`. |
| `anchorkit-<tag>.sha256` | A SHA-256 checksum file in `sha256sum`-compatible format. |

### Verifying a download

```sh
# Download both files from the release assets, then:
sha256sum -c anchorkit-v0.2.0.sha256
# anchorkit-v0.2.0.wasm: OK
```

---

## Release-notes convention

GitHub's auto-generated release notes are enabled (`generate_notes: true`).
They are seeded from merged pull-request titles and labels since the previous
tag. To produce useful auto-notes:

- Keep PR titles concise and descriptive (they become bullet points in the
  changelog).
- Label PRs appropriately (`bug`, `enhancement`, `documentation`, etc.) so
  GitHub can group them under the correct headings.

For significant releases, supplement the auto-generated notes with a
hand-written summary.  The preferred structure is:

```markdown
## Summary

One or two sentences describing the most important change in this release.

## Breaking changes

List any breaking contract ABI changes here (method removals, argument
reorders, changed return types).  If there are none, omit this section rather
than writing "none".

## Highlights

- Short bullet for each notable non-breaking addition or fix.

## Upgrade notes

Any migration steps callers need to take (re-deploy command, updated SDK
version, etc.).
```

Paste this body into the GitHub release editor after the workflow creates the
release, or pre-populate it by pushing an annotated tag:

```sh
git tag -a v0.2.0 -m "$(cat release-notes-v0.2.0.md)"
git push origin v0.2.0
```

When the tag has an annotation body, it appears verbatim in the release
(prepended before the auto-generated PR list).

---

## Cutting a production release — step by step

1. **Ensure `main` is green.** All CI jobs on the target commit must pass
   before tagging.

2. **Bump the version** in `Cargo.toml` (`[package] version = "…"`) and
   commit the change to `main` (or merge it via a PR).

3. **Dry-run with a pre-release tag first** (see below).

4. **Push the production tag:**

   ```sh
   # Lightweight tag (auto-notes only):
   git tag v0.2.0
   git push origin v0.2.0

   # Annotated tag (custom notes + auto-notes):
   git tag -a v0.2.0 -m "$(cat release-notes-v0.2.0.md)"
   git push origin v0.2.0
   ```

5. **Verify the release** on the GitHub Releases page:
   - Both `anchorkit-v0.2.0.wasm` and `anchorkit-v0.2.0.sha256` are attached.
   - The checksum file is correct (`sha256sum -c anchorkit-v0.2.0.sha256`).
   - The release is **not** marked as pre-release.

---

## Dry-run procedure (pre-release tag)

Before relying on the workflow for a real release, run it end-to-end against a
pre-release tag to confirm:

- The workflow triggers on the expected tag pattern.
- The WASM artifact builds successfully.
- Both asset files are attached to the GitHub release.
- The checksum file is valid.
- The release is correctly flagged as **pre-release**.

### Steps

```sh
# 1. From the commit you want to test (can be any commit on main):
git tag v0.1.0-rc.1
git push origin v0.1.0-rc.1
```

- Watch the `Release` workflow run on the **Actions** tab.
- Confirm the job summary shows a non-zero WASM size and a 64-character hex
  checksum.
- Download both release assets from the pre-release page and run:

  ```sh
  sha256sum -c anchorkit-v0.1.0-rc.1.sha256
  # anchorkit-v0.1.0-rc.1.wasm: OK
  ```

- Delete the pre-release tag and the GitHub release after validation (they
  will not appear as "latest" for end users while flagged as pre-release, but
  keeping them avoids confusion):

  ```sh
  git push origin --delete v0.1.0-rc.1
  git tag -d v0.1.0-rc.1
  # Also delete the draft/pre-release from the GitHub Releases UI.
  ```

Only proceed to a production tag (`v0.2.0`) after the dry run passes.

---

## Deleting a mis-pushed tag

If a tag was pushed by mistake:

```sh
# Delete locally
git tag -d v0.2.0

# Delete on origin (requires push permission)
git push origin --delete v0.2.0
```

Also delete the corresponding GitHub release from the Releases UI if the
workflow already created one.

---

## Related

- [`.github/workflows/release.yml`](../.github/workflows/release.yml) — the
  release workflow itself.
- [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) — the CI workflow
  that must be green before tagging.
- [`docs/wasm-size.md`](wasm-size.md) — WASM binary size history and profiling.
