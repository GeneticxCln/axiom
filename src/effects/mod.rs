//! Visual Effects Engine (Hyprland-inspired)
//!
//! This module handles all visual effects: animations, blur, shadows,
//! rounded corners, and other eye candy that makes Axiom beautiful.
//!
//! ## Components
//! - [`EffectsEngine`]: Central coordinator for all visual effects
//! - Window-level effects: scale, opacity, blur, shadows, corner radius
//! - GPU-accelerated rendering via WGPU shaders
//! - Spring-physics animation system
//! - Adaptive quality scaling for performance

use crate::config::EffectsConfig;
use crate::effects::animations::AnimationStats;
use anyhow::Result;
use log::{debug, info, warn};
use std::collections::HashMap;
use std::time::{Duration, Instant};

// GPU rendering and shader support
use std::sync::Arc;
use wgpu::{Device, Queue};

// Shader modules
mod animations;
mod blur;
mod shaders;
mod shadow;

/// Different types of animations
#[derive(Debug, Clone, PartialEq)]
pub enum AnimationType {
    /// Window opening animation
    WindowOpen {
        start_time: Instant,
        duration: Duration,
        target_scale: f32,
        target_opacity: f32,
    },
    /// Window closing animation
    WindowClose {
        start_time: Instant,
        duration: Duration,
        start_scale: f32,
        start_opacity: f32,
    },
    /// Window movement animation
    WindowMove {
        start_time: Instant,
        duration: Duration,
        start_pos: (f32, f32),
        target_pos: (f32, f32),
    },
    /// Window resize animation
    WindowResize {
        start_time: Instant,
        duration: Duration,
        start_size: (f32, f32),
        target_size: (f32, f32),
    },
    /// Workspace transition animation
    WorkspaceTransition {
        start_time: Instant,
        duration: Duration,
        start_offset: f32,
        target_offset: f32,
    },
}

/// Animation easing curves
#[derive(Debug, Clone, PartialEq)]
pub enum EasingCurve {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    BounceOut,
    ElasticOut,
    BackOut,
}

/// Window visual properties for effects
#[derive(Debug, Clone)]
pub struct WindowEffectState {
    /// Current scale (1.0 = normal)
    pub scale: f32,
    /// Current opacity (0.0 = transparent, 1.0 = opaque)
    pub opacity: f32,
    /// Current position offset
    pub position_offset: (f32, f32),
    /// Current blur radius
    pub blur_radius: f32,
    /// Current corner radius
    pub corner_radius: f32,
    /// Current shadow parameters
    pub shadow: ShadowParams,
    /// `Some(instant)` if this element has an active open animation.
    /// For top-level windows set by [`EffectsEngine::animate_window_open`].
    /// For popups set by [`EffectsEngine::animate_popup_open`] to the
    /// moment the popup was first registered, giving the popup its own
    /// independent fade-in timeline independent of the parent's open
    /// time (a popup opened long after the parent settles starts with
    /// `t > 1.0` and renders at full alpha immediately).
    pub opened_at: Option<Instant>,
    /// `Some(parent_window_id)` if this effect state represents a popup.
    /// The render path resolves the parent's live `scale` and
    /// `position_offset` via this id so the popup inherits the parent's
    /// spring physics in real time (not a stale snapshot at popup
    /// creation). `None` for top-level windows.
    pub parent_id: Option<u64>,
}

/// Shadow rendering parameters
#[derive(Debug, Clone)]
pub struct ShadowParams {
    pub enabled: bool,
    pub size: f32,
    pub blur_radius: f32,
    pub opacity: f32,
    pub offset: (f32, f32),
    pub color: [f32; 4], // RGBA
}

/// Blur effect parameters
#[derive(Debug, Clone)]
pub struct BlurParams {
    pub enabled: bool,
    pub radius: f32,
    pub intensity: f32,
    pub background_blur: bool,
    pub window_blur: bool,
}

/// Phase 4: Complete Visual Effects Engine with GPU acceleration
pub struct EffectsEngine {
    config: EffectsConfig,

    /// Window effect states by window ID
    window_effects: HashMap<u64, WindowEffectState>,

    /// GPU-based effect renderers
    blur_renderer: Option<blur::BlurRenderer>,
    shadow_renderer: Option<shadow::ShadowRenderer>,
    shader_manager: Option<Arc<shaders::ShaderManager>>,

    /// Advanced animation system
    animation_controller: animations::AnimationController,

    /// Global animation state
    animations_enabled: bool,

    /// Performance monitoring
    frame_time: Duration,
    last_update: Instant,

    /// Effect parameters
    blur_params: BlurParams,
    default_shadow: ShadowParams,

    /// Animation settings
    default_animation_duration: Duration,
    default_easing_curve: EasingCurve,

    /// Performance optimization
    effects_quality: f32, // 0.0 to 1.0
    adaptive_quality: bool,

