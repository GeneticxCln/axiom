//! Input event dispatch for the Smithay winit/GLES backend.
//!
//! Contains `AxiomSmithayBackendReal::handle_input_event` and the pointer/
//! keyboard/touch dispatch helpers it drives. A submodule of `backend` can
//! read the private fields of `AxiomSmithayBackendReal` (descendant modules
//! see ancestor privates), so no fields were made public for this move.

use log::{debug, info, warn};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisSource, Event, InputEvent, KeyboardKeyEvent, PointerAxisEvent,
    PointerButtonEvent, TouchEvent,
};
use smithay::backend::winit;
use smithay::input::keyboard::FilterResult;
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::input::touch::{DownEvent, MotionEvent as TouchMotionEvent, UpEvent};
use smithay::utils::{Logical, Point, Serial, SERIAL_COUNTER};
use wayland_server::Resource;

use super::{AxiomSmithayBackendReal, WindowInteraction};

impl AxiomSmithayBackendReal {
    /// Resolve the topmost client surface under a logical coordinate, for
    /// touch focus. Mirrors the pointer focus lookup in `PointerMotionAbsolute`.
    pub(super) fn touch_focus_under(
        &self,
        x: f64,
        y: f64,
    ) -> Option<(
        wayland_server::protocol::wl_surface::WlSurface,
        Point<f64, Logical>,
    )> {
        let floating = self.floating_rects();
        let under = self
            .state
            .workspace_manager
            .read()
            .element_under(x, y, &floating);
        under.and_then(|(window_id, (sx, sy))| {
            self.state
                .window_map
                .get(&window_id)
                .and_then(|surface_id| self.state.surfaces.get(surface_id))
                .and_then(|sd| sd.surface.as_ref())
                .filter(|s| s.is_alive())
                .cloned()
                .map(|surface| (surface, Point::from((sx, sy))))
        })
    }

