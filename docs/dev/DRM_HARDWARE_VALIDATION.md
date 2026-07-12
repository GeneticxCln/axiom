# DRM/KMS Hardware Validation

This document tracks the **real-hardware validation** status of Axiom's standalone DRM/KMS backend.

## Why this exists

Axiom's DRM path now has an early compositor-output path, but repository-side logic and CI are not enough to claim it is broadly validated. Real hardware validation is still required for:
- connector discovery
- scanout reliability
- input/session behavior
- hotplug handling
- multi-output layout correctness
- fractional scale / HiDPI sanity
- clean shutdown back to the host session/VT

## Current status

**Repository status today:**
- DRM/KMS backend exists
- output probing exists
- libinput integration exists
- udev hotplug monitoring exists
- multi-output layout scaffolding exists
- early fractional scaling exists
- CPU dumb-buffer scanout from WGPU-composed frames exists

**What is still missing:**
- broad real-hardware validation across GPUs/connectors/setups
- confidence matrix for common hardware combinations
- documented pass/fail/untested results per machine

## Validation rule

Until the matrix below has real pass results on actual hardware, the DRM path should still be described as:

> **early standalone alpha path, not the primary recommended runtime target**

## Recommended validation workflow

### 1. Capture a machine snapshot

Use the helper:

```bash
bash ./scripts/drm_validation_report.sh probe
```

To generate a report stub you can fill in after testing:

```bash
bash ./scripts/drm_validation_report.sh report
```

This creates a markdown file under:

```text
drm-validation-reports/
```

### 2. Run the standalone DRM backend

```bash
cargo run -- --backend=drm
```

Prefer running this from a real seat/session where `/dev/dri/*` and input devices are accessible.

### 3. Record results for these scenarios

- standalone startup succeeds
- input devices work
- at least one output scans out compositor content
- output unplug/replug is handled
- multi-output layout is usable
- mixed DPI / fractional scale behavior is sane enough for alpha
- shutdown returns control cleanly

## Validation matrix

Update this table as real hardware results come in.

| Machine / GPU | Outputs tested | Startup | Input | Hotplug | Multi-output | Fractional scale | Shutdown | Notes |
|---|---|---|---|---|---|---|---|---|
| _No validated hardware results recorded yet_ | — | TODO | TODO | TODO | TODO | TODO | TODO | Repository-side implementation exists, but no committed real-hardware pass matrix yet |

## Minimum evidence for “DRM validated enough for alpha”

The following should exist before strengthening public claims about the standalone path:

- at least one successful **single-output** machine report
- at least one successful **multi-output** machine report
- at least one report covering **hotplug remove + re-add**
- at least one report covering **fractional scale / HiDPI** behavior
- clear notes on any GPU/vendor-specific issues encountered

## Relationship to other docs

See also:
- `docs/dev/BACKEND_SELECTION.md`
- `docs/user/LIMITATIONS.md`
- `docs/dev/RELEASE_CHECKLIST.md`
- `MASTER_DEVELOPMENT_PLAN.md`
