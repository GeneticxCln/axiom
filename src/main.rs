//! # Axiom - Hybrid Wayland Compositor
//!
//! The first Wayland compositor combining niri's scrollable workspace innovation
//! with Hyprland's visual effects system.
//!
//! ## Architecture Overview
//!
//! Axiom is built on a modular architecture:
//! - `compositor`: Core compositor logic and event loop
//! - `workspace`: Scrollable workspace management (niri-inspired)
//! - `effects`: Visual effects engine (Hyprland-inspired)
//! - `window`: Window management and layout algorithms
//! - `input`: Keyboard, mouse, and gesture input handling
//! - `config`: Configuration parsing and management
//! - `xwayland`: X11 compatibility layer


use anyhow::Result;
use clap::Parser;
use log::{error, info};
#[cfg(feature = "smithay")]
use parking_lot::RwLock;
#[cfg(feature = "smithay")]
use std::sync::Arc;
#[cfg(all(feature = "smithay", feature = "wgpu-present"))]
use winit::event::{Event, WindowEvent};
#[cfg(all(feature = "smithay", feature = "wgpu-present"))]
use winit::event_loop::EventLoop;
#[cfg(all(feature = "smithay", feature = "wgpu-present"))]
use pollster;

mod compositor;
mod decoration;
mod config;
mod demo_phase4_effects;
mod demo_phase6_minimal;
mod demo_phase6_working;
mod demo_workspace;
mod effects;
mod input;
mod ipc;
mod window;
mod workspace;
mod xwayland;
mod renderer;

// Unified Smithay backend
#[cfg(feature = "smithay")]
pub mod smithay;

#[cfg(not(all(feature = "smithay", feature = "wgpu-present")))]
use compositor::AxiomCompositor;
use config::AxiomConfig;

