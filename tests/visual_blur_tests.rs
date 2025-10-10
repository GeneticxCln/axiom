// Visual golden image tests for blur effects
//
// This module tests the Gaussian blur effect system with various configurations,
// generating golden reference images for visual regression testing.

use axiom::visual_tests::{VisualTestContext, VisualTestConfig, ComparisonResult};
use std::sync::Arc;

/// Helper to create test texture with gradient pattern
fn create_test_gradient(width: u32, height: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    
    for y in 0..height {
        for x in 0..width {
            // Create a colorful gradient pattern
            let r = ((x as f32 / width as f32) * 255.0) as u8;
            let g = ((y as f32 / height as f32) * 255.0) as u8;
            let b = (((x + y) as f32 / (width + height) as f32) * 255.0) as u8;
            let a = 255u8;
            
            data.push(r);
            data.push(g);
            data.push(b);
            data.push(a);
        }
    }
    
    data
}

/// Helper to create test texture with checkerboard pattern
fn create_test_checkerboard(width: u32, height: u32, square_size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    
    for y in 0..height {
        for x in 0..width {
            let checker_x = (x / square_size) % 2;
            let checker_y = (y / square_size) % 2;
            let is_white = (checker_x + checker_y) % 2 == 0;
            
            let color = if is_white { 255u8 } else { 64u8 };
            
            data.push(color);
            data.push(color);
            data.push(color);
            data.push(255);
        }
    }
    
    data
}

/// Helper to create test texture with radial gradient
fn create_test_radial(width: u32, height: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    let center_x = width as f32 / 2.0;
    let center_y = height as f32 / 2.0;
    let max_dist = ((center_x * center_x) + (center_y * center_y)).sqrt();
    
    for y in 0..height {
        for x in 0..width {
            let dx = x as f32 - center_x;
            let dy = y as f32 - center_y;
            let dist = (dx * dx + dy * dy).sqrt();
            let normalized = (dist / max_dist).clamp(0.0, 1.0);
            
            let r = (normalized * 255.0) as u8;
            let g = ((1.0 - normalized) * 255.0) as u8;
            let b = 128u8;
            
            data.push(r);
            data.push(g);
            data.push(b);
            data.push(255);
        }
    }
    
    data
}

#[tokio::test]
async fn test_blur_gaussian_5px() {
    let config = VisualTestConfig {
        width: 512,
        height: 512,
        tolerance: 0.01,
        golden_dir: "tests/golden_images/blur".to_string(),
    };
    
    let mut ctx = VisualTestContext::new(config).await.expect("Failed to create test context");
    
    // Create test image
    let test_data = create_test_gradient(512, 512);
    
    // Apply Gaussian blur with 5px radius
    let result_data = ctx
        .apply_blur_effect(&test_data, 512, 512, 5.0, 1.0)
        .await
        .expect("Failed to apply blur");
    
    // Compare with golden image
    let comparison = ctx
        .compare_with_golden("gaussian_blur_5px.png", &result_data, 512, 512)
        .expect("Failed to compare");
    
    assert!(
        matches!(comparison, ComparisonResult::Match),
        "Gaussian blur 5px visual test failed: {:?}",
        comparison
    );
}

#[tokio::test]
async fn test_blur_gaussian_10px() {
    let config = VisualTestConfig {
        width: 512,
        height: 512,
        tolerance: 0.01,
        golden_dir: "tests/golden_images/blur".to_string(),
    };
    
    let mut ctx = VisualTestContext::new(config).await.expect("Failed to create test context");
    
    // Create test image
    let test_data = create_test_gradient(512, 512);
    
    // Apply Gaussian blur with 10px radius
    let result_data = ctx
        .apply_blur_effect(&test_data, 512, 512, 10.0, 1.0)
        .await
        .expect("Failed to apply blur");
    
    // Compare with golden image
    let comparison = ctx
        .compare_with_golden("gaussian_blur_10px.png", &result_data, 512, 512)
        .expect("Failed to compare");
    
    assert!(
        matches!(comparison, ComparisonResult::Match),
        "Gaussian blur 10px visual test failed: {:?}",
        comparison
    );
}

