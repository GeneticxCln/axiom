# Screencopy Protocol Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement `zwlr_screencopy_manager_v1` version 1 (SHM-only capture) so clients like `grim` can capture the composited output.

**Architecture:** The protocol dispatch stores a pending capture request on `State`. During the next `render()` call, after composing the scene into the winit backbuffer, pixels are read via `ExportMem::copy_framebuffer` and written to the client's SHM `wl_buffer` via `with_buffer_contents_mut`. No offscreen pass needed — reads from the already-rendered frame in the same render cycle.

**Tech Stack:** Smithay 0.7 (renderer_gl, wayland_frontend), wayland-protocols-wlr 0.3 (re-exported through smithay), wayland-server 0.31 (Dispatch/GlobalDispatch traits)

## Global Constraints

- V1 SHM-only — no damage tracking, no linux-dmabuf
- One-shot frames: `ready` or `failed` fires once, frame is done
- Capture output only (no region capture)
- Cursor overlay disabled (0) — cursor is already composited in the scene

---

### Task 1: Pending capture state + render integration

**Files:**
- Modify: `src/backend/render.rs` — add capture helper + call from render()
- Modify: `src/backend/mod.rs` — add PendingCapture struct + pending_capture field to State

**Interfaces:**
- Consumes: `State` struct, `AxiomSmithayBackendReal::render()` method
- Produces: `state.pending_capture: Option<PendingCapture>`, `PendingCapture` struct, `capture_screencopy()` method on `AxiomSmithayBackendReal`

- [ ] **Step 1: Add PendingCapture struct to State**

In `src/backend/mod.rs`, add to the State struct (around line 250, near `needs_redraw`):

```rust
/// A pending screencopy capture request, stored during `copy` dispatch
/// and processed during the next render cycle.
pub struct PendingCapture {
    /// The frame resource to send ready/failed on
    pub frame: ZwlrScreencopyFrameV1,
    /// The client's wl_buffer (SHM) to write pixel data into
    pub buffer: WlBuffer,
    /// Output dimensions (must match the buffer)
    pub size: Size<i32, BufferCoord>,
}

// In State struct:
pending_capture: Option<PendingCapture>,
```

Also add the import at the top of `mod.rs`:
```rust
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::{BufferCoord, Size};
```

Add the field initialization where State is constructed (search for `needs_redraw: true` and add after it):
```rust
pending_capture: None,
```

- [ ] **Step 2: Add capture_screencopy method to backend**

In `src/backend/render.rs`, add a method on `AxiomSmithayBackendReal`:

```rust
/// Capture the current composited frame into a pending screencopy buffer.
///
/// Called from `render()` after `render_scene_into()` has composed into the
/// winit backbuffer. Reads pixels from the backbuffer via `ExportMem::copy_framebuffer`,
/// writes them into the client's SHM buffer, and sends `ready`/`failed` on the frame.
fn capture_screencopy(
    &mut self,
    renderer: &mut GlesRenderer,
    framebuffer: &mut GlesTarget<'_>,
) {
    use smithay::backend::renderer::ExportMem;
    use smithay::backend::allocator::Fourcc;
    use smithay::wayland::shm::with_buffer_contents_mut;
    use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1;

    let Some(capture) = self.state.pending_capture.take() else { return; };

    let region = Rectangle::from_loc_and_size(
        (0, 0),
        (capture.size.w, capture.size.h),
    );

    match renderer.copy_framebuffer(framebuffer, region, Fourcc::Argb8888) {
        Ok(mapping) => {
            match renderer.map_texture(&mapping) {
                Ok(pixels) => {
                    // Write to the client's SHM buffer
                    let result = with_buffer_contents_mut(&capture.buffer, |ptr, len, _data| {
                        let dest = unsafe {
                            std::slice::from_raw_parts_mut(ptr as *mut u8, len)
                        };
                        let copy_len = pixels.len().min(dest.len());
                        dest[..copy_len].copy_from_slice(&pixels[..copy_len]);
                    });
                    match result {
                        Ok(()) => {
                            use std::time::{SystemTime, UNIX_EPOCH};
                            let now = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default();
                            capture.frame.flags(zwlr_screencopy_frame_v1::Flags::YInvert.bits());
                            capture.frame.ready(
                                (now.as_secs() >> 32) as u32,
                                (now.as_secs() & 0xFFFF_FFFF) as u32,
                                now.subsec_nanos(),
                            );
                        }
                        Err(e) => {
                            warn!("Screencopy SHM write failed: {:?}", e);
                            capture.frame.failed();
                        }
                    }
                }
                Err(e) => {
                    warn!("Screencopy map_texture failed: {:?}", e);
                    capture.frame.failed();
                }
            }
        }
        Err(e) => {
            warn!("Screencopy copy_framebuffer failed: {:?}", e);
            capture.frame.failed();
        }
    }
}
```

