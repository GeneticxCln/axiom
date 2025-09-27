//! Texture Pool Optimization
//! 
//! Implements advanced texture memory management with LRU eviction,
//! pre-allocation, and format-specific optimization for better performance.

use anyhow::Result;
use log::{debug, info, warn};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use wgpu::{Device, Texture, TextureDescriptor, TextureFormat, TextureDimension, TextureUsages, Extent3d};

/// Enhanced texture pool with LRU eviction and pre-allocation
pub struct OptimizedTexturePool {
    /// Format-specific texture pools  
    pools: HashMap<TextureFormat, FormatSpecificPool>,
    
    /// Pool statistics for monitoring and optimization
    stats: TexturePoolStats,
    
    /// Configuration for pool behavior
    config: TexturePoolConfig,
}

/// Pool for a specific texture format with LRU management
#[derive(Debug)]
struct FormatSpecificPool {
    /// Available textures by size, with LRU ordering
    available: HashMap<(u32, u32), VecDeque<PooledTexture>>,
    
    /// Currently allocated texture count by size
    allocated_count: HashMap<(u32, u32), usize>,
    
    /// Maximum textures to keep per size
    max_per_size: usize,
    
    /// Total memory usage for this format (approximate bytes)
    memory_usage: usize,
}

/// Texture wrapper with metadata for pool management
#[derive(Debug)]
struct PooledTexture {
    texture: Texture,
    created_at: Instant,
    last_used: Instant,
    size: (u32, u32),
    memory_estimate: usize,
}

/// Pool configuration parameters
#[derive(Debug, Clone)]
pub struct TexturePoolConfig {
    /// Maximum total memory usage (bytes)
    pub max_total_memory: usize,
    
    /// Maximum textures per size bucket
    pub max_per_size: usize,
    
    /// How long to keep unused textures (seconds)
    pub eviction_timeout: Duration,
    
    /// Pre-allocate these common window sizes
    pub common_sizes: Vec<(u32, u32)>,
    
    /// Enable aggressive cleanup when memory pressure is high
    pub aggressive_cleanup: bool,
}

/// Pool performance statistics
#[derive(Debug, Default)]
pub struct TexturePoolStats {
    pub total_allocations: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub evictions: u64,
    pub memory_pressure_cleanups: u64,
    pub current_memory_usage: usize,
    pub peak_memory_usage: usize,
}

impl Default for TexturePoolConfig {
    fn default() -> Self {
        Self {
            max_total_memory: 256 * 1024 * 1024, // 256 MB
            max_per_size: 8,
            eviction_timeout: Duration::from_secs(30),
            common_sizes: vec![
                (800, 600),    // Classic 4:3
                (1024, 768),   // XGA
                (1280, 720),   // 720p
                (1366, 768),   // Common laptop
                (1920, 1080),  // 1080p
                (2560, 1440),  // 1440p
                (3840, 2160),  // 4K
            ],
            aggressive_cleanup: true,
        }
    }
}

impl OptimizedTexturePool {
    /// Create a new optimized texture pool
    pub fn new(config: TexturePoolConfig) -> Self {
        info!("🔧 Creating optimized texture pool");
        info!("   Max memory: {} MB", config.max_total_memory / (1024 * 1024));
        info!("   Max per size: {}", config.max_per_size);
        info!("   Eviction timeout: {:?}", config.eviction_timeout);
        info!("   Common sizes: {:?}", config.common_sizes);
        
        Self {
            pools: HashMap::new(),
            stats: TexturePoolStats::default(),
            config,
        }
    }
    
    /// Pre-allocate textures for common window sizes
    pub fn pre_allocate(&mut self, device: &Device) -> Result<()> {
        info!("🚀 Pre-allocating textures for common sizes");
        
        let format = TextureFormat::Rgba8UnormSrgb;
        let pool = self.pools.entry(format).or_insert_with(|| FormatSpecificPool::new(self.config.max_per_size));
        
        for &(width, height) in &self.config.common_sizes {
            // Pre-allocate 2 textures per common size
            for _ in 0..2 {
                let texture = self.create_texture(device, width, height, format)?;
                let pooled = PooledTexture::new(texture, (width, height), Self::estimate_memory_usage(width, height, format));
                
                pool.available
                    .entry((width, height))
                    .or_insert_with(VecDeque::new)
                    .push_back(pooled);
            }
            
            debug!("✅ Pre-allocated 2 textures for {}x{}", width, height);
        }
        
        info!("✅ Pre-allocation complete");
        Ok(())
    }
    
