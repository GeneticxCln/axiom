# Visual Testing Guide for Axiom

## Overview

Axiom uses **golden image testing** to ensure visual effects render correctly and consistently across changes. This guide covers how to write, run, and maintain visual tests.

## What Are Golden Image Tests?

Golden image tests (also called snapshot tests or screenshot tests) work by:

1. Rendering a scene to an off-screen texture
2. Capturing the pixel data from the GPU
3. Comparing it to a reference "golden" image
4. Reporting differences if the rendering has changed

This ensures that visual effects like shadows, blur, animations, and rounded corners remain pixel-perfect (or within acceptable tolerance) across code changes.

## Test Infrastructure

### Core Components

- **`src/visual_tests.rs`**: Framework for capturing and comparing rendered frames
- **`tests/visual_effects_tests.rs`**: Integration tests for effects rendering
- **`tests/golden_images/`**: Directory storing reference images
- **`tests/golden_images/diffs/`**: Auto-generated diff images when tests fail

### Key Types

```rust
// Configuration for a visual test
VisualTestConfig {
    test_name: String,      // Unique name for the test
    width: u32,             // Render target width
    height: u32,            // Render target height
    tolerance: f32,         // Allowed difference (0.0-1.0)
    save_diffs: bool,       // Save diff images on failure
    golden_dir: PathBuf,    // Base directory for golden images
}

// Result of comparing captured image to golden
ComparisonResult {
    passed: bool,           // Did the test pass?
    difference: f32,        // Average pixel difference
    different_pixels: usize,// Number of pixels that differed
    total_pixels: usize,    // Total pixels compared
    diff_image_path: Option<PathBuf>, // Path to diff visualization
}
```

## Writing Visual Tests

### Basic Shadow Test

```rust
use axiom::visual_tests::{VisualTestConfig, VisualTestRunner};
use axiom::effects::shadow::ShadowRenderer;

#[tokio::test]
async fn test_shadow_rendering() {
    // Create GPU context
    let (device, queue) = create_test_gpu_context().await;
    
    // Configure test
    let config = VisualTestConfig {
        test_name: "drop_shadow_basic".to_string(),
        width: 800,
        height: 600,
        tolerance: 0.01, // 1% tolerance
        ..Default::default()
    };
    
    let runner = VisualTestRunner::new(device.clone(), queue.clone(), config);
    
    // Run test with render function
    let result = runner.run_test(|view| {
        // Your rendering code here
        render_shadow_to_view(view, shadow_params)?;
        Ok(())
    }).await?;
    
    assert!(result.passed, "Shadow rendering changed: {:.2}% difference", 
            result.difference * 100.0);
}
```

### Test With Multiple Effects

```rust
#[tokio::test]
async fn test_window_with_shadow_and_blur() {
    let config = VisualTestConfig {
        test_name: "window_full_effects".to_string(),
        width: 1920,
        height: 1080,
        tolerance: 0.02, // 2% tolerance for complex scene
        ..Default::default()
    };
    
    let runner = VisualTestRunner::new(device, queue, config);
    
    let result = runner.run_test(|view| {
        // Render window with full effects stack
        render_window(view, &window_data)?;
        render_shadow(view, &shadow_params)?;
        apply_blur(view, &blur_params)?;
        Ok(())
    }).await?;
    
    assert!(result.passed);
}
```

## Running Tests

### Run All Visual Tests

```bash
# Run all integration tests including visual tests
cargo test --test visual_effects_tests

# Run with logging to see details
RUST_LOG=debug cargo test --test visual_effects_tests
```

### Run Specific Test

```bash
# Run a single visual test
cargo test --test visual_effects_tests test_shadow_render_pipeline

# Run tests matching a pattern
cargo test --test visual_effects_tests shadow
```

### First Run (Generate Baselines)

When you first run a visual test, there's no golden image to compare against. The test will:

