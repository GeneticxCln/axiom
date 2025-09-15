//! Minimal REAL Smithay Backend - Focus on Core Functionality
//!
//! This is a stripped-down, focused implementation that prioritizes:
//! 1. Real Wayland client connections
//! 2. Surface management  
//! 3. Basic rendering pipeline
//! 4. Hardware interaction
//!
//! Everything else is secondary until these work.

use anyhow::{Context, Result};
use log::{debug, info, warn, error};
use std::time::Instant;

use smithay::{
    reexports::{
        calloop::{self, EventLoop},
        wayland_server::{protocol::{wl_buffer, wl_surface}, Display, DisplayHandle},
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        shell::xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState},
        shm::{ShmHandler, ShmState},
        output::{Output, Mode, PhysicalProperties, Subpixel, OutputManagerState},
        seat::{Seat, SeatHandler, SeatState},
        socket::ListeningSocketSource,
    },
};

/// Minimal state for real Wayland compositor
pub struct MinimalCompositorState {
    // Core Wayland protocol states
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Self>,

    // Display and event loop
    pub display: Display<Self>,
    pub start_time: Instant,

    // Seat and input
    pub seat: Seat<Self>,

    // Output
    pub output: Output,

    // Socket name
    pub socket_name: String,
}

// Minimal WGPU state to present frames
#[cfg(feature = "wgpu-present")]
pub struct WgpuState {
    pub instance: wgpu::Instance,
    pub surface: wgpu::Surface,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
}

impl MinimalCompositorState {
    pub fn new(display: &mut Display<Self>, _event_loop: &EventLoop<Self>) -> Result<Self> {
        // Initialize Wayland protocol states via helpers
        let compositor_state = crate::experimental::smithay::wayland::wayland_protocols::register_compositor::<Self>(display);
        let xdg_shell_state = crate::experimental::smithay::wayland::wayland_protocols::register_xdg_shell::<Self>(display);
        let shm_state = crate::experimental::smithay::wayland::wayland_protocols::register_shm::<Self>(display);
        let output_manager_state = crate::experimental::smithay::wayland::wayland_protocols::register_output_manager::<Self>(display);
        let mut seat_state = SeatState::new();

        // Create a seat with keyboard and pointer
        let (seat, _keyboard, _pointer) = crate::experimental::smithay::wayland::wayland_protocols::create_seat::<Self>(display, &mut seat_state, "seat0")?;

        // Create output
        let mut output = Output::new(
            display,
            "axiom-output".to_string(),
            PhysicalProperties {
                size: (600, 340).into(),
                subpixel: Subpixel::Unknown,
                make: "Axiom".to_string(),
                model: "Virtual".to_string(),
            },
        );

        // Set a mode
        let mode = Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), None, None, None);
        output.set_preferred(mode);
        // Notify output manager
        output_manager_state.output_created(&output);

        Ok(Self {
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            display: display.clone(),
            start_time: Instant::now(),
            seat,
            output,
            socket_name: String::new(),
        })
    }

    pub fn surface_under_pointer(
        &self,
        pointer_location: Point<f64, Logical>,
    ) -> Option<(Window, Point<i32, Logical>)> {
        self.space
            .element_under(pointer_location)
            .map(|(window, location)| (window.clone(), location))
    }
}

// Protocol handler implementations
impl CompositorHandler for MinimalCompositorState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a ClientData) -> &'a CompositorClientState {
        &client.get::<CompositorClientState>().unwrap()
    }

    fn commit(&mut self, surface: &wl_surface::WlSurface) {
        // Handle surface commits
        debug!("Surface commit: {:?}", surface);

        // Check if this surface belongs to a window
        for window in &self.windows {
            if window.toplevel().wl_surface() == surface {
                // Update the window in the space
                self.space.commit(surface);
                debug!("Window surface committed and updated in space");
            }
        }
    }
}

impl BufferHandler for MinimalCompositorState {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {
        // Handle buffer destruction
    }
}

