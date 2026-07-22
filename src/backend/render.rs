//! Rendering for the Smithay winit/GLES backend.
//!
//! Contains `AxiomSmithayBackendReal::render` and its scene-prep helper. A
//! submodule of `backend` can read the private fields of `State` and
//! `AxiomSmithayBackendReal` (descendant modules see ancestor privates), so
//! no fields were made public for this move.

use crate::decoration::{DecorationMode, WindowDecoration};
use crate::window::Rectangle as WindowRectangle;
use crate::workspace::scale_to_logical;
use anyhow::Result;
use log::{debug, warn};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::gles::{GlesFrame, GlesRenderer, GlesTarget, GlesTexture};
use smithay::backend::renderer::ExportMem;
use smithay::backend::renderer::{
    element::{
        solid::{SolidColorBuffer, SolidColorRenderElement},
        texture::{TextureBuffer, TextureRenderElement},
        Element, Kind, RenderElement,
    },
    Color32F, Frame, ImportAll, Renderer,
};
use smithay::utils::{Physical, Point, Rectangle, Size, Transform};
use smithay::wayland::compositor::{
    get_children, with_states, BufferAssignment, SubsurfaceCachedState, SurfaceAttributes,
};
use smithay::wayland::session_lock::LockSurface;
use smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState;
use smithay::wayland::shm::with_buffer_contents_mut;
use std::collections::{HashMap, HashSet};
use wayland_server::backend::ObjectId;
use wayland_server::protocol::wl_buffer::WlBuffer;
use wayland_server::protocol::wl_surface::WlSurface;
use wayland_server::Resource;

use super::{AxiomSmithayBackendReal, State};

impl State {
    /// Calculate workspace layouts, synchronize window geometry, and notify
    /// Wayland clients of size changes. Shared by nested and DRM render paths.
    fn prepare_render_scene(&mut self) -> HashMap<u64, WindowRectangle> {
        let mut layouts = self.workspace_manager.read().calculate_workspace_layouts();

        // Fullscreen windows fill the entire output viewport
        let fullscreen_ids: Vec<u64> = {
            let wm = self.window_manager.read();
            layouts
                .keys()
                .filter(|&id| {
                    wm.get_window(*id)
                        .map(|w| w.properties.fullscreen)
                        .unwrap_or(false)
                })
                .copied()
                .collect()
        };
        for &window_id in &fullscreen_ids {
            layouts.insert(
                window_id,
                WindowRectangle {
                    x: 0,
                    y: 0,
                    width: self.window_width,
                    height: self.window_height,
                },
            );
        }

        {
            let mut wm = self.window_manager.write();
            for (window_id, layout_rect) in &layouts {
                // Update window geometry (non-floating, non-fullscreen windows)
                if let Some(window) = wm.get_window_mut(*window_id) {
                    if !window.properties.floating {
                        if !window.properties.fullscreen {
                            window.window.set_position(layout_rect.x, layout_rect.y);
                        }
                        window
                            .window
                            .set_size(layout_rect.width, layout_rect.height);
                    }
                }

                // Send configure notifications to toplevels (same loop, avoids
                // a second HashMap iteration).
                if let Some(&surface_id) = self.window_map.get(window_id) {
                    if let Some(toplevel) = self.toplevels.get(&surface_id) {
                        self.update_surface_fractional_scale(toplevel.wl_surface());
                        let scale = self
                            .workspace_manager
                            .read()
                            .scale_factor_for_window(*window_id);
                        let new_w = (scale_to_logical(layout_rect.width as i32, scale).round() as i32).max(1);
                        let new_h = (scale_to_logical(layout_rect.height as i32, scale).round() as i32).max(1);

                        let needs_configure = self
                            .configured_sizes
                            .get(&surface_id)
                            .is_none_or(|&(cw, ch)| cw != new_w || ch != new_h);
                        let pending = self.pending_configure.contains(&surface_id);

                        if needs_configure && !pending {
                            toplevel.with_pending_state(|state| {
                                state.size = Some((new_w, new_h).into());
                            });
                            toplevel.send_configure();
                            self.configured_sizes.insert(surface_id, (new_w, new_h));
                            self.pending_configure.insert(surface_id);

                            debug!(
                                "📐 Configured surface {} to {}x{}",
                                surface_id, new_w, new_h
                            );
                        }
                    }
                }
            }
        }

        layouts
    }
}

