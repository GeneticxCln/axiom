// Integration test: ensure the real backend (feature `real-compositor`) accepts a Wayland client
// and supports a basic roundtrip on the registry.

#![cfg(feature = "real-compositor")]

use anyhow::Result;
use parking_lot::RwLock;
use std::sync::Arc;

use axiom::config::AxiomConfig;
use axiom::decoration::DecorationManager;
use axiom::effects::EffectsEngine;
use axiom::input::InputManager;
use axiom::window::WindowManager;
use axiom::workspace::ScrollableWorkspaces;

#[test]
fn test_real_backend_connectivity_registry_roundtrip() -> Result<()> {
    let _ = env_logger::try_init();

    // Build minimal Axiom managers
    let cfg = AxiomConfig::default();
    let wm = Arc::new(RwLock::new(WindowManager::new(&cfg.window)?));
    let ws = Arc::new(RwLock::new(ScrollableWorkspaces::new(&cfg.workspace)?));
    let effects = Arc::new(RwLock::new(EffectsEngine::new(&cfg.effects)?));
    let input = Arc::new(RwLock::new(InputManager::new(&cfg.input, &cfg.bindings)?));
    let _deco = Arc::new(RwLock::new(DecorationManager::new(&cfg.window)));

    // Start the real Smithay backend (registers globals, opens WAYLAND_DISPLAY)
    use axiom::experimental::smithay::smithay_backend_real::real_xdg_backend::AxiomSmithayBackendReal as Real;
    let mut rb = Real::new(cfg, wm, ws, effects, input)?;
    rb.initialize()?;

    // Connect a Wayland client to the backend using environment variables
    use wayland_client::{globals::registry_queue_init, Connection};
    let conn = Connection::connect_to_env()?;
    let (_globals, mut event_queue) = registry_queue_init(&conn)?;

    // Run a few backend cycles to make sure the server processes our registry requests
    for _ in 0..3 {
        rb.run_one_cycle()?;
    }

    // Roundtrip to ensure events are delivered and processed
    event_queue.roundtrip(&mut ())?;

    // Shutdown backend
    rb.shutdown()?;

    Ok(())
}
