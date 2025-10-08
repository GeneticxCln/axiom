# Axiom Phase 6.3 Testing Checklist

**Date**: Current Session  
**Status**: Ready for Execution  
**Priority**: HIGH

---

## Pre-Test Setup

### Prerequisites
- [ ] Verified `wayland-scanner` installed: `which wayland-scanner`
- [ ] Verified `wayland-client` available: `pkg-config --modversion wayland-client`
- [ ] No other Wayland compositors running: `ps aux | grep -E "(axiom|wayland)"`
- [ ] Project directory accessible: `cd /home/quinton/axiom`

### Build Test Client
- [ ] Navigate to tests directory: `cd tests`
- [ ] Clean previous builds: `make clean`
- [ ] Build C test client: `make`
- [ ] Verify binary exists: `ls -lh shm_test_client`
- [ ] Binary is executable and recent

---

## Test Execution

### Option A: Automated Test (Recommended)

- [ ] Navigate to project root: `cd /home/quinton/axiom`
- [ ] Run automated test: `./test_shm_rendering.sh`
- [ ] Monitor output for progress
- [ ] Wait for completion (2-5 minutes)
- [ ] Review final report (PASS/PARTIAL/FAIL)
- [ ] Check logs in `test_logs_shm/` directory

### Option B: Manual Test (Alternative)

**Terminal 1 - Compositor:**
- [ ] Navigate to project: `cd /home/quinton/axiom`
- [ ] Start compositor with logging:
  ```bash
  RUST_LOG=info,axiom=debug \
  WAYLAND_DISPLAY=wayland-axiom-test \
  cargo run --features wgpu-present --bin run_present_winit
  ```
- [ ] Wait for "Wayland server started" or similar message
- [ ] Compositor window appears on screen
- [ ] No crashes or errors in output

**Terminal 2 - Test Client:**
- [ ] Navigate to tests: `cd /home/quinton/axiom/tests`
- [ ] Run test client:
  ```bash
  WAYLAND_DISPLAY=wayland-axiom-test ./shm_test_client
  ```
- [ ] Monitor client output for success messages
- [ ] Client connects successfully
- [ ] Window creation messages appear

---

## Success Verification

### Client Output Checks
- [ ] ✅ "Connected to Wayland display"
- [ ] ✅ "Bound wl_compositor"
- [ ] ✅ "Bound wl_shm"
- [ ] ✅ "Bound xdg_wm_base"
- [ ] ✅ "Created wl_surface"
- [ ] ✅ "Created xdg_surface"
- [ ] ✅ "Created xdg_toplevel"
- [ ] ✅ "Created SHM buffer: 800x600"
- [ ] ✅ "Drew test pattern: 800x600 pixels"
- [ ] ✅ "XDG surface configured"
- [ ] ✅ "Attached buffer and committed surface"
- [ ] ✅ "Window is now visible and should display test pattern!"
- [ ] ✅ "Entering main loop..."

**Score**: ____ / 12 checks passed

### Visual Verification
- [ ] Window appears on screen
- [ ] Window size is approximately 800x600
- [ ] Window title is "Axiom SHM Test"
- [ ] Content is visible (not black/blank)
- [ ] Test pattern displays:
  - [ ] Checkerboard pattern visible
  - [ ] Red squares present
  - [ ] Blue squares present
  - [ ] Color gradients smooth
  - [ ] 32x32 pixel checker size (approximately)
  - [ ] No artifacts or corruption
  - [ ] No flickering or tearing

**Score**: ____ / 13 visual checks passed

### Compositor Log Checks
- [ ] "New client connected" message
- [ ] "Processing buffer" message
- [ ] "SHM buffer: 800x600" message
- [ ] "Queued texture update" or similar
- [ ] "Processing pending texture updates" or similar
- [ ] "Uploading texture" or similar
- [ ] "Rendering frame" message
- [ ] No panics or crashes
- [ ] No alignment errors
- [ ] No "wrong client" errors

**Score**: ____ / 10 log checks passed

---

## Stability Verification

