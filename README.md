# Axiom 🚀

**The next-generation Wayland compositor combining niri's scrollable workspaces with Hyprland's visual effects, enhanced by AI optimization.**

<div align="center">

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen)](#)
[![Rust Version](https://img.shields.io/badge/rust-1.70%2B-orange)](#)
[![License](https://img.shields.io/badge/license-GPLv3-blue)](#)
[![AI Optimized](https://img.shields.io/badge/AI-optimized-purple)](#)
[![Phase](https://img.shields.io/badge/phase-3%20in%20progress-orange)](#)

**Where productivity meets beauty.**

</div>

## ✨ Vision

Axiom bridges the gap between productivity-focused tiling window managers and visually stunning compositors. Why choose between infinite scrollable workspaces OR beautiful animations when you can have both?

## 🎯 Features

### Scrollable Workspaces (niri-inspired)
- **Infinite horizontal scrolling** - No artificial workspace limits
- **Smart window placement** - Automatic, intelligent tiling
- **Smooth navigation** - Keyboard and gesture-based scrolling
- **Dynamic layouts** - Adapts to your workflow

### Visual Effects (Hyprland-inspired)  
- **Smooth animations** - Buttery 60fps window operations
- **Blur effects** - Configurable background and border blur
- **Rounded corners** - Anti-aliased, customizable radius
- **Drop shadows** - Realistic lighting effects
- **Workspace transitions** - Animated scrolling between spaces

### Hybrid Innovations (Axiom-exclusive)
- **Animated workspace scrolling** - Smooth visual transitions while scrolling
- **Context-aware effects** - Smart performance scaling during intensive operations
- **Unified configuration** - Single config for both tiling and effects
- **AI-driven optimization** - Intelligent performance tuning via Lazy UI integration

## 🏗️ Architecture

### Core Technologies
- **Language**: Rust (memory safety + performance)
- **Async Runtime**: Tokio for high-performance I/O
- **Graphics**: wgpu for modern GPU acceleration
- **Wayland**: Smithay compositor framework
- **Configuration**: TOML with serde serialization

### Codebase Structure
```
axiom/
├── src/
│   ├── main.rs              # Entry point and CLI
│   ├── compositor.rs        # Main event loop and orchestration
│   ├── config/              # TOML configuration system
│   ├── workspace/           # Scrollable workspace management
│   ├── effects/             # Visual effects engine
│   ├── window/              # Window lifecycle management
│   ├── input/               # Input handling (keyboard/mouse/gestures)
│   ├── xwayland/            # X11 compatibility layer
│   └── ipc/                 # AI integration and IPC communication
├── Cargo.toml              # Dependencies and metadata
├── axiom.toml              # Default configuration
├── test_ipc.py             # IPC testing script
└── STATUS.md               # Detailed development status
```

## 🚀 Development Status

| Phase | Status | Description |
|-------|--------|--------------|
| **Phase 1** | ✅ **COMPLETE** | Basic compositor foundation, IPC, configuration |
| **Phase 2** | ✅ **COMPLETE** | Smithay integration, window management, Wayland protocols |
| **Phase 3** | 🔄 **IN PROGRESS** | Enhanced protocols, input handling, real window integration |
| **Phase 4** | 🔴 Planned | Visual effects system & polish |

**Current Status**: ✅ **Phase 2 Complete!** - Smithay backend integrated with proper Wayland compositor framework. Ready for Phase 3 development!

### ✅ What's Working Now (Phase 1)

- **🏗️ Complete Architecture**: Modular Rust codebase with clean separation of concerns
- **⚙️ Configuration System**: TOML-based config with defaults and validation
- **🔄 Event Loop**: Async Tokio-based main loop running at 60 FPS
- **🤖 IPC Integration**: Unix socket communication with Lazy UI optimization system
- **📊 Performance Monitoring**: Real-time CPU, memory, GPU usage reporting
- **🛡️ Error Handling**: Comprehensive error management with graceful shutdown
- **🔧 CLI Interface**: Full command-line interface with debug and windowed modes
- **📝 Logging**: Structured, emoji-enhanced logging for development and debugging

### ✅ What's New in Phase 2 (COMPLETE!)

- **✅ Smithay Integration**: Real Wayland compositor framework with Smithay 0.3.0
- **✅ Backend Architecture**: Functional backend with proper initialization and shutdown
- **✅ Window Management**: Enhanced AxiomWindow wrapper with properties and lifecycle
- **✅ Event Loop Integration**: Main compositor loop coordinating all subsystems
- **✅ Workspace Integration**: Backend properly connected to scrollable workspace system
- **✅ Error Handling**: Comprehensive error management and graceful shutdown

### 🔄 Currently Working On (Phase 3)

- **📜 Real Protocol Support**: Implementing actual XDG Shell, wl_compositor handlers
- **🖼️ OpenGL Rendering**: Real window rendering pipeline with hardware acceleration
- **⌨️ Input Processing**: Keyboard shortcuts, mouse interactions, and gesture support
- **🖥️ Multi-output Support**: Proper handling of multiple displays and output management
- **🎨 Surface Management**: Wayland surface creation, damage tracking, and composition
- **🧩 Client Communication**: Bidirectional communication with Wayland clients

## 🛠️ Building & Running

### Quick Start

```bash
# Clone and build
git clone https://github.com/GeneticxCln/axiom.git
cd axiom
cargo build --release

# Run in development mode
./target/debug/axiom --debug --windowed

# Run in production
sudo ./target/release/axiom
```

### Dependencies

```bash
# Ubuntu/Debian
sudo apt install libwayland-dev pkg-config build-essential

# Arch Linux 
sudo pacman -S rust wayland wayland-protocols pkg-config

# Fedora
sudo dnf install rust cargo wayland-devel wayland-protocols-devel
```

## ⚙️ Configuration

Axiom uses a single TOML configuration file combining the best of both worlds:

```toml
# ~/.config/axiom/axiom.toml
[workspace]
scroll_speed = 1.0
infinite_scroll = true

[animations] 
enabled = true
duration = 300
curve = "ease-out"

[effects]
blur_radius = 10
rounded_corners = 8
shadow_size = 20

[bindings]
scroll_left = "Super_L+Left"
scroll_right = "Super_L+Right"
```

## 🤖 AI Integration

Axiom seamlessly integrates with the **Lazy UI** optimization system:

### Features
- **Real-time performance monitoring** - CPU, memory, GPU usage tracking
- **Intelligent configuration optimization** - AI-driven parameter tuning
- **Adaptive effects scaling** - Automatic performance adjustments
- **Usage pattern learning** - Optimization based on your workflow

### IPC Communication
- **Unix socket**: `/tmp/axiom-lazy-ui.sock`
- **JSON protocol**: Structured message exchange
- **Async messaging**: Non-blocking optimization updates

### Testing IPC
```bash
# Start Axiom in one terminal
./target/debug/axiom --debug --windowed

# Test IPC communication in another terminal
python3 test_ipc.py
```

## 🤝 Contributing

Axiom is designed to be welcoming to contributors of all skill levels:

- **🐛 Bug Reports**: Help us identify issues
- **💡 Feature Ideas**: Share your vision
- **📝 Code**: Rust developers welcome!
- **📚 Documentation**: Help others understand Axiom

See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

## 🎨 Inspiration

Axiom stands on the shoulders of giants:

- **[niri](https://github.com/YaLTeR/niri)** - Revolutionary scrollable workspace concept
- **[Hyprland](https://github.com/hyprwm/Hyprland)** - Beautiful animations and effects
- **[wlroots](https://gitlab.freedesktop.org/wlroots/wlroots)** - Solid compositor foundation

## 📄 License

GPLv3 - keeping the Linux desktop ecosystem open and free.

## 🌟 Why Axiom?

*"An axiom is a statement that is taken to be true, serving as a premise for further reasoning."*

Our axiom: **You shouldn't have to choose between productivity and beauty.**

---

**Star this repo if you're excited about the future of Wayland compositors! ⭐**
