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
use log::{debug, info, warn};
use std::os::unix::io::AsRawFd;
use std::time::Instant;

// WGPU (optional)
#[cfg(feature = "wgpu-present")]
use wgpu::util::DeviceExt;

use smithay::{
    backend::{
        allocator::{dmabuf::Dmabuf, Format, Fourcc, Modifier},
        renderer::{
            element::surface::WaylandSurfaceRenderElement, gles2::Gles2Renderer, Bind, Frame,
            Renderer, Unbind,
        },
        winit::{self, WinitEvent, WinitEventLoop, WinitGraphicsBackend},
    },
    desktop::{
        space::{Space, SurfaceTree},
        Window, WindowSurfaceType,
    },
    input::{
        keyboard::{keysyms, FilterResult, KeyboardHandle, Keysym, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, PointerHandle, RelativeMotionEvent},
        Seat, SeatHandler, SeatState,
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            generic::Generic, EventLoop, Interest, LoopHandle, Mode as CallMode, PostAction,
        },
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_buffer, wl_shm, wl_surface},
            Display, DisplayHandle, Resource,
        },
    },
    utils::{IsAlive, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler,
            CompositorState, SurfaceAttributes, TraversalAction,
        },
        dmabuf::DmabufHandler,
        output::OutputManagerState,
        shell::xdg::{
            Configure, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler,
            XdgShellState, XdgToplevelSurfaceData,
        },
        shm::{ShmHandler, ShmState},
        socket::ListeningSocketSource,
    },
};

use smithay::delegate_compositor;
use smithay::delegate_output;
use smithay::delegate_seat;
use smithay::delegate_shm;
use smithay::delegate_xdg_shell;

/// Minimal state for real Wayland compositor
pub struct MinimalCompositorState {
    // Core Wayland protocol states
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Self>,

    // Display and event loop
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, Self>,
    pub start_time: Instant,

    // Space for window management
    pub space: Space<Window>,

    // Seat and input
    pub seat: Seat<Self>,
    pub keyboard: KeyboardHandle<Self>,
    pub pointer: PointerHandle<Self>,

    // Backend and renderer (will be set during initialization)
    pub backend: Option<WinitGraphicsBackend>,
    pub renderer: Option<Gles2Renderer>,

    // Optional WGPU state for presenting frames
    #[cfg(feature = "wgpu-present")]
    pub wgpu: Option<WgpuState>,

    // Output
    pub output: Output,

    // Window tracking
    pub windows: Vec<Window>,

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
    pub fn new(display: &mut Display<Self>, event_loop: &EventLoop<Self>) -> Result<Self> {
        let display_handle = display.handle();
        let loop_handle = event_loop.handle();

        // Initialize Wayland protocol states
        let compositor_state = CompositorState::new::<Self>(&display_handle);
        let xdg_shell_state = XdgShellState::new::<Self>(&display_handle);
        let shm_state = ShmState::new::<Self>(&display_handle, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&display_handle);
        let mut seat_state = SeatState::new();

        // Create a seat with keyboard and pointer
        let mut seat = seat_state.new_wl_seat(&display_handle, "seat0");
        let keyboard = seat.add_keyboard(Default::default(), 200, 25)?;
        let pointer = seat.add_pointer();

        // Create output
        let output = Output::new(
            "winit".to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
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
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            None,
            Some((0, 0).into()),
        );
        output.set_preferred(mode);

        // Create space for window management
        let space = Space::default();

        Ok(Self {
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            display_handle,
            loop_handle,
            start_time: Instant::now(),
            space,
            seat,
            keyboard,
            pointer,
            backend: None,
            renderer: None,
            #[cfg(feature = "wgpu-present")]
            wgpu: None,
            output,
            windows: Vec::new(),
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
delegate_compositor!(MinimalCompositorState);
delegate_shm!(MinimalCompositorState);
delegate_xdg_shell!(MinimalCompositorState);
delegate_seat!(MinimalCompositorState);
delegate_output!(MinimalCompositorState);

/// Main backend structure
pub struct MinimalRealBackend {
    pub state: Option<Rc<RefCell<MinimalCompositorState>>>,
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
        let mut event_loop = EventLoop::try_new().context("Failed to create event loop")?;

        // Create display
        let mut display = Display::new().context("Failed to create Wayland display")?;

        // Create compositor state
        let state = MinimalCompositorState::new(&mut display, &event_loop)?;

        // Add Wayland socket
        let socket_name = display
            .add_socket_auto()
            .context("Failed to add Wayland socket")?;
        let socket_name_str = socket_name.to_string_lossy().to_string();

        info!("✅ Wayland socket created: {}", socket_name_str);
        std::env::set_var("WAYLAND_DISPLAY", &socket_name_str);

        // Store everything
        let state_rc = Rc::new(RefCell::new(state));
        state_rc.borrow_mut().socket_name = socket_name_str.clone();

        self.state = Some(state_rc);
        self.display = Some(display);
        self.event_loop = Some(event_loop);

        // Initialize winit backend
        self.init_winit_backend()?;

        info!("✅ Real Wayland compositor initialized!");
        info!(
            "   Clients can connect via WAYLAND_DISPLAY={}",
            socket_name_str
        );

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

    fn render_frame(state: &mut MinimalCompositorState) -> Result<()> {
        // Prefer WGPU if available
        #[cfg(feature = "wgpu-present")]
        if let Some(wgpu) = state.wgpu.as_mut() {
            let frame = match wgpu.surface.get_current_texture() {
                Ok(frame) => frame,
                Err(e) => {
                    // Try to recover by reconfiguring
                    wgpu.surface.configure(&wgpu.device, &wgpu.config);
                    wgpu.surface.get_current_texture()?
                }
            };
            let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = wgpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("axiom-render-encoder"),
            });
            {
                let _rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("axiom-clear-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.08, g: 0.09, b: 0.11, a: 1.0 }), store: true },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            wgpu.queue.submit(Some(encoder.finish()));
            frame.present();
            return Ok(());
        }

        // Fallback to GLES2 path if WGPU is not available or feature disabled
        let renderer = state.renderer.as_mut().unwrap();
        let backend = state.backend.as_mut().unwrap();

        // Bind the renderer to the backend
        backend.bind()?;

        // Clear the frame
        renderer.clear([0.1, 0.1, 0.1, 1.0])?;

        // Finish the frame
        backend.submit(None)?;
        backend.unbind()?;

        Ok(())
    }

    pub fn run(&mut self) -> Result<()> {
        info!("🎬 Starting compositor main loop...");

        let event_loop = self.event_loop.take().unwrap();
        let mut display = self.display.take().unwrap();
        let state = self.state.as_ref().unwrap().clone();

        self.running = true;

        // Run the event loop
        event_loop.run(None, &mut state.borrow_mut(), |state| {
            // Dispatch clients
            state.display_handle.dispatch_clients(state).unwrap();

            // Check windows need redraw
            if !state.windows.is_empty() {
                if let Err(e) = Self::render_frame(state) {
                    error!("Render error: {}", e);
                }
            }
        })?;

        Ok(())
    }
}

// Add ClientData implementation
impl ClientData for CompositorClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
