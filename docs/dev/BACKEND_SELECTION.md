# Backend Selection

Axiom currently supports three runtime backend modes.

## 1. Winit backend (--windowed / --backend=winit) — recommended

WGPU surface rendering: windows + effects rendered directly to the winit window via a wgpu::Surface. Zero CPU readback.

The transitional GL bridge has been removed.

### Example
cargo run -- --windowed --debug

## 2. DRM/KMS backend (--backend=drm) — alpha

CPU software composite from buffer_cache to dumb-buffer. No WGPU, no effects. Multi-output, fractional scale.

### Example
cargo run -- --backend=drm

## 3. Noop backend (--backend=noop) — headless/test

## Accepted backend values

winit (windowed, dev) | drm (kms, session, tty) | noop (test, headless)

## Feature flags

default | examples | demo

xdg_decoration_protocol: off by default, negotiates CSD when enabled.
enable_minimize: off by default.

Axiom currently supports three runtime backend modes.

## 1. Winit backend (`--windowed` / `--backend=winit`)

This is the **recommended alpha target**.

It runs Axiom nested inside another graphical session and is the most complete path for day-to-day development.

### Current capabilities
- Wayland socket creation
- XDG toplevel and popup handling
- compositor shortcuts and input routing
- WGPU-based off-screen composition with transitional GL presentation
- IPC integration

See [Render Architecture](RENDER_ARCHITECTURE.md) for the rendering direction: WGPU is the primary compositor path, while GL remains a presentation bridge for now.

### Example
```bash
cargo run -- --windowed --debug
```

---

## 2. DRM/KMS backend (`--backend=drm`)

This is the intended standalone/session-compositor path.

### Current state
The DRM backend now has an early standalone compositor output path in place:
- DRM device probing
- KMS output enumeration
- libinput setup
- udev hotplug monitoring
- per-output scale/tape scaffolding
- workspace tape synchronization across hotplug/re-enumeration
- simple horizontal multi-output virtual-desktop coordinates
- early fractional output scale / HiDPI support
- WGPU-composed frames copied into CPU-writable dumb-buffer scanout

However, it is **still not the recommended runtime target** because the standalone path remains alpha-quality and needs more validation, optimization, and multi-output hardening compared with nested mode.

For the current real-hardware validation workflow and status matrix, see [DRM Hardware Validation](DRM_HARDWARE_VALIDATION.md).

### Example
```bash
cargo run -- --backend=drm
```

Use for development/testing only until the standalone render path is completed and validated.

---

## 3. Noop backend (`--backend=noop`)

A headless/test-oriented mode used primarily by tests.

### Current use
- constructing compositor/backend state without full system resources
- unit-style and integration-style test support

---

## Accepted backend values

| Value | Meaning |
|---|---|
| `winit` | Nested/windowed backend |
| `drm` | Standalone DRM/KMS backend |
| `noop` | Headless/test backend |

Aliases currently accepted in config/CLI parsing:
- `windowed`, `dev` → `winit`
- `kms`, `session`, `tty` → `drm`
- `test`, `headless` → `noop`

Unknown values fall back to `winit` with a warning.

---

## Feature flags and protocol notes

### Cargo features
Current manifest features:
- `default`
- `examples`
- `demo`

### XDG decoration protocol
The compositor can conditionally register `zxdg_decoration_manager_v1` when:

```toml
[features]
enable_xdg_decoration_protocol = true
```

However, visible server-side decoration rendering is **not yet part of the live compositor output path**. When this protocol is enabled, Axiom currently negotiates **client-side decorations** rather than claiming visible SSD support. Treat this as experimental until live decoration rendering is integrated.

### Minimize feature gate
The titlebar minimize affordance is intentionally gated behind:

```toml
[features]
enable_minimize = true
```

This remains off by default to keep scope manageable while lifecycle and protocol behavior are still being stabilized.
