# DRM Hardware Validation Matrix

## Tested Configuration

| Component | Detail |
|-----------|--------|
| GPU | NVIDIA AD106 (RTX 4060 Ti) |
| Driver | nvidia (proprietary) + nvidia_drm.modeset=1 |
| Connection | DisplayPort-3 @ 3840x2160, 141 DPI |
| Host Session | Wayland (parent compositor) |

## Phase 3.1 — Seatd/libseat ✅

- [x] `libseat::Seat` opened successfully through logind backend
- [x] seatd daemon active, socket at `/run/seatd.sock`  
- [x] User in `seat` + `video` groups
- [x] `LibinputDevice` session-aware (opens devices through seatd when available)
- [x] `Card::from_fd()` constructor for session-managed fds

## Phase 3.2 — Single-output DRM ❌ (not tested on VT)

- [ ] KMS connector/CRTC/encoder enumeration — verified works (Phase 2 test)
- [ ] GBM surface creation (3840x2160 XRGB8888) — verified works  
- [ ] DRM master acquire — blocked by parent compositor
- [ ] Page-flip with composited frame — needs dedicated VT

## Phase 3.3-3.8 — Multi-output, hotplug, VT, HiDPI

- [ ] Multi-monitor — needs second display
- [ ] Hotplug add/remove — needs physical hotplug or udev test
- [ ] VT switching — needs TTY access
- [ ] Fractional scaling — needs HiDPI display
- [ ] Session restore — needs seatd session management

## Setup Required for Full Validation

1. `sudo systemctl enable --now seatd` ✅
2. Add user to `seat` group ✅
3. Switch to VT3 (`chvt 3` or Ctrl+Alt+F3) and log in
4. Run: `axiom --config /etc/axiom/axiom.toml` (with `kind = "drm"`)
5. Verify display renders, keyboard/mouse work
6. Test multi-monitor if available
7. Test VT switching (Ctrl+Alt+F{1..7})
