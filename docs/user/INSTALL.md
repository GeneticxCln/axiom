# Installing Axiom

## Current recommendation

Axiom is currently an **alpha-stage project**. The best way to evaluate it is in **nested/windowed mode**, not as a full standalone session replacement yet.

## Dependencies

Before building Axiom, ensure you have the necessary dependencies installed for your system.

### Ubuntu / Debian
```bash
sudo apt install \
  build-essential \
  cargo \
  libwayland-dev \
  libxkbcommon-dev \
  pkg-config
```

### Arch Linux
```bash
sudo pacman -S \
  cargo \
  mesa \
  pkgconf \
  rust \
  wayland \
  wayland-protocols \
  libxkbcommon
```

### Fedora
```bash
sudo dnf install \
  cargo \
  rust \
  wayland-devel \
  wayland-protocols-devel \
  libxkbcommon-devel
```

Optional but useful for nested smoke testing:
- `weston` (for `weston-terminal` client)
- `wayland-utils`

## Building from source

Axiom is built using Rust's `cargo` build system.

1. **Clone the repository**
    ```bash
    git clone https://github.com/GeneticxCln/axiom.git
    cd axiom
    ```

2. **Build debug binary**
    ```bash
    cargo build
    ```
    This is the recommended build for current alpha evaluation.

3. **Build release binary**
    ```bash
    cargo build --release
    ```

## First run

After building, prefer the nested alpha path:

```bash
cargo run -- --windowed --debug
```

Then follow the smoke-test instructions in [RUNNING.md](RUNNING.md).