/// IPC socket discovery example for the Axiom compositor.
///
/// Searches for the Axiom IPC socket by priority:
/// 1. `AXIOM_SOCKET_PATH` environment variable
/// 2. `$XDG_RUNTIME_DIR/axiom/axiom.sock`
/// 3. `/tmp/axiom-*/axiom-lazy-ui.sock` (first match, sorted by pid)
///
/// Then sends a `HealthCheck` message and prints the JSON response.
///
/// ```sh
/// # Default discovery:
/// cargo run --example ipc_discover --features examples
///
/// # Explicit path:
/// AXIOM_SOCKET_PATH=/tmp/axiom-1234/axiom-lazy-ui.sock \
///     cargo run --example ipc_discover --features examples
/// ```
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn main() {
    let socket_path = discover_socket().unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        eprintln!();
        eprintln!(
            "Usage: The Axiom compositor must be running to discover its IPC socket."
        );
        eprintln!("  Set AXIOM_SOCKET_PATH to specify the socket path directly.");
        std::process::exit(1);
    });

    eprintln!("🔌 Connecting to Axiom IPC socket: {:?}", socket_path);

    match send_health_check(&socket_path) {
        Ok(response) => {
            println!("{}", response);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

/// Discover the Axiom IPC socket path using the priority order.
///
/// 1. `AXIOM_SOCKET_PATH` env var (explicit override)
/// 2. `$XDG_RUNTIME_DIR/axiom/axiom.sock` (standard user session)
/// 3. `/tmp/axiom-*/axiom-lazy-ui.sock` (fallback, pick first by sort order)
fn discover_socket() -> Result<PathBuf, String> {
    // 1. AXIOM_SOCKET_PATH env var
    if let Ok(path) = std::env::var("AXIOM_SOCKET_PATH") {
        if !path.is_empty() {
            return Ok(PathBuf::from(path));
        }
    }

    // 2. $XDG_RUNTIME_DIR/axiom/axiom.sock
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            let path = PathBuf::from(dir).join("axiom").join("axiom.sock");
            if path.exists() {
                return Ok(path);
            }
        }
    }

    // 3. /tmp/axiom-*/axiom-lazy-ui.sock — scan /tmp for matching dirs
    if let Ok(entries) = std::fs::read_dir("/tmp") {
        let mut candidates: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("axiom-"))
            .map(|e| e.path().join("axiom-lazy-ui.sock"))
            .filter(|p| p.exists())
            .collect();
        // Sort for deterministic ordering (by pid suffix)
        candidates.sort();
        if let Some(path) = candidates.into_iter().next() {
            return Ok(path);
        }
    }

    Err("Axiom IPC socket not found. Is the compositor running?".to_string())
}

/// Connect to the IPC socket, send a `HealthCheck`, and read the response.
fn send_health_check(socket_path: &PathBuf) -> Result<String, Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;

    // Send a HealthCheck message — newline-delimited JSON per the IPC protocol.
    let msg = b"{\"type\":\"HealthCheck\"}\n";
    stream.write_all(msg)?;

    // Read the response (first line = one JSON message).
    let mut response = String::new();
    BufReader::new(&stream).read_line(&mut response)?;

    // Pretty-print the JSON response for readability.
    let trimmed = response.trim();
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::to_string_pretty(&parsed)?)
    } else {
        Ok(trimmed.to_string())
    }
}
