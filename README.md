# Evolvarium

Artificial-life sim. GA grows blob bodies + brain shape, brains learn during life, a multi-nutrient ecosystem drives selection. Design spec lives in `../clients/evolvarium/` (`00-concept.md` .. `11-crate-stack.md`). This crate is the implementation, currently at milestone **M0** (skeleton).

## M0 scope (what works now)

- 3D world (bounded ground plane), Bevy + ECS.
- Free-fly camera: hold **right mouse** to look, **WASD** move, **Q/E** down/up, **Shift** sprint.
- A `GravityField` (the reusable field abstraction) pulls 25 blobs down; they fall and rest on the floor.
- Fixed-timestep sim, decoupled from frame rate.
- **Headless mode**: same sim, no window, runs fast, logs average blob height, exits. Proves the sim is independent of rendering (the basis for fast-forwarding generations later).

Not yet: genomes, brains, growth, metabolism, god panel. See `08-roadmap.md`.

## Prerequisites

Rust toolchain (not installed on this machine yet). Install:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# then restart shell or: source "$HOME/.cargo/env"
```

Linux also needs the usual Bevy system deps (Vulkan/X11/Wayland, alsa, udev). On Debian/Ubuntu:

```bash
sudo apt install g++ pkg-config libx11-dev libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev libvulkan-dev
```

## Run

```bash
# Render mode (window, fly around, watch blobs fall):
cargo run

# Headless mode (no window, fast, logs + exits):
cargo run -- --headless

# Faster compiles during dev (optional, dynamic linking):
cargo run --features bevy/dynamic_linking
```

First build pulls + compiles Bevy, so it is slow (minutes). Later builds are fast.

## Notes / known risks

- Bevy is pinned to `0.18` in `Cargo.toml` (do not float; ecosystem crates gate the version, see `11-crate-stack.md`). If `cargo` resolves a newer Bevy with API changes, a few call sites may need fixups (most likely: mouse-input API in `camera.rs`, `EventWriter::write` in `sim.rs`, `ScheduleRunnerPlugin` path in `main.rs`).
- This scaffold was written without a local compiler to verify it. Run `cargo run`, paste any compiler errors back, and they get fixed fast.
- Headless currently steps in real wall-clock time via `ScheduleRunnerPlugin`. True deterministic manual-step fast-forward arrives with the determinism work in `09-open-questions.md`.

## Layout

```
src/
  main.rs        app setup, mode flag, scene dressing
  camera.rs      free-fly camera (render mode)
  components.rs  Velocity, Blob marker
  fields.rs      GravityField (the field abstraction, reused later)
  sim.rs         fixed-timestep gravity + integrate, blob spawn, headless reporter
```
