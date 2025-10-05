# Bug Report: Wayland Protocol Error - Wrong Client Objects

**Date**: October 5, 2025  
**Severity**: HIGH - Server crash on client connection  
**Status**: IDENTIFIED - Fix in progress  
**Component**: `src/smithay/server.rs` - Focus management

---

## Summary

The Axiom Wayland server crashes when a client connects and receives focus, with the error:
```
Attempting to send an event with objects from wrong client.
```

This is a critical bug that prevents basic client interaction.

---

## Reproduction Steps

1. Start the minimal Wayland server:
   ```bash
   RUST_LOG=info ./target/debug/run_minimal_wayland
   ```

2. Connect any Wayland client:
   ```bash
   export WAYLAND_DISPLAY=wayland-2
   weston-terminal
   ```

3. **Result**: Server crashes immediately when client window is created

---

## Stack Trace

```
thread 'main' panicked at wayland-backend-0.3.11/src/sys/server_impl/mod.rs:1100:29:
Attempting to send an event with objects from wrong client.

stack backtrace:
  17: axiom::smithay::server::switch_focus_surfaces_inline
             at /home/quinton/axiom/src/smithay/server.rs:6523:16
  18: axiom::smithay::server::handle_events_inline
             at /home/quinton/axiom/src/smithay/server.rs:5328:29
  19: axiom::smithay::server::CompositorServer::run::{{closure}}
             at /home/quinton/axiom/src/smithay/server.rs:1395:21
```

---

## Root Cause Analysis

### The Problem

In `switch_focus_surfaces_inline()` at line 6523:

```rust
fn switch_focus_surfaces_inline(
    state: &mut CompositorState,
    prev: Option<&wl_surface::WlSurface>,
    next: Option<&wl_surface::WlSurface>,
) {
    // ... leave previous surfaces ...
    
    if let Some(ns) = next {
        let serial = state.next_serial();
        for kb in &state.keyboards {
            kb.enter(serial, ns, vec![]);  // ❌ LINE 6523 - CRASH HERE
        }
        // ...
    }
}
```

### Why It Fails

**Wayland Protocol Requirement**: Each protocol object (like `wl_keyboard`) belongs to a specific client. You can only send events to surfaces that belong to the **same client** as the keyboard.

**What's Happening**:
1. Multiple clients can connect to the compositor
2. Each client creates its own `wl_keyboard` resource via `wl_seat.get_keyboard()`
3. The compositor stores ALL keyboards from ALL clients in `state.keyboards`
4. When focusing a surface, the code tries to send `enter` events to that surface using **every keyboard**, including keyboards from **other clients**
5. Wayland backend detects this violation and panics

**Example Scenario**:
- Client A creates keyboard_A
- Client B creates keyboard_B and surface_B
- When surface_B gets focus, code tries: `keyboard_A.enter(surface_B)` ❌
- This fails because keyboard_A belongs to Client A, but surface_B belongs to Client B

---

## The Fix

### Option 1: Filter Keyboards by Client (Recommended)

```rust
fn switch_focus_surfaces_inline(
    state: &mut CompositorState,
    prev: Option<&wl_surface::WlSurface>,
    next: Option<&wl_surface::WlSurface>,
) {
    if let Some(ps) = prev {
        let serial = state.next_serial();
        // Get client that owns this surface
        if let Some(prev_client) = ps.client() {
            // Only send leave to keyboards from the same client
            for kb in &state.keyboards {
                if kb.client().map(|c| c.id()) == Some(prev_client.id()) {
                    kb.leave(serial, ps);
                }
            }
        }
        
        let serial = state.next_serial();
        if let Some(prev_client) = ps.client() {
            for ptr in &state.pointers {
                if ptr.client().map(|c| c.id()) == Some(prev_client.id()) {
                    ptr.leave(serial, ps);
                }
            }
        }
    }
    
    if let Some(ns) = next {
        let serial = state.next_serial();
        // Get client that owns this surface
        if let Some(next_client) = ns.client() {
            // Only send enter to keyboards from the same client
            for kb in &state.keyboards {
                if kb.client().map(|c| c.id()) == Some(next_client.id()) {
                    kb.enter(serial, ns, vec![]);
                }
            }
        }
        
        let serial = state.next_serial();
        if let Some(next_client) = ns.client() {
            for ptr in &state.pointers {
                if ptr.client().map(|c| c.id()) == Some(next_client.id()) {
                    ptr.enter(serial, ns, 0.0, 0.0);
                }
            }
        }
    }
}
```

