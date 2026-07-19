use super::*;
use crate::backend::xwm::{AxiomXwm, XwmEvent};
use anyhow::Result;
use serial_test::serial;
use std::os::unix::net::UnixStream;
use tokio::process::Command;
use tokio::time::{sleep, Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, Window};

async fn command_exists(binary: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {} >/dev/null 2>&1", binary))
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

fn decode_text_property(bytes: &[u8]) -> Option<String> {
    let trimmed = bytes.split(|b| *b == 0).next().unwrap_or(bytes);
    let text = String::from_utf8_lossy(trimmed).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn decode_wm_class(bytes: &[u8]) -> Option<String> {
    let parts: Vec<String> = bytes
        .split(|b| *b == 0)
        .filter_map(|part| {
            let s = String::from_utf8_lossy(part).trim().to_string();
            (!s.is_empty()).then_some(s)
        })
        .collect();

    match parts.as_slice() {
        [instance, class, ..] => Some(format!("{} ({})", class, instance)),
        [only] => Some(only.clone()),
        _ => None,
    }
}

struct TestAtoms {
    utf8_string: u32,
    wm_name: u32,
    wm_class: u32,
    net_wm_name: u32,
}

impl TestAtoms {
    fn new(conn: &x11rb::rust_connection::RustConnection) -> Result<Self> {
        Ok(Self {
            utf8_string: conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom,
            wm_name: conn.intern_atom(false, b"WM_NAME")?.reply()?.atom,
            wm_class: conn.intern_atom(false, b"WM_CLASS")?.reply()?.atom,
            net_wm_name: conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom,
        })
    }
}

fn read_window_title(
    conn: &x11rb::rust_connection::RustConnection,
    atoms: &TestAtoms,
    window: Window,
) -> Option<String> {
    if let Ok(reply) =
        conn.get_property(false, window, atoms.net_wm_name, atoms.utf8_string, 0, 1024)
    {
        if let Ok(prop) = reply.reply() {
            if let Some(title) = decode_text_property(&prop.value) {
                return Some(title);
            }
        }
    }

    if let Ok(reply) = conn.get_property(false, window, atoms.wm_name, AtomEnum::ANY, 0, 1024) {
        if let Ok(prop) = reply.reply() {
            if let Some(title) = decode_text_property(&prop.value) {
                return Some(title);
            }
        }
    }

    None
}

fn read_window_class(
    conn: &x11rb::rust_connection::RustConnection,
    atoms: &TestAtoms,
    window: Window,
) -> Option<String> {
    if let Ok(reply) = conn.get_property(false, window, atoms.wm_class, AtomEnum::STRING, 0, 1024) {
        if let Ok(prop) = reply.reply() {
            return decode_wm_class(&prop.value);
        }
    }
    None
}

async fn wait_for_window_metadata(
    display: u32,
    expected_title: &str,
    timeout: Duration,
) -> Result<Option<(Window, String, Option<String>)>> {
    let display_name = format!(":{}", display);
    let (conn, screen_num) = x11rb::connect(Some(&display_name))?;
    let atoms = TestAtoms::new(&conn)?;
    let root = conn.setup().roots[screen_num].root;
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        let tree = conn.query_tree(root)?.reply()?;
        for window in tree.children {
            if let Some(title) = read_window_title(&conn, &atoms, window) {
                if title == expected_title {
                    let class = read_window_class(&conn, &atoms, window);
                    return Ok(Some((window, title, class)));
                }
            }
        }
        sleep(Duration::from_millis(100)).await;
    }

    Ok(None)
}

async fn wait_for_xwm_map_event(
    xwm: &mut AxiomXwm,
    timeout: Duration,
) -> Result<Option<(u32, String, Option<String>)>> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        while let Some(event) = xwm.poll_event() {
            if let Some(XwmEvent::WindowMapped {
                x11_window_id,
                title,
                class,
            }) = xwm.handle_event(&event)?
            {
                return Ok(Some((x11_window_id, title, class)));
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    Ok(None)
}

/// XWayland lifecycle test — isolated to avoid failures on headless systems.
/// Skips when Xwayland binary is absent OR when startup fails (e.g. no
/// parent Wayland compositor is available).
#[tokio::test]
#[serial]
async fn test_xwayland_manager_lifecycle() {
    if !command_exists("Xwayland").await {
        log::warn!("Skipping XWayland test: Xwayland not found in PATH");
        return;
    }

    let config = XWaylandConfig {
        enabled: true,
        display: None,
    };

    let mut manager = XWaylandManager::new(&config)
        .await
        .expect("Failed to create XWayland manager");

    // Give it a moment to settle.
    sleep(Duration::from_millis(500)).await;

    // If XWayland failed to start (e.g. no free display, no parent Wayland
    // compositor, or permission denied), still verify shutdown works and bail
    // gracefully.
    if manager.server_state != XWaylandServerState::Running {
        log::warn!(
            "XWayland did not start (state: {:?}) — testing graceful shutdown only",
            manager.server_state
        );
        manager.shutdown().await.expect("Failed to shutdown");
        assert_eq!(manager.server_state, XWaylandServerState::Stopped);
        return;
    }

    // Server is running — verify state invariants.
    assert!(manager.xwayland_process.is_some());
    assert!(manager.display_number.is_some());
    if let Some(display) = manager.display_number {
        log::info!("XWayland started on :{}", display);
    }

    // Shutdown and verify cleanup.
    manager.shutdown().await.expect("Failed to shutdown");
    assert_eq!(manager.server_state, XWaylandServerState::Stopped);
    assert!(manager.xwayland_process.is_none());
    assert!(manager.display_number.is_none());
    std::env::remove_var("DISPLAY");
}

/// Real-client smoke test for the current XWayland server path.
///
/// This validates that, when a parent Wayland compositor is available,
/// `XWaylandManager` can launch an actual X11 client (`xdpyinfo`) against the
/// spawned display. It does NOT prove full compositor-side XWM window mapping
/// inside Axiom yet; that backend wiring remains a separate milestone.
#[tokio::test]
#[serial]
async fn test_xwayland_manager_accepts_real_x11_client() {
    if !command_exists("Xwayland").await {
        log::warn!("Skipping XWayland client smoke test: Xwayland not found in PATH");
        return;
    }
    if !command_exists("xdpyinfo").await {
        log::warn!("Skipping XWayland client smoke test: xdpyinfo not found in PATH");
        return;
    }

    let config = XWaylandConfig {
        enabled: true,
        display: None,
    };

    let mut manager = XWaylandManager::new(&config)
        .await
        .expect("Failed to create XWayland manager");

    sleep(Duration::from_millis(500)).await;

    if manager.server_state != XWaylandServerState::Running {
        log::warn!(
            "XWayland did not start (state: {:?}) — skipping real-client smoke path",
            manager.server_state
        );
        manager.shutdown().await.expect("Failed to shutdown");
        assert_eq!(manager.server_state, XWaylandServerState::Stopped);
        return;
    }

    let display = manager
        .display_number
        .expect("running XWayland server should expose display number");
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new("xdpyinfo")
            .env("DISPLAY", format!(":{}", display))
            .output(),
    )
    .await
    .expect("xdpyinfo timed out against XWayland display")
    .expect("failed to run xdpyinfo against XWayland display");

    assert!(
        output.status.success(),
        "xdpyinfo should connect successfully to XWayland display :{}\nstdout:\n{}\nstderr:\n{}",
        display,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("name of display"),
        "xdpyinfo output should describe the target display"
    );

    manager.shutdown().await.expect("Failed to shutdown");
    assert_eq!(manager.server_state, XWaylandServerState::Stopped);
    std::env::remove_var("DISPLAY");
}

/// Real X11 metadata smoke test.
///
/// Launches a real X11 client (`xmessage`) against the spawned XWayland server,
/// then queries the resulting window properties over X11 to verify that title
/// and class metadata are present and readable end-to-end. This still stops at
/// the XWayland/X11 boundary; compositor-side XWM mapping into Axiom remains a
/// separate wiring task.
#[tokio::test]
#[serial]
async fn test_xwayland_real_x11_client_metadata() {
    if !command_exists("Xwayland").await {
        log::warn!("Skipping XWayland metadata smoke test: Xwayland not found in PATH");
        return;
    }
    if !command_exists("xmessage").await {
        log::warn!("Skipping XWayland metadata smoke test: xmessage not found in PATH");
        return;
    }

    let config = XWaylandConfig {
        enabled: true,
        display: None,
    };

    let mut manager = XWaylandManager::new(&config)
        .await
        .expect("Failed to create XWayland manager");

    sleep(Duration::from_millis(500)).await;

    if manager.server_state != XWaylandServerState::Running {
        log::warn!(
            "XWayland did not start (state: {:?}) — skipping metadata smoke path",
            manager.server_state
        );
        manager.shutdown().await.expect("Failed to shutdown");
        assert_eq!(manager.server_state, XWaylandServerState::Stopped);
        return;
    }

    let display = manager
        .display_number
        .expect("running XWayland server should expose display number");
    let expected_title = "Axiom Metadata Smoke";
    let expected_instance = "axiom-metadata-smoke";

    let mut child = Command::new("xmessage")
        .env("DISPLAY", format!(":{}", display))
        .arg("-name")
        .arg(expected_instance)
        .arg("-title")
        .arg(expected_title)
        .arg("Axiom XWayland metadata smoke test")
        .spawn()
        .expect("failed to launch xmessage against XWayland display");

    let metadata = wait_for_window_metadata(display, expected_title, Duration::from_secs(5))
        .await
        .expect("failed to inspect X11 window metadata");

    let (_window, title, class) = metadata.expect("timed out waiting for xmessage window metadata");
    assert_eq!(
        title, expected_title,
        "window title should round-trip through XWayland"
    );

    let class = class.expect("WM_CLASS should be present for xmessage window");
    assert!(
        class.contains(expected_instance)
            || class.contains("Xmessage")
            || class.contains("xmessage"),
        "decoded WM_CLASS should expose the instance/class identity, got: {}",
        class
    );

    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;

    manager.shutdown().await.expect("Failed to shutdown");
    assert_eq!(manager.server_state, XWaylandServerState::Stopped);
    std::env::remove_var("DISPLAY");
}

/// Compositor-side XWM wiring smoke test.
///
/// Starts XWayland with a real `-wm` stream, constructs `AxiomXwm` on the
/// compositor side, and verifies that a real X11 client triggers a
/// `WindowMapped` event with usable metadata.
#[tokio::test]
#[serial]
async fn test_xwayland_manager_wired_xwm_receives_map_events() {
    if !command_exists("Xwayland").await {
        log::warn!("Skipping XWayland/XWM smoke test: Xwayland not found in PATH");
        return;
    }
    if !command_exists("xmessage").await {
        log::warn!("Skipping XWayland/XWM smoke test: xmessage not found in PATH");
        return;
    }

    let config = XWaylandConfig {
        enabled: false,
        display: None,
    };
    let mut manager = XWaylandManager::new(&config)
        .await
        .expect("Failed to create XWayland manager");

    let (xwm_stream, xwayland_stream) = UnixStream::pair().expect("socketpair for xwm/xwayland");
    manager
        .restart_with_wm_stream(xwayland_stream)
        .await
        .expect("Failed to start XWayland with compositor-side XWM stream");

    sleep(Duration::from_millis(500)).await;

    if manager.server_state != XWaylandServerState::Running {
        log::warn!(
            "XWayland did not start (state: {:?}) — skipping compositor-side XWM smoke path",
            manager.server_state
        );
        manager.shutdown().await.expect("Failed to shutdown");
        assert_eq!(manager.server_state, XWaylandServerState::Stopped);
        return;
    }

    let mut xwm = AxiomXwm::new(xwm_stream).expect("Failed to create compositor-side AxiomXwm");
    let display = manager
        .display_number
        .expect("running XWayland server should expose display number");

    let expected_title = "Axiom XWM Smoke";
    let mut child = Command::new("xmessage")
        .env("DISPLAY", format!(":{}", display))
        .arg("-name")
        .arg("axiom-xwm-smoke")
        .arg("-title")
        .arg(expected_title)
        .arg("Axiom compositor-side XWM smoke test")
        .spawn()
        .expect("failed to launch xmessage against XWayland display");

    let mapped = wait_for_xwm_map_event(&mut xwm, Duration::from_secs(5))
        .await
        .expect("failed while waiting for XWM map event")
        .expect("timed out waiting for compositor-side XWM map event");

    assert_eq!(
        mapped.1, expected_title,
        "mapped X11 title should match the real client title"
    );
    let class = mapped
        .2
        .expect("mapped X11 client should carry a WM_CLASS value");
    assert!(
        class.contains("axiom-xwm-smoke")
            || class.contains("Xmessage")
            || class.contains("xmessage"),
        "mapped X11 class should expose the instance/class identity, got: {}",
        class
    );

    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;

    manager.shutdown().await.expect("Failed to shutdown");
    assert_eq!(manager.server_state, XWaylandServerState::Stopped);
    std::env::remove_var("DISPLAY");
}
