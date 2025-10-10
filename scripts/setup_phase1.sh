#!/bin/bash
# Phase 1 Development Setup Script
# Sets up environment for xdg-shell robustness implementation

set -e

echo "🚀 Setting up Axiom Phase 1 Development Environment"

# Install additional dependencies
echo "📦 Installing development dependencies..."
if command -v pacman &> /dev/null; then
    # CachyOS/Arch
    sudo pacman -S --needed --noconfirm \
        tracy \
        wayland-protocols \
        wayland-utils \
        weston \
        alacritty
else
    echo "⚠️ Manual dependency installation required for non-Arch systems"
fi

# Install Rust development tools
echo "🦀 Installing Rust development tools..."
cargo install --locked \
    cargo-deny \
    cargo-audit \
    cargo-watch \
    cargo-nextest

# Create branch structure
echo "🌿 Creating development branches..."
git checkout -b phase-1/xdg-shell-robustness || echo "Branch already exists"

# Create directory structure for new files
mkdir -p src/smithay/protocols
mkdir -p src/test_utils
mkdir -p tests/protocol_tests

# Create initial files
echo "📝 Creating initial implementation files..."

# Surface state types
cat > src/smithay/surface_state.rs << 'EOF'
//! Surface lifecycle and state management for xdg-shell
//! 
//! This module implements proper configure/ack/commit state tracking
//! for robust xdg-shell protocol handling.

use anyhow::Result;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use wayland_protocols::xdg::shell::server::xdg_toplevel;

/// Surface lifecycle state machine
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceLifecycle {
    /// Surface created but no role assigned
    Created,
    /// Configure sent, waiting for ack
    AwaitingAck {
        serial: u32,
        deadline: Instant,
        configure: PendingConfigure,
    },
    /// Ack received, waiting for commit
    AwaitingCommit {
        serial: u32,
        deadline: Instant,
        configure: PendingConfigure,
    },
    /// Fully configured and ready
    Configured {
        serial: u32,
        active_config: ActiveConfigure,
    },
    /// Surface unmapped (but not destroyed)
    Unmapped,
    /// Surface destroyed
    Destroyed,
}

/// Configure that has been sent but not yet acknowledged
#[derive(Debug, Clone)]
pub struct PendingConfigure {
    pub size: (i32, i32),
    pub states: Vec<xdg_toplevel::State>,
    pub bounds: Option<(i32, i32)>,
}

/// Active configuration after ack + commit
#[derive(Debug, Clone)]
pub struct ActiveConfigure {
    pub size: (i32, i32),
    pub states: Vec<xdg_toplevel::State>,
}

/// Enhanced surface state manager
#[derive(Debug)]
pub struct SurfaceStateManager {
    pub lifecycle_state: SurfaceLifecycle,
    pub pending_configures: VecDeque<PendingConfigure>,
    pub configure_timeout: Duration,
    pub has_pending_buffer: bool,
    pub buffer_committed: bool,
}

impl Default for SurfaceStateManager {
    fn default() -> Self {
        Self {
            lifecycle_state: SurfaceLifecycle::Created,
            pending_configures: VecDeque::new(),
            configure_timeout: Duration::from_secs(5),
            has_pending_buffer: false,
            buffer_committed: false,
        }
    }
}

