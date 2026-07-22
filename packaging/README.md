# Axiom Packaging

## Directory Structure

```
packaging/
├── README.md             ← this file
├── systemd/
│   └── axiom.service     ← systemd user unit
├── debian/
│   └── control           ← Debian packaging skeleton
└── flatpak/
    └── manifest.json     ← Flatpak manifest skeleton
```

## systemd — User Unit

Install and enable the systemd user service:

```sh
cp packaging/systemd/axiom.service ~/.config/systemd/user/
systemctl --user enable --now axiom
```

This starts Axiom as part of the `graphical-session.target`. The service
uses `Type=notify` so the compositor signals readiness. On failure it
restarts after a 3-second delay.

To check status:
```sh
systemctl --user status axiom
```

To stop:
```sh
systemctl --user stop axiom
```

## Debian Package

The `debian/control` file is a minimal skeleton for building a `.deb`
package. To build:

```sh
dpkg-buildpackage -us -uc
```

Requires `debhelper-compat (= 13)`, `cargo`, `rustc`, and the Wayland/EGL
development headers listed in `Build-Depends`.

## Flatpak

The `flatpak/manifest.json` is a minimal skeleton. Build with:

```sh
flatpak-builder build-dir packaging/flatpak/manifest.json
flatpak-builder --install build-dir
```

Requires the Freedesktop 24.08 runtime and SDK.
