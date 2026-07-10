//! Advanced Animation System for Visual Effects
//!
//! This module provides a comprehensive animation framework:
//! - Keyframe-based animations
//! - Spring physics simulations
//! - Easing curves and timing functions
//! - Animation sequencing and composition

use anyhow::Result;
use cgmath::Vector2;
use log::{debug, info};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{AnimationType, EasingCurve};

/// Spring physics parameters for natural animations
#[derive(Debug, Clone, Copy)]
pub struct SpringParams {
    pub stiffness: f32, // How quickly spring responds (higher = faster)
    pub damping: f32,   // How much oscillation is dampened (higher = less bouncy)
    pub mass: f32,      // Mass of animated object (affects inertia)
    pub precision: f32, // When to consider spring "settled"
}

impl Default for SpringParams {
    fn default() -> Self {
        Self {
            stiffness: 300.0, // Good balance for UI animations
            damping: 30.0,    // Slight overshoot
            mass: 1.0,        // Standard mass
            precision: 0.01,  // Stop when within 1% of target
        }
    }
}

/// Advanced animation controller
pub struct AnimationController {
    /// Active animations by window ID
    active_animations: HashMap<u64, Vec<ActiveAnimation>>,

    /// Global animation settings
    global_speed_multiplier: f32,
    paused: bool,

    /// Spring physics cache
    spring_states: HashMap<(u64, String), SpringState>, // (window_id, property_name)

    /// Performance tracking
    last_update_time: Instant,
    animation_count: usize,
}

/// Active animation instance
#[derive(Debug, Clone)]
struct ActiveAnimation {
    #[allow(dead_code)]
    id: u64,
    animation_type: AnimationType,
    start_time: Instant,
    duration: Duration,
    delay: Duration,
    repeat_count: Option<u32>,
    current_repeat: u32,
    paused: bool,
    speed_multiplier: f32,
}

/// Spring physics state
#[derive(Debug, Clone)]
struct SpringState {
    current_value: f32,
    target_value: f32,
    velocity: f32,
    params: SpringParams,
    settled: bool,
}

impl AnimationController {
    /// Create a new [`AnimationController`] with default settings.
    ///
    /// The controller starts unpaused with a global speed multiplier of 1.0.
    /// No animations are scheduled initially.
    pub fn new() -> Self {
        info!("🎬 Initializing Advanced Animation Controller...");

        Self {
            active_animations: HashMap::new(),
            global_speed_multiplier: 1.0,
            paused: false,
            spring_states: HashMap::new(),
            last_update_time: Instant::now(),
            animation_count: 0,
        }
    }

    /// Start a new animation for a window
    pub fn start_animation(
        &mut self,
        window_id: u64,
        animation: AnimationType,
        delay: Duration,
        repeat_count: Option<u32>,
    ) -> u64 {
        let animation_id = self.generate_animation_id();

        let animation_duration = self.get_animation_duration(&animation);

        let active_animation = ActiveAnimation {
            id: animation_id,
            animation_type: animation,
            start_time: Instant::now(),
            duration: animation_duration,
            delay,
            repeat_count,
            current_repeat: 0,
            paused: false,
            speed_multiplier: 1.0,
        };

        self.active_animations
            .entry(window_id)
            .or_default()
            .push(active_animation);

        self.animation_count += 1;

        debug!(
            "🎬 Started animation {} for window {}",
            animation_id, window_id
        );

        animation_id
    }

    /// Start a spring-based animation
    pub fn start_spring_animation(
        &mut self,
        window_id: u64,
        property_name: String,
        target_value: f32,
        current_value: Option<f32>,
        params: Option<SpringParams>,
    ) {
        let params = params.unwrap_or_default();

        let spring_state = SpringState {
            current_value: current_value.unwrap_or(0.0),
            target_value,
            velocity: 0.0,
            params,
            settled: false,
        };

        self.spring_states
            .insert((window_id, property_name.clone()), spring_state);

        debug!(
            "🌸 Started spring animation for window {} property '{}': {} -> {}",
            window_id,
            property_name,
            current_value.unwrap_or(0.0),
            target_value
        );
    }

    /// Update all animations
    pub fn update(&mut self) -> Result<Vec<AnimationUpdate>> {
        let now = Instant::now();
        let delta_time = now.duration_since(self.last_update_time);
        self.last_update_time = now;

        if self.paused {
            return Ok(Vec::new());
        }

        let mut updates = Vec::new();

        // Update regular animations
        self.update_regular_animations(now, &mut updates)?;

        // Update spring animations
        self.update_spring_animations(delta_time, &mut updates)?;

        // Clean up finished animations
        self.cleanup_finished_animations();

        // Update animation count for performance monitoring
        self.animation_count =
            self.active_animations.values().map(Vec::len).sum::<usize>() + self.spring_states.len();

        if !updates.is_empty() {
            debug!(
                "🎬 Animation update: {} changes, {} active animations",
                updates.len(),
                self.animation_count
            );
        }

        Ok(updates)
    }

