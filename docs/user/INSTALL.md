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
  libdrm-dev \
  libegl1-mesa-dev \
  libgbm-dev \
  libinput-dev \
  libwayland-dev \
  libxkbcommon-dev \
  pkg-config
```

### Arch Linux
```bash
sudo pacman -S \
  cargo \
  libdrm \
  libinput \
  mesa \
  pkgconf \
  rust \
  wayland \
  wayland-protocols
```

### Fedora
```bash
sudo dnf install \
  cargo \
  libdrm-devel \
  libinput-devel \
  mesa-libEGL-devel \
  mesa-libgbm-devel \
  rust \
  wayland-devel \
  wayland-protocols-devel \
  libxkbcommon-devel
```

Optional but useful for nested smoke testing:
- `weston`
- `wayland-utils`
- `xorg-xwayland`

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

## Packaged session assets

The repository now includes basic session packaging assets for downstream packages:
- `packaging/axiom.desktop` — nested/windowed launcher (`axiom --windowed`)
- `packaging/axiom-wayland.desktop` — Wayland session entry for display managers (`Exec=axiom-session`)
- `packaging/axiom-session` — wrapper that starts `axiom --backend=drm` with user/system config discovery
- `assets/logo.svg` — application/session icon source

There is **no** `packaging/axiom.session` file; the display-manager session entry is `axiom-wayland.desktop` (installed as `axiom.desktop` under `wayland-sessions/` by packaging). These assets are still **alpha-quality** and should be treated as early integration scaffolding, not a promise of a polished standalone desktop session.

## First run

After building, prefer the nested alpha path:

```bash
cargo run -- --windowed --debug
```

Then follow the smoke-test instructions in [RUNNING.md](RUNNING.md).
