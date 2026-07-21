//! Rendering for the Smithay winit/GLES backend.
//!
//! Contains `AxiomSmithayBackendReal::render` and its scene-prep helper. A
//! submodule of `backend` can read the private fields of `State` and
//! `AxiomSmithayBackendReal` (descendant modules see ancestor privates), so
//! no fields were made public for this move.

use crate::decoration::{DecorationMode, WindowDecoration};
use crate::window::Rectangle as WindowRectangle;
use anyhow::Result;
use log::{debug, warn};
use smithay::backend::renderer::gles::{ffi, GlesRenderer, GlesTarget, GlesTexture};
use smithay::backend::renderer::{
    Color32F, Frame, ImportAll, Renderer,
    element::{
        Element, RenderElement, Kind,
        solid::{SolidColorBuffer, SolidColorRenderElement},
        texture::{TextureBuffer, TextureRenderElement},
    },
};
use smithay::utils::{Point, Rectangle, Size, Transform};
use smithay::wayland::compositor::{with_states, BufferAssignment, SurfaceAttributes};
use std::collections::HashMap;
use wayland_server::backend::ObjectId;
use wayland_server::protocol::wl_buffer::WlBuffer;
use wayland_server::Resource;

use super::{AxiomSmithayBackendReal, State};

impl State {
    /// Calculate workspace layouts, synchronize window geometry, and notify
    /// Wayland clients of size changes. Shared by nested and DRM render paths.
    fn prepare_render_scene(&mut self) -> HashMap<u64, WindowRectangle> {
        let layouts = self
            .workspace_manager
            .read()
            .calculate_workspace_layouts();

        {
            let mut wm = self.window_manager.write();
            for (window_id, layout_rect) in &layouts {
                if let Some(window) = wm.get_window_mut(*window_id) {
                    if !window.properties.floating {
                        window.window.set_position(layout_rect.x, layout_rect.y);
                        window
                            .window
                            .set_size(layout_rect.width, layout_rect.height);
                    }
                }
            }
        }

        for (window_id, rect) in &layouts {
            if let Some(&surface_id) = self.window_map.get(window_id) {
                if let Some(toplevel) = self.toplevels.get(&surface_id) {
                    self.update_surface_fractional_scale(toplevel.wl_surface());
                    let scale = self
                        .workspace_manager
                        .read()
                        .scale_factor_for_window(*window_id);
                    let new_w = ((rect.width as f64 / scale).round() as i32).max(1);
                    let new_h = ((rect.height as f64 / scale).round() as i32).max(1);

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
                        self.configured_sizes
                            .insert(surface_id, (new_w, new_h));
                        self.pending_configure.insert(surface_id);

                        debug!(
                            "📐 Configured surface {} to {}x{}",
                            surface_id, new_w, new_h
                        );
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
            let (mut renderer, mut framebuffer) = backend.bind()?;
            render_scene_into(&mut self.state, &mut renderer, &mut framebuffer)?;
        }
        backend.submit(None)?;
        backend.window().pre_present_notify();
        Ok(())
    }

    /// Read back the current composited frame as RGBA bytes (Winit backend only).
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
        let (mut renderer, mut framebuffer) = backend.bind().ok()?;
        // Re-composite into the back buffer without presenting, so the bytes we
        // read are the freshly-drawn frame.
        render_scene_into(&mut self.state, &mut renderer, &mut framebuffer).ok()?;

        let w = self.state.window_width;
        let h = self.state.window_height;
        let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
        renderer
            .with_context(|gl| {
                unsafe {
                    gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, 0);
                    gl.ReadBuffer(ffi::BACK);
                    gl.ReadPixels(
                        0,
                        0,
                        w as i32,
                        h as i32,
                        ffi::RGBA,
                        ffi::UNSIGNED_BYTE,
                        buf.as_mut_ptr() as *mut std::ffi::c_void,
                    );
                }
            })
            .ok()?;
        Some((w, h, buf))
    }
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

    // Collect data BEFORE borrowing winit_backend mutably via bind().
    let mut items: Vec<(u64, WindowRectangle, Option<WindowDecoration>)> = Vec::new();
    for (window_id, rect) in &layouts {
        if let Some(&surface_id) = state.window_map.get(window_id) {
            if state.toplevels.contains_key(&surface_id) {
                let dec = state
                    .decoration_manager
                    .read()
                    .get_decoration(*window_id)
                    .cloned();
                items.push((*window_id, rect.clone(), dec));
            }
        }
    }
    let decorations: Vec<(u64, DecorationMode, bool)> = state
        .decoration_manager
        .read()
        .decorations()
        .iter()
        .map(|(id, d)| (*id, d.mode, d.focused))
        .collect();

    let (w, h) = (state.window_width as i32, state.window_height as i32);