#[derive(Parser)]
#[command(name = "axiom")]
#[command(
    about = "A hybrid Wayland compositor combining scrollable workspaces with visual effects"
)]
#[command(version)]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "~/.config/axiom/axiom.toml")]
    config: String,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,

    /// Run in windowed mode (for development)
    #[arg(short, long)]
    windowed: bool,

    /// Force headless mode (no on-screen window; headless rendering only)
    #[arg(long, default_value_t = false)]
    headless: bool,

    /// Select GPU backend: auto, vulkan, gl
    #[arg(long, default_value = "auto")]
    backend: String,

    /// Disable visual effects (performance mode)
    #[arg(long)]
    no_effects: bool,

    /// Run scrollable workspace demo (Phase 3)
    #[arg(long)]
    demo: bool,

    /// Run visual effects demo (Phase 4)
    #[arg(long)]
    effects_demo: bool,

    /// Run Phase 6.2 Smithay backend demo with protocol simulation
    #[arg(long)]
    phase6_2_demo: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    if cli.debug {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }

    info!("🚀 Starting Axiom - Hybrid Wayland Compositor");
    info!("📄 Version: {}", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = match AxiomConfig::load(&cli.config) {
        Ok(config) => {
            info!("✅ Configuration loaded from: {}", cli.config);
            config
        }
        Err(e) => {
            error!("❌ Failed to load configuration: {}", e);
            info!("📝 Using default configuration");
            AxiomConfig::default()
        }
    };

    // Override config with CLI flags
    let mut config = config;
    if cli.no_effects {
        config.effects.enabled = false;
        info!("🚫 Visual effects disabled via CLI flag");
    }

    // === On-screen or headless presenter path (Smithay available) ===
    #[cfg(all(feature = "smithay", feature = "wgpu-present"))]
    {
        use crate::input::InputManager;
        use crate::smithay::server::CompositorServer;
        use crate::window::WindowManager;
        use crate::workspace::ScrollableWorkspaces;

        // Select backends based on CLI
        let selected_backends = match cli.backend.as_str() {
            "vulkan" => wgpu::Backends::VULKAN,
            "gl" => wgpu::Backends::GL,
            _ => wgpu::Backends::all(),
        };
        info!("🎛️ WGPU backend selection: {}", cli.backend);

        // If headless, run Smithay server in this thread with headless GPU loop
        if cli.headless {
            info!("🖥️ Headless mode enabled - no on-screen window will be created");
            let wm = Arc::new(RwLock::new(WindowManager::new(&config.window)?));
            let ws = Arc::new(RwLock::new(ScrollableWorkspaces::new(&config.workspace)?));
            let im = Arc::new(RwLock::new(InputManager::new(&config.input, &config.bindings)?));
            let server = CompositorServer::new(wm, ws, im, true, selected_backends)?; // spawn headless renderer
            return server.run().map(|_| ());
        }

        // Start Smithay server without headless renderer (we'll present on-screen)
        let cfg_clone = config.clone();
        std::thread::spawn(move || {
            let _ = env_logger::try_init();
            let wm = Arc::new(RwLock::new(WindowManager::new(&cfg_clone.window).expect("wm")));
            let ws = Arc::new(RwLock::new(ScrollableWorkspaces::new(&cfg_clone.workspace).expect("ws")));
            let im = Arc::new(RwLock::new(InputManager::new(&cfg_clone.input, &cfg_clone.bindings).expect("im")));
            let server = CompositorServer::new(wm, ws, im, false, selected_backends).expect("server");
            let _ = server.run();
        });

        // Create window and wgpu surface on the main thread
        let event_loop = EventLoop::new()?;
        let window = winit::window::WindowBuilder::new()
            .with_title("Axiom Compositor")
            .build(&event_loop)?;

        // Create wgpu surface
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: selected_backends,
            ..Default::default()
        });
        // Create surface for the window
        let surface = instance.create_surface(&window)?;
        let size = window.inner_size();

        // Create renderer with the same instance as the surface
        let mut renderer = pollster::block_on(crate::renderer::AxiomRenderer::new_with_instance(
            &instance,
            Some(&surface),
            size.width,
            size.height,
        ))?;

        // Run the event loop on the main thread
        return Ok(event_loop.run(|event, elwt| {
            match event {
                Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => elwt.exit(),
                Event::WindowEvent { event: WindowEvent::Resized(new_size), .. } => {
                    if new_size.width > 0 && new_size.height > 0 {
                        renderer = pollster::block_on(crate::renderer::AxiomRenderer::new_with_instance(
                            &instance,
                            Some(&surface),
                            new_size.width,
                            new_size.height,
                        ))
                        .expect("recreate renderer");
                        window.request_redraw();
                    }
                }
                Event::AboutToWait => {
                    window.request_redraw();
                }
                Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                    // Sync from shared render state and draw
                    renderer.sync_from_shared();
                    if renderer.can_present() {
                        if let Ok(frame) = surface.get_current_texture() {
                            if let Err(e) = renderer.render_to_surface(&surface, &frame) {
                                eprintln!("render error: {}", e);
                            }
                            frame.present();
                        }
                    } else {
                        // Headless fallback: no on-screen presentation
                        let _ = renderer.render();
                    }
                }
                _ => {}
            }
        })?);
    }

    // === Fallback path (no on-screen presenter): original async compositor ===
    #[cfg(not(all(feature = "smithay", feature = "wgpu-present")))]
    {
        // Initialize and run compositor
        info!("🏗️  Initializing Axiom compositor...");

        let mut compositor = AxiomCompositor::new(config.clone(), cli.windowed).await?;

        info!("✨ Axiom is ready! Where productivity meets beauty.");

        // Run demos if requested
    #[cfg(feature = "demo")]
    if cli.demo {
            info!("🎭 Running Phase 3 scrollable workspace demo...");
            demo_workspace::run_comprehensive_test(&mut compositor).await?;
            info!("🎆 Phase 3 demo completed!");
        }

    #[cfg(feature = "demo")]
    if cli.effects_demo {
            info!("🎨 Running Phase 4 visual effects demo...");
            demo_phase4_effects::display_effects_capabilities(&compositor);
            demo_phase4_effects::run_phase4_effects_demo(&mut compositor).await?;
            info!("🎆 Phase 4 effects demo completed!");
        }

        if cli.phase6_2_demo {
            info!("🌊 Phase 6.2 demo was removed during backend unification");
        }

    #[cfg(feature = "demo")]
    if cli.demo || cli.effects_demo || cli.phase6_2_demo {
            info!("🎆 All demos completed! Continuing with normal compositor operation...");
        }

        // Main event loop
        compositor.run().await?;

        info!("👋 Axiom compositor shutting down");
        return Ok(());
    }

}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_parsing() {
        // Test basic CLI parsing
        let cli = Cli::try_parse_from(&["axiom"]).unwrap();
        assert!(!cli.debug);
        assert!(!cli.windowed);
        assert!(!cli.no_effects);
    }

    #[test]
    fn test_cli_flags() {
        let cli = Cli::try_parse_from(&["axiom", "--debug", "--windowed", "--no-effects"]).unwrap();
        assert!(cli.debug);
        assert!(cli.windowed);
        assert!(cli.no_effects);
    }
}
