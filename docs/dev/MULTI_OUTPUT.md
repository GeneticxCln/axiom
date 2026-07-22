# Multi-Output Rendering & State Model

## Data Structures

### Output struct

A new `OutputInfo` struct holds per-output state. It is completely independent
of Smithay's `Output` (which is a protocol object sent to clients via
`wl_output`); this is the compositor-internal representation.

```rust
/// Per-output compositor state.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    /// Logical name, e.g. "eDP-1", "HDMI-A-1", or "Axiom-Output-N".
    pub name: String,
    /// Position in the global compositor coordinate space (logical pixels).
    pub position: (i32, i32),
    /// Logical resolution in output-local coordinates.
    pub size: (i32, i32),
    /// HiDPI scale factor.
    pub scale: f64,
    /// Transform applied to this output (normal, 90, 180, 270).
    pub transform: Transform,
    /// Accumulated damage regions since last present, in physical pixels.
    pub damage: Vec<Rectangle<i32, Physical>>,
}
```

### State additions

```rust
pub struct State {
    // …existing fields…

    /// Per-output information. Feature-gated behind `multi-output-experimental`.
    /// When the feature is disabled, this always contains exactly one entry
    /// (the default output) and behaves identically to the current single-output
    /// code path.
    #[cfg(feature = "multi-output-experimental")]
    pub output_info: Vec<OutputInfo>,

    /// Single-output alias — always present regardless of feature flag.
    /// Points to `output_info[0]` when the feature is enabled; holds the
    /// same struct when disabled. All single-output code accesses this field
    /// directly, so no conditional compilation leaks into logic paths.
    pub primary_output: OutputInfo,
}
```

Design rationale:

- `output_info` is **only** compiled when `multi-output-experimental` is active.
  The default build has zero new fields in `OutputInfo` — it just holds
  `primary_output`. This guarantees no memory or codegen bloat for existing
  users.
- `primary_output` aliases `output_info[0]` when the feature is on, or stands
  alone when off. Callers that want the "main" output (e.g. `focused_output_scale`,
  cursor confinement, fullscreen target) query `primary_output` regardless of
  the feature flag.
- `damage` moves from `State.output_damage: Vec<Rectangle<…>>` (a single flat
  list) into `OutputInfo.damage`. Each output tracks only its own dirty region.

### Feature-guarded `Vec<OutputInfo>` in State

```rust
#[cfg(feature = "multi-output-experimental")]
pub output_info: Vec<OutputInfo>,

// Not feature-gated — always present.
pub primary_output: OutputInfo,
```

When the feature is **disabled**:
- `output_info` does not exist.
- `primary_output` holds the single output (name "Axiom-Output-0").
- Damage is accumulated in the old `state.output_damage` field.
- `render()` follows today's single-bind-then-submit path.

When the feature is **enabled**:
- `output_info` is the source of truth.
- `primary_output` is updated to always match `output_info[0]`.
- `render()` iterates `output_info`, binding and presenting per-output.
- Damage is tracked per `OutputInfo.damage`; the old `state.output_damage` is
  not compiled.

### Default single-output behavior preserved when feature disabled

All existing code paths that reference `self.window_width`, `self.window_height`,
`self.output_damage`, and `self.outputs` continue to work unchanged. The
feature flag only adds *new* code; it never deletes or modifies an existing
field's definition. `PrimaryOutput` accessors return `&output_info[0]` or
`&self.primary_output` uniformly, keeping logic branches to a minimum.

---

## Lifecycle

### Output add/remove events

**Sources:**

| Source | Mechanism | Notes |
|--------|-----------|-------|
| Winit | `WinitEvent::Resized` — single-output; no hotplug. | Today's path; no change in default builds. |
| DRM (future) | Hotplug uevent → connector added/removed. | Out of scope for this doc; the plumbing should accept a `(name, size, scale)` tuple. |
| IPC / config | `config.output.order` already parsed. | The tape sync machinery in `AxiomSmithayBackendReal::new` accepts a list of output names. |

**Add path:**

