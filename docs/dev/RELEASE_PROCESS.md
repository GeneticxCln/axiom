# Axiom Alpha Release Process

This document is the canonical release-prep workflow for Axiom's current alpha stage.

## Scope

Axiom releases should currently be framed as:

> **alpha compositor prototype with a strong nested development path**

Do not present releases as production-ready desktop-session replacements.

## 1. Preflight checks

Run the full release readiness gate:

```bash
make release-check
```

Then run the release-prep helper in check mode:

```bash
bash ./scripts/release_prep.sh check v0.1.0-alpha.1
```

By default, `release_prep.sh` expects a **clean git tree**. If you explicitly need to draft notes before committing, you can override that guard temporarily:

```bash
AXIOM_RELEASE_ALLOW_DIRTY=true bash ./scripts/release_prep.sh check v0.1.0-alpha.1
```

## 2. Generate draft release notes

Create a draft markdown file for the release:

```bash
bash ./scripts/release_prep.sh draft-notes v0.1.0-alpha.1
```

This writes a file under:

```text
release-notes/v0.1.0-alpha.1.md
```

The draft is seeded from:

```text
docs/dev/RELEASE_NOTES_TEMPLATE.md
```

Review and edit it before publishing.

## 3. Combined release-prep flow

To run the full release-prep workflow at once:

```bash
bash ./scripts/release_prep.sh all v0.1.0-alpha.1
```

That will:
- validate release-prep prerequisites
- generate draft release notes
- print the exact `git` and `gh` commands to create the release

It does **not** create tags or publish releases automatically.

## 4. Tag and draft the GitHub release

After editing the generated notes file, run the printed commands, which will look like this:

```bash
git tag -a v0.1.0-alpha.1 -m "Axiom v0.1.0-alpha.1"
git push origin v0.1.0-alpha.1

gh release create v0.1.0-alpha.1 \
  --draft \
  --title "Axiom v0.1.0-alpha.1" \
  --notes-file "release-notes/v0.1.0-alpha.1.md"
```

## 5. Release note content requirements

Release notes should always include:
- a clear alpha-stage positioning statement
- the recommended nested/windowed evaluation path
- known limitations
- major user-visible improvements
- any packaging/session-wrapper changes
- any build or upgrade notes

## 6. Relationship to other docs

Before publishing, make sure these stay aligned:
- `docs/dev/RELEASE_CHECKLIST.md`
- `docs/user/LIMITATIONS.md`
- `README.md`
- `docs/dev/BUILD.md`
