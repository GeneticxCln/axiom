# Quick Test Reference

## 🚀 Start Axiom with Visible Window

```bash
cargo run --release --bin run_present_winit --features "smithay,wgpu-present"
```

**What you'll see:**
- A window titled "Axiom Compositor" 
- Black screen (empty until clients connect)
- No errors in the console ✅

## 🪟 Connect a Test Client

**While Axiom is running, open another terminal:**

```bash
# Check what display Axiom created (look in logs)
# Usually it's wayland-2

# Connect a terminal
WAYLAND_DISPLAY=wayland-2 foot

# Or try other apps:
WAYLAND_DISPLAY=wayland-2 alacritty
WAYLAND_DISPLAY=wayland-2 weston-terminal
WAYLAND_DISPLAY=wayland-2 kitty
```

## ⌨️ Default Keybindings

Once clients are connected:

- `Super+Left` - Scroll workspace left
- `Super+Right` - Scroll workspace right
- `Super+Shift+Left` - Move window left
- `Super+Shift+Right` - Move window right
- `Super+f` - Toggle fullscreen
- `Super+q` - Close window
- `Super+Shift+q` - Quit compositor

## 🎨 Visual Effects (Enabled by Default)

You should see:
- ✨ Smooth animations (300ms)
- 🌊 Blur effects
- 🌟 Drop shadows
- 🔄 Rounded corners (8px)

## 🐛 Troubleshooting

### No window appears
→ Use `run_present_winit`, not plain `axiom`

### Can't connect clients
→ Check logs for `WAYLAND_DISPLAY=wayland-X`
→ Use that exact display name

### Error: "No work has been submitted"
→ **This is fixed!** Update to latest code

## 📚 More Info

- `TESTING_WINDOWS.md` - Complete testing guide
- `FIXES_APPLIED.md` - What was fixed
- `WGPU_ERROR_FIX.md` - Technical details
- `~/.config/axiom/axiom.toml` - Configuration file