    /// Update regular keyframe-based animations
    fn update_regular_animations(
        &mut self,
        now: Instant,
        updates: &mut Vec<AnimationUpdate>,
    ) -> Result<()> {
        let global_speed = self.global_speed_multiplier;

        for (window_id, animations) in &mut self.active_animations {
            animations.retain_mut(|anim| {
                if anim.paused {
                    return true;
                }

                let total_elapsed = now.duration_since(anim.start_time);

                // Check if animation should start (handle delay)
                if total_elapsed < anim.delay {
                    return true;
                }

                let elapsed = total_elapsed.saturating_sub(anim.delay);
                let effective_duration = Duration::from_secs_f64(
                    anim.duration.as_secs_f64() / (anim.speed_multiplier * global_speed) as f64,
                );

                if elapsed >= effective_duration {
                    // Animation finished
                    if let Some(repeat_count) = anim.repeat_count {
                        if anim.current_repeat + 1 < repeat_count {
                            // Start next repetition
                            anim.current_repeat += 1;
                            anim.start_time = now;
                            return true;
                        }
                    } else {
                        // Infinite repeat
                        anim.start_time = now;
                        return true;
                    }

                    // Animation completely finished
                    return false;
                }

                // Calculate animation progress and apply easing
                let progress = elapsed.as_secs_f64() / effective_duration.as_secs_f64();
                let progress = progress.clamp(0.0, 1.0) as f32;

                // Clone the animation type to avoid borrowing issues
                let anim_type = anim.animation_type.clone();
                if let Some(update) = Self::calculate_animation_value_static(&anim_type, progress) {
                    updates.push(AnimationUpdate {
                        window_id: *window_id,
                        property: update.0,
                        value: update.1,
                    });
                }

                true
            });
        }

        Ok(())
    }

    /// Update spring physics animations
    fn update_spring_animations(
        &mut self,
        delta_time: Duration,
        updates: &mut Vec<AnimationUpdate>,
    ) -> Result<()> {
        // Clamp delta-time to 50 ms to prevent explosive overshoot after
        // a GC pause, debug breakpoint, or first-frame spike.
        let dt = delta_time.as_secs_f32().min(0.05);

        for ((window_id, property_name), spring_state) in &mut self.spring_states {
            if spring_state.settled {
                continue;
            }

            // Spring physics calculation
            let displacement = spring_state.current_value - spring_state.target_value;
            let spring_force = -spring_state.params.stiffness * displacement;
            let damping_force = -spring_state.params.damping * spring_state.velocity;

            let acceleration = (spring_force + damping_force) / spring_state.params.mass;

            // Integrate velocity and position
            spring_state.velocity += acceleration * dt;
            spring_state.current_value += spring_state.velocity * dt;

            // Check if spring has settled
            let velocity_threshold = spring_state.params.precision * 10.0; // Allow some velocity
            if displacement.abs() < spring_state.params.precision
                && spring_state.velocity.abs() < velocity_threshold
            {
                spring_state.current_value = spring_state.target_value;
                spring_state.velocity = 0.0;
                spring_state.settled = true;

                debug!(
                    "🌸 Spring animation settled for window {} property '{}'",
                    window_id, property_name
                );
            }

            updates.push(AnimationUpdate {
                window_id: *window_id,
                property: AnimationProperty::SpringProperty(property_name.clone()),
                value: AnimationValue::Float(spring_state.current_value),
            });
        }

        Ok(())
    }

    /// Calculate animation value based on type and progress (static version)
    fn calculate_animation_value_static(
        animation: &AnimationType,
        progress: f32,
    ) -> Option<(AnimationProperty, AnimationValue)> {
        Self::apply_easing_static(animation, progress)
    }

    /// Apply easing statically
    fn apply_easing_static(
        animation: &AnimationType,
        progress: f32,
    ) -> Option<(AnimationProperty, AnimationValue)> {
        let easing_curve = match animation {
            AnimationType::WindowClose { .. } => EasingCurve::EaseIn,
            AnimationType::WindowOpen { .. } | AnimationType::WindowMove { .. } => {
                EasingCurve::EaseOut
            }
            _ => EasingCurve::Linear,
        };

        let eased_progress = Self::apply_easing_curve_static(progress, &easing_curve);

        match animation {
            AnimationType::WindowOpen {
                target_scale,
                target_opacity,
                ..
            } => {
                let current_scale = 0.8 + (target_scale - 0.8) * eased_progress;
                let current_opacity = eased_progress * target_opacity;

                Some((
                    AnimationProperty::Transform,
                    AnimationValue::Transform {
                        scale: Vector2::new(current_scale, current_scale),
                        opacity: current_opacity,
                    },
                ))
            }

            AnimationType::WindowClose {
                start_scale,
                start_opacity,
                ..
            } => {
                let current_scale = start_scale * (1.0 - eased_progress * 0.2);
                let current_opacity = start_opacity * (1.0 - eased_progress);

                Some((
                    AnimationProperty::Transform,
                    AnimationValue::Transform {
                        scale: Vector2::new(current_scale, current_scale),
                        opacity: current_opacity,
                    },
                ))
            }

            AnimationType::WindowMove {
                start_pos,
                target_pos,
                ..
            } => {
                let current_x = start_pos.0 + (target_pos.0 - start_pos.0) * eased_progress;
                let current_y = start_pos.1 + (target_pos.1 - start_pos.1) * eased_progress;

                Some((
                    AnimationProperty::Position,
                    AnimationValue::Position(Vector2::new(current_x, current_y)),
                ))
            }

            _ => None, // Handle other animation types as needed
        }
    }

