"""Apply option (b) for ForeignToplevelListState minimize/restore in Smithay 0.7.

This script edits /home/quinton/axiom/src/backend/mod.rs in seven distinct
passes. Each anchor includes enough context to be unique within the file —
the previous run collided `new_for_test()` with `new()` because both
shared the five-line block from `texture_cache` to `configured_sizes`.
The disambiguating line is `outputs: Vec::new(),` (in new_for_test)
versus `outputs: vec![output],` (in new()); each anchor now includes
both.
"""

from pathlib import Path
p = Path('/home/quinton/axiom/src/backend/mod.rs')
text = p.read_text()

# ---- EDIT 1: extend imports to include ForeignToplevelHandle -----
OLD_U = 'use smithay::wayland::foreign_toplevel_list::{\n    ForeignToplevelListHandler, ForeignToplevelListState,\n};'
NEW_U = 'use smithay::wayland::foreign_toplevel_list::{\n    ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,\n};'
assert text.count(OLD_U) == 1, f'import anchor expected 1, got {text.count(OLD_U)}'
text = text.replace(OLD_U, NEW_U, 1)

# ---- EDIT 2: extend State struct with foreign_toplevel_handles -----
OLD_S = '''    // Keep ToplevelSurface handles alive (they get destroyed when dropped)
    pub toplevels: HashMap<u32, ToplevelSurface>,'''
NEW_S = '''    // Keep ToplevelSurface handles alive (they get destroyed when dropped)
    pub toplevels: HashMap<u32, ToplevelSurface>,

    /// Foreign-toplevel protocol handles keyed by Axiom surface id.
    /// Each entry is the Smithay handle backed by a
    /// `zext_foreign_toplevel_handle_v1` resource on every client that
    /// has bound the list global. Lifecycle:
    /// - `XdgShellHandler::new_toplevel` inserts an entry once the
    ///   Axiom window is created (via `new_toplevel(title, app_id)`).
    /// - `State::destroy_window` issues `send_closed` then
    ///   `remove_toplevel` on disposal.
    /// - `minimize_or_restore_focused` mirrors minimize → handle
    ///   destruction + closed event and restore → fresh `new_toplevel`
    ///   + insert. This close-then-recreate pattern is the most
    ///   accurate wire-level mirror of internal minimize/restore in
    ///   Smithay 0.7 — the per-state `wl_foreign_toplevel.state`
    ///   event (Minimized/Maximized/Activated/Fullscreen) is not
    ///   exposed as a typed helper on `ForeignToplevelHandle` in
    ///   this release and will land in a follow-up once Smithay
    ///   ships the API.
    pub foreign_toplevel_handles: HashMap<u32, ForeignToplevelHandle>,'''
assert text.count(OLD_S) == 1, f'State struct anchor expected 1, got {text.count(OLD_S)}'
text = text.replace(OLD_S, NEW_S, 1)

# ---- EDIT 3: initialise field in new_for_test() -----
# Anchor is disambiguated by the unique `outputs: Vec::new()` line above.
OLD_TT = '''            outputs: Vec::new(),
            xwm: None,
            toplevels: HashMap::new(),
            running: true,
            needs_redraw: true,
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            texture_cache: HashMap::new(),
            buffer_cache: HashMap::new(),
            buffer_cache_dimensions: HashMap::new(),
            dead_tex_handles: Vec::new(),
            configured_sizes: HashMap::new(),'''
NEW_TT = '''            outputs: Vec::new(),
            xwm: None,
            toplevels: HashMap::new(),
            running: true,
            needs_redraw: true,
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            texture_cache: HashMap::new(),
            buffer_cache: HashMap::new(),
            buffer_cache_dimensions: HashMap::new(),
            dead_tex_handles: Vec::new(),
            foreign_toplevel_handles: HashMap::new(),
            configured_sizes: HashMap::new(),'''
assert text.count(OLD_TT) == 1, f'new_for_test field anchor expected 1, got {text.count(OLD_TT)}'
text = text.replace(OLD_TT, NEW_TT, 1)

# ---- EDIT 4: initialise field in `pub fn new(...)` -----
# Anchor is disambiguated by the unique `outputs: vec![output],` line above.
OLD_N = '''            outputs: vec![output],
            xwm: None,
            toplevels: HashMap::new(),
            running: true,
            needs_redraw: true,
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            texture_cache: HashMap::new(),
            buffer_cache: HashMap::new(),
            buffer_cache_dimensions: HashMap::new(),
            dead_tex_handles: Vec::new(),
            configured_sizes: HashMap::new(),'''