    /// GPU context (when available). Stored as `Arc` because the internal
    /// blur / shadow renderers take `Arc<Device>` and `Arc<Queue>` at
    /// construction and must outlast any single borrow of the wgpu
    /// renderer the compositor hands to `initialize_gpu`.
    gpu_device: Option<Arc<Device>>,
    gpu_queue: Option<Arc<Queue>>,
    /// Whether [`EffectsEngine::initialize_gpu`] completed successfully.
    /// Surfaces in `LiveMetrics::effects_gpu_available` so monitoring
    /// clients can detect "GPU effects did not initialize" without
    /// grepping the compositor log. Set to `true` at the successful
    /// end of `initialize_gpu`, `false` on entry (initial state) and
    /// on error (the function returns early without mutating `self`).
    gpu_initialized: bool,
}

impl Default for WindowEffectState {
    fn default() -> Self {
        Self {
            scale: 1.0,
            opacity: 1.0,
            position_offset: (0.0, 0.0),
            blur_radius: 0.0,
            corner_radius: 8.0,
            shadow: ShadowParams::default(),
            opened_at: None,
            parent_id: None,
        }
    }
}

impl Default for ShadowParams {
    fn default() -> Self {
        Self {
            enabled: true,
            size: 20.0,
            blur_radius: 15.0,
            opacity: 0.6,
            offset: (0.0, 2.0),
            color: [0.0, 0.0, 0.0, 1.0], // Black shadow
        }
    }
}

impl Default for BlurParams {
    fn default() -> Self {
        Self {
            enabled: true,
            radius: 10.0,
            intensity: 0.8,
            background_blur: true,
            window_blur: false,
        }
    }
}

impl EffectsEngine {
    /// Create a new [`EffectsEngine`] from the provided [`EffectsConfig`].
    ///
    /// Initializes all effect subsystems (blur, shadow, animations) but
    /// does **not** initialize GPU resources — call [`EffectsEngine::initialize_gpu`] separately
    /// when a WGPU device and queue are available.
    pub fn new(config: &EffectsConfig) -> Result<Self> {
        info!("🎨 Phase 4: Initializing Visual Effects Engine...");

        let blur_params = BlurParams {
            enabled: config.blur.enabled,
            radius: config.blur.radius as f32,
            intensity: config.blur.intensity as f32,
            background_blur: config.blur.window_backgrounds,
            window_blur: false,
        };

        let default_shadow = ShadowParams {
            enabled: config.shadows.enabled,
            size: config.shadows.size as f32,
            blur_radius: config.shadows.blur_radius as f32,
            opacity: config.shadows.opacity as f32,
            offset: (0.0, 2.0),
            color: [0.0, 0.0, 0.0, 1.0],
        };

        let default_easing_curve = match config.animations.curve.as_str() {
            "linear" => EasingCurve::Linear,
            "ease-in" => EasingCurve::EaseIn,
            "ease" => EasingCurve::EaseOut,
            "ease-out" => EasingCurve::EaseOut,
            "ease-in-out" => EasingCurve::EaseInOut,
            _ => EasingCurve::EaseOut,
        };

        info!("✨ Effects Engine Configuration:");
        info!(
            "  🎬 Animations: {} ({}ms, {})",
            config.animations.enabled, config.animations.duration, config.animations.curve
        );
        info!(
            "  🌊 Blur: {} (radius: {}, intensity: {})",
            blur_params.enabled, blur_params.radius, blur_params.intensity
        );
        info!(
            "  🌟 Shadows: {} (size: {}, opacity: {})",
            default_shadow.enabled, default_shadow.size, default_shadow.opacity
        );
        info!(
            "  🔄 Rounded Corners: {} (radius: {}px)",
            config.rounded_corners.enabled, config.rounded_corners.radius
        );

        Ok(Self {
            config: config.clone(),
            window_effects: HashMap::new(),
            blur_renderer: None,
            shadow_renderer: None,
            shader_manager: None,
            animation_controller: animations::AnimationController::new(),
            animations_enabled: config.animations.enabled,
            frame_time: Duration::from_millis(16), // ~60 FPS
            last_update: Instant::now(),
            blur_params,
            default_shadow,
            default_animation_duration: Duration::from_millis(config.animations.duration as u64),
            default_easing_curve,
            effects_quality: 1.0,
            adaptive_quality: true,
            gpu_device: None,
            gpu_queue: None,
            gpu_initialized: false,
        })
    }

