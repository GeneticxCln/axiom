# Configuration

Axiom uses a TOML configuration file.

**Location:** `~/.config/axiom/axiom.toml`

## Support status

Because Axiom is still in alpha, not every parsed setting is fully wired into runtime behavior yet.

### High-value settings that are currently applied
- `window.focus_follows_mouse`
- `input.keyboard_repeat_delay`
- `input.keyboard_repeat_rate`
- `workspace.scroll_speed`
- `workspace.gaps`
- `effects.enabled`
- `effects.blur.radius`
- `general.max_fps`

### Settings that are accepted but still incomplete or informational in some paths
Examples include:
- `window.placement`
- `window.default_layout`
- `input.mouse_accel`
- `input.touchpad_tap`
- `input.natural_scrolling`
- `general.vsync`

### Decoration-related feature flags

Axiom currently exposes two feature-gated decoration toggles in the config:

```toml
[features]
enable_minimize = false
enable_xdg_decoration_protocol = false
```

Important current behavior:
- `enable_minimize = true` enables the compositor-side minimize affordance.
- `enable_xdg_decoration_protocol = true` can register the xdg-decoration protocol global.
- **However, enabling the decoration protocol does not mean Axiom renders visible live server-side decorations yet.**
- In the current alpha, the compositor still negotiates **client-side decorations** in the live runtime path.

For the full field-by-field matrix, see:
- [Developer config support matrix](../dev/CONFIG_SUPPORT.md)

## Example Configuration

```toml
[workspace]
scroll_speed = 1.0
infinite_scroll = true
auto_scroll = true
workspace_width = 1920
gaps = 10
smooth_scrolling = true

[effects]
enabled = true

[effects.animations]
enabled = true
duration = 300
curve = "ease-out"
workspace_transition = 250
window_animation = 200

[effects.blur]
enabled = true
radius = 10
intensity = 0.8
window_backgrounds = true

[effects.rounded_corners]
enabled = true
radius = 8
antialiasing = 2

[effects.shadows]
enabled = true
size = 20
blur_radius = 15
opacity = 0.6
color = "#000000"

[window]
placement = "smart"
focus_follows_mouse = false
border_width = 2
active_border_color = "#7C3AED"
inactive_border_color = "#374151"
gap = 10
default_layout = "horizontal"

[input]
keyboard_repeat_delay = 600
keyboard_repeat_rate = 25
mouse_accel = 0.0
touchpad_tap = true
natural_scrolling = true

[bindings]
scroll_left = "Super+Left"
scroll_right = "Super+Right"
move_window_left = "Super+Shift+Left"
move_window_right = "Super+Shift+Right"
close_window = "Super+q"
toggle_fullscreen = "Super+f"
launch_terminal = "Super+Enter"
launch_launcher = "Super+Space"
toggle_effects = "Super+e"
quit = "Super+Shift+q"

[xwayland]
enabled = true

[general]
debug = false
max_fps = 60
vsync = true
```
