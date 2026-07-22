# Axiom Architecture

## Overview

Axiom is an **alpha-stage Wayland compositor** built on **Smithay 0.7** with a
**winit-only backend** and **GLES rendering** via Smithay's `GlesRenderer`. It
implements a **scrollable workspace** model inspired by niri, where workspaces
are arranged as an infinite horizontal strip of columns and the user scrolls
between them.

Axiom is single-output only (winit window). There is no DRM/KMS backend, no
XWayland, and no separate renderer process. The compositor runs in a single
thread with a calloop-based event loop driven by a frame-pacing timer.

### Current state

- Alpha software — not ready for production desktop sessions.
- Winit-only: runs as a nested compositor window on an existing Wayland or X11
  session.
- Real client pixels are shown: each client's committed `wl_buffer` is imported
  into a `GlesTexture` and drawn alongside a solid backdrop and server-side
  decoration (SSD) titlebars/buttons.
- All subsystems (workspaces, windows, input, IPC) are wired and tested.

---

## Architecture Diagram

```
  +-----------------+       +----------------------+       +-----------------+
  | External IPC     |<----->|   AxiomCompositor    |<----->|  Wayland Clients |
  | Clients (Lazy UI)|  JSON |   (main loop)        |  Way- |  (xdg_toplevel, |
  +-----------------+  Unix |                       |  land |   layer_shell,  |
                            |  +-----------------+  |  sock.|   etc.)         |
                            |  | AxiomIPCServer   |  |       +-----------------+
                            |  | - socket accept  |  |
                            |  | - peer cred check |  |
                            |  | - cmd whitelist  |  |
                            |  +-----------------+  |
                            |         |              |
                            |         v              |
                            |  Calloop EventLoop     |
                            |  +----+---------+--+   |
                            |  |Timer| Signals |  |   |
                            |  +----+---------+--+   |
                            |         |              |
                            |   tick() every frame    |
                            |         |              |
                            |  +------+-----------+  |
                            |  | process_events() |  |
                            |  +------+-----------+  |
                            |         |              |
                            |  +------+-----------+  |
                            |  | render()         |  |
                            |  | - scene prep     |  |
                            |  | - GlesRenderer   |  |
                            |  | - damage submit  |  |
                            |  +------+-----------+  |
                            +-----------------------+
                                        |
                          +-------------+-------------+
                          |             |             |
                          v             v             v
                   +----------+  +--------+  +-------------+
                   | Workspace|  | Window |  | Input       |
                   | manager  |  | manager|  | manager     |
                   +----------+  +--------+  +-------------+
                          |             |             |
                          v             v             v
                   +----------+  +--------+  +-------------+
                   |Workspace |  | Axiom  |  | Keybindings |
                   |Tape +    |  |Window  |  | + dispatch  |
                   |Columns   |  |registry|  |             |
                   +----------+  +--------+  +-------------+
```

---

## Module Responsibilities

### `src/main.rs` — Entry point

CLI argument parsing (clap), config loading, subsystem initialization, systemd
`READY=1` notification. Creates all managers, wires them into `AxiomCompositor`,
sets `WAYLAND_DISPLAY`, and starts the event loop.

### `src/compositor.rs` — `AxiomCompositor`

Core orchestrator. Owns the calloop `EventLoop` with two event sources:
- `Signals` (SIGTERM, SIGINT) for graceful shutdown.
- `Timer` for frame pacing at the configured `max_fps`.

Public methods for workspace scrolling, window add/remove, minimize/restore,
fullscreen toggle, viewport resize, clipboard set. IPC message processing and
workspace command dispatch. Tick lifecycle: `process_events()` then
`render_frame()`.

### `src/backend/mod.rs` — `AxiomSmithayBackendReal` + `State`

