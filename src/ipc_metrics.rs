//! Enhanced IPC metrics collection and reporting for Lazy UI integration
//!
//! This module provides real-time performance monitoring and AI optimization
//! feedback to the Lazy UI system.

use anyhow::{Context, Result};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::mpsc;

/// Performance metrics collector with history tracking
pub struct MetricsCollector {
    /// CPU usage history (percentage)
    cpu_history: VecDeque<f32>,
    
    /// Memory usage history (MB)
    memory_history: VecDeque<f32>,
    
    /// GPU usage history (percentage)
    gpu_history: VecDeque<f32>,
    
    /// Frame time history (milliseconds)
    frame_time_history: VecDeque<f32>,
    
    /// Window count history
    window_count_history: VecDeque<u32>,
    
    /// Effects quality history (percentage)
    effects_quality_history: VecDeque<f32>,
    
    /// History size limit
    history_size: usize,
    
    /// Last update time
    last_update: Instant,
    
    /// Metrics sender channel
    sender: Option<mpsc::UnboundedSender<PerformanceMetrics>>,
}

/// Comprehensive performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    /// Unix timestamp
    pub timestamp: u64,
    
    /// Current metrics
    pub current: MetricsSnapshot,
    
    /// Average metrics over last minute
    pub average: MetricsSnapshot,
    
    /// Peak metrics over last minute
    pub peak: MetricsSnapshot,
    
    /// System health score (0-100)
    pub health_score: f32,
    
    /// Optimization suggestions
    pub suggestions: Vec<OptimizationSuggestion>,
}

/// Point-in-time metrics snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub cpu_usage: f32,
    pub memory_usage: f32,
    pub gpu_usage: f32,
    pub frame_time: f32,
    pub fps: f32,
    pub window_count: u32,
    pub effects_quality: f32,
    pub workspace_scroll_speed: f32,
}

