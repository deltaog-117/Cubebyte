# minecraft-clone

[![Rust 2024](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://www.rust-lang.org/)

**A voxel sandbox written from scratch in Rust, built around time rewind and code as gameplay.**

---

## 💡 About

This is a Minecraft-style sandbox: build, craft, fight and manage resources. It adds two features of its own. You can rewind time to undo builds, and you can write scripts that drive mobs and machines.

Both depend on a deterministic simulation. Every edit to the world is a logged event, so the same log always rebuilds the same world. That is why the project uses no game engine: it owns its game loop, world, meshing and random numbers.

**Status:** early development. The world model, event log, seeded random numbers and fixed-timestep clock exist. Terrain generation, chunk meshing and a basic renderer exist: `cargo run` opens a window where you walk over generated terrain with WASD, the mouse and Space to jump (`F` switches to free flight with Space and Left Shift; Esc quits). Left click breaks the block under the crosshair, right click places the selected block (1 stone, 2 dirt, 3 grass); every edit goes through the event log. See the [roadmap](ROADMAP.md).

---

## ✨ Features

- 🔹 **Event-only world** – chunks change only by applying events, and every event has an inverse for undo.
- 🔹 **Deterministic by design** – a seeded xoshiro256++ generator with a pinned output stream, and an integer fixed-timestep clock.
- 🔹 **Planned: time rewind** – scrub backward through the event log.
- 🔹 **Planned: code as gameplay** – a sandboxed scripting language with a per-tick instruction budget.

---

## 🧰 Tech Stack

| Layer | Choice |
|---|---|
| Language | Rust (2024 edition) |
| Graphics | [`wgpu`](https://wgpu.rs/) |
| Windowing and input | [`winit`](https://github.com/rust-windowing/winit) |
| Engine | None |
| Targets | Native first, WASM possible later |

`wgpu`, `winit` and `glam` drive the renderer. The reasoning is in [DIARY.md](DIARY.md).

---

## 📋 Requirements

- **Rust** – a toolchain that supports the 2024 edition (1.85 or newer).

---

## 📦 Installation

```bash
git clone <repository-url>
cd minecraft-clone
cargo build
```

---

## 🚀 Usage

The game does not run yet. For now, the library is exercised through its tests and benchmarks.

```bash
cargo bench
```

---

## 📁 Project Structure

```
src/
├── world/       # chunked world and terrain generation
├── events/      # event log, replay and save format
├── building/    # placing and breaking blocks
├── items/       # item table: names, stack limits, what they place
├── inventory/   # stacks, gathering and placing with items, save format
├── crafting/    # recipe table and crafting
├── survival/    # health, hunger and eating
├── combat/      # damage, death, respawning and weapon strength
├── mobs/        # the zombie: chasing, hitting, spawning
├── player/      # player model and input
├── rendering/   # camera, meshing, pipeline
├── simulation/  # tick loop, intent log, session files and replay
├── shared/      # voxel types, events, rng, clock, walking body
├── app.rs       # wiring
└── main.rs      # entry point
tests/integration/   # cross-feature tests
benches/             # criterion benchmarks
```

---

## 🧪 Testing

```bash
# Format check, lint (warnings are errors) and tests
scripts/check

# Tests only
cargo test
```

---

## 📜 Changelog

See [CHANGELOG.md](CHANGELOG.md) for version history.
