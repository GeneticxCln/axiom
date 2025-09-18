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
#[cfg(all(feature = "smithay", feature = "wgpu-present"))]
use parking_lot::RwLock;
#[cfg(all(feature = "smithay", feature = "wgpu-present"))]
use std::sync::Arc;
#[cfg(all(feature = "smithay", feature = "wgpu-present"))]
use winit::event::{Event, WindowEvent};
#[cfg(all(feature = "smithay", feature = "wgpu-present"))]
use winit::event_loop::EventLoop;

mod clipboard;
mod compositor;
mod config;
mod decoration;
mod demo_phase4_effects;
mod demo_phase6_minimal;
mod demo_phase6_working;
mod demo_workspace;
mod effects;
mod input;
mod ipc;
mod renderer;
mod window;
mod workspace;
mod xwayland;

#[cfg(all(feature = "smithay", feature = "wgpu-present"))]
fn start_output_control_server(tx: std::sync::mpsc::Sender<crate::smithay::server::OutputOp>) {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::thread;

    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let sock_path = format!("{}/axiom-control-{}.sock", runtime_dir, std::process::id());
    // Best-effort cleanup
    let _ = std::fs::remove_file(&sock_path);

    let listener = match UnixListener::bind(&sock_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("failed to bind control socket {}: {}", sock_path, e);
            return;
        }
    };
    eprintln!("axiom control socket listening at {}", sock_path);

    thread::spawn(move || {
        for stream_res in listener.incoming() {
            match stream_res {
                Ok(stream) => {
                    let mut reader = BufReader::new(stream);
                    let mut buf = String::new();
                    loop {
                        buf.clear();
                        match reader.read_line(&mut buf) {
                            Ok(0) => break,
                            Ok(_) => {
                                let line = buf.trim();
                                if line.is_empty() {
                                    continue;
                                }
                                // Commands:
                                // add WIDTHxHEIGHT@SCALE+X,Y
                                // remove INDEX
                                let mut parts = line.split_whitespace();
                                if let Some(cmd) = parts.next() {
                                    match cmd {
                                        "add" => {
                                            if let Some(spec) = parts.next() {
                                                if let Some(vec) = parse_outputs_spec(spec) {
                                                    for init in vec {
                                                        let _ = tx.send(
                                                            crate::smithay::server::OutputOp::Add(
                                                                init,
                                                            ),
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                        "remove" => {
                                            if let Some(idx_s) = parts.next() {
                                                if let Ok(idx) = idx_s.parse::<usize>() {
                                                    let _ = tx.send(
                                                        crate::smithay::server::OutputOp::Remove {
                                                            index: idx,
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });
}

#[cfg(all(feature = "smithay", feature = "wgpu-present"))]
fn parse_outputs_spec(spec: &str) -> Option<Vec<crate::smithay::server::OutputInit>> {
    // Format: "WIDTHxHEIGHT@SCALE+X,Y;WIDTHxHEIGHT@SCALE+X,Y;..."
    let mut out = Vec::new();
    for chunk in spec.split(';') {
        let s = chunk.trim();
        if s.is_empty() {
            continue;
        }
        // Split name/model optional prefix? Keep it minimal for now.
        // Parse WIDTHxHEIGHT@SCALE+X,Y
let part = s;
        // WIDTHxHEIGHT
        let (wh, rest1) = match part.split_once('@') {
            Some(t) => t,
            None => (part, "1+0,0"),
        };
        let (w_str, h_str) = match wh.split_once('x') {
            Some(t) => t,
            None => continue,
        };
        let width: i32 = w_str.parse().ok()?;
        let height: i32 = h_str.parse().ok()?;
        // SCALE+X,Y
        let (scale_str, xy_str) = match rest1.split_once('+') {
            Some(t) => t,
            None => (rest1, "0,0"),
        };
        let scale: i32 = scale_str.parse().unwrap_or(1).max(1);
        let (x_str, y_str) = xy_str.split_once(',').unwrap_or(("0", "0"));
        let pos_x: i32 = x_str.parse().unwrap_or(0);
        let pos_y: i32 = y_str.parse().unwrap_or(0);
        out.push(crate::smithay::server::OutputInit {
            width,
            height,
            scale,
            pos_x,
            pos_y,
            name: None,
            model: None,
            refresh_mhz: 60000,
        });
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

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

    /// Define outputs topology, e.g. "1920x1080@1+0,0;1280x1024@1+1920,0"
    #[arg(long)]
    outputs: Option<String>,

    /// Split frame callbacks across all outputs overlapped by a surface
    #[arg(long, default_value_t = false)]
    split_frame_callbacks: bool,

    /// Show debug overlay for output regions
    #[arg(long, default_value_t = false)]
    debug_outputs: bool,

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

    /// Present mode override for on-screen rendering: auto, fifo, mailbox, immediate
    #[arg(long, default_value = "auto")]
    present_mode: String,

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
        use crate::clipboard::ClipboardManager;
        use crate::input::InputManager;
        use crate::smithay::server::{CompositorServer, PresentEvent};
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
            let im = Arc::new(RwLock::new(InputManager::new(
                &config.input,
                &config.bindings,
            )?));
            let clip = Arc::new(RwLock::new(ClipboardManager::new()));
            // Prefer Smithay libinput backend
            let input_rx = crate::smithay::input_backend::init_libinput_backend()
                .or_else(crate::smithay::server::CompositorServer::spawn_evdev_input_channel);
            // Parse outputs if provided
            let outputs_init = cli.outputs.as_ref().and_then(|s| parse_outputs_spec(s));
            // Set decoration policy env for server
            if config.window.force_client_side_decorations {
                std::env::set_var("AXIOM_FORCE_CSD", "1");
            } else {
                std::env::remove_var("AXIOM_FORCE_CSD");
            }
            let (outputs_tx, outputs_rx) =
                std::sync::mpsc::channel::<crate::smithay::server::OutputOp>();
            // Start control socket server on main thread
            start_output_control_server(outputs_tx);
            if cli.split_frame_callbacks {
                std::env::set_var("AXIOM_SPLIT_FRAME_CALLBACKS", "1");
            }
            if cli.debug_outputs {
                std::env::set_var("AXIOM_DEBUG_OUTPUTS", "1");
            }
            // Create decoration manager
            let deco = Arc::new(RwLock::new(crate::decoration::DecorationManager::new(&config.window)));
            let server = CompositorServer::new(
                wm,
                ws,
                im,
                clip,
                deco,
                None,
                None,
                None,
                input_rx,
                true,
                selected_backends,
                outputs_init,
                Some(outputs_rx),
            )?; // spawn headless renderer
            return server.run().map(|_| ());
        }

        // Start Smithay server without headless renderer (we'll present on-screen)
        let cfg_clone = config.clone();
        let outputs_init = cli.outputs.as_ref().and_then(|s| parse_outputs_spec(s));
        // Keep a local copy for the presenter thread
        let outputs_init_main = outputs_init.clone();
        // Create runtime dynamic outputs channel now so we can run control server on main thread
        let (outputs_tx, outputs_rx) =
            std::sync::mpsc::channel::<crate::smithay::server::OutputOp>();
        start_output_control_server(outputs_tx.clone());
        let (present_tx, present_rx) = std::sync::mpsc::channel::<PresentEvent>();
        let (size_tx, size_rx) = std::sync::mpsc::channel::<crate::smithay::server::SizeUpdate>();
        let (redraw_tx, redraw_rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            let _ = env_logger::try_init();
            let wm = Arc::new(RwLock::new(
                WindowManager::new(&cfg_clone.window).expect("wm"),
            ));
            let ws = Arc::new(RwLock::new(
                ScrollableWorkspaces::new(&cfg_clone.workspace).expect("ws"),
            ));
            let im = Arc::new(RwLock::new(
                InputManager::new(&cfg_clone.input, &cfg_clone.bindings).expect("im"),
            ));
            let clip = Arc::new(RwLock::new(ClipboardManager::new()));
            let input_rx = crate::smithay::input_backend::init_libinput_backend()
                .or_else(crate::smithay::server::CompositorServer::spawn_evdev_input_channel);
            if cfg_clone.window.force_client_side_decorations {
                std::env::set_var("AXIOM_FORCE_CSD", "1");
            } else {
                std::env::remove_var("AXIOM_FORCE_CSD");
            }
            if cli.split_frame_callbacks {
                std::env::set_var("AXIOM_SPLIT_FRAME_CALLBACKS", "1");
            }
            if cli.debug_outputs {
                std::env::set_var("AXIOM_DEBUG_OUTPUTS", "1");
            }
            let deco = Arc::new(RwLock::new(crate::decoration::DecorationManager::new(&cfg_clone.window)));
            let server = CompositorServer::new(
                wm,
                ws,
                im,
                clip,
                deco,
                Some(present_rx),
                Some(size_rx),
                Some(redraw_tx),
                input_rx,
                false,
                selected_backends,
                outputs_init,
                Some(outputs_rx),
            )
            .expect("server");
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

        // Apply present mode override via environment for renderer selection
        let pm = cli.present_mode.to_lowercase();
        if pm == "fifo" || pm == "mailbox" || pm == "immediate" || pm == "auto" {
            std::env::set_var("AXIOM_PRESENT_MODE", pm);
        }
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
                        let _ = size_tx.send(crate::smithay::server::SizeUpdate { width: new_size.width, height: new_size.height, scale: window.scale_factor() as i32, name: window.current_monitor().and_then(|m| m.name()), model: None });
                        window.request_redraw();
                    }
                }
                Event::AboutToWait => {
                    // If the compositor requested a redraw, drain once and redraw
                    while redraw_rx.try_recv().is_ok() { /* drain */ }
                    window.request_redraw();
                }
                Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                    // Sync from shared render state and draw
                    renderer.sync_from_shared();
                    if renderer.can_present() {
                        if let Ok(frame) = surface.get_current_texture() {
                            // Compute output rectangles from the CLI topology if provided
                            let outputs_rects: Vec<(u32,u32,u32,u32)> = outputs_init_main
                                .as_ref()
                                .map(|outs| {
                                    outs.iter()
                                        .map(|o| (o.pos_x.max(0) as u32, o.pos_y.max(0) as u32, o.width.max(0) as u32, o.height.max(0) as u32))
                                        .collect()
                                })
                                .unwrap_or_else(|| vec![(0, 0, size.width, size.height)]);

                            let debug_overlay = std::env::var("AXIOM_DEBUG_OUTPUTS").ok().map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
                            if let Err(e) = renderer.render_to_surface_with_outputs(&surface, &frame, &outputs_rects, debug_overlay) {
                                eprintln!("render error: {}", e);
                            }
                            frame.present();
                            // Send per-output presentation feedback timing (one event per logical output)
                            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                            let tv_sec = now.as_secs();
                            let tv_nsec = now.subsec_nanos();
                            let tv_sec_hi: u32 = (tv_sec >> 32) as u32;
                            let tv_sec_lo: u32 = (tv_sec & 0xFFFF_FFFF) as u32;
                            // Query monitor refresh if available
                            let refresh_ns: u32 = window.current_monitor()
                                .and_then(|m| m.refresh_rate_millihertz())
                                .map(|mhz| {
                                    // ns per frame = 1e9 / (mhz/1000) = 1e12 / mhz
                                    let ns = 1_000_000_000_000u64 / (mhz as u64);
                                    ns.clamp(8_000_000, 33_333_333) as u32 // clamp between 120Hz and 30Hz typical bounds
                                })
                                .unwrap_or(16_666_666);
                            // Flags: vsync + hw clock
                            let flags: u32 = (wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::Vsync | wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::HwClock).bits();
                            let outputs_count = outputs_init_main.as_ref().map(|v| v.len()).unwrap_or(1);
                            for idx in 0..outputs_count {
                                let _ = present_tx.send(PresentEvent { tv_sec_hi, tv_sec_lo, tv_nsec, refresh_ns, flags, output_idx: Some(idx) });
                            }
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

        let compositor = AxiomCompositor::new(config.clone(), cli.windowed).await?;

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
        let cli = Cli::try_parse_from(["axiom"]).unwrap();
        assert!(!cli.debug);
        assert!(!cli.windowed);
        assert!(!cli.no_effects);
    }

    #[test]
    fn test_cli_flags() {
        let cli = Cli::try_parse_from(["axiom", "--debug", "--windowed", "--no-effects"]).unwrap();
        assert!(cli.debug);
        assert!(cli.windowed);
        assert!(cli.no_effects);
    }
}
