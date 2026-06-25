use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::rc::Rc;

use anyhow::Result;
use log::info;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt, EventMask, MapRequestEvent,
    Window,
};
use x11rb::rust_connection::{DefaultStream, RustConnection};
use x11rb::wrapper::ConnectionExt as _; // Validation trait

// X11 Atoms
#[allow(non_snake_case)]
pub struct Atoms {
    pub CLIPBOARD: x11rb::protocol::xproto::Atom,
    pub PRIMARY: x11rb::protocol::xproto::Atom,
    pub TARGETS: x11rb::protocol::xproto::Atom,
    pub UTF8_STRING: x11rb::protocol::xproto::Atom,
    pub TEXT: x11rb::protocol::xproto::Atom,
    pub STRING: x11rb::protocol::xproto::Atom,
}

impl Atoms {
    pub fn new(conn: &RustConnection) -> Result<Self> {
        let clipboard = conn.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
        let primary = conn.intern_atom(false, b"PRIMARY")?.reply()?.atom;
        let targets = conn.intern_atom(false, b"TARGETS")?.reply()?.atom;
        let utf8_string = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
        let text = conn.intern_atom(false, b"TEXT")?.reply()?.atom;
        let string = conn.intern_atom(false, b"STRING")?.reply()?.atom;

        Ok(Self {
            CLIPBOARD: clipboard,
            PRIMARY: primary,
            TARGETS: targets,
            UTF8_STRING: utf8_string,
            TEXT: text,
            STRING: string,
        })
    }
}

// use crate::experimental::smithay::smithay_backend_real::State;

/// X11 Window Manager (`XWayland`) integration
#[allow(missing_docs)]
#[derive(Debug)]
pub enum XwmEvent {
    ClipboardRequest {
        requestor: Window,
        selection: x11rb::protocol::xproto::Atom,
        target: x11rb::protocol::xproto::Atom,
        property: x11rb::protocol::xproto::Atom,
        time: x11rb::protocol::xproto::Timestamp,
    },
}

pub struct AxiomXwm {
    conn: Rc<RustConnection>,
    #[allow(dead_code)]
    screen_num: usize,
    #[allow(dead_code)]
    root: Window,
    windows: HashMap<Window, X11WindowData>,
    pub atoms: Atoms,
    selection_window: Window,
}

#[derive(Debug, Clone)]
pub struct X11WindowData {
    pub window_id: Window,
    pub mapped: bool,
    pub title: String,
}

impl AxiomXwm {
    /// Start the XWM with the given connection
    pub fn new(connection: std::os::unix::net::UnixStream) -> Result<Self> {
        info!("🏗️ Initializing Axiom XWM...");

        // Wrap the UnixStream into an x11rb DefaultStream (which implements Stream)
        let (stream, _peer_addr) = DefaultStream::from_unix_stream(connection)?;
        let conn = RustConnection::connect_to_stream(stream, 0)?;
        let screen_num = 0;
        let conn = Rc::new(conn);

        let root = conn.setup().roots[screen_num].root;

        // Select SubstructureNotify and SubstructureRedirect on root window
        // This allows us to intercept MapRequests
        let values = ChangeWindowAttributesAux::new()
            .event_mask(EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT);

        conn.change_window_attributes(root, &values)?;
        conn.flush()?;

        info!("✅ Axiom XWM initialized on root window {}", root);

        // Initialize Atoms
        let atoms = Atoms::new(&conn)?;

        // Create a dummy window for selection management
        let selection_window = conn.generate_id()?;
        conn.create_window(
            0, // depth (0 = copy from parent)
            selection_window,
            root,
            0,
            0,
            1,
            1, // x, y, width, height
            0, // border width
            x11rb::protocol::xproto::WindowClass::INPUT_ONLY,
            0, // visual (0 = copy from parent)
            &x11rb::protocol::xproto::CreateWindowAux::new().event_mask(
                EventMask::PROPERTY_CHANGE, // To listen for property updates during transfers
            ),
        )?;

        log::info!("📋 Created X11 selection window: {}", selection_window);

        Ok(Self {
            conn,
            screen_num,
            root,
            windows: HashMap::new(),
            atoms,
            selection_window,
        })
    }

    /// Handle X11 events
    pub fn handle_event(&mut self, event: &x11rb::protocol::Event) -> Result<Option<XwmEvent>> {
        match event {
            x11rb::protocol::Event::MapRequest(event) => {
                self.handle_map_request(*event)?;
                Ok(None)
            }
            x11rb::protocol::Event::ConfigureRequest(event) => {
                // Grant configure request for now
                let values = ConfigureWindowAux::from_configure_request(event);
                self.conn.configure_window(event.window, &values)?;
                Ok(None)
            }
            x11rb::protocol::Event::MapNotify(event) => {
                info!("🪟 X11 Window Mapped: {}", event.window);
                Ok(None)
            }
            x11rb::protocol::Event::UnmapNotify(event) => {
                info!("🪟 X11 Window Unmapped: {}", event.window);
                self.windows.remove(&event.window);
                Ok(None)
            }
            x11rb::protocol::Event::SelectionNotify(event) => {
                self.handle_selection_notify(*event);
                Ok(None)
            }
            x11rb::protocol::Event::SelectionRequest(event) => {
                // Return event to backend for processing
                Ok(Some(XwmEvent::ClipboardRequest {
                    requestor: event.requestor,
                    selection: event.selection,
                    target: event.target,
                    property: event.property,
                    time: event.time,
                }))
            }
            _ => {
                // trace!("Unhandled X11 event: {:?}", event);
                Ok(None)
            }
        }
    }

