# Manual SHM Rendering Test Guide

## Quick Start Guide for Testing Phase 6.3 Rendering Pipeline

This guide walks you through manually testing the Axiom compositor's rendering pipeline with a simple SHM (Shared Memory) client.

---

## Step 1: Check Prerequisites

```bash
# Check for required tools
which wayland-scanner
which pkg-config
pkg-config --modversion wayland-client

# Should show version 1.x or higher
```

If missing, install:
```bash
# Debian/Ubuntu
sudo apt-get install wayland-protocols libwayland-dev

# Fedora
sudo dnf install wayland-protocols-devel wayland-devel
```

---

## Step 2: Build the Test Client

```bash
cd tests
make clean
make
```

**Expected Output:**
```
✅ Build complete: shm_test_client
```

Verify binary exists:
```bash
ls -lh shm_test_client
```

---

## Step 3: Open Two Terminals

You'll need two terminal windows/panes:
- **Terminal 1**: Run the compositor
- **Terminal 2**: Run the test client

---

## Step 4: Start the Compositor (Terminal 1)

```bash
cd /home/quinton/axiom

# Start with debug logging
RUST_LOG=info,axiom=debug \
WAYLAND_DISPLAY=wayland-axiom-test \
cargo run --features wgpu-present --bin run_present_winit
```

**Wait for these messages:**
```
[INFO] Initializing compositor...
[INFO] Starting event loop...
```

**Leave this running!** Do not close Terminal 1.

---

## Step 5: Run the Test Client (Terminal 2)

```bash
cd /home/quinton/axiom/tests

# Run the test client
WAYLAND_DISPLAY=wayland-axiom-test ./shm_test_client
```

**Expected Output:**
```
🚀 Starting Axiom SHM Test Client
================================

✅ Connected to Wayland display
✅ Bound wl_compositor
✅ Bound wl_shm
✅ Bound xdg_wm_base

📐 Creating window (800x600)
✅ Created wl_surface
✅ Created xdg_surface
✅ Created xdg_toplevel

🎨 Creating SHM buffer
✅ Created SHM buffer: 800x600, stride=3200, size=1920000 bytes
✅ Drew test pattern: 800x600 pixels

⏳ Waiting for configure event...
✅ XDG surface configured (serial=1)
✅ Attached buffer and committed surface

✨ Window is now visible and should display test pattern!
   - Red/blue checkerboard with gradients
   - Press Ctrl+C to exit

🔄 Entering main loop...
```

---

## Step 6: Verify Visual Output

You should see a window on your screen with:

- **Size**: 800x600 pixels
- **Title**: "Axiom SHM Test"
- **Content**: Red and blue checkerboard pattern with color gradients
  - Red squares fade from dark to bright (left to right)
  - Blue squares fade from dark to bright (top to bottom)
  - 32x32 pixel checker size

**This means the rendering pipeline is working!**

---

## Step 7: Check Compositor Logs (Terminal 1)

Look for these messages in the compositor output:

```
[DEBUG axiom::smithay::server] New client connected
[DEBUG axiom::smithay::server] Processing buffer for window
[DEBUG axiom::smithay::server] SHM buffer: 800x600, format: Argb8888
[DEBUG axiom::renderer] Processing pending texture updates
[DEBUG axiom::renderer] Uploading texture: 800x600
[DEBUG axiom::renderer] Rendering frame with 1 windows
```

---

## Step 8: Stop Everything

1. In **Terminal 2** (client): Press `Ctrl+C`
2. In **Terminal 1** (compositor): Press `Ctrl+C`

---

## Success Criteria

✅ **PASS** if:
- Client connects without errors
- Client shows "Window is now visible" message
- Window appears on screen
- Test pattern is visible and correct
- No crashes or panics in compositor logs

❌ **FAIL** if:
- Client fails to connect
- No window appears
- Window appears but is black/blank
- Compositor crashes
- Client crashes

---

## Troubleshooting

### Client says "Failed to connect to Wayland display"

**Cause**: Compositor not running or wrong socket name

**Fix**: 
1. Check compositor is running in Terminal 1
2. Verify WAYLAND_DISPLAY matches in both terminals
3. Check socket exists: `ls -la /tmp/wayland-axiom-test`

### Window appears but is black/blank

**Cause**: Rendering pipeline issue

**Check**:
1. Compositor logs show "Processing buffer"?
2. Compositor logs show "Uploading texture"?
3. Any errors about alignment or texture creation?

**Enable trace logging**:
```bash
RUST_LOG=trace,axiom=trace \
WAYLAND_DISPLAY=wayland-axiom-test \
cargo run --features wgpu-present --bin run_present_winit
```

### Compositor crashes on startup

**Check**:
1. wgpu-present feature enabled?
2. GPU/graphics drivers working?
3. Can you run other graphics apps?

### Client crashes

**Check**:
1. Build completed successfully?
2. All shared libraries available: `ldd ./shm_test_client`
3. Run with strace: `strace ./shm_test_client 2>&1 | less`

---

## Alternative: Python Client

If the C client has issues, try the Python version:

```bash
# Install pywayland (if needed)
pip install pywayland

# Run Python client
cd /home/quinton/axiom/tests
WAYLAND_DISPLAY=wayland-axiom-test python3 shm_test_client.py
```

Same expected output and behavior as C client.

---

## What Success Means

If the test pattern displays correctly, it confirms:

✅ Phase 6.2 - Protocol implementation works
✅ Phase 6.3 - Rendering pipeline works
✅ Buffer reception from clients works
✅ SHM buffer format conversion works
✅ GPU texture upload works
✅ Texture alignment handling works
✅ Bind groups and uniforms work
✅ Render pass executes correctly
✅ Pixels reach the screen

**This validates the entire compositor rendering stack!**

---

## Next Steps After Success

1. Test multiple windows (run client multiple times)
2. Test with real applications
3. Implement effects (blur, shadows, etc.)
4. Performance optimization
5. Production release preparation

---

## Getting Help

If tests fail:

1. Save logs:
   ```bash
   # Compositor log
   RUST_LOG=debug cargo run ... > compositor.log 2>&1
   
   # Client log
   ./shm_test_client > client.log 2>&1
   ```

2. Check logs for errors
3. Review PHASE_6_3_PROGRESS.md for known issues
4. Consult tests/README_SHM_TESTING.md for detailed troubleshooting

---

**Status**: Ready for testing
**Estimated Time**: 5-10 minutes
**Difficulty**: Easy

Good luck! 🚀