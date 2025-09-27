//! Performance Monitoring and Benchmarking
//! 
//! Comprehensive performance monitoring system for the Axiom compositor
//! with real-time metrics collection, benchmarking, and optimization guidance.

use anyhow::Result;
use log::{debug, info, warn};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

/// Comprehensive performance monitoring system
pub struct PerformanceMonitor {
    /// Frame timing metrics
    frame_metrics: Arc<RwLock<FrameMetrics>>,
    
    /// Texture operation metrics
    texture_metrics: Arc<RwLock<TextureMetrics>>,
    
    /// Memory usage metrics
    memory_metrics: Arc<RwLock<MemoryMetrics>>,
    
    /// Window management metrics
    window_metrics: Arc<RwLock<WindowMetrics>>,
    
    /// GPU operation metrics
    gpu_metrics: Arc<RwLock<GpuMetrics>>,
    
    /// Historical performance data
    history: Arc<RwLock<PerformanceHistory>>,
    
    /// Performance targets and thresholds
    targets: PerformanceTargets,
    
    /// Monitoring configuration
    config: MonitoringConfig,
}

/// Frame rendering performance metrics
#[derive(Debug, Clone)]
pub struct FrameMetrics {
    /// Total frames rendered
    pub total_frames: u64,
    
    /// Current frame rate (FPS)
    pub current_fps: f32,
    
    /// Average frame time over last 60 frames (ms)
    pub avg_frame_time_ms: f32,
    
    /// Minimum frame time in current session (ms)
    pub min_frame_time_ms: f32,
    
    /// Maximum frame time in current session (ms)  
    pub max_frame_time_ms: f32,
    
    /// Frame times for last 60 frames
    pub recent_frame_times: VecDeque<f32>,
    
    /// Number of frames that missed 16ms target
    pub missed_vsync_frames: u64,
    
    /// GPU rendering time (excluding CPU overhead)
    pub gpu_render_time_ms: f32,
    
    /// Time spent in surface commit processing
    pub surface_commit_time_ms: f32,
    
    /// Time spent in damage region processing
    pub damage_processing_time_ms: f32,
}

/// Texture operation performance metrics
#[derive(Debug, Clone)]
pub struct TextureMetrics {
    /// Total texture uploads
    pub total_uploads: u64,
    
    /// Texture upload rate (uploads/second)
    pub upload_rate: f32,
    
    /// Average texture upload time (ms)
    pub avg_upload_time_ms: f32,
    
    /// Texture cache hit rate (%)
    pub cache_hit_rate: f32,
    
    /// Total texture memory allocated (bytes)
    pub texture_memory_bytes: u64,
    
    /// Number of active textures
    pub active_texture_count: u32,
    
    /// Texture pool efficiency metrics
    pub pool_utilization: f32,
    
    /// Damage region optimization effectiveness
    pub damage_optimization_ratio: f32,
}

/// Memory usage performance metrics
#[derive(Debug, Clone)]
pub struct MemoryMetrics {
    /// Total system memory usage (bytes)
    pub system_memory_bytes: u64,
    
    /// GPU memory usage (bytes)  
    pub gpu_memory_bytes: u64,
    
    /// Texture memory usage (bytes)
    pub texture_memory_bytes: u64,
    
    /// Buffer memory usage (bytes)
    pub buffer_memory_bytes: u64,
    
    /// Memory growth rate (bytes/second)
    pub memory_growth_rate: f64,
    
    /// Memory pressure indicator (0.0-1.0)
    pub memory_pressure: f32,
    
    /// Peak memory usage in session
    pub peak_memory_bytes: u64,
    
    /// Memory cleanup events
    pub cleanup_events: u32,
}

/// Window management performance metrics
#[derive(Debug, Clone)]
pub struct WindowMetrics {
    /// Total windows created
    pub total_windows_created: u64,
    
    /// Current active window count
    pub active_window_count: u32,
    
    /// Average window creation time (ms)
    pub avg_window_creation_time_ms: f32,
    
    /// Window layout calculation time (ms)
    pub layout_calculation_time_ms: f32,
    
    /// Focus change frequency (changes/minute)
    pub focus_change_rate: f32,
    
    /// Window resize operations per second
    pub resize_operations_rate: f32,
    
    /// Workspace scroll operations per second
    pub scroll_operations_rate: f32,
}