    /// Get a texture from the pool or create new one
    pub fn get_texture(&mut self, device: &Device, width: u32, height: u32, format: TextureFormat) -> Result<Texture> {
        self.stats.total_allocations += 1;
        
        let pool = self.pools.entry(format).or_insert_with(|| FormatSpecificPool::new(self.config.max_per_size));
        
        // Try to get from cache first
        if let Some(available) = pool.available.get_mut(&(width, height)) {
            if let Some(mut pooled) = available.pop_front() {
                pooled.last_used = Instant::now();
                self.stats.cache_hits += 1;
                
                // Track as allocated
                *pool.allocated_count.entry((width, height)).or_insert(0) += 1;
                
                debug!("🎯 Cache hit for {}x{} texture (format: {:?})", width, height, format);
                return Ok(pooled.texture);
            }
        }
        
        // Cache miss - create new texture
        self.stats.cache_misses += 1;
        let texture = self.create_texture(device, width, height, format)?;
        
        // Track allocation
        *pool.allocated_count.entry((width, height)).or_insert(0) += 1;
        
        debug!("🔧 Cache miss - created new {}x{} texture (format: {:?})", width, height, format);
        Ok(texture)
    }
    
    /// Return a texture to the pool for reuse
    pub fn return_texture(&mut self, texture: Texture, width: u32, height: u32, format: TextureFormat) {
        let pool = self.pools.entry(format).or_insert_with(|| FormatSpecificPool::new(self.config.max_per_size));
        
        // Update allocation count
        if let Some(count) = pool.allocated_count.get_mut(&(width, height)) {
            *count = count.saturating_sub(1);
        }
        
        // Check if we have room in the pool
        let available_queue = pool.available.entry((width, height)).or_insert_with(VecDeque::new);
        
        if available_queue.len() >= self.config.max_per_size {
            // Pool is full - evict oldest texture
            if let Some(old) = available_queue.pop_front() {
                pool.memory_usage = pool.memory_usage.saturating_sub(old.memory_estimate);
                self.stats.current_memory_usage = self.stats.current_memory_usage.saturating_sub(old.memory_estimate);
                self.stats.evictions += 1;
                debug!("🗑️ Evicted old texture due to pool size limit");
            }
        }
        
        // Add texture back to pool
        let memory_estimate = Self::estimate_memory_usage(width, height, format);
        let pooled = PooledTexture::new(texture, (width, height), memory_estimate);
        
        available_queue.push_back(pooled);
        pool.memory_usage += memory_estimate;
        self.stats.current_memory_usage += memory_estimate;
        
        if self.stats.current_memory_usage > self.stats.peak_memory_usage {
            self.stats.peak_memory_usage = self.stats.current_memory_usage;
        }
        
        debug!("♻️ Returned {}x{} texture to pool", width, height);
        
        // Check for memory pressure
        self.check_memory_pressure();
    }
    
    /// Periodic cleanup of old textures
    pub fn cleanup_expired(&mut self) {
        let now = Instant::now();
        let mut total_evicted = 0;
        
        for (format, pool) in &mut self.pools {
            for (size, queue) in &mut pool.available {
                let original_len = queue.len();
                
                // Remove expired textures
                queue.retain(|pooled| {
                    let should_keep = now.duration_since(pooled.last_used) < self.config.eviction_timeout;
                    if !should_keep {
                        pool.memory_usage = pool.memory_usage.saturating_sub(pooled.memory_estimate);
                        self.stats.current_memory_usage = self.stats.current_memory_usage.saturating_sub(pooled.memory_estimate);
                    }
                    should_keep
                });
                
                let evicted = original_len - queue.len();
                total_evicted += evicted;
                
                if evicted > 0 {
                    debug!("🧹 Evicted {} expired {}x{} textures (format: {:?})", evicted, size.0, size.1, format);
                }
            }
        }
        
        if total_evicted > 0 {
            self.stats.evictions += total_evicted as u64;
            info!("🧹 Cleaned up {} expired textures", total_evicted);
        }
    }
    
    /// Force cleanup when memory pressure is high
    fn check_memory_pressure(&mut self) {
        if self.stats.current_memory_usage > self.config.max_total_memory {
            info!("⚠️ Memory pressure detected - triggering aggressive cleanup");
            self.aggressive_cleanup();
        }
    }
    