    /// Apply easing curve statically.
    ///
    /// Implements the full set of advertised curves — EaseIn, EaseOut,
    /// EaseInOut, BounceOut, ElasticOut, BackOut, and Linear. Previously
    /// BounceOut/ElasticOut/BackOut silently fell through to linear.
    fn apply_easing_curve_static(progress: f32, curve: &EasingCurve) -> f32 {
        let t = progress.clamp(0.0, 1.0);
        const PI: f32 = std::f32::consts::PI;

        match curve {
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
                const N1: f32 = 7.5625;
                const D1: f32 = 2.75;
                if t < 1.0 / D1 {
                    N1 * t * t
                } else if t < 2.0 / D1 {
                    let t2 = t - 1.5 / D1;
                    N1 * t2 * t2 + 0.75
                } else if t < 2.5 / D1 {
                    let t2 = t - 2.25 / D1;
                    N1 * t2 * t2 + 0.9375
                } else {
                    let t2 = t - 2.625 / D1;
                    N1 * t2 * t2 + 0.984_375
                }
            }
            EasingCurve::ElasticOut => {
                if t == 0.0 {
                    return 0.0;
                }
                if t == 1.0 {
                    return 1.0;
                }
                let c4 = (2.0 * PI) / 3.0;
                (2.0_f32).powf(-10.0 * t) * ((t - 1.0) * c4).sin() + 1.0
            }
            EasingCurve::BackOut => {
                const C1: f32 = 1.70158;
                const C3: f32 = C1 + 1.0;
                1.0 + C3 * (t - 1.0).powi(3) + C1 * (t - 1.0).powi(2)
            }
            EasingCurve::Linear => t,
        }
    }

    /// Clean up finished animations
    fn cleanup_finished_animations(&mut self) {
        self.active_animations.retain(|_, animations| {
            animations.retain(|_| true); // Already filtered in update
            !animations.is_empty()
        });

        self.spring_states
            .retain(|_, spring_state| !spring_state.settled);
    }

    /// Generate unique animation ID
    fn generate_animation_id(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        Instant::now().hash(&mut hasher);
        self.animation_count.hash(&mut hasher);
        hasher.finish()
    }

    /// Get animation duration from type
    fn get_animation_duration(&self, animation: &AnimationType) -> Duration {
        match animation {
            AnimationType::WindowOpen { duration, .. }
            | AnimationType::WindowClose { duration, .. }
            | AnimationType::WindowMove { duration, .. }
            | AnimationType::WindowResize { duration, .. }
            | AnimationType::WorkspaceTransition { duration, .. } => *duration,
        }
    }

    /// Get animation statistics
    pub fn get_animation_stats(&self) -> AnimationStats {
        AnimationStats {
            active_animations: self.animation_count,
            spring_animations: self.spring_states.len(),
            global_speed: self.global_speed_multiplier,
            paused: self.paused,
        }
    }
}

/// Animation update event sent to effects engine
#[derive(Debug, Clone)]
pub struct AnimationUpdate {
    pub window_id: u64,
    pub property: AnimationProperty,
    pub value: AnimationValue,
}

/// Property being animated
#[derive(Debug, Clone)]
pub enum AnimationProperty {
    Transform,
    Position,
    // Forward-compat scaffolding: these variants are matched in
    // EffectsEngine::update() (effects/mod.rs) but the AnimationController
    // does not yet construct them. Keep for future animation property types.
    #[allow(dead_code)]
    Opacity,
    #[allow(dead_code)]
    Scale,
    SpringProperty(String),
}

/// Value for animated property
#[derive(Debug, Clone)]
pub enum AnimationValue {
    Float(f32),
    Position(Vector2<f32>),
    Transform { scale: Vector2<f32>, opacity: f32 },
}

/// Animation system statistics
#[derive(Debug, Clone)]
pub struct AnimationStats {
    pub active_animations: usize,
    pub spring_animations: usize,
    pub global_speed: f32,
    pub paused: bool,
}
