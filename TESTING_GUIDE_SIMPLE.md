# Simple Testing Guide for Axiom Compositor

## Current Status
- ✅ Test client built (`tests/shm_test_client`)
- 🔄 Compositor building in background (PID: check with `ps aux | grep cargo`)
- 📍 Build log: `/tmp/axiom_build.log`

## Once Build Completes

### Option 1: Automated Test Script (Recommended)
```bash
cd /home/quinton/axiom
./test_shm_rendering.sh
```

This script will:
1. Start the Axiom compositor in a window
2. Launch the test client
3. Validate rendering
4. Generate a report

### Option 2: Manual Testing (More Control)

**Terminal 1 - Start Compositor:**
```bash
cd /home/quinton/axiom
RUST_LOG=info,axiom=debug \
  WAYLAND_DISPLAY=wayland-test \
  ./target/debug/run_present_winit
```

Wait for:
- A window titled "Axiom Compositor" to appear
- Log message: "Wayland server started" or similar

**Terminal 2 - Run Test Client:**
```bash
cd /home/quinton/axiom/tests
WAYLAND_DISPLAY=wayland-test ./shm_test_client
```

### What to Look For

✅ **Success Indicators:**
- Test window appears in the Axiom compositor window
- Checkerboard pattern visible (red and blue squares)
- Window size ~800x600
- No crashes or errors

❌ **Failure Indicators:**
- Window doesn't appear
- Black/blank window
- Compositor crashes
- "Connection refused" errors

## Monitoring Build Progress

Check progress anytime:
```bash
tail -f /tmp/axiom_build.log
```

Check if build is still running:
```bash
ps aux | grep "cargo build"
```

## Expected Build Time
- **Fresh build**: 3-5 minutes
- **Incremental**: 30-60 seconds

## When Build Completes

You'll see in `/tmp/axiom_build.log`:
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in X.XXs
```

Then the binary will be at:
```
/home/quinton/axiom/target/debug/run_present_winit
```

## Next Steps After Successful Test

1. Document results in `PHASE_6_3_VALIDATION_RESULTS.md`
2. Test with multiple windows (run 2-3 clients)
3. Test window focus and Z-ordering
4. Run 24-hour stability test
5. Begin effects integration (Priority 2)

## Troubleshooting

### Build Takes Too Long
- Check: `tail -f /tmp/axiom_build.log`
- If stuck on same package > 5 min: Ctrl+C and retry

### Compositor Won't Start
- Check GPU drivers: `glxinfo | grep "OpenGL"`
- Try headless mode: `--headless` flag
- Check logs for specific error

### Client Can't Connect
- Verify compositor is running: `ps aux | grep axiom`
- Check socket exists: `ls /tmp/wayland-test`
- Try different socket name

### Window Appears But Blank
- Check texture upload in logs
- Verify SHM buffer processing
- Enable trace logging: `RUST_LOG=trace`

## Background Build Monitoring

Every 30 seconds, check progress:
```bash
watch -n 30 'tail -5 /tmp/axiom_build.log'
```

Or check completion:
```bash
while ps aux | grep -q "cargo build.*axiom"; do 
    echo "Still building... $(date)"
    sleep 10
done
echo "Build complete!"
```