1. Render the scene normally
2. Save the result as the golden image
3. Pass the test (since there's nothing to compare yet)
4. Log: `📸 No golden image found for 'test_name', saving baseline`

**Important**: Review the generated golden images to ensure they're correct before committing!

```bash
# Check generated golden images
ls -la tests/golden_images/
eog tests/golden_images/*.png  # Or use your image viewer
```

## Understanding Test Results

### Test Pass

```
✅ Visual test 'drop_shadow_basic' passed
   Difference: 0.03% (within 1.00% tolerance)
   Different pixels: 12 / 480000
```

### Test Failure

```
❌ Visual test 'drop_shadow_basic' failed
   Difference: 2.35% (exceeds 1.00% tolerance)
   Different pixels: 11280 / 480000
   Diff image saved to: tests/golden_images/diffs/drop_shadow_basic_diff.png
```

### Reading Diff Images

Diff images highlight differences in **red**:
- **Red pixels**: Changed from golden image
- **Original colors**: Unchanged pixels

This makes it easy to spot exactly what changed in the rendering.

## Updating Golden Images

### When to Update

Update golden images when:
- ✅ You intentionally improved visual effects
- ✅ You fixed a rendering bug
- ✅ You changed shader code
- ❌ NOT when tests fail unexpectedly (investigate first!)

### How to Update

#### Option 1: Manually Delete and Re-run

```bash
# Delete the golden image
rm tests/golden_images/drop_shadow_basic.png

# Re-run the test to regenerate
cargo test --test visual_effects_tests test_shadow_render_pipeline
```

#### Option 2: Use Update Helper (if implemented)

```rust
#[tokio::test]
#[ignore] // Mark as ignored so it doesn't run by default
async fn update_shadow_golden() {
    let config = VisualTestConfig {
        test_name: "drop_shadow_basic".to_string(),
        ..Default::default()
    };
    
    let runner = VisualTestRunner::new(device, queue, config);
    
    runner.update_golden(|view| {
        render_shadow_to_view(view, shadow_params)?;
        Ok(())
    }).await?;
}
```

Run with: `cargo test --test visual_effects_tests update_shadow_golden --ignored`

## Best Practices

### Test Naming

Use descriptive, hierarchical names:

```rust
// Good names
"shadow/drop_shadow_basic"
"shadow/drop_shadow_large_blur"
"shadow/shadow_batch_multiple_windows"
"blur/gaussian_blur_radius_10"
"blur/gaussian_blur_radius_50"
"rounded_corners/radius_8px"
"rounded_corners/radius_16px_with_border"

// Avoid
"test1"
"shadow_test"
"my_test"
```

### Tolerance Settings

Choose appropriate tolerance based on complexity:

```rust
// Pixel-perfect tests (simple geometry, solid colors)
tolerance: 0.001  // 0.1%

// Standard tests (typical effects)
tolerance: 0.01   // 1%

// Complex scenes (multiple effects, gradients, blur)
tolerance: 0.02   // 2%

// Very complex or animation frames
tolerance: 0.05   // 5%
```

### Test Resolution

Use resolutions that match real usage:

```rust
// Fast tests for CI
(width: 640, height: 480)    // VGA

// Standard window sizes
(width: 800, height: 600)    // SVGA
(width: 1280, height: 720)   // 720p

// High resolution for detail
(width: 1920, height: 1080)  // 1080p
(width: 3840, height: 2160)  // 4K (use sparingly - slow!)
```

### Organizing Tests

Group related tests:

```rust
mod shadow_tests {
    #[tokio::test]
    async fn test_basic_drop_shadow() { /* ... */ }
    
    #[tokio::test]
    async fn test_shadow_with_blur() { /* ... */ }
    
    #[tokio::test]
    async fn test_shadow_batch_rendering() { /* ... */ }
}

mod blur_tests {
    // ...
}

mod animation_tests {
    // ...
}
```

## Continuous Integration

### CI Configuration

Add to `.github/workflows/test.yml` or similar:

```yaml
- name: Run visual tests
  run: |
    cargo test --test visual_effects_tests
  env:
    RUST_LOG: info
    # Ensure GPU backend is available (use software rasterization if needed)
    WGPU_BACKEND: vulkan
```

### Handling CI Failures

Visual tests can be sensitive to:
- **Different GPUs**: Slight rounding differences
- **Driver versions**: Minor implementation variations
- **OS differences**: Platform-specific rendering

Solutions:
1. Increase tolerance slightly for CI (e.g., 0.01 → 0.015)
2. Use software rasterization for consistent results
3. Generate platform-specific golden images
4. Skip visual tests in CI (not recommended)

## Troubleshooting

### Test Passes Locally but Fails in CI

**Cause**: Different GPU or driver behavior

**Solution**: 
```rust
// Use slightly higher tolerance for CI
let tolerance = if cfg!(ci) { 0.015 } else { 0.01 };
```

### Diff Image Shows No Visible Difference

**Cause**: Sub-pixel differences that average out

**Solution**: Check the difference percentage - it might be just barely over threshold. Consider increasing tolerance slightly.

### Test is Flaky (Sometimes Passes/Fails)

**Cause**: 
- Non-deterministic rendering
- Animation timing issues
- GPU state not fully reset

**Solution**:
- Add explicit synchronization (device.poll)
- Reset all GPU state between tests
- Ensure animations use fixed time steps

### Golden Image Looks Wrong

**Cause**: The baseline was captured incorrectly

**Solution**:
1. Delete the golden image
2. Fix the rendering code
3. Re-run test to generate new baseline
4. Verify the new image looks correct

## Advanced Topics

### Testing Animations

For animated effects, test key frames:

```rust
#[tokio::test]
async fn test_window_open_animation() {
    // Test frame 0 (start)
    test_animation_frame("window_open_frame_0", 0.0).await?;
    
    // Test frame at 50% (mid-animation)
    test_animation_frame("window_open_frame_50", 0.5).await?;
    
    // Test frame at 100% (end)
    test_animation_frame("window_open_frame_100", 1.0).await?;
}
```

### Performance Testing

Combine visual tests with performance checks:

```rust
let start = std::time::Instant::now();
let result = runner.run_test(|view| {
    render_complex_scene(view)?;
    Ok(())
}).await?;
let duration = start.elapsed();

assert!(result.passed);
assert!(duration.as_millis() < 16, "Frame took too long: {}ms", duration.as_millis());
```

### Fuzzy Comparison

For content that's inherently variable (noise, random patterns):

```rust
let config = VisualTestConfig {
    tolerance: 0.10,  // 10% tolerance
    // ... other fields
};
```

## Summary

Visual testing ensures Axiom's effects remain beautiful and consistent. Follow these guidelines:

✅ **DO**:
- Write tests for all visual effects
- Use descriptive test names
- Set appropriate tolerances
- Review golden images before committing
- Update golden images when you improve rendering

❌ **DON'T**:
- Blindly update golden images when tests fail
- Use unnecessarily high resolutions
- Set tolerance too loose (defeats the purpose)
- Forget to commit golden images with your PR

## Questions?

- Check `src/visual_tests.rs` for API documentation
- See `tests/visual_effects_tests.rs` for examples
- Ask in #testing or #graphics channels

---

**Last updated**: 2025-10-10  
**Maintainer**: Axiom Graphics Team
