# 📘 DIARY.md

## Architectural Decision Log

*A chronological record of decisions, trade-offs and reasoning. Git tracks the code changes; this tracks the **why**.*

---

## 📋 Decision Index

| Date | Decision Area | Choice | Status |
|------|---------------|--------|--------|
| 2026-10-25 | Tech stack | Rust + `wgpu` + `winit`, no engine | ✅ Confirmed |
| 2026-10-25 | Chunk shape | 16×16×16 cubes | ✅ Confirmed |
| 2026-10-25 | Chunk storage | Flat `BlockId` array behind a `Chunk` API | ✅ Confirmed |
| 2026-10-25 | World mutation | Event-only (`World::apply`) | ✅ Confirmed |
| 2026-10-25 | Module layout and tests | 2024-style `name.rs` files; unit tests in-file | ✅ Confirmed |

---

## 📝 Decision Entries

### Tech Stack

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
A Minecraft-like game with building, crafting, combat and resources, plus time rewind and code as gameplay. Rewind and replay need a deterministic simulation that we fully control.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Rust + `wgpu` + Bevy | Mature ECS, fast to a playable game | Scheduler can reorder systems; frequent API breaks | Hard |
| B: Rust + `wgpu` + `winit`, no engine | Full control of loop, order and RNG; native and WASM | Write input, camera, meshing and UI ourselves | Hard |
| C: TypeScript + Three.js | Fastest iteration, shareable by link | GC pauses, slower meshing, determinism needs care | Medium |

#### Decision & Rationale
**Chosen Option:** B.

Determinism and the event log are the core of the project, and owning the loop makes both simpler. Minecraft Alpha was built the same way: a thin platform library (LWJGL) with the game logic written in-house. A voxel game needs little engine.

**Trade-offs accepted:** `wgpu` setup is verbose, and we write input, camera and UI ourselves. A middle path (Bevy for window and rendering only, with simulation in a plain crate) remains possible if rendering becomes a bottleneck.

#### Implementation Notes
- Native first. WASM stays possible because `wgpu` targets WebGPU.
- Add `wgpu`, `winit` and `glam` when rendering starts, not before.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### World and Chunk Module

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
First feature. Everything else (meshing, building, rewind, scripting) reads or writes the world, so its shape and its write path are hard to change later.

#### Options Considered

**Chunk storage**

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Flat `u16` array | Simplest; O(1) access; easy to snapshot and compare | 8 KB per chunk even when uniform | Easy |
| B: Palette + packed indices | 5-10× smaller, same access cost | Bit packing and palette growth | Medium |
| C: Run-length or octree | Smallest for uniform terrain | Slow edits, awkward meshing and replay | Hard |

**Chunk shape**

| Option | Advantages | Disadvantages |
|---|---|---|
| Cube 16³ | Streams on all axes; cheap remeshing; fits local rewind zones | More chunks and borders |
| Column 16×16×256 | Fewer chunks; simple heightmaps | Coarse remeshing and rewind; fixed height |

**World mutation**

| Option | Advantages | Disadvantages |
|---|---|---|
| Event-only | Rewind and replay correct by construction | Event type needed up front |
| Direct, events later | Fastest to a working world | Every missed caller silently breaks replay |

#### Decision & Rationale
**Chosen:** flat array (A), 16³ cubes, event-only mutation.

- Memory is not the bottleneck yet; determinism and the event log are. A flat array is the easiest to snapshot and property-test, and it sits behind `Chunk::get`/`set`, so palette storage can replace it later.
- Cubes suit local rewind zones and snapshots between events.
- Retrofitting an event log is hard, so `World::apply` is the only mutator.

**Trade-offs accepted:** wasted memory on uniform chunks; the event type had to exist from the first commit.

#### Implementation Notes
- `Event` lives in `shared::event`, not in `events`, so `world` and `events` can both use it without depending on each other (the delete test).
- `Event::BlockSet` stores `old` as well as `new`, so undo is inversion with no snapshot. `apply` rejects an event whose `old` does not match the world (`ApplyError::StaleEvent`) and leaves the world unchanged.
- Chunks track a solid-block count, and all-air chunks are removed. A world therefore has one canonical form, so `==` means "same blocks", which replay tests rely on.
- `LocalPos` can only be built in range, so chunk indexing cannot go out of bounds.
- Negative coordinates use floor division (`div_euclid`), covered by property tests.
- `World::apply` logs rejected events with `tracing::warn!`. The success path is not logged because it is a measured hot path (benchmark: about 146 ns for a place plus a remove).

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### Module Layout and Test Placement

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
The scaffold started with `mod.rs` files and empty `tests/unit/` folders mirroring `src/`.

#### Decision & Rationale
- **2024-style modules** (`world.rs` beside `world/`): one entry file per module, as in the Rust etiquette guide.
- **Unit tests in-file** (`#[cfg(test)]`): tests under `tests/` can only see public items, which would force internals public. Cross-module tests live in `tests/integration/`.
- **Integration entry:** `tests/integration/main.rs`, because Cargo resolves `mod` files in a `tests/*.rs` entry relative to `tests/`, not to a matching subfolder.

**Trade-offs accepted:** the empty `tests/unit/` folders are unused for now.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |
