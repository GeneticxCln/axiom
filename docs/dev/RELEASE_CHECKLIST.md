# Axiom Alpha Release Checklist

Use this checklist before cutting an alpha release.

## 1. Scope and honesty
- [ ] README status still describes Axiom as an **alpha compositor prototype**
- [ ] Known limitations in `docs/user/LIMITATIONS.md` are current
- [ ] Release notes clearly distinguish:
  - recommended nested/windowed path
  - server-side decorations (live)
  - Wayland clipboard works (tested)
  - DnD/touch compile-verified but need runtime verification

## 2. Build and test gates
- [ ] `cargo fmt -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test --all-features -- --test-threads=1`
- [ ] nested smoke test passes:
  - `xvfb-run -a cargo test --test real_client_smoke`
- [ ] clipboard round-trip passes:
  - `cargo test --test clipboard_round_trip`

## 3. Runtime spot checks
- [ ] `cargo run -- --windowed --debug` starts successfully
- [ ] a real Wayland client maps in nested mode
- [ ] client close removes the mapped window cleanly
- [ ] IPC socket path matches documented behavior (`wayland-axiom-<pid>`)
- [ ] helper clients work against the current IPC schema

## 4. Packaging assets
- [ ] Nested desktop entry references valid icon
- [ ] Wayland session entry launches the compositor

## 5. Documentation sync
- [ ] `README.md` quick-start commands still work
- [ ] `docs/user/INSTALL.md` and `docs/user/RUNNING.md` match current behavior
- [ ] `docs/user/LIMITATIONS.md` reflects current state

## 6. Release publication
- [ ] version number updated in `Cargo.toml`
- [ ] `docs/dev/RELEASE_NOTES_TEMPLATE.md` still matches the current alpha posture
- [ ] git tag created for the alpha release
- [ ] release notes include:
  - summary of user-visible changes
  - recommended evaluation path
  - known limitations

## Current release posture
> **alpha compositor prototype with a strong nested development path**