/// GPU operation performance metrics
#[derive(Debug, Clone)]
pub struct GpuMetrics {
    /// GPU utilization percentage (0.0-100.0)
    pub gpu_utilization: f32,
    
    /// GPU memory utilization percentage (0.0-100.0)
    pub gpu_memory_utilization: f32,
    
    /// GPU temperature (Celsius)
    pub gpu_temperature: f32,
    
    /// GPU frequency (MHz)
    pub gpu_frequency: u32,
    
    /// Number of GPU command submissions per second
    pub command_submission_rate: f32,
    
    /// Average GPU command execution time (ms)
    pub avg_command_time_ms: f32,
    
    /// GPU pipeline stalls per second
    pub pipeline_stalls_rate: f32,
}

/// Historical performance data for trend analysis
#[derive(Debug, Default)]
pub struct PerformanceHistory {
    /// Frame rate history (samples every second)
    pub fps_history: VecDeque<(u64, f32)>, // (timestamp, fps)
    
    /// Memory usage history (samples every 10 seconds)
    pub memory_history: VecDeque<(u64, u64)>, // (timestamp, bytes)
    
    /// Window count history (samples when changed)
    pub window_count_history: VecDeque<(u64, u32)>, // (timestamp, count)
    
    /// Performance event log
    pub events: VecDeque<PerformanceEvent>,
}

/// Performance event for logging and analysis
#[derive(Debug, Clone)]
pub struct PerformanceEvent {
    pub timestamp: u64,
    pub event_type: PerformanceEventType,
    pub description: String,
    pub severity: EventSeverity,
}

#[derive(Debug, Clone)]
pub enum PerformanceEventType {
    FrameRateDrop,
    MemoryPressure,
    GpuStall,
    TexturePoolMiss,
    WindowCreationSlow,
    DamageProcessingSlow,
    Other(String),
}

#[derive(Debug, Clone)]
pub enum EventSeverity {
    Info,
    Warning,
    Critical,
}

/// Performance targets and thresholds
#[derive(Debug, Clone)]
pub struct PerformanceTargets {
    /// Target frame rate (FPS)
    pub target_fps: f32,
    
    /// Maximum acceptable frame time (ms)
    pub max_frame_time_ms: f32,
    
    /// Maximum memory usage (bytes)
    pub max_memory_bytes: u64,
    
    /// Maximum GPU utilization (%)
    pub max_gpu_utilization: f32,
    
    /// Minimum texture cache hit rate (%)
    pub min_cache_hit_rate: f32,
    
    /// Maximum window creation time (ms)
    pub max_window_creation_time_ms: f32,
}

/// Monitoring configuration
#[derive(Debug, Clone)]
pub struct MonitoringConfig {
    /// Enable detailed GPU metrics collection
    pub enable_gpu_metrics: bool,
    
    /// Enable memory pressure monitoring
    pub enable_memory_monitoring: bool,
    
    /// History retention duration (seconds)
    pub history_retention: Duration,
    
    /// Sampling interval for continuous metrics (ms)
    pub sampling_interval_ms: u64,
    
    /// Enable automatic performance optimization
    pub enable_auto_optimization: bool,
}

impl Default for PerformanceTargets {
    fn default() -> Self {
        Self {
            target_fps: 60.0,
            max_frame_time_ms: 16.67, // 60 FPS target
            max_memory_bytes: 512 * 1024 * 1024, // 512 MB
            max_gpu_utilization: 80.0,
            min_cache_hit_rate: 80.0,
            max_window_creation_time_ms: 50.0,
        }
    }
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            enable_gpu_metrics: true,
            enable_memory_monitoring: true,
            history_retention: Duration::from_secs(300), // 5 minutes
            sampling_interval_ms: 1000, // 1 second
            enable_auto_optimization: true,
        }
    }
}

impl PerformanceMonitor {
    /// Create a new performance monitor
    pub fn new(targets: PerformanceTargets, config: MonitoringConfig) -> Self {
        info!("🔧 Initializing performance monitoring system");
        info!("   Target FPS: {}", targets.target_fps);
        info!("   Max frame time: {:.2}ms", targets.max_frame_time_ms);
        info!("   Max memory: {} MB", targets.max_memory_bytes / (1024 * 1024));
        info!("   Sampling interval: {}ms", config.sampling_interval_ms);
        
        Self {
            frame_metrics: Arc::new(RwLock::new(FrameMetrics::default())),
            texture_metrics: Arc::new(RwLock::new(TextureMetrics::default())),
            memory_metrics: Arc::new(RwLock::new(MemoryMetrics::default())),
            window_metrics: Arc::new(RwLock::new(WindowMetrics::default())),
            gpu_metrics: Arc::new(RwLock::new(GpuMetrics::default())),
            history: Arc::new(RwLock::new(PerformanceHistory::default())),
            targets,
            config,
        }
    }
    