The largest module (~2300 lines). Implements all Smithay handler traits:
`CompositorHandler`, `XdgShellHandler`, `XdgDecorationHandler`,
`WlrLayerShellHandler`, `SeatHandler`, `DataDeviceHandler`, `SessionLockHandler`,
`ForeignToplevelListHandler`, `ShmHandler`, `FractionalScaleHandler`,
`OutputHandler`. Manages:
- Winit backend lifecycle (`initialize_winit`, `run_one_cycle`, `shutdown`).
- Wayland display socket creation and event dispatch.
- `State` struct holding all protocol state: surface map, toplevel registry,
  output, seat, keyboard/pointer focus, configured sizes.
- Window-to-surface mapping via `window_map`.
- Winit event handling: `Resized` → viewport update, `Redraw` → `needs_redraw`,
  input forwarding to `InputManager`.
- Screencopy capture (`PendingCapture` + `Dispatch` impls for
  `zwlr_screencopy_manager_v1`).
- Drag-and-drop tracking.

### `src/backend/render.rs` — Rendering pipeline

GLES rendering submodule (descendant of `backend`, reads private fields of
`State` and `AxiomSmithayBackendReal`).

`render()` method:
1. Binds the winit GLES backend frame.
2. Calls `State::prepare_render_scene()` to calculate layouts, synchronize
   window geometry, and notify clients of size changes.
3. Renders a solid backdrop color.
4. For each visible window: collects SSD titlebar/button render elements
   (`SolidColorRenderElement`) and client surface render elements
   (`TextureRenderElement`), renders them in the correct z-order.
5. Damage tracking via `output.damage_formed()` with bounding-box merge.
6. Submits to the winit backend.

Separate `capture_screencopy()` for `zwlr_screencopy_manager_v1` SHM capture,
called after the normal scene composite. Uses `ExportMem::copy_framebuffer` to
read the already-rendered GLES backbuffer and writes to client SHM via
`with_buffer_contents_mut`.

### `src/backend/input.rs` — Input event dispatch

Routes winit input events (pointer motion, button, axis, keyboard, touch)
through Smithay's input pipeline. Handles:
- Pointer focus tracking and surface-relative coordinate conversion.
- Decoration hit-testing (titlebar drag, button click).
- Keyboard focus, compositor keybinding dispatch, text input.
- Touch down/motion/up/cancel with window move/resize.
- Touch tap-to-click synthesis.

### `src/backend/clipboard.rs` — Clipboard extraction

Small submodule for Wayland selection data transfer. `create_clipboard_pipe()`
creates a Unix pipe, `spawn_clipboard_read_worker()` reads selection data on a
background thread. `set_clipboard_data()` pushes text into the Wayland clipboard
(from IPC or internal use).

### `src/decoration.rs` — `DecorationManager`

Server-side decoration (SSD) state. Manages:
- `WindowDecoration` per window: mode, title, titlebar height, button state
  (close, maximize, minimize), hit-test regions.
- `DecorationMode`: `ClientSide`, `ServerSide`, or `None`.
- Titlebar/button geometry generation and focus-aware rendering data.
- SSD button hit-testing (consumed presses for close/maximize/minimize).

### `src/config/mod.rs` — Configuration

TOML config loading, parsing, validation. `AxiomConfig` holds sub-configs:
`WorkspaceConfig`, `WindowConfig`, `InputConfig`, `BindingsConfig`,
`BackendConfig`, `FeaturesConfig`. Config defaults are set via `Default` impl;
`::load()` expands `~` and reads from disk, falling back to defaults silently
on parse failure.

### `src/input/mod.rs` — `InputManager`

Key binding definitions (`InputAction` enum: `FocusLeft`, `FocusRight`,
`ScrollLeft`, `ScrollRight`, `Close`, `ToggleFloating`, `ToggleFullscreen`,
`ToggleMinimize`, etc.). Event translation from raw device events to compositor
actions. Keybinding resolution against `BindingsConfig`.

### `src/ipc/mod.rs` — `AxiomIPCServer`

Unix socket JSON IPC server (~1580 lines). Binds to
`$XDG_RUNTIME_DIR/axiom/axiom.sock` (fallback: `/tmp/axiom-<pid>/`). Accepts
concurrent connections (max 16 via semaphore), verifies peer UID, enforces
command whitelist, rate-limits at 64 messages/tick/client, idle-timeouts
inactive connections at 60s.