/// AI optimization suggestion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationSuggestion {
    pub category: String,
    pub description: String,
    pub config_key: String,
    pub suggested_value: serde_json::Value,
    pub impact: String,
    pub priority: u8,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new(history_size: usize) -> Self {
        Self {
            cpu_history: VecDeque::with_capacity(history_size),
            memory_history: VecDeque::with_capacity(history_size),
            gpu_history: VecDeque::with_capacity(history_size),
            frame_time_history: VecDeque::with_capacity(history_size),
            window_count_history: VecDeque::with_capacity(history_size),
            effects_quality_history: VecDeque::with_capacity(history_size),
            history_size,
            last_update: Instant::now(),
            sender: None,
        }
    }
    
    /// Set the metrics sender channel
    pub fn set_sender(&mut self, sender: mpsc::UnboundedSender<PerformanceMetrics>) {
        self.sender = Some(sender);
    }
    
    /// Update metrics with new data
    pub fn update(
        &mut self,
        cpu_usage: f32,
        memory_usage: f32,
        gpu_usage: f32,
        frame_time: f32,
        window_count: u32,
        effects_quality: f32,
    ) {
        // Add to history
        self.add_to_history(&mut self.cpu_history, cpu_usage);
        self.add_to_history(&mut self.memory_history, memory_usage);
        self.add_to_history(&mut self.gpu_history, gpu_usage);
        self.add_to_history(&mut self.frame_time_history, frame_time);
        self.add_to_history(&mut self.window_count_history, window_count as f32);
        self.add_to_history(&mut self.effects_quality_history, effects_quality);
        
        self.last_update = Instant::now();
        
        // Generate and send metrics if we have a sender
        if let Some(sender) = &self.sender {
            if let Ok(metrics) = self.generate_metrics() {
                let _ = sender.send(metrics);
            }
        }
    }
    
    /// Add value to history with size limit
    fn add_to_history(&self, history: &mut VecDeque<f32>, value: f32) {
        if history.len() >= self.history_size {
            history.pop_front();
        }
        history.push_back(value);
    }
    
    /// Generate comprehensive performance metrics
    pub fn generate_metrics(&self) -> Result<PerformanceMetrics> {
        let current = self.get_current_snapshot();
        let average = self.get_average_snapshot();
        let peak = self.get_peak_snapshot();
        let health_score = self.calculate_health_score(&current);
        let suggestions = self.generate_suggestions(&current, &average);
        
        Ok(PerformanceMetrics {
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_secs(),
            current,
            average,
            peak,
            health_score,
            suggestions,
        })
    }
    
    /// Get current metrics snapshot
    fn get_current_snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            cpu_usage: self.cpu_history.back().copied().unwrap_or(0.0),
            memory_usage: self.memory_history.back().copied().unwrap_or(0.0),
            gpu_usage: self.gpu_history.back().copied().unwrap_or(0.0),
            frame_time: self.frame_time_history.back().copied().unwrap_or(16.67),
            fps: 1000.0 / self.frame_time_history.back().copied().unwrap_or(16.67),
            window_count: self.window_count_history.back()
                .copied()
                .unwrap_or(0.0) as u32,
            effects_quality: self.effects_quality_history.back()
                .copied()
                .unwrap_or(100.0),
            workspace_scroll_speed: crate::workspace::get_global_scroll_speed() as f32,
        }
    }
    
    /// Get average metrics snapshot
    fn get_average_snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            cpu_usage: self.calculate_average(&self.cpu_history),
            memory_usage: self.calculate_average(&self.memory_history),
            gpu_usage: self.calculate_average(&self.gpu_history),
            frame_time: self.calculate_average(&self.frame_time_history),
            fps: 1000.0 / self.calculate_average(&self.frame_time_history).max(1.0),
            window_count: self.calculate_average(&self.window_count_history) as u32,
            effects_quality: self.calculate_average(&self.effects_quality_history),
            workspace_scroll_speed: crate::workspace::get_global_scroll_speed() as f32,
        }
    }
    
    /// Get peak metrics snapshot
    fn get_peak_snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            cpu_usage: self.calculate_peak(&self.cpu_history),
            memory_usage: self.calculate_peak(&self.memory_history),
            gpu_usage: self.calculate_peak(&self.gpu_history),
            frame_time: self.calculate_peak(&self.frame_time_history),
            fps: 1000.0 / self.calculate_min(&self.frame_time_history).max(1.0),
            window_count: self.calculate_peak(&self.window_count_history) as u32,
            effects_quality: self.calculate_min(&self.effects_quality_history),
            workspace_scroll_speed: crate::workspace::get_global_scroll_speed() as f32,
        }
    }
    
    /// Calculate average of history
    fn calculate_average(&self, history: &VecDeque<f32>) -> f32 {
        if history.is_empty() {
            return 0.0;
        }
        history.iter().sum::<f32>() / history.len() as f32
    }
    
    /// Calculate peak (maximum) of history
    fn calculate_peak(&self, history: &VecDeque<f32>) -> f32 {
        history.iter().cloned().fold(0.0, f32::max)
    }
    
    /// Calculate minimum of history
    fn calculate_min(&self, history: &VecDeque<f32>) -> f32 {
        history.iter().cloned().fold(f32::MAX, f32::min)
    }
    
    /// Calculate system health score (0-100)
    fn calculate_health_score(&self, current: &MetricsSnapshot) -> f32 {
        let mut score = 100.0;
        
        // Penalize high CPU usage
        if current.cpu_usage > 80.0 {
            score -= (current.cpu_usage - 80.0) * 0.5;
        }
        
        // Penalize high memory usage
        if current.memory_usage > 1024.0 { // Over 1GB
            score -= ((current.memory_usage - 1024.0) / 100.0).min(20.0);
        }
        
        // Penalize low FPS
        if current.fps < 30.0 {
            score -= (30.0 - current.fps) * 2.0;
        } else if current.fps < 60.0 {
            score -= (60.0 - current.fps) * 0.5;
        }
        
        // Penalize reduced effects quality
        score -= (100.0 - current.effects_quality) * 0.3;
        
        score.max(0.0).min(100.0)
    }
    
    /// Generate optimization suggestions based on metrics
    fn generate_suggestions(
        &self,
        current: &MetricsSnapshot,
        average: &MetricsSnapshot,
    ) -> Vec<OptimizationSuggestion> {
        let mut suggestions = Vec::new();
        
        // Check if effects should be reduced
        if current.fps < 30.0 && current.effects_quality > 50.0 {
            suggestions.push(OptimizationSuggestion {
                category: "Performance".to_string(),
                description: "Reduce effects quality to improve frame rate".to_string(),
                config_key: "effects.quality".to_string(),
                suggested_value: serde_json::json!(current.effects_quality * 0.7),
                impact: "High".to_string(),
                priority: 1,
            });
        }
        
        // Check if blur radius should be reduced
        if current.gpu_usage > 80.0 {
            suggestions.push(OptimizationSuggestion {
                category: "GPU".to_string(),
                description: "Reduce blur radius to lower GPU usage".to_string(),
                config_key: "effects.blur.radius".to_string(),
                suggested_value: serde_json::json!(5),
                impact: "Medium".to_string(),
                priority: 2,
            });
        }
        
        // Check if animation speed should be adjusted
        if current.window_count > 10 && average.frame_time > 20.0 {
            suggestions.push(OptimizationSuggestion {
                category: "Animation".to_string(),
                description: "Speed up animations for better responsiveness".to_string(),
                config_key: "effects.animations.duration".to_string(),
                suggested_value: serde_json::json!(200),
                impact: "Low".to_string(),
                priority: 3,
            });
        }
        
        // Check memory usage
        if current.memory_usage > 2048.0 {
            suggestions.push(OptimizationSuggestion {
                category: "Memory".to_string(),
                description: "Consider closing unused windows to free memory".to_string(),
                config_key: "workspace.max_windows".to_string(),
                suggested_value: serde_json::json!(20),
                impact: "Medium".to_string(),
                priority: 2,
            });
        }
        
        suggestions
    }
}

