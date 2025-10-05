use std::sync::mpsc::Receiver;

use log::warn;

use crate::smithay::server::HwInputEvent;

pub fn init_libinput_backend() -> Option<Receiver<HwInputEvent>> {
    // Integrated libinput is handled inside the server event loop.
    // Returning None here ensures the caller may choose a fallback (evdev) if desired.
    None
}

pub fn create_libinput_context() -> Option<input::Libinput> {
    use input::{Libinput, LibinputInterface};
    use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
    use std::path::Path;

    struct Interface;
    impl LibinputInterface for Interface {
        fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
            use std::fs::OpenOptions;
            use std::os::unix::fs::OpenOptionsExt;
            match OpenOptions::new()
                .custom_flags(flags)
                .read(true)
                .write((flags & libc::O_WRONLY) != 0 || (flags & libc::O_RDWR) != 0)
                .open(path)
            {
                Ok(file) => Ok(unsafe { OwnedFd::from_raw_fd(file.into_raw_fd()) }),
                Err(e) => Err(e.raw_os_error().unwrap_or(0)),
            }
        }
        fn close_restricted(&mut self, _fd: OwnedFd) { /* OwnedFd drops */
        }
    }

    let mut li = Libinput::new_with_udev::<Interface>(Interface);
    if li.udev_assign_seat("seat0").is_err() {
        warn!("libinput: failed to assign seat — integrated backend disabled");
        return None;
    }
    Some(li)
}

pub fn drain_libinput_events(li: &mut input::Libinput) -> Vec<HwInputEvent> {
    use input::event::{
        keyboard::{self, KeyboardEventTrait},
        pointer::{self, PointerScrollEvent},
        Event,
    };
    let mut out = Vec::new();
    li.dispatch().unwrap_or_default();
    for event in li.by_ref() {
        match event {
            Event::Keyboard(keyboard::KeyboardEvent::Key(k)) => {
                let code = k.key();
                let pressed = matches!(k.key_state(), keyboard::KeyState::Pressed);
                let key_name = match code {
                    103 => Some("Up"),
                    108 => Some("Down"),
                    105 => Some("Left"),
                    106 => Some("Right"),
                    35 => Some("H"),
                    36 => Some("J"),
                    37 => Some("K"),
                    38 => Some("L"),
                    87 => Some("F11"),
                    _ => None,
                };
                if let Some(name) = key_name {
                    out.push(HwInputEvent::Key {
                        key: name.to_string(),
                        modifiers: Vec::new(),
                        pressed,
                    });
                }
            }
            Event::Pointer(pointer::PointerEvent::Motion(m)) => {
                out.push(HwInputEvent::PointerMotion {
                    dx: m.dx(),
                    dy: m.dy(),
                });
            }
            Event::Pointer(pointer::PointerEvent::Button(b)) => {
                let btn = match b.button() {
                    272 => Some(1),
                    273 => Some(2),
                    274 => Some(3),
                    _ => None,
                };
                if let Some(button) = btn {
                    let pressed = matches!(b.button_state(), pointer::ButtonState::Pressed);
                    out.push(HwInputEvent::PointerButton { button, pressed });
                }
            }
            Event::Pointer(pointer::PointerEvent::ScrollWheel(sw)) => {
                use pointer::Axis;
                let mut h = 0.0;
                let mut v = 0.0;
                if sw.has_axis(Axis::Horizontal) {
                    h = sw.scroll_value(Axis::Horizontal);
                } else if sw.has_axis(Axis::Vertical) {
                    v = sw.scroll_value(Axis::Vertical);
                }
                if h != 0.0 || v != 0.0 {
                    out.push(HwInputEvent::PointerAxis {
                        horizontal: h,
                        vertical: v,
                    });
                }
            }
            Event::Pointer(pointer::PointerEvent::ScrollFinger(sf)) => {
                use pointer::Axis;
                let mut h = 0.0;
                let mut v = 0.0;
                if sf.has_axis(Axis::Horizontal) {
                    h = sf.scroll_value(Axis::Horizontal);
                } else if sf.has_axis(Axis::Vertical) {
                    v = sf.scroll_value(Axis::Vertical);
                }
                if h != 0.0 || v != 0.0 {
                    out.push(HwInputEvent::PointerAxis {
                        horizontal: h,
                        vertical: v,
                    });
                }
            }
            Event::Pointer(pointer::PointerEvent::ScrollContinuous(sc)) => {
                use pointer::Axis;
                let mut h = 0.0;
                let mut v = 0.0;
                if sc.has_axis(Axis::Horizontal) {
                    h = sc.scroll_value(Axis::Horizontal);
                } else if sc.has_axis(Axis::Vertical) {
                    v = sc.scroll_value(Axis::Vertical);
                }
                if h != 0.0 || v != 0.0 {
                    out.push(HwInputEvent::PointerAxis {
                        horizontal: h,
                        vertical: v,
                    });
                }
            }
            _ => {}
        }
    }
    out
}
