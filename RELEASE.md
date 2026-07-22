# Axiom Release Process

## Version Numbering

Axiom uses **pre-release semver**: `0.<minor>.<patch>-alpha.<N>`. The current
version is `0.1.0` (no alpha suffix in `Cargo.toml`, though past releases used
`v0.1.0-alpha.1` through `v0.1.0-alpha.3`).

Rules:
- **Minor bump** (`0.2.0-alpha.1`) — significant new feature or protocol
  support that changes the compositor's capability surface.
- **Patch bump** (`0.1.1-alpha.1`) — bug fixes, performance improvements,
  dependency updates, or internal refactoring with no user-facing protocol
  changes.
- **Alpha increment** (e.g. `0.1.0-alpha.2` → `0.1.0-alpha.3`) — between
  patch-level releases when iterative testing is warranted.
- Pre-release tags always carry the `-alpha.N` suffix until a stable 1.0.

## Prerequisites

Before initiating a release:

1. All CI jobs on `main` pass (fmt, clippy, test matrix, audit,
   benchmarks).
2. `cargo build` produces zero warnings.
3. `cargo test --all-targets` passes (two `#[ignore]`d screencopy tests
   that require `xvfb-run` are acceptable).
4. The existing release checklist has been reviewed and all applicable
   items are checked: `docs/dev/RELEASE_CHECKLIST.md`.
5. `CHANGELOG.md` has been updated with the unreleased changes section.
6. Release notes draft is prepared under `release-notes/`.

## Full Procedure

See the detailed step-by-step checklist at:

- **`docs/dev/RELEASE_CHECKLIST.md`** — pre-release scope verification,
  build/test gates, runtime spot checks, packaging, documentation sync,
  and publication steps.
- **`docs/dev/RELEASE_PROCESS.md`** — the canonical release-prep workflow
  including the `make release-check` gate, `release_prep.sh` helper script,
  and `gh release create` commands.

The automated helper (from `docs/dev/RELEASE_PROCESS.md`):

```bash
make release-check                        # full preflight gate
bash scripts/release_prep.sh all v0.X.Y-alpha.N   # validate, draft notes, print commands
```

## Post-Release Steps

After the release is published:

1. **Tag** — the release tag (`v0.X.Y-alpha.N`) must be pushed to GitHub:
   ```bash
   git tag -a v0.X.Y-alpha.N -m "Axiom v0.X.Y-alpha.N"
   git push origin v0.X.Y-alpha.N
   ```
2. **GitHub Release** — create or publish the draft release via `gh`:
   ```bash
   gh release create v0.X.Y-alpha.N \
     --title "Axiom v0.X.Y-alpha.N" \
     --notes-file "release-notes/v0.X.Y-alpha.N.md"
   ```
3. **CI** — verify that CI runs on the tag and all jobs pass.
4. **Packages** — if the release includes packaging changes, validate the
   PKGBUILD (Arch Linux) and session wrapper: `make test-package`.
5. **Announce** — update the project's communication channels (if any) with
   a link to the GitHub release notes.