Message types: `WorkspaceCommand` (10 known actions), `SetClipboard`,
`SetWindowBlur`, `GetConfig`, `SetConfig`, `HealthCheck`,
`GetPerformanceReport`, `LiveMetrics` broadcast. Broadcast channel for state
change notifications (workspace scroll, window add/remove, focus change,
shutdown).

### `src/window/mod.rs` — `WindowManager`

Window registry: `AxiomWindow` records with position, size, properties
(floating, fullscreen, minimized, sticky), focus tracking. Methods for
add/remove, focus management, floating toggle, layout query. `BackendWindow` is
the raw geometry record populated by the Smithay backend.

### `src/workspace/mod.rs` — `ScrollableWorkspaces` + `WorkspaceTape`

Niri-inspired scrollable workspace engine:
- `WorkspaceTape`: a single scrollable strip of `WorkspaceColumn`s. Manages
  window-to-column assignment, scroll position with eased/momentum animation,
  gap calculations, scale factors.
- `ScrollableWorkspaces`: manager holding tape(s), scroll direction dispatch,
  window move between columns, minimize/restore, layout cache.

### `benches/compositor_benchmarks.rs` — Criterion benchmarks

Performance benchmarks for workspace layout, window management, config
operations. Used with cached baselines and regression detection in CI.

### `tests/` — Integration tests

Seven integration test files:
- `clipboard_round_trip.rs` — Wayland clipboard data transfer.
- `dnd_data_transfer.rs` — Drag-and-drop protocol.
- `fullscreen_test.rs` — Fullscreen toplevel behavior.
- `integration_tests.rs` — Broader compositor integration tests.
- `pixel_render.rs` — Pixel-level rendering verification.
- `real_client_smoke.rs` — Real Wayland client smoke test (requires xvfb-run).
- `screencopy_test.rs` — Screencopy protocol capture (requires xvfb-run).

---

## Event Loop Model

The compositor uses a **calloop `EventLoop`** with two event sources:

1. **Signals source** — captures SIGTERM and SIGINT, triggers graceful shutdown
   via `shutdown()`.

2. **Timer source** — fires at the configured frame interval
   (`max_fps` config, default ~60fps, clamped to 1-1000). Each timer tick
   calls `tick()`, which does:

```
tick():
  1. process_events()
     a. smithay_backend.process_events() — dispatch Wayland protocol events,
        winit events, input events.
     b. ipc_server.poll() — accept new connections, read/write pending data.
     c. ipc_server.process_messages() — handle config changes and pending
        actions (workspace commands, clipboard set, blur).
     d. Dispatch pending actions to workspace/window subsystems.
  2. render_frame() — logs frame position info (post-render monitoring).
  3. Update stability metrics: consecutive error count, broadcast performance
     data via IPC, update LiveMetrics snapshot.
  4. If consecutive_error_count >= 5: emergency shutdown.
```

The event loop is **single-threaded**: all work happens inline in the timer
callback. Subsystems are locked via `parking_lot::RwLock` (taken in convention
order: `workspace` → `window_manager` → `decoration_manager` to prevent
inversions).

---

## Rendering Pipeline

```
render():
  1. Bind winit GlesFrame (begin frame)
  2. State::prepare_render_scene():
     - Calculate workspace layouts (column tiling, gaps)
     - Override fullscreen windows to fill viewport
     - Update window geometry, notify clients via configure
     - Handle fractional scaling
  3. Clear with backdrop color
  4. For each window in the focused workspace column:
     a. Collect SSD render elements:
        - Titlebar background (SolidColorRenderElement)
        - Titlebar buttons (close/maximize/minimize, colored quads)
        - Window title text (if font atlas available)
     b. If window has a committed wl_buffer:
        - Import into GlesTexture (TextureBuffer)
        - Collect as TextureRenderElement
        - Otherwise: skip (no placeholder drawn)
     c. Render elements in z-order: backdrop → titlebar → client content
  5. Output damage tracking via output.damage_formed()
  6. Present via winit backend (submit GLES frame)
  7. Screencopy capture (if pending): ExportMem::copy_framebuffer → client SHM
```

