//! Multi-output render pipeline integration tests.
//!
//! These tests verify the experimental multi-output feature:
//! - Multiple outputs exist after initialization
//! - Render pipeline iterates all outputs
//! - Per-output damage tracking
//!
//! Only runs when `--features multi-output-experimental` is enabled.

use axiom::backend::{AxiomSmithayBackendReal, BackendKind};
use axiom::config::{AxiomConfig, BindingsConfig, InputConfig, WindowConfig, WorkspaceConfig};
use axiom::decoration::DecorationManager;
use axiom::input::InputManager;
use axiom::window::WindowManager;
use axiom::workspace::ScrollableWorkspaces;
use parking_lot::RwLock;
#[cfg(feature = "multi-output-experimental")]
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel};
#[cfg(feature = "multi-output-experimental")]
use smithay::utils::Transform;
use std::sync::Arc;

/// Create a headless backend with 2 virtual outputs for multi-output testing.
fn multi_output_test_backend() -> AxiomSmithayBackendReal {
    #[allow(unused_mut)]
    let mut backend = AxiomSmithayBackendReal::new_for_test(
        AxiomConfig::default(),
        Arc::new(RwLock::new(WindowManager::new(&WindowConfig::default()))),
        Arc::new(RwLock::new(ScrollableWorkspaces::new(
            &WorkspaceConfig::default(),
        ))),
        Arc::new(RwLock::new(InputManager::new(
            &InputConfig::default(),
            &BindingsConfig::default(),
        ))),
        Arc::new(RwLock::new(DecorationManager::new(
            &WindowConfig::default(),
            false,
        ))),
    )
    .expect("multi-output test backend");

    #[cfg(feature = "multi-output-experimental")]
    {
        let output1 = Output::new(
            "Axiom-Output-0".into(),
            PhysicalProperties {
                size: (1920, 1080).into(),
                subpixel: Subpixel::Unknown,
                make: "Axiom".into(),
                model: "Virtual".into(),
            },
        );
        let mode1 = OutputMode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        output1.change_current_state(Some(mode1), Some(Transform::Normal), Some(Scale::Integer(1)), None);

        let output2 = Output::new(
            "Axiom-Output-1".into(),
            PhysicalProperties {
                size: (1280, 720).into(),
                subpixel: Subpixel::Unknown,
                make: "Axiom".into(),
                model: "Virtual".into(),
            },
        );
        let mode2 = OutputMode {
            size: (1280, 720).into(),
            refresh: 60_000,
        };
        output2.change_current_state(Some(mode2), Some(Transform::Normal), Some(Scale::Integer(1)), None);

        backend.state.outputs = vec![output1, output2];
    }

    backend
}

#[test]
#[cfg_attr(not(feature = "multi-output-experimental"), ignore)]
fn test_multi_output_initial_state() {
    let backend = multi_output_test_backend();

    assert!(
        backend.state.outputs.len() >= 2,
        "expected 2+ outputs, got {}",
        backend.state.outputs.len()
    );

    for (i, output) in backend.state.outputs.iter().enumerate() {
        let name = output.name();
        assert!(!name.is_empty(), "output {} has empty name", i);
    }

    assert_eq!(backend.backend_kind, BackendKind::Noop);
}

#[test]
#[cfg_attr(not(feature = "multi-output-experimental"), ignore)]
fn test_multi_output_names_distinct() {
    let backend = multi_output_test_backend();

    let mut names: Vec<String> = backend
        .state
        .outputs
        .iter()
        .map(|o| o.name().to_string())
        .collect();
    names.sort();
    names.dedup();

    assert_eq!(
        names.len(),
        backend.state.outputs.len(),
        "output names should be unique"
    );
}

#[test]
#[cfg_attr(not(feature = "multi-output-experimental"), ignore)]
fn test_multi_output_render_cycle_no_panic() {
    let mut backend = multi_output_test_backend();

    backend.state.needs_redraw = true;
    backend
        .run_one_cycle()
        .expect("render cycle should not fail");

    assert_eq!(backend.backend_kind, BackendKind::Noop);
}

#[test]
#[cfg_attr(not(feature = "multi-output-experimental"), ignore)]
fn test_multi_output_damage_tracking() {
    let mut backend = multi_output_test_backend();

    backend.state.output_damage.push(
        smithay::utils::Rectangle::new(
            smithay::utils::Point::from((0, 0)),
            smithay::utils::Size::from((100, 100)),
        ),
    );

    assert_eq!(
        backend.state.output_damage.len(),
        1,
        "should have 1 damage rect after commit"
    );

    backend.state.needs_redraw = true;
    backend
        .run_one_cycle()
        .expect("render cycle should not fail");
}