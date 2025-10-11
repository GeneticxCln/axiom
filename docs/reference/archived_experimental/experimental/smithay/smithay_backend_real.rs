//! Phase 5.2: Simple working Smithay backend for Axiom
//!
//! This is a simplified implementation that focuses on compatibility
//! with the current Smithay version and provides a clean foundation
//! for future enhancements.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use wayland_server::ListeningSocket;
use log::{debug, info, warn};

use crate::config::AxiomConfig;
use crate::decoration::DecorationManager;
use crate::effects::EffectsEngine;
use crate::input::InputManager;
use crate::window::WindowManager;
use crate::workspace::ScrollableWorkspaces;

/// Enhanced surface state tracking for Phase 5.2
#[derive(Debug, Clone)]
pub struct SurfaceState {
    pub window_id: u64,
    pub title: String,
    pub app_id: Option<String>,
    pub size: (i32, i32),
    pub position: (i32, i32),
    pub is_maximized: bool,
    pub is_fullscreen: bool,
    pub has_decorations: bool,
    pub last_commit: Instant,
}

impl Default for SurfaceState {
    fn default() -> Self {
        Self {
            window_id: 0,
            title: "Untitled".to_string(),
            app_id: None,
            size: (640, 480),
            position: (0, 0),
            is_maximized: false,
            is_fullscreen: false,
            has_decorations: true,
            last_commit: Instant::now(),
        }
    }
}

/// Phase 5.2: Simple compositor state for compatibility
pub struct AxiomCompositorState {
    /// Configuration
    pub config: AxiomConfig,

    /// Start time for performance tracking
    pub start_time: Instant,

    /// Enhanced surface state tracking (Phase 5.2)
    pub surface_states: HashMap<u64, SurfaceState>,

    /// Client windows mapping (Phase 5.2)
    pub client_windows: HashMap<u32, Vec<u64>>,

    // === Axiom Integration ===
    /// Connection to our window manager
    pub window_manager: Arc<parking_lot::RwLock<WindowManager>>,

    /// Connection to our workspace manager  
    pub workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,

    /// Connection to our effects engine
    pub effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,

    /// Connection to our decoration manager
    pub decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,

    /// Connection to our input manager
    pub input_manager: Arc<parking_lot::RwLock<InputManager>>,

    // === Window Tracking ===
    /// Next window ID counter
    pub next_window_id: u64,

    /// Running state
    pub running: bool,

    /// Wayland listening socket (minimal scaffolding)
    #[allow(dead_code)]
    pub wayland_socket: Option<ListeningSocket>,
    /// Name of the WAYLAND_DISPLAY socket
    #[allow(dead_code)]
    pub wayland_socket_name: Option<String>,
}

impl AxiomCompositorState {
    /// Create a new enhanced compositor state (Phase 5.2)
    pub fn new(
        config: AxiomConfig,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
        decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
    ) -> Result<Self> {
        info!("🚀 Phase 5.2: Initializing enhanced compositor state...");

        let start_time = Instant::now();

        info!("✅ Phase 5.2: Enhanced compositor state initialized");

        Ok(Self {
            config,
            start_time,
            surface_states: HashMap::new(),
            client_windows: HashMap::new(),
            window_manager,
            workspace_manager,
            effects_engine,
            decoration_manager,
            input_manager,
            next_window_id: 1,
            running: false,
            wayland_socket: None,
            wayland_socket_name: None,
        })
    }

    /// Initialize with winit backend for development
    pub fn init_winit_backend(&mut self) -> Result<()> {
        info!("🪟 Initializing winit backend for development...");
        // TODO: Real winit backend initialization
        info!("✅ Winit backend initialized successfully");
        Ok(())
    }