    /// Aggressive cleanup to free memory
    fn aggressive_cleanup(&mut self) {
        if !self.config.aggressive_cleanup {
            return;
        }
        
        self.stats.memory_pressure_cleanups += 1;
        let mut freed_memory = 0;
        
        // Sort textures by last used time and evict oldest first
        let mut textures_by_age: Vec<(Instant, (u32, u32), TextureFormat)> = Vec::new();
        
        for (format, pool) in &self.pools {
            for (size, queue) in &pool.available {
                for pooled in queue {
                    textures_by_age.push((pooled.last_used, *size, *format));
                }
            }
        }
        
        textures_by_age.sort_by_key(|(last_used, _, _)| *last_used);
        
        // Remove oldest 25% of textures
        let to_remove = textures_by_age.len() / 4;
        for (_, (width, height), format) in textures_by_age.into_iter().take(to_remove) {
            if let Some(pool) = self.pools.get_mut(&format) {
                if let Some(queue) = pool.available.get_mut(&(width, height)) {
                    if let Some(pooled) = queue.pop_front() {
                        freed_memory += pooled.memory_estimate;
                        pool.memory_usage = pool.memory_usage.saturating_sub(pooled.memory_estimate);
                    }
                }
            }
        }
        
        self.stats.current_memory_usage = self.stats.current_memory_usage.saturating_sub(freed_memory);
        self.stats.evictions += to_remove as u64;
        
        info!("🧹 Aggressive cleanup freed {} MB", freed_memory / (1024 * 1024));
    }
    
    /// Get pool statistics
    pub fn stats(&self) -> &TexturePoolStats {
        &self.stats
    }
    
    /// Create a new texture with the given parameters
    fn create_texture(&self, device: &Device, width: u32, height: u32, format: TextureFormat) -> Result<Texture> {
        let texture = device.create_texture(&TextureDescriptor {
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            label: Some(&format!("Pooled Texture {}x{}", width, height)),
            view_formats: &[],
        });
        
        Ok(texture)
    }
    
    /// Estimate memory usage for a texture
    fn estimate_memory_usage(width: u32, height: u32, format: TextureFormat) -> usize {
        let bytes_per_pixel = match format {
            TextureFormat::Rgba8UnormSrgb => 4,
            TextureFormat::Rgb8UnormSrgb => 3,
            TextureFormat::Rg8Unorm => 2,
            TextureFormat::R8Unorm => 1,
            _ => 4, // Default assumption
        };
        
        (width * height * bytes_per_pixel) as usize
    }
}

impl FormatSpecificPool {
    fn new(max_per_size: usize) -> Self {
        Self {
            available: HashMap::new(),
            allocated_count: HashMap::new(),
            max_per_size,
            memory_usage: 0,
        }
    }
}

impl PooledTexture {
    fn new(texture: Texture, size: (u32, u32), memory_estimate: usize) -> Self {
        let now = Instant::now();
        Self {
            texture,
            created_at: now,
            last_used: now,
            size,
            memory_estimate,
        }
    }
}

/// Damage Region Coalescing
/// 
/// Optimizes texture update operations by merging adjacent and overlapping damage regions
pub struct DamageCoalescer {
    /// Maximum number of regions to maintain per window
    max_regions: usize,
    
    /// Minimum region size to avoid micro-updates
    min_region_size: u32,
    
    /// Distance threshold for merging adjacent regions
    merge_threshold: u32,
}

impl Default for DamageCoalescer {
    fn default() -> Self {
        Self {
            max_regions: 8,
            min_region_size: 64, // 8x8 minimum
            merge_threshold: 32, // Merge if within 32 pixels
        }
    }
}

impl DamageCoalescer {
    pub fn new(max_regions: usize, min_region_size: u32, merge_threshold: u32) -> Self {
        Self {
            max_regions,
            min_region_size,
            merge_threshold,
        }
    }
    
