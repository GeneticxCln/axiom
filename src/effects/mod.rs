//! Phase 4: Visual Effects Engine (Hyprland-inspired)
//!
//! This module handles all visual effects: animations, blur, shadows,
//! rounded corners, and other eye candy that makes Axiom beautiful.

use crate::config::EffectsConfig;
use anyhow::Result;
use log::{debug, info};
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

// Re-export animation types for use in effects engine
pub use animations::{
    AnimationController, AnimationProperty, AnimationStats, AnimationUpdate, AnimationValue,
};

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
    /// Active animations for this window
    pub active_animations: Vec<AnimationType>,
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

    /// GPU context (when available)
    gpu_device: Option<Arc<Device>>,
    gpu_queue: Option<Arc<Queue>>,
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
            active_animations: Vec::new(),
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
        })
    }

    /// Update all animations and effects
    pub fn update(&mut self) -> Result<()> {
        let now = Instant::now();
        let delta_time = now.duration_since(self.last_update);
        self.last_update = now;
        self.frame_time = delta_time;

        if !self.animations_enabled {
            return Ok(());
        }

        // Update all window animations
        // Update window animations - collect data first to avoid borrow conflicts
        let mut animation_updates = Vec::new();
        let window_ids: Vec<u64> = self.window_effects.keys().copied().collect();

        for window_id in window_ids {
            if let Some(effect_state) = self.window_effects.get_mut(&window_id) {
                if let Ok(updates) = Self::update_window_animations_static(
                    window_id,
                    effect_state,
                    now,
                    &self.default_easing_curve,
                ) {
                    animation_updates.extend(updates);
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

    /// Start a window opening animation
    pub fn animate_window_open(&mut self, window_id: u64) {
        if !self.animations_enabled {
            return;
        }

        let effect_state = self
            .window_effects
            .entry(window_id)
            .or_insert_with(WindowEffectState::default);

        // Start with small scale and transparent
        effect_state.scale = 0.8;
        effect_state.opacity = 0.0;

        let animation = AnimationType::WindowOpen {
            start_time: Instant::now(),
            duration: self.default_animation_duration,
            target_scale: 1.0,
            target_opacity: 1.0,
        };

        effect_state.active_animations.push(animation);

        info!("🎬 Started window open animation for window {}", window_id);
    }

    /// Start a window closing animation
    pub fn animate_window_close(&mut self, window_id: u64) {
        if !self.animations_enabled {
            return;
        }

        let effect_state = self
            .window_effects
            .entry(window_id)
            .or_insert_with(WindowEffectState::default);

        let animation = AnimationType::WindowClose {
            start_time: Instant::now(),
            duration: self.default_animation_duration,
            start_scale: effect_state.scale,
            start_opacity: effect_state.opacity,
        };

        effect_state.active_animations.push(animation);

        info!("🎬 Started window close animation for window {}", window_id);
    }

    /// Start a window movement animation
    pub fn animate_window_move(&mut self, window_id: u64, from: (f32, f32), to: (f32, f32)) {
        if !self.animations_enabled {
            return;
        }

        let effect_state = self
            .window_effects
            .entry(window_id)
            .or_insert_with(WindowEffectState::default);

        let animation = AnimationType::WindowMove {
            start_time: Instant::now(),
            duration: Duration::from_millis(200), // Faster for movement
            start_pos: from,
            target_pos: to,
        };

        effect_state.active_animations.push(animation);

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

    /// Apply blur effect to a window
    pub fn apply_blur_effect(&self, window_id: u64, _surface_data: &mut [u8]) {
        if !self.blur_params.enabled {
            return;
        }

        // In a real implementation, this would apply GPU-based blur
        // For now, we simulate the effect
        debug!(
            "🌊 Applying blur effect to window {} (radius: {:.1})",
            window_id, self.blur_params.radius
        );
    }

    /// Get current visual state for a window
    pub fn get_window_effects(&self, window_id: u64) -> Option<&WindowEffectState> {
        self.window_effects.get(&window_id)
    }

    /// Set blur radius for a specific window
    pub fn set_window_blur(&mut self, window_id: u64, blur_radius: f32) {
        let effect_state = self
            .window_effects
            .entry(window_id)
            .or_insert_with(WindowEffectState::default);

        effect_state.blur_radius = blur_radius.max(0.0); // Ensure non-negative

        debug!(
            "🌊 Set window {} blur radius to {:.1}",
            window_id, effect_state.blur_radius
        );
    }

    /// Remove window from effects tracking
    pub fn remove_window(&mut self, window_id: u64) {
        if let Some(_) = self.window_effects.remove(&window_id) {
            debug!("🗑️ Removed window {} from effects tracking", window_id);
        }
    }

    /// Static version of window animation updates to avoid borrow checker issues
    fn update_window_animations_static(
        window_id: u64,
        effect_state: &mut WindowEffectState,
        now: Instant,
        default_easing_curve: &EasingCurve,
    ) -> Result<Vec<String>> {
        let mut animations_to_remove = Vec::new();
        let mut animation_updates = Vec::new();

        for (i, animation) in effect_state.active_animations.iter().enumerate() {
            match animation {
                AnimationType::WindowOpen {
                    start_time,
                    duration,
                    target_scale,
                    target_opacity,
                } => {
                    let elapsed = now.duration_since(*start_time);

                    if elapsed >= *duration {
                        // Animation finished
                        effect_state.scale = *target_scale;
                        effect_state.opacity = *target_opacity;
                        animations_to_remove.push(i);
                        animation_updates
                            .push(format!("Window {} open animation completed", window_id));
                    } else {
                        // Update animation
                        let progress = elapsed.as_secs_f64() / duration.as_secs_f64();
                        let eased_progress =
                            Self::apply_easing_curve_static(progress as f32, default_easing_curve);

                        effect_state.scale = 0.8 + (target_scale - 0.8) * eased_progress;
                        effect_state.opacity = eased_progress * target_opacity;
                    }
                }

                AnimationType::WindowClose {
                    start_time,
                    duration,
                    start_scale,
                    start_opacity,
                } => {
                    let elapsed = now.duration_since(*start_time);

                    if elapsed >= *duration {
                        // Animation finished - window should be removed
                        effect_state.scale = 0.0;
                        effect_state.opacity = 0.0;
                        animations_to_remove.push(i);
                        animation_updates
                            .push(format!("Window {} close animation completed", window_id));
                    } else {
                        // Update animation
                        let progress = elapsed.as_secs_f64() / duration.as_secs_f64();
                        let eased_progress =
                            Self::apply_easing_curve_static(progress as f32, &EasingCurve::EaseIn);

                        effect_state.scale = start_scale * (1.0 - eased_progress * 0.2);
                        effect_state.opacity = start_opacity * (1.0 - eased_progress);
                    }
                }

                AnimationType::WindowMove {
                    start_time,
                    duration,
                    start_pos,
                    target_pos,
                } => {
                    let elapsed = now.duration_since(*start_time);

                    if elapsed >= *duration {
                        // Animation finished
                        effect_state.position_offset =
                            (target_pos.0 - start_pos.0, target_pos.1 - start_pos.1);
                        animations_to_remove.push(i);
                        animation_updates
                            .push(format!("Window {} move animation completed", window_id));
                    } else {
                        // Update animation
                        let progress = elapsed.as_secs_f64() / duration.as_secs_f64();
                        let eased_progress =
                            Self::apply_easing_curve_static(progress as f32, &EasingCurve::EaseOut);

                        let current_x = start_pos.0 + (target_pos.0 - start_pos.0) * eased_progress;
                        let current_y = start_pos.1 + (target_pos.1 - start_pos.1) * eased_progress;

                        effect_state.position_offset =
                            (current_x - start_pos.0, current_y - start_pos.1);
                    }
                }

                _ => {
                    // Handle other animation types
                }
            }
        }

        // Remove finished animations (in reverse order to maintain indices)
        for i in animations_to_remove.into_iter().rev() {
            effect_state.active_animations.remove(i);
        }

        Ok(animation_updates)
    }

    /// Static version of easing curve application
    fn apply_easing_curve_static(t: f32, curve: &EasingCurve) -> f32 {
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
            _ => t, // Simplified for other curves
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

    /// Remove finished animations and inactive windows
    fn cleanup_finished_animations(&mut self) {
        self.window_effects.retain(|_, effect_state| {
            !effect_state.active_animations.is_empty()
                || effect_state.opacity > 0.0
                || effect_state.scale > 0.0
        });
    }

    /// Get current effects quality (for performance monitoring)
    pub fn get_effects_quality(&self) -> f32 {
        self.effects_quality
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

    pub fn shutdown(&mut self) -> Result<()> {
        info!("🎨 Shutting down Visual Effects Engine...");
        self.window_effects.clear();
        info!("✅ Effects Engine shutdown complete");
        Ok(())
    }

    /// Toggle effects on/off
    pub fn toggle_effects(&mut self) {
        self.animations_enabled = !self.animations_enabled;
        info!(
            "✨ Effects {}",
            if self.animations_enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
    }

    /// Initialize GPU context for hardware-accelerated effects
    pub fn initialize_gpu(&mut self, device: Arc<Device>, queue: Arc<Queue>) -> Result<()> {
        info!("🚀 Initializing GPU acceleration for effects...");

        // Store GPU context
        self.gpu_device = Some(device.clone());
        self.gpu_queue = Some(queue.clone());

        // Initialize shader manager with Arc<Device>
        // Initialize blur renderer
        if self.blur_params.enabled {
            // Create shader manager for effects
            let mut shader_manager = shaders::ShaderManager::new(device.clone());
            shader_manager.compile_all_shaders()?;

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
                Arc::new(shader_manager),
                blur_params,
            )?);
            debug!("🌊 GPU blur renderer initialized");
        }

        // Initialize shadow renderer - temporarily disabled until shader manager is properly integrated
        // TODO: Re-enable once we have proper GPU context and shader management
        if self.default_shadow.enabled {
            debug!("🌟 Shadow rendering configured (GPU initialization deferred)");
        }

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
}
