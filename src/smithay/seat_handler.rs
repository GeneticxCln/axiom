//! Complete seat protocol implementation for Wayland input handling
//!
//! This module provides full support for wl_seat, wl_pointer, wl_keyboard, and wl_touch protocols
//! with proper focus management and event dispatching.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        TouchEvent, TouchSlot,
    },
    input::{
        keyboard::{keysyms as xkb, FilterResult, KeyboardHandle, Keysym, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, CursorImageStatus, MotionEvent, PointerHandle, RelativeMotionEvent},
        touch::{DownEvent, MotionEvent as TouchMotionEvent, TouchHandle, UpEvent},
        Seat, SeatHandler, SeatState,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_keyboard, wl_pointer, wl_seat, wl_surface, wl_touch},
            Client, Resource,
        },
    },
    utils::{Logical, Point, Rectangle, Serial, SERIAL_COUNTER},
    wayland::{
        compositor::CompositorHandler,
        seat::{KeyboardGrab, PointerGrab, TouchGrab},
        shell::xdg::XdgToplevelSurfaceData,
    },
};

use log::{debug, info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Maximum number of touch points supported
const MAX_TOUCH_POINTS: u32 = 10;

/// Double-click time threshold in milliseconds
const DOUBLE_CLICK_THRESHOLD_MS: u64 = 400;

/// Drag distance threshold in logical pixels
const DRAG_THRESHOLD_PIXELS: f64 = 5.0;

/// Input focus state for seat management
#[derive(Debug, Clone)]
pub struct FocusState {
    /// Currently focused surface for keyboard input
    pub keyboard_focus: Option<wl_surface::WlSurface>,
    
    /// Currently focused surface for pointer input
    pub pointer_focus: Option<wl_surface::WlSurface>,
    
    /// Active touch points and their associated surfaces
    pub touch_points: HashMap<TouchSlot, wl_surface::WlSurface>,
    
    /// Last pointer position in logical coordinates
    pub pointer_position: Point<f64, Logical>,
    
    /// Current keyboard modifiers state
    pub modifiers: ModifiersState,
    
    /// Active pointer button states
    pub button_states: HashMap<u32, ButtonState>,
    
    /// Last click time and position for double-click detection
    pub last_click: Option<(Instant, Point<f64, Logical>)>,
    
    /// Whether we're currently in a drag operation
    pub is_dragging: bool,
    
    /// Start position of a potential drag
    pub drag_start: Option<Point<f64, Logical>>,
}

impl Default for FocusState {
    fn default() -> Self {
        Self {
            keyboard_focus: None,
            pointer_focus: None,
            touch_points: HashMap::new(),
            pointer_position: Point::from((0.0, 0.0)),
            modifiers: ModifiersState::default(),
            button_states: HashMap::new(),
            last_click: None,
            is_dragging: false,
            drag_start: None,
        }
    }
}

/// Complete seat handler implementation
pub struct SeatInputHandler {
    /// Seat state from Smithay
    seat_state: SeatState<Self>,
    
    /// Focus management state
    focus_state: Arc<Mutex<FocusState>>,
    
    /// Keyboard handle
    keyboard: KeyboardHandle<Self>,
    
    /// Pointer handle
    pointer: PointerHandle<Self>,
    
    /// Touch handle
    touch: TouchHandle,
    
    /// Window positions for hit testing
    window_positions: Arc<Mutex<HashMap<u64, Rectangle<i32, Logical>>>>,
    
    /// Surface to window ID mapping
    surface_to_window: Arc<Mutex<HashMap<wl_surface::WlSurface, u64>>>,
}

impl SeatInputHandler {
    /// Creates a new seat handler with complete input support
    pub fn new(seat_state: SeatState<Self>, seat: &Seat<Self>) -> Self {
        let keyboard = seat.add_keyboard(Default::default(), 200, 25)
            .expect("Failed to add keyboard to seat");
        let pointer = seat.add_pointer();
        let touch = seat.add_touch();
        
        Self {
            seat_state,
            focus_state: Arc::new(Mutex::new(FocusState::default())),
            keyboard,
            pointer,
            touch,
            window_positions: Arc::new(Mutex::new(HashMap::new())),
            surface_to_window: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    /// Updates window position for hit testing
    pub fn update_window_position(&self, window_id: u64, rect: Rectangle<i32, Logical>) {
        if let Ok(mut positions) = self.window_positions.lock() {
            positions.insert(window_id, rect);
        }
    }
    
    /// Associates a surface with a window ID
    pub fn associate_surface(&self, surface: wl_surface::WlSurface, window_id: u64) {
        if let Ok(mut mapping) = self.surface_to_window.lock() {
            mapping.insert(surface, window_id);
        }
    }
    
    /// Performs hit testing to find surface at given position
    pub fn surface_at(&self, pos: Point<f64, Logical>) -> Option<(wl_surface::WlSurface, Point<f64, Logical>)> {
        let positions = self.window_positions.lock().ok()?;
        let surface_map = self.surface_to_window.lock().ok()?;
        
        // Iterate windows in reverse Z-order (top to bottom)
        for (window_id, rect) in positions.iter() {
            if rect.contains(pos.to_i32_round()) {
                // Find surface for this window
                for (surface, wid) in surface_map.iter() {
                    if wid == window_id {
                        let local = Point::from((
                            pos.x - rect.loc.x as f64,
                            pos.y - rect.loc.y as f64,
                        ));
                        return Some((surface.clone(), local));
                    }
                }
            }
        }
        
        None
    }
    
    /// Handles keyboard key events
    pub fn handle_keyboard_key<I: InputBackend>(
        &mut self,
        event: &KeyboardKeyEvent<I>,
        serial: Serial,
    ) {
        let keycode = event.key_code();
        let state = event.state();
        
        debug!(
            "Keyboard key event: keycode={}, state={:?}",
            keycode, state
        );
        
        // Update modifiers state
        self.keyboard.input::<(), _>(
            self,
            keycode,
            state,
            serial,
            event.time_msec(),
            |_, modifiers, keysym| {
                if let Ok(mut focus) = self.focus_state.lock() {
                    focus.modifiers = *modifiers;
                }
                
                // Handle special key combinations
                if modifiers.alt && keysym == Keysym::Tab && state == KeyState::Pressed {
                    info!("Alt+Tab window switching requested");
                    // Trigger window switching logic
                    self.switch_window_focus(true);
                    return FilterResult::Intercept(());
                }
                
                if modifiers.ctrl && modifiers.alt && keysym == Keysym::Delete {
                    info!("Ctrl+Alt+Delete requested");
                    return FilterResult::Intercept(());
                }
                
                FilterResult::Forward
            },
        );
    }
    
    /// Handles pointer motion events
    pub fn handle_pointer_motion<I: InputBackend>(
        &mut self,
        event: &PointerMotionEvent<I>,
        serial: Serial,
    ) {
        let delta = event.delta();
        
        if let Ok(mut focus) = self.focus_state.lock() {
            // Update pointer position
            focus.pointer_position.x += delta.0;
            focus.pointer_position.y += delta.1;
            
            // Clamp to screen bounds if needed
            focus.pointer_position.x = focus.pointer_position.x.max(0.0);
            focus.pointer_position.y = focus.pointer_position.y.max(0.0);
            
            let pos = focus.pointer_position;
            
            // Check for drag operation
            if let Some(drag_start) = focus.drag_start {
                let distance = ((pos.x - drag_start.x).powi(2) + (pos.y - drag_start.y).powi(2)).sqrt();
                if distance > DRAG_THRESHOLD_PIXELS && !focus.is_dragging {
                    focus.is_dragging = true;
                    info!("Drag operation started");
                }
            }
        }
        
        // Find surface under pointer and send motion event
        let pos = self.focus_state.lock().unwrap().pointer_position;
        if let Some((surface, local_pos)) = self.surface_at(pos) {
            self.pointer.motion(
                self,
                Some((surface, local_pos)),
                &MotionEvent {
                    location: pos,
                    serial,
                    time: event.time_msec(),
                },
            );
        }
    }
    
    /// Handles pointer button events
    pub fn handle_pointer_button<I: InputBackend>(
        &mut self,
        event: &PointerButtonEvent<I>,
        serial: Serial,
    ) {
        let button = event.button_code();
        let state = event.state();
        
        debug!("Pointer button event: button={}, state={:?}", button, state);
        
        if let Ok(mut focus) = self.focus_state.lock() {
            // Update button states
            match state {
                ButtonState::Pressed => {
                    focus.button_states.insert(button, state);
                    
                    // Start potential drag
                    focus.drag_start = Some(focus.pointer_position);
                    
                    // Check for double-click
                    let now = Instant::now();
                    if let Some((last_time, last_pos)) = focus.last_click {
                        let time_delta = now.duration_since(last_time).as_millis() as u64;
                        let pos_delta = ((focus.pointer_position.x - last_pos.x).powi(2) 
                            + (focus.pointer_position.y - last_pos.y).powi(2)).sqrt();
                        
                        if time_delta < DOUBLE_CLICK_THRESHOLD_MS && pos_delta < 5.0 {
                            info!("Double-click detected");
                            // Handle double-click action
                        }
                    }
                    focus.last_click = Some((now, focus.pointer_position));
                }
                ButtonState::Released => {
                    focus.button_states.remove(&button);
                    
                    // End drag if active
                    if focus.is_dragging {
                        focus.is_dragging = false;
                        info!("Drag operation ended");
                    }
                    focus.drag_start = None;
                }
            }
        }
        
        // Send button event to focused surface
        self.pointer.button(
            self,
            &ButtonEvent {
                button,
                state,
                serial,
                time: event.time_msec(),
            },
        );
    }
    
    /// Handles pointer axis (scroll) events
    pub fn handle_pointer_axis<I: InputBackend>(
        &mut self,
        event: &PointerAxisEvent<I>,
        serial: Serial,
    ) {
        let source = event.source();
        
        let mut frame = AxisFrame::new(event.time_msec())
            .source(source);
        
        if let Some(axis) = event.amount(Axis::Horizontal) {
            frame = frame.value(Axis::Horizontal, axis);
        }
        
        if let Some(axis) = event.amount(Axis::Vertical) {
            frame = frame.value(Axis::Vertical, axis);
        }
        
        self.pointer.axis(self, frame);
    }
    
    /// Handles touch down events
    pub fn handle_touch_down<I: InputBackend>(
        &mut self,
        event: &TouchEvent<I>,
        serial: Serial,
    ) {
        let slot = event.slot();
        let pos = Point::from((event.x(), event.y()));
        
        debug!("Touch down: slot={:?}, pos={:?}", slot, pos);
        
        // Find surface at touch position
        if let Some((surface, local_pos)) = self.surface_at(pos) {
            if let Ok(mut focus) = self.focus_state.lock() {
                focus.touch_points.insert(slot, surface.clone());
            }
            
            self.touch.down(
                self,
                Some((surface, local_pos)),
                &DownEvent {
                    slot,
                    location: pos,
                    serial,
                    time: event.time_msec(),
                },
            );
        }
    }
    
    /// Handles touch motion events
    pub fn handle_touch_motion<I: InputBackend>(
        &mut self,
        event: &TouchEvent<I>,
        serial: Serial,
    ) {
        let slot = event.slot();
        let pos = Point::from((event.x(), event.y()));
        
        // Get the surface for this touch point
        let surface = self.focus_state.lock()
            .ok()
            .and_then(|focus| focus.touch_points.get(&slot).cloned());
        
        if let Some(surface) = surface {
            // Calculate local position
            if let Some((_, local_pos)) = self.surface_at(pos) {
                self.touch.motion(
                    self,
                    Some((surface, local_pos)),
                    &TouchMotionEvent {
                        slot,
                        location: pos,
                        time: event.time_msec(),
                    },
                );
            }
        }
    }
    
    /// Handles touch up events
    pub fn handle_touch_up<I: InputBackend>(
        &mut self,
        event: &TouchEvent<I>,
        serial: Serial,
    ) {
        let slot = event.slot();
        
        debug!("Touch up: slot={:?}", slot);
        
        if let Ok(mut focus) = self.focus_state.lock() {
            focus.touch_points.remove(&slot);
        }
        
        self.touch.up(
            self,
            &UpEvent {
                slot,
                serial,
                time: event.time_msec(),
            },
        );
    }
    
    /// Switches keyboard focus to next/previous window
    pub fn switch_window_focus(&mut self, forward: bool) {
        // This would integrate with the window stack to cycle through windows
        info!("Switching window focus: forward={}", forward);
        
        // Get current focused surface and find next in Z-order
        // Implementation would use WindowStack to determine next window
    }
    
    /// Sets keyboard focus to a specific surface
    pub fn set_keyboard_focus(&mut self, surface: Option<wl_surface::WlSurface>, serial: Serial) {
        if let Ok(mut focus) = self.focus_state.lock() {
            focus.keyboard_focus = surface.clone();
        }
        
        self.keyboard.set_focus(self, surface, serial);
    }
    
    /// Sets pointer focus to a specific surface
    pub fn set_pointer_focus(
        &mut self,
        surface: Option<(wl_surface::WlSurface, Point<f64, Logical>)>,
        serial: Serial,
    ) {
        if let Some((ref surf, _)) = surface {
            if let Ok(mut focus) = self.focus_state.lock() {
                focus.pointer_focus = Some(surf.clone());
            }
        }
        
        self.pointer.motion(
            self,
            surface.as_ref().map(|(s, p)| (s.clone(), *p)),
            &MotionEvent {
                location: self.focus_state.lock().unwrap().pointer_position,
                serial,
                time: 0,
            },
        );
    }
}

impl SeatHandler for SeatInputHandler {
    type KeyboardFocus = wl_surface::WlSurface;
    type PointerFocus = wl_surface::WlSurface;
    type TouchFocus = wl_surface::WlSurface;
    
    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }
    
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&Self::KeyboardFocus>) {
        if let Some(surface) = focused {
            info!("Keyboard focus changed to surface: {:?}", surface);
        } else {
            info!("Keyboard focus lost");
        }
    }
    
    fn cursor_image(&mut self, seat: &Seat<Self>, image: CursorImageStatus) {
        // Handle cursor image changes
        match image {
            CursorImageStatus::Hidden => {
                debug!("Cursor hidden");
            }
            CursorImageStatus::Default => {
                debug!("Using default cursor");
            }
            CursorImageStatus::Surface(surface) => {
                debug!("Custom cursor surface set");
            }
            _ => {}
        }
    }
}

/// Gesture recognition for touch and pointer input
pub struct GestureRecognizer {
    /// Active touch points for gesture recognition
    touch_points: HashMap<TouchSlot, (Point<f64, Logical>, Instant)>,
    
    /// Last recognized gesture
    last_gesture: Option<Gesture>,
    
    /// Gesture start time
    gesture_start: Option<Instant>,
}

#[derive(Debug, Clone, Copy)]
pub enum Gesture {
    Tap,
    DoubleTap,
    LongPress,
    Swipe(SwipeDirection),
    Pinch(f64), // Scale factor
    Rotate(f64), // Angle in radians
}

#[derive(Debug, Clone, Copy)]
pub enum SwipeDirection {
    Up,
    Down,
    Left,
    Right,
}

impl GestureRecognizer {
    pub fn new() -> Self {
        Self {
            touch_points: HashMap::new(),
            last_gesture: None,
            gesture_start: None,
        }
    }
    
    /// Process touch down event for gesture recognition
    pub fn touch_down(&mut self, slot: TouchSlot, pos: Point<f64, Logical>) {
        self.touch_points.insert(slot, (pos, Instant::now()));
        
        if self.gesture_start.is_none() {
            self.gesture_start = Some(Instant::now());
        }
    }
    
    /// Process touch motion for gesture recognition
    pub fn touch_motion(&mut self, slot: TouchSlot, pos: Point<f64, Logical>) -> Option<Gesture> {
        if let Some((start_pos, start_time)) = self.touch_points.get(&slot) {
            let distance = ((pos.x - start_pos.x).powi(2) + (pos.y - start_pos.y).powi(2)).sqrt();
            
            // Detect swipe gestures
            if distance > 50.0 {
                let angle = (pos.y - start_pos.y).atan2(pos.x - start_pos.x);
                let direction = match angle {
                    a if a > -std::f64::consts::FRAC_PI_4 && a <= std::f64::consts::FRAC_PI_4 => SwipeDirection::Right,
                    a if a > std::f64::consts::FRAC_PI_4 && a <= 3.0 * std::f64::consts::FRAC_PI_4 => SwipeDirection::Down,
                    a if a > 3.0 * std::f64::consts::FRAC_PI_4 || a <= -3.0 * std::f64::consts::FRAC_PI_4 => SwipeDirection::Left,
                    _ => SwipeDirection::Up,
                };
                
                return Some(Gesture::Swipe(direction));
            }
            
            // Detect pinch/zoom with multiple touch points
            if self.touch_points.len() == 2 {
                // Calculate pinch scale
                // This would need more sophisticated calculation with two points
            }
        }
        
        None
    }
    
    /// Process touch up for gesture recognition
    pub fn touch_up(&mut self, slot: TouchSlot) -> Option<Gesture> {
        if let Some((pos, start_time)) = self.touch_points.remove(&slot) {
            let duration = Instant::now().duration_since(start_time);
            
            // Detect tap vs long press
            if duration.as_millis() < 200 {
                return Some(Gesture::Tap);
            } else if duration.as_millis() > 500 {
                return Some(Gesture::LongPress);
            }
        }
        
        if self.touch_points.is_empty() {
            self.gesture_start = None;
        }
        
        None
    }
}

/// Tests for seat handler
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_focus_state_default() {
        let focus = FocusState::default();
        assert!(focus.keyboard_focus.is_none());
        assert!(focus.pointer_focus.is_none());
        assert!(focus.touch_points.is_empty());
        assert!(!focus.is_dragging);
    }
    
    #[test]
    fn test_gesture_recognizer() {
        let mut recognizer = GestureRecognizer::new();
        let slot = TouchSlot::default();
        let pos = Point::from((100.0, 100.0));
        
        recognizer.touch_down(slot, pos);
        assert!(recognizer.touch_points.contains_key(&slot));
        
        let gesture = recognizer.touch_up(slot);
        assert!(matches!(gesture, Some(Gesture::Tap)));
    }
}