    // Import client buffers FIRST (before the frame, to avoid double-borrowing renderer).
    // Cache imported textures by the buffer's ObjectId: a client's buffer
    // pool (e.g. double-buffering) is uploaded to the GPU exactly once
    // and reused across frames instead of re-importing every tick.
    let mut client_textures: HashMap<u64, ObjectId> = HashMap::new();
    for (window_id, _rect, _dec) in &items {
        if let Some(&surface_id) = state.window_map.get(window_id) {
            if let Some(t) = state.toplevels.get(&surface_id) {
                let buf: Option<WlBuffer> = with_states(t.wl_surface(), |states| {
                    match states.cached_state.get::<SurfaceAttributes>().current().buffer {
                        Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
                        _ => None,
                    }
                });
                if let Some(buf) = buf {
                    let bid = buf.id();
                    if state.texture_cache.get(&bid).is_none() {
                        match renderer.import_buffer(&buf, None, &[]) {
                            Some(Ok(tex)) => {
                                let tb = TextureBuffer::from_texture(
                                    &*renderer, tex, 1, Transform::Normal, None,
                                );
                                state.texture_cache.insert(bid.clone(), tb);
                            }
                            Some(Err(e)) => {
                                warn!(
                                    "⚠️ Failed to import buffer for surface {}: {:?}",
                                    surface_id, e
                                );
                            }
                            None => {
                                debug!(
                                    "Skipping non-texture buffer for surface {}",
                                    surface_id
                                );
                            }
                        }
                    }
                    if state.texture_cache.contains_key(&bid) {
                        client_textures.insert(*window_id, bid);
                    }
                }
            }
        }
    }
    // Import DnD icon texture before frame creation so renderer is available.
    let _dnd_bid: Option<ObjectId> = if state.dnd_active {
        state.dnd_icon.as_ref().and_then(|icon_surface| {
            let icon_buf: Option<WlBuffer> = with_states(icon_surface, |states| {
                match states.cached_state.get::<SurfaceAttributes>().current().buffer {
                    Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
                    _ => None,
                }
            });
            icon_buf.and_then(|buf| {
                let bid = buf.id();
                if state.texture_cache.get(&bid).is_none() {
                    match renderer.import_buffer(&buf, None, &[]) {
                        Some(Ok(tex)) => {
                            let tb = TextureBuffer::from_texture(
                                &*renderer, tex, 1, Transform::Normal, None,
                            );
                            state.texture_cache.insert(bid.clone(), tb);
                        }
                        Some(Err(e)) => warn!("⚠️ Failed to import DnD icon buffer: {:?}", e),
                        None => {}
                    }
                }
                Some(bid)
            })
        })
    } else {
        None
    };
    let mut frame =
        renderer.render(framebuffer, Size::from((w, h)), Transform::Normal)?;
    frame.clear(
        Color32F::from([0.05f32, 0.05, 0.08, 1.0]),
        &[Rectangle::new(Point::from((0, 0)), Size::from((w, h)))],
    )?;
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
        let g = bg_elem.geometry(smithay::utils::Scale::from(1.0));
        <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
            &bg_elem, &mut frame, bg_elem.src(), g, &[g], &[],
        )?;
        if let Some(bid) = client_textures.get(window_id) {
            if let Some(tb) = state.texture_cache.get(bid) {
            let te = TextureRenderElement::from_texture_buffer(
                Point::from((content.x as f64, content.y as f64)),
                tb,
                None,
                None,
                None,
                Kind::Unspecified,
            );
            let tg = te.geometry(smithay::utils::Scale::from(1.0));
            <TextureRenderElement<GlesTexture> as RenderElement<GlesRenderer>>::draw(
                &te, &mut frame, te.src(), tg, &[tg], &[],
            )?;
            }
        }
    }
    // SSD decorations: titlebar + 3 buttons as solid rects.
    for (window_id, mode, focused) in &decorations {
        if *mode == DecorationMode::ServerSide {
            if let Some(rect) = layouts.get(window_id) {
                let titlebar_h = 30;
                let tb_color: [f32; 4] = if *focused {
                    [0.3, 0.3, 0.5, 1.0]
                } else {
                    [0.2, 0.2, 0.3, 1.0]
                };
                let tb = SolidColorBuffer::new((rect.width as i32, titlebar_h), tb_color);
                let tb_elem = SolidColorRenderElement::from_buffer(
                    &tb,
                    Point::from((rect.x, rect.y)),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                );
                let tg = tb_elem.geometry(smithay::utils::Scale::from(1.0));
                <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
                    &tb_elem, &mut frame, tb_elem.src(), tg, &[tg], &[],
                )?;
                for (off, col) in [
                    (5i32, [0.8, 0.2, 0.2, 1.0]),
                    (30, [0.2, 0.6, 0.8, 1.0]),
                    (55, [0.2, 0.8, 0.2, 1.0]),
                ] {
                    let bx = rect.x + rect.width as i32 - 20 - off;
                    let b = SolidColorBuffer::new((20, 20), col);
                    let be = SolidColorRenderElement::from_buffer(
                        &b,
                        Point::from((bx, rect.y + 5)),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    );
                    let bg2 = be.geometry(smithay::utils::Scale::from(1.0));
                    <SolidColorRenderElement as RenderElement<GlesRenderer>>::draw(
                        &be, &mut frame, be.src(), bg2, &[bg2], &[],
                    )?;
                }
            }
        }
    }
    // If a DnD session is active with a drag icon, render it
    // at the current pointer position as an overlay.
    if state.dnd_active {
        if let Some(ref icon_surface) = state.dnd_icon {
            let icon_buf: Option<WlBuffer> = with_states(icon_surface, |states| {
                match states.cached_state.get::<SurfaceAttributes>().current().buffer {
                    Some(BufferAssignment::NewBuffer(ref b)) => Some(b.clone()),
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
                    let tg = te.geometry(smithay::utils::Scale::from(1.0));
                    <TextureRenderElement<GlesTexture> as RenderElement<GlesRenderer>>::draw(
                        &te, &mut frame, te.src(), tg, &[tg], &[],
                    )?;
                }
            }
        }
    }
    let _ = frame.finish()?;
    Ok(())
}