impl ShmHandler for MinimalCompositorState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl XdgShellHandler for MinimalCompositorState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        info!("🪟 New XDG toplevel window created!");

        // Create a window from the toplevel surface
        let window = Window::new(surface);

        // Map the window in our space at origin (we'll position it properly later)
        self.space.map_element(window.clone(), (0, 0), true);

        // Store the window
        self.windows.push(window.clone());

        info!(
            "✅ Window mapped to space. Total windows: {}",
            self.windows.len()
        );
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // Handle popup creation
        debug!("New popup created");
    }

    fn move_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
        // Handle move requests
        debug!("Move request received");
    }

    fn resize_request(
        &mut self,
        _surface: ToplevelSurface,
        _seat: wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
        _edges: xdg_toplevel::ResizeEdge,
    ) {
        // Handle resize requests
        debug!("Resize request received");
    }

    fn grab(
        &mut self,
        _surface: PopupSurface,
        _seat: wl_seat::WlSeat,
        _serial: smithay::utils::Serial,
    ) {
        // Handle popup grabs
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        info!("Window destroyed");
        self.windows.retain(|w| w.toplevel() != &surface);
    }
}

impl SeatHandler for MinimalCompositorState {
    type KeyboardFocus = wl_surface::WlSurface;
    type PointerFocus = wl_surface::WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
        // Handle cursor image changes
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&Self::KeyboardFocus>) {
        // Handle focus changes
    }
}

// Smithay delegate macros for protocol handling
smithay::delegate_compositor!(MinimalCompositorState);
smithay::delegate_shm!(MinimalCompositorState);
smithay::delegate_xdg_shell!(MinimalCompositorState);
smithay::delegate_seat!(MinimalCompositorState);
smithay::delegate_output!(MinimalCompositorState);

/// Main backend structure
pub struct MinimalRealBackend {
    pub state: Option<MinimalCompositorState>,
    pub display: Option<Display<MinimalCompositorState>>,
    pub event_loop: Option<EventLoop<'static, MinimalCompositorState>>,
    pub running: bool,
}

impl MinimalRealBackend {
    pub fn new() -> Result<Self> {
        Ok(Self {
            state: None,
            display: None,
            event_loop: None,
            running: false,
        })
    }

    pub fn initialize(&mut self) -> Result<()> {
        info!("🚀 Initializing REAL minimal Wayland compositor...");

        // Create event loop
        let mut event_loop: EventLoop<MinimalCompositorState> = EventLoop::try_new().context("Failed to create event loop")?;

        // Create display
        let mut display: Display<MinimalCompositorState> = Display::new().context("Failed to create Wayland display")?;

        // Create compositor state
        let mut state = MinimalCompositorState::new(&mut display, &event_loop)?;

        // Add Wayland socket
        let listening = ListeningSocketSource::new_auto().context("Failed to create Wayland listening socket")?;
        let socket_name = listening.socket_name().to_string();
        info!("✅ Wayland socket created: {}", socket_name);
        std::env::set_var("WAYLAND_DISPLAY", &socket_name);

        // Insert listening socket source into event loop
        let dh = display.handle();
        event_loop.handle().insert_source(listening, move |client, _, state| {
            if let Err(e) = dh.insert_client(client, Arc::new(CompositorClientState::default())) {
                error!("Failed to insert client: {}", e);
            }
        })?;

        // Store everything
        state.socket_name = socket_name.clone();
        self.state = Some(state);
        self.display = Some(display);
        self.event_loop = Some(event_loop);

        info!("✅ Real Wayland compositor initialized! Clients: WAYLAND_DISPLAY={}", socket_name);
        Ok(())
    }

