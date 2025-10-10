proceed # Axiom Output Control Socket Guide

Axiom exposes a simple runtime control socket to manage logical outputs (monitors) dynamically while the compositor is running. This is useful for testing multi-output scissoring, layout, and presentation feedback.

The control socket is created at:

- $XDG_RUNTIME_DIR/axiom-control-<PID>.sock

Where <PID> is the process id of the Axiom main process.

Commands are newline-delimited and accepted in best-effort fashion. Each line is parsed as a command with optional parameters.

Supported commands
- add WIDTHxHEIGHT@SCALE+X,Y
  - Adds a logical output with the specified mode, scale and position.
  - Examples:
    - add 1920x1080@1+0,0
    - add 1280x1024@2+1920,0
- remove INDEX
  - Removes the logical output at the given index (0-based). Indices are assigned in creation order.

Notes
- If no outputs are added explicitly, a default single output is present.
- When using the Axiom CLI, you can preload outputs with --outputs "spec;spec;...".
- When an output is removed, pending frame callbacks for that output are completed or pruned as needed.
- Output add/remove operations also update the workspace viewport to the union of enabled outputs.

Examples
1) Add two outputs to the running session

```
PID=$(pidof -s axiom)
SOCK="$XDG_RUNTIME_DIR/axiom-control-$PID.sock"

# Add a 1920x1080 output at (0,0) and a 1280x720 output at (1920,0)
printf "add 1920x1080@1+0,0\n" | socat - UNIX-CONNECT:"$SOCK"
printf "add 1280x720@1+1920,0\n" | socat - UNIX-CONNECT:"$SOCK"
```

2) Remove the first output (index 0)

```
PID=$(pidof -s axiom)
SOCK="$XDG_RUNTIME_DIR/axiom-control-$PID.sock"
printf "remove 0\n" | socat - UNIX-CONNECT:"$SOCK"
```

3) Start with predefined outputs via CLI

```
cargo run --release -- \
  --outputs "1920x1080@1+0,0;1280x720@1+1920,0"
```

Troubleshooting
- Socket not found:
  - Ensure Axiom is running and $XDG_RUNTIME_DIR is set (falls back to /tmp if not set).
  - Confirm the process id and socket path.
- socat missing:
  - Install socat or use an alternative tool that can write to a Unix domain socket (e.g., `ncat -U`).
- No visible rendering on extra outputs:
  - The on-screen presenter draws to a single window but uses scissor rectangles for logical outputs; use the `--debug-outputs` flag to overlay output borders.

Flags affecting outputs behavior
- --outputs: Predefine logical outputs layout.
- --split-frame-callbacks: Split frame callbacks across overlapped outputs.
- --debug-outputs: Enable a debug overlay to visualize output rectangles.