    /// Process a single winit input event
    pub(super) fn handle_input_event(&mut self, event: InputEvent<winit::WinitInput>) {
        match event {
            InputEvent::Keyboard { event } => {
                if let Some(keyboard) = self.state.seat.get_keyboard() {
                    let serial = SERIAL_COUNTER.next_serial();
                    let time = Event::time_msec(&event);
                    let pressed = event.state() == smithay::backend::input::KeyState::Pressed;

                    let input_manager = self.state.input_manager.clone();
                    let pending_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
                    let pending_clone = pending_actions.clone();

                    keyboard.input::<(), _>(
                        &mut self.state,
                        event.key_code(),
                        event.state(),
                        serial,
                        time,
                        |_, modifiers, handle| {
                            if pressed {
                                let syms = handle.modified_syms();
                                if let Some(keysym) = syms.first() {
                                    let key_name = xkbcommon::xkb::keysym_get_name(*keysym);

                                    let mut mod_names: Vec<String> = Vec::new();
                                    if modifiers.ctrl {
                                        mod_names.push("Ctrl".to_string());
                                    }
                                    if modifiers.alt {
                                        mod_names.push("Alt".to_string());
                                    }
                                    if modifiers.logo {
                                        mod_names.push("Super".to_string());
                                    }
                                    if modifiers.shift {
                                        mod_names.push("Shift".to_string());
                                    }

                                    let key_combo = if mod_names.is_empty() {
                                        key_name.to_lowercase()
                                    } else {
                                        format!("{}+{}", mod_names.join("+"), key_name)
                                    };

                                    let axiom_event = crate::input::InputEvent::Keyboard {
                                        key: key_combo.clone(),
                                        modifiers: mod_names,
                                        pressed: true,
                                    };

                                    let actions =
                                        input_manager.write().process_input_event(axiom_event);

                                    if !actions.is_empty() {
                                        debug!("⌨️ Global shortcut: {}", key_combo);
                                        *pending_clone.borrow_mut() = actions;
                                        return FilterResult::Intercept(());
                                    }
                                }
                            }
                            FilterResult::Forward
                        },
                    );

                    // Process any actions that were intercepted
                    let actions: Vec<_> = pending_actions.borrow_mut().drain(..).collect();
                    if !actions.is_empty() {
                        self.process_actions(actions);
                    }
                }
            }

            InputEvent::PointerMotion { event: _event } => {
                // ponytail: winit maps PointerMotionEvent to UnusedEvent and never emits
                // this variant; the delta is always 0.0. Kept for future backends that
                // send relative motion (e.g. libinput).
                let new_x = (self.state.pointer_x + 0.0).clamp(0.0, self.state.window_width as f64);
                let new_y =
                    (self.state.pointer_y + 0.0).clamp(0.0, self.state.window_height as f64);
                self.process_pointer_motion(new_x, new_y);
            }

            InputEvent::PointerMotionAbsolute { event } => {
                self.process_pointer_motion(event.x(), event.y());
            }

            InputEvent::PointerButton { event } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);

                // Dismiss active popup grab on any button press outside the popup
                if let Some(popup_id) = self.state.active_popup_grab {
                    let dismiss = if let Some(p) = self.state.popups.get(&popup_id) {
                        let px = self.state.pointer_x as i32;
                        let py = self.state.pointer_y as i32;
                        // Find the popup's absolute position by locating its parent window
                        let (abs_x, abs_y) = self
                            .state
                            .window_map
                            .iter()
                            .find_map(|(&wid, &sid)| {
                                if sid == p.parent_surface_id {
                                    self.state
                                        .window_manager
                                        .read()
                                        .get_window(wid)
                                        .map(|w| (w.window.position.0, w.window.position.1))
                                } else {
                                    None
                                }
                            })
                            .unwrap_or((0, 0));
                        let popup_x = abs_x + p.x;
                        let popup_y = abs_y + p.y;
                        px < popup_x
                            || px > popup_x + p.width
                            || py < popup_y
                            || py > popup_y + p.height
                    } else {
                        true
                    };

                    if dismiss {
                        if let Some(p) = self.state.popups.remove(&popup_id) {
                            info!("🗑️ Dismissing popup surface {}", popup_id);
                            p.surface.send_popup_done();
                            self.state.needs_redraw = true;
                        }
                        self.state.active_popup_grab = None;
                    }
                }

                let pressed = event.state() == smithay::backend::input::ButtonState::Pressed;

                // Decoration hit-testing: close/minimize/maximize buttons
                // on server-side decorations.
                if pressed {
                    if self.handle_decoration_button(
                        self.state.pointer_x,
                        self.state.pointer_y,
                        true,
                    ) {
                        // handle_decoration_button already set decoration_consumed_press = true
                        // on a hit; keep it so the matching release is swallowed below.
                        return;
                    }
                } else if self.decoration_consumed_press {
                    self.handle_decoration_button(
                        self.state.pointer_x,
                        self.state.pointer_y,
                        false,
                    );
                    self.decoration_consumed_press = false;
                    return;
                }

                if let Some(pointer) = self.state.seat.get_pointer() {
                    // Convert MouseButton to u32 button code
                    let button_code = match event.button() {
                        Some(smithay::backend::input::MouseButton::Left) => 0x110,
                        Some(smithay::backend::input::MouseButton::Right) => 0x111,
                        Some(smithay::backend::input::MouseButton::Middle) => 0x112,
                        None => 0,
                        _ => 0,
                    };
                    let button_event = ButtonEvent {
                        serial,
                        time,
                        button: button_code,
                        state: event.state(),
                    };
                    pointer.button(&mut self.state, &button_event);
                }
            }

            InputEvent::PointerAxis { event } => {
                // Forward axis/scroll events via seat with actual axis values
                let time = Event::time_msec(&event);

                if let Some(pointer) = self.state.seat.get_pointer() {
                    let mut axis_frame = AxisFrame::new(time);

                    // Extract and forward horizontal/vertical scroll amounts
                    // Using the `input` crate's Axis enum (Horizontal/Vertical)
                    if let Some(amount) = event.amount(Axis::Horizontal) {
                        if amount.abs() > 0.0 {
                            axis_frame = axis_frame.value(Axis::Horizontal, amount);
                        }
                    }
                    if let Some(amount) = event.amount(Axis::Vertical) {
                        if amount.abs() > 0.0 {
                            axis_frame = axis_frame.value(Axis::Vertical, amount);
                        }
                    }

                    pointer.axis(&mut self.state, axis_frame);
                    pointer.frame(&mut self.state);

                    // Workspace navigation via scroll.
                    // Smooth scroll sources (touchpad) feed velocity into momentum physics;
                    // discrete sources (mouse wheel) snap to adjacent columns.
                    let source = event.source();
                    match source {
                        AxisSource::Continuous | AxisSource::Finger => {
                            if let Some(amount) = event.amount(Axis::Horizontal) {
                                let speed = self.state.config.workspace.scroll_speed;
                                let velocity = amount * speed * 8.0;
                                if velocity.abs() > 0.0 {
                                    self.state
                                        .workspace_manager
                                        .write()
                                        .start_momentum_scroll(velocity);
                                    self.state.needs_redraw = true;
                                }
                            }
                        }
                        AxisSource::Wheel | AxisSource::WheelTilt => {
                            if let Some(amount) = event.amount(Axis::Horizontal) {
                                if amount > 5.0 {
                                    self.state.workspace_manager.write().scroll_right();
                                    self.state.needs_redraw = true;
                                } else if amount < -5.0 {
                                    self.state.workspace_manager.write().scroll_left();
                                    self.state.needs_redraw = true;
                                }
                            }
                        }
                    }
                }
            }