    /// Record frame rendering completion
    pub async fn record_frame(&self, frame_time_ms: f32, gpu_time_ms: f32) {
        let mut metrics = self.frame_metrics.write().await;
        
        metrics.total_frames += 1;
        
        // Update frame time statistics
        metrics.recent_frame_times.push_back(frame_time_ms);
        if metrics.recent_frame_times.len() > 60 {
            metrics.recent_frame_times.pop_front();
        }
        
        metrics.avg_frame_time_ms = metrics.recent_frame_times.iter().sum::<f32>() / metrics.recent_frame_times.len() as f32;
        metrics.current_fps = 1000.0 / metrics.avg_frame_time_ms;
        
        if frame_time_ms < metrics.min_frame_time_ms || metrics.min_frame_time_ms == 0.0 {
            metrics.min_frame_time_ms = frame_time_ms;
        }
        
        if frame_time_ms > metrics.max_frame_time_ms {
            metrics.max_frame_time_ms = frame_time_ms;
        }
        
        metrics.gpu_render_time_ms = gpu_time_ms;
        
        // Check for performance issues
        if frame_time_ms > self.targets.max_frame_time_ms {
            metrics.missed_vsync_frames += 1;
            
            if frame_time_ms > self.targets.max_frame_time_ms * 2.0 {
                self.record_performance_event(
                    PerformanceEventType::FrameRateDrop,
                    format!("Severe frame drop: {:.2}ms (target: {:.2}ms)", frame_time_ms, self.targets.max_frame_time_ms),
                    EventSeverity::Critical,
                ).await;
            }
        }
        
        // Record FPS history
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let mut history = self.history.write().await;
        history.fps_history.push_back((timestamp, metrics.current_fps));
        
        // Limit history size
        while history.fps_history.len() > (self.config.history_retention.as_secs() as usize) {
            history.fps_history.pop_front();
        }
        
        debug!("📊 Frame #{}: {:.2}ms ({:.1} FPS) GPU: {:.2}ms", 
               metrics.total_frames, frame_time_ms, metrics.current_fps, gpu_time_ms);
    }
    
    /// Record texture operation
    pub async fn record_texture_upload(&self, upload_time_ms: f32, cache_hit: bool, texture_size_bytes: u64) {
        let mut metrics = self.texture_metrics.write().await;
        
        metrics.total_uploads += 1;
        metrics.avg_upload_time_ms = (metrics.avg_upload_time_ms * (metrics.total_uploads - 1) as f32 + upload_time_ms) / metrics.total_uploads as f32;
        
        if cache_hit {
            // Update cache hit rate calculation
            let total_operations = metrics.total_uploads;
            let current_hits = (metrics.cache_hit_rate * (total_operations - 1) as f32 / 100.0) + 1.0;
            metrics.cache_hit_rate = (current_hits / total_operations as f32) * 100.0;
        }
        
        metrics.texture_memory_bytes = metrics.texture_memory_bytes.saturating_add(texture_size_bytes);
        
        debug!("📊 Texture upload: {:.2}ms, {} bytes, cache hit: {}", upload_time_ms, texture_size_bytes, cache_hit);
    }
    
    /// Record memory usage update
    pub async fn record_memory_usage(&self, system_bytes: u64, gpu_bytes: u64, texture_bytes: u64) {
        let mut metrics = self.memory_metrics.write().await;
        
        let previous_total = metrics.system_memory_bytes;
        metrics.system_memory_bytes = system_bytes;
        metrics.gpu_memory_bytes = gpu_bytes;
        metrics.texture_memory_bytes = texture_bytes;
        
        let total_memory = system_bytes + gpu_bytes;
        
        if total_memory > metrics.peak_memory_bytes {
            metrics.peak_memory_bytes = total_memory;
        }
        
        // Calculate memory pressure
        metrics.memory_pressure = (total_memory as f32 / self.targets.max_memory_bytes as f32).min(1.0);
        
        // Check for memory pressure
        if metrics.memory_pressure > 0.8 {
            self.record_performance_event(
                PerformanceEventType::MemoryPressure,
                format!("High memory usage: {} MB ({:.1}% of limit)", total_memory / (1024 * 1024), metrics.memory_pressure * 100.0),
                if metrics.memory_pressure > 0.95 { EventSeverity::Critical } else { EventSeverity::Warning },
            ).await;
        }
        
        // Record memory history
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let mut history = self.history.write().await;
        history.memory_history.push_back((timestamp, total_memory));
        
        // Limit history size (sample every 10 seconds)
        let retention_samples = (self.config.history_retention.as_secs() / 10) as usize;
        while history.memory_history.len() > retention_samples {
            history.memory_history.pop_front();
        }
    }
    
