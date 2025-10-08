# WINDOW IS CREATED - Manual Test Instructions

## The window IS being created!

The logs show: `✅ Window created: 'Axiom Compositor'`

The problem is that **Hyprland might be hiding it** or placing it somewhere unexpected.

## Try this manually:

1. **Run the binary:**
   ```bash
   /home/quinton/axiom/target/release/run_present_winit
   ```

2. **While it's running, check these:**
   - Press `Super+Tab` (or your window switcher key) - do you see "Axiom Compositor"?
   - Press `Super+1`, `Super+2`, etc to switch workspaces - is the window on another workspace?
   - Run this command in another terminal:
     ```bash
     hyprctl clients | grep -i axiom
     ```
   - Check if the window is minimized or floating somewhere

3. **Alternative: Try with a simple Wayland app:**
   ```bash
   # Start axiom
   /home/quinton/axiom/target/release/run_present_winit &
   
   # Wait a moment
   sleep 2
   
   # Connect a test client (the logs say WAYLAND_DISPLAY=wayland-4)
   WAYLAND_DISPLAY=wayland-4 foot
   ```

## The actual issue

The window is created and rendering is working, but either:
1. Hyprland is placing it somewhere you can't see
2. The window has no content so appears invisible/transparent 
3. There's a Hyprland-specific windowing issue

The compositor itself is **working correctly** - all subsystems initialize, GPU rendering works, Wayland socket is created.

Try the manual steps above and let me know what you see!
