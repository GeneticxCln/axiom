# Phase 1.1: xdg-shell Robustness Implementation

## Current State Analysis

From examining `/src/smithay/server.rs`, the current xdg-shell implementation has:

### ✅ Currently Implemented:
- Basic xdg_surface creation and configure cycle
- xdg_toplevel role assignment 
- ack_configure handling (lines 7073-7081)
- Initial configure sending (lines 7014-7020)
- Title/app_id handling
- Basic popup support
- Surface lifecycle tracking with `last_sent_configure` and `last_acked_configure`

### ⚠️ Missing Production Features:
1. **Configure/Ack State Machine**: No validation of configure → ack → commit ordering
2. **Timeout Handling**: No deadlines for unresponsive clients  
3. **Surface State Validation**: No proper lifecycle state enforcement
4. **Edge Case Handling**: Unmap during operations, client disconnects
5. **Configure Content**: Static 800x600 size, no dynamic sizing
6. **Error Recovery**: No handling of protocol violations

## Implementation Plan

### Task 1: Surface State Machine (Week 1, Days 1-3)

**Goal**: Implement proper configure/ack/commit state tracking

#### 1.1: Define Surface State Types
```rust
// Add to src/smithay/server.rs or new src/smithay/surface_state.rs

#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug, Clone)]
pub struct PendingConfigure {
    pub size: (i32, i32),
    pub states: Vec<xdg_toplevel::State>,
    pub bounds: Option<(i32, i32)>,
}

#[derive(Debug, Clone)]
pub struct ActiveConfigure {
    pub size: (i32, i32),
    pub states: Vec<xdg_toplevel::State>,
}
```

#### 1.2: Extend Window Entry Structure
```rust
// Modify WindowEntry in src/smithay/server.rs
pub struct WindowEntry {
    // ... existing fields ...
    
    // Enhanced state tracking
    pub lifecycle_state: SurfaceLifecycle,
    pub pending_configures: VecDeque<PendingConfigure>,
    pub configure_timeout: Duration, // default 5 seconds
    
    // Buffer state tracking  
    pub has_pending_buffer: bool,
    pub buffer_committed: bool,
}
```

#### 1.3: Update Configure Sending Logic
```rust
impl CompositorServer {
    fn send_configure_to_surface(
        state: &mut CompositorState,
        window_entry: &mut WindowEntry,
        size: (i32, i32),
        states: Vec<xdg_toplevel::State>,
    ) -> Result<u32> {
        let serial = state.next_serial();
        let deadline = Instant::now() + window_entry.configure_timeout;
        
        let configure = PendingConfigure {
            size,
            states: states.clone(),
            bounds: None,
        };
        
        // Send the configure
        if let Some(ref toplevel) = window_entry.xdg_toplevel {
            toplevel.configure(size.0, size.1, states);
        }
        
        if let Some(ref xdg_surface) = Some(&window_entry.xdg_surface) {
            xdg_surface.configure(serial);
        }
        
        // Update state
        window_entry.lifecycle_state = SurfaceLifecycle::AwaitingAck {
            serial,
            deadline,
            configure: configure.clone(),
        };
        
        window_entry.pending_configures.push_back(configure);
        window_entry.last_sent_configure = Some(serial);
        
        log::debug!("📐 Sent configure serial={} size={}x{} to surface", 
                   serial, size.0, size.1);
        
        Ok(serial)
    }
}
```

