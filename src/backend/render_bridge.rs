//! Temporary helpers for the WGPU rendering path.
//!
//! `popup_render_id` assigns a render-ID namespace for popup surfaces
//! staged into the WGPU scene graph via `update_window_texture`.

/// Return `true` when the current frame has actual scene content.
#[cfg(test)]
pub(super) fn should_render(
    has_tiled_windows: bool,
    has_floating_windows: bool,
    committed_popup_count: usize,
) -> bool {
    has_tiled_windows || has_floating_windows || committed_popup_count > 0
}

/// Temporary render-ID namespace for popup surfaces staged into the WGPU
/// scene graph.
pub(super) fn popup_render_id(popup_id: u32) -> u64 {
    0x8000_0000 + popup_id as u64
}