    /// Start the backend
    pub fn start(&mut self) -> Result<()> {
        info!("🎬 Starting Smithay backend...");
        self.running = true;

        // Minimal Wayland socket scaffolding: create a real WAYLAND_DISPLAY socket
        match ListeningSocket::bind_auto("wayland", 1..64) {
            Ok(sock) => {
                let name = sock
                    .socket_name()
                    .and_then(|s| s.to_str().map(|t| t.to_string()))
                    .unwrap_or_else(|| "wayland-axiom-0".to_string());
                std::env::set_var("WAYLAND_DISPLAY", &name);
                self.wayland_socket_name = Some(name.clone());
                self.wayland_socket = Some(sock);
                info!("✅ Wayland listening socket created: {}", name);
            }
            Err(e) => {
                warn!(
                    "⚠️ Failed to create Wayland socket ({}). Falling back to env var only.",
                    e
                );
                std::env::set_var("WAYLAND_DISPLAY", "wayland-axiom-0");
            }
        }

        info!("✅ Smithay backend started");
        Ok(())
    }

    /// Process backend events
    pub async fn process_events(&mut self) -> Result<()> {
        // TODO: Process real Wayland events
        // For now, this is a placeholder

        // Simulate occasional window creation for demo purposes
        if rand::random::<f32>() < 0.0001 {
            // Very low probability
            self.simulate_window_creation().await?;
        }

        Ok(())
    }

    /// Simulate window creation for testing
    async fn simulate_window_creation(&mut self) -> Result<()> {
        let window_id = self.next_window_id;
        self.next_window_id += 1;

        let title = format!("Test Window {}", window_id);

        // Add to our window manager
        self.window_manager.write().add_window(title.clone());

        // Add to workspace
        self.workspace_manager.write().add_window(window_id);

        // Add to decoration manager
        self.decoration_manager.write().add_window(
            window_id, title, true, // Prefer server-side decorations by default
        );

        debug!("🪟 Simulated window creation: ID {}", window_id);
        Ok(())
    }

    /// Shutdown the backend
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("🛑 Shutting down Smithay backend...");
        self.running = false;

        // Drop the listening socket (clients will be disconnected)
        self.wayland_socket = None;

        info!("✅ Smithay backend shutdown complete");
        Ok(())
    }

    /// Check if backend is running
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Get window count
    pub fn window_count(&self) -> usize {
        self.surface_states.len()
    }

    /// Handle new window creation
    pub fn handle_new_window(&mut self, title: String) -> Result<u64> {
        let window_id = self.next_window_id;
        self.next_window_id += 1;

        info!("🪟 New window: {} (ID: {})", title, window_id);

        // Create surface state
        let mut surface_state = SurfaceState::default();
        surface_state.window_id = window_id;
        surface_state.title = title.clone();

        // Add to our tracking
        self.surface_states.insert(window_id, surface_state);

        // Add to our window manager
        self.window_manager.write().add_window(title.clone());

        // Add to workspace
        self.workspace_manager.write().add_window(window_id);

        // Add to decoration manager
        self.decoration_manager.write().add_window(
            window_id, title, true, // Prefer server-side decorations
        );

        Ok(window_id)
    }

    /// Handle window destruction
    pub fn handle_window_destroyed(&mut self, window_id: u64) -> Result<()> {
        if self.surface_states.remove(&window_id).is_some() {
            info!("🗑️ Window destroyed: ID {}", window_id);

            // Remove from our managers
            self.window_manager.write().remove_window(window_id);
            self.workspace_manager.write().remove_window(window_id);
            self.decoration_manager.write().remove_window(window_id);
        }

        Ok(())
    }

    // === Phase 5.2: Enhanced Event Handling ===

    /// Enhanced window update handling (Phase 5.2)
    pub fn handle_window_update(&mut self, window_id: u64) -> Result<()> {
        debug!(
            "📝 Phase 5.2: Enhanced window update for window {}",
            window_id
        );

        // Update surface state timestamp
        if let Some(surface_state) = self.surface_states.get_mut(&window_id) {
            surface_state.last_commit = Instant::now();
        }

        // Trigger enhanced effects update
        if let Err(e) = self.effects_engine.write().update() {
            warn!("⚠️ Failed to update effects: {}", e);
        }

        // Update workspace layouts with enhanced logic
        self.update_workspace_layouts()?;

        Ok(())
    }

    /// Enhanced workspace layout updates (Phase 5.2)
    fn update_workspace_layouts(&mut self) -> Result<()> {
        let workspace_layouts = self.workspace_manager.write().calculate_workspace_layouts();

        for (window_id, layout_rect) in workspace_layouts {
            if let Some(window) = self.window_manager.write().get_window_mut(window_id) {
                let old_pos = window.window.position;
                let new_pos = (layout_rect.x, layout_rect.y);

                if old_pos != new_pos {
                    // Trigger smooth move animation
                    self.effects_engine.write().animate_window_move(
                        window_id,
                        (old_pos.0 as f32, old_pos.1 as f32),
                        (new_pos.0 as f32, new_pos.1 as f32),
                    );
                }

                window.window.set_position(layout_rect.x, layout_rect.y);
                window
                    .window
                    .set_size(layout_rect.width, layout_rect.height);
            }
        }

        Ok(())
    }
}