#### 1.4: Enhance ack_configure Handling
```rust
// Update the existing ack_configure handler in xdg_surface dispatch
xdg_surface::Request::AckConfigure { serial } => {
    if let Some(win) = state
        .windows
        .iter_mut()
        .find(|w| w.xdg_surface == *resource)
    {
        let result = handle_ack_configure(win, serial);
        match result {
            Ok(_) => {
                log::debug!("✅ Valid ack_configure serial={}", serial);
            }
            Err(e) => {
                log::warn!("⚠️ Invalid ack_configure serial={}: {}", serial, e);
                // Optionally disconnect client for protocol violation
            }
        }
    }
}

fn handle_ack_configure(
    window: &mut WindowEntry, 
    acked_serial: u32
) -> Result<()> {
    match &window.lifecycle_state {
        SurfaceLifecycle::AwaitingAck { serial, configure, .. } => {
            if *serial == acked_serial {
                let deadline = Instant::now() + window.configure_timeout;
                window.lifecycle_state = SurfaceLifecycle::AwaitingCommit {
                    serial: acked_serial,
                    deadline,
                    configure: configure.clone(),
                };
                window.last_acked_configure = Some(acked_serial);
                Ok(())
            } else {
                Err(anyhow::anyhow!("Serial mismatch: expected {}, got {}", serial, acked_serial))
            }
        }
        _ => {
            Err(anyhow::anyhow!("ack_configure in wrong state: {:?}", window.lifecycle_state))
        }
    }
}
```

### Task 2: Buffer Commit State Integration (Week 1, Days 4-5)

#### 2.1: Track Buffer Commits
```rust
// Update the wl_surface commit handler
wl_surface::Request::Commit => {
    // ... existing damage/callback logic ...
    
    // Handle surface state completion
    if let Some(win) = state.windows.iter_mut()
        .find(|w| w.wl_surface.as_ref() == Some(resource)) 
    {
        handle_surface_commit(win, resource)?;
    }
}

fn handle_surface_commit(
    window: &mut WindowEntry,
    surface: &wl_surface::WlSurface,
) -> Result<()> {
    match &window.lifecycle_state {
        SurfaceLifecycle::AwaitingCommit { serial, configure, .. } => {
            // Transition to fully configured
            window.lifecycle_state = SurfaceLifecycle::Configured {
                serial: *serial,
                active_config: ActiveConfigure {
                    size: configure.size,
                    states: configure.states.clone(),
                },
            };
            
            // Mark as mapped if has buffer and not previously mapped
            if !window.mapped && window.has_pending_buffer {
                window.mapped = true;
                log::info!("🗺️ Surface mapped with size {}x{}", 
                          configure.size.0, configure.size.1);
            }
            
            Ok(())
        }
        _ => Ok(()) // Allow commits in other states
    }
}
```

### Task 3: Timeout and Error Handling (Week 1, Day 6-7)

#### 3.1: Periodic Timeout Checking
```rust
impl CompositorServer {
    /// Check for timed out configures and handle unresponsive clients
    fn check_configure_timeouts(state: &mut CompositorState) {
        let now = Instant::now();
        let mut timed_out_windows = Vec::new();
        
        for window in &mut state.windows {
            match &window.lifecycle_state {
                SurfaceLifecycle::AwaitingAck { deadline, serial, .. } |
                SurfaceLifecycle::AwaitingCommit { deadline, serial, .. } => {
                    if now > *deadline {
                        timed_out_windows.push((window.axiom_id, *serial));
                    }
                }
                _ => {}
            }
        }
        
        for (window_id, serial) in timed_out_windows {
            handle_configure_timeout(state, window_id, serial);
        }
    }
    
    fn handle_configure_timeout(
        state: &mut CompositorState,
        window_id: Option<u64>,
        serial: u32,
    ) {
        log::warn!("⏰ Configure timeout for window {:?} serial={}", window_id, serial);
        
        // Find the window and reset to a safe state
        if let Some(win) = state.windows.iter_mut()
            .find(|w| w.axiom_id == window_id) 
        {
            // Reset to unmapped state
            win.lifecycle_state = SurfaceLifecycle::Unmapped;
            win.mapped = false;
            
            // Optionally: Send a new configure to recover
            let _ = Self::send_configure_to_surface(
                state, 
                win, 
                (800, 600), 
                vec![]
            );
        }
    }
}
```

#### 3.2: Integrate Timeout Checking in Main Loop
```rust
// Add to the main server dispatch loop in CompositorServer::run
// Around line 1426 in the existing timer callback

// Check for configure timeouts every 1 second
if last_timeout_check.elapsed() >= Duration::from_secs(1) {
    Self::check_configure_timeouts(data);
    last_timeout_check = Instant::now();
}
```