    /// Coalesce damage regions for optimal texture updates
    pub fn coalesce_regions(&self, mut regions: Vec<(u32, u32, u32, u32)>, window_width: u32, window_height: u32) -> Vec<(u32, u32, u32, u32)> {
        if regions.is_empty() {
            return regions;
        }
        
        // Filter out tiny regions
        regions.retain(|(_, _, w, h)| w * h >= self.min_region_size);
        
        // Sort regions by position for efficient merging
        regions.sort_by_key(|(x, y, _, _)| (*y, *x));
        
        let mut coalesced = Vec::new();
        let mut current = regions[0];
        
        for &(x, y, w, h) in &regions[1..] {
            if self.should_merge(current, (x, y, w, h)) {
                // Merge regions
                current = self.merge_regions(current, (x, y, w, h));
            } else {
                // Add current region and start new one
                coalesced.push(self.clamp_region(current, window_width, window_height));
                current = (x, y, w, h);
            }
        }
        
        // Add the last region
        coalesced.push(self.clamp_region(current, window_width, window_height));
        
        // If still too many regions, merge most adjacent ones
        while coalesced.len() > self.max_regions && coalesced.len() > 1 {
            let merge_idx = self.find_best_merge_candidates(&coalesced);
            let region_a = coalesced[merge_idx];
            let region_b = coalesced[merge_idx + 1];
            let merged = self.merge_regions(region_a, region_b);
            
            coalesced[merge_idx] = merged;
            coalesced.remove(merge_idx + 1);
        }
        
        debug!("🔧 Coalesced {} regions into {} (window: {}x{})", regions.len(), coalesced.len(), window_width, window_height);
        coalesced
    }
    
    /// Check if two regions should be merged
    fn should_merge(&self, a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> bool {
        let (ax, ay, aw, ah) = a;
        let (bx, by, bw, bh) = b;
        
        // Calculate distance between regions
        let ax2 = ax + aw;
        let ay2 = ay + ah;
        let bx2 = bx + bw;
        let by2 = by + bh;
        
        // Check if regions overlap or are within merge threshold
        let overlap_x = ax < bx2 && bx < ax2;
        let overlap_y = ay < by2 && by < ay2;
        
        let distance_x = if overlap_x { 0 } else { ax.min(bx).abs_diff(ax.max(bx).min(ax2).max(bx2)) };
        let distance_y = if overlap_y { 0 } else { ay.min(by).abs_diff(ay.max(by).min(ay2).max(by2)) };
        
        overlap_x && overlap_y || (distance_x + distance_y) <= self.merge_threshold
    }
    
    /// Merge two regions into a bounding rectangle
    fn merge_regions(&self, a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
        let (ax, ay, aw, ah) = a;
        let (bx, by, bw, bh) = b;
        
        let min_x = ax.min(bx);
        let min_y = ay.min(by);
        let max_x = (ax + aw).max(bx + bw);
        let max_y = (ay + ah).max(by + bh);
        
        (min_x, min_y, max_x - min_x, max_y - min_y)
    }
    
    /// Find the best pair of regions to merge
    fn find_best_merge_candidates(&self, regions: &[(u32, u32, u32, u32)]) -> usize {
        let mut best_idx = 0;
        let mut best_area_increase = u32::MAX;
        
        for i in 0..regions.len() - 1 {
            let merged = self.merge_regions(regions[i], regions[i + 1]);
            let area_a = regions[i].2 * regions[i].3;
            let area_b = regions[i + 1].2 * regions[i + 1].3;
            let area_merged = merged.2 * merged.3;
            let area_increase = area_merged.saturating_sub(area_a + area_b);
            
            if area_increase < best_area_increase {
                best_area_increase = area_increase;
                best_idx = i;
            }
        }
        
        best_idx
    }
    
    /// Clamp region to window bounds
    fn clamp_region(&self, region: (u32, u32, u32, u32), window_width: u32, window_height: u32) -> (u32, u32, u32, u32) {
        let (x, y, w, h) = region;
        let clamped_x = x.min(window_width);
        let clamped_y = y.min(window_height);
        let clamped_w = w.min(window_width.saturating_sub(clamped_x));
        let clamped_h = h.min(window_height.saturating_sub(clamped_y));
        
        (clamped_x, clamped_y, clamped_w, clamped_h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_damage_coalescing() {
        let coalescer = DamageCoalescer::default();
        
        // Test merging overlapping regions
        let regions = vec![
            (0, 0, 100, 100),     // Base region
            (50, 50, 100, 100),   // Overlapping region
            (200, 200, 50, 50),   // Separate region
        ];
        
        let coalesced = coalescer.coalesce_regions(regions, 1000, 1000);
        
        // Should merge first two regions, keep third separate
        assert_eq!(coalesced.len(), 2);
        
        // First region should be bounding box of first two
        assert_eq!(coalesced[0], (0, 0, 150, 150));
        assert_eq!(coalesced[1], (200, 200, 50, 50));
    }
    
    #[test] 
    fn test_texture_pool_stats() {
        let config = TexturePoolConfig::default();
        let pool = OptimizedTexturePool::new(config);
        
        // Initially no allocations
        assert_eq!(pool.stats().total_allocations, 0);
        assert_eq!(pool.stats().cache_hits, 0);
        assert_eq!(pool.stats().cache_misses, 0);
    }
}