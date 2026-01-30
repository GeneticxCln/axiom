# Installing Axiom

## Dependencies

Before building Axiom, ensure you have the necessary dependencies installed for your system.

### Ubuntu / Debian
```bash
sudo apt install libwayland-dev pkg-config build-essential
```

### Arch Linux
```bash
sudo pacman -S rust wayland wayland-protocols pkg-config
```

### Fedora
```bash
sudo dnf install rust cargo wayland-devel wayland-protocols-devel
```

## Building from Source

Axiom is built using Rust's `cargo` build system.

1.  **Clone the repository:**
    ```bash
    git clone https://github.com/GeneticxCln/axiom.git
    cd axiom
    ```

2.  **Build Release Binary:**
    ```bash
    cargo build --release
    ```
    The compiled binary will be located at `target/release/axiom`.

3.  **Build Debug Binary (for development):**
    ```bash
    cargo build
    ```
    The binary will be at `target/debug/axiom`.
