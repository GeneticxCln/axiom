//! Damage tracking for efficient compositor rendering
//!
//! This module provides damage tracking functionality that allows the compositor
//! to only re-render portions of the screen that have actually changed, dramatically
//! improving performance and reducing power consumption.
//!
//! # Overview
//!
//! Damage tracking works by:
//! 1. Tracking which windows have changed (received new buffer commits)
//! 2. Recording the specific regions within each window that changed
//! 3. Computing output damage (screen regions that need repainting)
//! 4. Only rendering the damaged regions during the render pass
//!
//! # Example
//!
//! ```
//! use axiom::renderer::damage::{DamageRegion, FrameDamage};
//!
//! let mut frame_damage = FrameDamage::new();
//!
//! // Window 1 updated a small region
//! frame_damage.add_window_damage(1, DamageRegion::new(10, 10, 50, 50));
//!
//! // Window 2 fully updated
//! frame_damage.mark_window_damaged(2);
//!
//! // Check if frame has damage
//! assert!(frame_damage.has_any_damage());
//! ```

use std::collections::HashMap;

/// Maximum number of damage regions per window before coalescing to full damage
const MAX_DAMAGE_REGIONS: usize = 16;

/// Represents a rectangular region that needs repainting
///
/// Damage regions are axis-aligned rectangles specified in pixel coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DamageRegion {
    /// X coordinate (pixels)
    pub x: i32,
    /// Y coordinate (pixels)
    pub y: i32,
    /// Width (pixels)
    pub width: u32,
    /// Height (pixels)
    pub height: u32,
}

impl DamageRegion {
    /// Creates a new damage region
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate in pixels
    /// * `y` - Y coordinate in pixels
    /// * `width` - Width in pixels
    /// * `height` - Height in pixels
    ///
    /// # Example
    ///
    /// ```
    /// use axiom::renderer::damage::DamageRegion;
    ///
    /// let region = DamageRegion::new(100, 100, 200, 150);
    /// assert_eq!(region.area(), 30000);
    /// ```
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Creates a damage region from two corners
    pub fn from_corners(x1: i32, y1: i32, x2: i32, y2: i32) -> Self {
        let x = x1.min(x2);
        let y = y1.min(y2);
        let width = (x1.max(x2) - x) as u32;
        let height = (y1.max(y2) - y) as u32;

        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Returns the area of this region in pixels
    pub fn area(&self) -> u32 {
        self.width * self.height
    }

    /// Checks if this region intersects another region
    ///
    /// Two regions intersect if they share any pixels.
    pub fn intersects(&self, other: &DamageRegion) -> bool {
        !(self.x + self.width as i32 <= other.x
            || other.x + other.width as i32 <= self.x
            || self.y + self.height as i32 <= other.y
            || other.y + other.height as i32 <= self.y)
    }

    /// Checks if this region is adjacent to another (touching but not overlapping)
    pub fn is_adjacent(&self, other: &DamageRegion, threshold: u32) -> bool {
        let h_adjacent = (self.x + self.width as i32 + threshold as i32 >= other.x
            && self.x <= other.x + other.width as i32 + threshold as i32)
            && (self.y < other.y + other.height as i32 && other.y < self.y + self.height as i32);

        let v_adjacent = (self.y + self.height as i32 + threshold as i32 >= other.y
            && self.y <= other.y + other.height as i32 + threshold as i32)
            && (self.x < other.x + other.width as i32 && other.x < self.x + self.width as i32);

        h_adjacent || v_adjacent
    }

    /// Computes the union of two regions (smallest bounding box containing both)
    pub fn union(&self, other: &DamageRegion) -> DamageRegion {
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = (self.x + self.width as i32).max(other.x + other.width as i32);
        let y2 = (self.y + self.height as i32).max(other.y + other.height as i32);

        DamageRegion {
            x: x1,
            y: y1,
            width: (x2 - x1) as u32,
            height: (y2 - y1) as u32,
        }
    }

    /// Computes the intersection of two regions
    ///
    /// Returns `None` if the regions don't intersect.
    pub fn intersection(&self, other: &DamageRegion) -> Option<DamageRegion> {
        if !self.intersects(other) {
            return None;
        }

        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.width as i32).min(other.x + other.width as i32);
        let y2 = (self.y + self.height as i32).min(other.y + other.height as i32);

        Some(DamageRegion {
            x: x1,
            y: y1,
            width: (x2 - x1) as u32,
            height: (y2 - y1) as u32,
        })
    }