NEW_N = '''            outputs: vec![output],
            xwm: None,
            toplevels: HashMap::new(),
            running: true,
            needs_redraw: true,
            window_width: 1920,
            window_height: 1080,
            pointer_x: 0.0,
            pointer_y: 0.0,
            texture_cache: HashMap::new(),
            buffer_cache: HashMap::new(),
            buffer_cache_dimensions: HashMap::new(),
            dead_tex_handles: Vec::new(),
            foreign_toplevel_handles: HashMap::new(),
            configured_sizes: HashMap::new(),'''
assert text.count(OLD_N) == 1, f'new() field anchor expected 1, got {text.count(OLD_N)}'
text = text.replace(OLD_N, NEW_N, 1)

# ---- EDIT 5: create handle in XdgShellHandler::new_toplevel -----
OLD_NT = '''        self.create_window_from_surface(
            surface_id,
            String::from("Wayland Client"),
            None,
            wl_surface,
        );
        self.needs_redraw = true;'''
NEW_NT = '''        // Mint a corresponding ForeignToplevelHandle so external
        // taskbars/docks binding to `wl_foreign_toplevel` see this
        // window with the right title/app_id. App_id is delivered
        // out-of-band via wl_surface events; we currently have no
        // listener, so we pass an empty string. The handle is moved
        // into `state.foreign_toplevel_handles` immediately so
        // `destroy_window` + `minimize_or_restore_focused` can locate
        // it deterministically by Axiom surface id. Smithay 0.7's
        // `new_toplevel` returns the handle by value.
        let ftl_handle = self
            .foreign_toplevel_list_state
            .new_toplevel(String::from("Wayland Client"), String::new());
        self.foreign_toplevel_handles.insert(surface_id, ftl_handle);

        self.create_window_from_surface(
            surface_id,
            String::from("Wayland Client"),
            None,
            wl_surface,
        );
        self.needs_redraw = true;'''
assert text.count(OLD_NT) == 1, f'new_toplevel anchor expected 1, got {text.count(OLD_NT)}'
text = text.replace(OLD_NT, NEW_NT, 1)

# ---- EDIT 6: destroy_window → drop the FTL handle -----
OLD_DW = '''    pub fn destroy_window(&mut self, surface_id: u32) {
        // Release the toplevel handle to prevent memory leaks
        self.toplevels.remove(&surface_id);'''
NEW_DW = '''    pub fn destroy_window(&mut self, surface_id: u32) {
        // Drop the ForeignToplevelHandle first so external taskbars
        // see the closed event before we mutate any Axiom-internal
        // state. `send_closed` is idempotent on Smithay 0.7; clearing
        // our HashMap entry BEFORE `remove_toplevel` guarantees
        // `minimize_or_restore_focused` / `prune_dead_surfaces`
        // cannot resurrect a half-destroyed record. Smithay's
        // `send_closed` borrows the handle immutably; `remove_toplevel`
        // borrows the list state mutably — no aliasing conflict.
        if let Some(handle) = self.foreign_toplevel_handles.remove(&surface_id) {
            handle.send_closed();
            self.foreign_toplevel_list_state.remove_toplevel(&handle);
        }

        // Release the toplevel handle to prevent memory leaks
        self.toplevels.remove(&surface_id);'''
assert text.count(OLD_DW) == 1, f'destroy_window anchor expected 1, got {text.count(OLD_DW)}'
text = text.replace(OLD_DW, NEW_DW, 1)

# ---- EDIT 7: minimize_or_restore_focused → mirror FTL state -----
OLD_MO = '''        let focused_id = match self.state.window_manager.read().focused_window_id() {
            Some(id) => id,
            None => {
                debug!("minimize/restore: no focused window — no-op");
                return;
            }
        };
        let is_minimized = self.state.window_manager.read().is_minimized(focused_id);

        if is_minimized {
            // ── Restore path ────────────────────────────────────────
            // Drop the effect entry first so animate_window_open seeds
            // a fresh open t=0 spring. (If the entry is missing entirely,
            // animate_window_open creates it with scale=0.8/opacity=0.)
            self.state.effects_engine.write().remove_window(focused_id);
            // Workspace re-adds the window to the focused column on the
            // active tape; the renderer will see a rectangle on the next
            // layout query.
            self.state
                .workspace_manager
                .write()
                .restore_window(focused_id);
            // Flip the per-window state last so the renderer snapshot
            // and the keyboard-focus routing agree with the layout
            // decision.
            self.state
                .window_manager
                .write()
                .restore_window(focused_id);
            self.state.window_manager.write().focus_window(focused_id);
            self.state
                .effects_engine
                .write()
                .animate_window_open(focused_id);
            info!(
                "📤 Restored minimized window {} (focus returned to it)",
                focused_id
            );
        } else {
            // ── Minimize path ─────────────────────────────────────
            // 1. Trigger the spring close-fade via effects first. The
            //    fade is async (driven by AnimationController.update()),
            //    so the renderer still sees the window during the fade.
            // 2. Remove from the workspace column so the layout map
            //    no longer emits a rect (renderer skips it).
            // 3. Set the per-window minimized flag last so the
            //    WindowManager's keyboard-focus drop happens
            //    exactly when visible state collapses.
            self.state
                .effects_engine
                .write()
                .animate_window_close(focused_id);
            self.state
                .workspace_manager
                .write()
                .minimize_window(focused_id);
            self.state
                .window_manager
                .write()
                .minimize_window(focused_id);
            info!("📥 Minimized focused window {}", focused_id);
        }'''
