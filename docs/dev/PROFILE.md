# Render Path Profiling

Axiom includes a profiling script (`scripts/profile_render.sh`) that uses
Linux `perf(1)` to sample the compositor's render path under realistic
nested (winit) operation.

## Prerequisites

- **Linux** with `perf` installed:
  ```sh
  sudo apt install linux-tools-common linux-tools-$(uname -r)
  ```
- **Rust toolchain** (`cargo`)
- **FlameGraph** (optional, for SVG output):
  ```sh
  git clone https://github.com/brendangregg/FlameGraph /opt/FlameGraph
  export FLAMEGRAPH_DIR=/opt/FlameGraph
  ```

## Usage

```sh
# Basic profile (30 seconds)
./scripts/profile_render.sh

# Profile for 15 seconds
./scripts/profile_render.sh --duration 15

# Profile with flamegraph SVG
./scripts/profile_render.sh --flamegraph

# Custom output directory
./scripts/profile_render.sh --output /tmp/my-profile

# Help
./scripts/profile_render.sh --help
```

### What it does

1. Builds the compositor in `--release` mode for representative performance.
2. Starts the compositor in nested (winit) mode as a background process.
3. Attaches `perf record -g` (with call-graph sampling) to the compositor PID
   for the specified duration.
4. Stops the compositor.
5. Writes a flat `perf report` (text).
6. Optionally generates a FlameGraph SVG via Brendan Gregg's scripts.

## Interpreting the output

All output lands in `/tmp/axiom-profile-*` (or your `--output` directory).

### `perf.data`
Raw perf sampling data. Open with `perf report -i <file>` for interactive
exploration or `perf annotate` for assembly-level hot-spots.

### `perf.report`
Text report sorted by total sampling cost. Look for functions in the
render path:
- `render()` and `capture_screencopy()` in the backend,
- `GlesRenderer` / `TextureRenderElement` / `SolidColorRenderElement` calls,
- GLES driver dispatch (`mesa`/`i965` frames).

High self-cost in `gl*` calls usually points to excessive state changes or
upload bandwidth. High cost in `swap_buffers` or `make_current` suggests
blocking on the display server (v-sync / buffer flip).

### `flamegraph.svg` (with `--flamegraph`)
Interactive SVG where the x-axis is stack frequency and the y-axis is stack
depth. Wide columns are hot paths. Click to zoom into a call subtree.

## Tips

- Run with a representative workload (e.g., a few client windows visible).
- The compositor must be the foreground process for the display; profiling
  works best in a separate SSH session or a terminal multiplexer.
- If `perf` reports "perf_event_open(…): No such file or directory", your
  kernel may need `kernel.perf_event_paranoid = -1` or `sudo`.
