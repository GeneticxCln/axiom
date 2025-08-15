//! Advanced Animation System for Visual Effects
//!
//! This module provides a comprehensive animation framework:
//! - Keyframe-based animations
//! - Spring physics simulations
//! - Easing curves and timing functions
//! - Animation sequencing and composition

use anyhow::Result;
use cgmath::{Vector2, Vector3, Vector4};
use log::{debug, info};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{AnimationType, EasingCurve};

/// Animation keyframe for property interpolation
#[derive(Debug, Clone)]
pub struct Keyframe<T> {
    pub time: f32,           // Time in seconds (0.0 to 1.0 for normalized)
    pub value: T,            // Value at this keyframe
    pub easing: EasingCurve, // Easing curve to next keyframe
}

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

/// Animation timeline for complex sequences
#[derive(Debug, Clone)]
pub struct AnimationTimeline {
    pub name: String,
    pub total_duration: Duration,
    pub repeat_count: Option<u32>, // None = infinite
    pub start_delay: Duration,
    pub end_delay: Duration,
    pub keyframes: Vec<TimelineEvent>,
}

/// Event in an animation timeline
#[derive(Debug, Clone)]
pub struct TimelineEvent {
    pub start_time: f32, // 0.0 to 1.0 (percentage of total duration)
    pub duration: f32,   // Duration as percentage of total
    pub animation_type: AnimationType,
    pub target_window: Option<u64>, // None = all windows
}

/// Advanced animation controller
pub struct AnimationController {
    /// Active animations by window ID
    active_animations: HashMap<u64, Vec<ActiveAnimation>>,