impl AxiomSmithayBackendReal {
    /// Render the current frame.
    ///
    /// Binds the winit GL surface, composites the current scene, then presents
    /// it. The scene-compositing step is shared with `capture_pixels` via the
    /// `render_scene_into` helper so the pixel-readback test renders exactly
    /// what the live path presents.
    pub(super) fn render(&mut self) -> Result<()> {
        // Headless Noop backend performs no rendering and creates no GL/winit
        // context — this lets `tick()` run in tests/CI without a display.
        if self.backend_kind == crate::backend::BackendKind::Noop {
            return Ok(());
        }

        let Some(backend) = self.winit_backend.as_mut() else {
            return Ok(());
        };
        if let Some(icon) = self.state.cursor_icon {
            backend.window().set_cursor(icon);
        }
        {
            // Composite into the bound framebuffer; drop the framebuffer borrow
            // before presenting so `backend.submit` can re-borrow `winit_backend`.
            let (renderer, mut framebuffer) = backend.bind()?;

            // When multi-output is enabled, prepare elements per-output.
            // Each output renders its region within the shared framebuffer.
            #[cfg(feature = "multi-output-experimental")]
            {
                let outputs = self.state.outputs.clone();
                for output in &outputs {
                    let _layouts = prepare_render_elements_for_output(&mut self.state, output);
                    render_scene_into(&mut self.state, renderer, &mut framebuffer)?;
                }
            }

            // Default single-output path — unchanged.
            #[cfg(not(feature = "multi-output-experimental"))]
            render_scene_into(&mut self.state, renderer, &mut framebuffer)?;

            // Capture screencopy after rendering (if a client requested one).
            Self::capture_screencopy(&mut self.state, renderer, &mut framebuffer);
        }
        let damage: Option<Vec<Rectangle<i32, Physical>>> = if self.state.output_damage.is_empty() {
            None
        } else {
            // ponytail: bounding-box merge of all output damage for simplicity.
            // Switch to OutputDamageTracker for per-element occlusion culling.
            let mut min_x = i32::MAX;
            let mut min_y = i32::MAX;
            let mut max_x = i32::MIN;
            let mut max_y = i32::MIN;
            for r in &self.state.output_damage {
                min_x = min_x.min(r.loc.x);
                min_y = min_y.min(r.loc.y);
                max_x = max_x.max(r.loc.x + r.size.w);
                max_y = max_y.max(r.loc.y + r.size.h);
            }
            // Clamp to window dimensions
            let w = self.state.window_width as i32;
            let h = self.state.window_height as i32;
            min_x = min_x.max(0);
            min_y = min_y.max(0);
            max_x = max_x.min(w);
            max_y = max_y.min(h);
            if min_x >= max_x || min_y >= max_y {
                None
            } else {
                Some(vec![Rectangle::new(
                    Point::from((min_x, min_y)),
                    Size::from((max_x - min_x, max_y - min_y)),
                )])
            }
        };
        backend.submit(damage.as_deref())?;
        self.state.output_damage.clear();
        backend.window().pre_present_notify();
        Ok(())
    }
}

/// Prepare render elements for a single output.
///
/// Returns the window layout for the given output. When multi-output is
/// enabled, each output's layout is computed independently; when disabled,
/// the function is a no-op wrapper that returns the single-output layout.
/// ponytail: per-output layout computation is a forward-looking API hook.
/// Currently delegates to `prepare_render_scene` which uses the global
/// viewport. Upgrade to per-output viewport sizing when the workspace
/// manager supports per-output tapes with distinct viewport sizes.
#[cfg(feature = "multi-output-experimental")]
fn prepare_render_elements_for_output(
    state: &mut State,
    _output: &smithay::output::Output,
) -> HashMap<u64, WindowRectangle> {
    state.prepare_render_scene()
}

impl AxiomSmithayBackendReal {
    ///
    /// Binds the winit GL context, re-composites the current scene into the
    /// (un-swapped) back buffer, and reads it with `glReadPixels`. Returns
    /// `None` on the `Noop` backend or if no GL context is available, so a
    /// caller can treat absence as "no pixels to verify".
    pub fn capture_pixels(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        if self.backend_kind == crate::backend::BackendKind::Noop {
            return None;
        }
        let backend = self.winit_backend.as_mut()?;
        let (renderer, mut framebuffer) = backend.bind().ok()?;
        // Re-composite into the back buffer without presenting, so the bytes we
        // read are the freshly-drawn frame.
        render_scene_into(&mut self.state, renderer, &mut framebuffer).ok()?;

        let w = self.state.window_width;
        let h = self.state.window_height;
        let region = Rectangle::new(Point::from((0, 0)), Size::from((w as i32, h as i32)));
        let mapping = renderer
            .copy_framebuffer(&framebuffer, region, Fourcc::Argb8888)
            .ok()?;
        let pixels = renderer.map_texture(&mapping).ok()?;
        Some((w, h, pixels.to_vec()))
    }

