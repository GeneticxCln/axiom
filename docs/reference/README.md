# Archived Backend Implementations

This directory contains archived reference implementations of Wayland compositor backends. These are **not used in production** but kept for educational and reference purposes.

## ⚠️ Important Notice

**The production Axiom compositor uses `src/smithay/server.rs`**

These archived backends are kept as:
- Learning references for Wayland protocol implementation
- Examples of clean, well-documented code
- Historical record of development iterations

## Files in This Directory

### `backend_real_archived.rs` (1,437 lines)

**Status:** ✅ Enhanced with modern improvements, then archived

The most complete of the experimental backends. Features:
- ✅ Full wl_keyboard protocol with XKB keymap and modifiers
- ✅ wl_pointer with frame batching and axis (scroll) support
- ✅ XDG serial tracking and validation
- ✅ Calloop event loop integration (non-blocking)
- ✅ wl_compositor, wl_shm, wl_seat, wl_output
- ✅ XDG shell (xdg_wm_base, xdg_surface, xdg_toplevel, xdg_popup)
- ✅ Subsurface support
- ⚠️ Missing: Real buffer rendering, DMABUF, layer-shell, clipboard

**Why Archived:**
- Would need 6000+ more lines to match smithay/server.rs
- Missing critical features (buffer rendering, layer-shell, clipboard)
- Smithay already has everything this has + much more
- Risk of code duplication and maintenance burden

**Use Case:**
- Reference for understanding Wayland protocols
- Teaching example for keyboard/pointer implementation
- Verification that our protocol patterns are correct

### `backend_basic_archived.rs` (243 lines)

**Status:** Basic proof-of-concept

Minimal Wayland compositor that:
- ✅ Creates wl_compositor and wl_shm globals
- ✅ Accepts client connections
- ✅ Processes surface creation and commits
- ❌ No input, no XDG shell, no rendering

**Use Case:**
- Simplest possible Wayland compositor
- Understanding Display/ListeningSocket basics
- Testing socket creation and client connection

### `backend_simple_archived.rs` (49 lines)

**Status:** Socket-only test

Absolute minimum:
- Creates Wayland display and socket
- No globals, no protocols, no functionality

**Use Case:**
- Verifying Wayland socket binding works
- Minimal example for debugging socket issues

## What We Learned

Building these backends taught us:

### 1. XKB Keyboard Implementation
```rust
// Generate keymap
let keymap = xkb::Keymap::new_from_names(&ctx, "", "", "us", "", ...);
let keymap_string = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);

// Share via memfd
let fd = memfd_create("axiom-xkb-keymap", MFD_CLOEXEC);
write(fd, keymap_string);
keyboard.keymap(WlKeyboard::KeymapFormat::XkbV1, fd, size);

// Send modifiers with every key event
keyboard.modifiers(serial, depressed, latched, locked, group);
```

### 2. Pointer Frame Batching
```rust
// Always batch pointer events with frame()
pointer.motion(time, x, y);
if version >= 5 {
    pointer.frame();  // Critical for proper batching!
}
```

### 3. XDG Serial Validation
```rust
// Track what we sent vs what client acknowledged
pub struct Window {
    last_configure_serial: Option<u32>,  // What we sent
    last_acked_serial: Option<u32>,      // What client acked
    is_configured: bool,
    // ...
}

// Validate before allowing map
if serial != window.last_configure_serial {
    warn!("Client acked unknown serial!");
    return;  // Protocol violation
}
```

### 4. Calloop Event Loop
```rust
// Event-driven, not busy-loop
let socket_source = calloop::generic::Generic::new(
    listening_socket,
    calloop::Interest::READ,
    calloop::Mode::Level,
);

loop_handle.insert_source(socket_source, |_, socket, data| {
    if let Some(stream) = socket.accept()? {
        data.display_handle.insert_client(stream, ...);
    }
    Ok(calloop::PostAction::Continue)
})?;

// No sleep(1ms) needed - event-driven!
event_loop.dispatch(Some(Duration::from_millis(10)), &mut data)?;
```

## Why Smithay/Server.rs Won

| Feature | smithay/server.rs | backend_real | Winner |
|---------|-------------------|--------------|---------|
| Lines of Code | 7,542 | 1,437 | smithay (5x more functionality) |
| SHM Rendering | ✅ Full | ❌ None | **smithay** |
| DMABUF | ✅ v4 | ❌ None | **smithay** |
| Layer Shell | ✅ Full | ❌ None | **smithay** |
| Clipboard | ✅ Full | ❌ None | **smithay** |
| Multi-output | ✅ Full | ❌ Single | **smithay** |
| Production Ready | ✅ NOW | ❌ 6+ months | **smithay** |

## Timeline

- **Early Development**: Created basic/simple backends for testing
- **Mid Development**: Built backend_real as learning exercise
- **January 2025**: Enhanced backend_real with modern improvements
- **January 11, 2025**: Analyzed and decided to consolidate on smithay
- **Status**: All experimental backends archived as reference

## Using These as References

### To Learn Keyboard Protocol:
See `backend_real_archived.rs` lines 342-395 (keymap generation and delivery)

### To Learn Pointer Protocol:
See `backend_real_archived.rs` lines 449-503 (frame batching and axis)

### To Learn XDG Serial Validation:
See `backend_real_archived.rs` lines 79-109 (role tracking and serial validation)

### To Learn Calloop Integration:
See `backend_real_archived.rs` lines 283-377 (event loop setup)

## Production Architecture

```
Axiom Compositor (Production)
├── src/smithay/server.rs        ← PRODUCTION BACKEND (7500+ lines)
│   ├── Full protocol support
│   ├── Real buffer rendering
│   ├── DMABUF, layer-shell, clipboard
│   └── Integrated with all Axiom managers
│
└── docs/reference/               ← ARCHIVED IMPLEMENTATIONS
    ├── backend_real_archived.rs  (learning reference)
    ├── backend_basic_archived.rs (minimal example)
    └── backend_simple_archived.rs (socket test)
```

## Further Reading

- Main architecture decision: `docs/ARCHITECTURE_DECISION.md`
- Session improvements: `docs/session_summary_2025-01-11.md`
- Quick changes reference: `docs/CHANGES.md`

---

**Remember:** Use `src/smithay/server.rs` for all production work.  
These archived files are references only!
