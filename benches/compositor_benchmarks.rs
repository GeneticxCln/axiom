//! Performance benchmarks for Axiom compositor
//!
//! These benchmarks test performance-critical operations to prevent regressions
//! and guide optimization efforts.

use axiom::{config::WorkspaceConfig, window::Rectangle, workspace::ScrollableWorkspaces};
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use lru::LruCache;
use std::num::NonZeroUsize;

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

// ──────────────────────────────────────────────
// Render Path Benchmarks
// ──────────────────────────────────────────────

/// Benchmark the collection and preparation of render elements.
///
/// Simulates the scene-prep work that `render_scene_into` performs:
/// calculating workspace layouts via `calculate_workspace_layouts()` and
/// building the render item list from the returned layout rectangles.
fn bench_render_element_collection(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_path/element_collection");

    for window_count in [10usize, 50usize].iter() {
        group.bench_with_input(
            format!("collect_{}_windows", window_count),
            window_count,
            |b, &window_count| {
                b.iter_batched(
                    || {
                        let config = WorkspaceConfig::default();
                        let mut workspaces = ScrollableWorkspaces::new(&config);
                        for i in 1..=window_count {
                            workspaces.add_window(i as u64);
                        }
                        workspaces
                    },
                    |workspaces| {
                        // Step 1: calculate workspace layouts (prepare_render_scene's work)
                        let layouts = workspaces.calculate_workspace_layouts();

                        // Step 2: build render item list (the early Vec construction in render_scene_into)
                        let items: Vec<(u64, Rectangle)> = layouts
                            .iter()
                            .map(|(id, rect)| (*id, rect.clone()))
                            .collect();

                        black_box((layouts, items));
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

/// Benchmark damage region bounding-box merge.
///
/// Simulates the damage merge performed at the end of `render()`:
/// N damage rectangles are spread across the output, and the benchmark
/// measures the time to compute the merged bounding rectangle with
/// clamping to the output dimensions (1920×1080).
fn bench_damage_tracking(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_path/damage_tracking");

    for region_count in [10usize, 50usize, 200usize].iter() {
        group.bench_with_input(
            format!("merge_{}_damage_regions", region_count),
            region_count,
            |b, &region_count| {
                b.iter_batched(
                    || {
                        // Spread rectangles pseudo-randomly across a 1920×1080 output
                        let mut damage_rects: Vec<Rectangle> = Vec::with_capacity(region_count);
                        let mut rng = 0u32;
                        for _ in 0..region_count {
                            rng = rng.wrapping_add(1);
                            let x = (rng as i32 % 1800) + 50;
                            let y = (rng as i32 % 900) + 50;
                            let w = (rng % 200) + 50;
                            let h = (rng % 200) + 50;
                            damage_rects.push(Rectangle { x, y, width: w, height: h });
                        }
                        damage_rects
                    },
                    |damage_rects| {
                        // Bounding-box merge (matching the logic in backend/mod.rs render())
                        let mut min_x = i32::MAX;
                        let mut min_y = i32::MAX;
                        let mut max_x = i32::MIN;
                        let mut max_y = i32::MIN;

                        for r in &damage_rects {
                            min_x = min_x.min(r.x);
                            min_y = min_y.min(r.y);
                            max_x = max_x.max(r.x + r.width as i32);
                            max_y = max_y.max(r.y + r.height as i32);
                        }

                        // Clamp to 1920×1080 output
                        let (out_w, out_h) = (1920i32, 1080i32);
                        min_x = min_x.max(0);
                        min_y = min_y.max(0);
                        max_x = max_x.min(out_w);
                        max_y = max_y.min(out_h);

                        let merged = if min_x < max_x && min_y < max_y {
                            Some(Rectangle {
                                x: min_x,
                                y: min_y,
                                width: (max_x - min_x) as u32,
                                height: (max_y - min_y) as u32,
                            })
                        } else {
                            None
                        };

                        black_box(merged);
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

/// Benchmark LRU texture cache operations.
///
/// Uses `lru::LruCache<u64, u64>` to simulate the compositor's texture
/// cache (`lru::LruCache<ObjectId, TextureBuffer<GlesTexture>>`).
/// Measures lookup throughput (hits), eviction-pressure inserts, and a
/// mixed interleaved workload.
fn bench_texture_cache_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("render_path/texture_cache");

    // Measure lookup throughput on a warm cache (200 entries, 256 capacity)
    group.bench_function("lookup_hits", |b| {
        let mut cache = LruCache::new(NonZeroUsize::new(256).unwrap());
        for i in 0..200u64 {
            cache.put(i, i);
        }
        let keys: Vec<u64> = (0..200).collect();

        b.iter(|| {
            let mut sum = 0u64;
            for &k in &keys {
                sum += cache.get(&k).copied().unwrap_or(0);
            }
            black_box(sum);
        });
    });

    // Insert 512 entries into a 64-slot cache — exercises eviction
    group.bench_function("put_with_eviction", |b| {
        b.iter_batched(
            || LruCache::<u64, u64>::new(NonZeroUsize::new(64).unwrap()),
            |mut cache| {
                for i in 0..512u64 {
                    cache.put(i, i);
                }
                black_box(cache);
            },
            BatchSize::SmallInput,
        );
    });

    // Interleaved lookups (hits + misses) and inserts under eviction pressure
    group.bench_function("mixed_workload", |b| {
        b.iter_batched(
            || LruCache::<u64, u64>::new(NonZeroUsize::new(128).unwrap()),
            |mut cache| {
                for i in 0..64u64 {
                    cache.put(i, i);
                }
                for i in 0..128u64 {
                    let _ = cache.get(&(i % 96));       // ~2/3 hit rate
                    cache.put(200 + i, 200 + i);        // new inserts drive eviction
                }
                black_box(cache.len());
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
    bench_configuration,
    bench_memory_operations,
    bench_concurrency,
    bench_render_element_collection,
    bench_damage_tracking,
    bench_texture_cache_lookup,
);

criterion_main!(benches);
