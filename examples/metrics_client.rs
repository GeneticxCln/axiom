use serde_json::json;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

fn main() {
    let args: Vec<String> = env::args().collect();
    let socket_path = args.get(1).cloned().unwrap_or_else(|| {
        eprintln!("Usage: metrics_client <socket_path>");
        eprintln!("  Socket path from compositor log: 'Wayland socket: wayland-axiom-<pid>'");
        std::process::exit(1);
    });
    match connect_and_query(&socket_path) {
        Ok(report) => println!("{}", report),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn connect_and_query(socket_path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut buf = serde_json::to_vec(&json!({"type": "GetPerformanceReport"}))?;
    buf.push(b'\n');
    stream.write_all(&buf)?;
    let mut response = String::new();
    BufReader::new(&stream).read_line(&mut response)?;
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response) {
        Ok(serde_json::to_string_pretty(&parsed)?)
    } else {
        Ok(response)
    }
}