Texture caching is done via an `lru` cache keyed on `WlBuffer` IDs. Rendering
uses `GlesRenderer` from Smithay, with `SolidColorRenderElement` for solid
quads (backdrop, titlebars, buttons) and `TextureRenderElement` for client
surface content.

---

## IPC Architecture

`AxiomIPCServer` provides JSON-based communication over Unix domain sockets:

### Socket
- Path: `$XDG_RUNTIME_DIR/axiom/axiom.sock` (preferred) or
  `/tmp/axiom-<pid>/axiom-lazy-ui.sock` (fallback).
- Directory permissions: `0o700`. Socket file: `0o600`.

### Security
- UID-based peer credential verification (`SO_PEERCRED`).
- Connection semaphore (max 16 concurrent).
- Command whitelist: `KNOWN_WORKSPACE_ACTIONS` (10 actions) — unknown actions
  rejected with `unknown_action` ACK.
- Rate limit: 64 messages per tick per client.
- Idle timeout: 60s.
- Maximum write buffer: 1 MiB per client.

### Message flow
```
Client → [Unix socket] → AxiomIPCServer → cmd_tx channel → AxiomCompositor
                                                                    ↓
AxiomCompositor ← broadcast_tx ← AxiomIPCServer ← [Unix socket] ← Client
```

### Request types
- `WorkspaceCommand { action, parameters }` — scroll, add/remove/move windows,
  toggle floating/fullscreen/minimize.
- `SetClipboard { text }` — push text into Wayland clipboard.
- `SetWindowBlur { window_id, radius }` — set blur radius.
- `GetConfig` / `SetConfig { config }` — read/write compositor config.
- `HealthCheck` — returns compositor health + live metrics.
- `GetPerformanceReport` — returns frame time, active windows, workspace index.

### Push broadcasts
State change events (workspace scroll, window add/remove, focus change,
shutdown) and periodic `LiveMetrics` are broadcast to all connected clients.

---

## Key Data Structures

### `AxiomSmithayBackendReal` (`src/backend/mod.rs`)
The central backend struct. Fields (in order, enforced by
`static_assertions::assert_fields!`):
- `state: State` — all protocol/compositor state.
- `winit_backend: Option<WinitGraphicsBackend<GlesRenderer>>` — the winit
  GLES backend handle (present during winit mode, `None` for noop).
- `winit_event_loop: Option<WinitEventLoop<()>>` — the winit event loop handle.
- `backend_kind: BackendKind` — `Winit` or `Noop`.
- `socket_name: String` — the Wayland display socket name.
- `display_handle: DisplayHandle` — Smithay display handle.
- `shutdown_initiated: bool` — graceful shutdown flag.

### `State` (`src/backend/mod.rs`)
Held inside `AxiomSmithayBackendReal`. Contains:
- `compositor_state: CompositorState`, `xdg_shell_state: XdgShellState`,
  `data_device_state: DataDeviceState`, `shm_state: ShmState`,
  `session_lock_state: SessionLockManagerState`,
  `fractional_scale_manager: FractionalScaleManagerState`,
  `layer_shell_state: WlrLayerShellState`, `seat_state: SeatState`,
  `foreign_toplevel_list_state: ForeignToplevelListState`,
  `xdg_decoration_state: XdgDecorationState`.
- `output: Output` — the single output.
- `seat: Seat<Self>` — the single seat (keyboard + pointer + touch).
- `window_map: HashMap<u64, ObjectId>` — maps Axiom window IDs to Wayland
  surface IDs.
