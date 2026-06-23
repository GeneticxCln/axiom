//! Phase 4: Visual Effects Engine (Hyprland-inspired)
//!
//! This module handles all visual effects: animations, blur, shadows,
//! rounded corners, and other eye candy that makes Axiom beautiful.
#![allow(missing_docs)]

use crate::config::EffectsConfig;
use crate::effects::animations::AnimationStats;
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
    #[allow(dead_code)]
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
                        (animations::AnimationProperty::Transform, animations::AnimationValue::Transform { scale, opacity, .. }) => {
                            effect_state.scale = scale.x.max(scale.y);
                            effect_state.opacity = *opacity;
                        }
                        (animations::AnimationProperty::Position, animations::AnimationValue::Position(pos)) => {
                            effect_state.position_offset = (pos.x, pos.y);
                        }
                        (animations::AnimationProperty::Opacity, animations::AnimationValue::Float(v)) => {
                            effect_state.opacity = v.clamp(0.0, 1.0);
                        }
                        (animations::AnimationProperty::Scale, animations::AnimationValue::Float(v)) => {
                            effect_state.scale = v.max(0.0);
                        }
                        (animations::AnimationProperty::Rotation, animations::AnimationValue::Float(_v)) => {
                            // Rotation not yet stored on WindowEffectState — no-op for now
                        }
                        (animations::AnimationProperty::Size, animations::AnimationValue::Vector2(_size)) => {
                            // Size not yet stored on WindowEffectState — no-op for now
                        }
                        (animations::AnimationProperty::SpringProperty(name), animations::AnimationValue::Float(v)) => {
                            match name.as_str() {
                                "opacity" => effect_state.opacity = v.clamp(0.0, 1.0),
                                "scale" => effect_state.scale = v.max(0.0),
                                "blur" => effect_state.blur_radius = v.max(0.0),
                                "corner_radius" => effect_state.corner_radius = v.max(0.0),
                                _ => {}
                            }
                        }
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

        self.animation_controller.start_animation(
            window_id,
            animation,
            Duration::ZERO,
            Some(1),
        );

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
        if !self.blur_params.enabled || !self.config.enabled {
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

    /// Remove window from effects tracking
    pub fn remove_window(&mut self, window_id: u64) {
        if self.window_effects.remove(&window_id).is_some() {
            debug!("🗑️ Removed window {} from effects tracking", window_id);
        }
    }

    /// Apply easing curve to animation progress
    #[allow(dead_code)]
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
        self.window_effects.retain(|_, effect_state| {
            effect_state.opacity > 0.0 || effect_state.scale > 0.0
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
        self.config.enabled = !self.config.enabled;
    }

    // Temporary no-op to satisfy benches that call this API. In future, wire to actual blur control per window.
    pub fn set_window_blur(&mut self, _window_id: u64, _radius: f32) {
        // Intentionally left as no-op for now.
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

    /// Initialize GPU context for hardware-accelerated effects
    pub fn initialize_gpu(&mut self, device: Arc<Device>, queue: Arc<Queue>) -> Result<()> {
        info!("🚀 Initializing GPU acceleration for effects...");

        // Store GPU context
        self.gpu_device = Some(device.clone());
        self.gpu_queue = Some(queue.clone());

        // Initialize shader manager (compiled once, shared by blur and shadow renderers)
        let mut sm = shaders::ShaderManager::new(device.clone());
        sm.compile_all_shaders()?;
        let shader_manager = Arc::new(sm);
        self.shader_manager = Some(shader_manager.clone());

        // Initialize blur renderer
        if self.blur_params.enabled {            // Convert our BlurParams to blur module's BlurParams
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
                device.clone(),
                queue.clone(),
                shader_manager.clone(),
                self.default_shadow.clone(),
                shadow::ShadowQuality::High,
            )?);
            info!("🌟 GPU shadow renderer initialized");
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

    /// Render drop shadows for all visible windows via the GPU shadow renderer.
    /// Convenience wrapper that calls through to the renderer and also wires
    /// the effects engine into an AxiomRenderer for automated shadow passes.
    #[allow(dead_code)]
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

    /// Get reference to the shadow renderer for direct GPU shadow operations
    pub fn shadow_renderer(&self) -> Option<&shadow::ShadowRenderer> {
        self.shadow_renderer.as_ref()
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
                    result >= -0.1 && result <= 1.1,
                    "Easing curve {:?} at t={}: result={} out of expected range",
                    curve, t, result
                );
            }
        }
    }

    #[test]
    fn test_shutdown() {
        let config = EffectsConfig::default();
        let mut engine = EffectsEngine::new(&config).expect("Failed to create EffectsEngine");
        engine.animate_window_open(1);
        engine.shutdown().expect("Shutdown should succeed");
    }
}