Add these imports at the top of `render.rs` (if not already present):
```rust
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::ExportMem;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::utils::{BufferCoord, Size};
```

- [ ] **Step 3: Call capture_screencopy from render()**

In `src/backend/render.rs`, inside `AxiomSmithayBackendReal::render()`, after `render_scene_into` returns and before `backend.submit()`:

```rust
        {
            let (mut renderer, mut framebuffer) = backend.bind()?;
            render_scene_into(&mut self.state, &mut renderer, &mut framebuffer)?;
            // Capture screencopy from the just-rendered frame (before submit)
            self.capture_screencopy(&mut renderer, &mut framebuffer);
        }
        // ... existing submit/damage code follows ...
```

Currently the bind/render is in a block `{ let (mut renderer, mut framebuffer) = backend.bind()?; render_scene_into(...)?; }` — just add the `capture_screencopy` call inside that block after `render_scene_into`.

- [ ] **Step 4: Build check**

Run: `cargo build 2>&1 | head -40`
Expected: Clean build (warnings allowed for unused imports that will be used in Task 2)

---

### Task 2: Protocol dispatch implementations

**Files:**
- Modify: `src/backend/mod.rs` — add GlobalDispatch + Dispatch impls, create global in init

**Interfaces:**
- Consumes: `PendingCapture`, `State`, display handle
- Produces: working screencopy protocol handler

- [ ] **Step 1: Add imports for protocol types**

In `src/backend/mod.rs`, add imports at the top (near existing `use` lines):

```rust
use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
};
use smithay::reexports::wayland_server::{Client, DataInit, Dispatch, GlobalDispatch};
```

- [ ] **Step 2: Implement GlobalDispatch for the manager**

Add anywhere in `mod.rs` (before the delegate macros, around line 1220):