- `toplevels: HashMap<ObjectId, ToplepleSurface>` — surface ID to toplevel.
- `surfaces: HashMap<ObjectId, SurfaceData>` — surface state.
- `configured_sizes: HashMap<ObjectId, (i32, i32)>` — pending configure sizes.
- `window_width, window_height: u32` — output dimensions.
- `needs_redraw: bool` — dirty flag.
- `clipboard_data: Option<Vec<u8>>` — cached clipboard content.
- `texture_cache: lru::LruCache<ObjectId, GlesTexture>` — client texture cache.
- Shared managers: `workspace_manager`, `window_manager`, `input_manager`,
  `decoration_manager`.

### `AxiomCompositor` (`src/compositor.rs`)
The top-level compositor orchestrator:
- `config: AxiomConfig`
- `running: bool`
- `workspace_manager: Arc<RwLock<ScrollableWorkspaces>>`
- `window_manager: Arc<RwLock<WindowManager>>`
- `input_manager: Arc<RwLock<InputManager>>`
- `ipc_server: AxiomIPCServer`
- `decoration_manager: Arc<RwLock<DecorationManager>>`
- `smithay_backend: AxiomSmithayBackendReal`
- `consecutive_error_count: u32` — stability tracking (>=5 triggers shutdown).

### `AxiomIPCServer` (`src/ipc/mod.rs`)
- `listener: UnixListener` — the bound socket.
- `clients: HashMap<RawFd, IpcClient>` — active connections.
- `cmd_tx: mpsc::Sender<LazyUIMessage>` — channel to compositor.
- `broadcast_tx: broadcast::Sender<IpcBroadcastEvent>` — push broadcast channel.
- `config_handle: Arc<RwLock<AxiomConfig>>` — live config snapshot.
- `live_metrics: Arc<RwLock<LiveMetrics>>` — latest metrics snapshot.
- `connection_count: Arc<AtomicUsize>` — semaphore-backed limit.

### `ScrollableWorkspaces` + `WorkspaceTape` (`src/workspace/mod.rs`)
- `WorkspaceTape`: columns `VecDeque<WorkspaceColumn>`, current scroll position,
  target position, velocity (for momentum scrolling), viewport dimensions,
  layout cache.
- `ScrollableWorkspaces`: holds one `WorkspaceTape`, provides scroll direction
  methods, window move between columns, minimize/restore, layout calculation.

### `WindowManager` (`src/window/mod.rs`)
- `windows: HashMap<u64, AxiomWindow>` — window registry.
- `next_id: u64` — monotonic ID counter.
- `focused_window: Option<u64>` — current focus.

---

## Testing Strategy

### Unit tests
- Inline `#[cfg(test)] mod tests` in each module: compositor workspace
  operations, config parsing/validation, input event dispatch, decoration
  geometry, workspace layout invariants.
- Run via `cargo test`.

### Property-based tests
- Config module uses `proptest` for property-based validation of config parsing
  invariants (`src/config/property_tests.rs`).

### Integration tests (`tests/`)
Seven integration test files run via `cargo test --test <name>`:
- `clipboard_round_trip.rs` — Wayland clipboard data push/pull.
- `dnd_data_transfer.rs` — drag-and-drop data transfer.
- `fullscreen_test.rs` — fullscreen toplevel lifecycle.
- `integration_tests.rs` — compositor integration (window lifecycle,
  workspace scrolling, error recovery).
- `pixel_render.rs` — pixel-level output verification.
- `real_client_smoke.rs` — real Wayland client connect and render (requires
  `xvfb-run`).
- `screencopy_test.rs` — `zwlr_screencopy_manager_v1` capture (requires
  `xvfb-run`).

### Headless testing
`BackendKind::Noop` provides a headless backend for CI and unit tests that
creates no winit window and performs no rendering.

### Benchmarks
Criterion benchmarks in `benches/compositor_benchmarks.rs` for workspace,
window, and config operations. CI caches baselines and detects regressions.

### CI pipeline (`.github/workflows/ci.yml`)
6+ job matrix including:
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`.
- `cargo test` on stable/beta/nightly with multiple feature combinations.
- `xvfb-run` for headless integration tests.
- Code coverage via cargo-tarpaulin.
- Security audit via cargo-audit / cargo-deny.
- Criterion benchmark baseline comparison.
