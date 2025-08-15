# Axiom Development Setup Guide

## Prerequisites for Real Wayland Compositor Development

### System Dependencies (CachyOS/Arch Linux)

```bash
# Install Wayland development libraries
sudo pacman -S wayland wayland-protocols libxkbcommon mesa

# Install input and graphics libraries  
sudo pacman -S libinput libudev0-shim libdrm

# Install development tools
sudo pacman -S pkg-config cmake ninja

# Install optional Wayland utilities for testing
sudo pacman -S weston wayland-utils
```

### Rust Dependencies

Add to your `Cargo.toml` for real Smithay integration:

```toml
[dependencies]
# Update Smithay with all required features
smithay = { version = "0.3.0", features = [
    "backend_winit",
    "backend_drm", 
    "backend_libinput",
    "renderer_gl",
    "wayland_frontend"
] }

# Add missing dependencies for real compositor
libloading = "0.8"
gbm = "0.12"
drm = "0.9"
input = "0.8"
```

## Development Environment Setup

### 1. Wayland Testing Setup

Create a nested Wayland session for testing:

```bash
# Terminal 1: Start a Wayland compositor (weston) for testing
weston --backend=wayland-backend.so --width=1920 --height=1080

# Terminal 2: Set WAYLAND_DISPLAY for your compositor
export WAYLAND_DISPLAY=wayland-1
cd /home/sasha/axiom
cargo run -- --debug --windowed
```

### 2. Create First Real Smithay Backend

Replace `smithay_backend_simple.rs` with minimal real implementation:

```rust
// Start with this minimal real backend
use smithay::{
    backend::winit::{self, WinitError, WinitGraphicsBackend},
    desktop::{Space, Window},
    output::{Output, PhysicalProperties, Subpixel, Mode as OutputMode},
    reexports::{
        calloop::EventLoop,
        wayland_server::{Display, DisplayHandle, Client},
        winit::event_loop::EventLoop as WinitEventLoop,
    },
    wayland::{
        compositor::{CompositorState, CompositorHandler},
        shell::xdg::{XdgShellState, XdgShellHandler, ToplevelSurface},
        shm::{ShmState, ShmHandler},
        seat::{SeatState, SeatHandler, Seat, CursorImageStatus},
    },
    delegate_compositor, delegate_xdg_shell, delegate_shm, delegate_seat,
};

pub struct AxiomState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub space: Space<Window>,
    pub seat: Seat<Self>,
}

// Implement required handlers
impl CompositorHandler for AxiomState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn new_surface(&mut self, surface: &wayland_server::protocol::wl_surface::WlSurface) {
        // Hook into your existing window management system
    }
}

impl XdgShellHandler for AxiomState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new(surface);
        self.space.map_element(window, (0, 0), false);
        
        // Connect to your workspace manager here
        // self.workspace_manager.add_window(window_id);
    }
}
```

### 3. Testing Applications

Start with simple applications for testing:

```bash
# Test with simple terminal
weston-terminal

# Test with basic image viewer  
weston-image /path/to/image.png

# Test with calculator
gnome-calculator
```

### 4. Debugging Tools

Install helpful debugging tools:

```bash
# Install wayland debugging utilities
sudo pacman -S wayland-utils

# Monitor Wayland protocol messages
wayland-scanner client-header /usr/share/wayland/wayland.xml wayland-client-protocol.h

# Use weston-info to see available protocols
weston-info

# Monitor performance
htop
```

### 5. Development Workflow

Recommended development cycle:

1. **Start Simple**: Get basic window creation working
2. **Test Incrementally**: Test each feature with real applications
3. **Debug Protocol Issues**: Use `WAYLAND_DEBUG=1` for protocol debugging
4. **Performance Monitor**: Watch memory and CPU usage constantly
5. **Version Control**: Commit working states frequently

### 6. Example Testing Script

Create `test_compositor.sh`:

```bash
#!/bin/bash
# Test script for Axiom development

echo "🚀 Testing Axiom Compositor Development"

# Build in debug mode
echo "📦 Building..."
cargo build --debug || exit 1

# Kill any existing compositor
pkill axiom

# Start compositor in background
echo "🏗️ Starting Axiom..."
./target/debug/axiom --debug --windowed &
AXIOM_PID=$!

# Wait a bit for startup
sleep 2

# Test with simple application
echo "🧪 Testing with weston-terminal..."
weston-terminal &
TERM_PID=$!

# Wait for user to test
echo "✨ Axiom is running! Press Enter to stop..."
read

# Clean shutdown
echo "🛑 Shutting down..."
kill $TERM_PID 2>/dev/null
kill $AXIOM_PID 2>/dev/null

echo "✅ Test complete!"
```

## Quick Start Commands

```bash
# Setup environment
cd /home/sasha/axiom

# Install dependencies (if needed)
sudo pacman -S wayland-protocols libxkbcommon libinput

# Start development
cargo build --debug
./target/debug/axiom --debug --windowed --demo

# In another terminal, test IPC
python3 test_ipc.py
```

## Performance Monitoring

Monitor your compositor during development:

```bash
# Watch memory usage
watch -n 1 'ps aux | grep axiom'

# Monitor frame timing
perf top -p $(pgrep axiom)

# Check GPU usage (if available)
nvidia-smi  # or similar for your GPU
```

## Common Issues and Solutions

### Issue: "No Wayland display found"
**Solution**: Make sure `WAYLAND_DISPLAY` is set correctly
```bash
export WAYLAND_DISPLAY=wayland-0  # or wayland-1
```

### Issue: Permission denied for DRM
**Solution**: Add user to video group
```bash
sudo usermod -a -G video $USER
# Log out and back in
```

### Issue: Applications don't appear
**Solution**: Check protocol implementation
```bash
# Enable Wayland debugging
export WAYLAND_DEBUG=1
./target/debug/axiom --debug
```

## Next Steps

1. **Week 1**: Replace `smithay_backend_simple.rs` with minimal real implementation
2. **Week 2**: Get `weston-terminal` running successfully  
3. **Week 3**: Add proper input handling
4. **Week 4**: Connect workspace manager to real windows

Your existing architecture is excellent - this is just about connecting it to real Wayland protocols! 🚀
