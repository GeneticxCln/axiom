# Axiom SHM Testing Suite

## Overview

This directory contains a comprehensive testing suite for validating the Axiom compositor's rendering pipeline using **Shared Memory (SHM)** buffers. These tests verify end-to-end rendering from client buffer submission to GPU display.

**Purpose**: Validate Phase 6.3 (Rendering Pipeline) completion

**Status**: ✅ Ready for testing

---

## Why SHM Testing?

### The Problem

GPU-backed Wayland clients (like `alacritty`, `weston-terminal`) use DMA-BUF for direct GPU buffer sharing. However, these clients fail to initialize on some systems with:

```
libEGL warning: egl: failed to create dri2 screen
```

### The Solution

SHM (Shared Memory) buffers provide a simpler, more reliable path for testing:

- ✅ Works on any system without GPU driver complexity
- ✅ Validates the complete rendering pipeline
- ✅ Tests texture upload, bind groups, and render passes
- ✅ Provides visual confirmation of rendering

Once SHM rendering works, the compositor's GPU rendering pipeline is proven functional.

---

## Test Clients

We provide **two** test client implementations:

### 1. C Client (`shm_test_client.c`)

**Advantages**:
- Native Wayland protocol implementation
- Minimal dependencies
- Fast and lightweight
- Industry-standard approach

**Requirements**:
- `libwayland-client`
- `wayland-protocols`
- `wayland-scanner`
- C compiler (gcc/clang)

**Build**:
```bash
cd tests
make
```

**Run**:
```bash
WAYLAND_DISPLAY=wayland-axiom-test ./shm_test_client
```

### 2. Python Client (`shm_test_client.py`)

**Advantages**:
- Easier to read and modify
- Good for debugging
- Cross-platform
- No compilation needed

**Requirements**:
- Python 3.8+
- `pywayland` library

**Install Dependencies**:
```bash
pip install pywayland
```

**Run**:
```bash
WAYLAND_DISPLAY=wayland-axiom-test python3 shm_test_client.py
```

---

## What They Test

Both clients perform identical testing:

### 1. Protocol Binding
- ✅ Connect to Wayland display
- ✅ Bind `wl_compositor`
- ✅ Bind `wl_shm` (shared memory interface)
- ✅ Bind `xdg_wm_base` (window management)

### 2. Buffer Creation
- ✅ Create anonymous shared memory file
- ✅ Map memory with `mmap()`
- ✅ Create `wl_shm_pool`
- ✅ Create `wl_buffer` with ARGB8888 format

### 3. Content Rendering
- ✅ Draw test pattern (red/blue checkerboard with gradients)
- ✅ Write directly to shared memory
- ✅ Verify pixel data integrity

### 4. Window Management
- ✅ Create XDG surface
- ✅ Create XDG toplevel
- ✅ Handle configure events
- ✅ Set window title

### 5. Buffer Submission
- ✅ Attach buffer to surface
- ✅ Mark damage region
- ✅ Commit surface
- ✅ Wait for compositor acknowledgment

### 6. Event Loop
- ✅ Process Wayland events
- ✅ Handle window lifecycle
- ✅ Graceful shutdown

---

## Expected Visual Output

When successful, you should see a window displaying:

```
┌─────────────────────────────────────┐
│  █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █   │  Red squares fade left→right
│ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █  │  Blue squares fade top→bottom
│  █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █   │  32x32 pixel checkerboard
│ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █  │
│  █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █   │  Color Scheme:
│ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █  │  - Red: RGB(x*255/width, 50, 50)
│  █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █   │  - Blue: RGB(50, y*255/height, 200)
│ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █ █  │
└─────────────────────────────────────┘
```

**Window Title**: "Axiom SHM Test" (C) or "Axiom SHM Test (Python)"

**Size**: 800x600 pixels

---

## Automated Testing

### Full Test Script (`test_shm_rendering.sh`)

This script automates the entire testing workflow:

**What it does**:
1. ✅ Builds the C test client
2. ✅ Builds the Axiom compositor
3. ✅ Starts the compositor
4. ✅ Runs the test client
5. ✅ Monitors for success
6. ✅ Analyzes logs
7. ✅ Reports results

**Run**:
```bash
./test_shm_rendering.sh
```

**Success Criteria** (8 checks):
- [x] Client connects to Wayland
- [x] wl_compositor bound
- [x] wl_shm bound
- [x] xdg_wm_base bound
- [x] SHM buffer created
- [x] Test pattern drawn
- [x] XDG surface configured
- [x] Buffer attached and committed

