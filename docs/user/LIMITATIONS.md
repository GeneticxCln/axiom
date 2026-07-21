# Known Limitations

Axiom is a **winit-only Wayland compositor** with real GLES rendering and server-side decorations. The nested/windowed path is the complete, recommended evaluation target.

**Recently completed:** Drag-and-drop protocol (client-initiated sessions with icon rendering), touch input handling (down/motion/up/cancel with touch-based window move/resize), and compositor→clipboard push via `SetClipboard` IPC command are now implemented.

## Recommended usage

Use Axiom primarily in nested mode:

```bash
cargo run -- --windowed --debug
```

## Current limitations

### Rendering
- Rendering uses winit + GLES (not WGPU or DRM/KMS). GPU acceleration is available via OpenGL.
- Render performance has been hardened with texture caching (each client buffer is imported once), but full-frame redraws still happen (no damage tracking).
- Pixel-level verification (scale/layout correctness, titlebar/content overlap) requires `xvfb-run` (CI) — local tests use headless `Noop` backend.

### Decorations
- Server-side decorations (titlebars + close/maximize/minimize buttons) are rendered and functional.
- Title text uses system fonts when available; falls back to no text gracefully.

### Multi-monitor / HiDPI
- Single output only (hardcoded 1920×1080 virtual size). Multi-output infrastructure exists but is not wired.
- Fractional scale is advertised to clients but sourced from the workspace tape, not the output.

### Clipboard
- Wayland→compositor clipboard works (tested: real client offers selection → compositor receives).
- Compositor→Wayland clipboard is triggerable via the `SetClipboard` IPC command, wired end-to-end.

### IPC
- Unix-socket JSON IPC with UID peer check and action whitelist.

## What to use Axiom for right now

Good use cases:
- Evaluating/niri-style scrollable workspace logic
- Testing real Wayland client flow (surface commit → compositor tracking)
- Experimenting with server-side decoration geometry
- Contributing fixes and tests to a real compositor

Less suitable use cases:
- Replacing your daily standalone compositor (single-output, nested only)
- Expecting polished multi-monitor behavior