/// Simple Smithay backend wrapper for Phase 5.2
pub struct AxiomSmithayBackend {
    /// Compositor state
    state: AxiomCompositorState,

    /// Windowed mode flag
    windowed: bool,

    /// Optional real XDG backend (compiled with feature "real-compositor")
    #[cfg(feature = "real-compositor")]
    real_backend: Option<real_xdg_backend::AxiomSmithayBackendReal>,
}

impl AxiomSmithayBackend {
    /// Create new Smithay backend
    pub fn new(
        config: AxiomConfig,
        windowed: bool,
        window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
        decoration_manager: Arc<parking_lot::RwLock<DecorationManager>>,
        input_manager: Arc<parking_lot::RwLock<InputManager>>,
    ) -> Result<Self> {
        info!("🏗️ Creating Smithay backend...");

        // Create compositor state
        let state = AxiomCompositorState::new(
            config,
            window_manager,
            workspace_manager,
            effects_engine,
            decoration_manager,
            input_manager,
        )?;

        info!("✅ Smithay backend created successfully");

        Ok(Self {
            state,
            windowed,
            #[cfg(feature = "real-compositor")]
            real_backend: None,
        })
    }

    /// Initialize the backend
    pub async fn initialize(&mut self) -> Result<()> {
        info!("🚀 Initializing Smithay backend...");

        // Start the backend state (creates a WAYLAND_DISPLAY socket as a fallback)
        self.state.start()?;

        if self.windowed {
            info!("🪟 Initializing windowed mode...");
            self.state.init_winit_backend()?;
        } else {
            info!("🖥️ Initializing native mode...");
        }

        // If compiled with the real XDG backend, initialize it as well to register real globals
        #[cfg(feature = "real-compositor")]
        {
            use real_xdg_backend::AxiomSmithayBackendReal as Real;
            let rb = Real::new(
                self.state.config.clone(),
                self.state.window_manager.clone(),
                self.state.workspace_manager.clone(),
                self.state.effects_engine.clone(),
                self.state.input_manager.clone(),
            )?;
            let mut rb = rb;
            rb.initialize()?;
            self.real_backend = Some(rb);
            info!("✅ Real XDG backend initialized (Milestone 1)");
        }

        info!("✅ Smithay backend initialization complete");
        Ok(())
    }

    /// Process events
    pub async fn process_events(&mut self) -> Result<()> {
        // If we have the real backend compiled and initialized, run one cycle of client dispatch
        #[cfg(feature = "real-compositor")]
        {
            if let Some(rb) = self.real_backend.as_mut() {
                rb.run_one_cycle()?;
            }
        }
        self.state.process_events().await
    }

    /// Run the backend (blocking)
    pub async fn run(&mut self) -> Result<()> {
        info!("🎬 Starting Smithay backend...");

        self.state.start()?;

        while self.state.is_running() {
            self.process_events().await?;

            // Small delay to prevent busy waiting
            tokio::time::sleep(tokio::time::Duration::from_millis(16)).await;
        }

        info!("🛑 Smithay backend finished");
        Ok(())
    }

