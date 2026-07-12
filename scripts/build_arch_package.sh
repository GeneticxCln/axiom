#!/usr/bin/env bash
# Real PKGBUILD stage validation for Axiom.
#
# This executes the repository's Arch PKGBUILD functions (`prepare`, `build`,
# `package`) against the current checkout and validates the staged install tree.
# It does not invoke `makepkg`, but it does run the actual packaging logic from
# `packaging/arch/PKGBUILD` with a real build and a real `$pkgdir`.

set -euo pipefail

MODE="${1:-run}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
WORKDIR=""
KEEP_STAGE="${AXIOM_KEEP_STAGE:-false}"
PKGBUILD_PATH="$PROJECT_ROOT/packaging/arch/PKGBUILD"

log() {
    echo "[arch-package-build] $*"
}

fail() {
    echo "[arch-package-build] ERROR: $*" >&2
    exit 1
}

usage() {
    cat <<EOF
Axiom Arch package build validator

Usage:
  $0 run
  $0 help

Modes:
  run   Execute PKGBUILD prepare/build/package, validate the staged tree, and smoke-test installed artifacts
  help  Show this help text

Environment:
  AXIOM_KEEP_STAGE=true   Preserve the temporary work directory after success
EOF
}

cleanup() {
    local exit_code=$?
    if [[ "$KEEP_STAGE" == "true" && -n "$WORKDIR" && -d "$WORKDIR" ]]; then
        log "Preserving stage directory: $WORKDIR"
    elif [[ -n "$WORKDIR" && -d "$WORKDIR" ]]; then
        rm -rf "$WORKDIR"
    fi
    return "$exit_code"
}
trap cleanup EXIT

require_tool() {
    local tool="$1"
    command -v "$tool" >/dev/null 2>&1 || fail "required tool missing: $tool"
}

validate_staged_tree() {
    local pkgdir="$1"
    local pkgname="$2"

    local required_paths=(
        "$pkgdir/usr/bin/axiom"
        "$pkgdir/usr/bin/axiom-session"
        "$pkgdir/usr/share/applications/axiom.desktop"
        "$pkgdir/usr/share/wayland-sessions/axiom.desktop"
        "$pkgdir/usr/share/icons/hicolor/scalable/apps/axiom.svg"
        "$pkgdir/usr/share/axiom/axiom.toml"
        "$pkgdir/etc/axiom/axiom.toml"
        "$pkgdir/usr/share/doc/$pkgname/README.md"
        "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
    )

    for path in "${required_paths[@]}"; do
        [[ -e "$path" ]] || fail "staged package missing required path: $path"
    done

    if command -v desktop-file-validate >/dev/null 2>&1; then
        desktop-file-validate \
            "$pkgdir/usr/share/applications/axiom.desktop" \
            "$pkgdir/usr/share/wayland-sessions/axiom.desktop"
    fi

    log "Staged package contents validated successfully"
}