    /// Converts this region to screen coordinates given a window position
    ///
    /// # Arguments
    ///
    /// * `window_x` - Window X position on screen
    /// * `window_y` - Window Y position on screen
    pub fn to_screen_coords(&self, window_x: i32, window_y: i32) -> DamageRegion {
        DamageRegion {
            x: self.x + window_x,
            y: self.y + window_y,
            width: self.width,
            height: self.height,
        }
    }

    /// Checks if this region contains a point
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        x >= self.x
            && x < self.x + self.width as i32
            && y >= self.y
            && y < self.y + self.height as i32
    }

    /// Checks if this region completely contains another region
    pub fn contains_region(&self, other: &DamageRegion) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.width as i32 <= self.x + self.width as i32
            && other.y + other.height as i32 <= self.y + self.height as i32
    }
}

/// Tracks damage state for a single window
///
/// A window can be either fully damaged (entire contents need repainting) or
/// have specific damaged regions.
#[derive(Debug, Clone)]
pub struct WindowDamage {
    /// Window ID
    pub window_id: u64,

    /// Damaged regions in window coordinates
    pub regions: Vec<DamageRegion>,

    /// Is the entire window damaged?
    pub full_damage: bool,

    /// Frame number when damage was last added
    pub frame_number: u64,
}

impl WindowDamage {
    /// Creates new window damage state
    pub fn new(window_id: u64, frame_number: u64) -> Self {
        Self {
            window_id,
            regions: Vec::new(),
            full_damage: false,
            frame_number,
        }
    }

    /// Adds damage to a specific region
    ///
    /// If the window already has full damage, this is a no-op.
    /// If too many regions accumulate, the window is marked as fully damaged.
    pub fn add_region(&mut self, region: DamageRegion) {
        if self.full_damage {
            return; // Already fully damaged
        }

        self.regions.push(region);

        // If too many regions, mark as fully damaged to avoid overhead
        if self.regions.len() > MAX_DAMAGE_REGIONS {
            self.full_damage = true;
            self.regions.clear();
        }
    }

    /// Marks the entire window as damaged
    pub fn mark_full(&mut self) {
        self.full_damage = true;
        self.regions.clear();
    }

    /// Clears all damage
    pub fn clear(&mut self) {
        self.full_damage = false;
        self.regions.clear();
    }

    /// Checks if the window has any damage
    pub fn has_damage(&self) -> bool {
        self.full_damage || !self.regions.is_empty()
    }

    /// Merges overlapping and adjacent damage regions to reduce overhead
    ///
    /// This is an optimization that reduces the number of regions while
    /// maintaining coverage of all damaged areas.
    pub fn merge_regions(&mut self) {
        if self.full_damage || self.regions.len() <= 1 {
            return;
        }

        // Sort regions by position for efficient merging
        self.regions.sort_by_key(|r| (r.y, r.x));

        let mut merged = Vec::new();
        let mut current = self.regions[0];

        for region in &self.regions[1..] {
            if current.intersects(region) || current.is_adjacent(region, 10) {
                // Merge overlapping or nearby regions
                current = current.union(region);
            } else {
                merged.push(current);
                current = *region;
            }
        }
        merged.push(current);

        self.regions = merged;
    }

    /// Returns the total damaged area in pixels
    pub fn total_area(&self) -> u32 {
        if self.full_damage {
            u32::MAX // Represent as maximum
        } else {
            self.regions.iter().map(|r| r.area()).sum()
        }
    }
}

/// Accumulates damage across all windows for a frame
///
/// This tracks which windows have damage and computes the output damage
/// (screen regions that need repainting).
#[derive(Debug, Clone)]
pub struct FrameDamage {
    /// Per-window damage
    window_damage: HashMap<u64, WindowDamage>,

    /// Output damage in screen coordinates (computed from window damage)
    output_regions: Vec<DamageRegion>,

