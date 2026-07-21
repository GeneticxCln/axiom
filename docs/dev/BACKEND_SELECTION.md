# Backend Selection

Axiom has a **single runtime backend: winit** (nested/windowed). The
standalone DRM/KMS backend, the Noop/test backend, and the libinput path were
removed during the cleanup that stripped over-engineering. There is no
`--backend` flag and no `BackendKind::Drm` — `backend.kind` in the config
defaults to `"winit"` and is the only accepted value.

## Winit backend (nested / windowed) — the only backend

Runs Axiom nested inside another graphical session. This is the complete,
recommended path for day-to-day development and the only way to run Axiom
today.

### Current capabilities
- Wayland socket creation
- XDG toplevel and popup handling
- compositor shortcuts and input routing
- GLES rendering through the winit window (real client pixels + SSD titlebars)
- live resize via `WinitEvent::Resized`
- IPC integration

### Example
```bash
cargo run -- --debug
```

## Removed backends (historical)

These were deleted and no longer exist in the codebase:

- **DRM/KMS backend** (`--backend=drm`): device probing, KMS output
  enumeration, libinput setup, udev hotplug, and dumb-buffer scanout were all
  removed. There is no standalone/session compositor path.
- **Noop backend** (`--backend=noop`): the headless/test mode was removed.
  Integration tests now exercise the winit path under `xvfb-run`.

## Feature flags and protocol notes

### Cargo features
Current manifest features:
- `default`
- `examples`

### XDG decoration protocol
The compositor negotiates `zxdg_decoration_manager_v1` and renders
**server-side decorations** (titlebars + buttons) directly in the GLES output
path. The `enable_xdg_decoration_protocol` gate is retained for compatibility.

### Minimize feature gate
The titlebar minimize affordance is intentionally gated behind:

```toml
[features]
enable_minimize = true
```

This remains off by default to keep scope manageable while lifecycle and
protocol behavior are still being stabilized.