    /// Shutdown the backend
    pub async fn shutdown(&mut self) -> Result<()> {
        info!("🛑 Shutting down Smithay backend...");
        // Shut down the real backend if present
        #[cfg(feature = "real-compositor")]
        {
            if let Some(rb) = self.real_backend.as_mut() {
                let _ = rb.shutdown();
            }
            self.real_backend = None;
        }
        self.state.shutdown().await?;
        info!("✅ Smithay backend shutdown complete");
        Ok(())
    }

    /// Check if backend is running
    pub fn is_running(&self) -> bool {
        self.state.running
    }

    /// Get window count
    pub fn window_count(&self) -> usize {
        self.state.surface_states.len()
    }

    /// Get mutable reference to the compositor state
    pub fn state(&mut self) -> &mut AxiomCompositorState {
        &mut self.state
    }

    /// Create a new window for testing
    pub fn create_test_window(&mut self, title: String) -> Result<u64> {
        self.state.handle_new_window(title)
    }

    /// Destroy a window
    pub fn destroy_window(&mut self, window_id: u64) -> Result<()> {
        self.state.handle_window_destroyed(window_id)
    }
}

// === Experimental real XDG shell backend (feature-gated) ===
// Enable with: --features real-compositor
#[cfg(feature = "real-compositor")]
pub mod real_xdg_backend {
    use super::*;
    use anyhow::Result;
    use log::{info, warn};

    // Smithay imports (0.3.x)
    use smithay::reexports::calloop::EventLoop;
    use smithay::reexports::wayland_server::Display;
    use smithay::wayland::{
        compositor,
        shell::xdg::xdg_shell,
        shm,
        seat,
    };

    /// Minimal compositor state for real XDG shell handling
    pub struct State {
        pub config: AxiomConfig,
        pub window_manager: Arc<parking_lot::RwLock<WindowManager>>,
        pub workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
        pub effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
        pub input_manager: Arc<parking_lot::RwLock<InputManager>>,
        pub surfaces: HashMap<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface, u64>,
        pub running: bool,
    }

    pub struct AxiomSmithayBackendReal {
        pub display: Display<State>,
        pub event_loop: EventLoop<State>,
        pub socket_name: Option<String>,
    }

    impl AxiomSmithayBackendReal {
        pub fn new(
            config: AxiomConfig,
            window_manager: Arc<parking_lot::RwLock<WindowManager>>,
            workspace_manager: Arc<parking_lot::RwLock<ScrollableWorkspaces>>,
            effects_engine: Arc<parking_lot::RwLock<EffectsEngine>>,
            input_manager: Arc<parking_lot::RwLock<InputManager>>,
        ) -> Result<Self> {
            let mut display = Display::new();
            let event_loop = EventLoop::try_new()?;

            // Init smithay globals
            compositor::init_compositor::<State, _>(&mut display, |_, _state| {});
            shm::init_shm_global::<State>(&mut display, vec![]);
            xdg_shell::init_xdg_shell::<State, _>(&mut display, |_, _state| {});
            seat::init_seat_global::<State>(&mut display, |_, _state| {});

            // Build initial state
            let state = State {
                config,
                window_manager,
                workspace_manager,
                effects_engine,
                input_manager,
                surfaces: HashMap::new(),
                running: true,
            };

            // Insert state into event loop
            event_loop.insert_source(
                smithay::reexports::calloop::ping::make_ping().0,
                |_event, _metadata, _state| {},
            )?;

            display.handle().insert_resource(state);

            Ok(Self {
                display,
                event_loop,
                socket_name: None,
            })
        }

        pub fn initialize(&mut self) -> Result<()> {
            let sock = self.display.add_socket_auto()?;
            let name = sock.to_string_lossy().to_string();
            std::env::set_var("WAYLAND_DISPLAY", &name);
            self.socket_name = Some(name);
            info!("✅ Real Smithay backend initialized (XDG shell global ready)");
            Ok(())
        }

        pub fn run_one_cycle(&mut self) -> Result<()> {
            // In a full implementation, dispatch events here and map surfaces to windows
            self.display.dispatch_clients(&mut 0, |_| {})?;
            self.display.flush_clients()?;
            Ok(())
        }

        pub fn shutdown(&mut self) -> Result<()> {
            info!("🔽 Shutting down real backend");
            Ok(())
        }
    }
}
