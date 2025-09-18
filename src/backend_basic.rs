//! Basic Wayland Backend - Absolute Minimum for Real Functionality
//!
//! This focuses ONLY on getting a real Wayland socket working that clients can connect to.

use anyhow::{Context, Result};
use log::{debug, info};

// Use basic Wayland server functionality
use wayland_server::protocol::{wl_buffer, wl_compositor, wl_shm, wl_shm_pool, wl_surface};
use wayland_server::{
    backend::ClientData, Client, DataInit, Display, DisplayHandle, GlobalDispatch, ListeningSocket,
    New,
};

/// Basic backend that creates a real Wayland compositor
pub struct BasicBackend {
    display: Display<State>,
    listening: ListeningSocket,
    socket_name: String,
    running: bool,
}

/// State for our compositor
#[derive(Default)]
pub struct State {
    surfaces: Vec<wl_surface::WlSurface>,
}

impl BasicBackend {
    pub fn new() -> Result<Self> {
        info!("🚀 Creating basic Wayland backend...");

        // Create display with our state
        let display = Display::<State>::new()?;
        let dh = display.handle();

        // Create globals
        dh.create_global::<State, wl_compositor::WlCompositor, _>(4, ());
        dh.create_global::<State, wl_shm::WlShm, _>(1, ());

        // Create socket
        let listening = ListeningSocket::bind_auto("wayland", 1..32)
            .context("Failed to bind Wayland socket")?;
        let socket_name = listening
            .socket_name()
            .map(|s| s.to_string_lossy().to_string())
            .ok_or_else(|| anyhow::anyhow!("missing socket name"))?;

        info!("✅ Wayland socket created: {}", socket_name);
        std::env::set_var("WAYLAND_DISPLAY", &socket_name);

        Ok(Self {
            display,
            listening,
            socket_name,
            running: false,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        info!("🎬 Starting basic Wayland compositor...");
        info!(
            "   To test: WAYLAND_DISPLAY={} weston-info",
            self.socket_name
        );

        self.running = true;

        let mut state = State::default();

        struct BasicClientData;
        impl ClientData for BasicClientData {
            fn initialized(&self, _client: wayland_server::backend::ClientId) {}
            fn disconnected(
                &self,
                _client: wayland_server::backend::ClientId,
                _reason: wayland_server::backend::DisconnectReason,
            ) {
            }
        }

        // Main event loop
        while self.running {
            // Accept clients
            match self.listening.accept() {
                Ok(Some(stream)) => {
                    let _ = self
                        .display
                        .handle()
                        .insert_client(stream, std::sync::Arc::new(BasicClientData));
                }
                Ok(None) => {}
                Err(_) => {}
            }

            // Dispatch client events
            let _ = self.display.dispatch_clients(&mut state);

            // Flush clients
            let _ = self.display.flush_clients();

            // Log activity
if !state.surfaces.is_empty() {
                debug!("Active surfaces: {}", state.surfaces.len());
            }
        }

        Ok(())
    }
}

// Implement compositor protocol
impl GlobalDispatch<wl_compositor::WlCompositor, ()> for State {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_compositor::WlCompositor>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl wayland_server::Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &wl_compositor::WlCompositor,
        request: wl_compositor::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_compositor::Request::CreateSurface { id } => {
                info!("🪟 Surface creation requested!");
                let surface = data_init.init(id, ());
                state.surfaces.push(surface);
                info!(
                    "✅ Surface created. Total surfaces: {}",
                    state.surfaces.len()
                );
            }
            wl_compositor::Request::CreateRegion { .. } => {
                debug!("Region creation requested");
            }
            _ => {}
        }
    }
}

// Implement surface protocol
impl wayland_server::Dispatch<wl_surface::WlSurface, ()> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_surface::WlSurface,
        request: wl_surface::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_surface::Request::Attach { .. } => {
                debug!("Surface attach");
            }
            wl_surface::Request::Commit => {
                debug!("Surface commit");
            }
            wl_surface::Request::Destroy => {
                debug!("Surface destroy");
            }
            _ => {}
        }
    }
}

// Implement shm protocol
impl GlobalDispatch<wl_shm::WlShm, ()> for State {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_shm::WlShm>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let shm = data_init.init(resource, ());
        // Send supported formats
        shm.format(wl_shm::Format::Argb8888);
        shm.format(wl_shm::Format::Xrgb8888);
    }
}

impl wayland_server::Dispatch<wl_shm::WlShm, ()> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_shm::WlShm,
        request: wl_shm::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
if let wl_shm::Request::CreatePool { id, .. } = request {
            debug!("SHM pool creation requested");
            data_init.init(id, ());
        }
    }
}

impl wayland_server::Dispatch<wl_shm_pool::WlShmPool, ()> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_shm_pool::WlShmPool,
        request: wl_shm_pool::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
if let wl_shm_pool::Request::CreateBuffer { id, .. } = request {
            debug!("Buffer creation requested");
            data_init.init(id, ());
        }
    }
}

impl wayland_server::Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_buffer::WlBuffer,
        _request: wl_buffer::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // Handle buffer requests
    }
}
