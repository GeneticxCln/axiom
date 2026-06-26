//! Performance benchmarks for Axiom compositor
//!
//! These benchmarks test performance-critical operations to prevent regressions
//! and guide optimization efforts.

use axiom::{
    config::{EffectsConfig, WorkspaceConfig},
    effects::EffectsEngine,
    workspace::ScrollableWorkspaces,
};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};

/// Benchmark workspace scrolling operations
fn bench_workspace_scrolling(c: &mut Criterion) {
    let mut group = c.benchmark_group("workspace_scrolling");

    // Test with different numbers of windows
    for window_count in [10, 50, 100, 500].iter() {
        group.bench_with_input(
            format!("scroll_with_{}_windows", window_count),
            window_count,
            |b, &window_count| {
                b.iter_batched(
                    || {
                        let config = WorkspaceConfig::default();
                        let mut workspaces = ScrollableWorkspaces::new(&config);

                        // Add windows
                        for i in 1..=window_count {
                            workspaces.add_window(i);
                        }
                        workspaces
                    },
                    |mut workspaces| {
                        // Benchmark scrolling operations
                        for _ in 0..10 {
                            workspaces.scroll_right();
                            workspaces.scroll_left();
                        }
                        black_box(());
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

/// Benchmark window layout calculations
fn bench_window_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("window_layout");

    for window_count in [10, 50, 100, 500].iter() {
        group.bench_with_input(
            format!("layout_calculation_{}_windows", window_count),
            window_count,
            |b, &window_count| {
                b.iter_batched(
                    || {
                        let config = WorkspaceConfig::default();
                        let mut workspaces = ScrollableWorkspaces::new(&config);

                        // Add windows
                        for i in 1..=window_count {
                            workspaces.add_window(i);
                        }
                        workspaces
                    },
                    |workspaces| {
                        // Benchmark layout calculation
                        black_box(workspaces.calculate_workspace_layouts());
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

/// Benchmark effects engine operations
fn bench_effects_engine(c: &mut Criterion) {
    let mut group = c.benchmark_group("effects_engine");

    // Benchmark animation updates
    group.bench_function("animation_updates", |b| {
        b.iter_batched(
            || {
                let config = EffectsConfig::default();
                let mut effects = EffectsEngine::new(&config).unwrap();

                // Add multiple animations
                for i in 1..=50 {
                    effects.animate_window_move(
                        i,
                        (i as f32 * 10.0, i as f32 * 10.0),
                        ((i + 50) as f32 * 10.0, (i + 50) as f32 * 10.0),
                    );
                }
                effects
            },
            |mut effects| {
                // Benchmark one update cycle
                black_box(effects.update().ok());
                black_box(());
            },
            BatchSize::SmallInput,
        );
    });

    // Benchmark blur effect processing
    group.bench_function("blur_processing", |b| {
        b.iter_batched(
            || {
                let config = EffectsConfig::default();
                let mut effects = EffectsEngine::new(&config).unwrap();

                // Enable blur for multiple windows
                for i in 1..=20 {
                    effects.set_window_blur(i, 10.0);
                }
                effects
            },
            |mut effects| {
                black_box(effects.update().ok());
                black_box(());
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

/// Benchmark configuration parsing and validation
fn bench_configuration(c: &mut Criterion) {
    let mut group = c.benchmark_group("configuration");

    group.bench_function("default_config_creation", |b| {
        b.iter(|| {
            use axiom::config::AxiomConfig;
            black_box(AxiomConfig::default());
        });
    });

    group.bench_function("toml_serialization", |b| {
        use axiom::config::AxiomConfig;
        let config = AxiomConfig::default();

        b.iter(|| {
            black_box(toml::to_string(&config).unwrap());
        });
    });

    group.bench_function("toml_deserialization", |b| {
        use axiom::config::AxiomConfig;
        let config = AxiomConfig::default();
        let toml_str = toml::to_string(&config).unwrap();

        b.iter(|| {
            black_box(toml::from_str::<AxiomConfig>(&toml_str).unwrap());
        });
    });

    group.finish();
}

/// Benchmark memory allocation patterns
fn bench_memory_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_operations");

    group.bench_function("window_creation_destruction", |b| {
        b.iter(|| {
            let config = WorkspaceConfig::default();
            let mut workspaces = ScrollableWorkspaces::new(&config);

            // Create many windows
            let mut window_ids = Vec::new();
            for i in 1..=100 {
                workspaces.add_window(i);
                window_ids.push(i);
            }

            // Remove half of them
            for &id in window_ids.iter().take(50) {
                workspaces.remove_window(id);
            }

            black_box(workspaces);
        });
    });

    group.finish();
}

/// Benchmark concurrent operations
fn bench_concurrency(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrency");

    group.bench_function("concurrent_window_operations", |b| {
        use std::sync::Arc;
        use std::sync::Mutex;

        b.iter(|| {
            let config = WorkspaceConfig::default();
            let workspaces = Arc::new(Mutex::new(ScrollableWorkspaces::new(&config)));

            // Simulate concurrent operations
            let handles: Vec<_> = (0..4)
                .map(|thread_id| {
                    let workspaces_clone = Arc::clone(&workspaces);
                    std::thread::spawn(move || {
                        let mut ws = workspaces_clone.lock().unwrap();

                        // Each thread adds and removes windows
                        for i in 0..25 {
                            let window_id = thread_id * 25 + i + 1000;
                            ws.add_window(window_id as u64);
                        }

                        for i in 0..10 {
                            let window_id = thread_id * 25 + i + 1000;
                            ws.remove_window(window_id as u64);
                        }
                    })
                })
                .collect();

            // Wait for all threads to complete
            for handle in handles {
                handle.join().unwrap();
            }

            black_box(workspaces);
        });
    });

    group.finish();
}

/// Benchmark input processing latency
fn bench_input_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("input_processing");

    group.bench_function("scroll_event_processing", |b| {
        use axiom::config::{BindingsConfig, InputConfig};
        use axiom::input::{InputEvent, InputManager};

        let input_config = InputConfig::default();
        let bindings_config = BindingsConfig::default();
        let mut input_manager = InputManager::new(&input_config, &bindings_config);

        b.iter(|| {
            let event = InputEvent::Scroll {
                x: 100.0,
                y: 100.0,
                delta_x: 50.0,
                delta_y: 0.0,
            };

            black_box(input_manager.process_input_event(event));
        });
    });

    group.finish();
}

/// Build a 64×64 RGBA benchmark texture with a simple gradient pattern.
fn make_bench_texture_rgba() -> Vec<u8> {
    let w = 64usize;
    let h = 64usize;
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            pixels.push((x * 4) as u8);
            pixels.push((y * 4) as u8);
            pixels.push(128u8);
            pixels.push(255u8);
        }
    }
    pixels
}

/// Benchmark `render_to_headless_target` with cached projection buffer.
///
/// Two variants:
/// - `cold_path`: fresh renderer each iteration → always cache miss.
/// - `hot_path`: same renderer across iterations → first call populates
///   the projection cache; subsequent calls in the same iteration reuse
///   it, measuring the per-frame speedup from the caching optimisation.
fn bench_headless_render(c: &mut Criterion) {
    use axiom::renderer::AxiomRenderer;

    let mut group = c.benchmark_group("headless_render");

    // ── Cold path: fresh renderer → cache always empty ────────────
    group.bench_function("composite_128x128_cold", |b| {
        b.iter_batched(
            || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let mut renderer = rt.block_on(AxiomRenderer::new_headless()).unwrap();
                let tex = make_bench_texture_rgba();
                renderer.add_window(1, (0.0, 0.0), (64.0, 64.0));
                renderer.update_window_texture(1, 64, 64, &tex);
                renderer
            },
            |mut renderer| {
                // Cache miss — allocates projection buffer
                black_box(
                    renderer
                        .render_to_headless_target(128, 128)
                        .unwrap(),
                );
            },
            BatchSize::SmallInput,
        );
    });

    // ── Hot path: call twice — first populates cache, second reuses │
    group.bench_function("composite_128x128_hot", |b| {
        b.iter_batched(
            || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let mut renderer = rt.block_on(AxiomRenderer::new_headless()).unwrap();
                let tex = make_bench_texture_rgba();
                renderer.add_window(1, (0.0, 0.0), (64.0, 64.0));
                renderer.update_window_texture(1, 64, 64, &tex);
                // Warm: populate the projection cache (untimed)
                renderer.render_to_headless_target(128, 128).unwrap();
                renderer
            },
            |mut renderer| {
                // Cache hit — projection buffer reused from warm call
                black_box(
                    renderer
                        .render_to_headless_target(128, 128)
                        .unwrap(),
                );
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();

    // ── Throughput: N composites with warm cache (separate group) ─
    let mut tp_group = c.benchmark_group("headless_render_throughput");
    const BATCH: u32 = 100;
    tp_group.throughput(criterion::Throughput::Elements(BATCH as u64));
    tp_group.bench_function("composite_128x128", |b| {
        b.iter_batched(
            || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let mut renderer = rt.block_on(AxiomRenderer::new_headless()).unwrap();
                let tex = make_bench_texture_rgba();
                renderer.add_window(1, (0.0, 0.0), (64.0, 64.0));
                renderer.update_window_texture(1, 64, 64, &tex);
                // Warm: populate the projection cache (untimed)
                renderer.render_to_headless_target(128, 128).unwrap();
                renderer
            },
            |mut renderer| {
                // Timed: BATCH cache-hit composites (projection buffer reused,
                // window resources cached after first call)
                for _ in 0..BATCH {
                    black_box(
                        renderer
                            .render_to_headless_target(128, 128)
                            .unwrap(),
                    );
                }
            },
            BatchSize::SmallInput,
        );
    });
    tp_group.finish();
}

/// Benchmark frame timing simulation
fn bench_frame_timing(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_timing");

    group.bench_function("full_frame_simulation", |b| {
        use axiom::config::{EffectsConfig, WorkspaceConfig};

        b.iter_batched(
            || {
                // Setup simulated compositor state
                let workspace_config = WorkspaceConfig::default();
                let effects_config = EffectsConfig::default();

                let mut workspaces = ScrollableWorkspaces::new(&workspace_config);
                let mut effects = EffectsEngine::new(&effects_config).unwrap();

                // Add some windows and animations to simulate real usage
                for i in 1..=20 {
                    workspaces.add_window(i);
                    if i % 3 == 0 {
                        effects.animate_window_move(
                            i,
                            (i as f32 * 10.0, i as f32 * 10.0),
                            ((i + 10) as f32 * 10.0, (i + 10) as f32 * 10.0),
                        );
                    }
                }

                (workspaces, effects)
            },
            |(mut workspaces, mut effects)| {
                // Simulate a full frame update
                workspaces.update_animations();
                effects.update().unwrap();

                let layouts = workspaces.calculate_workspace_layouts();
                black_box(layouts);

                let stats = effects.get_performance_stats();
                black_box(stats);
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_workspace_scrolling,
    bench_window_layout,
    bench_effects_engine,
    bench_configuration,
    bench_memory_operations,
    bench_concurrency,
    bench_input_processing,
    bench_frame_timing,
    bench_headless_render
);

criterion_main!(benches);