#[tokio::test]
async fn test_blur_gaussian_20px() {
    let config = VisualTestConfig {
        width: 512,
        height: 512,
        tolerance: 0.01,
        golden_dir: "tests/golden_images/blur".to_string(),
    };
    
    let mut ctx = VisualTestContext::new(config).await.expect("Failed to create test context");
    
    // Create test image
    let test_data = create_test_gradient(512, 512);
    
    // Apply Gaussian blur with 20px radius
    let result_data = ctx
        .apply_blur_effect(&test_data, 512, 512, 20.0, 1.0)
        .await
        .expect("Failed to apply blur");
    
    // Compare with golden image
    let comparison = ctx
        .compare_with_golden("gaussian_blur_20px.png", &result_data, 512, 512)
        .expect("Failed to compare");
    
    assert!(
        matches!(comparison, ComparisonResult::Match),
        "Gaussian blur 20px visual test failed: {:?}",
        comparison
    );
}

#[tokio::test]
async fn test_blur_gaussian_40px() {
    let config = VisualTestConfig {
        width: 512,
        height: 512,
        tolerance: 0.01,
        golden_dir: "tests/golden_images/blur".to_string(),
    };
    
    let mut ctx = VisualTestContext::new(config).await.expect("Failed to create test context");
    
    // Create test image
    let test_data = create_test_gradient(512, 512);
    
    // Apply Gaussian blur with 40px radius
    let result_data = ctx
        .apply_blur_effect(&test_data, 512, 512, 40.0, 1.0)
        .await
        .expect("Failed to apply blur");
    
    // Compare with golden image
    let comparison = ctx
        .compare_with_golden("gaussian_blur_40px.png", &result_data, 512, 512)
        .expect("Failed to compare");
    
    assert!(
        matches!(comparison, ComparisonResult::Match),
        "Gaussian blur 40px visual test failed: {:?}",
        comparison
    );
}

#[tokio::test]
async fn test_blur_checkerboard_pattern() {
    let config = VisualTestConfig {
        width: 512,
        height: 512,
        tolerance: 0.01,
        golden_dir: "tests/golden_images/blur".to_string(),
    };
    
    let mut ctx = VisualTestContext::new(config).await.expect("Failed to create test context");
    
    // Create checkerboard test image
    let test_data = create_test_checkerboard(512, 512, 32);
    
    // Apply blur
    let result_data = ctx
        .apply_blur_effect(&test_data, 512, 512, 15.0, 1.0)
        .await
        .expect("Failed to apply blur");
    
    // Compare with golden image
    let comparison = ctx
        .compare_with_golden("blur_checkerboard_15px.png", &result_data, 512, 512)
        .expect("Failed to compare");
    
    assert!(
        matches!(comparison, ComparisonResult::Match),
        "Blur checkerboard visual test failed: {:?}",
        comparison
    );
}

#[tokio::test]
async fn test_blur_radial_gradient() {
    let config = VisualTestConfig {
        width: 512,
        height: 512,
        tolerance: 0.01,
        golden_dir: "tests/golden_images/blur".to_string(),
    };
    
    let mut ctx = VisualTestContext::new(config).await.expect("Failed to create test context");
    
    // Create radial gradient test image
    let test_data = create_test_radial(512, 512);
    
    // Apply blur
    let result_data = ctx
        .apply_blur_effect(&test_data, 512, 512, 12.0, 1.0)
        .await
        .expect("Failed to apply blur");
    
    // Compare with golden image
    let comparison = ctx
        .compare_with_golden("blur_radial_12px.png", &result_data, 512, 512)
        .expect("Failed to compare");
    
    assert!(
        matches!(comparison, ComparisonResult::Match),
        "Blur radial gradient visual test failed: {:?}",
        comparison
    );
}

