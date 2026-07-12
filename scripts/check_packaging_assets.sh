#!/usr/bin/env bash
# Validate packaging/session assets without building distro packages.
#
# This is intentionally lightweight so CI can exercise it on multiple Ubuntu
# versions even when a full Rust build/package pass is handled elsewhere.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

log() {
    echo "[packaging-check] $*"
}

require_file() {
    local path="$1"
    [[ -f "$path" ]] || {
        echo "[packaging-check] ERROR: missing file: $path" >&2
        exit 1
    }
}

require_contains() {
    local path="$1"
    local needle="$2"
    if ! grep -Fq -- "$needle" "$path"; then
        echo "[packaging-check] ERROR: expected '$needle' in $path" >&2
        exit 1
    fi
}

require_file "packaging/arch/PKGBUILD"
require_file "packaging/axiom.desktop"
require_file "packaging/axiom.session"
require_file "packaging/axiom-session"
require_file "assets/logo.svg"
require_file "config/axiom.toml"
require_file "README.md"
require_file "LICENSE"

log "Checking shell syntax for packaging launcher"
bash -n packaging/axiom-session

log "Checking desktop entry metadata"
if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate packaging/axiom.desktop packaging/axiom.session
else
    log "desktop-file-validate not installed; skipping spec validation"
fi

require_contains "packaging/axiom.desktop" "Exec=axiom --windowed"
require_contains "packaging/axiom.desktop" "TryExec=axiom"
require_contains "packaging/axiom.desktop" "Icon=axiom"

require_contains "packaging/axiom.session" "Exec=axiom-session"
require_contains "packaging/axiom.session" "TryExec=axiom-session"
require_contains "packaging/axiom.session" "DesktopNames=Axiom"
require_contains "packaging/axiom.session" "Icon=axiom"

log "Checking session launcher behavior"
require_contains "packaging/axiom-session" "XDG_RUNTIME_DIR"
require_contains "packaging/axiom-session" "--backend=drm"
require_contains "packaging/axiom-session" 'XDG_CONFIG_HOME'
require_contains "packaging/axiom-session" '/etc/axiom/axiom.toml'

log "Checking PKGBUILD install payload"
require_contains "packaging/arch/PKGBUILD" 'install -Dm755 "target/release/axiom" "$pkgdir/usr/bin/axiom"'
require_contains "packaging/arch/PKGBUILD" 'install -Dm755 "packaging/axiom-session" "$pkgdir/usr/bin/axiom-session"'
require_contains "packaging/arch/PKGBUILD" 'install -Dm644 "packaging/axiom.desktop" "$pkgdir/usr/share/applications/axiom.desktop"'
require_contains "packaging/arch/PKGBUILD" 'install -Dm644 "packaging/axiom.session" "$pkgdir/usr/share/wayland-sessions/axiom.desktop"'
require_contains "packaging/arch/PKGBUILD" 'install -Dm644 "assets/logo.svg" "$pkgdir/usr/share/icons/hicolor/scalable/apps/axiom.svg"'
require_contains "packaging/arch/PKGBUILD" 'install -Dm644 "config/axiom.toml" "$pkgdir/etc/axiom/axiom.toml"'
require_contains "packaging/arch/PKGBUILD" 'install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"'

log "Packaging asset validation passed"