    fn init_winit_backend(&mut self) -> Result<()> {
        let event_loop = self.event_loop.as_ref().unwrap();
        let display = self.display.as_mut().unwrap();
        let state_rc = self.state.as_ref().unwrap().clone();

        info!("Initializing Winit backend for windowed mode...");

        // Create winit event loop and window
        let (backend, winit_event_loop) = winit::init()?;

        // Get the renderer
        let renderer = backend.renderer();

        // Store backend and renderer in state
        {
            let mut state = state_rc.borrow_mut();
            // SAFETY: we need the window to create a WGPU surface
            // Assuming smithay's WinitGraphicsBackend exposes a window() accessor
            let window = backend.window();

            #[cfg(feature = "wgpu-present")]
            {
            // Initialize WGPU
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::PRIMARY,
                dx12_shader_compiler: Default::default(),
                flags: wgpu::InstanceFlags::default(),
                gles_minor_version: wgpu::Gles3MinorVersion::Automatic,
            });
            let surface = unsafe { instance.create_surface(window) }.expect("create_surface");
            let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })).expect("request_adapter");
            let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                features: wgpu::Features::empty(),
                limits: wgpu::Limits::default(),
                label: Some("axiom-device"),
            }, None)).expect("request_device");

            let size = window.inner_size();
            let surface_caps = surface.get_capabilities(&adapter);
            let format = surface_caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(surface_caps.formats[0]);
            let config = wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: size.width.max(1),
                height: size.height.max(1),
                present_mode: surface_caps.present_modes[0],
                alpha_mode: surface_caps.alpha_modes[0],
                view_formats: vec![],
            };
            surface.configure(&device, &config);

            state.wgpu = Some(WgpuState { instance, surface, adapter, device, queue, config });
            }

            state.backend = Some(backend);
            state.renderer = Some(renderer);

            // Add output to space
            state.space.map_output(&state.output, (0, 0));
        }

        // Insert winit event source into calloop
        event_loop
            .handle()
            .insert_source(winit_event_loop, move |event, _, state| {
                match event {
                    WinitEvent::Resized { size, .. } => {
                        // Handle resize
                        state.output.change_current_state(
                            Some(Mode {
                                size,
                                refresh: 60_000,
                            }),
                            None,
                            None,
                            None,
                        );
                        // Reconfigure WGPU surface
                        #[cfg(feature = "wgpu-present")]
                        if let Some(wgpu) = state.wgpu.as_mut() {
                            wgpu.config.width = size.w.max(1);
                            wgpu.config.height = size.h.max(1);
                            wgpu.surface.configure(&wgpu.device, &wgpu.config);
                        }
                    }
                    WinitEvent::Input(event) => {
                        // Handle input event
                        debug!("Input event: {:?}", event);
                    }
                    WinitEvent::Redraw => {
                        // Trigger redraw
                        if let Err(e) = Self::render_frame(state) {
                            error!("Failed to render frame: {}", e);
                        }
                    }
                    WinitEvent::CloseRequested => {
                        info!("Window close requested");
                        // TODO: Handle close
                    }
                }
            })
            .context("Failed to insert winit source")?;

        // Also insert the Wayland display source
        event_loop
            .handle()
            .insert_source(
                Generic::new(
                    display.backend().poll_fd().as_raw_fd(),
                    Interest::READ,
                    CallMode::Level,
                ),
                |_, _, state| {
                    // Dispatch Wayland clients
                    state.display_handle.dispatch_clients(state).unwrap();
                    Ok(PostAction::Continue)
                },
            )
            .context("Failed to insert Wayland source")?;

        Ok(())
    }

    fn render_frame(_state: &mut MinimalCompositorState) -> Result<()> {
        // No-op rendering in minimal backend
        Ok(())
    }

    pub fn run(&mut self) -> Result<()> {
        info!("🎬 Starting compositor main loop...");

        let mut event_loop = self.event_loop.take().unwrap();
        let mut display = self.display.take().unwrap();
        let mut state = self.state.take().unwrap();

        self.running = true;

        // Run the event loop
        loop {
            event_loop.dispatch(std::time::Duration::from_millis(16), &mut state)?;
            display.flush_clients()?;
            Self::render_frame(&mut state)?;
        }

        Ok(())
    }
}

// Add ClientData implementation
impl ClientData for CompositorClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
