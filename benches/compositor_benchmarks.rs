//! Performance benchmarks for Axiom compositor
//!
//! These benchmarks test performance-critical operations to prevent regressions
//! and guide optimization efforts.

use axiom::{config::WorkspaceConfig, workspace::ScrollableWorkspaces};
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

criterion_group!(
    benches,
    bench_workspace_scrolling,
    bench_window_layout,
    bench_configuration,
    bench_memory_operations,
    bench_concurrency,
);

criterion_main!(benches);
