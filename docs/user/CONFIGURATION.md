# Configuration

Axiom uses a TOML configuration file.

**Location:** `~/.config/axiom/axiom.toml` (or supply via `--config <path>`)

## Support status

Because Axiom is still in alpha, some settings are informational only. Here is the current status.

### High-value settings that are fully applied

- `window.focus_follows_mouse`
- `input.keyboard_repeat_delay`
- `input.keyboard_repeat_rate`
- `workspace.scroll_speed`
- `workspace.gaps`
- `general.max_fps`

### Settings accepted but not yet wired

- `window.placement`, `window.default_layout`
- `input.mouse_accel`, `input.touchpad_tap`, `input.natural_scrolling`
- `general.vsync`

### Feature flags (decorations)

```toml
[features]
enable_minimize = false        # Show minimize button on titlebar
enable_xdg_decoration_protocol = false  # Register xdg-decoration global
```

- `enable_minimize = true` enables the minimize affordance (button + IPC command).
- `enable_xdg_decoration_protocol = true` registers the xdg-decoration protocol global. When enabled, Axiom negotiates `ServerSide` and renders visible SSD titlebars/buttons.
- The `effects` section is accepted by the parser but effects are no-ops (module removed).

## Example Configuration

```toml
[workspace]
scroll_speed = 1.0
workspace_width = 1920
gaps = 10

[window]
focus_follows_mouse = true

[input]
keyboard_repeat_delay = 600
keyboard_repeat_rate = 25

[bindings]
scroll_left = "Super+Left"
scroll_right = "Super+Right"
close_window = "Super+q"
toggle_fullscreen = "Super+f"
launch_terminal = "Super+Enter"
quit = "Super+Shift+q"

[general]
max_fps = 60
```