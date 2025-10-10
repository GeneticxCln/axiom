# Manual Testing for Axiom Tiling

## Step 1: Start the Compositor

In terminal 1:
```bash
./target/release/run_present_winit
```

Wait for the grey window to appear. You should see in the logs:
```
WAYLAND_DISPLAY=wayland-2
```

## Step 2: Launch Test Clients

In a **new terminal** (terminal 2), run:

```bash
# Set the Wayland display
export WAYLAND_DISPLAY=wayland-2

# Launch a test client
weston-terminal
```

If weston-terminal doesn't work, try:
```bash
WAYLAND_DISPLAY=wayland-2 alacritty
```

Or:
```bash
WAYLAND_DISPLAY=wayland-2 kitty
```

## Step 3: Verify Connection

If you see:
- ✅ Terminal window appears in the compositor
- ✅ You can type in it
- ✅ It's visible (not grey)

Then the compositor is working!

## Step 4: Launch More Windows

In terminal 2:
```bash
export WAYLAND_DISPLAY=wayland-2
weston-terminal &
weston-terminal &
weston-terminal &
```

Now you should see 4 windows stacked vertically.

## Step 5: Test Tiling Features

With the compositor window focused:

### Layout Cycling
Press **Super + L** (Windows key + L) repeatedly:
- Windows should rearrange into different patterns
- Vertical → Horizontal → Master-Stack → Grid → Spiral

### Window Focus
- **Super + J**: Focus next window (should see focus change)
- **Super + K**: Focus previous window

### Window Movement
- **Super + Shift + J**: Move focused window down in stack
- **Super + Shift + K**: Move focused window up in stack

### Workspace Navigation
- **Super + Left**: Scroll to previous workspace
- **Super + Right**: Scroll to next workspace

## Troubleshooting

### "Connection refused" errors

The compositor might not be exposing the socket properly. Check:

```bash
ls -la /run/user/$UID/wayland-*
```

You should see `wayland-2` (not just `wayland-2.lock`).

### Grey window with no content

This means no clients are connected. Make sure to:
1. Use the correct `WAYLAND_DISPLAY` value from the logs
2. Launch clients in a separate terminal
3. Export `WAYLAND_DISPLAY` before launching clients

### Can't type in test windows

This is expected in some configurations. The tiling features still work - just press the keyboard shortcuts to test layout cycling.