            InputEvent::TouchDown { event } => {
                let width = self.state.window_width as i32;
                let height = self.state.window_height as i32;
                let (x, y) = (event.x_transformed(width), event.y_transformed(height));
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time_msec();
                // Record for tap-to-click detection
                self.touch_tap_state = Some((x, y, time));

                // Check for decoration button hits before forwarding to client.
                let floating = self.floating_rects();
                let under = self
                    .state
                    .workspace_manager
                    .read()
                    .element_under(x, y, &floating);
                if let Some((window_id, _)) = under {
                    // Compute window-relative coordinates for decoration hit-testing
                    let rel = self
                        .state
                        .window_manager
                        .read()
                        .get_window(window_id)
                        .map(|w| {
                            let rx = (x - w.window.position.0 as f64) as i32;
                            let ry = (y - w.window.position.1 as f64) as i32;
                            (rx, ry)
                        });
                    if let Some((rx, ry)) = rel {
                        let action = self
                            .state
                            .decoration_manager
                            .write()
                            .handle_button_press(window_id, rx, ry);
                        match action {
                            Some(crate::decoration::DecorationAction::Close) => {
                                if let Some(&surface_id) = self.state.window_map.get(&window_id) {
                                    self.state.destroy_window(surface_id);
                                    self.state.needs_redraw = true;
                                }
                                return;
                            }
                            Some(crate::decoration::DecorationAction::Minimize) => {
                                let is_minimized =
                                    self.state.window_manager.read().is_minimized(window_id);
                                if is_minimized {
                                    self.state
                                        .workspace_manager
                                        .write()
                                        .restore_window(window_id);
                                    self.state.window_manager.write().restore_window(window_id);
                                } else {
                                    self.state
                                        .workspace_manager
                                        .write()
                                        .minimize_window(window_id);
                                    self.state.window_manager.write().minimize_window(window_id);
                                }
                                self.state.needs_redraw = true;
                                return;
                            }
                            Some(crate::decoration::DecorationAction::ToggleMaximize) => {
                                self.state
                                    .window_manager
                                    .write()
                                    .toggle_fullscreen(window_id);
                                self.state.needs_redraw = true;
                                return;
                            }
                            Some(crate::decoration::DecorationAction::StartMove) => {
                                self.state
                                    .workspace_manager
                                    .write()
                                    .set_window_floating(window_id, true);
                                let wm = self.state.window_manager.read();
                                if let Some(w) = wm.get_window(window_id) {
                                    let offset_x = x - w.window.position.0 as f64;
                                    let offset_y = y - w.window.position.1 as f64;
                                    self.touch_interaction = Some(WindowInteraction::Move {
                                        window_id,
                                        offset_x,
                                        offset_y,
                                    });
                                }
                                self.state.needs_redraw = true;
                                return;
                            }
                            Some(crate::decoration::DecorationAction::StartResize(edge)) => {
                                self.state
                                    .workspace_manager
                                    .write()
                                    .set_window_floating(window_id, true);
                                let wm = self.state.window_manager.read();
                                if let Some(w) = wm.get_window(window_id) {
                                    self.touch_interaction = Some(WindowInteraction::Resize {
                                        window_id,
                                        edge,
                                        initial_rect: (
                                            w.window.position.0,
                                            w.window.position.1,
                                            w.window.size.0,
                                            w.window.size.1,
                                        ),
                                        start_x: x,
                                        start_y: y,
                                    });
                                }
                                self.state.needs_redraw = true;
                                return;
                            }
                            None => {} // No decoration hit, fall through to client
                        }
                    }
                }

                // Check for popup dismiss: if there's an active popup grab and
                // the touch is outside the popup rect, dismiss the popup.
                if let Some(popup_id) = self.state.active_popup_grab {
                    let dismiss = if let Some(p) = self.state.popups.get(&popup_id) {
                        let px = x as i32;
                        let py = y as i32;
                        // Find the popup's absolute position by locating its parent window
                        let (abs_x, abs_y) = self
                            .state
                            .window_map
                            .iter()
                            .find_map(|(&wid, &sid)| {
                                if sid == p.parent_surface_id {
                                    self.state
                                        .window_manager
                                        .read()
                                        .get_window(wid)
                                        .map(|w| (w.window.position.0, w.window.position.1))
                                } else {
                                    None
                                }
                            })
                            .unwrap_or((0, 0));
                        let popup_x = abs_x + p.x;
                        let popup_y = abs_y + p.y;
                        px < popup_x
                            || px > popup_x + p.width
                            || py < popup_y
                            || py > popup_y + p.height
                    } else {
                        true
                    };

                    if dismiss {
                        if let Some(p) = self.state.popups.remove(&popup_id) {
                            info!("🗑️ Dismissing popup surface {}", popup_id);
                            p.surface.send_popup_done();
                            self.state.needs_redraw = true;
                        }
                        self.state.active_popup_grab = None;
                        return;
                    }
                }

                // No decoration consumed — forward to the touch client.
                let focus = self.touch_focus_under(x, y);
                let down_event = DownEvent {
                    slot: event.slot(),
                    location: Point::from((x, y)),
                    serial,
                    time,
                };
                let Some(touch_handle) = self.state.seat.get_touch() else {
                    return;
                };
                touch_handle.down(&mut self.state, focus, &down_event);
                touch_handle.frame(&mut self.state);
            }