impl SurfaceStateManager {
    /// Handle acknowledgment of a configure event
    pub fn handle_ack_configure(&mut self, acked_serial: u32) -> Result<()> {
        match &self.lifecycle_state {
            SurfaceLifecycle::AwaitingAck { serial, configure, .. } => {
                if *serial == acked_serial {
                    let deadline = Instant::now() + self.configure_timeout;
                    self.lifecycle_state = SurfaceLifecycle::AwaitingCommit {
                        serial: acked_serial,
                        deadline,
                        configure: configure.clone(),
                    };
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "Serial mismatch: expected {}, got {}",
                        serial,
                        acked_serial
                    ))
                }
            }
            _ => Err(anyhow::anyhow!(
                "ack_configure in wrong state: {:?}",
                self.lifecycle_state
            )),
        }
    }

    /// Handle surface commit
    pub fn handle_commit(&mut self) -> Result<bool> {
        match &self.lifecycle_state {
            SurfaceLifecycle::AwaitingCommit { serial, configure, .. } => {
                self.lifecycle_state = SurfaceLifecycle::Configured {
                    serial: *serial,
                    active_config: ActiveConfigure {
                        size: configure.size,
                        states: configure.states.clone(),
                    },
                };
                
                // Return true if this should trigger mapping
                Ok(!self.buffer_committed && self.has_pending_buffer)
            }
            _ => Ok(false), // Allow commits in other states
        }
    }

    /// Check if configure has timed out
    pub fn check_timeout(&self) -> Option<u32> {
        let now = Instant::now();
        match &self.lifecycle_state {
            SurfaceLifecycle::AwaitingAck { deadline, serial, .. }
            | SurfaceLifecycle::AwaitingCommit { deadline, serial, .. } => {
                if now > *deadline {
                    Some(*serial)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Transition to awaiting ack state
    pub fn send_configure(
        &mut self,
        serial: u32,
        size: (i32, i32),
        states: Vec<xdg_toplevel::State>,
    ) {
        let deadline = Instant::now() + self.configure_timeout;
        let configure = PendingConfigure {
            size,
            states: states.clone(),
            bounds: None,
        };

        self.lifecycle_state = SurfaceLifecycle::AwaitingAck {
            serial,
            deadline,
            configure: configure.clone(),
        };

        self.pending_configures.push_back(configure);
    }

    /// Reset to unmapped state (timeout recovery)
    pub fn reset_to_unmapped(&mut self) {
        self.lifecycle_state = SurfaceLifecycle::Unmapped;
        self.pending_configures.clear();
    }
}
EOF

# Test utilities module
cat > src/test_utils.rs << 'EOF'
//! Test utilities for Axiom compositor testing
//! 
//! Provides mock compositor and client implementations for
//! protocol conformance testing.

#[cfg(test)]
pub mod protocol_testing {
    use anyhow::Result;
    use std::time::Duration;
    use tokio::sync::mpsc;
    
    /// Mock compositor for protocol testing
    pub struct MockCompositor {
        pub socket_name: String,
        shutdown_tx: Option<mpsc::Sender<()>>,
    }
    
    impl MockCompositor {
        pub async fn new() -> Result<Self> {
            // TODO: Implement minimal compositor for testing
            Ok(Self {
                socket_name: "wayland-test-0".to_string(),
                shutdown_tx: None,
            })
        }
        
        pub async fn shutdown(mut self) -> Result<()> {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(()).await;
            }
            Ok(())
        }
    }
    
    /// Test client for protocol validation
    pub struct TestClient {
        _socket_name: String,
    }
    
    impl TestClient {
        pub async fn connect(socket_name: &str) -> Result<Self> {
            // TODO: Implement Wayland client for testing
            Ok(Self {
                _socket_name: socket_name.to_string(),
            })
        }
        
        pub async fn expect_configure(&mut self) -> Result<ConfigureEvent> {
            // TODO: Implement configure event waiting
            Ok(ConfigureEvent {
                serial: 1,
                width: 800,
                height: 600,
                states: vec![],
            })
        }
    }
    
    #[derive(Debug, Clone)]
    pub struct ConfigureEvent {
        pub serial: u32,
        pub width: i32,
        pub height: i32,
        pub states: Vec<u32>, // Simplified for now
    }
}
EOF

# Basic protocol test structure
cat > tests/protocol_conformance_tests.rs << 'EOF'
//! Protocol conformance tests for xdg-shell
//! 
//! These tests validate proper xdg-shell protocol implementation
//! including configure/ack/commit cycles and error handling.

#[cfg(test)]
mod xdg_shell_tests {
    use axiom::test_utils::protocol_testing::*;
    use std::time::Duration;
    
    #[tokio::test]
    async fn test_basic_configure_ack_cycle() {
        // TODO: Implement when MockCompositor is ready
        // This test validates the basic xdg_surface configure -> ack -> commit cycle
    }
    
    #[tokio::test]
    async fn test_configure_timeout() {
        // TODO: Implement timeout testing
        // This test validates that unresponsive clients are handled properly
    }
    
    #[tokio::test] 
    async fn test_rapid_configure_sequence() {
        // TODO: Implement rapid resize testing
        // This test validates handling of multiple rapid configure events
    }
}
EOF

# Update module declarations
echo "📝 Updating module declarations..."

# Add surface_state module to lib.rs if not already present
if ! grep -q "pub mod surface_state" src/lib.rs 2>/dev/null; then
    echo "pub mod surface_state;" >> src/lib.rs
fi

if ! grep -q "pub mod test_utils" src/lib.rs 2>/dev/null; then
    echo "pub mod test_utils;" >> src/lib.rs
fi

# Create development tasks file
cat > PHASE1_TASKS.md << 'EOF'
# Phase 1 Development Tasks

## Current Status: Setup Complete ✅

### Next Steps:

1. **Implement Surface State Manager**
   ```bash
   # Edit src/smithay/surface_state.rs and implement remaining TODOs
   cargo test --lib surface_state
   ```

2. **Integrate with Smithay Server**
   ```bash
   # Edit src/smithay/server.rs to use SurfaceStateManager
   # Update WindowEntry to include surface_state_manager field
   ```

3. **Add Configure Sending Logic**
   ```bash
   # Implement send_configure_to_surface method
   # Update xdg_surface dispatch handlers
   ```

4. **Add Timeout Checking**
   ```bash
   # Add timeout checking to main server loop
   # Implement timeout recovery logic
   ```

5. **Test with Real Applications**
   ```bash
   # Build and test
   cargo build --release
   RUST_LOG=axiom::smithay=debug ./target/release/axiom
   
   # In another terminal:
   alacritty
   ```

### Development Commands:

```bash
# Watch for changes and run tests
cargo watch -x "test --lib"

# Run with debug logging
RUST_LOG=axiom=debug cargo run

# Run protocol tests
cargo test --test protocol_conformance_tests

# Check for issues
cargo clippy
cargo audit
```
EOF

echo "✅ Phase 1 development environment setup complete!"
echo ""
echo "📋 Next steps:"
echo "1. Review the implementation plan: docs/PHASE_1_XDG_SHELL_IMPLEMENTATION.md"
echo "2. Follow the tasks in: PHASE1_TASKS.md"
echo "3. Start implementing: edit src/smithay/surface_state.rs"
echo ""
echo "🚀 Ready to begin Phase 1 implementation!"