/// System resource monitor using /proc filesystem
pub struct SystemMonitor {
    /// Previous CPU stats for usage calculation
    prev_cpu_stats: Option<CpuStats>,
    
    /// Process ID
    pid: u32,
}

#[derive(Debug, Clone)]
struct CpuStats {
    user: u64,
    system: u64,
    idle: u64,
}

impl SystemMonitor {
    /// Create a new system monitor
    pub fn new() -> Self {
        Self {
            prev_cpu_stats: None,
            pid: std::process::id(),
        }
    }
    
    /// Get current CPU usage percentage
    pub fn get_cpu_usage(&mut self) -> f32 {
        if let Ok(stats) = self.read_cpu_stats() {
            if let Some(prev) = &self.prev_cpu_stats {
                let total_diff = (stats.user + stats.system + stats.idle)
                    - (prev.user + prev.system + prev.idle);
                let active_diff = (stats.user + stats.system)
                    - (prev.user + prev.system);
                
                let usage = if total_diff > 0 {
                    (active_diff as f32 / total_diff as f32) * 100.0
                } else {
                    0.0
                };
                
                self.prev_cpu_stats = Some(stats);
                return usage;
            }
            self.prev_cpu_stats = Some(stats);
        }
        0.0
    }
    
    /// Get current memory usage in MB
    pub fn get_memory_usage(&self) -> f32 {
        if let Ok(content) = std::fs::read_to_string(format!("/proc/{}/status", self.pid)) {
            for line in content.lines() {
                if line.starts_with("VmRSS:") {
                    if let Some(kb_str) = line.split_whitespace().nth(1) {
                        if let Ok(kb) = kb_str.parse::<f32>() {
                            return kb / 1024.0; // Convert KB to MB
                        }
                    }
                }
            }
        }
        0.0
    }
    
    /// Read CPU statistics from /proc/stat
    fn read_cpu_stats(&self) -> Result<CpuStats> {
        let content = std::fs::read_to_string("/proc/stat")
            .context("Failed to read /proc/stat")?;
        
        let cpu_line = content
            .lines()
            .find(|line| line.starts_with("cpu "))
            .ok_or_else(|| anyhow::anyhow!("No CPU line in /proc/stat"))?;
        
        let values: Vec<u64> = cpu_line
            .split_whitespace()
            .skip(1)
            .filter_map(|v| v.parse().ok())
            .collect();
        
        if values.len() < 4 {
            return Err(anyhow::anyhow!("Invalid CPU stats format"));
        }
        
        Ok(CpuStats {
            user: values[0] + values[1], // user + nice
            system: values[2],
            idle: values[3],
        })
    }
}