            InputEvent::TouchMotion { event } => {
                let width = self.state.window_width as i32;
                let height = self.state.window_height as i32;
                let (x, y) = (event.x_transformed(width), event.y_transformed(height));

                // If a touch-based move/resize is active, handle it and skip
                // forwarding to the client.
                if let Some(ref interaction) = self.touch_interaction.clone() {
                    self.handle_interaction(interaction, x, y);
                    return;
                }

                let time = event.time_msec();
                let focus = self.touch_focus_under(x, y);
                let motion_event = TouchMotionEvent {
                    slot: event.slot(),
                    location: Point::from((x, y)),
                    time,
                };
                let Some(touch_handle) = self.state.seat.get_touch() else {
                    return;
                };
                touch_handle.motion(&mut self.state, focus, &motion_event);
                touch_handle.frame(&mut self.state);
            }

            InputEvent::TouchUp { event } => {
                // If a touch-based move/resize was active, end it and skip
                // forwarding to the client.
                if self.touch_interaction.take().is_some() {
                    self.state.needs_redraw = true;
                    return;
                }

                // Tap-to-click: if the touch was a quick tap (short duration,
                // minimal movement) and no decoration consumed it, synthesize
                // a pointer left-click at the recorded touch-down position.
                let time = event.time_msec();
                let is_tap = self
                    .touch_tap_state
                    .map(|(_, _, tt)| time.saturating_sub(tt) < 400)
                    .unwrap_or(false);
                if is_tap {
                    let (tx, ty) = self
                        .touch_tap_state
                        .map(|(x, y, _)| (x, y))
                        .unwrap_or((self.state.pointer_x, self.state.pointer_y));
                    self.touch_tap_state = None;
                    self.state.pointer_x = tx;
                    self.state.pointer_y = ty;
                    if let Some(pointer) = self.state.seat.get_pointer() {
                        let serial = SERIAL_COUNTER.next_serial();
                        let press = smithay::backend::input::ButtonState::Pressed;
                        let release = smithay::backend::input::ButtonState::Released;
                        pointer.button(
                            &mut self.state,
                            &ButtonEvent {
                                serial,
                                time,
                                button: 0x110,
                                state: press,
                            },
                        );
                        let serial = SERIAL_COUNTER.next_serial();
                        pointer.button(
                            &mut self.state,
                            &ButtonEvent {
                                serial,
                                time,
                                button: 0x110,
                                state: release,
                            },
                        );
                    }
                    return;
                }

                let up_event = UpEvent {
                    slot: event.slot(),
                    serial: SERIAL_COUNTER.next_serial(),
                    time: event.time_msec(),
                };
                let Some(touch_handle) = self.state.seat.get_touch() else {
                    return;
                };
                touch_handle.up(&mut self.state, &up_event);
                touch_handle.frame(&mut self.state);
            }

            InputEvent::TouchCancel { event: _event } => {
                self.touch_interaction = None;
                self.touch_tap_state = None;
                let Some(touch_handle) = self.state.seat.get_touch() else {
                    return;
                };
                touch_handle.cancel(&mut self.state);
            }

