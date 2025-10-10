//! Visual golden image tests that generate or compare baseline snapshots

use axiom::effects::shaders::ShaderManager;
use axiom::effects::shadow::{ShadowQuality, ShadowRenderer};
use axiom::effects::ShadowParams;
use axiom::visual_tests::{VisualTestConfig, VisualTestRunner};
use cgmath::Vector2;
use std::sync::Arc;

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
                label: Some("Golden Test Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        )
        .await
        .expect("Failed to create device");

    (Arc::new(device), Arc::new(queue))
}

// Helper to render shadow with given parameters
async fn render_shadow_golden(
    test_name: &str,
    shadow_params: ShadowParams,
    quality: ShadowQuality,
    window_pos: Vector2<f32>,
    window_size: Vector2<f32>,
    width: u32,
    height: u32,
) {
    let (device, queue) = create_test_gpu_context().await;

    let mut shader_manager = ShaderManager::new(device.clone());
    shader_manager
        .compile_all_shaders()
        .expect("Failed to compile shaders");

    let mut shadow_renderer = ShadowRenderer::new(
        device.clone(),
        queue.clone(),
        Arc::new(shader_manager),
        shadow_params.clone(),
        quality,
    )
    .expect("Failed to create shadow renderer");

    let config = VisualTestConfig {
        test_name: test_name.to_string(),
        width,
        height,
        tolerance: 0.01,
        save_diffs: true,
        ..Default::default()
    };

    let runner = VisualTestRunner::new(device.clone(), queue.clone(), config);

    let result = runner
        .run_test(|view| {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Golden Shadow Encoder"),
            });

            shadow_renderer
                .render_drop_shadow(&mut encoder, view, window_pos, window_size, &shadow_params)
                .expect("Shadow render failed");

            queue.submit(Some(encoder.finish()));
            Ok(())
        })
        .await
        .expect("Visual test run failed");

    assert!(result.passed, "Golden comparison failed: diff={}", result.difference);
}

#[tokio::test]
async fn generate_shadow_drop_baseline() {
    render_shadow_golden(
        "shadow/drop_shadow_basic",
        ShadowParams {
            enabled: true,
            size: 20.0,
            blur_radius: 15.0,
            opacity: 0.6,
            offset: (0.0, 6.0),
            color: [0.0, 0.0, 0.0, 1.0],
        },
        ShadowQuality::Medium,
        Vector2::new(200.0, 150.0),
        Vector2::new(400.0, 300.0),
        800,
        600,
    )
    .await;
}

#[tokio::test]
async fn generate_shadow_large_blur() {
    render_shadow_golden(
        "shadow/drop_shadow_large_blur",
        ShadowParams {
            enabled: true,
            size: 30.0,
            blur_radius: 30.0,
            opacity: 0.7,
            offset: (0.0, 8.0),
            color: [0.0, 0.0, 0.0, 1.0],
        },
        ShadowQuality::High,
        Vector2::new(200.0, 150.0),
        Vector2::new(400.0, 300.0),
        800,
        600,
    )
    .await;
}

#[tokio::test]
async fn generate_shadow_small_sharp() {
    render_shadow_golden(
        "shadow/drop_shadow_small_sharp",
        ShadowParams {
            enabled: true,
            size: 10.0,
            blur_radius: 5.0,
            opacity: 0.8,
            offset: (2.0, 2.0),
            color: [0.0, 0.0, 0.0, 1.0],
        },
        ShadowQuality::Low,
        Vector2::new(300.0, 200.0),
        Vector2::new(200.0, 200.0),
        800,
        600,
    )
    .await;
}

#[tokio::test]
async fn generate_shadow_colored() {
    render_shadow_golden(
        "shadow/drop_shadow_colored",
        ShadowParams {
            enabled: true,
            size: 20.0,
            blur_radius: 15.0,
            opacity: 0.5,
            offset: (0.0, 6.0),
            color: [0.2, 0.2, 0.8, 1.0], // Blue shadow
        },
        ShadowQuality::Medium,
        Vector2::new(200.0, 150.0),
        Vector2::new(400.0, 300.0),
        800,
        600,
    )
    .await;
}

#[tokio::test]
async fn generate_shadow_offset_diagonal() {
    render_shadow_golden(
        "shadow/drop_shadow_offset_diagonal",
        ShadowParams {
            enabled: true,
            size: 20.0,
            blur_radius: 12.0,
            opacity: 0.6,
            offset: (10.0, 10.0),
            color: [0.0, 0.0, 0.0, 1.0],
        },
        ShadowQuality::Medium,
        Vector2::new(200.0, 150.0),
        Vector2::new(400.0, 300.0),
        800,
        600,
    )
    .await;
}
