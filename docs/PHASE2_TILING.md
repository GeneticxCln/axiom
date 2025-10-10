# Phase 2: Advanced Tiling Window Management

## Overview

Axiom now includes a comprehensive tiling window management system with multiple layout modes, intelligent window placement, and powerful keyboard-driven controls.

## Layout Modes

The compositor supports five distinct tiling layouts per workspace column:

### 1. **Vertical** (Default)
- Windows stack vertically top-to-bottom
- Each window gets equal height
- Perfect for reading/browsing workflows

### 2. **Horizontal**
- Windows stack horizontally left-to-right
- Each window gets equal width
- Great for monitoring multiple terminals side-by-side

### 3. **Master-Stack**
- One large master window on the left (50% width)
- Remaining windows stack vertically on the right
- Ideal for code + documentation workflows

### 4. **Grid**
- Windows arranged in an optimal grid pattern
- Automatically calculates best rows/columns
- Excellent for dashboard-style layouts

### 5. **Spiral**
- Fibonacci-style spiral tiling pattern
- Alternates horizontal/vertical splits
- Unique aesthetic for creative workflows

## Keyboard Shortcuts

All keybindings use **Super (Windows key)** as the primary modifier by default:

### Layout Management
- **Super + L**: Cycle through layout modes (Vertical → Horizontal → Master-Stack → Grid → Spiral)

### Window Focus Navigation
- **Super + J**: Focus next window in current column
- **Super + K**: Focus previous window in current column

### Window Movement (Within Column)
- **Super + Shift + J**: Move focused window down in stack
- **Super + Shift + K**: Move focused window up in stack

### Window Movement (Between Columns)
- **Super + Shift + Left**: Move window to left column
- **Super + Shift + Right**: Move window to right column

### Window Swapping
- **Super + Ctrl + J**: Swap focused window with next
- **Super + Ctrl + K**: Swap focused window with previous

### Workspace Navigation
- **Super + Left**: Scroll to previous workspace
- **Super + Right**: Scroll to next workspace

### Other Actions
- **Super + F**: Toggle fullscreen for focused window
- **Super + Q**: Close focused window
- **Super + Shift + Q**: Quit compositor

## Architecture

### New Components

#### 1. `LayoutMode` Enum
```rust
pub enum LayoutMode {
    Vertical,      // Stack vertically
    Horizontal,    // Stack horizontally
    MasterStack,   // Master + stack
    Grid,          // Optimal grid
    Spiral,        // Fibonacci spiral
}
```

#### 2. Enhanced `WorkspaceColumn`
Each column now tracks:
- **`layout_mode`**: Current layout algorithm
- **`split_ratios`**: Window size ratios for resizing
- **`focused_window_index`**: Currently focused window in column

#### 3. Layout Calculation Functions
- `layout_vertical()`: Equal-height vertical stacking
- `layout_horizontal()`: Equal-width horizontal stacking
- `layout_master_stack()`: 50/50 master-stack split
- `layout_grid()`: Optimal rows×columns arrangement
- `layout_spiral()`: Alternating H/V fibonacci splits

#### 4. Window Management Operations
- `cycle_layout_mode()`: Switch between layout modes
- `focus_next_window_in_column()`: Focus management
- `focus_previous_window_in_column()`: Focus management
- `move_focused_window_up()`: Reorder windows
- `move_focused_window_down()`: Reorder windows
- `swap_windows_in_column()`: Direct window swapping

### Integration Points

#### Input System (`src/input/mod.rs`)
New `CompositorAction` variants:
- `CycleLayoutMode`
- `FocusNextWindow`, `FocusPreviousWindow`
- `MoveWindowUp`, `MoveWindowDown`
- `SwapWindowUp`, `SwapWindowDown`

#### Configuration (`src/config/mod.rs`)
New `BindingsConfig` fields with sensible vim-style defaults:
- `cycle_layout`: "Super+L"
- `focus_next_window`: "Super+J"
- `focus_previous_window`: "Super+K"
- `move_window_up`: "Super+Shift+K"
- `move_window_down`: "Super+Shift+J"

#### Compositor (`src/compositor.rs`)
Action handlers integrated into event loop for immediate visual feedback.

#### Smithay Server (`src/smithay/server.rs`)
Real-time layout recalculation on keyboard events with proper window repositioning.

## Usage Examples

### Example 1: Code Review Workflow
1. Open editor in first column (auto-vertical layout)
2. Press **Super + Right** to create new column
3. Open documentation browser
4. Press **Super + L** to cycle to **Master-Stack** layout
5. Editor now fills left half, browser on right

### Example 2: Multi-Terminal Dashboard
1. Open 4 terminal windows
2. Press **Super + L** twice to switch to **Grid** layout
3. All terminals arranged in 2×2 grid automatically

### Example 3: Focus Management
1. In vertical stack with 3 windows
2. Press **Super + J** to focus next window down
3. Press **Super + K** to focus back up
4. Press **Super + Shift + J** to move focused window down in stack

## Technical Details

### Gap Handling
All layouts respect the configured gap size (`workspace.gaps` in config):
- Applied between windows
- Applied at edges
- Maintained during layout transitions

### Reserved Space
Layouts automatically account for:
- Top/bottom bars (layer-shell panels)
- Left/right reserved zones
- Output-specific exclusive zones

### Animation Support
Layout transitions are smooth:
- Position changes are interpolated
- Size changes use easing functions
- No jarring instant repositions

### Performance
- Layouts calculated only when needed (on change events)
- Efficient HashMap-based window tracking
- O(n) complexity for most operations

## Configuration

Add to your `~/.config/axiom/config.toml`:

```toml
[bindings]
# Layout management
cycle_layout = "Super+L"

# Focus navigation
focus_next_window = "Super+J"
focus_previous_window = "Super+K"

# Window movement
move_window_up = "Super+Shift+K"
move_window_down = "Super+Shift+J"

# Window swapping
swap_window_up = "Super+Ctrl+K"
swap_window_down = "Super+Ctrl+J"

[workspace]
gaps = 10  # Gap size in pixels
```

## Future Enhancements

Potential additions for future phases:
- [ ] Dynamic split ratio adjustment (window resizing)
- [ ] Per-window floating toggle
- [ ] Tabbed container mode
- [ ] Saved layout presets
- [ ] Multi-monitor layout sync
- [ ] Directional focus (vim-style hjkl)
- [ ] Custom layout algorithms via scripting

## Testing

The implementation has been fully integrated and compiles cleanly. To test:

1. **Build the compositor:**
   ```bash
   cargo build --release
   ```

2. **Run the compositor:**
   ```bash
   ./target/release/axiom
   ```

3. **Test layout cycling:**
   - Launch multiple windows
   - Press **Super + L** to cycle through layouts
   - Observe window repositioning

4. **Test window focus:**
   - Press **Super + J/K** to navigate between windows
   - Focused window should be highlighted

5. **Test window movement:**
   - Press **Super + Shift + J/K** to reorder windows
   - Windows should swap positions

## Credits

- Inspired by tiling window managers: i3, sway, dwm, xmonad
- Niri-style infinite scrolling workspace concept
- Fibonacci spiral layout based on bspwm

## License

Part of the Axiom Wayland compositor project.