1. Source emits `OutputAdded { name, size, scale, position }`.
2. `State::add_output(info)`:
   - Creates a Smithay `Output` (if not already registered) and calls
     `output.create_global::<State>(&dh)` so Wayland clients see it.
   - Pushes a new `OutputInfo` into `output_info`.
   - Updates the workspace manager tape list (`known_tape_ids`) to include the
     new output name. The tape already supports per-output viewport size and
     scale factor.
   - Schedules a redraw.
   - Clients bound to this output via `wl_output` receive the new mode/scale
     events.

**Remove path:**

1. Source emits `OutputRemoved { name }`.
2. `State::remove_output(name)`:
   - Removes the entry from `output_info` (minimum one output must remain —
     the compositor always needs at least one output).
   - Destroys the Smithay `Output` global so clients learn of the removal.
   - Prunes the workspace manager tape for the removed output.
   - Migrates any windows that were exclusively on the removed output to the
     remaining output.
   - Schedules a redraw.

### Per-output prepare_render_scene

Today `prepare_render_scene()` calculates window layouts using a single
viewport (`self.window_width × self.window_height`). In the multi-output
model, each output:

1. Has its own viewport size and position in the global coordinate space.
2. Hosts a subset of windows (those assigned to its workspace column by the
   workspace manager).
3. Renders only its own visible surfaces.

**Multi-output `prepare_render_scene` contract:**

```rust
fn prepare_output_scene(
    &mut self,
    output: &OutputInfo,
) -> HashMap<u64, WindowRectangle>
```

- Takes an `OutputInfo` reference, not the global viewport.
- Queries the workspace manager for the tape matching `output.name` to get the
  tiling layout for windows assigned to that output.