### Task 4: Dynamic Configure Logic (Week 2, Days 1-2)

#### 4.1: Window Manager Integration
```rust
impl CompositorServer {
    fn calculate_window_configure(
        &self,
        state: &CompositorState,
        window_id: u64,
    ) -> (i32, i32, Vec<xdg_toplevel::State>) {
        let wm = self.window_manager.read();
        
        // Get window from window manager
        if let Some(axiom_window) = wm.get_window(window_id) {
            let mut states = Vec::new();
            let mut size = (800, 600); // default
            
            // Apply window manager state
            if axiom_window.properties.maximized {
                states.push(xdg_toplevel::State::Maximized);
                // Get output size for maximized windows
                size = self.get_maximized_size(state);
            }
            
            if axiom_window.properties.minimized {
                // Minimized windows get 0x0 size hint
                size = (0, 0);
            }
            
            if axiom_window.properties.fullscreen {
                states.push(xdg_toplevel::State::Fullscreen);
                size = self.get_fullscreen_size(state);
            }
            
            // Add activation state if focused
            if wm.focused_window_id() == Some(window_id) {
                states.push(xdg_toplevel::State::Activated);
            }
            
            (size.0, size.1, states)
        } else {
            (800, 600, vec![])
        }
    }
    
    fn get_maximized_size(&self, state: &CompositorState) -> (i32, i32) {
        if let Some(output) = state.logical_outputs.first() {
            (output.width as i32, output.height as i32)
        } else {
            (1920, 1080)
        }
    }
    
    fn get_fullscreen_size(&self, state: &CompositorState) -> (i32, i32) {
        // Same as maximized for now
        self.get_maximized_size(state)
    }
}
```

### Task 5: Test Suite Implementation (Week 2, Days 3-5)

#### 5.1: Protocol Conformance Tests
```rust
// Create tests/protocol_conformance_tests.rs
use axiom::test_utils::{TestClient, MockCompositor};

#[tokio::test]
async fn test_xdg_surface_configure_ack_cycle() {
    let mut compositor = MockCompositor::new().await;
    let mut client = TestClient::connect(&compositor.socket_name).await?;
    
    // Create surface and xdg_surface
    let surface = client.create_surface().await;
    let xdg_surface = client.create_xdg_surface(&surface).await;
    let toplevel = client.create_xdg_toplevel(&xdg_surface).await;
    
    // Should receive initial configure
    let configure_event = client.expect_configure().await?;
    assert_eq!(configure_event.width, 800);
    assert_eq!(configure_event.height, 600);
    
    // Ack the configure
    xdg_surface.ack_configure(configure_event.serial);
    
    // Commit to complete the cycle
    surface.commit();
    
    // Surface should now be mapped
    assert!(compositor.is_surface_mapped(&surface));
}

#[tokio::test] 
async fn test_configure_timeout_handling() {
    let mut compositor = MockCompositor::new().await;
    let mut client = TestClient::connect(&compositor.socket_name).await?;
    
    let surface = client.create_surface().await;
    let xdg_surface = client.create_xdg_surface(&surface).await;
    let _toplevel = client.create_xdg_toplevel(&xdg_surface).await;
    
    // Receive configure but don't ack
    let configure_event = client.expect_configure().await?;
    
    // Wait for timeout (should be ~5 seconds)
    tokio::time::sleep(Duration::from_secs(6)).await;
    
    // Surface should be unmapped due to timeout
    assert!(!compositor.is_surface_mapped(&surface));
}

#[tokio::test]
async fn test_rapid_configure_sequence() {
    let mut compositor = MockCompositor::new().await;
    let mut client = TestClient::connect(&compositor.socket_name).await?;
    
    // Test rapid window resize scenario
    let surface = client.create_surface().await;
    let xdg_surface = client.create_xdg_surface(&surface).await;
    let _toplevel = client.create_xdg_toplevel(&xdg_surface).await;
    
    // Trigger multiple rapid resizes
    for size in [(400, 300), (500, 400), (600, 500), (700, 600)] {
        compositor.request_window_resize(size);
        
        let configure = client.expect_configure().await?;
        assert_eq!((configure.width, configure.height), size);
        
        xdg_surface.ack_configure(configure.serial);
        surface.commit();
    }
    
    // Final state should match last configure
    let final_size = compositor.get_surface_size(&surface);
    assert_eq!(final_size, (700, 600));
}
```