    fn handle_map_request(&mut self, event: MapRequestEvent) -> Result<()> {
        info!("🪟 Processing MapRequest for window {}", event.window);

        // Map the window
        self.conn.map_window(event.window)?;
        self.conn.flush()?;

        // Track the window
        let data = X11WindowData {
            window_id: event.window,
            mapped: true,
            title: "X11 Window".into(), // TODO: Fetch title property
        };
        self.windows.insert(event.window, data);

        // Notify Axiom state about new window
        // We'll use a placeholder surface ID for now, or need a way to wrap X11 window as surface
        // Since Smithay 0.3 doesn't give us X11Surface, we might just track it internally
        // or create a dummy surface if possible.

        // For Phase 7.2.2, simply seeing the window appear (managed by XWayland/X11) is the goal.
        // Axiom needs to know it exists to layout it?
        // If we map it, XWayland creates a Wayland surface for it.
        // We will receive a XdgNewToplevel or similar?
        // actually XWayland clients usually appear as wl_surface/xdg_surface to the compositor!
        // So Smithay's `xdg_shell_init` callback might catch it IF XWayland uses XDG shell.
        // Standard XWayland uses `zxdg_shell_v6` or `xdg_shell`.

        info!("✅ Mapped X11 window {}", event.window);
        Ok(())
    }

    fn handle_selection_notify(&mut self, event: x11rb::protocol::xproto::SelectionNotifyEvent) {
        log::info!(
            "📋 SelectionNotify: selection={:?}, target={:?}, property={:?}",
            event.selection,
            event.target,
            event.property
        );
        // This means someone else successfully claimed selection, or we successfully requested it.
        // For now, identifying that X11 selection changed is enough to log.
    }

    /// Respond to an X11 selection request with the given clipboard content.
    /// If `clipboard_data` is `Some`, sends those bytes as the selection value.
    /// If `None`, sends a default placeholder and logs a warning.
    pub fn handle_selection_request(
        &self,
        requestor: Window,
        selection: x11rb::protocol::xproto::Atom,
        target: x11rb::protocol::xproto::Atom,
        property: x11rb::protocol::xproto::Atom,
        time: x11rb::protocol::xproto::Timestamp,
        clipboard_data: Option<&[u8]>,
    ) -> Result<()> {
        log::debug!(
            "📋 SelectionRequest: requestor={}, selection={:?}, target={:?}, property={:?}",
            requestor,
            selection,
            target,
            property
        );

        if target == self.atoms.TARGETS {
            // Advertise supported formats
            let targets = [self.atoms.TARGETS, self.atoms.UTF8_STRING, self.atoms.TEXT];
            self.conn.change_property32(
                x11rb::protocol::xproto::PropMode::REPLACE,
                requestor,
                property,
                x11rb::protocol::xproto::AtomEnum::ATOM,
                &targets,
            )?;
        } else if target == self.atoms.UTF8_STRING || target == self.atoms.TEXT {
            let data = clipboard_data.unwrap_or_else(|| {
                log::warn!(
                    "⚠️ X11 clipboard request but no Wayland data cached — sending placeholder"
                );
                b"[axiom: no Wayland clipboard data]"
            });
            self.conn.change_property8(
                x11rb::protocol::xproto::PropMode::REPLACE,
                requestor,
                property,
                self.atoms.UTF8_STRING,
                data,
            )?;
        }

        // Notify the requestor that the selection data is ready
        let notify = x11rb::protocol::xproto::SelectionNotifyEvent {
            response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time,
            requestor,
            selection,
            target,
            property: if target == self.atoms.TARGETS
                || target == self.atoms.UTF8_STRING
                || target == self.atoms.TEXT
            {
                property
            } else {
                x11rb::protocol::xproto::AtomEnum::NONE.into()
            },
        };

        self.conn
            .send_event(false, requestor, EventMask::NO_EVENT, notify)?;
        self.conn.flush()?;

        Ok(())
    }

    /// Claim the X11 clipboard selection
    pub fn own_selection(&self) -> Result<()> {
        let time = x11rb::CURRENT_TIME; // Should use real timestamp if possible
        self.conn
            .set_selection_owner(self.selection_window, self.atoms.CLIPBOARD, time)?;
        self.conn.flush()?;
        log::info!("📋 Axiom claimed X11 CLIPBOARD ownership");
        Ok(())
    }

    pub fn fd(&self) -> std::os::unix::io::RawFd {
        self.conn.stream().as_raw_fd()
    }

    pub fn flush(&self) -> Result<()> {
        self.conn.flush()?;
        Ok(())
    }

    pub fn poll_event(&self) -> Option<x11rb::protocol::Event> {
        self.conn.poll_for_event().unwrap_or(None)
    }
    pub fn send_selection_notify(
        &self,
        requestor: Window,
        selection: x11rb::protocol::xproto::Atom,
        target: x11rb::protocol::xproto::Atom,
        property: x11rb::protocol::xproto::Atom,
        time: x11rb::protocol::xproto::Timestamp,
        success: bool,
    ) -> Result<()> {
        let property = if success {
            property
        } else {
            x11rb::protocol::xproto::AtomEnum::NONE.into()
        };
        let notify = x11rb::protocol::xproto::SelectionNotifyEvent {
            response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time,
            requestor,
            selection,
            target,
            property,
        };
        self.conn
            .send_event(false, requestor, EventMask::NO_EVENT, notify)?;
        self.conn.flush()?;
        Ok(())
    }

    pub fn write_property_string(
        &self,
        window: Window,
        property: x11rb::protocol::xproto::Atom,
        type_: x11rb::protocol::xproto::Atom,
        data: &[u8],
    ) -> Result<()> {
        self.conn.change_property8(
            x11rb::protocol::xproto::PropMode::REPLACE,
            window,
            property,
            type_,
            data,
        )?;
        Ok(())
    }
}
