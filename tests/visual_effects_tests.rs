//! Integration tests for visual effects rendering
//!
//! These tests verify that the effects engine correctly renders:
//! - Drop shadows
//! - Blur effects
//! - Rounded corners
//! - Animations

use axiom::effects::shaders::ShaderManager;
use axiom::effects::shadow::{ShadowQuality, ShadowRenderer};
use axiom::effects::ShadowParams;
use cgmath::Vector2;
use std::sync::Arc;

/// Helper to create a headless GPU context for testing
async fn create_test_gpu_context() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        ..Default::default()
    });

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
        })
        .await
        .expect("Failed to get adapter");

    let (device, queue) = adapter
        .request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Test Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        )
        .await
        .expect("Failed to create device");

    (Arc::new(device), Arc::new(queue))
}

#[tokio::test]
async fn test_shadow_shader_compilation() {
    // Test that shadow shaders compile successfully
    let (device, _queue) = create_test_gpu_context().await;

    let mut shader_manager = ShaderManager::new(device.clone());
    let result = shader_manager.compile_all_shaders();

    assert!(
        result.is_ok(),
        "Shadow shaders should compile: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_shadow_renderer_initialization() {
    // Test that shadow renderer initializes correctly
    let (device, queue) = create_test_gpu_context().await;

    let mut shader_manager = ShaderManager::new(device.clone());
    shader_manager
        .compile_all_shaders()
        .expect("Failed to compile shaders");

    let shadow_params = ShadowParams::default();

    let renderer = ShadowRenderer::new(
        device.clone(),
        queue.clone(),
        Arc::new(shader_manager),
        shadow_params,
        ShadowQuality::Medium,
    );

    assert!(renderer.is_ok(), "Shadow renderer should initialize");
}

#[tokio::test]
async fn test_shadow_render_pipeline() {
    // Test that shadow rendering pipeline executes without errors
    let (device, queue) = create_test_gpu_context().await;

    let mut shader_manager = ShaderManager::new(device.clone());
    shader_manager
        .compile_all_shaders()
        .expect("Failed to compile shaders");

    let shadow_params = ShadowParams {
        enabled: true,
        size: 20.0,
        blur_radius: 15.0,
        opacity: 0.6,
        offset: (0.0, 2.0),
        color: [0.0, 0.0, 0.0, 1.0],
    };

    let mut renderer = ShadowRenderer::new(
        device.clone(),
        queue.clone(),
        Arc::new(shader_manager),
        shadow_params.clone(),
        ShadowQuality::Medium,
    )
    .expect("Failed to create shadow renderer");

    // Create test render target
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Test Shadow Texture"),
        size: wgpu::Extent3d {
            width: 800,
            height: 600,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Test Shadow Encoder"),
    });

    // Render shadow
    let result = renderer.render_drop_shadow(
        &mut encoder,
        &view,
        Vector2::new(100.0, 100.0),
        Vector2::new(400.0, 300.0),
        &shadow_params,
    );

    assert!(
        result.is_ok(),
        "Shadow rendering should succeed: {:?}",
        result.err()
    );

    queue.submit(Some(encoder.finish()));
}

#[tokio::test]
async fn test_shadow_quality_levels() {
    // Test that different quality levels work
    let (device, queue) = create_test_gpu_context().await;

    let mut shader_manager = ShaderManager::new(device.clone());
    shader_manager
        .compile_all_shaders()
        .expect("Failed to compile shaders");

    let shadow_params = ShadowParams::default();

    // Test each quality level
    let qualities = [
        ShadowQuality::Low,
        ShadowQuality::Medium,
        ShadowQuality::High,
        ShadowQuality::Ultra,
    ];
    
    for quality in qualities {
        // Create fresh shader manager for each test
        let mut test_shader_manager = ShaderManager::new(device.clone());
        test_shader_manager
            .compile_all_shaders()
            .expect("Failed to compile shaders");
        
        let renderer = ShadowRenderer::new(
            device.clone(),
            queue.clone(),
            Arc::new(test_shader_manager),
            shadow_params.clone(),
            quality,
        );

        assert!(
            renderer.is_ok(),
            "Shadow renderer should work with {:?} quality",
            quality
        );
    }
}