    /// Current frame number
    frame_number: u64,

    /// Whether output damage has been computed for this frame
    output_damage_valid: bool,
}

impl FrameDamage {
    /// Creates a new frame damage accumulator
    pub fn new() -> Self {
        Self {
            window_damage: HashMap::new(),
            output_regions: Vec::new(),
            frame_number: 0,
            output_damage_valid: false,
        }
    }

    /// Adds damage for a specific window region
    pub fn add_window_damage(&mut self, window_id: u64, region: DamageRegion) {
        let damage = self
            .window_damage
            .entry(window_id)
            .or_insert_with(|| WindowDamage::new(window_id, self.frame_number));

        damage.add_region(region);
        damage.frame_number = self.frame_number;
        self.output_damage_valid = false;
    }

    /// Marks an entire window as damaged
    pub fn mark_window_damaged(&mut self, window_id: u64) {
        let damage = self
            .window_damage
            .entry(window_id)
            .or_insert_with(|| WindowDamage::new(window_id, self.frame_number));

        damage.mark_full();
        damage.frame_number = self.frame_number;
        self.output_damage_valid = false;
    }

    /// Checks if any window has damage
    pub fn has_any_damage(&self) -> bool {
        self.window_damage
            .values()
            .any(|damage| damage.has_damage())
    }

    /// Gets damage for a specific window
    pub fn get_window_damage(&self, window_id: u64) -> Option<&WindowDamage> {
        self.window_damage.get(&window_id)
    }

    /// Gets mutable damage for a specific window
    pub fn get_window_damage_mut(&mut self, window_id: u64) -> Option<&mut WindowDamage> {
        self.window_damage.get_mut(&window_id)
    }

    /// Returns all damaged windows
    pub fn damaged_windows(&self) -> impl Iterator<Item = u64> + '_ {
        self.window_damage
            .iter()
            .filter(|(_, damage)| damage.has_damage())
            .map(|(&id, _)| id)
    }

    /// Computes output damage from window damage
    ///
    /// This converts per-window damage (in window coordinates) to screen damage
    /// (in screen coordinates) based on window positions.
    ///
    /// # Arguments
    ///
    /// * `window_positions` - HashMap of window ID to (x, y) position
    /// * `window_sizes` - HashMap of window ID to (width, height)
    pub fn compute_output_damage(
        &mut self,
        window_positions: &HashMap<u64, (i32, i32)>,
        window_sizes: &HashMap<u64, (u32, u32)>,
    ) {
        self.output_regions.clear();

        for (window_id, damage) in &self.window_damage {
            if !damage.has_damage() {
                continue;
            }

            let pos = match window_positions.get(window_id) {
                Some(&pos) => pos,
                None => continue, // Window not visible
            };

            if damage.full_damage {
                // Entire window damaged
                if let Some(&size) = window_sizes.get(window_id) {
                    let region = DamageRegion::new(pos.0, pos.1, size.0, size.1);
                    self.output_regions.push(region);
                }
            } else {
                // Specific regions damaged
                for region in &damage.regions {
                    let screen_region = region.to_screen_coords(pos.0, pos.1);
                    self.output_regions.push(screen_region);
                }
            }
        }

        // Coalesce overlapping/adjacent output regions to reduce render work
        self.merge_output_regions();

        self.output_damage_valid = true;
    }

    /// Returns the output damage regions in screen coordinates
    ///
    /// You must call `compute_output_damage()` first.
    pub fn output_regions(&self) -> &[DamageRegion] {
        &self.output_regions
    }

    /// Merge overlapping or adjacent output damage regions (screen space)
    pub fn merge_output_regions(&mut self) {
        if self.output_regions.len() <= 1 {
            return;
        }
        // Sort by scanline then x for deterministic merging
        self.output_regions.sort_by_key(|r| (r.y, r.x));
        let mut merged: Vec<DamageRegion> = Vec::with_capacity(self.output_regions.len());
        let mut current = self.output_regions[0];
        for r in &self.output_regions[1..] {
            if current.intersects(r) || current.is_adjacent(r, 10) {
                current = current.union(r);
            } else {
                merged.push(current);
                current = *r;
            }
        }
        merged.push(current);
        self.output_regions = merged;
    }

    /// Clears all damage after rendering
    ///
    /// This should be called after a frame has been successfully rendered.
    pub fn clear(&mut self) {
        self.window_damage.clear();
        self.output_regions.clear();
        self.frame_number += 1;
        self.output_damage_valid = false;
    }

    /// Returns the current frame number
    pub fn frame_number(&self) -> u64 {
        self.frame_number
    }

    /// Merges all window damage regions to optimize rendering
    pub fn merge_all_regions(&mut self) {
        for damage in self.window_damage.values_mut() {
            damage.merge_regions();
        }
    }
}

