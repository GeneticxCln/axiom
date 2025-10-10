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

#[tokio::test]
async fn generate_shadow_drop_baseline() {
    // GPU context
    let (device, queue) = create_test_gpu_context().await;

    // Compile shaders and init shadow renderer
    let mut shader_manager = ShaderManager::new(device.clone());
    shader_manager
        .compile_all_shaders()
        .expect("Failed to compile shaders");

    let shadow_params = ShadowParams {
        enabled: true,
        size: 20.0,
        blur_radius: 15.0,
        opacity: 0.6,
        offset: (0.0, 6.0),
        color: [0.0, 0.0, 0.0, 1.0],
    };

    let mut shadow_renderer = ShadowRenderer::new(
        device.clone(),
        queue.clone(),
        Arc::new(shader_manager),
        shadow_params.clone(),
        ShadowQuality::Medium,
    )
    .expect("Failed to create shadow renderer");

    // Visual test runner
    let config = VisualTestConfig {
        test_name: "shadow/drop_shadow_basic".to_string(),
        width: 800,
        height: 600,
        tolerance: 0.01,
        save_diffs: true,
        ..Default::default()
    };

    let runner = VisualTestRunner::new(device.clone(), queue.clone(), config);

    // Render function will record commands into the provided view
    let result = runner
        .run_test(|view| {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Golden Shadow Encoder"),
            });

            // Render a centered window shadow sized 400x300
            let window_pos = Vector2::new(200.0, 150.0);
            let window_size = Vector2::new(400.0, 300.0);

            shadow_renderer
                .render_drop_shadow(&mut encoder, view, window_pos, window_size, &shadow_params)
                .expect("Shadow render failed");

            queue.submit(Some(encoder.finish()));
            Ok(())
        })
        .await
        .expect("Visual test run failed");

    // First run creates baseline and passes; subsequent runs compare
    assert!(result.passed, "Golden comparison failed: diff={}", result.difference);
}