#### 5.2: Test Utilities
```rust
// Create src/test_utils.rs
pub struct MockCompositor {
    pub socket_name: String,
    server_handle: tokio::task::JoinHandle<()>,
}

impl MockCompositor {
    pub async fn new() -> Self {
        // Start minimal compositor server for testing
        // Implementation details...
    }
    
    pub fn request_window_resize(&mut self, size: (i32, i32)) {
        // Trigger resize through window manager
    }
    
    pub fn is_surface_mapped(&self, surface: &wl_surface::WlSurface) -> bool {
        // Check if surface is mapped in compositor state
    }
}

pub struct TestClient {
    connection: wayland_client::Connection,
    event_queue: wayland_client::EventQueue<()>,
}

impl TestClient {
    pub async fn connect(socket_name: &str) -> Result<Self> {
        // Connect to test compositor
        // Implementation details...
    }
    
    pub async fn expect_configure(&mut self) -> Result<ConfigureEvent> {
        // Wait for and parse configure event
        // Implementation details...
    }
}
```

### Task 6: Integration and Validation (Week 2, Days 6-7)

#### 6.1: End-to-End Testing
```bash
# Test with real applications
cargo build --release

# Terminal apps
alacritty &
WAYLAND_DISPLAY=wayland-1 gnome-terminal &

# GUI apps  
firefox &
code &

# Validate window lifecycle with debug logging
RUST_LOG=axiom::smithay=debug ./target/release/axiom
```

#### 6.2: Performance Impact Assessment
```rust
// Add timing metrics to configure handling
use std::time::Instant;

let start = Instant::now();
let result = handle_ack_configure(win, serial);
let duration = start.elapsed();

if duration > Duration::from_millis(1) {
    log::warn!("Slow configure handling: {:?}", duration);
}
```

## Success Criteria

### Functional Requirements
- [ ] **Configure/Ack/Commit cycle**: All steps properly validated
- [ ] **Timeout handling**: Unresponsive clients handled gracefully  
- [ ] **Dynamic sizing**: Window manager state reflected in configures
- [ ] **Error recovery**: Protocol violations logged and handled
- [ ] **Real app compatibility**: Firefox, VSCode, terminals work reliably

### Performance Requirements  
- [ ] **Configure latency**: < 1ms average handling time
- [ ] **Memory overhead**: < 1KB per window for state tracking
- [ ] **Timeout efficiency**: No busy polling, event-driven timeouts

### Testing Requirements
- [ ] **Unit tests**: 90%+ coverage of state machine logic
- [ ] **Integration tests**: All lifecycle scenarios covered
- [ ] **Real app tests**: Manual validation with common applications
- [ ] **Stress tests**: Rapid configure sequences, many windows

## Implementation Timeline

| Week | Days | Tasks | Deliverable |
|------|------|-------|-------------|
| 1    | 1-3  | Surface state machine | Working state tracking |
| 1    | 4-5  | Buffer commit integration | Complete configure cycle |
| 1    | 6-7  | Timeout and error handling | Robust error recovery |
| 2    | 1-2  | Dynamic configure logic | WM integration |
| 2    | 3-5  | Test suite implementation | Automated test coverage |
| 2    | 6-7  | Integration and validation | Production readiness |

## Risk Mitigation

### Technical Risks
- **Smithay API changes**: Pin smithay version, track upstream
- **State complexity**: Start simple, add complexity incrementally
- **Performance regressions**: Benchmark before/after changes

### Integration Risks  
- **Window manager coupling**: Keep xdg-shell logic separate
- **Test environment**: Use containers for reproducible testing
- **Real app testing**: Test early and often with real applications

Would you like me to start implementing any specific part of this plan? I recommend beginning with **Task 1.1: Surface State Types** as it's the foundation for everything else.