    /// Capture the current composited frame into a pending screencopy buffer.
    ///
    /// Called from `render()` after `render_scene_into()` has composed into the
    /// winit backbuffer. Reads pixels from the backbuffer via `ExportMem::copy_framebuffer`,
    /// writes them into the client's SHM buffer, and sends `ready`/`failed` on the frame.
    ///
    /// Takes `state` separately (not `&mut self`) to avoid borrow conflicts with
    /// `self.winit_backend` which is borrowed by the caller's renderer/framebuffer.
    fn capture_screencopy(
        state: &mut State,
        renderer: &mut GlesRenderer,
        framebuffer: &mut GlesTarget<'_>,
    ) {
        let Some(capture) = state.pending_capture.take() else {
            return;
        };

        let region = Rectangle::new(
            Point::from((0, 0)),
            Size::from((capture.size.w, capture.size.h)),
        );

        match renderer.copy_framebuffer(framebuffer, region, Fourcc::Argb8888) {
            Ok(mapping) => {
                match renderer.map_texture(&mapping) {
                    Ok(pixels) => {
                        match with_buffer_contents_mut(&capture.buffer, |ptr, len, _data| {
                            // SAFETY: Smithay guarantees `ptr` is valid for `len` bytes
                            // during the callback. The slice is immediately copied before
                            // the closure returns.
                            let dest =
                                unsafe { std::slice::from_raw_parts_mut(ptr, len) };
                            let copy_len = pixels.len().min(dest.len());
                            dest[..copy_len].copy_from_slice(&pixels[..copy_len]);
                        }) {
                            Ok(()) => {
                                use std::time::{SystemTime, UNIX_EPOCH};
                                let now = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default();
                                use smithay::reexports::wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1;
                                capture
                                    .frame
                                    .flags(zwlr_screencopy_frame_v1::Flags::YInvert);
                                capture.frame.ready(
                                    (now.as_secs() >> 32) as u32,
                                    (now.as_secs() & 0xFFFF_FFFF) as u32,
                                    now.subsec_nanos(),
                                );
                            }
                            Err(e) => {
                                warn!("Screencopy SHM write failed: {:?}", e);
                                capture.frame.failed();
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Screencopy map_texture failed: {:?}", e);
                        capture.frame.failed();
                    }
                }
            }
            Err(e) => {
                warn!("Screencopy copy_framebuffer failed: {:?}", e);
                capture.frame.failed();
            }
        }
    }
}

/// Recursively import buffers for a surface and all its subsurface children
/// into the texture cache.
fn import_surface_tree(state: &mut State, renderer: &mut GlesRenderer, surface: &WlSurface) {
    use smithay::backend::renderer::element::texture::TextureBuffer;
    use smithay::utils::Transform;

    let buf: Option<WlBuffer> = with_states(surface, |states| {
        match states
            .cached_state
            .get::<SurfaceAttributes>()
            .current()
            .buffer
        {
            Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
            Some(BufferAssignment::Removed) => {
                let bid = surface.id();
                state.texture_cache.pop_entry(&bid);
                None
            }
            _ => None,
        }
    });
    if let Some(ref buf) = buf {
        let bid = buf.id();
        if !state.texture_cache.contains(&bid) {
            match renderer.import_buffer(buf, None, &[]) {
                Some(Ok(tex)) => {
                    let tb =
                        TextureBuffer::from_texture(&*renderer, tex, 1, Transform::Normal, None);
                    state.texture_cache.put(bid.clone(), tb);
                }
                Some(Err(e)) => warn!("⚠️ Subsurface buffer import error: {:?}", e),
                None => {}
            }
        }
    }
    for child in get_children(surface) {
        import_surface_tree(state, renderer, &child);
    }
}

/// Recursively draw a surface and all its subsurface children from the
/// texture cache. `offset_x/offset_y` is the absolute screen position of
/// this surface's top-left corner in logical pixels.
fn draw_surface_tree(
    state: &mut State,
    frame: &mut GlesFrame<'_, '_>,
    surface: &WlSurface,
    offset_x: f64,
    offset_y: f64,
    scale: smithay::utils::Scale<f64>,
) -> Result<(), anyhow::Error> {
    use smithay::backend::renderer::element::texture::TextureRenderElement;
    use smithay::backend::renderer::element::Kind;
    use smithay::backend::renderer::element::RenderElement;
    use smithay::backend::renderer::gles::GlesTexture;
    use smithay::utils::Point;

    // Draw this surface's texture if available
    let buf: Option<WlBuffer> = with_states(surface, |states| {
        match states
            .cached_state
            .get::<SurfaceAttributes>()
            .current()
            .buffer
        {
            Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
            Some(BufferAssignment::Removed) => None,
            _ => None,
        }
    });
    if let Some(buf) = buf {
        if let Some(tb) = state.texture_cache.get(&buf.id()) {
            let te = TextureRenderElement::from_texture_buffer(
                Point::from((offset_x, offset_y)),
                tb,
                None,
                None,
                None,
                Kind::Unspecified,
            );
            let tg = te.geometry(scale);
            <TextureRenderElement<GlesTexture> as RenderElement<GlesRenderer>>::draw(
                &te,
                frame,
                te.src(),
                tg,
                &[tg],
                &[],
            )?;
        }
    }
    // Draw children (subsurfaces) — their position is relative to this surface
    for child in get_children(surface) {
        let child_offset = with_states(&child, |states| {
            let loc = states
                .cached_state
                .get::<SubsurfaceCachedState>()
                .current()
                .location;
            (loc.x as f64, loc.y as f64)
        });
        draw_surface_tree(
            state,
            frame,
            &child,
            offset_x + child_offset.0,
            offset_y + child_offset.1,
            scale,
        )?;
    }
    Ok(())
}

/// Composite the current scene into an already-bound winit framebuffer.
///
/// Shared by `render` (which then presents) and `capture_pixels` (which reads
/// the un-swapped back buffer). Mirrors the previous inline `render` body;
/// the only difference is the caller owns the bind/submit steps.
fn render_scene_into(
    state: &mut State,
    renderer: &mut GlesRenderer,
    framebuffer: &mut GlesTarget<'_>,
) -> Result<()> {
    let layouts = state.prepare_render_scene(); // HashMap<u64, crate::window::Rectangle>
    let scale = smithay::utils::Scale::from(state.focused_output_scale());

    // Update surface previous rects for damage tracking and collect render items
    // in a single pass over layouts (avoids iterating the HashMap twice).
    let mut items: Vec<(u64, WindowRectangle, Option<WindowDecoration>)> =
        Vec::with_capacity(layouts.len());
    let wm = state.window_manager.read();
    let dm = state.decoration_manager.read();
    for (window_id, rect) in &layouts {
        let &surface_id = match state.window_map.get(window_id) {
            Some(sid) => sid,
            None => continue,
        };
        state.surface_previous_rects.insert(
            surface_id,
            Rectangle::new(
                Point::from((rect.x, rect.y)),
                Size::from((rect.width as i32, rect.height as i32)),
            ),
        );
        if state.toplevels.contains_key(&surface_id) {
            // Skip decorations for fullscreen windows
            let is_fullscreen = wm
                .get_window(*window_id)
                .map(|w| w.properties.fullscreen)
                .unwrap_or(false);
            let dec = if is_fullscreen {
                None
            } else {
                dm.get_decoration(*window_id).cloned()
            };
            items.push((*window_id, rect.clone(), dec));
        }
    }
    let decorations: Vec<(u64, DecorationMode, bool)> = {
        let mut decs = Vec::with_capacity(dm.decorations().len());
        for (id, d) in dm.decorations().iter() {
            let is_fullscreen = wm
                .get_window(*id)
                .map(|w| w.properties.fullscreen)
                .unwrap_or(true);
            if !is_fullscreen {
                decs.push((*id, d.mode, d.focused));
            }
        }
        decs
    };
    drop(wm);
    drop(dm);

    let (w, h) = (state.window_width as i32, state.window_height as i32);

    // Import client buffers FIRST (before frame creation, to avoid double-borrowing renderer).
    // Walk the full subsurface tree for each visible window so child buffers are cached too.
    let surfaces_to_import: Vec<WlSurface> = {
        let mut surfaces = Vec::with_capacity(items.len());
        for (window_id, _rect, _dec) in &items {
            if let Some(&surface_id) = state.window_map.get(window_id) {
                if let Some(t) = state.toplevels.get(&surface_id) {
                    surfaces.push(t.wl_surface().clone());
                }
            }
        }
        surfaces
    };
    for surface in &surfaces_to_import {
        import_surface_tree(state, renderer, surface);
    }
    // Update SurfaceData.size from imported textures (fixes #19)
    for surface in &surfaces_to_import {
        let buf: Option<WlBuffer> = with_states(surface, |states| {
            match states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .buffer
            {
                Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
                _ => None,
            }
        });
        if let Some(ref buf) = buf {
            if let Some(tb) = state.texture_cache.get(&buf.id()) {
                let te_temp = TextureRenderElement::from_texture_buffer(
                    Point::from((0.0, 0.0)),
                    tb,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                );
                let geo = te_temp.geometry(scale);
                let surface_id = surface.id().protocol_id();
                if let Some(sd) = state.surfaces.get_mut(&surface_id) {
                    sd.size = (geo.size.w, geo.size.h);
                }
            }
        }
    }
    // Import DnD icon texture before frame creation so renderer is available.
    let _dnd_bid: Option<ObjectId> = if state.dnd_active {
        state.dnd_icon.as_ref().and_then(|icon_surface| {
            let icon_buf: Option<WlBuffer> = with_states(icon_surface, |states| {
                match states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .buffer
                {
                    Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
                    Some(BufferAssignment::Removed) => {
                        state.texture_cache.pop_entry(&icon_surface.id());
                        None
                    }
                    _ => None,
                }
            });
            icon_buf.map(|buf| {
                let bid = buf.id();
                if !state.texture_cache.contains(&bid) {
                    match renderer.import_buffer(&buf, None, &[]) {
                        Some(Ok(tex)) => {
                            let tb = TextureBuffer::from_texture(
                                &*renderer,
                                tex,
                                1,
                                Transform::Normal,
                                None,
                            );
                            state.texture_cache.put(bid.clone(), tb);
                        }
                        Some(Err(e)) => warn!("⚠️ Failed to import DnD icon buffer: {:?}", e),
                        None => {}
                    }
                }
                bid
            })
        })
    } else {
        None
    };
    // Import lock surface textures before frame creation (same reason)
    if state.session_locked {
        state.lock_surfaces.retain(LockSurface::alive);
        for lock_surface in &state.lock_surfaces {
            let buf: Option<WlBuffer> =
                with_states(lock_surface.wl_surface(), |states| {
                    match states
                        .cached_state
                        .get::<SurfaceAttributes>()
                        .current()
                        .buffer
                    {
                        Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
                        Some(BufferAssignment::Removed) => {
                            state.texture_cache.pop_entry(&lock_surface.wl_surface().id());
                            None
                        }
                        _ => None,
                    }
                });
            if let Some(buf) = buf {
                let bid = buf.id();
                if !state.texture_cache.contains(&bid) {
                    match renderer.import_buffer(&buf, None, &[]) {
                        Some(Ok(tex)) => {
                            let tb = TextureBuffer::from_texture(
                                &*renderer,
                                tex,
                                1,
                                Transform::Normal,
                                None,
                            );
                            state.texture_cache.put(bid.clone(), tb);
                        }
                        Some(Err(e)) => warn!("⚠️ Failed to import lock surface buffer: {:?}", e),
                        None => {}
                    }
                }
            }
        }
    }

    // Import layer surface textures before frame creation
    let mut layer_textures: HashMap<ObjectId, ()> = HashMap::new();
    for layer_surface in state.layer_shell_state.layer_surfaces() {
        let buf: Option<WlBuffer> = with_states(layer_surface.wl_surface(), |states| match states
            .cached_state
            .get::<SurfaceAttributes>()
            .current()
            .buffer
        {
            Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
            Some(BufferAssignment::Removed) => {
                state.texture_cache.pop_entry(&layer_surface.wl_surface().id());
                None
            }
            _ => None,
        });
        if let Some(buf) = buf {
            let bid = buf.id();
            if !state.texture_cache.contains(&bid) {
                match renderer.import_buffer(&buf, None, &[]) {
                    Some(Ok(tex)) => {
                        let tb = TextureBuffer::from_texture(
                            &*renderer,
                            tex,
                            1,
                            Transform::Normal,
                            None,
                        );
                        state.texture_cache.put(bid.clone(), tb);
                    }
                    Some(Err(e)) => {
                        warn!("⚠️ Failed to import layer surface buffer: {:?}", e);
                    }
                    None => {}
                }
            }
            if state.texture_cache.contains(&bid) {
                layer_textures.insert(bid, ());
            }
        }
    }
    let mut frame = renderer.render(framebuffer, Size::from((w, h)), Transform::Normal)?;
    frame.clear(
        Color32F::from([0.05f32, 0.05, 0.08, 1.0]),
        &[Rectangle::new(Point::from((0, 0)), Size::from((w, h)))],
    )?;

    // When locked, only render lock screen surfaces (skip normal shell content)
    if state.session_locked {
        render_lock_surfaces(state, &mut frame, scale)?;
        let _ = frame.finish()?;
        return Ok(());
    }

    // Occlusion culling: process front-to-back to identify fully covered windows,
    // then draw back-to-front skipping occluded surface trees.
    // Items are in back-to-front order, so reversed iteration is front-to-back.
    let mut occluded_windows: HashSet<u64> = HashSet::new();
    {
        let dm = state.decoration_manager.read();
        let mut occluded_regions: Vec<Rectangle<i32, Physical>> = Vec::with_capacity(items.len());
        for (window_id, rect, _dec) in items.iter().rev() {
            let content = dm.get_content_rect(*window_id, rect.clone());
            let content_rect: Rectangle<i32, Physical> = Rectangle::new(
                Point::from((content.x, content.y)),
                Size::from((content.width as i32, content.height as i32)),
            );
            // Check if this window is fully covered by any already-rendered region
            let covered = occluded_regions
                .iter()
                .any(|r| r.contains_rect(content_rect));
            if covered {
                occluded_windows.insert(*window_id);
            }
            occluded_regions.push(content_rect);
        }
    } // dm dropped here, unblocking &mut state in the drawing loop

    for (window_id, rect, dec) in &items {
        let content = state
            .decoration_manager
            .read()
            .get_content_rect(*window_id, rect.clone());
        let color: [f32; 4] = match dec {
            Some(d) if d.focused => [0.2, 0.2, 0.4, 1.0],
            Some(_) => [0.1, 0.1, 0.2, 1.0],
            None => [0.3, 0.3, 0.3, 1.0],
        };
        let bg = SolidColorBuffer::new((content.width as i32, content.height as i32), color);
        let bg_elem = SolidColorRenderElement::from_buffer(
            &bg,
            Point::from((content.x, content.y)),
            1.0,
            1.0,
            Kind::Unspecified,
        );
        let g = bg_elem.geometry(scale);
        <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
            &bg_elem,
            &mut frame,
            bg_elem.src(),
            g,
            &[g],
            &[],
        )?;
        // Draw the full surface tree (including subsurfaces) from the texture cache,
        // unless this window is fully occluded (behind another opaque window).
        if !occluded_windows.contains(window_id) {
            if let Some(&surface_id) = state.window_map.get(window_id) {
                if let Some(t) = state.toplevels.get(&surface_id) {
                    let wl_surface = t.wl_surface().clone();
                    draw_surface_tree(
                        state,
                        &mut frame,
                        &wl_surface,
                        content.x as f64,
                        content.y as f64,
                        scale,
                    )?;
                }
            }
        }
    }
    // SSD decorations: titlebar + 3 buttons with theme colors and symbol shapes.
    let theme = state.decoration_manager.read().theme().clone();
    for (window_id, mode, focused) in &decorations {
        if *mode == DecorationMode::ServerSide {
            if let Some(rect) = layouts.get(window_id) {
                let titlebar_h = theme.titlebar_height as i32;
                let tb_color = if *focused {
                    theme.titlebar_bg_focused
                } else {
                    theme.titlebar_bg_unfocused
                };
                let tb = SolidColorBuffer::new((rect.width as i32, titlebar_h), tb_color);
                let tb_elem = SolidColorRenderElement::from_buffer(
                    &tb,
                    Point::from((rect.x, rect.y)),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                );
                let tg = tb_elem.geometry(scale);
                <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
                    &tb_elem,
                    &mut frame,
                    tb_elem.src(),
                    tg,
                    &[tg],
                    &[],
                )?;

                // Draw buttons using theme colors and positions matching decoration.rs hit-testing.
                let btn_size = theme.button_size as i32;
                let margin = 8i32;
                let button_y = ((theme.titlebar_height - theme.button_size) / 2) as i32;
                let sym_color = if *focused {
                    theme.text_color_focused
                } else {
                    theme.text_color_unfocused
                };

                // Close button (idx=0)
                let cx = rect.x + rect.width as i32 - (btn_size + margin);
                let cy = rect.y + button_y;
                let cb = SolidColorBuffer::new((btn_size, btn_size), theme.close_normal);
                let ce = SolidColorRenderElement::from_buffer(
                    &cb,
                    Point::from((cx, cy)),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                );
                let cg = ce.geometry(scale);
                <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
                    &ce,
                    &mut frame,
                    ce.src(),
                    cg,
                    &[cg],
                    &[],
                )?;
                // × symbol: two thin rectangles forming a cross (+)
                let s = 3; // stroke width
                let l = btn_size / 2 - 2; // arm length
                let center = l + 2; // offset from button edge
                let h_rect = SolidColorBuffer::new((l * 2 + 1, s), sym_color);
                let v_rect = SolidColorBuffer::new((s, l * 2 + 1), sym_color);
                for (x_off, y_off) in [(center - l, center - s / 2), (center - s / 2, center - l)] {
                    let r = if x_off == center - l {
                        &h_rect
                    } else {
                        &v_rect
                    };
                    let re = SolidColorRenderElement::from_buffer(
                        r,
                        Point::from((cx + x_off, cy + y_off)),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    );
                    let rg = re.geometry(scale);
                    <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
                        &re,
                        &mut frame,
                        re.src(),
                        rg,
                        &[rg],
                        &[],
                    )?;
                }

                // Maximize button (idx=1)
                let mx = rect.x + rect.width as i32 - (btn_size + margin) * 2;
                let my = rect.y + button_y;
                let mb = SolidColorBuffer::new((btn_size, btn_size), theme.button_normal);
                let me = SolidColorRenderElement::from_buffer(
                    &mb,
                    Point::from((mx, my)),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                );
                let mg = me.geometry(scale);
                <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
                    &me,
                    &mut frame,
                    me.src(),
                    mg,
                    &[mg],
                    &[],
                )?;
                // □ symbol: a small filled square
                let sq_size = btn_size / 2 - 2;
                let sq_off = (btn_size - sq_size) / 2;
                let sq = SolidColorBuffer::new((sq_size, sq_size), sym_color);
                let sq_e = SolidColorRenderElement::from_buffer(
                    &sq,
                    Point::from((mx + sq_off, my + sq_off)),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                );
                let sq_g = sq_e.geometry(scale);
                <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
                    &sq_e,
                    &mut frame,
                    sq_e.src(),
                    sq_g,
                    &[sq_g],
                    &[],
                )?;

                // Minimize button (idx=2)
                let nx = rect.x + rect.width as i32 - (btn_size + margin) * 3;
                let ny = rect.y + button_y;
                let nb = SolidColorBuffer::new((btn_size, btn_size), theme.button_normal);
                let ne = SolidColorRenderElement::from_buffer(
                    &nb,
                    Point::from((nx, ny)),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                );
                let ng = ne.geometry(scale);
                <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
                    &ne,
                    &mut frame,
                    ne.src(),
                    ng,
                    &[ng],
                    &[],
                )?;
                // — symbol: a thin horizontal line
                let line_w = btn_size / 2 + 2;
                let line_h = 3;
                let line_off_y = (btn_size - line_h) / 2;
                let line_off_x = (btn_size - line_w) / 2;
                let line = SolidColorBuffer::new((line_w, line_h), sym_color);
                let line_e = SolidColorRenderElement::from_buffer(
                    &line,
                    Point::from((nx + line_off_x, ny + line_off_y)),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                );
                let line_g = line_e.geometry(scale);
                <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
                    &line_e,
                    &mut frame,
                    line_e.src(),
                    line_g,
                    &[line_g],
                    &[],
                )?;
            }
        }
    }
    // Render layer shell surfaces (panels, bars, etc.)
    for layer_surface in state.layer_shell_state.layer_surfaces() {
        // Get anchor and margin from the client's committed state.
        let (anchor, margin) = with_states(layer_surface.wl_surface(), |states| {
            let mut cs = states.cached_state.get::<LayerSurfaceCachedState>();
            let s = cs.current();
            (s.anchor, s.margin)
        });
        let buf: Option<WlBuffer> = with_states(layer_surface.wl_surface(), |states| match states
            .cached_state
            .get::<SurfaceAttributes>()
            .current()
            .buffer
        {
            Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
            Some(BufferAssignment::Removed) => None,
            _ => None,
        });
        if let Some(buf) = buf {
            if let Some(tb) = state.texture_cache.get(&buf.id()) {
                // Create a temporary element at (0,0) just to discover its logical size,
                // then reposition it according to anchor + margin + output size.
                let te_temp = TextureRenderElement::from_texture_buffer(
                    Point::from((0.0, 0.0)),
                    tb,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                );
                let geo = te_temp.geometry(scale);
                let (tw, th) = (geo.size.w, geo.size.h);
                let (w, h) = (w, h);
                let pos_x = if anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::LEFT)
                    && anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::RIGHT)
                {
                    margin.left
                } else if anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::RIGHT) {
                    (w - tw - margin.right).max(margin.left)
                } else if anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::LEFT) {
                    margin.left
                } else {
                    ((w - tw) / 2).max(0)
                };
                let pos_y = if anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::TOP)
                    && anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::BOTTOM)
                {
                    margin.top
                } else if anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::BOTTOM) {
                    (h - th - margin.bottom).max(margin.top)
                } else if anchor.contains(smithay::wayland::shell::wlr_layer::Anchor::TOP) {
                    margin.top
                } else {
                    ((h - th) / 2).max(0)
                };
                let te = TextureRenderElement::from_texture_buffer(
                    Point::from((pos_x as f64, pos_y as f64)),
                    tb,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                );
                let tg = te.geometry(scale);
                <TextureRenderElement<GlesTexture> as RenderElement<GlesRenderer>>::draw(
                    &te,
                    &mut frame,
                    te.src(),
                    tg,
                    &[tg],
                    &[],
                )?;
            }
        }
    }
    // If a DnD session is active with a drag icon, render it
    // at the current pointer position as an overlay.
    if state.dnd_active {
        if let Some(ref icon_surface) = state.dnd_icon {
            let icon_buf: Option<WlBuffer> = with_states(icon_surface, |states| {
                match states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .buffer
                {
                    Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
                    Some(BufferAssignment::Removed) => None,
                    _ => None,
                }
            });
            if let Some(buf) = icon_buf {
                if let Some(tb) = state.texture_cache.get(&buf.id()) {
                    let icon_x = state.pointer_x as i32;
                    let icon_y = state.pointer_y as i32;
                    let te = TextureRenderElement::from_texture_buffer(
                        Point::from((icon_x as f64, icon_y as f64)),
                        tb,
                        None,
                        None,
                        None,
                        Kind::Unspecified,
                    );
                    let tg = te.geometry(scale);
                    <TextureRenderElement<GlesTexture> as RenderElement<GlesRenderer>>::draw(
                        &te,
                        &mut frame,
                        te.src(),
                        tg,
                        &[tg],
                        &[],
                    )?;
                }
            }
        }
    }
    let _ = frame.finish()?;
    Ok(())
}

/// Render lock surfaces from the texture cache.
/// Texture import happens before frame creation (see `render_scene_into`).
fn render_lock_surfaces(
    state: &mut State,
    frame: &mut GlesFrame<'_, '_>,
    scale: smithay::utils::Scale<f64>,
) -> Result<()> {
    use smithay::backend::renderer::element::Kind;

    for lock_surface in &state.lock_surfaces {
        let buf: Option<WlBuffer> = with_states(lock_surface.wl_surface(), |states| {
            match states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .buffer
            {
                Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
                Some(BufferAssignment::Removed) => None,
                _ => None,
            }
        });
        if let Some(buf) = buf {
            if let Some(tb) = state.texture_cache.get(&buf.id()) {
                let te = TextureRenderElement::from_texture_buffer(
                    Point::from((0.0, 0.0)),
                    tb,
                    None,
                    None,
                    None,
                    Kind::Unspecified,
                );
                let tg = te.geometry(scale);
                <TextureRenderElement<GlesTexture> as RenderElement<GlesRenderer>>::draw(
                    &te,
                    frame,
                    te.src(),
                    tg,
                    &[tg],
                    &[],
                )?;
            }
        }
    }
    Ok(())
}