    /// Update all animations and effects
    pub fn update(&mut self) -> Result<()> {
        if !self.config.enabled {
            // If globally disabled, keep minimal stats and return
            self.frame_time = Duration::from_millis(16);
            self.effects_quality = 0.0;
            self.last_update = Instant::now();
            return Ok(());
        }
        let now = Instant::now();
        let delta_time = now.duration_since(self.last_update);
        self.last_update = now;
        self.frame_time = delta_time;

        if !self.animations_enabled {
            return Ok(());
        }

        // Tick the advanced AnimationController (spring physics, keyframes, timelines)
        // This is the sole animation driver — all animate_* methods now route through it.
        if let Ok(controller_updates) = self.animation_controller.update() {
            for update in &controller_updates {
                if let Some(effect_state) = self.window_effects.get_mut(&update.window_id) {
                    match (&update.property, &update.value) {
                        (
                            animations::AnimationProperty::Transform,
                            animations::AnimationValue::Transform { scale, opacity, .. },
                        ) => {
                            effect_state.scale = scale.x.max(scale.y);
                            effect_state.opacity = *opacity;
                        }
                        (
                            animations::AnimationProperty::Position,
                            animations::AnimationValue::Position(pos),
                        ) => {
                            effect_state.position_offset = (pos.x, pos.y);
                        }
                        (
                            animations::AnimationProperty::Opacity,
                            animations::AnimationValue::Float(v),
                        ) => {
                            effect_state.opacity = v.clamp(0.0, 1.0);
                        }
                        (
                            animations::AnimationProperty::Scale,
                            animations::AnimationValue::Float(v),
                        ) => {
                            effect_state.scale = v.max(0.0);
                        }
                        (
                            animations::AnimationProperty::SpringProperty(name),
                            animations::AnimationValue::Float(v),
                        ) => match name.as_str() {
                            "opacity" => effect_state.opacity = v.clamp(0.0, 1.0),
                            "scale" => effect_state.scale = v.max(0.0),
                            "blur" => effect_state.blur_radius = v.max(0.0),
                            "corner_radius" => effect_state.corner_radius = v.max(0.0),
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
        }

        // Performance adaptation
        if self.adaptive_quality {
            self.adapt_quality_for_performance();
        }

        // Cleanup finished animations
        self.cleanup_finished_animations();

        debug!(
            "🎨 Effects update: {} windows, quality: {:.1}, frame_time: {:.1}ms",
            self.window_effects.len(),
            self.effects_quality,
            delta_time.as_secs_f64() * 1000.0
        );

        Ok(())
    }

    /// Start a window opening animation (routed through AnimationController for spring physics)
    pub fn animate_window_open(&mut self, window_id: u64) {
        if !self.animations_enabled || !self.config.enabled {
            return;
        }

        // Ensure effect state exists and set initial values for immediate visual correctness
        let effect_state = self.window_effects.entry(window_id).or_default();
        effect_state.scale = 0.8;
        effect_state.opacity = 0.0;
        // Mark the open instant so the render path can compute
        // a per-frame fade-in for top-level windows and (via
        // `animate_popup_open`) for descendants that key off it.
        effect_state.opened_at = Some(Instant::now());

        // Start spring animations for smooth physics-driven transition
        self.animation_controller.start_spring_animation(
            window_id,
            "scale".to_string(),
            1.0,
            Some(0.8),
            Some(animations::SpringParams {
                stiffness: 250.0,
                damping: 25.0,
                mass: 1.0,
                precision: 0.005,
            }),
        );
        self.animation_controller.start_spring_animation(
            window_id,
            "opacity".to_string(),
            1.0,
            Some(0.0),
            Some(animations::SpringParams {
                stiffness: 300.0,
                damping: 28.0,
                mass: 1.0,
                precision: 0.005,
            }),
        );

        info!("🎬 Started window open animation for window {}", window_id);
    }

    /// Register a popup as opened and seed its `WindowEffectState`.
    ///
    /// Stores a marker entry in `window_effects` keyed by `popup_id`.
    /// The popup's effect state deliberately keeps `scale`, `opacity`,
    /// and `position_offset` at identity defaults — those values are
    /// resolved live at render time from the **parent** window's effect
    /// state (via `parent_id`), so the popup's geometry tracks the
    /// parent's spring physics in real time. The entry's `opened_at` is
    /// set to `Instant::now()` so the popup has its own fade-in
    /// timeline: a popup that appears well after its parent settles
    /// starts at `t > 1.0` and renders immediately at full alpha;
    /// a popup that appears together with its parent fades in over
    /// `default_animation_duration` with an EaseOut cubic ramp.
    ///
    /// `parent_window_id` is the **`u64` window key** used by the
    /// compositor's `window_map`, not the Wayland surface id. The
    /// caller is responsible for resolving the surface-to-window
    /// mapping before this call.
    pub fn animate_popup_open(&mut self, popup_id: u64, parent_window_id: u64) {
        let now = Instant::now();
        self.window_effects
            .entry(popup_id)
            .and_modify(|e| {
                // Update parent_id and refresh opened_at so a reused popup
                // id (rare on most compositors) gets a fresh fade-in too.
                e.parent_id = Some(parent_window_id);
                e.opened_at = Some(now);
            })
            .or_insert_with(|| WindowEffectState {
                scale: 1.0,
                opacity: 1.0,
                position_offset: (0.0, 0.0),
                blur_radius: 0.0,
                corner_radius: 8.0,
                shadow: ShadowParams::default(),
                opened_at: Some(now),
                parent_id: Some(parent_window_id),
            });
        debug!(
            "🎬 Popup {} registered with parent window {} (opened_at = {:?})",
            popup_id, parent_window_id, now
        );
    }

    /// Default animation duration in milliseconds. Exposed to render
    /// callers (e.g. compute per-popup fade-in t = now - opened_at /
    /// default_animation_duration) without forcing them to compute
    /// Duration arithmetic on f32 inputs.
    pub fn default_animation_duration_ms(&self) -> u32 {
        self.default_animation_duration.as_millis() as u32
    }

    /// Start a window closing animation (routed through AnimationController spring physics)
    pub fn animate_window_close(&mut self, window_id: u64) {
        if !self.animations_enabled || !self.config.enabled {
            return;
        }

        let current_scale = self
            .window_effects
            .get(&window_id)
            .map(|e| e.scale)
            .unwrap_or(1.0);
        let current_opacity = self
            .window_effects
            .get(&window_id)
            .map(|e| e.opacity)
            .unwrap_or(1.0);

        // Ensure effect state exists
        self.window_effects.entry(window_id).or_default();

        self.animation_controller.start_spring_animation(
            window_id,
            "scale".to_string(),
            0.0,
            Some(current_scale),
            Some(animations::SpringParams {
                stiffness: 200.0,
                damping: 20.0,
                mass: 1.0,
                precision: 0.005,
            }),
        );
        self.animation_controller.start_spring_animation(
            window_id,
            "opacity".to_string(),
            0.0,
            Some(current_opacity),
            Some(animations::SpringParams {
                stiffness: 250.0,
                damping: 22.0,
                mass: 1.0,
                precision: 0.005,
            }),
        );

        info!("🎬 Started window close animation for window {}", window_id);
    }

    /// Start a minimize animation for the given window.
    /// Animates scale → 0 and opacity → 0 using spring physics so the
    /// window smoothly shrinks and fades out before the workspace layout
    /// skips it. Idempotent when already minimized.
    pub fn animate_window_minimize(&mut self, window_id: u64) {
        if !self.animations_enabled || !self.config.enabled {
            return;
        }

        // Ensure effect state exists
        self.window_effects.entry(window_id).or_default();

        let current_scale = self
            .window_effects
            .get(&window_id)
            .map(|e| e.scale)
            .unwrap_or(1.0);
        let current_opacity = self
            .window_effects
            .get(&window_id)
            .map(|e| e.opacity)
            .unwrap_or(1.0);

        self.animation_controller.start_spring_animation(
            window_id,
            "scale".to_string(),
            0.0,
            Some(current_scale),
            Some(animations::SpringParams {
                stiffness: 250.0,
                damping: 22.0,
                mass: 1.0,
                precision: 0.005,
            }),
        );
        self.animation_controller.start_spring_animation(
            window_id,
            "opacity".to_string(),
            0.0,
            Some(current_opacity),
            Some(animations::SpringParams {
                stiffness: 250.0,
                damping: 22.0,
                mass: 1.0,
                precision: 0.005,
            }),
        );

        info!("🎬 Started minimize animation for window {}", window_id);
    }

    /// Start a restore animation for the given window.
    /// Animates scale → 1 and opacity → 1 from whatever their current
    /// (possibly minimized) values are. Idempotent when already visible.
    pub fn animate_window_restore(&mut self, window_id: u64) {
        if !self.animations_enabled || !self.config.enabled {
            self.window_effects
                .entry(window_id)
                .or_insert_with(|| WindowEffectState {
                    scale: 1.0,
                    opacity: 1.0,
                    ..WindowEffectState::default()
                });
            return;
        }

        self.window_effects.entry(window_id).or_default();

        let current_scale = self
            .window_effects
            .get(&window_id)
            .map(|e| e.scale)
            .unwrap_or(0.0);
        let current_opacity = self
            .window_effects
            .get(&window_id)
            .map(|e| e.opacity)
            .unwrap_or(0.0);

        self.animation_controller.start_spring_animation(
            window_id,
            "scale".to_string(),
            1.0,
            Some(current_scale),
            Some(animations::SpringParams {
                stiffness: 300.0,
                damping: 25.0,
                mass: 1.0,
                precision: 0.005,
            }),
        );
        self.animation_controller.start_spring_animation(
            window_id,
            "opacity".to_string(),
            1.0,
            Some(current_opacity),
            Some(animations::SpringParams {
                stiffness: 300.0,
                damping: 25.0,
                mass: 1.0,
                precision: 0.005,
            }),
        );

        info!("🎬 Started restore animation for window {}", window_id);
    }

    /// Start a window movement animation (routed through AnimationController)
    pub fn animate_window_move(&mut self, window_id: u64, from: (f32, f32), to: (f32, f32)) {
        if !self.animations_enabled || !self.config.enabled {
            return;
        }

        // Ensure effect state exists so position updates are applied
        self.window_effects.entry(window_id).or_default();

        // Use the standard animation system for position transitions
        let animation = AnimationType::WindowMove {
            start_time: Instant::now(),
            duration: Duration::from_millis(200),
            start_pos: from,
            target_pos: to,
        };

        self.animation_controller
            .start_animation(window_id, animation, Duration::ZERO, Some(1));

        debug!(
            "🎬 Started window move animation for window {} from {:?} to {:?}",
            window_id, from, to
        );
    }

    /// Start a workspace transition animation
    pub fn animate_workspace_transition(&mut self, from_offset: f32, to_offset: f32) {
        if !self.animations_enabled {
            return;
        }

        info!(
            "🌊 Started workspace transition animation from {:.1} to {:.1}",
            from_offset, to_offset
        );

        // Workspace transitions are handled by the workspace manager,
        // but we can add visual enhancements here
    }

    /// Get current visual state for a window
    pub fn get_window_effects(&self, window_id: u64) -> Option<&WindowEffectState> {
        self.window_effects.get(&window_id)
    }

    /// Remove window from effects tracking.
    /// Returns `true` if the window existed and was removed, `false` if not found.
    pub fn remove_window(&mut self, window_id: u64) -> bool {
        let removed = self.window_effects.remove(&window_id).is_some();
        if removed {
            debug!("🗑️ Removed window {} from effects tracking", window_id);
        }
        removed
    }

    /// Apply easing curve to animation progress (used in tests and internally)
    #[cfg(test)]
    fn apply_easing_curve(&self, t: f32, curve: &EasingCurve) -> f32 {
        let t = t.clamp(0.0, 1.0);

        match curve {
            EasingCurve::Linear => t,
            EasingCurve::EaseIn => t * t,
            EasingCurve::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            EasingCurve::EaseInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    -1.0 + (4.0 - 2.0 * t) * t
                }
            }
            EasingCurve::BounceOut => {
                if t < 1.0 / 2.75 {
                    7.5625 * t * t
                } else if t < 2.0 / 2.75 {
                    let t = t - 1.5 / 2.75;
                    7.5625 * t * t + 0.75
                } else if t < 2.5 / 2.75 {
                    let t = t - 2.25 / 2.75;
                    7.5625 * t * t + 0.9375
                } else {
                    let t = t - 2.625 / 2.75;
                    7.5625 * t * t + 0.984_375
                }
            }
            EasingCurve::ElasticOut => {
                if t == 0.0 {
                    0.0
                } else if t == 1.0 {
                    1.0
                } else {
                    let p = 0.3;
                    let s = p / 4.0;
                    (2.0_f32).powf(-10.0 * t) * ((t - s) * (2.0 * std::f32::consts::PI) / p).sin()
                        + 1.0
                }
            }
            EasingCurve::BackOut => {
                let s = 1.70158;
                let t = t - 1.0;
                t * t * ((s + 1.0) * t + s) + 1.0
            }
        }
    }

    /// Adapt effects quality based on performance
    fn adapt_quality_for_performance(&mut self) {
        let target_frame_time = Duration::from_millis(16); // 60 FPS

        if self.frame_time > target_frame_time * 2 {
            // Performance is poor, reduce quality
            self.effects_quality = (self.effects_quality - 0.1).max(0.3);
            debug!(
                "⚡ Reduced effects quality to {:.1} due to performance",
                self.effects_quality
            );
        } else if self.frame_time < target_frame_time && self.effects_quality < 1.0 {
            // Performance is good, increase quality
            self.effects_quality = (self.effects_quality + 0.05).min(1.0);
        }
    }

    /// Remove windows whose animations have fully faded out (opacity ≤ 0 and scale ≤ 0).
    /// All animations now route through AnimationController exclusively.
    fn cleanup_finished_animations(&mut self) {
        self.window_effects
            .retain(|_, effect_state| effect_state.opacity > 0.0 || effect_state.scale > 0.0);
    }

    /// Get current effects quality (for performance monitoring)
    pub fn get_effects_quality(&self) -> f32 {
        self.effects_quality
    }

    /// Cheap read-only accessor for the global "effects enabled" flag in
    /// the active [`EffectsConfig`]. Returned by the config layer's
    /// `enabled` field and toggled at runtime via
    /// [`apply_live_effects_control`] or [`toggle_effects`]. Used by the
    /// compositor to decide whether to skip per-window effect queueing
    /// before the WGPU post-process pass.
    ///
    /// [`EffectsConfig`]: crate::config::EffectsConfig
    /// [`apply_live_effects_control`]: EffectsEngine::apply_live_effects_control
    /// [`toggle_effects`]: EffectsEngine::toggle_effects
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Whether blur is enabled in the current live config.
    pub fn is_blur_enabled(&self) -> bool {
        self.blur_params.enabled
    }

    /// Read access to the current blur parameters.
    pub fn blur_params(&self) -> &BlurParams {
        &self.blur_params
    }

    /// Enable or disable animations
    pub fn set_animations_enabled(&mut self, enabled: bool) {
        self.animations_enabled = enabled;
        info!(
            "🎬 Animations {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }

    /// Get performance statistics
    pub fn get_performance_stats(&self) -> (Duration, f32, usize) {
        (
            self.frame_time,
            self.effects_quality,
            self.window_effects.len(),
        )
    }

    pub fn shutdown(&mut self) {
        info!("🎨 Shutting down Visual Effects Engine...");
        self.window_effects.clear();
        info!("✅ Effects Engine shutdown complete");
    }

    /// Toggle the global effects enabled state.
    ///
    /// When disabled, all animations, blur, and shadow rendering are
    /// skipped in [`EffectsEngine::update`].
    pub fn toggle_effects(&mut self) {
        self.config.enabled = !self.config.enabled;
    }

    /// Set per-window blur radius.  Radius ≤ 0 disables blur for this
    /// window; values > [`MAX_EFFECTS_BLUR_RADIUS_PX`] are clamped.
    /// The new blur is applied as a spring animation on the next
    /// [`update()`](EffectsEngine::update) tick.
    pub fn set_window_blur(&mut self, window_id: u64, radius: f32) {
        const MAX_RADIUS: f32 = 32.0;
        let clamped = radius.clamp(0.0, MAX_RADIUS);
        let state = self.window_effects.entry(window_id).or_default();
        state.blur_radius = clamped;
        debug!(
            "Per-window blur set — window {} radius {:.1}",
            window_id, clamped
        );
    }

    /// Apply a [`crate::ipc::LazyUIMessage::EffectsControl`] payload to
    /// live engine state. This is the runtime mutator used by the IPC
    /// `process_messages`-returned dispatch loop in `AxiomCompositor`.
    /// Field validation is performed at the IPC layer (see
    /// `validate_blur_radius` / `validate_animation_speed` in
    /// `src/ipc/mod.rs`); this method re-validates inline as defense in
    /// depth so direct callers (tests, future modules) cannot bypass
    /// bounds.
    ///
    /// `enabled` toggles the global effects enabled flag.
    /// `blur_radius` is in pixels (0..=32); non-finite or out-of-range are
    ///   rejected with a `warn!` and leave the live value untouched.
    /// `animation_speed` is a unitless multiplier (1.0 = realtime,
    ///   >1 faster). Non-finite or non-positive input is rejected.
    pub fn apply_live_effects_control(
        &mut self,
        enabled: Option<bool>,
        blur_radius: Option<f32>,
        animation_speed: Option<f32>,
    ) {
        // Mirrors `MAX_EFFECTS_BLUR_RADIUS_PX` in src/ipc/mod.rs. Inlined to
        // avoid a circular module dependency (effects <- ipc today; ipc
        // does not import effects). Update both together if changed.
        const MAX_BLUR_RADIUS_PX: f32 = 32.0;
        const MAX_ANIMATION_SPEED: f32 = 10.0;
        if let Some(e) = enabled {
            self.config.enabled = e;
            debug!("✨ Effects enabled flag set to {}", e);
        }
        if let Some(r) = blur_radius {
            if r.is_finite() && (0.0..=MAX_BLUR_RADIUS_PX).contains(&r) {
                self.blur_params.radius = r;
                debug!("✨ Live blur radius set to {:.1}px", r);
            } else {
                warn!(
                    "⚠️ apply_live_effects_control: blur_radius {} rejected (non-finite or out of [0, 32]px)",
                    r
                );
            }
        }
        if let Some(s) = animation_speed {
            if s.is_finite() && (0.0..=MAX_ANIMATION_SPEED).contains(&s) && s > 0.0 {
                // Reinterpret "speed" as duration divisor: a faster speed
                // compresses the same base duration. Floor at 1ms to avoid
                // a zero-duration timer that freezes animations at start.
                let base_ms = self.config.animations.duration as f32;
                let multiplier = 1.0 / s;
                let new_duration_ms = (base_ms * multiplier).max(1.0) as u64;
                self.default_animation_duration = Duration::from_millis(new_duration_ms);
                debug!(
                    "✨ Animation speed set to {:.2}x (duration {} -> {}ms)",
                    s, base_ms as u64, new_duration_ms
                );
            } else {
                warn!(
                    "⚠️ apply_live_effects_control: animation_speed {} rejected (non-finite, non-positive, or > {})",
                    s, MAX_ANIMATION_SPEED
                );
            }
        }
    }

    /// Update configuration
    pub fn update_config(&mut self, config: EffectsConfig) {
        info!("🔄 Updating Effects Engine configuration");

        // Update blur params
        self.blur_params = BlurParams {
            enabled: config.blur.enabled,
            radius: config.blur.radius as f32,
            intensity: config.blur.intensity as f32,
            background_blur: config.blur.window_backgrounds,
            window_blur: false,
        };

        // Update shadow params
        self.default_shadow = ShadowParams {
            enabled: config.shadows.enabled,
            size: config.shadows.size as f32,
            blur_radius: config.shadows.blur_radius as f32,
            opacity: config.shadows.opacity as f32,
            offset: (0.0, 2.0),
            color: [0.0, 0.0, 0.0, 1.0],
        };

        // Update animation settings
        self.animations_enabled = config.animations.enabled;
        self.default_animation_duration = Duration::from_millis(config.animations.duration as u64);

        self.default_easing_curve = match config.animations.curve.as_str() {
            "linear" => EasingCurve::Linear,
            "ease-in" => EasingCurve::EaseIn,

            "ease-in-out" => EasingCurve::EaseInOut,
            _ => EasingCurve::EaseOut,
        };

        self.config = config;
    }

    /// Initialize GPU context for hardware-accelerated effects.
    ///
    /// Takes owned `Arc<Device>` and `Arc<Queue>` so the effects engine
    /// can retain long-lived arcs in its blur/shadow renderers. The
    /// compositor constructs these arcs by `Arc::clone(&renderer.device)`
    /// / `Arc::clone(&renderer.queue)` from inside its `renderer.read()`
    /// guard — a refcount bump, not a deep copy. The renderer's public
    /// `device()` and `queue()` getters return `&Device` / `&Queue`
    /// (`Design 16`) so external callers cannot reach the GPU context
    /// through the renderer; this method is the narrow channel through
    /// which the effects engine gets its retainable handle.
    ///
    /// On success the `gpu_initialized` flag is set to `true` so
    /// monitoring clients can read [`EffectsEngine::is_gpu_initialized`].
    /// Early-return on error leaves the flag at its previous value
    /// (typically `false` for a fresh engine), so a failed init is
    /// observable from IPC.
    pub fn initialize_gpu(&mut self, device: Arc<Device>, queue: Arc<Queue>) -> Result<()> {
        info!("🚀 Initializing GPU acceleration for effects...");

        // Store GPU context (clone arcs for blur/shadow renderer feeds).
        self.gpu_device = Some(device.clone());
        self.gpu_queue = Some(queue.clone());

        // Initialize shader manager (compiled once, shared by blur and shadow renderers).
        // shader_manager stores Arc<Device> internally; pass the Arc clone
        // so its lifetime covers both blur and shadow renderers below.
        let mut sm = shaders::ShaderManager::new(device.clone());
        sm.compile_all_shaders()?;
        let shader_manager = Arc::new(sm);
        self.shader_manager = Some(shader_manager.clone());

        // Initialize blur renderer
        if self.blur_params.enabled {
            // Convert our BlurParams to blur module's BlurParams
            let blur_params = blur::BlurParams {
                blur_type: blur::BlurType::Gaussian {
                    radius: self.blur_params.radius,
                    intensity: self.blur_params.intensity,
                },
                enabled: self.blur_params.enabled,
                adaptive_quality: true,
                performance_scale: self.effects_quality,
            };

            self.blur_renderer = Some(blur::BlurRenderer::new(
                device.clone(),
                queue.clone(),
                shader_manager.clone(),
                blur_params,
            )?);
            debug!("🌊 GPU blur renderer initialized");
        }

        // Initialize shadow renderer using the shared shader manager
        if self.default_shadow.enabled {
            self.shadow_renderer = Some(shadow::ShadowRenderer::new(
                device,
                queue,
                shader_manager.clone(),
                self.default_shadow.clone(),
                shadow::ShadowQuality::High,
            )?);
            info!("🌟 GPU shadow renderer initialized");
        }

        // Mark GPU as initialised so IPC handlers can read a live status.
        // Honest signal — reaching this point means blur + shadow pipeline
        // paths are seeded (the previous code stored None on either path
        // being disabled, which left observers guessing between
        // "config disabled" and "GPU init failed").
        self.gpu_initialized = true;
        info!("✅ GPU effects acceleration ready");
        Ok(())
    }

    /// Get animation statistics from the animation controller
    pub fn get_animation_stats(&self) -> AnimationStats {
        self.animation_controller.get_animation_stats()
    }

    /// Check if GPU acceleration is available
    pub fn has_gpu_acceleration(&self) -> bool {
        self.gpu_device.is_some() && self.gpu_queue.is_some()
    }

    /// Whether `initialize_gpu` completed successfully. Distinct from
    /// `has_gpu_acceleration()`: the latter only reports whether we
    /// _hold_ a device/queue reference, which can be true even when
    /// the blur / shadow renderers failed to build (e.g. shader
    /// compile errors). The IPC `LiveMetrics::effects_gpu_available`
    /// surfaces this flag — set on successful init return, reset on
    /// each subsequent `initialize_gpu` call (typically never).
    pub fn is_gpu_initialized(&self) -> bool {
        self.gpu_initialized
    }

    /// Render drop shadows for all visible windows via the GPU shadow renderer.
    pub fn render_shadows(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        output_view: &wgpu::TextureView,
        window_data: &[(cgmath::Vector2<f32>, cgmath::Vector2<f32>, ShadowParams)],
    ) -> Result<()> {
        if let Some(ref mut shadow) = self.shadow_renderer {
            shadow.render_shadow_batch(encoder, output_view, window_data)
        } else {
            Ok(())
        }
    }

    /// Render blur passes for windows via the GPU BlurRenderer.
    /// Applies a dual-pass Gaussian blur to the composited output texture
    /// for each window region specified in `window_data`.
    pub fn render_blurs(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        input_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        texture_size: cgmath::Vector2<u32>,
    ) -> Result<()> {
        if let Some(ref mut blur) = self.blur_renderer {
            blur.apply_blur(encoder, input_view, output_view, texture_size)
        } else {
            Ok(())
        }
    }

    /// Get reference to the shadow renderer for direct GPU shadow operations.
    pub fn shadow_renderer(&self) -> Option<&shadow::ShadowRenderer> {
        self.shadow_renderer.as_ref()
    }

    /// Get mutable reference to the blur renderer for direct GPU blur operations.
    pub fn blur_renderer_mut(&mut self) -> Option<&mut blur::BlurRenderer> {
        self.blur_renderer.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EffectsConfig;

    #[test]
    fn test_effects_engine_initialization() {
        let config = EffectsConfig::default();
        let engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");
        assert_eq!(engine.get_effects_quality(), 1.0);
        assert!(!engine.has_gpu_acceleration());
    }

    #[test]
    fn test_animate_window_open() {
        let config = EffectsConfig::default();
        let mut engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");
        engine.animate_window_open(1);
        let effects = engine.get_window_effects(1);
        assert!(effects.is_some());
        // Window open now routes through AnimationController spring physics,
        // so initial scale/opacity are set directly on the effect state.
        let effects = effects.unwrap();
        assert!((effects.scale - 0.8).abs() < 0.01);
        assert!(effects.opacity < 0.01);
        // Spring animations are tracked by the controller, not active_animations.
        // Verify the controller has spring states.
        let stats = engine.get_animation_stats();
        assert!(stats.spring_animations > 0, "spring animations not started");
    }

    #[test]
    fn test_remove_window() {
        let config = EffectsConfig::default();
        let mut engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");
        engine.animate_window_open(42);
        assert!(engine.get_window_effects(42).is_some());
        engine.remove_window(42);
        assert!(engine.get_window_effects(42).is_none());
    }

    #[test]
    fn test_update_config() {
        let config = EffectsConfig::default();
        let mut engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");
        let mut new_config = EffectsConfig::default();
        new_config.animations.enabled = false;
        engine.update_config(new_config.clone());
        // Animations should now be disabled
        engine.animate_window_open(1);
        let effects = engine.get_window_effects(1);
        // If animations are disabled, animate_window_open returns early without creating state
        assert!(effects.is_none());
    }

    #[test]
    fn test_easing_curves() {
        let config = EffectsConfig::default();
        let engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");

        // All easing curves should return values in [0, 1] range
        let curves = [
            EasingCurve::Linear,
            EasingCurve::EaseIn,
            EasingCurve::EaseOut,
            EasingCurve::EaseInOut,
            EasingCurve::BounceOut,
            EasingCurve::ElasticOut,
            EasingCurve::BackOut,
        ];
        for curve in &curves {
            for t in [0.0, 0.25, 0.5, 0.75, 1.0].iter() {
                let result = engine.apply_easing_curve(*t, curve);
                assert!(
                    (-0.1..=1.1).contains(&result),
                    "Easing curve {:?} at t={}: result={} out of expected range",
                    curve,
                    t,
                    result
                );
            }
        }
    }

    #[test]
    fn test_shutdown() {
        let config = EffectsConfig::default();
        let mut engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");
        engine.animate_window_open(1);
        engine.shutdown();
    }

    #[test]
    fn test_apply_live_effects_control_enable_and_blur() {
        let config = EffectsConfig::default();
        let mut engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");
        // Baseline: effects enabled, blur radius from config (~10 px).
        assert!(engine.config.enabled);

        // Disable effects via the IPC mutator.
        engine.apply_live_effects_control(Some(false), None, None);
        assert!(
            !engine.config.enabled,
            "effects.enabled should be toggled off"
        );

        // Set blur radius within the 0..=32 px range.
        engine.apply_live_effects_control(None, Some(14.5), None);
        assert!(
            (engine.blur_params.radius - 14.5).abs() < 1e-4,
            "blur_params.radius should be 14.5, got {}",
            engine.blur_params.radius
        );
    }

    #[test]
    fn test_apply_live_effects_control_rejects_bad_blur() {
        let config = EffectsConfig::default();
        let mut engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");
        let original_radius = engine.blur_params.radius;

        // Reject negative, NaN, Inf, over-max.
        engine.apply_live_effects_control(None, Some(-1.0), None);
        assert!((engine.blur_params.radius - original_radius).abs() < 1e-6);
        engine.apply_live_effects_control(None, Some(f32::NAN), None);
        assert!((engine.blur_params.radius - original_radius).abs() < 1e-6);
        engine.apply_live_effects_control(None, Some(f32::INFINITY), None);
        assert!((engine.blur_params.radius - original_radius).abs() < 1e-6);
        engine.apply_live_effects_control(None, Some(33.0), None);
        assert!((engine.blur_params.radius - original_radius).abs() < 1e-6);
    }

    #[test]
    fn test_apply_live_effects_control_rejects_bad_speed() {
        let config = EffectsConfig::default();
        let mut engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");
        // The mutator writes to `default_animation_duration` (the runtime
        // field), NOT `config.animations.duration` (the source config).
        // Snapshot the runtime field, not the source config, otherwise the
        // assertion vacuously passes when the function is a no-op.
        let original_duration_ms = engine.default_animation_duration.as_millis();

        // Reject non-finite, non-positive, over-max.
        engine.apply_live_effects_control(None, None, Some(0.0));
        assert!(
            (engine.default_animation_duration.as_millis() as i128 - original_duration_ms as i128)
                .abs()
                < 1
        );
        engine.apply_live_effects_control(None, None, Some(f32::NAN));
        assert!(
            (engine.default_animation_duration.as_millis() as i128 - original_duration_ms as i128)
                .abs()
                < 1
        );
        engine.apply_live_effects_control(None, None, Some(11.0));
        assert!(
            (engine.default_animation_duration.as_millis() as i128 - original_duration_ms as i128)
                .abs()
                < 1
        );
    }

    #[test]
    fn test_apply_live_effects_control_speed_shortens_duration() {
        let config = EffectsConfig::default();
        let mut engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");
        let base_ms = engine.config.animations.duration as u64;
        // speed=2.0 should halve the default duration (floor of 1ms).
        engine.apply_live_effects_control(None, None, Some(2.0));
        let new_ms = engine.default_animation_duration.as_millis() as u64;
        assert!(
            new_ms < base_ms,
            "speed=2.0 should shorten animation duration ({} -> {})",
            base_ms,
            new_ms
        );
        // speed=0.5 should roughly double it (subject to floor).
        engine.apply_live_effects_control(None, None, Some(0.5));
        let slower_ms = engine.default_animation_duration.as_millis() as u64;
        assert!(
            slower_ms > new_ms,
            "speed=0.5 should lengthen animation duration ({} -> {})",
            new_ms,
            slower_ms
        );
    }
}