impl Default for FrameDamage {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_damage_region_new() {
        let region = DamageRegion::new(10, 20, 100, 50);
        assert_eq!(region.x, 10);
        assert_eq!(region.y, 20);
        assert_eq!(region.width, 100);
        assert_eq!(region.height, 50);
    }

    #[test]
    fn test_damage_region_area() {
        let region = DamageRegion::new(0, 0, 100, 50);
        assert_eq!(region.area(), 5000);
    }

    #[test]
    fn test_damage_region_intersection() {
        let r1 = DamageRegion::new(0, 0, 100, 100);
        let r2 = DamageRegion::new(50, 50, 100, 100);

        assert!(r1.intersects(&r2));
        assert!(r2.intersects(&r1));

        let r3 = DamageRegion::new(200, 200, 50, 50);
        assert!(!r1.intersects(&r3));
    }

    #[test]
    fn test_damage_region_intersection_result() {
        let r1 = DamageRegion::new(0, 0, 100, 100);
        let r2 = DamageRegion::new(50, 50, 100, 100);

        let intersection = r1.intersection(&r2).unwrap();
        assert_eq!(intersection, DamageRegion::new(50, 50, 50, 50));
    }

    #[test]
    fn test_damage_region_no_intersection() {
        let r1 = DamageRegion::new(0, 0, 50, 50);
        let r2 = DamageRegion::new(100, 100, 50, 50);

        assert!(r1.intersection(&r2).is_none());
    }

    #[test]
    fn test_damage_region_union() {
        let r1 = DamageRegion::new(0, 0, 100, 100);
        let r2 = DamageRegion::new(50, 50, 100, 100);

        let union = r1.union(&r2);
        assert_eq!(union, DamageRegion::new(0, 0, 150, 150));
    }

    #[test]
    fn test_damage_region_to_screen_coords() {
        let region = DamageRegion::new(10, 10, 50, 50);
        let screen = region.to_screen_coords(100, 200);

        assert_eq!(screen.x, 110);
        assert_eq!(screen.y, 210);
        assert_eq!(screen.width, 50);
        assert_eq!(screen.height, 50);
    }

    #[test]
    fn test_damage_region_contains_point() {
        let region = DamageRegion::new(10, 10, 50, 50);

        assert!(region.contains_point(10, 10));
        assert!(region.contains_point(30, 30));
        assert!(region.contains_point(59, 59));
        assert!(!region.contains_point(60, 60));
        assert!(!region.contains_point(0, 0));
    }

    #[test]
    fn test_damage_region_contains_region() {
        let outer = DamageRegion::new(0, 0, 100, 100);
        let inner = DamageRegion::new(10, 10, 50, 50);
        let overlapping = DamageRegion::new(50, 50, 100, 100);

        assert!(outer.contains_region(&inner));
        assert!(!outer.contains_region(&overlapping));
        assert!(!inner.contains_region(&outer));
    }

    #[test]
    fn test_window_damage_new() {
        let damage = WindowDamage::new(1, 0);
        assert_eq!(damage.window_id, 1);
        assert!(!damage.has_damage());
        assert!(!damage.full_damage);
    }

    #[test]
    fn test_window_damage_add_region() {
        let mut damage = WindowDamage::new(1, 0);
        damage.add_region(DamageRegion::new(0, 0, 10, 10));

        assert!(damage.has_damage());
        assert!(!damage.full_damage);
        assert_eq!(damage.regions.len(), 1);
    }