smoke_test_staged_install() {
    local pkgdir="$1"
    local staged_axiom="$pkgdir/usr/bin/axiom"
    local staged_session="$pkgdir/usr/bin/axiom-session"

    [[ -x "$staged_axiom" ]] || fail "staged axiom binary is not executable"
    [[ -x "$staged_session" ]] || fail "staged axiom-session wrapper is not executable"

    log "Running staged binary smoke test"
    timeout 10s "$staged_axiom" --help >/dev/null

    local fakebin="$WORKDIR/fake-bin"
    local marker="$WORKDIR/fake-axiom-args.txt"
    mkdir -p "$fakebin"
    cat > "$fakebin/axiom" <<EOF
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\$@" > "$marker"
EOF
    chmod +x "$fakebin/axiom"

    log "Testing staged session wrapper refuses missing XDG_RUNTIME_DIR"
    if env -i PATH="$fakebin:$pkgdir/usr/bin:/usr/bin:/bin" HOME="$WORKDIR/home-missing-runtime" \
        XDG_CONFIG_HOME="$WORKDIR/config-missing-runtime" \
        "$staged_session" >"$WORKDIR/missing-runtime.out" 2>"$WORKDIR/missing-runtime.err"; then
        fail "staged axiom-session should fail when XDG_RUNTIME_DIR is missing"
    fi
    grep -Fq "XDG_RUNTIME_DIR is not set" "$WORKDIR/missing-runtime.err" || \
        fail "missing-runtime failure did not mention XDG_RUNTIME_DIR"

    log "Testing staged session wrapper prefers user config"
    local runtime_user="$WORKDIR/runtime-user"
    local home_user="$WORKDIR/home-user"
    local config_home_user="$WORKDIR/config-user"
    mkdir -p "$runtime_user" "$home_user" "$config_home_user/axiom"
    chmod 700 "$runtime_user"
    cat > "$config_home_user/axiom/axiom.toml" <<EOF
[general]
debug = true
EOF
    rm -f "$marker"
    env -i \
        PATH="$fakebin:$pkgdir/usr/bin:/usr/bin:/bin" \
        HOME="$home_user" \
        XDG_CONFIG_HOME="$config_home_user" \
        XDG_RUNTIME_DIR="$runtime_user" \
        "$staged_session" --debug
    [[ -f "$marker" ]] || fail "fake axiom was not invoked for user-config case"
    grep -Fx -- "--config" "$marker" >/dev/null || fail "user-config case missing --config"
    grep -Fx -- "$config_home_user/axiom/axiom.toml" "$marker" >/dev/null || \
        fail "user-config case did not point at staged user config"
    grep -Fx -- "--backend=drm" "$marker" >/dev/null || fail "user-config case missing --backend=drm"
    grep -Fx -- "--debug" "$marker" >/dev/null || fail "user-config case did not preserve passthrough args"

    log "Testing staged session wrapper fallback path"
    local runtime_fallback="$WORKDIR/runtime-fallback"
    local home_fallback="$WORKDIR/home-fallback"
    local config_home_fallback="$WORKDIR/config-fallback"
    mkdir -p "$runtime_fallback" "$home_fallback" "$config_home_fallback"
    chmod 700 "$runtime_fallback"
    rm -f "$marker"
    env -i \
        PATH="$fakebin:$pkgdir/usr/bin:/usr/bin:/bin" \
        HOME="$home_fallback" \
        XDG_CONFIG_HOME="$config_home_fallback" \
        XDG_RUNTIME_DIR="$runtime_fallback" \
        "$staged_session"
    [[ -f "$marker" ]] || fail "fake axiom was not invoked for fallback case"
    grep -Fx -- "--backend=drm" "$marker" >/dev/null || fail "fallback case missing --backend=drm"
    if [[ -f /etc/axiom/axiom.toml ]]; then
        grep -Fx -- "--config" "$marker" >/dev/null || fail "system-config fallback missing --config"
        grep -Fx -- "/etc/axiom/axiom.toml" "$marker" >/dev/null || \
            fail "system-config fallback did not point at /etc/axiom/axiom.toml"
    else
        if grep -Fx -- "--config" "$marker" >/dev/null; then
            fail "fallback case unexpectedly used --config without /etc/axiom/axiom.toml present"
        fi
    fi

    log "Installed-artifact smoke tests passed"
}

run_pkgbuild_stage() {
    require_tool bash
    require_tool cargo
    require_tool git

    [[ -f "$PKGBUILD_PATH" ]] || fail "missing PKGBUILD: $PKGBUILD_PATH"

    log "Running packaging asset preflight"
    bash "$PROJECT_ROOT/scripts/check_packaging_assets.sh"

    WORKDIR="$(mktemp -d)"
    local srcdir="$WORKDIR/src"
    local pkgdir="$WORKDIR/pkg"
    mkdir -p "$srcdir" "$pkgdir"
    ln -s "$PROJECT_ROOT" "$srcdir/axiom"

    log "Temporary srcdir: $srcdir"
    log "Temporary pkgdir: $pkgdir"

    (
        set -euo pipefail
        export srcdir pkgdir CARCH="${CARCH:-x86_64}"
        # shellcheck source=/dev/null
        source "$PKGBUILD_PATH"
        resolved_pkgver="$(pkgver)"
        log "Resolved pkgver: $resolved_pkgver"
        prepare
        build
        package
        validate_staged_tree "$pkgdir" "$pkgname"
        smoke_test_staged_install "$pkgdir"
    )
}

case "$MODE" in
    run)
        run_pkgbuild_stage
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        fail "unknown mode: $MODE (try: $0 help)"
        ;;
esac