    /// Animation timelines
    timelines: HashMap<String, AnimationTimeline>,
    active_timelines: HashMap<String, TimelineState>,

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

/// Timeline state for active timeline
#[derive(Debug, Clone)]
struct TimelineState {
    start_time: Instant,
    current_repeat: u32,
    paused: bool,
    active_events: Vec<(usize, ActiveAnimation)>, // (event_index, animation)
}

/// Spring physics state
#[derive(Debug, Clone)]
struct SpringState {
    current_value: f32,
    target_value: f32,
    velocity: f32,
    params: SpringParams,
    last_update: Instant,
    settled: bool,
}

impl AnimationController {
    pub fn new() -> Self {
        info!("🎬 Initializing Advanced Animation Controller...");

        Self {
            active_animations: HashMap::new(),
            timelines: HashMap::new(),
            active_timelines: HashMap::new(),
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
            .or_insert_with(Vec::new)
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
            last_update: Instant::now(),
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

    /// Create and start an animation timeline
    pub fn start_timeline(&mut self, timeline: AnimationTimeline) -> Result<()> {
        let timeline_name = timeline.name.clone();

        let timeline_state = TimelineState {
            start_time: Instant::now(),
            current_repeat: 0,
            paused: false,
            active_events: Vec::new(),
        };

        self.timelines.insert(timeline_name.clone(), timeline);
        self.active_timelines
            .insert(timeline_name.clone(), timeline_state);

        info!(
            "🎭 Started animation timeline '{}' with {} events",
            timeline_name,
            self.timelines[&timeline_name].keyframes.len()
        );

        Ok(())
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

        // Update timelines
        self.update_timelines(now, &mut updates)?;

        // Clean up finished animations
        self.cleanup_finished_animations();

        // Update animation count for performance monitoring
        self.animation_count = self
            .active_animations
            .values()
            .map(|anims| anims.len())
            .sum::<usize>()
            + self.spring_states.len();

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

        for (window_id, animations) in self.active_animations.iter_mut() {
            animations.retain_mut(|anim| {
                if anim.paused {
                    return true;
                }

                let total_elapsed = now.duration_since(anim.start_time);

                // Check if animation should start (handle delay)
                if total_elapsed < anim.delay {
                    return true;
                }

                let elapsed = total_elapsed - anim.delay;
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
                    updates.push(AnimationUpdate {
                        window_id: *window_id,
                        property: AnimationProperty::Finished(anim.id),
                        value: AnimationValue::None,
                    });

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
        let dt = delta_time.as_secs_f32();

        for ((window_id, property_name), spring_state) in self.spring_states.iter_mut() {
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

    /// Update timeline-based animations
    fn update_timelines(
        &mut self,
        now: Instant,
        _updates: &mut Vec<AnimationUpdate>,
    ) -> Result<()> {
        let timeline_names: Vec<String> = self.active_timelines.keys().cloned().collect();

        for timeline_name in timeline_names {
            let timeline = match self.timelines.get(&timeline_name) {
                Some(tl) => tl.clone(),
                None => continue,
            };

            let timeline_state = match self.active_timelines.get_mut(&timeline_name) {
                Some(ts) => ts,
                None => continue,
            };

            if timeline_state.paused {
                continue;
            }

            let total_elapsed = now.duration_since(timeline_state.start_time);

            // Check if timeline should start (handle delay)
            if total_elapsed < timeline.start_delay {
                continue;
            }

            let elapsed_in_timeline = total_elapsed - timeline.start_delay;
            let progress =
                elapsed_in_timeline.as_secs_f64() / timeline.total_duration.as_secs_f64();

            if progress >= 1.0 {
                // Timeline finished
                if let Some(repeat_count) = timeline.repeat_count {
                    if timeline_state.current_repeat + 1 < repeat_count {
                        // Start next repetition
                        timeline_state.current_repeat += 1;
                        timeline_state.start_time = now;
                        timeline_state.active_events.clear();
                        continue;
                    }
                } else {
                    // Infinite repeat
                    timeline_state.start_time = now;
                    timeline_state.active_events.clear();
                    continue;
                }

                // Timeline completely finished
                self.active_timelines.remove(&timeline_name);
                info!("🎭 Timeline '{}' finished", timeline_name);
                continue;
            }

            // Check for new events to start
            for (event_index, event) in timeline.keyframes.iter().enumerate() {
                let event_start_progress = event.start_time;
                let event_end_progress = event.start_time + event.duration;

                if progress >= event_start_progress as f64 && progress < event_end_progress as f64 {
                    // Event should be active
                    let already_active = timeline_state
                        .active_events
                        .iter()
                        .any(|(idx, _)| *idx == event_index);

                    if !already_active {
                        // Start this event
                        let _window_id = event.target_window.unwrap_or(0); // 0 = global
                                                                           // For now, just track that the event started
                                                                           // In a full implementation, we'd manage these animations properly

                        // This is a bit of a hack - we'd need to track this better
                        // For now, just create a dummy ActiveAnimation
                        let dummy_id = 0; // In full implementation, generate proper ID
                        let active_animation = ActiveAnimation {
                            id: dummy_id,
                            animation_type: event.animation_type.clone(),
                            start_time: now,
                            duration: Duration::from_secs_f64(
                                (event.duration as f64) * timeline.total_duration.as_secs_f64(),
                            ),
                            delay: Duration::ZERO,
                            repeat_count: Some(1),
                            current_repeat: 0,
                            paused: false,
                            speed_multiplier: 1.0,
                        };

                        timeline_state
                            .active_events
                            .push((event_index, active_animation));
                    }
                }
            }

            // Remove finished events
            timeline_state.active_events.retain(|(event_index, _)| {
                let event = &timeline.keyframes[*event_index];
                let event_end_progress = event.start_time + event.duration;
                progress < event_end_progress as f64
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
            AnimationType::WindowOpen { .. } => EasingCurve::EaseOut,
            AnimationType::WindowClose { .. } => EasingCurve::EaseIn,
            AnimationType::WindowMove { .. } => EasingCurve::EaseOut,
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
                        rotation: 0.0,
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
                        rotation: 0.0,
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

    /// Apply easing curve statically
    fn apply_easing_curve_static(progress: f32, curve: &EasingCurve) -> f32 {
        let t = progress.clamp(0.0, 1.0);

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
            _ => t, // Simplified for now
        }
    }

    /// Calculate animation value based on type and progress
    fn calculate_animation_value(
        &self,
        animation: &AnimationType,
        progress: f32,
    ) -> Option<(AnimationProperty, AnimationValue)> {
        match animation {
            AnimationType::WindowOpen {
                target_scale,
                target_opacity,
                ..
            } => {
                let eased_progress = self.apply_easing(progress, &EasingCurve::EaseOut);
                let current_scale = 0.8 + (target_scale - 0.8) * eased_progress;
                let current_opacity = eased_progress * target_opacity;

                Some((
                    AnimationProperty::Transform,
                    AnimationValue::Transform {
                        scale: Vector2::new(current_scale, current_scale),
                        opacity: current_opacity,
                        rotation: 0.0,
                    },
                ))
            }

            AnimationType::WindowClose {
                start_scale,
                start_opacity,
                ..
            } => {
                let eased_progress = self.apply_easing(progress, &EasingCurve::EaseIn);
                let current_scale = start_scale * (1.0 - eased_progress * 0.2);
                let current_opacity = start_opacity * (1.0 - eased_progress);

                Some((
                    AnimationProperty::Transform,
                    AnimationValue::Transform {
                        scale: Vector2::new(current_scale, current_scale),
                        opacity: current_opacity,
                        rotation: 0.0,
                    },
                ))
            }

            AnimationType::WindowMove {
                start_pos,
                target_pos,
                ..
            } => {
                let eased_progress = self.apply_easing(progress, &EasingCurve::EaseOut);
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

    /// Apply easing curve to progress value
    fn apply_easing(&self, progress: f32, curve: &EasingCurve) -> f32 {
        let t = progress.clamp(0.0, 1.0);

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
                    7.5625 * t * t + 0.984375
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
            AnimationType::WindowOpen { duration, .. } => *duration,
            AnimationType::WindowClose { duration, .. } => *duration,
            AnimationType::WindowMove { duration, .. } => *duration,
            AnimationType::WindowResize { duration, .. } => *duration,
            AnimationType::WorkspaceTransition { duration, .. } => *duration,
        }
    }

    /// Get human-readable animation name
    fn get_animation_name(&self, animation: &AnimationType) -> &'static str {
        match animation {
            AnimationType::WindowOpen { .. } => "Window Open",
            AnimationType::WindowClose { .. } => "Window Close",
            AnimationType::WindowMove { .. } => "Window Move",
            AnimationType::WindowResize { .. } => "Window Resize",
            AnimationType::WorkspaceTransition { .. } => "Workspace Transition",
        }
    }

    /// Pause/unpause all animations
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if paused {
            info!("⏸️ Animation system paused");
        } else {
            info!("▶️ Animation system resumed");
        }
    }

    /// Set global speed multiplier
    pub fn set_global_speed(&mut self, speed: f32) {
        self.global_speed_multiplier = speed.max(0.1);
        info!(
            "⚡ Animation speed set to {:.1}x",
            self.global_speed_multiplier
        );
    }

    /// Get animation statistics
    pub fn get_animation_stats(&self) -> AnimationStats {
        AnimationStats {
            active_animations: self.animation_count,
            active_timelines: self.active_timelines.len(),
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
    Size,
    Opacity,
    Rotation,
    Scale,
    SpringProperty(String),
    Finished(u64), // Animation ID that finished
}

/// Value for animated property
#[derive(Debug, Clone)]
pub enum AnimationValue {
    Float(f32),
    Vector2(Vector2<f32>),
    Vector3(Vector3<f32>),
    Vector4(Vector4<f32>),
    Position(Vector2<f32>),
    Transform {
        scale: Vector2<f32>,
        opacity: f32,
        rotation: f32,
    },
    None,
}

/// Animation system statistics
#[derive(Debug, Clone)]
pub struct AnimationStats {
    pub active_animations: usize,
    pub active_timelines: usize,
    pub spring_animations: usize,
    pub global_speed: f32,
    pub paused: bool,
}