    /// Record window management operation
    pub async fn record_window_operation(&self, operation_type: &str, duration_ms: f32) {
        let mut metrics = self.window_metrics.write().await;
        
        match operation_type {
            "create" => {
                metrics.total_windows_created += 1;
                metrics.avg_window_creation_time_ms = 
                    (metrics.avg_window_creation_time_ms * (metrics.total_windows_created - 1) as f32 + duration_ms) 
                    / metrics.total_windows_created as f32;
                
                if duration_ms > self.targets.max_window_creation_time_ms {
                    self.record_performance_event(
                        PerformanceEventType::WindowCreationSlow,
                        format!("Slow window creation: {:.2}ms (target: {:.2}ms)", duration_ms, self.targets.max_window_creation_time_ms),
                        EventSeverity::Warning,
                    ).await;
                }
            }
            "layout" => {
                metrics.layout_calculation_time_ms = duration_ms;
            }
            _ => {}
        }
        
        debug!("📊 Window operation '{}': {:.2}ms", operation_type, duration_ms);
    }
    
    /// Update window count
    pub async fn update_window_count(&self, count: u32) {
        let mut metrics = self.window_metrics.write().await;
        metrics.active_window_count = count;
        
        // Record in history if significantly changed
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let mut history = self.history.write().await;
        
        if let Some(&(_, last_count)) = history.window_count_history.back() {
            if count != last_count {
                history.window_count_history.push_back((timestamp, count));
            }
        } else {
            history.window_count_history.push_back((timestamp, count));
        }
        
        // Limit history
        while history.window_count_history.len() > 1000 {
            history.window_count_history.pop_front();
        }
    }
    
    /// Generate comprehensive performance report
    pub async fn generate_report(&self) -> PerformanceReport {
        let frame_metrics = self.frame_metrics.read().await.clone();
        let texture_metrics = self.texture_metrics.read().await.clone();
        let memory_metrics = self.memory_metrics.read().await.clone();
        let window_metrics = self.window_metrics.read().await.clone();
        let gpu_metrics = self.gpu_metrics.read().await.clone();
        let history = self.history.read().await;
        
        // Calculate health score (0-100)
        let mut health_score = 100.0;
        
        // Frame rate health (30% of score)
        let fps_ratio = frame_metrics.current_fps / self.targets.target_fps;
        health_score *= 0.7 + 0.3 * fps_ratio.min(1.0);
        
        // Memory health (25% of score)
        health_score *= 0.75 + 0.25 * (1.0 - memory_metrics.memory_pressure);
        
        // Texture efficiency health (25% of score)
        let texture_efficiency = texture_metrics.cache_hit_rate / 100.0;
        health_score *= 0.75 + 0.25 * texture_efficiency;
        
        // Window performance health (20% of score)
        let window_perf = if window_metrics.avg_window_creation_time_ms > 0.0 {
            (self.targets.max_window_creation_time_ms / window_metrics.avg_window_creation_time_ms).min(1.0)
        } else {
            1.0
        };
        health_score *= 0.8 + 0.2 * window_perf;
        
        // Identify bottlenecks
        let mut bottlenecks = Vec::new();
        
        if frame_metrics.current_fps < self.targets.target_fps * 0.9 {
            bottlenecks.push("Frame rate below target".to_string());
        }
        
        if memory_metrics.memory_pressure > 0.8 {
            bottlenecks.push("High memory usage".to_string());
        }
        
        if texture_metrics.cache_hit_rate < self.targets.min_cache_hit_rate {
            bottlenecks.push("Low texture cache efficiency".to_string());
        }
        
        if window_metrics.avg_window_creation_time_ms > self.targets.max_window_creation_time_ms {
            bottlenecks.push("Slow window creation".to_string());
        }
        
        // Generate optimization recommendations
        let mut recommendations = Vec::new();
        
        if frame_metrics.current_fps < self.targets.target_fps {
            recommendations.push("Consider reducing visual effects quality".to_string());
            recommendations.push("Enable adaptive quality scaling".to_string());
        }
        
        if memory_metrics.memory_pressure > 0.7 {
            recommendations.push("Increase texture pool cleanup frequency".to_string());
            recommendations.push("Reduce texture cache size limits".to_string());
        }
        
        if texture_metrics.cache_hit_rate < 70.0 {
            recommendations.push("Increase texture pool size".to_string());
            recommendations.push("Optimize texture format usage".to_string());
        }
        
        PerformanceReport {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            health_score: health_score as u8,
            frame_metrics,
            texture_metrics,
            memory_metrics,
            window_metrics,
            gpu_metrics,
            bottlenecks,
            recommendations,
            recent_events: history.events.iter().rev().take(10).cloned().collect(),
        }
    }
    