    #[test]
    fn test_window_damage_mark_full() {
        let mut damage = WindowDamage::new(1, 0);
        damage.add_region(DamageRegion::new(0, 0, 10, 10));
        damage.mark_full();

        assert!(damage.has_damage());
        assert!(damage.full_damage);
        assert_eq!(damage.regions.len(), 0);
    }

    #[test]
    fn test_window_damage_too_many_regions() {
        let mut damage = WindowDamage::new(1, 0);

        // Add more than MAX_DAMAGE_REGIONS
        for i in 0..20 {
            damage.add_region(DamageRegion::new(i * 10, 0, 10, 10));
        }

        // Should coalesce to full damage
        assert!(damage.full_damage);
        assert_eq!(damage.regions.len(), 0);
    }

    #[test]
    fn test_window_damage_clear() {
        let mut damage = WindowDamage::new(1, 0);
        damage.mark_full();
        damage.clear();

        assert!(!damage.has_damage());
        assert!(!damage.full_damage);
    }

    #[test]
    fn test_frame_damage_new() {
        let frame_damage = FrameDamage::new();
        assert!(!frame_damage.has_any_damage());
        assert_eq!(frame_damage.frame_number(), 0);
    }

    #[test]
    fn test_frame_damage_add_window_damage() {
        let mut frame_damage = FrameDamage::new();
        frame_damage.add_window_damage(1, DamageRegion::new(0, 0, 10, 10));

        assert!(frame_damage.has_any_damage());
        assert!(frame_damage.get_window_damage(1).is_some());
    }

    #[test]
    fn test_frame_damage_mark_window_damaged() {
        let mut frame_damage = FrameDamage::new();
        frame_damage.mark_window_damaged(1);

        let damage = frame_damage.get_window_damage(1).unwrap();
        assert!(damage.full_damage);
    }

    #[test]
    fn test_frame_damage_clear() {
        let mut frame_damage = FrameDamage::new();
        frame_damage.mark_window_damaged(1);

        let old_frame = frame_damage.frame_number();
        frame_damage.clear();

        assert!(!frame_damage.has_any_damage());
        assert_eq!(frame_damage.frame_number(), old_frame + 1);
    }

    #[test]
    fn test_frame_damage_compute_output() {
        let mut frame_damage = FrameDamage::new();
        frame_damage.add_window_damage(1, DamageRegion::new(10, 10, 50, 50));

        let mut positions = HashMap::new();
        positions.insert(1, (100, 100));

        let mut sizes = HashMap::new();
        sizes.insert(1, (200, 200));

        frame_damage.compute_output_damage(&positions, &sizes);

        let output = frame_damage.output_regions();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0], DamageRegion::new(110, 110, 50, 50));
    }

    #[test]
    fn test_frame_damage_damaged_windows() {
        let mut frame_damage = FrameDamage::new();
        frame_damage.mark_window_damaged(1);
        frame_damage.mark_window_damaged(2);

        let damaged: Vec<u64> = frame_damage.damaged_windows().collect();
        assert_eq!(damaged.len(), 2);
        assert!(damaged.contains(&1));
        assert!(damaged.contains(&2));
    }

    #[test]
    fn test_damage_region_from_corners() {
        let region = DamageRegion::from_corners(10, 20, 110, 70);
        assert_eq!(region.x, 10);
        assert_eq!(region.y, 20);
        assert_eq!(region.width, 100);
        assert_eq!(region.height, 50);
    }

    #[test]
    fn test_damage_region_from_corners_reversed() {
        let region = DamageRegion::from_corners(110, 70, 10, 20);
        assert_eq!(region.x, 10);
        assert_eq!(region.y, 20);
        assert_eq!(region.width, 100);
        assert_eq!(region.height, 50);
    }

    #[test]
    fn test_window_damage_merge_regions() {
        let mut damage = WindowDamage::new(1, 0);
        damage.add_region(DamageRegion::new(0, 0, 50, 50));
        damage.add_region(DamageRegion::new(40, 0, 50, 50));
        damage.add_region(DamageRegion::new(80, 0, 50, 50));

        damage.merge_regions();

        // Adjacent regions should be merged
        assert!(damage.regions.len() < 3);
    }
}