            _ => {}
        }
    }

    /// If an interactive window manipulation is active (move or resize),
    /// apply the new pointer position and return `true` so the motion
    /// event is NOT forwarded to Smithay for pointer focus updates.
    pub(super) fn handle_interaction(
        &mut self,
        interaction: &WindowInteraction,
        px: f64,
        py: f64,
    ) -> bool {
        let interaction = interaction.clone();
        match interaction {
            WindowInteraction::Move {
                window_id,
                offset_x,
                offset_y,
            } => {
                let new_x = (px - offset_x).round() as i32;
                let new_y = (py - offset_y).round() as i32;
                let mut wm = self.state.window_manager.write();
                if let Some(w) = wm.get_window_mut(window_id) {
                    w.window.set_position(new_x, new_y);
                    self.state.needs_redraw = true;
                }
            }
            WindowInteraction::Resize {
                window_id,
                edge,
                initial_rect: (ix, iy, iw, ih),
                start_x,
                start_y,
            } => {
                let dx = (px - start_x) as i32;
                let dy = (py - start_y) as i32;
                let mut wm = self.state.window_manager.write();
                if let Some(w) = wm.get_window_mut(window_id) {
                    use crate::decoration::ResizeEdge;
                    let (new_x, new_y, new_w, new_h) = match edge {
                        ResizeEdge::Right => (ix, iy, (iw as i32 + dx).max(100) as u32, ih),
                        ResizeEdge::Bottom => (ix, iy, iw, (ih as i32 + dy).max(100) as u32),
                        ResizeEdge::BottomRight => {
                            let w = (iw as i32 + dx).max(100) as u32;
                            let h = (ih as i32 + dy).max(100) as u32;
                            (ix, iy, w, h)
                        }
                        ResizeEdge::Left => {
                            let w = (iw as i32 - dx).max(100) as u32;
                            let x = ix + (iw as i32 - w as i32);
                            (x, iy, w, ih)
                        }
                        ResizeEdge::Top => {
                            let h = (ih as i32 - dy).max(100) as u32;
                            let y = iy + (ih as i32 - h as i32);
                            (ix, y, iw, h)
                        }
                        ResizeEdge::TopLeft => {
                            let w = (iw as i32 - dx).max(100) as u32;
                            let h = (ih as i32 - dy).max(100) as u32;
                            let x = ix + (iw as i32 - w as i32);
                            let y = iy + (ih as i32 - h as i32);
                            (x, y, w, h)
                        }
                        ResizeEdge::TopRight => {
                            let w = (iw as i32 + dx).max(100) as u32;
                            let h = (ih as i32 - dy).max(100) as u32;
                            let y = iy + (ih as i32 - h as i32);
                            (ix, y, w, h)
                        }
                        ResizeEdge::BottomLeft => {
                            let w = (iw as i32 - dx).max(100) as u32;
                            let h = (ih as i32 + dy).max(100) as u32;
                            let x = ix + (iw as i32 - w as i32);
                            (x, iy, w, h)
                        }
                    };
                    w.window.position = (new_x, new_y);
                    w.window.size = (new_w, new_h);
                    self.state.needs_redraw = true;
                }
            }
        }
        true
    }

    /// Process pointer motion to a given (x, y) position.
    /// Shared by PointerMotionAbsolute and PointerMotion handlers.
    fn process_pointer_motion(&mut self, x: f64, y: f64) {
        self.state.pointer_x = x;
        self.state.pointer_y = y;

        // Interactive move/resize consumes the motion event.
        if let Some(ref interaction) = self.interaction.clone() {
            if self.handle_interaction(interaction, x, y) {
                return;
            }
        }

        let serial = SERIAL_COUNTER.next_serial();
        let time = 0; // time is not available in the relative motion path

        // Find the surface under the pointer and forward motion
        let floating = self.floating_rects();
        let under = self
            .state
            .workspace_manager
            .read()
            .element_under(x, y, &floating);
        self.maybe_focus_window_under_pointer(under, serial);

        if let Some(pointer) = self.state.seat.get_pointer() {
            let focus = under.and_then(|(window_id, (sx, sy))| {
                self.state
                    .window_map
                    .get(&window_id)
                    .and_then(|surface_id| {
                        self.state.surfaces.get(surface_id).and_then(|sd| {
                            sd.surface.as_ref().and_then(|s| {
                                if s.is_alive() {
                                    Some(s.clone())
                                } else {
                                    None
                                }
                            })
                        })
                    })
                    .map(|surface| (surface, Point::from((sx, sy))))
            });

            let motion_event = MotionEvent {
                serial,
                time,
                location: Point::from((x, y)),
            };
            pointer.motion(&mut self.state, focus, &motion_event);
        }
    }

    /// Build a list of floating window rects for pointer hit-testing.
    /// Each entry is `(window_id, x, y, width, height)`. Called on every
    /// motion and button event so `element_under` can find floating windows.
    fn floating_rects(&self) -> Vec<(u64, i32, i32, u32, u32)> {
        self.state.cached_floating_rects.clone()
    }

    /// If configured, move keyboard focus to the window under the pointer.
    /// This keeps live backend focus behavior aligned with `window.focus_follows_mouse`.
    fn maybe_focus_window_under_pointer(
        &mut self,
        under: Option<(u64, (f64, f64))>,
        serial: Serial,
    ) {
        if !self.state.config.window.focus_follows_mouse {
            return;
        }

        let target_window_id = under.map(|(window_id, _)| window_id);
        if self.state.window_manager.read().focused_window_id() == target_window_id {
            return;
        }

        let target_surface = target_window_id.and_then(|window_id| {
            self.state
                .window_map
                .get(&window_id)
                .and_then(|surface_id| self.state.surfaces.get(surface_id))
                .and_then(|sd| sd.surface.as_ref())
                .filter(|surface| surface.is_alive())
                .cloned()
        });

        if let Some(keyboard) = self.state.seat.get_keyboard() {
            keyboard.set_focus(&mut self.state, target_surface, serial);
        }
    }

    /// Decoration hit-testing for pointer button events. Returns `true` if
    /// the button press was consumed by a decoration (close/minimize/etc.),
    /// in which case the caller should **not** forward the event to Smithay's
    /// `PointerHandle::button`. On release the decoration pressed states are
    /// cleared regardless, but the `decoration_consumed_press` flag is also
    /// consulted to decide whether to forward the release to Smithay.
    fn handle_decoration_button(&mut self, pointer_x: f64, pointer_y: f64, pressed: bool) -> bool {
        if pressed {
            // Find the window under the cursor.
            let floating = self.floating_rects();
            let under = self
                .state
                .workspace_manager
                .read()
                .element_under(pointer_x, pointer_y, &floating);
            let Some((window_id, _)) = under else {
                return false;
            };
            // Compute window-relative coordinates for decoration hit-testing.
            let rel = self
                .state
                .window_manager
                .read()
                .get_window(window_id)
                .map(|w| {
                    let rx = (pointer_x - w.window.position.0 as f64) as i32;
                    let ry = (pointer_y - w.window.position.1 as f64) as i32;
                    (rx, ry)
                });
            let Some((rx, ry)) = rel else {
                return false;
            };
            let action = self
                .state
                .decoration_manager
                .write()
                .handle_button_press(window_id, rx, ry);
            match action {
                Some(crate::decoration::DecorationAction::Close) => {
                    if let Some(&surface_id) = self.state.window_map.get(&window_id) {
                        self.state.destroy_window(surface_id);
                        self.state.needs_redraw = true;
                    }
                    self.decoration_consumed_press = true;
                    return true;
                }
                Some(crate::decoration::DecorationAction::Minimize) => {
                    let is_minimized = self.state.window_manager.read().is_minimized(window_id);
                    if is_minimized {
                        self.state
                            .workspace_manager
                            .write()
                            .restore_window(window_id);
                        self.state.window_manager.write().restore_window(window_id);
                    } else {
                        self.state
                            .workspace_manager
                            .write()
                            .minimize_window(window_id);
                        self.state.window_manager.write().minimize_window(window_id);
                    }
                    self.state.needs_redraw = true;
                    self.decoration_consumed_press = true;
                    return true;
                }
                Some(crate::decoration::DecorationAction::ToggleMaximize) => {
                    self.state
                        .window_manager
                        .write()
                        .toggle_fullscreen(window_id);
                    self.state.needs_redraw = true;
                    self.decoration_consumed_press = true;
                    return true;
                }
                Some(crate::decoration::DecorationAction::StartMove) => {
                    // Enter interactive move mode: set the window as floating,
                    // record the pointer offset and enter grab-like state.
                    self.state
                        .workspace_manager
                        .write()
                        .set_window_floating(window_id, true);
                    let wm = self.state.window_manager.read();
                    if let Some(w) = wm.get_window(window_id) {
                        let offset_x = pointer_x - w.window.position.0 as f64;
                        let offset_y = pointer_y - w.window.position.1 as f64;
                        self.interaction = Some(WindowInteraction::Move {
                            window_id,
                            offset_x,
                            offset_y,
                        });
                    }
                    self.state.needs_redraw = true;
                    self.decoration_consumed_press = true;
                    return true;
                }
                Some(crate::decoration::DecorationAction::StartResize(edge)) => {
                    // Enter interactive resize mode. Set the window as
                    // floating so the layout system doesn't overwrite the
                    // custom size each frame.
                    self.state
                        .workspace_manager
                        .write()
                        .set_window_floating(window_id, true);
                    let wm = self.state.window_manager.read();
                    if let Some(w) = wm.get_window(window_id) {
                        let (ix, iy) = w.window.position;
                        let (iw, ih) = w.window.size;
                        self.interaction = Some(WindowInteraction::Resize {
                            window_id,
                            edge,
                            initial_rect: (ix, iy, iw, ih),
                            start_x: pointer_x,
                            start_y: pointer_y,
                        });
                    }
                    self.state.needs_redraw = true;
                    self.decoration_consumed_press = true;
                    return true;
                }
                None => {}
            }
            // If no decoration button matched, check for edge-resize on tiled
            // / floating windows. A click within `RESIZE_HANDLE` pixels of the
            // window's right or bottom edge starts a resize (bottom-right
            // corner is the most natural resize affordance).
            {
                let resize_handle = (8.0 * self.state.focused_output_scale()) as i32;
                let (window_id, _) = match under {
                    Some(t) => t,
                    None => return false,
                };
                // Compute window-relative coordinates for edge hit-testing.
                let rel = self
                    .state
                    .window_manager
                    .read()
                    .get_window(window_id)
                    .map(|w| {
                        let rx = (pointer_x - w.window.position.0 as f64) as i32;
                        let ry = (pointer_y - w.window.position.1 as f64) as i32;
                        (rx, ry, w.window.size.0 as i32, w.window.size.1 as i32)
                    });
                let Some((rx, ry, ww, wh)) = rel else {
                    return false;
                };
                use crate::decoration::ResizeEdge;
                let in_right = rx >= ww - resize_handle;
                let in_bottom = ry >= wh - resize_handle;
                let in_left = rx <= resize_handle;
                let in_top = ry <= resize_handle;
                let edge = if in_left && in_top {
                    Some(ResizeEdge::TopLeft)
                } else if in_right && in_top {
                    Some(ResizeEdge::TopRight)
                } else if in_left && in_bottom {
                    Some(ResizeEdge::BottomLeft)
                } else if in_right && in_bottom {
                    Some(ResizeEdge::BottomRight)
                } else if in_left {
                    Some(ResizeEdge::Left)
                } else if in_right {
                    Some(ResizeEdge::Right)
                } else if in_top {
                    Some(ResizeEdge::Top)
                } else if in_bottom {
                    Some(ResizeEdge::Bottom)
                } else {
                    None
                };
                if let Some(edge) = edge {
                    // Set as floating so the layout doesn't overwrite size.
                    self.state
                        .workspace_manager
                        .write()
                        .set_window_floating(window_id, true);
                    let (ix, iy) = (pointer_x - rx as f64, pointer_y - ry as f64);
                    let (ix, iy) = (ix as i32, iy as i32);
                    let (iw, ih) = (ww as u32, wh as u32);
                    self.interaction = Some(WindowInteraction::Resize {
                        window_id,
                        edge,
                        initial_rect: (ix, iy, iw, ih),
                        start_x: pointer_x,
                        start_y: pointer_y,
                    });
                    self.state.needs_redraw = true;
                    self.decoration_consumed_press = true;
                    return true;
                }
            }
            false
        } else {
            // Release: clear decoration visual state AND stop any interaction.
            let floating = self.floating_rects();
            let under = self
                .state
                .workspace_manager
                .read()
                .element_under(pointer_x, pointer_y, &floating);
            if let Some((window_id, _)) = under {
                let rel = self
                    .state
                    .window_manager
                    .read()
                    .get_window(window_id)
                    .map(|w| {
                        let rx = (pointer_x - w.window.position.0 as f64) as i32;
                        let ry = (pointer_y - w.window.position.1 as f64) as i32;
                        (rx, ry)
                    });
                if let Some((rx, ry)) = rel {
                    self.state
                        .decoration_manager
                        .write()
                        .handle_button_release(window_id, rx, ry);
                }
            }
            // If an interactive move/resize was in progress, finalize it.
            if let Some(interaction) = self.interaction.take() {
                // For resize, send a configure event so the client resizes
                // its buffer to match the new dimensions.
                if let WindowInteraction::Resize { window_id, .. } = interaction {
                    if let Some(&surface_id) = self.state.window_map.get(&window_id) {
                        if let Some(toplevel) = self.state.toplevels.get(&surface_id) {
                            let size = self
                                .state
                                .window_manager
                                .read()
                                .get_window(window_id)
                                .map(|w| w.window.size);
                            if let Some((new_w, new_h)) = size {
                                // Convert physical-pixel window size to
                                // logical pixels for the configure event,
                                // matching the tiling reconfigure path.
                                let scale = self
                                    .state
                                    .workspace_manager
                                    .read()
                                    .scale_factor_for_window(window_id);
                                let logical_w = ((new_w as f64 / scale).round() as i32).max(1);
                                let logical_h = ((new_h as f64 / scale).round() as i32).max(1);
                                toplevel.with_pending_state(|state| {
                                    state.size = Some((logical_w, logical_h).into());
                                });
                                toplevel.send_configure();
                                self.state
                                    .configured_sizes
                                    .insert(surface_id, (logical_w, logical_h));
                            }
                        }
                    }
                }
                self.decoration_consumed_press = true;
                return true;
            }
            // Consume the release if the press was also consumed, so
            // Wayland clients never see an unmatched button-release.
            self.decoration_consumed_press
        }
    }

    /// Process actions generated by InputManager
    fn process_actions(&mut self, actions: Vec<crate::input::CompositorAction>) {
        use crate::input::CompositorAction;
        for action in actions {
            match action {
                CompositorAction::ScrollWorkspaceLeft => {
                    info!("⬅️  Input: Scroll workspace left");
                    self.state.workspace_manager.write().scroll_left();
                    self.state.needs_redraw = true;
                }
                CompositorAction::ScrollWorkspaceRight => {
                    info!("➡️  Input: Scroll workspace right");
                    self.state.workspace_manager.write().scroll_right();
                    self.state.needs_redraw = true;
                }
                CompositorAction::Quit => {
                    info!("💼 Input: Quit compositor");
                    self.state.running = false;
                }
                CompositorAction::CloseWindow => {
                    let focused_id = self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        info!("🗑️  Input: Close window {}", window_id);
                        if let Some(&surface_id) = self.state.window_map.get(&window_id) {
                            self.state.destroy_window(surface_id);
                            self.state.needs_redraw = true;
                        }
                    }
                }
                CompositorAction::ToggleFullscreen => {
                    let focused_id = self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        self.state.toggle_fullscreen_window(window_id);
                    }
                }
                CompositorAction::MoveWindowLeft => {
                    let focused_id = self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        self.state
                            .workspace_manager
                            .write()
                            .move_window_left(window_id);
                        self.state.needs_redraw = true;
                    }
                }
                CompositorAction::ToggleFloating => {
                    let focused_id = self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        self.state.window_manager.write().toggle_floating(window_id);
                        self.state
                            .workspace_manager
                            .write()
                            .toggle_window_floating(window_id);
                        self.state.needs_redraw = true;
                    }
                }
                CompositorAction::ToggleMinimize => {
                    let focused_id = self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        let is_minimized = self.state.window_manager.read().is_minimized(window_id);
                        if is_minimized {
                            self.state
                                .workspace_manager
                                .write()
                                .restore_window(window_id);
                            self.state.window_manager.write().restore_window(window_id);
                            info!("🔲 Input: Restored window {}", window_id);
                        } else {
                            self.state
                                .workspace_manager
                                .write()
                                .minimize_window(window_id);
                            self.state.window_manager.write().minimize_window(window_id);
                            info!("🔳 Input: Minimized window {}", window_id);
                        }
                        self.state.needs_redraw = true;
                    }
                }
                CompositorAction::MoveWindowRight => {
                    let focused_id = self.state.window_manager.read().focused_window_id();
                    if let Some(window_id) = focused_id {
                        self.state
                            .workspace_manager
                            .write()
                            .move_window_right(window_id);
                        self.state.needs_redraw = true;
                    }
                }
                CompositorAction::LaunchTerminal => {
                    let cmd = &self.state.config.general.default_terminal;
                    let _ = std::process::Command::new(cmd)
                        .spawn()
                        .map(|_| debug!("🚀 Launched terminal: {}", cmd))
                        .map_err(|e| warn!("Failed to launch terminal '{}': {}", cmd, e));
                }
                CompositorAction::LaunchLauncher => {
                    let cmd = &self.state.config.general.default_launcher;
                    let _ = std::process::Command::new(cmd)
                        .spawn()
                        .map(|_| debug!("🚀 Launched launcher: {}", cmd))
                        .map_err(|e| warn!("Failed to launch launcher '{}': {}", cmd, e));
                }
                CompositorAction::FocusNextOutput => {
                    self.state.workspace_manager.write().focus_next_output();
                    self.state.needs_redraw = true;
                    info!("📺 Input: Focus next output");
                }
            }
        }
    }
}