#[tokio::test]
async fn test_blur_intensity_variations() {
    let config = VisualTestConfig {
        width: 512,
        height: 512,
        tolerance: 0.01,
        golden_dir: "tests/golden_images/blur".to_string(),
    };
    
    let mut ctx = VisualTestContext::new(config).await.expect("Failed to create test context");
    let test_data = create_test_gradient(512, 512);
    
    // Test different intensity levels
    for (intensity, suffix) in [(0.5, "half"), (0.75, "75pct"), (1.0, "full")] {
        let result_data = ctx
            .apply_blur_effect(&test_data, 512, 512, 10.0, intensity)
            .await
            .expect("Failed to apply blur");
        
        let filename = format!("blur_intensity_{}.png", suffix);
        let comparison = ctx
            .compare_with_golden(&filename, &result_data, 512, 512)
            .expect("Failed to compare");
        
        assert!(
            matches!(comparison, ComparisonResult::Match),
            "Blur intensity {} visual test failed: {:?}",
            suffix,
            comparison
        );
    }
}

#[tokio::test]
async fn test_blur_horizontal_pass() {
    let config = VisualTestConfig {
        width: 512,
        height: 512,
        tolerance: 0.01,
        golden_dir: "tests/golden_images/blur".to_string(),
    };
    
    let mut ctx = VisualTestContext::new(config).await.expect("Failed to create test context");
    
    // Create test image
    let test_data = create_test_gradient(512, 512);
    
    // Apply horizontal blur only
    let result_data = ctx
        .apply_blur_pass(&test_data, 512, 512, 15.0, 1.0, true)
        .await
        .expect("Failed to apply horizontal blur");
    
    // Compare with golden image
    let comparison = ctx
        .compare_with_golden("blur_horizontal_15px.png", &result_data, 512, 512)
        .expect("Failed to compare");
    
    assert!(
        matches!(comparison, ComparisonResult::Match),
        "Horizontal blur pass visual test failed: {:?}",
        comparison
    );
}

#[tokio::test]
async fn test_blur_vertical_pass() {
    let config = VisualTestConfig {
        width: 512,
        height: 512,
        tolerance: 0.01,
        golden_dir: "tests/golden_images/blur".to_string(),
    };
    
    let mut ctx = VisualTestContext::new(config).await.expect("Failed to create test context");
    
    // Create test image
    let test_data = create_test_gradient(512, 512);
    
    // Apply vertical blur only
    let result_data = ctx
        .apply_blur_pass(&test_data, 512, 512, 15.0, 1.0, false)
        .await
        .expect("Failed to apply vertical blur");
    
    // Compare with golden image
    let comparison = ctx
        .compare_with_golden("blur_vertical_15px.png", &result_data, 512, 512)
        .expect("Failed to compare");
    
    assert!(
        matches!(comparison, ComparisonResult::Match),
        "Vertical blur pass visual test failed: {:?}",
        comparison
    );
}

#[tokio::test]
async fn test_blur_dual_pass_combined() {
    let config = VisualTestConfig {
        width: 512,
        height: 512,
        tolerance: 0.01,
        golden_dir: "tests/golden_images/blur".to_string(),
    };
    
    let mut ctx = VisualTestContext::new(config).await.expect("Failed to create test context");
    
    // Create test image
    let test_data = create_test_gradient(512, 512);
    
    // Apply horizontal pass first
    let intermediate_data = ctx
        .apply_blur_pass(&test_data, 512, 512, 15.0, 1.0, true)
        .await
        .expect("Failed to apply horizontal blur");
    
    // Then apply vertical pass
    let result_data = ctx
        .apply_blur_pass(&intermediate_data, 512, 512, 15.0, 1.0, false)
        .await
        .expect("Failed to apply vertical blur");
    
    // Compare with golden image
    let comparison = ctx
        .compare_with_golden("blur_dual_pass_15px.png", &result_data, 512, 512)
        .expect("Failed to compare");
    
    assert!(
        matches!(comparison, ComparisonResult::Match),
        "Dual-pass blur visual test failed: {:?}",
        comparison
    );
}