```rust
// ── Screencopy protocol (zwlr_screencopy_manager_v1, V1 SHM-only) ──

impl GlobalDispatch<ZwlrScreencopyManagerV1, State, ()> for State {
    fn bind(
        _state: &mut State,
        _dh: &DisplayHandle,
        _client: &Client,
        _resource: &ZwlrScreencopyManagerV1,
        _data: &(),
        _data_init: &mut DataInit<'_, State>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, (), State> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        _resource: &ZwlrScreencopyManagerV1,
        request: &<ZwlrScreencopyManagerV1 as Resource>::Request,
        _data: &(),
        dh: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        use smithay::reexports::wayland_server::Resource as WlResource;

        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput { frame, cursor: _, output: _ } => {
                let w = state.window_width;
                let h = state.window_height;

                // Validate output dimensions
                if w <= 0 || h <= 0 {
                    warn!("Screencopy: output has zero area, refusing capture");
                    return;
                }

                // Initialize the new frame resource
                data_init.init(frame, ());

                // Send buffer params — stride is width * 4 (RGBA8, 4 bytes per pixel)
                let stride = w * 4;
                frame.buffer(w, h, stride);
                // buffer_done signals end of buffer type enumeration (v3+)
                // In v1 we send it anyway — clients ignore unknown events
                frame.buffer_done();
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion { .. } => {
                warn!("Screencopy: capture_output_region not supported in V1");
            }
            zwlr_screencopy_manager_v1::Request::Destroy => {
                // Nothing to clean up — frame cleanup is per-frame
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 3: Implement Dispatch for the frame**

```rust
impl Dispatch<ZwlrScreencopyFrameV1, (), State> for State {
    fn request(
        state: &mut State,
        _client: &Client,
        _resource: &ZwlrScreencopyFrameV1,
        request: &<ZwlrScreencopyFrameV1 as Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => {
                if state.pending_capture.is_some() {
                    warn!("Screencopy: already have a pending capture, ignoring duplicate");
                    return;
                }
                let w = state.window_width;
                let h = state.window_height;
                if w <= 0 || h <= 0 {
                    warn!("Screencopy: cannot capture, output has zero area");
                    // Can't send failed() here without the frame resource
                    // The render cycle won't find a pending capture
                    return;
                }
                state.pending_capture = Some(PendingCapture {
                    frame: _resource.clone(),
                    buffer: buffer.clone(),
                    size: Size::from((w, h)),
                });
                state.needs_redraw = true;
            }
            zwlr_screencopy_frame_v1::Request::Destroy => {
                // Clear pending capture if this frame was queued
                if let Some(ref pc) = state.pending_capture {
                    if pc.frame.id() == _resource.id() {
                        state.pending_capture = None;
                    }
                }
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 4: Create the global in init**

Find where the compositor creates other globals (e.g., session lock state initialization). Search for `SessionLockManagerState::new` or similar. Add after that:

```rust
dh.create_global::<State, ZwlrScreencopyManagerV1, _>(1, ()).unwrap();
```

Add right after the other globals are created (around line 1430-1480 in `src/backend/mod.rs` inside `run_one_cycle` or the init function).

Also need to add `use smithay::reexports::wayland_server::Resource as WlResource;` — either at the top of the file or locally in the Dispatch impl. Since `Resource` is already used elsewhere, it should be a top-level import.

- [ ] **Step 5: Build check**

Run: `cargo build 2>&1 | head -60`
Expected: Clean build (0 errors, 0 warnings)

---

### Task 3: Integration test

**Files:**
- Modify: `tests/integration_tests.rs` — add screencopy capture test

This test follows the `pixel_render.rs` pattern: starts the compositor on the
Winit backend (requires a display / `xvfb-run`), connects a real Wayland
client that exercises the full screencopy protocol flow (bind manager → create
frame → receive buffer params → create SHM buffer → send copy → receive
ready/failed).

- [ ] **Step 1: Add dev-dependency on wayland-protocols-wlr**

In `Cargo.toml`, add after the existing `wayland-protocols` dev-dependency:

```toml
wayland-protocols-wlr = { version = "0.3", features = ["client"] }
```

- [ ] **Step 2: Write the test in tests/screencopy_test.rs**

Create new file `tests/screencopy_test.rs`:

```rust
//! Screencopy protocol integration test.
//!
//! Verifies that `zwlr_screencopy_manager_v1` version 1 SHM capture works end-to-end.
//! The compositor runs on the Winit/GL backend (needs display), connects a real
//! Wayland client with a visible surface, captures the composited scene, and
//! asserts the capture received pixel data (via the `ready` event).
//!
//! Run with:
//! ```text
//! xvfb-run -a cargo test --test screencopy_test -- --ignored --nocapture
//! ```

use anyhow::Result;
use axiom::{
    backend::AxiomSmithayBackendReal,
    config::AxiomConfig,
    input::InputManager,
    window::WindowManager,
    workspace::ScrollableWorkspaces,
};
use parking_lot::RwLock;
use std::os::fd::AsFd;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface,
    },
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

/// Size of the client's known-solid-red SHM buffer.
const CLIENT_W: u32 = 256;
const CLIENT_H: u32 = 192;
const RED_PIXEL: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF];

struct ClientState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    output: Option<wl_output::WlOutput>,
    surface: Option<wl_surface::WlSurface>,
    configured: bool,
    // Screencopy state
    screencopy_manager: Option<ZwlrScreencopyManagerV1>,
    frame: Option<ZwlrScreencopyFrameV1>,
    capture_buffer: Option<wl_buffer::WlBuffer>,
    capture_done: bool,
    capture_ok: bool,
}

