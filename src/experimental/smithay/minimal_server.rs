//! Minimal Wayland server using wayland-server 0.31 and calloop
//! This is a thin, compiling server that accepts clients and advertises
//! wl_compositor, wl_shm, wl_output, and xdg_wm_base. No rendering.

use anyhow::{Context, Result};
use log::info;
use std::sync::Arc;

use calloop::{EventLoop, LoopSignal};
use wayland_protocols::xdg::shell::server::xdg_wm_base;
use wayland_server::{
    backend::ClientData,
    protocol::{wl_compositor, wl_output, wl_seat, wl_shm, wl_subcompositor},
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, ListeningSocket, New,
};

/// Global compositor state for this minimal server
#[derive(Default)]
pub struct MinimalState {
    pub seat_name: String,
}

/// A minimal Wayland server that runs a Display and accepts clients
pub struct MinimalServer {
    pub display: Display<MinimalState>,
    pub listening_socket: ListeningSocket,
    pub socket_name: String,
    pub event_loop: Option<EventLoop<'static, MinimalState>>,
    pub loop_signal: Option<LoopSignal>,
}

impl MinimalServer {
    pub fn new() -> Result<Self> {
        let display: Display<MinimalState> = Display::new().context("create display")?;
        let dh = display.handle();

        // Create core globals
        dh.create_global::<MinimalState, wl_compositor::WlCompositor, _>(4, ());
        dh.create_global::<MinimalState, wl_shm::WlShm, _>(1, ());
        dh.create_global::<MinimalState, wl_output::WlOutput, _>(3, ());
        dh.create_global::<MinimalState, wl_seat::WlSeat, _>(7, ());
        dh.create_global::<MinimalState, xdg_wm_base::XdgWmBase, _>(3, ());
        dh.create_global::<MinimalState, wl_subcompositor::WlSubcompositor, _>(1, ());

        let listening_socket = ListeningSocket::bind_auto("wayland", 1..32).context("bind socket")?;
        let socket_name = listening_socket
            .socket_name()
            .and_then(|s| Some(s.to_string_lossy().to_string()))
            .ok_or_else(|| anyhow::anyhow!("missing socket name"))?;

        Ok(Self {
            display,
            listening_socket,
            socket_name,
            event_loop: None,
            loop_signal: None,
        })
    }

    pub fn run(mut self) -> Result<()> {
        std::env::set_var("WAYLAND_DISPLAY", &self.socket_name);
        info!("WAYLAND_DISPLAY={}", self.socket_name);

        let mut state = MinimalState { seat_name: "seat0".into() };
        let mut event_loop: EventLoop<MinimalState> = EventLoop::try_new().context("event loop")?;
        let handle = event_loop.handle();
        let dh = self.display.handle();

        handle.insert_source(self.listening_socket, move |client, _, _state| {
            let _ = dh.insert_client(client, Arc::new(ServerClientData));
        })?;

        loop {
            event_loop.dispatch(std::time::Duration::from_millis(16), &mut state)?;
            self.display.flush_clients()?;
        }
    }
}

struct ServerClientData;
impl ClientData for ServerClientData {}

// wl_compositor global
impl GlobalDispatch<wl_compositor::WlCompositor, ()> for MinimalState {
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
impl Dispatch<wl_compositor::WlCompositor, ()> for MinimalState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_compositor::WlCompositor,
        _request: wl_compositor::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

// wl_shm global
impl GlobalDispatch<wl_shm::WlShm, ()> for MinimalState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_shm::WlShm>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let shm = data_init.init(resource, ());
        shm.format(wl_shm::Format::Argb8888);
        shm.format(wl_shm::Format::Xrgb8888);
    }
}
impl Dispatch<wl_shm::WlShm, ()> for MinimalState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_shm::WlShm,
        _request: wl_shm::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

// wl_output global
impl GlobalDispatch<wl_output::WlOutput, ()> for MinimalState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<wl_output::WlOutput>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let output = data_init.init(resource, ());
        output.geometry(
            0,
            0,
            300,
            200,
            wl_output::Subpixel::Unknown,
            "Axiom".to_string(),
            "Minimal".to_string(),
            wl_output::Transform::Normal,
        );
        output.mode(wl_output::Mode::Current | wl_output::Mode::Preferred, 1920, 1080, 60000);
        output.scale(1);
        output.done();
    }
}
impl Dispatch<wl_output::WlOutput, ()> for MinimalState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &wl_output::WlOutput,
        _request: wl_output::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

// xdg_wm_base global
impl GlobalDispatch<xdg_wm_base::XdgWmBase, ()> for MinimalState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<xdg_wm_base::XdgWmBase>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for MinimalState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &xdg_wm_base::XdgWmBase,
        request: xdg_wm_base::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_wm_base::Request::Pong { serial: _ } => {}
            _ => {}
        }
    }
}
