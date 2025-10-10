//! On-screen presenter (winit + wgpu) for Axiom
//!
//! This binary runs only when built with features: `smithay` and `wgpu-present`.
//! It creates a winit window and wgpu surface, spawns the unified Smithay
//! compositor server in a background thread, and renders frames to the surface.

use anyhow::Result;
use clap::Parser;
use env_logger;
use log::{error, info};
use parking_lot::RwLock;
use std::sync::mpsc;
use std::sync::Arc;
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;

// Axiom imports
use axiom::clipboard::ClipboardManager;
use axiom::decoration::DecorationManager;
use axiom::renderer::AxiomRenderer;
use axiom::{AxiomConfig, InputManager, ScrollableWorkspaces, WindowManager};

// Smithay unified server types
use axiom::smithay::server::{CompositorServer, OutputInit, OutputOp, PresentEvent, SizeUpdate};

/// Presenter CLI
#[derive(Parser, Debug)]
#[command(name = "run_present_winit")]
#[command(about = "Run Axiom presenter (winit + wgpu) with unified Smithay server")]
struct Cli {
    /// Outputs topology: "WIDTHxHEIGHT@SCALE+X,Y;..."
    #[arg(long)]
    outputs: Option<String>,

    /// GPU backend: auto|vulkan|gl
    #[arg(long, default_value = "auto")]
    backend: String,

    /// Present mode: auto|fifo|mailbox|immediate (honored if supported)
    #[arg(long, default_value = "auto")]
    present_mode: String,

    /// Split frame callbacks across overlapped outputs (server behavior)
    #[arg(long, default_value_t = false)]
    split_frame_callbacks: bool,

    /// Draw output region debug overlay
    #[arg(long, default_value_t = false)]
    debug_outputs: bool,
}

