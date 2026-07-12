use std::collections::HashMap;
use std::os::unix::io::AsRawFd;
use std::rc::Rc;

use anyhow::Result;
use log::info;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt, EventMask, MapRequestEvent,
    Window,
};
use x11rb::rust_connection::{DefaultStream, RustConnection};
use x11rb::wrapper::ConnectionExt as _; // Validation trait

fn decode_text_property(bytes: &[u8]) -> Option<String> {
    let trimmed = bytes.split(|b| *b == 0).next().unwrap_or(bytes);
    let text = String::from_utf8_lossy(trimmed).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn decode_wm_class(bytes: &[u8]) -> Option<String> {
    let parts: Vec<String> = bytes
        .split(|b| *b == 0)
        .filter_map(|part| {
            let s = String::from_utf8_lossy(part).trim().to_string();
            (!s.is_empty()).then_some(s)
        })
        .collect();

    match parts.as_slice() {
        [instance, class, ..] => Some(format!("{} ({})", class, instance)),
        [only] => Some(only.clone()),
        _ => None,
    }
}

// X11 Atoms
#[allow(non_snake_case)]
pub struct Atoms {
    pub CLIPBOARD: x11rb::protocol::xproto::Atom,
    pub PRIMARY: x11rb::protocol::xproto::Atom,
    pub TARGETS: x11rb::protocol::xproto::Atom,
    pub UTF8_STRING: x11rb::protocol::xproto::Atom,
    pub TEXT: x11rb::protocol::xproto::Atom,
    pub STRING: x11rb::protocol::xproto::Atom,
    pub WM_NAME: x11rb::protocol::xproto::Atom,
    pub WM_CLASS: x11rb::protocol::xproto::Atom,
    pub _NET_WM_NAME: x11rb::protocol::xproto::Atom,
    pub _NET_SUPPORTING_WM_CHECK: x11rb::protocol::xproto::Atom,
    pub _NET_SUPPORTED: x11rb::protocol::xproto::Atom,
    pub _NET_WM_STATE: x11rb::protocol::xproto::Atom,
    pub _NET_WM_STATE_FULLSCREEN: x11rb::protocol::xproto::Atom,
    pub _NET_WM_STATE_MAXIMIZED_VERT: x11rb::protocol::xproto::Atom,
    pub _NET_WM_STATE_MAXIMIZED_HORZ: x11rb::protocol::xproto::Atom,
    pub _NET_ACTIVE_WINDOW: x11rb::protocol::xproto::Atom,
    pub _NET_FRAME_EXTENTS: x11rb::protocol::xproto::Atom,
    pub _NET_WM_MOVERESIZE: x11rb::protocol::xproto::Atom,
    pub WM_S0: x11rb::protocol::xproto::Atom,
    pub _NET_WM_CM_S0: x11rb::protocol::xproto::Atom,
    pub AXIOM_CLIPBOARD_TRANSFER: x11rb::protocol::xproto::Atom,
}

impl Atoms {
    pub fn new(conn: &RustConnection) -> Result<Self> {
        let clipboard = conn.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
        let primary = conn.intern_atom(false, b"PRIMARY")?.reply()?.atom;
        let targets = conn.intern_atom(false, b"TARGETS")?.reply()?.atom;
        let utf8_string = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
        let text = conn.intern_atom(false, b"TEXT")?.reply()?.atom;
        let string = conn.intern_atom(false, b"STRING")?.reply()?.atom;
        let wm_name = conn.intern_atom(false, b"WM_NAME")?.reply()?.atom;
        let wm_class = conn.intern_atom(false, b"WM_CLASS")?.reply()?.atom;
        let net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;
        let net_supporting_wm_check = conn
            .intern_atom(false, b"_NET_SUPPORTING_WM_CHECK")?
            .reply()?
            .atom;
        let net_supported = conn.intern_atom(false, b"_NET_SUPPORTED")?.reply()?.atom;
        let net_wm_state = conn.intern_atom(false, b"_NET_WM_STATE")?.reply()?.atom;
        let net_wm_state_fullscreen = conn
            .intern_atom(false, b"_NET_WM_STATE_FULLSCREEN")?
            .reply()?
            .atom;
        let net_wm_state_maximized_vert = conn
            .intern_atom(false, b"_NET_WM_STATE_MAXIMIZED_VERT")?
            .reply()?
            .atom;
        let net_wm_state_maximized_horz = conn
            .intern_atom(false, b"_NET_WM_STATE_MAXIMIZED_HORZ")?
            .reply()?
            .atom;
        let net_active_window = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
            .reply()?
            .atom;
        let net_frame_extents = conn
            .intern_atom(false, b"_NET_FRAME_EXTENTS")?
            .reply()?
            .atom;
        let net_wm_moveresize = conn
            .intern_atom(false, b"_NET_WM_MOVERESIZE")?
            .reply()?
            .atom;
        let wm_s0 = conn.intern_atom(false, b"WM_S0")?.reply()?.atom;
        let net_wm_cm_s0 = conn.intern_atom(false, b"_NET_WM_CM_S0")?.reply()?.atom;
        let axiom_clipboard_transfer = conn
            .intern_atom(false, b"AXIOM_CLIPBOARD_TRANSFER")?
            .reply()?
            .atom;

        Ok(Self {
            CLIPBOARD: clipboard,
            PRIMARY: primary,
            TARGETS: targets,
            UTF8_STRING: utf8_string,
            TEXT: text,
            STRING: string,
            WM_NAME: wm_name,
            WM_CLASS: wm_class,
            _NET_WM_NAME: net_wm_name,
            _NET_SUPPORTING_WM_CHECK: net_supporting_wm_check,
            _NET_SUPPORTED: net_supported,
            _NET_WM_STATE: net_wm_state,
            _NET_WM_STATE_FULLSCREEN: net_wm_state_fullscreen,
            _NET_WM_STATE_MAXIMIZED_VERT: net_wm_state_maximized_vert,
            _NET_WM_STATE_MAXIMIZED_HORZ: net_wm_state_maximized_horz,
            _NET_ACTIVE_WINDOW: net_active_window,
            _NET_FRAME_EXTENTS: net_frame_extents,
            _NET_WM_MOVERESIZE: net_wm_moveresize,
            WM_S0: wm_s0,
            _NET_WM_CM_S0: net_wm_cm_s0,
            AXIOM_CLIPBOARD_TRANSFER: axiom_clipboard_transfer,
        })
    }
}

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
    /// Clipboard bytes read back from an external X11 selection owner.
    ClipboardUpdated {
        owner: Window,
        mime_type: &'static str,
        data: Vec<u8>,
    },
    /// The external X11 clipboard owner disappeared or the transfer failed.
    ClipboardCleared,
    /// A new X11 window was mapped and should be added to the compositor.
    WindowMapped {
        x11_window_id: u32,
        title: String,
        class: Option<String>,
    },
    /// An X11 window was unmapped and should be removed from the compositor.
    WindowUnmapped { x11_window_id: u32 },
}

