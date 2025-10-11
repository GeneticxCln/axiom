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
use log::{error, info, warn};
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

#[cfg(feature = "dmabuf-vulkan")]
mod dmabuf_vulkan;

#[cfg(all(feature = "smithay", feature = "wgpu-present"))]
fn start_output_control_server(tx: std::sync::mpsc::Sender<axiom::smithay::server::OutputOp>) {
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
            error!("failed to bind control socket {}: {}", sock_path, e);
            return;
        }
    };
    
    // Harden socket permissions to 0600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&sock_path, std::fs::Permissions::from_mode(0o600)) {
            error!("failed to set permissions on control socket {}: {}", sock_path, e);
        }
    }
    
    info!("axiom control socket listening at {}", sock_path);

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
                                                            axiom::smithay::server::OutputOp::Add(
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
                                                        axiom::smithay::server::OutputOp::Remove {
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
fn parse_outputs_spec(spec: &str) -> Option<Vec<axiom::smithay::server::OutputInit>> {
    // Format: "WIDTHxHEIGHT@SCALE+X,Y;WIDTHxHEIGHT@SCALE+X,Y;..."
    // Note: X,Y can be negative for multi-monitor topologies with outputs
    // positioned to the left or above the origin (e.g., "-1920,0" for a monitor left of primary).
    // OutputInit stores pos_x/pos_y as i32 to support this. Negative coordinates are preserved
    // through the Smithay server and used for layout calculations.
    // The presenter path clamps them to u32 (line 542) since GPU scissor rectangles require
    // non-negative coordinates in framebuffer space.
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
        out.push(axiom::smithay::server::OutputInit {
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

// Unified Smithay backend is provided by the library
// Use axiom::smithay instead of declaring it again here

#[cfg(not(all(feature = "smithay", feature = "wgpu-present")))]
use axiom::compositor::AxiomCompositor;
use axiom::config::AxiomConfig;

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
    #[arg(long, value_parser = ["auto", "vulkan", "gl"], default_value = "auto")]
    backend: String,

    /// Present mode override for on-screen rendering: auto, fifo, mailbox, immediate
    #[arg(long, value_parser = ["auto", "fifo", "mailbox", "immediate"], default_value = "auto")]
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
        use axiom::clipboard::ClipboardManager;
        use axiom::input::InputManager;
        use axiom::smithay::server::{CompositorServer, PresentEvent};
        use axiom::window::WindowManager;
        use axiom::workspace::ScrollableWorkspaces;

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
            // Initialize security manager
            let mut sec = axiom::security::SecurityManager::default();
            sec.init().expect("Failed to initialize security manager");
            let security = Arc::new(parking_lot::Mutex::new(sec));
            // Prefer Smithay libinput backend
            let input_rx = axiom::smithay::input_backend::init_libinput_backend()
                .or_else(axiom::smithay::server::CompositorServer::spawn_evdev_input_channel);
            // Parse outputs if provided
            let outputs_init = cli.outputs.as_ref().and_then(|s| parse_outputs_spec(s));
            // Set decoration policy env for server
            if config.window.force_client_side_decorations {
                std::env::set_var("AXIOM_FORCE_CSD", "1");
            } else {
                std::env::remove_var("AXIOM_FORCE_CSD");
            }
            let (outputs_tx, outputs_rx) =
                std::sync::mpsc::channel::<axiom::smithay::server::OutputOp>();
            // Start control socket server on main thread
            start_output_control_server(outputs_tx);
            if cli.split_frame_callbacks {
                std::env::set_var("AXIOM_SPLIT_FRAME_CALLBACKS", "1");
            }
            if cli.debug_outputs {
                std::env::set_var("AXIOM_DEBUG_OUTPUTS", "1");
            }
            // Create decoration manager
            let deco = Arc::new(RwLock::new(axiom::decoration::DecorationManager::new(
                &config.window,
            )));
            let server = CompositorServer::new(
                wm,
                ws,
                im,
                clip,
                deco,
                security,
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
            std::sync::mpsc::channel::<axiom::smithay::server::OutputOp>();
        start_output_control_server(outputs_tx.clone());
        let (present_tx, present_rx) = std::sync::mpsc::channel::<PresentEvent>();
        let (size_tx, size_rx) = std::sync::mpsc::channel::<axiom::smithay::server::SizeUpdate>();
        let (redraw_tx, redraw_rx) = std::sync::mpsc::channel::<()>();
        // Start IPC server in a background Tokio runtime and keep it alive
        let ipc_server = std::sync::Arc::new(std::sync::Mutex::new(axiom::ipc::AxiomIPCServer::new()));
        axiom::ipc::AxiomIPCServer::set_config_snapshot(config.clone());
        let ipc_server_for_thread = ipc_server.clone();
        std::thread::spawn(move || {
            match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(rt) => {
                    rt.block_on(async move {
                        if let Ok(mut guard) = ipc_server_for_thread.lock() {
                            if let Err(e) = guard.start().await {
                                error!("Failed to start IPC server: {}", e);
                            }
                        }
                        loop { tokio::time::sleep(std::time::Duration::from_secs(3600)).await; }
                    });
                }
                Err(e) => error!("Failed to create Tokio runtime for IPC server: {}", e),
            }
        });
        std::thread::spawn(move || {
                        let wm = match WindowManager::new(&cfg_clone.window) {
                Ok(w) => Arc::new(RwLock::new(w)),
                Err(e) => { error!("Failed to initialize WindowManager: {}", e); return; }
            };
            let ws = match ScrollableWorkspaces::new(&cfg_clone.workspace) {
                Ok(w) => Arc::new(RwLock::new(w)),
                Err(e) => { error!("Failed to initialize ScrollableWorkspaces: {}", e); return; }
            };
            let im = match InputManager::new(&cfg_clone.input, &cfg_clone.bindings) {
                Ok(m) => Arc::new(RwLock::new(m)),
                Err(e) => { error!("Failed to initialize InputManager: {}", e); return; }
            };
            let clip = Arc::new(RwLock::new(ClipboardManager::new()));
            // Initialize security manager
            let mut sec = axiom::security::SecurityManager::default();
            sec.init().expect("Failed to initialize security manager");
            let security = Arc::new(parking_lot::Mutex::new(sec));
            let input_rx = axiom::smithay::input_backend::init_libinput_backend()
                .or_else(axiom::smithay::server::CompositorServer::spawn_evdev_input_channel);
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
            let deco = Arc::new(RwLock::new(axiom::decoration::DecorationManager::new(
                &cfg_clone.window,
            )));
            let server = CompositorServer::new(
                wm,
                ws,
                im,
                clip,
                deco,
                security,
                Some(present_rx),
                Some(size_rx),
                Some(redraw_tx),
                input_rx,
                false,
                selected_backends,
                outputs_init,
                Some(outputs_rx),
            );
            let server = match server {
                Ok(s) => s,
                Err(e) => { error!("Failed to create CompositorServer: {}", e); return; }
            };
            let _ = server.run();
        });

        // Create window and wgpu surface on the main thread
        let event_loop = EventLoop::new()?;
        let window = Arc::new(winit::window::WindowBuilder::new()
            .with_title("Axiom Compositor")
            .build(&event_loop)?);

        // Create wgpu surface
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: selected_backends,
            ..Default::default()
        });
        // Create surface for the window
        let surface = instance.create_surface(window.clone())?;
        let mut window_size = window.inner_size();
        let _window_id = window.id();

        // Apply present mode override via environment for renderer selection
        let pm = cli.present_mode.to_lowercase();
        if pm == "fifo" || pm == "mailbox" || pm == "immediate" || pm == "auto" {
            std::env::set_var("AXIOM_PRESENT_MODE", pm);
        }
        // Create renderer with the same instance as the surface
        let mut renderer = pollster::block_on(axiom::renderer::AxiomRenderer::new_with_instance(
            &instance,
            Some(&surface),
            window_size.width,
            window_size.height,
        ))?;

        let mut last_frame_time_inst = std::time::Instant::now();
        // Run the event loop on the main thread
        return Ok(event_loop.run(move |event, elwt| {
            match event {
                Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => elwt.exit(),
                Event::WindowEvent { event: WindowEvent::Resized(new_size), .. } => {
                    if new_size.width > 0 && new_size.height > 0 {
                        match renderer.resize(Some(&surface), new_size.width, new_size.height) {
                            Ok(()) => {
                                window_size = new_size;
                                let _ = size_tx.send(axiom::smithay::server::SizeUpdate { width: new_size.width, height: new_size.height, scale: 1, name: None, model: None });
                            }
                            Err(e) => {
                                error!("Failed to resize renderer: {}", e);
                                // Surface errors (Lost, Outdated) require reconfiguration; skip frame and retry next cycle
                            }
                        }
                        elwt.set_control_flow(winit::event_loop::ControlFlow::Poll);
                    }
                }
                Event::AboutToWait => {
                    // If the compositor requested a redraw, request a window redraw
                    let mut needs_redraw = false;
                    while redraw_rx.try_recv().is_ok() { needs_redraw = true; }
                    if needs_redraw {
                        window.request_redraw();
                    }
                    elwt.set_control_flow(winit::event_loop::ControlFlow::Poll);
                }
                Event::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                    // Sync from shared render state and draw
                    renderer.sync_from_shared();
                    
                    // Only render and present if we have content to show or this is the initial frame
                    let has_content = renderer.window_count() > 0;
                    
                    if renderer.can_present() {
                        let frame_result = surface.get_current_texture();
                        match frame_result {
                            Err(wgpu::SurfaceError::Lost) => {
                                // Surface lost (e.g., DPMS suspend); reconfigure and skip frame
                                info!("Surface lost; reconfiguring");
                                let _ = renderer.resize(Some(&surface), window_size.width, window_size.height);
                            }
                            Err(wgpu::SurfaceError::Outdated) => {
                                // Surface outdated (e.g., resize race); reconfigure
                                info!("Surface outdated; reconfiguring");
                                let _ = renderer.resize(Some(&surface), window_size.width, window_size.height);
                            }
                            Err(wgpu::SurfaceError::Timeout) => {
                                warn!("Surface timeout; skipping frame");
                            }
                            Err(wgpu::SurfaceError::OutOfMemory) => {
                                error!("Surface out of memory; cannot recover");
                                elwt.exit();
                            }
                            Ok(frame) => {
                            // Compute output rectangles from the CLI topology if provided
                            // WHY: Negative coordinates from multi-monitor topology are clamped to 0
                            // because wgpu scissor rectangles operate in framebuffer space (u32 only).
                            // For example, an output at (-1920, 0, 1920, 1080) becomes (0, 0, 1920, 1080)
                            // in the presenter window's coordinate space.
                            // 
                            // CORRECTNESS: This is safe because:
                            // 1. The Smithay server maintains full i32 coordinate space for layout
                            // 2. The presenter window shows a single viewport into that space
                            // 3. Window positions in the shared render state are already transformed
                            //    by Smithay to viewport-relative coordinates before reaching the renderer
                            // 4. Clamping only affects the debug overlay scissor calculation (line 1442-1503)
                            let outputs_rects: Vec<(u32,u32,u32,u32)> = outputs_init_main
                                .as_ref()
                                .map(|outs| {
                                    outs.iter()
                                        .map(|o| (o.pos_x.max(0) as u32, o.pos_y.max(0) as u32, o.width.max(0) as u32, o.height.max(0) as u32))
                                        .collect()
                                })
                                .unwrap_or_else(|| vec![(0, 0, window_size.width, window_size.height)]);

                            let debug_overlay = std::env::var("AXIOM_DEBUG_OUTPUTS").ok().map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
                            
                                // Only render if we have windows, otherwise just clear and present once
                                if has_content {
                                    if let Err(e) = renderer.render_to_surface_with_outputs(&surface, &frame, &outputs_rects, debug_overlay) {
                                        error!("render error: {}", e);
                                    }
                                    frame.present();
                                
                                    // Send per-output presentation feedback timing (one event per logical output)
                                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                                    let tv_sec = now.as_secs();
                                    let tv_nsec = now.subsec_nanos();
                                    let tv_sec_hi: u32 = (tv_sec >> 32) as u32;
                                    let tv_sec_lo: u32 = (tv_sec & 0xFFFF_FFFF) as u32;
                                    // Query monitor refresh if available
                                    let refresh_ns: u32 = 16_666_666; // Default to 60Hz
                                    // Flags: vsync + hw clock
                                    let flags: u32 = (wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::Vsync | wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::HwClock).bits();
                                    let outputs_count = outputs_init_main.as_ref().map(|v| v.len()).unwrap_or(1);
                                    for idx in 0..outputs_count {
                                        let _ = present_tx.send(PresentEvent { tv_sec_hi, tv_sec_lo, tv_nsec, refresh_ns, flags, output_idx: Some(idx) });
                                    }

                                    // Broadcast IPC performance metrics (rate-limited inside IPC)
                                    if let Ok(mut guard) = ipc_server.lock() {
                                        let now_inst = std::time::Instant::now();
                                        let frame_time_ms = (now_inst.duration_since(last_frame_time_inst).as_secs_f32() * 1000.0).max(0.0);
                                        last_frame_time_inst = now_inst;
                                        let active_windows = renderer.window_count() as u32;
                                        let current_workspace = 0; // server thread owns workspace index
                                        guard.maybe_broadcast_performance_metrics(frame_time_ms, active_windows, current_workspace);
                                    }
                                } else {
                                    // No windows yet - just drop the frame without presenting to avoid wgpu errors
                                    drop(frame);
                                }
                            }
                        }
                    } else {
                        // Headless fallback: no on-screen presentation
                        if has_content {
                            let _ = renderer.render();
                        }
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
