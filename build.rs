use std::process::Command;

fn main() {
    // Set build date
    let now = chrono::Utc::now()
        .format("%Y-%m-%d %H:%M:%S UTC")
        .to_string();
    println!("cargo:rustc-env=BUILD_DATE={}", now);

    // Set target triple - use CARGO_CFG_TARGET_TRIPLE if available, otherwise use TARGET
    let target = std::env::var("CARGO_CFG_TARGET_TRIPLE")
        .or_else(|_| std::env::var("TARGET"))
        .unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=TARGET_TRIPLE={}", target);

    // Set git commit hash if available
    if let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output() {
        if output.status.success() {
            let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=GIT_COMMIT={}", commit);
        }
    }

    // Tell cargo to re-run if git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");
}