### Option 2: Store Per-Client Input Resources

Instead of storing all keyboards/pointers globally, store them per-client:

```rust
pub struct CompositorState {
    // ... existing fields ...
    
    // Replace global lists:
    // pub keyboards: Vec<wl_keyboard::WlKeyboard>,
    // pub pointers: Vec<wl_pointer::WlPointer>,
    
    // With per-client maps:
    pub client_keyboards: HashMap<ClientId, Vec<wl_keyboard::WlKeyboard>>,
    pub client_pointers: HashMap<ClientId, Vec<wl_pointer::WlPointer>>,
}
```

---

## Testing Results

### Before Fix
- ❌ Server crashes immediately on client connection
- ❌ Cannot test any window operations
- ❌ No clients can successfully map windows

### Expected After Fix
- ✅ Server accepts client connections without crashing
- ✅ Clients can create and display windows
- ✅ Keyboard focus works correctly
- ✅ Multiple clients can coexist

---

## Test Case

```rust
#[test]
fn test_multi_client_focus() {
    // Create compositor
    let mut state = CompositorState::new();
    
    // Simulate two clients
    let client_a = create_test_client();
    let client_b = create_test_client();
    
    // Each client creates a keyboard
    let kb_a = client_a.create_keyboard();
    let kb_b = client_b.create_keyboard();
    
    // Client B creates a surface
    let surface_b = client_b.create_surface();
    
    // Focus surface_b - should NOT crash
    switch_focus_surfaces_inline(&mut state, None, Some(&surface_b));
    
    // Verify only kb_b received enter event, not kb_a
    assert!(kb_b.received_enter());
    assert!(!kb_a.received_enter());
}
```

---

## Impact Assessment

### Current Impact
- **Severity**: CRITICAL - Blocks all client interaction
- **Affected Functionality**:
  - ❌ Window creation and mapping
  - ❌ Focus management
  - ❌ Keyboard input
  - ❌ Multi-client support

### Timeline
- **Discovery**: October 5, 2025
- **First Test**: Successful server start and socket creation ✅
- **Client Test**: Revealed crash on focus handling ❌
- **Estimated Fix Time**: 2-4 hours
- **Testing Time**: 1-2 hours

---

## Related Issues

This is a common mistake in Wayland compositor development. Similar issues:
- Sending pointer events with wrong surface
- Using wl_buffer from different client's surface
- Cross-client data offer/source usage

**General Rule**: Always verify client ownership before sending protocol events.

---

## Next Steps

1. ✅ **Identify root cause** - COMPLETE
2. 🔄 **Implement fix** - IN PROGRESS
3. ⏳ **Test with single client** - TODO
4. ⏳ **Test with multiple clients** - TODO
5. ⏳ **Test with various client types** (terminal, Firefox, etc.) - TODO
6. ⏳ **Add regression test** - TODO

---

## Additional Notes

### Good News
- The protocol implementation is otherwise working correctly
- Socket creation, client binding, and surface creation all work
- The fix is localized to a single function
- This is the ONLY blocking issue found in initial testing

### Architecture Review
The current architecture of storing all input devices globally is common but error-prone. Consider refactoring to:
- Store devices per-client for safety
- Add helper methods that automatically filter by client
- Create a `FocusManager` abstraction that handles these details

---

## References

- Wayland Protocol Spec: https://wayland.freedesktop.org/docs/html/
- Smithay Examples: Anvil compositor's focus management
- Stack Overflow: "Wayland wrong client panic" discussions

---

**Status**: Ready for fix implementation  
**Blocking**: Phase 6.2 completion  
**Priority**: P0 - Highest