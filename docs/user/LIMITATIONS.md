# Known Limitations

Axiom is currently an **alpha compositor prototype**. The nested/windowed path is the intended evaluation target.

## Recommended usage

Use Axiom primarily in nested mode:

```bash
cargo run -- --windowed --debug
```

## Current limitations

### Standalone DRM/KMS
- The DRM/KMS path is not yet the recommended runtime target.
- An early standalone compositor output path now exists using WGPU-composed frames copied into CPU-writable dumb-buffer scanout.
- It still needs broader validation, optimization, and multi-output hardening.
- There is not yet a committed real-hardware pass matrix showing broad validation across GPUs/connectors/setups.

### Rendering
- The nested path still uses a transitional WGPU-first composition flow with a compatibility presentation bridge.
- Some performance work remains before the render/present architecture is considered settled.

### Decorations
- Visible server-side decoration rendering is not yet integrated into the live compositor output path.
- When xdg-decoration negotiation is enabled, the compositor currently prefers client-side decorations instead of claiming visible SSD support.

### Multi-monitor / HiDPI
- Multi-output layout now uses a simple horizontal virtual-desktop strip.
- Output scale factors can now be fractional, and the compositor can advertise preferred fractional surface scales to capable clients.
- More advanced topology handling and broader validation are still pending.
- Fractional scaling support is still early and needs more real-world verification.

### XWayland
- XWayland infrastructure exists, and clipboard exchange now works for:
  - Wayland selections served to X11
  - compositor-owned clipboard data served back to Wayland clients
  - best-effort ingestion of external X11 clipboard owners into compositor/Wayland clipboard state
- Expect rough edges with some X11 applications, especially beyond the currently tested lifecycle/metadata/clipboard paths.

### Packaging / release state
- Packaging assets are still being completed.
- The project should be treated as alpha-quality software, not a stable desktop session replacement.

## What to use Axiom for right now

Good use cases:
- evaluating compositor architecture
- testing nested Wayland client flow
- experimenting with workspace logic and renderer behavior
- contributing fixes and tests

Less suitable use cases today:
- replacing your daily standalone compositor session
- expecting polished multi-monitor or HiDPI behavior
- relying on complete XWayland compatibility