NEW_MO = '''        let focused_id = match self.state.window_manager.read().focused_window_id() {
            Some(id) => id,
            None => {
                debug!("minimize/restore: no focused window — no-op");
                return;
            }
        };
        let is_minimized = self.state.window_manager.read().is_minimized(focused_id);
        // Resolve the surface id BEFORE any writes below so the FTL
        // mirror can locate our handle without dropping the locks.
        // The surface-id-to-window-id mapping lives in `state.window_map`.
        let surface_id = self.state.window_map.get(&focused_id).copied();

        if is_minimized {
            // ── Restore path ────────────────────────────────────────
            // Drop the effect entry first so animate_window_open seeds
            // a fresh open t=0 spring. (If the entry is missing entirely,
            // animate_window_open creates it with scale=0.8/opacity=0.)
            self.state.effects_engine.write().remove_window(focused_id);
            // Workspace re-adds the window to the focused column on the
            // active tape; the renderer will see a rectangle on the next
            // layout query.
            self.state
                .workspace_manager
                .write()
                .restore_window(focused_id);
            // Flip the per-window state last so the renderer snapshot
            // and the keyboard-focus routing agree with the layout
            // decision.
            self.state
                .window_manager
                .write()
                .restore_window(focused_id);
            self.state.window_manager.write().focus_window(focused_id);
            self.state
                .effects_engine
                .write()
                .animate_window_open(focused_id);
            info!(
                "📤 Restored minimized window {} (focus returned to it)",
                focused_id
            );

            // ── FTL mirror (restore) ────────────────────────────────
            // Re-add a fresh handle so external taskbars see the
            // window reappear in the list. Smithay 0.7's
            // `ForeignToplevelHandle` does not expose a typed
            // `set_states` helper for the wl_foreign_toplevel.state
            // event (per inspection of
            // smithay-0.7.0/src/wayland/foreign_toplevel_list/mod.rs),
            // so the close-then-recreate pattern is the most accurate
            // wire-level mirror of internal minimize/restore available
            // in this release. We borrow the title from
            // WindowManager (single read; the locked borrows above
            // all finished before this point) so the FTL entry
            // matches what the renderer is about to draw.
            if let Some(surface_id) = surface_id {
                let restored_title = self
                    .state
                    .window_manager
                    .read()
                    .get_window(focused_id)
                    .map(|w| w.window.title.clone())
                    .unwrap_or_else(|| String::from("Wayland Client"));
                let restored_handle = self
                    .state
                    .foreign_toplevel_list_state
                    .new_toplevel(restored_title, String::new());
                self.state
                    .foreign_toplevel_handles
                    .insert(surface_id, restored_handle);
            }
        } else {
            // ── Minimize path ─────────────────────────────────────
            // 1. Trigger the spring close-fade via effects first. The
            //    fade is async (driven by AnimationController.update()),
            //    so the renderer still sees the window during the fade.
            // 2. Remove from the workspace column so the layout map
            //    no longer emits a rect (renderer skips it).
            // 3. Set the per-window minimized flag last so the
            //    WindowManager's keyboard-focus drop happens
            //    exactly when visible state collapses.
            self.state
                .effects_engine
                .write()
                .animate_window_close(focused_id);
            self.state
                .workspace_manager
                .write()
                .minimize_window(focused_id);
            self.state
                .window_manager
                .write()
                .minimize_window(focused_id);
            info!("📥 Minimized focused window {}", focused_id);

            // ── FTL mirror (minimize) ───────────────────────────────
            // Drop the handle so external taskbars see the closed
            // event immediately. The handle map entry is removed
            // BEFORE the protocol message so no other code path
            // (e.g. destroy_window racing a focused minimize) can
            // see a half-destroyed record.
            if let Some(surface_id) = surface_id {
                if let Some(handle) =
                    self.state.foreign_toplevel_handles.remove(&surface_id)
                {
                    handle.send_closed();
                    self.state
                        .foreign_toplevel_list_state
                        .remove_toplevel(&handle);
                }
            }
        }'''
assert text.count(OLD_MO) == 1, f'minimize_or_restore_focused anchor expected 1, got {text.count(OLD_MO)}'
text = text.replace(OLD_MO, NEW_MO, 1)

p.write_text(text)
print('OK: FTL minimize/restore hook applied via Smithay 0.7 close-recreate pattern')