pub struct AxiomXwm {
    conn: Rc<RustConnection>,
    windows: HashMap<Window, X11WindowData>,
    pub atoms: Atoms,
    selection_window: Window,
    #[allow(dead_code)]
    wm_window: Window,
    last_clipboard_owner: Window,
    pending_clipboard_owner: Option<Window>,
    pending_clipboard_target: Option<Atom>,
}

#[derive(Debug, Clone)]
pub struct X11WindowData {
    pub window_id: Window,
    pub mapped: bool,
    pub title: String,
    pub class: Option<String>,
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

        // Create a dummy window for clipboard selection management.
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

        // Create a tiny input/output window for EWMH window-manager ownership.
        let wm_window = conn.generate_id()?;
        conn.create_window(
            x11rb::COPY_FROM_PARENT as u8,
            wm_window,
            root,
            0,
            0,
            10,
            10,
            0,
            x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
            0,
            &x11rb::protocol::xproto::CreateWindowAux::new(),
        )?;
        conn.change_property32(
            x11rb::protocol::xproto::PropMode::REPLACE,
            wm_window,
            atoms._NET_SUPPORTING_WM_CHECK,
            x11rb::protocol::xproto::AtomEnum::WINDOW,
            &[wm_window],
        )?;
        conn.change_property8(
            x11rb::protocol::xproto::PropMode::REPLACE,
            wm_window,
            atoms._NET_WM_NAME,
            atoms.UTF8_STRING,
            b"Axiom",
        )?;
        conn.change_property32(
            x11rb::protocol::xproto::PropMode::REPLACE,
            root,
            atoms._NET_SUPPORTING_WM_CHECK,
            x11rb::protocol::xproto::AtomEnum::WINDOW,
            &[wm_window],
        )?;
        let supported_atoms = [
            atoms._NET_WM_MOVERESIZE,
            atoms._NET_WM_STATE,
            atoms._NET_WM_STATE_FULLSCREEN,
            atoms._NET_WM_STATE_MAXIMIZED_VERT,
            atoms._NET_WM_STATE_MAXIMIZED_HORZ,
            atoms._NET_ACTIVE_WINDOW,
            atoms._NET_FRAME_EXTENTS,
        ];
        conn.change_property32(
            x11rb::protocol::xproto::PropMode::REPLACE,
            root,
            atoms._NET_SUPPORTED,
            x11rb::protocol::xproto::AtomEnum::ATOM,
            &supported_atoms,
        )?;
        conn.set_selection_owner(wm_window, atoms.WM_S0, x11rb::CURRENT_TIME)?;
        conn.set_selection_owner(wm_window, atoms._NET_WM_CM_S0, x11rb::CURRENT_TIME)?;
        conn.flush()?;