**Output**:
- Logs saved to `test_logs_shm/`
- `compositor.log` - Compositor output
- `client.log` - Client output
- `build.log` - Build output

---

## Manual Testing

### Step 1: Start Compositor

```bash
# Terminal 1
RUST_LOG=debug \
WAYLAND_DISPLAY=wayland-axiom-test \
cargo run --features wgpu-present --bin run_present_winit
```

### Step 2: Run Test Client

```bash
# Terminal 2 (C client)
cd tests
make
WAYLAND_DISPLAY=wayland-axiom-test ./shm_test_client
```

**OR**

```bash
# Terminal 2 (Python client)
WAYLAND_DISPLAY=wayland-axiom-test python3 tests/shm_test_client.py
```

### Step 3: Verify

Look for these success messages in the client output:

```
✅ Connected to Wayland display
✅ Bound wl_compositor
✅ Bound wl_shm
✅ Bound xdg_wm_base
✅ Created SHM buffer: 800x600
✅ Drew test pattern: 800x600 pixels
✅ XDG surface configured
✅ Attached buffer and committed surface
✨ Window is now visible and should display test pattern!
```

---

## Troubleshooting

### Client fails to connect

**Error**: `Failed to connect to Wayland display`

**Solution**:
1. Check compositor is running
2. Verify `WAYLAND_DISPLAY` matches compositor socket
3. Check socket exists: `ls -la /tmp/wayland-*`

### Missing Wayland interfaces

**Error**: `Missing required Wayland interfaces`

**Solution**:
1. Compositor may not have initialized fully
2. Wait 2-3 seconds after starting compositor
3. Check compositor logs for errors

### Build failures (C client)

**Error**: `wayland-scanner not found`

**Solution**:
```bash
# Debian/Ubuntu
sudo apt-get install wayland-protocols libwayland-dev

# Fedora
sudo dnf install wayland-protocols-devel wayland-devel

# Arch
sudo pacman -S wayland-protocols wayland
```

### Build failures (Python client)

**Error**: `ModuleNotFoundError: No module named 'pywayland'`

**Solution**:
```bash
pip install pywayland

# OR with system package manager
sudo apt-get install python3-pywayland  # Debian/Ubuntu
```

### Client connects but no window appears

**Possible causes**:
1. Compositor rendering pipeline not complete
2. Buffer not attached properly
3. Surface not committed
4. Configure event not acknowledged

**Debug steps**:
1. Check client log for `✅ Attached buffer and committed`
2. Check compositor log for buffer processing
3. Enable trace logging: `RUST_LOG=trace`
4. Verify `process_pending_texture_updates()` is called

### Window appears but no content

**Possible causes**:
1. Texture upload failed
2. Bytes-per-row alignment issue
3. Format mismatch
4. Render pass not executing

**Debug steps**:
1. Check for alignment errors in compositor log
2. Verify format is ARGB8888
3. Check `update_window_texture()` succeeds
4. Verify render pass draws windows

---

## Success Indicators

### Client-Side

The client should output:

```
🚀 Starting Axiom SHM Test Client
================================

✅ Connected to Wayland display
📋 Registry: wl_compositor (id=1, version=4)
✅ Bound wl_compositor
📋 Registry: wl_shm (id=2, version=1)
✅ Bound wl_shm
📋 Registry: xdg_wm_base (id=3, version=1)
✅ Bound xdg_wm_base

📐 Creating window (800x600)
✅ Created wl_surface
✅ Created xdg_surface
✅ Created xdg_toplevel
✅ Committed initial surface

🎨 Creating SHM buffer
✅ Created SHM buffer: 800x600, stride=3200, size=1920000 bytes
✅ Drew test pattern: 800x800 pixels

⏳ Waiting for configure event...
✅ XDG surface configured (serial=1)
✅ Attached buffer and committed surface

✨ Window is now visible and should display test pattern!
   - Red/blue checkerboard with gradients
   - Press Ctrl+C to exit

🔄 Entering main loop...
```

### Compositor-Side

The compositor should log:

```
[DEBUG axiom::smithay::server] New client connected: ClientId(...)
[DEBUG axiom::smithay::server] Processing buffer for window: WindowId(...)
[DEBUG axiom::smithay::server] SHM buffer: 800x600, format: Argb8888
[DEBUG axiom::smithay::server] Queued texture update: 800x600 (1920000 bytes)
[DEBUG axiom::renderer] Processing 1 pending texture updates
[DEBUG axiom::renderer] Uploading texture: 800x600
[DEBUG axiom::renderer] Texture upload complete
[DEBUG axiom::renderer] Rendering frame with 1 windows
[DEBUG axiom::renderer] Render pass complete
```