#[tokio::test]
async fn test_shadow_batch_rendering() {
    // Test batch shadow rendering
    let (device, queue) = create_test_gpu_context().await;

    let mut shader_manager = ShaderManager::new(device.clone());
    shader_manager
        .compile_all_shaders()
        .expect("Failed to compile shaders");

    let shadow_params = ShadowParams::default();

    let mut renderer = ShadowRenderer::new(
        device.clone(),
        queue.clone(),
        Arc::new(shader_manager),
        shadow_params.clone(),
        ShadowQuality::Medium,
    )
    .expect("Failed to create shadow renderer");

    // Create test render target
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Test Batch Shadow Texture"),
        size: wgpu::Extent3d {
            width: 1920,
            height: 1080,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Test Batch Shadow Encoder"),
    });

    // Create batch data for multiple shadows
    let shadow_data = vec![
        (
            Vector2::new(100.0, 100.0),
            Vector2::new(400.0, 300.0),
            shadow_params.clone(),
        ),
        (
            Vector2::new(600.0, 200.0),
            Vector2::new(500.0, 400.0),
            shadow_params.clone(),
        ),
        (
            Vector2::new(200.0, 700.0),
            Vector2::new(300.0, 200.0),
            shadow_params.clone(),
        ),
    ];

    // Render batch
    let result = renderer.render_shadow_batch(&mut encoder, &view, &shadow_data);

    assert!(
        result.is_ok(),
        "Batch shadow rendering should succeed: {:?}",
        result.err()
    );

    queue.submit(Some(encoder.finish()));
}

#[tokio::test]
async fn test_shadow_performance_optimization() {
    // Test that performance optimization adjusts quality appropriately
    let (device, queue) = create_test_gpu_context().await;

    let mut shader_manager = ShaderManager::new(device.clone());
    shader_manager
        .compile_all_shaders()
        .expect("Failed to compile shaders");

    let shadow_params = ShadowParams::default();

    let mut renderer = ShadowRenderer::new(
        device.clone(),
        queue.clone(),
        Arc::new(shader_manager),
        shadow_params,
        ShadowQuality::Ultra,
    )
    .expect("Failed to create shadow renderer");

    // Simulate poor performance
    let poor_frame_time = std::time::Duration::from_millis(50); // 20 FPS
    let target_frame_time = std::time::Duration::from_millis(16); // 60 FPS

    renderer.optimize_for_performance(poor_frame_time, target_frame_time);

    let (_, quality) = renderer.get_performance_stats();

    // Quality should have been reduced from Ultra
    assert!(
        !matches!(quality, ShadowQuality::Ultra),
        "Quality should be reduced due to poor performance"
    );
}

#[tokio::test]
async fn test_dynamic_shadow_rendering() {
    // Test dynamic shadow with light position
    let (device, queue) = create_test_gpu_context().await;

    let mut shader_manager = ShaderManager::new(device.clone());
    shader_manager
        .compile_all_shaders()
        .expect("Failed to compile shaders");

    let shadow_params = ShadowParams::default();

    let mut renderer = ShadowRenderer::new(
        device.clone(),
        queue.clone(),
        Arc::new(shader_manager),
        shadow_params.clone(),
        ShadowQuality::Medium,
    )
    .expect("Failed to create shadow renderer");

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Test Dynamic Shadow Texture"),
        size: wgpu::Extent3d {
            width: 800,
            height: 600,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Test Dynamic Shadow Encoder"),
    });

    // Render dynamic shadow with light position
    let result = renderer.render_dynamic_shadow(
        &mut encoder,
        &view,
        Vector2::new(400.0, 300.0),
        Vector2::new(200.0, 150.0),
        cgmath::Vector3::new(600.0, 200.0, 150.0), // Light position
        &shadow_params,
    );

    assert!(
        result.is_ok(),
        "Dynamic shadow rendering should succeed: {:?}",
        result.err()
    );

    queue.submit(Some(encoder.finish()));
}
