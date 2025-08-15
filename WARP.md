# WARP.md

This file provides guidance to WARP (warp.dev) when working with code in this repository.

## Development Commands

### Building and Running

```bash
# Build the project (debug mode for development)
cargo build

# Build optimized release version
cargo build --release

# Run in development mode with debug logging
./target/debug/axiom --debug --windowed

# Run in production mode (requires root access)
sudo ./target/release/axiom

# Run with effects disabled (performance mode)
./target/debug/axiom --debug --windowed --no-effects
```

### Testing

```bash
# Run unit tests
cargo test

# Run unit and integration tests
cargo test --all-targets

# Run tests with output
cargo test -- --nocapture

# Run specific test module
cargo test workspace

# Run IPC communication test (requires running compositor)
python3 test_ipc.py
```

### Demos and Development Testing

```bash
# Run Phase 3 scrollable workspace demo
./target/debug/axiom --debug --windowed --demo

# Run Phase 4 visual effects demo  
./target/debug/axiom --debug --windowed --effects-demo

# Run both demos
./target/debug/axiom --debug --windowed --demo --effects-demo
```

### Development Tools and Quality

```bash
# Format code
cargo fmt

# Run linter
cargo clippy --all-targets --all-features

# Check compilation without building
cargo check

# Run benchmarks
cargo bench

# Generate documentation
cargo doc --open

# Run security audit
cargo audit

# Check for outdated dependencies
cargo outdated
```

### Phase 5 Development Setup

```bash
# Run the Phase 5 development kickoff script
./scripts/phase5_kickoff.sh

# Install additional cargo tools for development
cargo install cargo-audit cargo-tarpaulin cargo-expand cargo-outdated
```

### Configuration and IPC

```bash
# Copy default configuration
cp config/axiom.toml ~/.config/axiom/axiom.toml

# Test IPC communication (after starting compositor)
python3 test_ipc.py

# Run compositor with custom config
./target/debug/axiom --config /path/to/custom/axiom.toml --debug --windowed
```

## Architecture Overview

Axiom is a hybrid Wayland compositor combining niri's innovative scrollable workspaces with Hyprland's beautiful visual effects. The codebase is structured around modular subsystems that work together to provide both productivity and aesthetics.

### Core Philosophy
- **Infinite Scrollable Workspaces**: Horizontal workspace scrolling without artificial limits
- **Beautiful Visual Effects**: GPU-accelerated animations, blur, shadows, and rounded corners
- **AI-Driven Optimization**: Real-time performance tuning via Lazy UI integration
- **Production Ready**: Built with Rust for memory safety and performance

### Project Structure

```
src/
├── main.rs              # Entry point with CLI handling
├── compositor.rs        # Core event loop and subsystem coordination
├── smithay_backend.rs   # Smithay Wayland compositor integration
├── workspace/           # Scrollable workspace management (niri-inspired)
├── effects/             # Visual effects engine (Hyprland-inspired)
├── window/              # Window lifecycle and positioning
├── input/               # Input event processing and gesture recognition
├── config/              # TOML configuration parsing and validation
├── xwayland/            # X11 compatibility layer
└── ipc/                 # Lazy UI integration and IPC communication
```

### Key Subsystems

#### Compositor Core (`compositor.rs`)
The `AxiomCompositor` orchestrates all subsystems and runs the main event loop. It handles:
- Tokio-based async event processing
- Signal handling (SIGTERM, SIGINT) for graceful shutdown
- Integration between workspace management, effects, and input systems
- Communication with Smithay backend for Wayland protocol handling

#### Scrollable Workspaces (`workspace/`)
Implements niri-style infinite horizontal scrolling:
- Dynamic window placement algorithms
- Smooth scrolling with configurable speed and curves
- Column-based layout with intelligent window positioning
- Multi-monitor support with independent workspace scrolling

#### Visual Effects Engine (`effects/`)
Hyprland-inspired effects system with GPU acceleration:
- Animation framework with configurable timing curves
- Real-time blur effects for windows and backgrounds
- Rounded corners with anti-aliasing
- Drop shadows with realistic lighting
- Workspace transition animations
- Adaptive performance scaling based on system load

#### Input Management (`input/`)
Comprehensive input handling system:
- Keyboard shortcut processing with modifier support
- Mouse and trackpad gesture recognition
- Touch input support for scrollable workspaces
- Configurable key bindings via TOML
- Input event simulation for testing

#### IPC Integration (`ipc/`)
Lazy UI communication system for AI optimization:
- Unix socket-based JSON protocol
- Real-time performance metrics reporting
- Configuration optimization commands
- Health monitoring and status reporting
- Non-blocking async message processing

### Configuration System

Axiom uses a unified TOML configuration file that combines both workspace and effects settings:

```toml
[workspace]
scroll_speed = 1.0
infinite_scroll = true
gaps = 10

[effects]
enabled = true

[effects.animations]
duration = 300
curve = "ease-out"

[effects.blur]
radius = 10
intensity = 0.8

[bindings]
scroll_left = "Super_L+Left"
scroll_right = "Super_L+Right"
```

### Development Phases

The project follows a structured development approach:

- **Phase 1-2 (COMPLETE)**: Basic compositor foundation and Smithay integration
- **Phase 3 (COMPLETE)**: Enhanced protocols, input handling, real window integration  
- **Phase 4 (COMPLETE)**: Visual effects system with GPU-accelerated animations
- **Phase 5 (CURRENT)**: Production readiness, comprehensive testing, and packaging

### AI Integration

Axiom is designed to work with Lazy UI for intelligent performance optimization:
- Real-time performance monitoring (CPU, memory, GPU usage)
- Automatic quality scaling during intensive operations
- Usage pattern learning for predictive optimization
- Adaptive configuration updates based on system load

### Testing Strategy

The project maintains comprehensive test coverage:
- **Unit Tests**: Individual module functionality
- **Integration Tests**: End-to-end compositor functionality  
- **IPC Tests**: Communication with external systems
- **Performance Tests**: Benchmarking and regression detection
- **Demo Systems**: Interactive testing of major features

### Build System

Uses Cargo with optimized build profiles:
- **Debug Profile**: Fast compilation with opt-level 1
- **Release Profile**: Full LTO optimization with panic=abort
- **Dependencies**: Modern Rust ecosystem (Tokio, Smithay, wgpu, serde)

### Key Dependencies

- **Smithay**: Wayland compositor framework
- **wgpu**: Modern GPU acceleration
- **Tokio**: Async runtime
- **serde/TOML**: Configuration management
- **interprocess**: IPC communication
- **cgmath**: 3D mathematics for effects

## Development Guidelines

### Code Organization
- Each major subsystem is in its own module with clear interfaces
- Use `anyhow::Result` for error handling with context
- Async/await throughout for non-blocking operations
- Structured logging with emoji-enhanced output for development

### Testing Approach
- Write unit tests for core logic functions
- Create integration tests for subsystem interactions
- Use the demo systems to verify major functionality
- Test IPC communication separately with `test_ipc.py`

### Performance Considerations
- Effects can be disabled via `--no-effects` for debugging
- GPU memory management is handled by the effects engine
- Frame timing is adaptive with configurable target FPS
- Use windowed mode (`--windowed`) for development testing

### Configuration Management
- Default config is embedded in the binary
- User config overrides defaults via TOML merging
- Runtime configuration updates are supported via IPC
- Validate all configuration on load with helpful error messages

### IPC Protocol
- JSON-based messages over Unix sockets
- Non-blocking async message processing
- Health checks and status reporting
- Configuration optimization commands from AI systems