- Returns a `HashMap<window_id, rect>` in **output-local** coordinates (origin
  at the output's top-left). The caller offsets by `output.position` when
  compositing the scene.

### Per-output present

Each output needs its own `bind()` / `submit()` cycle:

```rust
for output_info in &state.output_info {
    let (renderer, framebuffer) = backend.bind_for_output(&output_info)?;
    let layouts = state.prepare_output_scene(output_info);
    render_scene_into(state, renderer, &mut framebuffer, &layouts, output_info)?;
    Self::capture_screencopy(state, renderer, &mut framebuffer);

    let damage = merge_output_damage(&output_info.damage);
    backend.submit_for_output(&output_info, damage)?;
    output_info.damage.clear();
}
```

- `bind_for_output` activates the appropriate GL context for that output
  (winit window per output, or a shared context with per-output FBO).
- `submit_for_output` presents the per-output backbuffer.

### Damage tracking per-output

Damage accumulation today is a single `Vec<Rectangle<i32, Physical>>`:

```rust
// Current (single-output):
pub output_damage: Vec<Rectangle<i32, Physical>>,
```

Proposed:

```rust
// Per-output damage (multi-output):
// Stored inside each OutputInfo:
pub damage: Vec<Rectangle<i32, Physical>>,
```

- When a surface commits, the damage rectangle is pushed to **every output**
  that the surface is visible on (determined by which output's workspace column
  contains the surface's window).
- When an output presents, only its own `damage` vec is merged and submitted.
- The merge logic (bounding-box union) is identical to today's — just moved
  into the per-output loop.

---

## Render Loop Changes

### Current single-output path

```
render()
  ├─ bind()             → winit GLES context
  ├─ render_scene_into()
  │    ├─ state.prepare_render_scene()   → global viewport layout
  │    ├─ import_surface_tree()          → texture cache (shared)
  │    ├─ frame.clear()
  │    ├─ draw windows + decorations
  │    └─ frame.finish()
  ├─ capture_screencopy()
  └─ submit(damage)     → present to winit window
```

### Proposed multi-output path

```
render()
  │
  ├─ #[cfg(multi-output-experimental)]
  │  for output_info in &state.output_info {
  │      bind_for_output(output_info)
  │      render_scene_for_output(output_info)
  │      capture_screencopy(output_info)
  │      submit_for_output(output_info)
  │  }
  │
  └─ #[cfg(not(multi-output-experimental))]
     // unchanged single-output path
     bind() → render_scene_into() → capture_screencopy() → submit(damage)
```

**`render_scene_for_output` decomposition:**

```
render_scene_for_output(state, renderer, framebuffer, output_info)
  ├─ layouts = state.prepare_output_scene(output_info)
  ├─ import_surface_tree()    ← shared texture cache, unchanged
  ├─ frame.clear()
  ├─ draw windows (offset by -output_info.position)
  ├─ draw decorations
  ├─ draw layer-shell surfaces
  ├─ draw DnD icon
  └─ frame.finish()
```

### Shared texture cache across outputs

The `texture_cache: LruCache<ObjectId, TextureBuffer<GlesTexture>>` on `State`
remains **single, shared, and unchanged**. A client `wl_buffer` is imported
into the GPU exactly once regardless of how many outputs display it. The
import step (`renderer.import_buffer`) happens in the first render pass, and
subsequent outputs draw from the same cached texture.

Constraint: GL contexts must **share resource lists** when there are multiple
winit windows (one per output). Smithay's `GlesRenderer` supports this via
`gles::GlesRenderer::new_shared`. If a single winit window with per-output
FBOs is used instead, sharing is automatic.

### Per-output damage merge

The bounding-box merge that today sits in `render()` moves into each output's
iteration:

```rust
fn merge_output_damage(damage: &[Rectangle<i32, Physical>]) -> Option<Vec<Rectangle<i32, Physical>>> {
    if damage.is_empty() {
        return None;
    }
    // Same bounding-box merge as today's render() body.
    // ponytail: bounding-box merge; switch to OutputDamageTracker for
    // per-element occlusion culling if profiling shows it matters.
    let mut min_x = i32::MAX;
    // …
}
```

---

## API Changes

### AxiomSmithayBackendReal additions

**New methods** (all feature-gated):

```rust
impl AxiomSmithayBackendReal {
    fn bind_for_output(&mut self, output: &OutputInfo) -> Result<(&mut GlesRenderer, GlesTarget<'_>)>;
    fn submit_for_output(&mut self, output: &OutputInfo, damage: Option<&[Rectangle<i32, Physical>]>) -> Result<()>;
}
```

- `bind_for_output`: If using per-output winit windows, selects the
  corresponding backend. If using FBO-based multi-output, binds the shared
  backend then sets the scissor/viewport to the output's region.
- `submit_for_output`: Presents the per-output buffer (swap for per-output
  winit windows, blit to the correct region of the shared framebuffer for
  FBO-based).

**New state manipulation methods:**

```rust
impl State {
    pub fn add_output(&mut self, info: OutputInfo) -> Result<()>;
    pub fn remove_output(&mut self, name: &str) -> Result<()>;
    pub fn output_by_name(&self, name: &str) -> Option<&OutputInfo>;
    pub fn output_by_name_mut(&mut self, name: &str) -> Option<&mut OutputInfo>;
}
```

These are compiled unconditionally but the `output_info` backing store is only
present when the feature is enabled. When disabled, `add_output` is a no-op
that logs a warning, `remove_output` is a no-op, and `output_by_name` always
returns `Some(&primary_output)`.

### AxiomCompositor changes for multi-output awareness

```rust
impl AxiomCompositor {
    /// Called when the backend detects an output hotplug event.
    pub fn handle_output_added(&mut self, name: String, size: (i32, i32), scale: f64) {
        let info = OutputInfo {
            name: name.clone(),
            position: self.compute_next_output_position(),
            size,
            scale,
            transform: Transform::Normal,
            damage: Vec::new(),
        };
        self.smithay_backend.state.add_output(info);
        self.smithay_backend.state.needs_redraw = true;
    }

    /// Called when an output is disconnected.
    pub fn handle_output_removed(&mut self, name: &str) {
        self.smithay_backend.state.remove_output(name);
        self.smithay_backend.state.needs_redraw = true;
    }

    /// Compute the position for a new output (right of the rightmost current output).
    fn compute_next_output_position(&self) -> (i32, i32) {
        let outputs = &self.smithay_backend.state.output_info; // or primary fallback
        let max_x = outputs.iter().map(|o| o.position.0 + o.size.0).max().unwrap_or(0);
        (max_x, 0)
    }
}
```

### Integration test strategy

**Simulate 2 outputs, verify both present:**

```
#[test]
#[cfg(feature = "multi-output-experimental")]
fn test_multi_output_render_cycle() {
    // 1. Create a backend with two outputs.
    // 2. Add a window assigned to output 0.
    // 3. Add a window assigned to output 1.
    // 4. Tick the compositor (render cycle).
    // 5. Verify that render_scene_for_output was called for each output
    //    (e.g. by inspecting damage accumulation or using capture_pixels
    //    on each output's region).
    // 6. Verify the texture cache has exactly 2 entries (shared across outputs).
}
```

Key assertions:

- `state.output_info.len() == 2` after adding.
- Each output has its own non-overlapping damage.
- `texture_cache.len()` equals the number of unique client buffers (not per-output).
- Both outputs' present calls complete without error.
- Feature-disabled builds still pass all existing single-output tests unchanged.

---

## Migration Notes

### Single output to multi-output: what changes

| Aspect | Single-output (current) | Multi-output (proposed) |
|--------|------------------------|------------------------|
| Viewport | `state.window_width` / `window_height` | Per-output `OutputInfo.size` |
| Layout calculation | Single-pass `prepare_render_scene()` | Per-output `prepare_output_scene()` |
| Render loop | One `bind → render → submit` | Iterate outputs, each with bind/render/submit |
| Damage | Single `output_damage` vec | Per-output `OutputInfo.damage` |
| Texture cache | Shared `LruCache` on `State` | Unchanged — still shared |
| Screencopy | Captures the single output | Need per-output capture (or capture the unified compositor space) |
| Input mapping | Pointer coordinates relative to single output | Pointer (x, y) is in the global compositor space; hit-test against each output's position |
| Workspace tapes | One active tape ("default") | One tape per output name; tape switching logic unchanged |

**Compositor code that references `window_width`/`window_height` directly**
must be audited. Many of these calls are in `focused_output_scale` paths,
fullscreen/layer-surface size calculations, and lock-surface sizing. Each
should be replaced with a query against the relevant `OutputInfo`.

### Cargo feature flag: `multi-output-experimental`

```toml
[features]
multi-output-experimental = []
```

- Default builds (`cargo build`, `cargo test`) do **not** enable this feature.
- All multi-output types and methods are behind `#[cfg(feature = "multi-output-experimental")]`.
- The `primary_output` field provides a feature-agnostic access path so
  existing code does not need conditional compilation.
- CI adds a dedicated job: `cargo build --features multi-output-experimental`
  to prevent bit-rot.

### Backward compatibility: default builds unchanged

- No existing field is moved or renamed.
- `outputs: Vec<Output>` remains as-is for Smithay protocol objects.
- `output_scale_factors` remains as-is.
- `window_width` / `window_height` remain as-is for the single-output path.
- All existing tests pass without modification.
- The only new struct introduced to non-feature builds is `primary_output` (a
  single `OutputInfo`), and it is initialized to match today's defaults.

---

## Acceptance Criteria

- [ ] Feature-guarded code compiles:
  `cargo build --features multi-output-experimental` succeeds with no warnings.
- [ ] Default builds unaffected:
  `cargo build` (no feature flag) produces identical binary size (modulo the
  trivial `primary_output` field) and identical behavior.
- [ ] Tests pass:
  - `cargo test` (no feature flag) — all existing tests pass.
  - `cargo test --features multi-output-experimental` — new multi-output tests
    pass.
- [ ] Integration test verifies two outputs:
  A new test creates two `OutputInfo` entries, assigns a window to each,
  runs a render cycle, and asserts non-overlapping damage + successful
  present on both outputs.
- [ ] Single-output render path is structurally untouched:
  `render()` in the non-feature build is byte-identical to today's
  implementation (the per-output iteration loop is not compiled in).