fn start_output_control_server(tx: mpsc::Sender<OutputOp>) {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::thread;

    let runtime_dir = std::env::var("XXDGRUNTIME_DIR").ok().unwrap_or_else(|| {
        // typo-guard: fall back to correct var if user typo'd
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string())
    });
    let sock_path = format!("{}/axiom-control-{}.sock", runtime_dir, std::process::id());
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
                                // add WIDTHxHEIGHT@SCALE+X,Y[;...]
                                // remove INDEX
                                let mut parts = line.split_whitespace();
                                if let Some(cmd) = parts.next() {
                                    match cmd {
                                        "add" => {
                                            if let Some(spec) = parts.next() {
                                                if let Some(vec) = parse_outputs_spec(spec) {
                                                    for init in vec {
                                                        let _ = tx.send(OutputOp::Add(init));
                                                    }
                                                }
                                            }
                                        }
                                        "remove" => {
                                            if let Some(idx_s) = parts.next() {
                                                if let Ok(idx) = idx_s.parse::<usize>() {
                                                    let _ =
                                                        tx.send(OutputOp::Remove { index: idx });
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

fn parse_outputs_spec(spec: &str) -> Option<Vec<OutputInit>> {
    // Format: "WIDTHxHEIGHT@SCALE+X,Y;WIDTHxHEIGHT@SCALE+X,Y;..."
    let mut out = Vec::new();
    for chunk in spec.split(';') {
        let s = chunk.trim();
        if s.is_empty() {
            continue;
        }
        let part = s;
        let (wh, rest1) = match part.split_once('@') {
            Some(t) => t,
            None => (part, "1+0,0"),
        };
        let (w_str, h_str) = match wh.split_once('x') {
            Some(t) => t,
            None => continue,
        };
        let width: i32 = match w_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let height: i32 = match h_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let (scale_str, xy_str) = match rest1.split_once('+') {
            Some(t) => t,
            None => (rest1, "0,0"),
        };
        let scale: i32 = scale_str.parse().unwrap_or(1).max(1);
        let (x_str, y_str) = xy_str.split_once(',').unwrap_or(("0", "0"));
        let pos_x: i32 = x_str.parse().unwrap_or(0);
        let pos_y: i32 = y_str.parse().unwrap_or(0);
        out.push(OutputInit {
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

fn backends_from_str(s: &str) -> wgpu::Backends {
    match s.to_lowercase().as_str() {
        "vulkan" => wgpu::Backends::VULKAN,
        "gl" => wgpu::Backends::GL,
        _ => wgpu::Backends::all(),
    }
}

fn main() -> Result<()> {
    // Logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli = Cli::parse();
    info!("🎛️ Presenter backend: {}", cli.backend);

    // Env toggles for server behavior
    if cli.split_frame_callbacks {
        std::env::set_var("AXIOM_SPLIT_FRAME_CALLBACKS", "1");
    }
    if cli.debug_outputs {
        std::env::set_var("AXIOM_DEBUG_OUTPUTS", "1");
    }

    // Select WGPU backends
    let selected_backends = backends_from_str(&cli.backend);

    // Parse desired outputs topology if provided (CLI)
    let outputs_init_cli = cli.outputs.as_ref().and_then(|s| parse_outputs_spec(s));

    // Create channels for server coordination
    let (present_tx, present_rx) = mpsc::channel::<PresentEvent>();
    let (size_tx, size_rx) = mpsc::channel::<SizeUpdate>();
    let (redraw_tx, redraw_rx) = mpsc::channel::<()>();
    let (outputs_tx, outputs_rx) = mpsc::channel::<OutputOp>();
    let outputs_tx_control = outputs_tx.clone();

    // Start control socket for runtime output ops
    start_output_control_server(outputs_tx_control);

    // Build winit window and wgpu surface on main thread
    let event_loop = EventLoop::new()?;
    let window = winit::window::WindowBuilder::new()
        .with_title("Axiom Compositor")
        .with_visible(true)  // Explicitly request visibility
        .with_resizable(true)
        .build(&event_loop)?;
    
    info!("✅ Window created: '{}'", window.title());
    window.set_visible(true);  // Ensure it's visible
    window.focus_window();  // Request focus

    // Get window size (use regular windowed mode for better compatibility)
    let size = window.inner_size();
    let monitor = window
        .current_monitor()
        .or_else(|| event_loop.primary_monitor())
        .or_else(|| {
            // fallback: first available monitor if any
            let mut iter = event_loop.available_monitors();
            iter.next()
        });
    
    // NOTE: Fullscreen mode disabled for better Wayland compatibility
    // Users can press F11 or use window manager keybindings to fullscreen if needed
    // Uncomment below to enable fullscreen mode:
    // if let Some(m) = &monitor {
    //     window.set_fullscreen(Some(Fullscreen::Borderless(Some(m.clone()))));
    //     let msize = m.size();
    //     if msize.width > 0 && msize.height > 0 {
    //         size = msize;
    //     }
    // }

    // Compose outputs init: prefer CLI spec, else auto-detected from all available monitors
    let auto_outputs = {
        let mut outs = Vec::new();
        let mut x_off: i32 = 0;
        // Use available monitors; fall back to current if iterator empty
        let mut mm: Vec<winit::monitor::MonitorHandle> = event_loop.available_monitors().collect();
        if mm.is_empty() {
            if let Some(m) = &monitor {
                mm.push(m.clone());
            }
        }
        for m in &mm {
            let msize = m.size();
            let width = (msize.width as i32).max(1);
            let height = (msize.height as i32).max(1);
            let refresh_mhz = m
                .refresh_rate_millihertz()
                .map(|v| v as i32)
                .unwrap_or(60_000);
            outs.push(OutputInit {
                width,
                height,
                scale: 1, // Wayland scale per-monitor isn't exposed here; server can adapt later
                pos_x: x_off,
                pos_y: 0,
                name: m.name(),
                model: None,
                refresh_mhz,
            });
            x_off += width;
        }
        if outs.is_empty() {
            Some(vec![OutputInit {
                width: size.width as i32,
                height: size.height as i32,
                scale: 1,
                pos_x: 0,
                pos_y: 0,
                name: monitor.as_ref().and_then(|m| m.name()),
                model: None,
                refresh_mhz: 60_000,
            }])
        } else {
            Some(outs)
        }
    };
    let outputs_init = outputs_init_cli.or(auto_outputs);

    // Runtime copy of rectangles used by renderer scissor per-output
    let mut outputs_rects_runtime: Vec<(u32, u32, u32, u32)> = outputs_init
        .as_ref()
        .map(|outs| {
            outs.iter()
                .map(|o| {
                    (
                        o.pos_x.max(0) as u32,
                        o.pos_y.max(0) as u32,
                        o.width.max(0) as u32,
                        o.height.max(0) as u32,
                    )
                })
                .collect()
        })
        .unwrap_or_else(|| vec![(0, 0, size.width, size.height)]);

    // Track current monitor fingerprint and output count for dynamic updates
    let mut last_monitors_fingerprint: Option<String> = Some({
        let mut s = String::new();
        let mut first = true;
        for m in event_loop.available_monitors() {
            if !first {
                s.push('|');
            } else {
                first = false;
            }
            let sz = m.size();
            let rf = m.refresh_rate_millihertz().unwrap_or(60_000);
            s.push_str(&format!(
                "{}:{}x{}@{}",
                m.name().unwrap_or_else(|| "?".into()),
                sz.width,
                sz.height,
                rf
            ));
        }
        if first {
            // No monitors reported; synthesize from window
            s = format!("win:{}x{}@{}", size.width, size.height, 60_000);
        }
        s
    });
    let mut current_outputs_len: usize = outputs_rects_runtime.len();

    // Spawn unified compositor server (Smithay) in a background thread
    let cfg = AxiomConfig::default();
    let cfg_clone = cfg.clone();
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
        let deco = Arc::new(RwLock::new(DecorationManager::new(&cfg_clone.window)));

        // Prefer libinput; fallback to evdev channel if available
        let input_rx = axiom::smithay::input_backend::init_libinput_backend()
            .or_else(CompositorServer::spawn_evdev_input_channel);

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
            /* spawn_headless_renderer */ false,
            selected_backends,
            outputs_init,
            Some(outputs_rx),
        )
        .expect("server");

        if let Err(e) = server.run() {
            error!("smithay server run error: {}", e);
        }
    });

    // Create instance with selected backends
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: selected_backends,
        ..Default::default()
    });

    // Create surface compatible with the instance
    let surface = instance.create_surface(&window)?;
    let mut size = window.inner_size();

    // Honor present mode override via env var for the renderer's surface config
    let pm = cli.present_mode.to_lowercase();
    if ["fifo", "mailbox", "immediate", "auto"].contains(&pm.as_str()) {
        std::env::set_var("AXIOM_PRESENT_MODE", pm);
    }

    // Create renderer bound to this instance + surface
    let mut renderer = pollster::block_on(AxiomRenderer::new_with_instance(
        &instance,
        Some(&surface),
        size.width.max(1),
        size.height.max(1),
    ))?;

    // Request initial redraw to display the window
    window.request_redraw();

    // Enter event loop
    Ok(event_loop.run(|event, elwt| {
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => elwt.exit(),

            Event::WindowEvent {
                event: WindowEvent::Resized(new_size),
                ..
            } => {
                size = new_size;
                if size.width > 0 && size.height > 0 {
                    // Resize existing renderer (much more efficient than recreating)
                    if let Err(e) = renderer.resize(Some(&surface), size.width, size.height) {
                        eprintln!("renderer resize error: {}", e);
                    }

                    // Send size update to server (primary output)
                    let _ = size_tx.send(SizeUpdate {
                        width: size.width,
                        height: size.height,
                        scale: window.scale_factor() as i32,
                        name: window.current_monitor().and_then(|m| m.name()),
                        model: None,
                    });
                    window.request_redraw();
                }
            }

            Event::AboutToWait => {
                // Dynamic monitor detection and output reconfiguration
                let mut s = String::new();
                let mut first = true;
                let mut new_rects: Vec<(u32, u32, u32, u32)> = Vec::new();
                let mut new_outputs: Vec<OutputInit> = Vec::new();
                let mut x_off: i32 = 0;
                for m in elwt.available_monitors() {
                    if !first { s.push('|'); } else { first = false; }
                    let sz = m.size();
                    let rf = m.refresh_rate_millihertz().unwrap_or(60_000);
                    s.push_str(&format!("{}:{}x{}@{}", m.name().unwrap_or_else(|| "?".into()), sz.width, sz.height, rf));
                    let w = (sz.width as i32).max(1);
                    let h = (sz.height as i32).max(1);
                    new_outputs.push(OutputInit {
                        width: w,
                        height: h,
                        scale: 1,
                        pos_x: x_off,
                        pos_y: 0,
                        name: m.name(),
                        model: None,
                        refresh_mhz: rf as i32,
                    });
                    new_rects.push((x_off.max(0) as u32, 0, w.max(0) as u32, h.max(0) as u32));
                    x_off += w;
                }
                if first {
                    // No monitors reported; keep single rect matching window size
                    s = format!("win:{}x{}@{}", size.width, size.height, 60_000);
                    new_rects = vec![(0, 0, size.width, size.height)];
                    new_outputs = vec![OutputInit {
                        width: size.width as i32,
                        height: size.height as i32,
                        scale: 1,
                        pos_x: 0,
                        pos_y: 0,
                        name: window.current_monitor().and_then(|m| m.name()),
                        model: None,
                        refresh_mhz: 60_000,
                    }];
                }

                if last_monitors_fingerprint.as_ref().map(|t| t.as_str()) != Some(s.as_str()) {
                    // Remove all existing outputs in reverse order for stability
                    for idx in (0..current_outputs_len).rev() {
                        let _ = outputs_tx.send(OutputOp::Remove { index: idx });
                    }
                    // Add new outputs left-to-right
                    for out in &new_outputs {
                        let _ = outputs_tx.send(OutputOp::Add(out.clone()));
                    }
                    // Update runtime data
                    outputs_rects_runtime = new_rects;
                    current_outputs_len = outputs_rects_runtime.len();
                    last_monitors_fingerprint = Some(s);
                }

                // Only request redraw if server has sent redraw requests
                let mut needs_redraw = false;
                while redraw_rx.try_recv().is_ok() {
                    needs_redraw = true;
                }
                if needs_redraw {
                    window.request_redraw();
                }
            }

            Event::WindowEvent {
                event: WindowEvent::RedrawRequested,
                ..
            } => {
                // Sync renderer shared state (if any) and present
                renderer.sync_from_shared();
                // Process any pending texture updates from the Wayland server
                let _ = renderer.process_pending_texture_updates();
                
                // Only render and present if we have content
                let has_content = renderer.window_count() > 0;
                
                if renderer.can_present() {
                    if let Ok(frame) = surface.get_current_texture() {
                        // Use runtime rectangles reflecting current monitor layout
                        let outputs_rects: Vec<(u32, u32, u32, u32)> = outputs_rects_runtime.clone();

                        let debug_overlay = std::env::var("AXIOM_DEBUG_OUTPUTS")
                            .ok()
                            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                            .unwrap_or(false);
                        
                        // Always render something to make the window visible
                        if has_content {
                            // Render actual windows
                            if let Err(e) = renderer.render_to_surface_with_outputs(
                                &surface,
                                &frame,
                                &outputs_rects,
                                debug_overlay,
                            ) {
                                eprintln!("render error: {}", e);
                            }
                        } else {
                            // No windows yet - render a dark gray background so the window is visible
                            let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                            let device = renderer.device();
                            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("Empty Frame Encoder"),
                            });
                            {
                                let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                    label: Some("Empty Frame Render Pass"),
                                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                        view: &view,
                                        resolve_target: None,
                                        ops: wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                                r: 0.1,
                                                g: 0.1,
                                                b: 0.12,
                                                a: 1.0,
                                            }),
                                            store: wgpu::StoreOp::Store,
                                        },
                                    })],
                                    depth_stencil_attachment: None,
                                    timestamp_writes: None,
                                    occlusion_query_set: None,
                                });
                            }
                            renderer.queue().submit(std::iter::once(encoder.finish()));
                        }
                        
                        // Present the frame
                        frame.present();

                        // Send per-output presentation feedback timing (only if we have windows)
                        if has_content {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default();
                            let tv_sec = now.as_secs();
                            let tv_nsec = now.subsec_nanos();
                            let tv_sec_hi: u32 = (tv_sec >> 32) as u32;
                            let tv_sec_lo: u32 = (tv_sec & 0xFFFF_FFFF) as u32;

                            let refresh_ns: u32 = window
                                .current_monitor()
                                .and_then(|m| m.refresh_rate_millihertz())
                                .map(|mhz| {
                                    let ns = 1_000_000_000_000u64 / (mhz as u64);
                                    ns.clamp(8_000_000, 33_333_333) as u32 // ~120Hz..30Hz
                                })
                                .unwrap_or(16_666_666);

                            let flags: u32 = (wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::Vsync
                                | wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind::HwClock)
                                .bits();

                            let outputs_count = current_outputs_len.max(1);
                            for idx in 0..outputs_count {
                                let _ = present_tx.send(PresentEvent {
                                    tv_sec_hi,
                                    tv_sec_lo,
                                    tv_nsec,
                                    refresh_ns,
                                    flags,
                                    output_idx: Some(idx),
                                });
                            }
                        }
                    }
                } else {
                    // Headless fallback render
                    if has_content {
                        let _ = renderer.render();
                    }
                }
            }

            _ => {}
        }
    })?)
}
