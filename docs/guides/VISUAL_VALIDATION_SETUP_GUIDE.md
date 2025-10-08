# Visual Validation Setup Guide for Axiom

**System:** CachyOS Linux (Arch-based)  
**Current Session:** Wayland (nested)  
**Available Tools:** Xwayland, weston  
**Goal:** Set up environment to run and visually test Axiom compositor

---

## Current Situation

You're running in a nested Wayland session, which prevents Axiom from directly accessing display hardware. To test Axiom visually, you need one of these environments:

1. **Nested X Server (Xephyr)** - Easiest, works in current session
2. **TTY with DRM/KMS** - Most authentic, requires switching to console
3. **Nested Wayland (weston --backend=wayland-backend.so)** - Alternative nested option

---

## Option 1: Xephyr (Recommended - Easiest)

### Install Xephyr

```bash
sudo pacman -S xorg-server-xephyr
```

### Start Xephyr

```bash
# Start a 1920x1080 nested X server on display :2
Xephyr :2 -screen 1920x1080 -ac &
```

### Run Axiom in Xephyr

```bash
# Set display to Xephyr
export DISPLAY=:2

# Run Axiom compositor
cd /home/quinton/axiom
cargo run --release --bin run_present_winit
```

### Expected Result

- A new window appears showing Xephyr
- Axiom starts rendering inside that window
- You can see visual output!

### Test with Client

In another terminal:

```bash
# Also set DISPLAY for client
export DISPLAY=:2

# Run test client
cd /home/quinton/axiom
./tests/shm_test_client  # If compiled

# OR use weston-terminal as a test
weston-terminal
```

### Advantages
✅ Works in current session  
✅ Easy to capture screenshots  
✅ Safe to test  
✅ No need to leave desktop environment

### Disadvantages
⚠️ Nested overhead (not true hardware performance)  
⚠️ X11 protocol, not native Wayland

---

## Option 2: Nested Weston (Alternative)

If you want to test in a Wayland-native nested environment:

### Start Nested Weston

```bash
# Create a nested weston session
weston --width=1920 --height=1080 &

# Wait for it to start, then get socket name
export WAYLAND_DISPLAY=wayland-2  # Usually wayland-2 or wayland-3
```

### Run Axiom in Nested Weston

```bash
cd /home/quinton/axiom
cargo run --release --bin run_present_winit
```

### Advantages
✅ Native Wayland environment  
✅ Works in current session  
✅ More authentic than X11

### Disadvantages
⚠️ Compositor inside compositor (complex)  
⚠️ May have socket conflicts  
⚠️ Harder to debug

---

## Option 3: TTY with KMS/DRM (Most Authentic)

This gives you direct hardware access for true performance testing.

### Switch to TTY

```bash
# Save your work first!

# Switch to TTY2 (Ctrl+Alt+F2)
# Login as your user
```

### Set Environment

```bash
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export WAYLAND_DISPLAY=wayland-1
```

### Run Axiom

```bash
cd /home/quinton/axiom
cargo run --release --bin run_present_winit
```

### Return to Desktop

```bash
# Ctrl+C to stop Axiom
# Ctrl+Alt+F1 (or F7) to return to graphical session
```

### Advantages
✅ Direct hardware access  
✅ True performance numbers  
✅ Real-world scenario  
✅ No nesting overhead

### Disadvantages
⚠️ Leaves desktop environment  
⚠️ Can't easily capture screenshots  
⚠️ Requires physical console access  
⚠️ More risk if something goes wrong

---

## Recommended Approach

### Phase 1: Quick Validation (Use Xephyr)

**Goal:** Verify rendering works at all

```bash
# Install Xephyr
sudo pacman -S xorg-server-xephyr

# Start Xephyr
Xephyr :2 -screen 1920x1080 -ac &

# Run Axiom
export DISPLAY=:2
cd /home/quinton/axiom
cargo run --release --bin run_present_winit
```

**What to verify:**
- [ ] Axiom starts without errors
- [ ] Window appears in Xephyr
- [ ] No crashes or panics

### Phase 2: Visual Testing (Still in Xephyr)

**Run test script:**

```bash
# Terminal 1: Axiom running in Xephyr (from Phase 1)

# Terminal 2: Run test client
export DISPLAY=:2
cd /home/quinton/axiom
./test_shm_rendering.sh
```

**What to verify:**
- [ ] Test client connects to Axiom
- [ ] Window appears with test pattern
- [ ] Colors render correctly
- [ ] No flickering or corruption
- [ ] Logs show damage optimization active

### Phase 3: Real Application Testing (In Xephyr)

```bash
export DISPLAY=:2

# Test simple terminal
weston-terminal

# Test other apps if available
xterm
xclock
```

**What to verify:**
- [ ] Applications launch successfully
- [ ] Content renders correctly
- [ ] Input works (keyboard, mouse)
- [ ] Windows can be moved/resized
- [ ] Multiple windows work together

### Phase 4: Performance Testing (Move to TTY)

Once everything works in Xephyr, switch to TTY for real performance numbers:

```bash
# Switch to TTY2
# Run benchmarks in real hardware environment
# Measure actual FPS, CPU, GPU usage
```

---

## Quick Start Commands

### Fastest Path to Visual Validation

```bash
# 1. Install Xephyr (one-time setup)
sudo pacman -S xorg-server-xephyr

# 2. Start Xephyr in background
Xephyr :2 -screen 1920x1080 -ac &

# 3. Run Axiom (in new terminal)
export DISPLAY=:2
cd /home/quinton/axiom
cargo run --release --bin run_present_winit

# 4. Run test client (in another terminal)
export DISPLAY=:2
cd /home/quinton/axiom
# Compile test client if needed
gcc -o tests/shm_test_client tests/shm_test_client.c -lwayland-client
# Run it
./tests/shm_test_client
```