---

## What Success Means

If the test clients work and display the test pattern, it confirms:

✅ **Phase 6.2** - Protocol implementation is correct
✅ **Phase 6.3** - Rendering pipeline is functional
✅ **Buffer Reception** - Compositor receives client buffers
✅ **Format Conversion** - SHM data converts to RGBA correctly
✅ **Texture Upload** - GPU texture creation works
✅ **Texture Alignment** - 256-byte alignment handled
✅ **Bind Groups** - Texture bindings created
✅ **Uniform Buffers** - Window transforms applied
✅ **Render Pass** - Draw commands executed
✅ **Display Output** - Pixels reach the screen

**This validates the entire rendering pipeline from client to display!**

---

## Next Steps After Success

Once SHM rendering works:

### 1. Multi-Window Testing
- Run multiple test clients simultaneously
- Verify Z-ordering works correctly
- Test overlapping windows

### 2. Real Application Testing
- Test with SHM-based applications
- Test with older terminal emulators
- Validate workspace switching

### 3. DMA-BUF Implementation (Optional)
- Implement full GPU buffer support
- Test with GPU-accelerated clients
- Enable zero-copy rendering

### 4. Performance Optimization
- Add damage tracking
- Optimize texture uploads
- Profile render times

### 5. Effects Integration
- Add blur shaders
- Implement rounded corners
- Add drop shadows

### 6. Production Polish
- Comprehensive application testing
- Stability testing
- Documentation updates
- Release preparation

---

## Technical Details

### Buffer Format

**Format**: `WL_SHM_FORMAT_ARGB8888` (0x34325241)

**Layout**: 
```
Byte 0: Blue
Byte 1: Green
Byte 2: Red
Byte 3: Alpha
```

**Little-endian 32-bit**: `0xAARRGGBB`

### Memory Layout

```
Width:  800 pixels
Height: 600 pixels
Stride: 3200 bytes (800 * 4)
Size:   1,920,000 bytes (3200 * 600)
```

### Test Pattern Algorithm

```c
for (y = 0; y < height; y++) {
    for (x = 0; x < width; x++) {
        int checker = ((x / 32) + (y / 32)) % 2;
        
        if (checker) {
            // Red gradient (left to right)
            r = (x * 255) / width;
            g = 50;
            b = 50;
        } else {
            // Blue gradient (top to bottom)
            r = 50;
            g = (y * 255) / height;
            b = 200;
        }
        
        color = 0xFF000000 | (r << 16) | (g << 8) | b;
        pixels[y * width + x] = color;
    }
}
```

### GPU Pipeline

```
Client SHM Buffer
    ↓
wl_buffer.attach()
    ↓
Compositor receives buffer
    ↓
convert_shm_to_rgba()
    ↓
queue_texture_update()
    ↓
SharedRenderState.pending_textures
    ↓
[FRAME START]
    ↓
process_pending_texture_updates()
    ↓
update_window_texture()
    ↓
queue.write_texture() → GPU
    ↓
create_bind_group()
    ↓
render_pass.draw_indexed()
    ↓
queue.submit()
    ↓
[DISPLAY ON SCREEN]
```

---

## Files in This Directory

- `shm_test_client.c` - C implementation of test client
- `shm_test_client.py` - Python implementation of test client
- `Makefile` - Build system for C client
- `README_SHM_TESTING.md` - This file
- `integration_tests.rs` - Rust integration tests
- `real_backend_connectivity.rs` - Backend connectivity tests

---

## Contributing

To add new test cases:

1. Create a new client that uses SHM buffers
2. Draw different test patterns to verify specific features
3. Add to the automated test suite
4. Document expected behavior
5. Update this README

---

## References

- [Wayland Protocol Specification](https://wayland.freedesktop.org/docs/html/)
- [XDG Shell Protocol](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/xdg-shell/xdg-shell.xml)
- [Wayland Book](https://wayland-book.com/)
- [wgpu Documentation](https://docs.rs/wgpu/)
- [Smithay Documentation](https://docs.rs/smithay/)

---

## License

Same as the main Axiom project.

---

**Last Updated**: Phase 6.3 implementation  
**Status**: ✅ Ready for production validation  
**Maintainer**: Axiom Development Team