    /// Record a performance event
    async fn record_performance_event(&self, event_type: PerformanceEventType, description: String, severity: EventSeverity) {
        let event = PerformanceEvent {
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
            event_type,
            description: description.clone(),
            severity,
        };
        
        let mut history = self.history.write().await;
        history.events.push_back(event);
        
        // Limit event history
        while history.events.len() > 1000 {
            history.events.pop_front();
        }
        
        match severity {
            EventSeverity::Info => info!("📊 {}", description),
            EventSeverity::Warning => warn!("⚠️ {}", description),
            EventSeverity::Critical => warn!("🔴 {}", description),
        }
    }
    
    /// Start automatic performance monitoring background task
    pub async fn start_monitoring(&self) -> tokio::task::JoinHandle<()> {
        let monitor = Arc::new(self.clone());
        let interval = Duration::from_millis(self.config.sampling_interval_ms);
        
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            
            loop {
                interval_timer.tick().await;
                
                // Sample system metrics
                if monitor.config.enable_memory_monitoring {
                    if let Ok(memory_info) = monitor.sample_system_memory().await {
                        monitor.record_memory_usage(
                            memory_info.system_bytes,
                            memory_info.gpu_bytes,
                            memory_info.texture_bytes,
                        ).await;
                    }
                }
                
                if monitor.config.enable_gpu_metrics {
                    if let Ok(gpu_info) = monitor.sample_gpu_metrics().await {
                        let mut gpu_metrics = monitor.gpu_metrics.write().await;
                        *gpu_metrics = gpu_info;
                    }
                }
                
                // Auto-optimization if enabled
                if monitor.config.enable_auto_optimization {
                    monitor.auto_optimize().await;
                }
            }
        })
    }
    
    /// Sample system memory usage
    async fn sample_system_memory(&self) -> Result<MemoryInfo> {
        // Implementation would read from /proc/meminfo, GPU driver APIs, etc.
        // This is a placeholder for the actual implementation
        Ok(MemoryInfo {
            system_bytes: 0,
            gpu_bytes: 0,
            texture_bytes: 0,
        })
    }
    
    /// Sample GPU metrics
    async fn sample_gpu_metrics(&self) -> Result<GpuMetrics> {
        // Implementation would query GPU driver (NVML, sysfs, etc.)
        // This is a placeholder for the actual implementation
        Ok(GpuMetrics::default())
    }
    
    /// Automatic performance optimization
    async fn auto_optimize(&self) {
        let frame_metrics = self.frame_metrics.read().await;
        let memory_metrics = self.memory_metrics.read().await;
        
        // Example: Reduce effects quality if frame rate is low
        if frame_metrics.current_fps < self.targets.target_fps * 0.8 {
            debug!("🔧 Auto-optimization: Frame rate low, suggesting effects reduction");
            // Would trigger effects quality reduction
        }
        
        // Example: Trigger cleanup if memory pressure is high
        if memory_metrics.memory_pressure > 0.85 {
            debug!("🔧 Auto-optimization: High memory pressure, triggering cleanup");
            // Would trigger texture pool cleanup
        }
    }
}

