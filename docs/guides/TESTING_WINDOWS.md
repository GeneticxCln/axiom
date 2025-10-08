# Testing Axiom with Visible Windows

## Why No Window Appears

The main `axiom` binary runs in **headless mode** by default. This is by design because it's a real Wayland compositor that creates a Wayland socket for clients to connect to, similar to how Sway, Hyprland, or GNOME Shell work.

When you run `cargo run --release --bin axiom`, it:
- ✅ Starts successfully
- ✅ Creates a Wayland socket at `/run/user/1000/axiom/axiom.sock`
- ✅ Initializes all subsystems (workspaces, effects, input, XWayland)
- ❌ Does NOT create a visible window (headless GPU rendering only)

## Option 1: Use the Windowed Test Binary (Recommended for Testing)

The easiest way to test Axiom with a visible window is to use the dedicated test binary:

```bash
cargo run --release --bin run_present_winit --features "smithay,wgpu-present"
```

This will:
- Create a visible window showing the Axiom compositor output
- Run a nested Wayland compositor (clients can connect to it)
- Display the compositor's rendering in real-time
- Allow you to see effects, workspaces, and window management visually

### What You'll See
- A window titled "Axiom Compositor"
- The compositor's render output (currently empty until clients connect)
- Real-time updates as windows are created/destroyed

### Connecting Test Clients
While `run_present_winit` is running, open another terminal and run:

```bash
# Find the Axiom socket
AXIOM_SOCKET=$(ls /run/user/$(id -u)/axiom-*/*.sock 2>/dev/null | grep wayland | head -1)
AXIOM_DISPLAY=$(basename $(dirname $AXIOM_SOCKET))/$(basename $AXIOM_SOCKET)

# Launch a Wayland application
WAYLAND_DISPLAY=$AXIOM_DISPLAY weston-terminal
# or
WAYLAND_DISPLAY=$AXIOM_DISPLAY foot
# or
WAYLAND_DISPLAY=$AXIOM_DISPLAY alacritty
```

## Option 2: Connect Clients to Headless Axiom

If you run the main `axiom` binary in headless mode, you can still test it by connecting Wayland clients:

### Step 1: Start Axiom
```bash
cargo run --release --bin axiom
```

Keep this running in one terminal.

### Step 2: Connect a Client
In another terminal:

```bash
# The socket is at /run/user/$(id -u)/axiom/axiom.sock
WAYLAND_DISPLAY=axiom weston-terminal
# or any other Wayland app
```

### Step 3: Use the IPC to Query State
```bash
# Connect to the IPC socket
echo '{"method": "get_workspaces"}' | nc -U /run/user/$(id -u)/axiom/axiom.sock
```

## Option 3: Run as Your Main Compositor (Advanced)

⚠️ **Warning**: This will replace your current desktop session!

To run Axiom as your actual compositor:

1. Log out of your current session
2. At the login screen, select "Axiom" (if you've installed it)
3. Or from a TTY (Ctrl+Alt+F2):
   ```bash
   cd /home/quinton/axiom
   cargo run --release --bin axiom
   ```

## Understanding the Architecture

```
┌─────────────────────────────────────────────┐
│  Axiom Compositor (Headless by default)    │
│  - Wayland socket: /run/user/1000/axiom/   │
│  - IPC socket: /run/user/1000/axiom/...    │
│  - GPU rendering (offscreen)                │
└─────────────────────────────────────────────┘
                    │
                    │ Clients connect via WAYLAND_DISPLAY
                    │
        ┌───────────┴───────────┐
        │                       │
        ▼                       ▼
┌──────────────┐      ┌──────────────┐
│ Terminal App │      │  GUI App     │
│ (foot/kitty) │      │ (firefox)    │
└──────────────┘      └──────────────┘


Alternative: Windowed Mode (run_present_winit)
┌─────────────────────────────────────────────┐
│  Your Current Desktop (KDE/GNOME/etc)       │
│  ┌─────────────────────────────────────┐   │
│  │  Axiom Window (nested compositor)   │   │
│  │  Shows Axiom's render output        │   │
│  │  ┌──────────┐  ┌──────────┐        │   │
│  │  │ Client 1 │  │ Client 2 │        │   │
│  │  └──────────┘  └──────────┘        │   │
│  └─────────────────────────────────────┘   │
└─────────────────────────────────────────────┘
```

## Quick Test Commands

### Test with windowed mode:
```bash
cargo run --release --bin run_present_winit --features "smithay,wgpu-present"
```

### Test scrollable workspaces:
```bash
# Launch the windowed compositor, then connect multiple clients
WAYLAND_DISPLAY=<axiom-display> foot &
WAYLAND_DISPLAY=<axiom-display> foot &
# Use Super+Left/Right to scroll between workspaces
```

### Test visual effects:
The effects are enabled by default. You should see:
- ✨ Smooth animations (300ms ease-out)
- 🌊 Blur effects (radius: 10px)
- 🌟 Drop shadows (size: 20px)
- 🔄 Rounded corners (8px radius)

## Troubleshooting

### "No window appears"
- Use `run_present_winit` instead of `axiom` binary
- Or connect Wayland clients to the headless compositor

### "Socket not found"
- Check `/run/user/$(id -u)/axiom/` directory
- Make sure Axiom is still running

### "libEGL warnings"
- These are normal GPU initialization messages
- They don't affect functionality

### "No work has been submitted for this frame" (FIXED)
- **This error has been fixed!** Previously appeared when no windows were visible
- The compositor now only presents frames when there's actual content to render
- You should not see this error anymore in the latest version

### "Can't connect clients"
- Verify Axiom is running: `ps aux | grep axiom`
- Check socket exists: `ls /run/user/$(id -u)/axiom/`
- Try explicit display: `WAYLAND_DISPLAY=axiom your-app`