---

## Troubleshooting

### Xephyr won't start

**Error:** "Cannot open display :2"

**Solution:**
```bash
# Check if display :2 is already in use
ps aux | grep Xephyr

# Try different display number
Xephyr :3 -screen 1920x1080 -ac &
export DISPLAY=:3
```

### Axiom crashes with "no suitable adapter"

**Error:** "Failed to find suitable GPU adapter"

**Solution:**
```bash
# Check GPU info
ls -la /dev/dri/

# Try forcing a specific backend
WGPU_BACKEND=vulkan cargo run --release --bin run_present_winit
# OR
WGPU_BACKEND=gl cargo run --release --bin run_present_winit
```

### Test client can't connect

**Error:** "Failed to connect to Wayland display"

**Solution:**
```bash
# Axiom creates wayland-1 socket, but client needs to know about it
export XDG_RUNTIME_DIR=/run/user/$(id -u)
export WAYLAND_DISPLAY=wayland-1

# Check if socket exists
ls -la $XDG_RUNTIME_DIR/wayland-*

# Try the test
./tests/shm_test_client
```

### "Permission denied" on /dev/dri/

**Solution:**
```bash
# Add yourself to video group (one-time)
sudo usermod -a -G video $USER

# Log out and back in for changes to take effect
# OR for immediate effect in current shell:
newgrp video
```

---

## Expected Visual Output

### Successful Test Pattern

When you run `./tests/shm_test_client`, you should see:

```
Window with:
  ├─ Size: 256×256 pixels
  ├─ Color: Gradient from red → green → blue
  ├─ Border: Clean edges
  └─ No corruption or flickering
```

### Axiom Logs (Expected)

```
🎨 Creating real GPU renderer with surface (1920x1080)
✅ Headless GPU renderer initialized
💥 Frame has 1 damage regions (area: 65536/2073600 pixels, 3.2% of screen)
🪟 Rendering 1 windows in Z-order: [1] (bottom to top)
📊 Render stats: 1 windows rendered (0 occluded), 1 total draw calls 
   (1 damage-optimized, 0 full-window)
✅ Rendered 1 windows to surface
```

**Key indicators:**
- ✅ "Frame has X damage regions" - Damage tracking working
- ✅ "damage-optimized" count > 0 - Scissor optimization active
- ✅ Small damage % - Efficient rendering

---

## Screenshot Capture

### In Xephyr

```bash
# Install screenshot tool if needed
sudo pacman -S scrot

# Capture Xephyr window
export DISPLAY=:2
scrot axiom_test_%Y%m%d_%H%M%S.png

# OR use ImageMagick
import -window root axiom_test.png
```

### Document Results

Create `VISUAL_VALIDATION_RESULTS.md`:

```markdown
# Visual Validation Results

## Environment
- Display: Xephyr :2 (1920×1080)
- GPU: [Your GPU from `lspci | grep VGA`]
- Date: 2025-10-05

## Test 1: SHM Test Client
- Status: ✅ PASS / ❌ FAIL
- Screenshot: ![Test Pattern](screenshots/shm_test.png)
- Notes: ...

## Logs
[Attach relevant log snippets]
```

---

## Next Steps After Visual Validation

Once you confirm visual rendering works:

1. **Test Real Applications**
   - Terminals: foot, alacritty, weston-terminal
   - Editors: gedit (if available)
   - Browsers: firefox (may need X11 compat)

2. **Performance Benchmarking**
   - Measure FPS with multiple windows
   - Check CPU/GPU usage with `htop` and `nvidia-smi`/`radeontop`
   - Verify damage tracking reduces load

3. **Move to TTY for Real Performance**
   - Test on actual hardware
   - Get true performance numbers
   - Validate battery life improvements

---

## Safety Notes

⚠️ **Important:**

1. **Save work before TTY testing** - Switching to TTY leaves your desktop
2. **Have a backup terminal** - In case Axiom crashes
3. **Know recovery commands:**
   - `Ctrl+Alt+F1` to return to desktop
   - `Ctrl+C` to stop Axiom
   - `killall axiom` if it hangs

4. **Test incrementally:**
   - Xephyr first (safest)
   - Nested weston second
   - TTY last (most risky but most authentic)

---

## Summary: Your Path Forward

```
Step 1: Install Xephyr
  └─> sudo pacman -S xorg-server-xephyr

Step 2: Start Xephyr  
  └─> Xephyr :2 -screen 1920x1080 -ac &

Step 3: Run Axiom
  └─> export DISPLAY=:2
  └─> cd /home/quinton/axiom
  └─> cargo run --release --bin run_present_winit

Step 4: Test with Client
  └─> export DISPLAY=:2
  └─> ./test_shm_rendering.sh

Step 5: Document Results
  └─> Create VISUAL_VALIDATION_RESULTS.md
  └─> Take screenshots
  └─> Save logs

Step 6: Report Success! 🎉
```

---

**Ready to start?** The easiest path is:

```bash
sudo pacman -S xorg-server-xephyr && Xephyr :2 -screen 1920x1080 -ac &
```

Then run Axiom and see it work! 🚀

---

**Document Version:** 1.0  
**Created:** October 5, 2025  
**For:** Axiom Compositor Visual Validation  
**System:** CachyOS Linux