impl ClientState {
    fn init_xdg_surface(&mut self, qh: &QueueHandle<Self>) {
        let (wm_base, compositor) = match (self.wm_base.as_ref(), self.compositor.as_ref()) {
            (Some(wm_base), Some(compositor)) => (wm_base, compositor),
            _ => return,
        };
        if self.surface.is_some() {
            return;
        }
        let surface = compositor.create_surface(qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, qh, ());
        let _toplevel = xdg_surface.get_toplevel(qh, ());
        surface.commit();
        self.surface = Some(surface);
    }

    fn try_capture(&mut self, qh: &QueueHandle<Self>) {
        let (manager, output) = match (self.screencopy_manager.as_ref(), self.output.as_ref()) {
            (Some(m), Some(o)) => (m, o),
            _ => return,
        };
        if self.frame.is_some() {
            return;
        }
        // Create a frame by calling capture_output on the manager.
        // The compositor will respond with buffer/buffer_done events.
        let frame = manager.capture_output(0, output, qh, ());
        self.frame = Some(frame);
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for ClientState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, .. } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, 1, qh, ()));
                    state.init_xdg_surface(qh);
                }
                "wl_shm" => {
                    let shm: wl_shm::WlShm = registry.bind(name, 1, qh, ());
                    let mut file = tempfile::tempfile().expect("tempfile");
                    let bytes = (CLIENT_W * CLIENT_H * 4) as usize;
                    file.set_len(bytes as u64).expect("set_len");
                    {
                        use std::io::Write;
                        let mut buf = std::io::BufWriter::new(&mut file);
                        for _ in 0..bytes / 4 {
                            buf.write_all(&RED_PIXEL).unwrap();
                        }
                    }
                    let pool = shm.create_pool(file.as_fd(), bytes as i32, qh, ());
                    let buffer = pool.create_buffer(
                        0, CLIENT_W as i32, CLIENT_H as i32,
                        (CLIENT_W * 4) as i32, wl_shm::Format::Argb8888, qh, (),
                    );
                    if let Some(surface) = state.surface.as_ref() {
                        surface.attach(Some(&buffer), 0, 0);
                        surface.commit();
                    }
                    state.shm = Some(shm);
                }
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind(name, 1, qh, ()));
                    state.init_xdg_surface(qh);
                }
                "zwlr_screencopy_manager_v1" => {
                    state.screencopy_manager = Some(registry.bind(name, 1, qh, ()));
                    state.try_capture(qh);
                }
                "wl_output" => {
                    let output = registry.bind(name, 1, qh, ());
                    state.output = Some(output);
                    state.try_capture(qh);
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(ClientState: ignore wl_compositor::WlCompositor);
delegate_noop!(ClientState: ignore wl_surface::WlSurface);
delegate_noop!(ClientState: ignore wl_shm::WlShm);
delegate_noop!(ClientState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(ClientState: ignore wl_buffer::WlBuffer);
delegate_noop!(ClientState: ignore wl_output::WlOutput);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ClientState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for ClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial, .. } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for ClientState {
    fn event(_: &mut Self, _: &xdg_toplevel::XdgToplevel, _: xdg_toplevel::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &ZwlrScreencopyManagerV1,
        _: <ZwlrScreencopyManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for ClientState {
    fn event(
        state: &mut Self,
        frame: &ZwlrScreencopyFrameV1,
        event: <ZwlrScreencopyFrameV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer { format, width, height, stride } => {
                // Create a SHM buffer matching the compositor's output
                if let Some(shm) = state.shm.as_ref() {
                    let bytes = (stride * height) as usize;
                    let mut file = tempfile::tempfile().expect("tempfile");
                    file.set_len(bytes as u64).expect("set_len");
                    let pool = shm.create_pool(file.as_fd(), bytes as i32, qh, ());
                    let buf = pool.create_buffer(0, width, height, stride, format, qh, ());
                    state.frame = Some(frame.clone());
                    state.capture_buffer = Some(buf);
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                if let (Some(frame), Some(buffer)) =
                    (state.frame.as_ref(), state.capture_buffer.as_ref())
                {
                    frame.copy(buffer);
                }
            }
            zwlr_screencopy_frame_v1::Event::Flags { .. } => {}
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                // Capture succeeded
                state.capture_ok = true;
                state.capture_done = true;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                state.capture_ok = false;
                state.capture_done = true;
            }
            _ => {}
        }
    }
}

/// Run the screencopy client in a background thread.
fn run_client(done: Arc<AtomicBool>, result_tx: mpsc::Sender<String>) {
    let res = (|| -> Result<()> {
        let conn = Connection::connect_to_env()?;
        let mut event_queue: EventQueue<ClientState> = conn.new_event_queue();
        let qh = event_queue.handle();

        let display = conn.display();
        display.get_registry(&qh, ());

        let mut state = ClientState {
            compositor: None,
            shm: None,
            wm_base: None,
            output: None,
            surface: None,
            configured: false,
            screencopy_manager: None,
            frame: None,
            capture_buffer: None,
            capture_done: false,
            capture_ok: false,
        };

        // Dispatch until the surface is configured and we've attempted capture
        for _ in 0..256 {
            event_queue.blocking_dispatch(&mut state)?;
            if state.capture_done {
                break;
            }
        }
        if !state.capture_done {
            anyhow::bail!("timeout: capture never completed");
        }
        if !state.capture_ok {
            anyhow::bail!("capture failed");
        }
        Ok(())
    })();

    let msg = match res {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("client error: {e:?}"),
    };
    let _ = result_tx.send(msg);
    done.store(true, Ordering::SeqCst);
}

#[test]
#[ignore]
#[serial_test::serial]
fn test_screencopy_capture() -> Result<()> {
    let mut config = AxiomConfig::default();
    config.backend.kind = "winit".to_string();

    let workspace_manager = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)));
    let window_manager = Arc::new(RwLock::new(WindowManager::new(&config.window)));
    let input_manager = Arc::new(RwLock::new(InputManager::new(&config.input, &config.bindings)));
    let decoration_manager = Arc::new(RwLock::new(axiom::decoration::DecorationManager::new(
        &config.window, config.features.enable_minimize,
    )));

    let mut backend = AxiomSmithayBackendReal::new(
        config, window_manager, workspace_manager, input_manager, decoration_manager,
    )?;
    backend.initialize()?;

    let socket_name = format!("wayland-axiom-{}", std::process::id());
    std::env::set_var("WAYLAND_DISPLAY", &socket_name);

    let done = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let client_done = done.clone();
    let client_handle = thread::spawn(move || run_client(client_done, tx));

    // Tick the compositor until client completes
    for _ in 0..240 {
        backend.run_one_cycle()?;
        if done.load(Ordering::SeqCst) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let client_msg = rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
    let _ = client_handle.join();
    assert_eq!(client_msg, "ok", "Wayland client failed: {client_msg}");

    Ok(())
}
```

- [ ] **Step 3: Build and verify the test compiles**

Run: `cargo build --tests 2>&1 | head -30`
Expected: Clean build (0 errors)

- [ ] **Step 4: Run the test under xvfb-run**

```bash
xvfb-run -a cargo test --test screencopy_test -- --ignored --nocapture
```
Expected: `test_screencopy_capture ... ok`

---

### Rollback Plan

If any step fails to build:
1. Read the compiler error carefully
2. If it's an API mismatch (wrong type path, wrong fn signature), fix the path/signature
3. If it's a missing import, add it
4. If it's a trait bound issue (Dispatch/GlobalDispatch generics), adjust the type parameters
5. Rebuild

If the integration test doesn't pass:
1. Add `dbg!()` or log statements to trace the protocol flow
2. Verify the global is being created (check the compositor init)
3. Verify the frame resource is being created and dispatched
4. Verify `copy_framebuffer` returns valid pixel data
5. Verify the SHM write succeeds
