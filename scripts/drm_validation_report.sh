#!/usr/bin/env bash
# Axiom DRM/KMS hardware validation helper.
#
# This script does not run the compositor for you. It prepares a machine-readable
# snapshot of the local DRM/KMS environment and prints the commands/observations
# maintainers should record when validating the standalone backend on real
# hardware.

set -euo pipefail

MODE="${1:-report}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
REPORT_DIR="$PROJECT_ROOT/drm-validation-reports"
TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
REPORT_PATH="$REPORT_DIR/drm-validation-${TIMESTAMP}.md"

log() {
    echo "[drm-validate] $*"
}

usage() {
    cat <<EOF
Axiom DRM/KMS hardware validation helper

Usage:
  $0 probe
  $0 report
  $0 help

Modes:
  probe   Print a concise hardware/software DRM probe to stdout
  report  Write a markdown validation stub under drm-validation-reports/
  help    Show this help text
EOF
}

maybe_run() {
    local label="$1"
    shift
    echo "## ${label}"
    if command -v "$1" >/dev/null 2>&1; then
        "$@" 2>&1 || true
    else
        echo "(tool not installed: $1)"
    fi
    echo
}

probe_stdout() {
    echo "# Axiom DRM/KMS Probe"
    echo
    echo "Timestamp: $(date --iso-8601=seconds 2>/dev/null || date)"
    echo "Hostname: $(hostname 2>/dev/null || echo unknown)"
    echo "Kernel: $(uname -srmo 2>/dev/null || uname -a)"
    echo

    echo "## DRM device nodes"
    if compgen -G "/dev/dri/*" >/dev/null; then
        ls -l /dev/dri
    else
        echo "No /dev/dri device nodes found"
    fi
    echo

    maybe_run "PCI graphics devices" sh -c "lspci -nn | grep -Ei 'vga|3d|display'"
    maybe_run "Seat/session summary" loginctl seat-status seat0
    maybe_run "Connected DRM connectors" sh -c 'for p in /sys/class/drm/card*-*/status; do echo "$p: $(cat "$p" 2>/dev/null)"; done'
    maybe_run "Current tty" tty
    maybe_run "modetest availability" modetest -h
}

generate_report() {
    mkdir -p "$REPORT_DIR"
    cat > "$REPORT_PATH" <<EOF
# Axiom DRM/KMS Validation Report

Generated: $(date --iso-8601=seconds 2>/dev/null || date)
Host: $(hostname 2>/dev/null || echo unknown)
Kernel: $(uname -srmo 2>/dev/null || uname -a)

## Environment snapshot

### DRM device nodes
$(if compgen -G "/dev/dri/*" >/dev/null; then ls -l /dev/dri | sed 's/^/    /'; else echo '    No /dev/dri device nodes found'; fi)

### PCI graphics devices
$(if command -v lspci >/dev/null 2>&1; then lspci -nn | grep -Ei 'vga|3d|display' | sed 's/^/    /' || echo '    (no PCI graphics lines found)'; else echo '    lspci not installed'; fi)

### Connected DRM connectors
$(for p in /sys/class/drm/card*-*/status; do echo "    $p: $(cat "$p" 2>/dev/null)"; done 2>/dev/null || true)

## Validation commands to run manually

### 1. Standalone startup

Run Axiom from a real DRM-capable seat/session:

	cargo run -- --backend=drm

Record:
- whether startup succeeds
- which outputs are detected
- whether input devices are usable

### 2. Hotplug / output re-enumeration

While Axiom is running:
- unplug one monitor
- replug it
- if available, add/remove a second monitor

Record:
- whether output re-enumeration logs appear
- whether windows remain visible/migrated sensibly
- whether focus remains usable

### 3. Multi-output layout

Record:
- active outputs and resolutions
- relative placement behavior
- whether the horizontal virtual-desktop assumption is acceptable or broken for this setup

### 4. Fractional scale / HiDPI

Record:
- reported scale factors
- whether buffer sizes and apparent window scale look sane
- whether mixed-DPI outputs behave acceptably

### 5. Shutdown path

Record:
- whether SIGINT/SIGTERM shutdown is clean
- whether the VT/session is returned in a usable state

## Result matrix

| Area | Status | Notes |
|---|---|---|
| DRM startup | TODO | |
| Input devices | TODO | |
| Single-output scanout | TODO | |
| Hotplug remove | TODO | |
| Hotplug re-add | TODO | |
| Multi-output layout | TODO | |
| Fractional scale / HiDPI | TODO | |
| Shutdown / session restore | TODO | |

## Overall conclusion

- TODO: summarize pass/fail/untested results for this machine
EOF

    log "Wrote validation report stub: $REPORT_PATH"
}

case "$MODE" in
    probe)
        probe_stdout
        ;;
    report)
        generate_report
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        echo "[drm-validate] ERROR: unknown mode '$MODE'" >&2
        usage >&2
        exit 1
        ;;
esac