        log::info!(
            "📋 Created X11 selection window: {} and WM owner window: {}",
            selection_window,
            wm_window
        );

        Ok(Self {
            conn,
            windows: HashMap::new(),
            atoms,
            selection_window,
            wm_window,
            last_clipboard_owner: x11rb::NONE,
            pending_clipboard_owner: None,
            pending_clipboard_target: None,
        })
    }

    fn clipboard_targets(&self) -> [Atom; 3] {
        [self.atoms.UTF8_STRING, self.atoms.STRING, self.atoms.TEXT]
    }

    fn clipboard_target_label(&self, atom: Atom) -> &'static str {
        if atom == self.atoms.UTF8_STRING {
            "text/plain;charset=utf-8"
        } else if atom == self.atoms.STRING || atom == self.atoms.TEXT {
            "text/plain"
        } else {
            "application/octet-stream"
        }
    }

    fn next_clipboard_target(&self, current: Atom) -> Option<Atom> {
        let targets = self.clipboard_targets();
        targets
            .iter()
            .position(|candidate| *candidate == current)
            .and_then(|idx| targets.get(idx + 1).copied())
    }

    fn request_external_clipboard_conversion(&mut self, owner: Window, target: Atom) -> Result<()> {
        self.conn.convert_selection(
            self.selection_window,
            self.atoms.CLIPBOARD,
            target,
            self.atoms.AXIOM_CLIPBOARD_TRANSFER,
            x11rb::CURRENT_TIME,
        )?;
        self.conn.flush()?;
        self.pending_clipboard_owner = Some(owner);
        self.pending_clipboard_target = Some(target);
        info!(
            "📋 Requested external X11 clipboard owner {} via target {:?}",
            owner, target
        );
        Ok(())
    }

    /// Poll the current X11 clipboard owner and request the payload when an
    /// external owner takes over.
    pub fn poll_external_clipboard_owner(&mut self) -> Result<Option<XwmEvent>> {
        let owner = self
            .conn
            .get_selection_owner(self.atoms.CLIPBOARD)?
            .reply()?
            .owner;

        if owner == x11rb::NONE {
            let should_clear = self.last_clipboard_owner != x11rb::NONE;
            self.last_clipboard_owner = x11rb::NONE;
            self.pending_clipboard_owner = None;
            self.pending_clipboard_target = None;
            return Ok(should_clear.then_some(XwmEvent::ClipboardCleared));
        }

        if owner == self.selection_window {
            self.last_clipboard_owner = owner;
            self.pending_clipboard_owner = None;
            self.pending_clipboard_target = None;
            return Ok(None);
        }

        if self.last_clipboard_owner != owner || self.pending_clipboard_owner != Some(owner) {
            self.last_clipboard_owner = owner;
            self.request_external_clipboard_conversion(owner, self.atoms.UTF8_STRING)?;
        }

        Ok(None)
    }

    /// Handle X11 events
    pub fn handle_event(&mut self, event: &x11rb::protocol::Event) -> Result<Option<XwmEvent>> {
        match event {
            x11rb::protocol::Event::MapRequest(event) => self.handle_map_request(*event),
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
                Ok(Some(XwmEvent::WindowUnmapped {
                    x11_window_id: event.window,
                }))
            }
            x11rb::protocol::Event::SelectionNotify(event) => self.handle_selection_notify(*event),
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

    fn handle_map_request(&mut self, event: MapRequestEvent) -> Result<Option<XwmEvent>> {
        info!("🪟 Processing MapRequest for window {}", event.window);

        // Map the window
        self.conn.map_window(event.window)?;
        self.conn.flush()?;

        // Fetch X11 metadata.
        let title = self.fetch_window_title(event.window);
        let class = self.fetch_window_class(event.window);

        // Track the window
        let data = X11WindowData {
            window_id: event.window,
            mapped: true,
            title: title.clone(),
            class: class.clone(),
        };
        self.windows.insert(event.window, data);

        info!(
            "✅ Mapped X11 window {}: title=\"{}\" class={:?}",
            event.window, title, class
        );

        // Return event so backend can create a compositor window
        Ok(Some(XwmEvent::WindowMapped {
            x11_window_id: event.window,
            title,
            class,
        }))
    }

    /// Fetch the window title from `_NET_WM_NAME` (UTF-8) or `WM_NAME`.
    fn fetch_window_title(&self, window: Window) -> String {
        // EWMH title first: property = _NET_WM_NAME, type = UTF8_STRING
        if let Ok(reply) = self.conn.get_property(
            false,
            window,
            self.atoms._NET_WM_NAME,
            self.atoms.UTF8_STRING,
            0,
            1024,
        ) {
            if let Ok(prop) = reply.reply() {
                if let Some(title) = decode_text_property(&prop.value) {
                    return title;
                }
            }
        }

        // ICCCM fallback: property = WM_NAME, allow legacy STRING/TEXT storage.
        if let Ok(reply) = self.conn.get_property(
            false,
            window,
            self.atoms.WM_NAME,
            x11rb::protocol::xproto::AtomEnum::ANY,
            0,
            1024,
        ) {
            if let Ok(prop) = reply.reply() {
                if let Some(title) = decode_text_property(&prop.value) {
                    return title;
                }
            }
        }

        // Last fallback: use WM_CLASS so launchers/tooling still get something
        // meaningful when the title itself is absent.
        if let Some(class) = self.fetch_window_class(window) {
            return class;
        }

        format!("X11 Window #{}", window)
    }

    /// Fetch the X11 window class from `WM_CLASS`.
    fn fetch_window_class(&self, window: Window) -> Option<String> {
        if let Ok(reply) = self.conn.get_property(
            false,
            window,
            self.atoms.WM_CLASS,
            x11rb::protocol::xproto::AtomEnum::STRING,
            0,
            1024,
        ) {
            if let Ok(prop) = reply.reply() {
                return decode_wm_class(&prop.value);
            }
        }
        None
    }

    fn handle_selection_notify(
        &mut self,
        event: x11rb::protocol::xproto::SelectionNotifyEvent,
    ) -> Result<Option<XwmEvent>> {
        log::info!(
            "📋 SelectionNotify: selection={:?}, target={:?}, property={:?}",
            event.selection,
            event.target,
            event.property
        );

        if event.selection != self.atoms.CLIPBOARD || event.requestor != self.selection_window {
            return Ok(None);
        }

        let pending_owner = self.pending_clipboard_owner;
        let pending_target = self.pending_clipboard_target;

        if event.property == x11rb::NONE {
            if let (Some(owner), Some(target)) = (pending_owner, pending_target) {
                if let Some(fallback) = self.next_clipboard_target(target) {
                    self.request_external_clipboard_conversion(owner, fallback)?;
                    return Ok(None);
                }
            }
            self.pending_clipboard_owner = None;
            self.pending_clipboard_target = None;
            return Ok(Some(XwmEvent::ClipboardCleared));
        }

        let prop = self.conn.get_property(
            true,
            self.selection_window,
            self.atoms.AXIOM_CLIPBOARD_TRANSFER,
            x11rb::protocol::xproto::AtomEnum::ANY,
            0,
            1 << 20,
        )?;
        let prop = prop.reply()?;
        self.pending_clipboard_owner = None;
        self.pending_clipboard_target = None;

        if prop.value.is_empty() {
            return Ok(Some(XwmEvent::ClipboardCleared));
        }

        Ok(Some(XwmEvent::ClipboardUpdated {
            owner: pending_owner.unwrap_or(self.last_clipboard_owner),
            mime_type: self.clipboard_target_label(event.target),
            data: prop.value,
        }))
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

    /// Claim the X11 clipboard selection.
    pub fn own_selection(&mut self) -> Result<()> {
        let time = x11rb::CURRENT_TIME; // Should use real timestamp if possible
        self.conn
            .set_selection_owner(self.selection_window, self.atoms.CLIPBOARD, time)?;
        self.conn.flush()?;
        self.last_clipboard_owner = self.selection_window;
        self.pending_clipboard_owner = None;
        self.pending_clipboard_target = None;
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

#[cfg(test)]
mod tests {
    use super::{decode_text_property, decode_wm_class};

    #[test]
    fn test_decode_text_property_trims_nul_and_whitespace() {
        let raw = b"Hello World\0ignored";
        assert_eq!(decode_text_property(raw).as_deref(), Some("Hello World"));
        assert_eq!(decode_text_property(b"   "), None);
    }

    #[test]
    fn test_decode_wm_class_prefers_class_and_instance() {
        let raw = b"org.example.Term\0Term\0";
        assert_eq!(
            decode_wm_class(raw).as_deref(),
            Some("Term (org.example.Term)")
        );
    }

    #[test]
    fn test_decode_wm_class_single_segment() {
        let raw = b"XTerm\0";
        assert_eq!(decode_wm_class(raw).as_deref(), Some("XTerm"));
    }
}