/// Performance report structure
#[derive(Debug, Clone)]
pub struct PerformanceReport {
    pub timestamp: u64,
    pub health_score: u8,
    pub frame_metrics: FrameMetrics,
    pub texture_metrics: TextureMetrics,
    pub memory_metrics: MemoryMetrics,
    pub window_metrics: WindowMetrics,
    pub gpu_metrics: GpuMetrics,
    pub bottlenecks: Vec<String>,
    pub recommendations: Vec<String>,
    pub recent_events: Vec<PerformanceEvent>,
}

/// Memory info structure for sampling
#[derive(Debug)]
struct MemoryInfo {
    system_bytes: u64,
    gpu_bytes: u64,
    texture_bytes: u64,
}

impl Default for FrameMetrics {
    fn default() -> Self {
        Self {
            total_frames: 0,
            current_fps: 0.0,
            avg_frame_time_ms: 0.0,
            min_frame_time_ms: 0.0,
            max_frame_time_ms: 0.0,
            recent_frame_times: VecDeque::new(),
            missed_vsync_frames: 0,
            gpu_render_time_ms: 0.0,
            surface_commit_time_ms: 0.0,
            damage_processing_time_ms: 0.0,
        }
    }
}

impl Default for TextureMetrics {
    fn default() -> Self {
        Self {
            total_uploads: 0,
            upload_rate: 0.0,
            avg_upload_time_ms: 0.0,
            cache_hit_rate: 0.0,
            texture_memory_bytes: 0,
            active_texture_count: 0,
            pool_utilization: 0.0,
            damage_optimization_ratio: 0.0,
        }
    }
}

impl Default for MemoryMetrics {
    fn default() -> Self {
        Self {
            system_memory_bytes: 0,
            gpu_memory_bytes: 0,
            texture_memory_bytes: 0,
            buffer_memory_bytes: 0,
            memory_growth_rate: 0.0,
            memory_pressure: 0.0,
            peak_memory_bytes: 0,
            cleanup_events: 0,
        }
    }
}

impl Default for WindowMetrics {
    fn default() -> Self {
        Self {
            total_windows_created: 0,
            active_window_count: 0,
            avg_window_creation_time_ms: 0.0,
            layout_calculation_time_ms: 0.0,
            focus_change_rate: 0.0,
            resize_operations_rate: 0.0,
            scroll_operations_rate: 0.0,
        }
    }
}

impl Default for GpuMetrics {
    fn default() -> Self {
        Self {
            gpu_utilization: 0.0,
            gpu_memory_utilization: 0.0,
            gpu_temperature: 0.0,
            gpu_frequency: 0,
            command_submission_rate: 0.0,
            avg_command_time_ms: 0.0,
            pipeline_stalls_rate: 0.0,
        }
    }
}

impl Clone for PerformanceMonitor {
    fn clone(&self) -> Self {
        Self {
            frame_metrics: Arc::clone(&self.frame_metrics),
            texture_metrics: Arc::clone(&self.texture_metrics),
            memory_metrics: Arc::clone(&self.memory_metrics),
            window_metrics: Arc::clone(&self.window_metrics),
            gpu_metrics: Arc::clone(&self.gpu_metrics),
            history: Arc::clone(&self.history),
            targets: self.targets.clone(),
            config: self.config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_performance_monitor_creation() {
        let targets = PerformanceTargets::default();
        let config = MonitoringConfig::default();
        let monitor = PerformanceMonitor::new(targets, config);
        
        let report = monitor.generate_report().await;
        assert_eq!(report.health_score, 100); // Should start with perfect health
    }
    
    #[tokio::test]
    async fn test_frame_recording() {
        let targets = PerformanceTargets::default();
        let config = MonitoringConfig::default();
        let monitor = PerformanceMonitor::new(targets, config);
        
        monitor.record_frame(16.0, 12.0).await;
        monitor.record_frame(18.0, 14.0).await;
        
        let metrics = monitor.frame_metrics.read().await;
        assert_eq!(metrics.total_frames, 2);
        assert_eq!(metrics.recent_frame_times.len(), 2);
    }
    
    #[tokio::test]
    async fn test_performance_event_recording() {
        let targets = PerformanceTargets::default();
        let config = MonitoringConfig::default();
        let monitor = PerformanceMonitor::new(targets, config);
        
        monitor.record_performance_event(
            PerformanceEventType::FrameRateDrop,
            "Test event".to_string(),
            EventSeverity::Warning,
        ).await;
        
        let history = monitor.history.read().await;
        assert_eq!(history.events.len(), 1);
    }
}