### Runtime Stability
- [ ] Compositor runs for at least 30 seconds without crashing
- [ ] Client runs for at least 30 seconds without crashing
- [ ] Window remains visible throughout test
- [ ] No memory leak warnings
- [ ] No GPU errors in logs
- [ ] CPU usage reasonable (not 100%)
- [ ] No zombie processes after cleanup

### Cleanup Verification
- [ ] Stop client with Ctrl+C - exits cleanly
- [ ] Stop compositor with Ctrl+C - exits cleanly
- [ ] No error messages during shutdown
- [ ] Socket file removed: `ls /tmp/wayland-axiom-test`
- [ ] No background processes remain

---

## Test Results

### Overall Assessment

**Total Checks**: 35+  
**Passed**: ____ / 35+  
**Failed**: ____  
**Percentage**: ____%

### Final Status

- [ ] **FULL PASS** - All checks passed, visual confirmed (35/35)
- [ ] **PASS WITH WARNINGS** - Most checks passed (30-34/35)
- [ ] **PARTIAL PASS** - Core functionality works (25-29/35)
- [ ] **FAIL** - Critical issues present (<25/35)

### Notes
```
(Record any observations, issues, or anomalies here)





```

---

## If Tests Pass

Phase 6.3 is validated! Next actions:

- [ ] Update `PHASE_6_3_PROGRESS.md` with test results
- [ ] Mark Phase 6.3 as 95% complete
- [ ] Create success report document
- [ ] Begin multi-window testing:
  - [ ] Run 2 test clients simultaneously
  - [ ] Run 3+ test clients simultaneously
  - [ ] Verify Z-ordering
  - [ ] Test overlapping windows
- [ ] Plan effects integration (blur, shadows, rounded corners)
- [ ] Schedule real application testing
- [ ] Update production roadmap

---

## If Tests Fail

Debug workflow:

- [ ] Save all logs:
  ```bash
  cp test_logs_shm/compositor.log ~/axiom-debug-compositor.log
  cp test_logs_shm/client.log ~/axiom-debug-client.log
  ```
- [ ] Review error messages in logs
- [ ] Check `tests/README_SHM_TESTING.md` troubleshooting section
- [ ] Try Python client alternative:
  ```bash
  pip install pywayland
  WAYLAND_DISPLAY=wayland-axiom-test python3 tests/shm_test_client.py
  ```
- [ ] Enable trace logging:
  ```bash
  RUST_LOG=trace,axiom=trace cargo run --features wgpu-present --bin run_present_winit
  ```
- [ ] Review recent code changes in:
  - [ ] `src/renderer/mod.rs`
  - [ ] `src/smithay/server.rs`
  - [ ] `src/bin/run_present_winit.rs`
- [ ] Check for known issues in `BUG_REPORT_*.md` files
- [ ] Document failure in test log

---

## Common Issues & Quick Fixes

### Issue: Client can't connect
**Check**: Is compositor running?  
**Fix**: Start compositor first, wait 2-3 seconds

### Issue: "Missing Wayland interfaces"
**Check**: Compositor fully initialized?  
**Fix**: Wait longer after compositor start

### Issue: Window appears but blank
**Check**: Texture upload working?  
**Fix**: Enable debug logging, check for alignment errors

### Issue: Build fails
**Check**: Dependencies installed?  
**Fix**: `sudo apt-get install wayland-protocols libwayland-dev`

### Issue: Compositor crashes on startup
**Check**: Graphics drivers OK?  
**Fix**: Test with `glxinfo` or similar

---

## Documentation Updates After Testing

- [ ] Update `PHASE_6_3_PROGRESS.md` with results
- [ ] Update `PHASE_6_3_TESTING_READY.md` if needed
- [ ] Create test results summary document
- [ ] Update `NEXT_STEPS_SUMMARY.md` with new priorities
- [ ] Update main `README.md` if milestone reached
- [ ] Commit all documentation changes

---

## Sign-Off

**Tester**: _________________  
**Date**: _________________  
**Result**: PASS / FAIL  
**Phase 6.3 Status**: _________________  
**Ready for Production**: YES / NO / NEEDS WORK

---

## Additional Notes

```
(Any additional observations, recommendations, or follow-up items)






```

---

**Last Updated**: Current session  
**Version**: 1.0  
**Status**: Ready for use