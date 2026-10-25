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
| 2026-10-25 | Random number generator | Own xoshiro256++ with SplitMix64 seeding | ✅ Confirmed |
| 2026-10-25 | Simulation clock | Integer fixed timestep with capped catch-up | ✅ Confirmed |
| 2026-10-25 | Terrain generation | Perlin heightmap, inserted without events | ✅ Confirmed |
| 2026-10-25 | Chunk meshing | Culled faces, one quad per exposed face | ✅ Confirmed |
| 2026-10-25 | Basic renderer | Vertical slice: `wgpu` + `winit` + `glam`, pure camera | ✅ Confirmed |
| 2026-10-25 | Block placing and breaking | Exact voxel ray cast; meshes keyed per chunk, only affected chunks rebuilt | ✅ Confirmed |
| 2026-10-25 | Chunk streaming | Unload everything; reload as generated terrain plus the chunk's logged events | ✅ Confirmed |

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
- The stack is recorded in `README.md` for readers who do not open the diary.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |
| 2026-10-25 | Stack recorded in README.md | deltaog-117 |

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

---

### Deterministic RNG and Fixed Clock

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
Replay, rewind and seeded terrain only work if the same seed and the same tick count always produce the same results, in every future build.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Own SplitMix64 + xoshiro256++ | Stream fixed forever; no dependency; same on native and WASM | We write and test it ourselves | Easy |
| B: `rand_chacha` or `rand_pcg` | Proven, documented stable stream | Dependency tree for little use | Easy |
| C: `rand` `StdRng` | Most convenient | Algorithm may change between versions and silently break saves | Easy |

#### Decision & Rationale
**Chosen Option:** A.

A change to the stream would invalidate every saved world and replay, so it must be impossible to change by accident. The generator is about 40 lines, and tests pin its output for known seeds against vectors from an independent implementation.

**Trade-offs accepted:** no ready-made distributions or shuffles. We add helpers when something needs them.

#### Implementation Notes
- `below(bound)` rejects the biased low range, so results are uniform.
- `fork(key)` takes `&self` and does not advance the parent. Fork from a generator that is never drawn from, keyed by something stable such as a chunk position, so a result never depends on generation order.
- `FixedClock` uses integer nanoseconds, so there is no floating-point drift. Simulation state depends only on the tick count, never on wall time or frame rate.
- Catch-up is capped per advance. After a long stall the extra backlog is dropped and the game runs slow instead of spiralling. A dropped backlog is logged with `tracing::warn!`.
- Benchmarks: about 1.1 ns per `next_u64` and 11 ns per `below`.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### Terrain Generation

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
The world needs seeded terrain that is the same on every run and every platform, and that fits the rule that the world changes only through events.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: 2D heightmap from value noise | No dependency, tiny | Blocky look, no caves | Easy |
| B: 2D heightmap from Perlin noise | Smooth hills, no dependency, can grow into C | More code than A, no caves | Medium |
| C: 3D density noise | Caves and overhangs | About 16× the samples per chunk, more tuning | Hard |

#### Decision & Rationale
**Chosen Option:** B, with generated chunks inserted directly through `World::insert_generated` and kept out of the event log.

Terrain is a pure function of the seed, so logging about 4096 events per chunk would bloat the log and make rewind heavy for no gain. Events are deltas on top of the generated baseline.

**Trade-offs accepted:** `World` is no longer changed *only* by events. Generated chunks are the baseline, and `insert_generated` refuses to overwrite a loaded chunk so edits cannot be lost.

#### Implementation Notes
- The roadmap planned one forked `Rng` per chunk position. Heights must be continuous across chunk borders, so the noise is instead hashed per lattice point from the world seed. `Rng::fork` derives one salt per octave, and the per-chunk fork stays reserved for features such as trees.
- Floats use only `+ - * /` and `round`, which are exactly specified, so results match across platforms. Wavelengths are powers of two so the position inside a cell is exact.
- Heights are clamped to `MIN_HEIGHT..=MAX_HEIGHT` (-8..=40); chunks fully above that return without sampling any noise.
- The tests that pin heights for seeds 0 and 42 are regression pins from this implementation, not independent reference values.
- Benchmarks: about 34 µs per surface chunk and 110 ns per sky chunk.
- Open for replay wiring: replay must regenerate the baseline chunks before applying logged events, since events record the block they replaced.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### Chunk Meshing

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
The renderer needs vertex data for each chunk. Meshing should be testable without a window or a GPU, and should not tie the world to `wgpu`.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Culled faces, one quad per exposed face | Simple, easy to prove, per-face data is easy to add | Most vertices | Easy |
| B: Greedy meshing | Far fewer vertices | Conflicts with per-vertex AO, lighting and per-face textures; harder to prove | Hard |
| C: Culled faces behind a `Mesher` trait | Cheap to swap later | An abstraction with one implementation | Medium |

#### Decision & Rationale
**Chosen Option:** A. Chunks are only 16³, so the vertex count is not a bottleneck yet, and greedy meshing would have to be reworked once AO or lighting arrives. A trait can be added if a benchmark ever shows meshing or draw cost matters.

**Trade-offs accepted:** More vertices than greedy meshing would produce. Flat terrain costs one quad per exposed block face.

#### Implementation Notes
- `mesh_chunk(world, pos) -> MeshData` is a pure function. `MeshData` holds `Vec`s of `Vertex` and `u32` indices with no `wgpu` types, and its fields are private so every mesh is whole quads with in-range indices.
- Vertex positions are relative to the chunk origin (`0.0..=16.0`), which keeps `f32` exact far from the world origin. The renderer adds the chunk offset.
- A border face reads the neighbouring chunk through `World::block_at`. An unloaded neighbour reads as air, so those faces are drawn until it loads. Inside the chunk the array is read directly. Coordinates use checked arithmetic, so a chunk at the edge of the representable world cannot overflow.
- Winding is counter-clockwise from outside. A property test checks that every triangle's cross product equals its face normal.
- The main property test compares the quad count with a count built from `block_at` alone, which shares no code with the mesher's neighbour lookup.
- Open for chunk streaming: when a chunk loads, its loaded neighbours must be remeshed, because their border faces were drawn against air.
- Benchmarks (`--quick` run): about 302 µs per surface chunk and 26 ns per sky chunk. These are the baseline for the 5% gate.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### Basic Renderer

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
Terrain generation and meshing existed but nothing could be seen. Block editing and chunk streaming both need a visible world and a camera to aim with.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Camera maths first, GPU later | Fully testable, small diffs | Nothing on screen for a cycle | Medium |
| B: Vertical slice: window, pipeline, fly camera | Terrain on screen in one cycle | Largest diff; GPU code not unit-testable | Hard |
| C: Headless offscreen render with golden pixels | GPU path verified without a display | Needs a software adapter that may not exist | Hard |

#### Decision & Rationale
**Chosen Option:** B, with the camera kept as pure tested code. It is the only option that gives something to look at, which block editing and streaming both need.

**Trade-offs accepted:** The window, surface and draw calls are verified by hand, not by `scripts/check`. Only the camera, input and vertex conversion are covered by tests.

#### Implementation Notes
- `wgpu` 27 and `winit` 0.30 with `glam` for matrices, `bytemuck` for vertex upload and `pollster` to block on adapter setup. `unsafe_code = "forbid"` still holds, since `bytemuck` derives compile under it.
- `Camera` and `MoveKeys` use only `glam` types. `Renderer::new` takes any `wgpu::SurfaceTarget`, so `rendering` does not import `winit`; `app` is the only place that joins window, input, world and renderer.
- glam 0.34 moved matrix helpers to `glam::camera::rh`; `proj::directx::perspective` gives depth in `0..1` with Y up, which is what `wgpu` clip space expects.
- Lighting is baked into vertex colour on the CPU (block colour times a fixed per-face shade), so the shader only interpolates.
- The chunk grid is fixed (9×9 columns around the origin) and one buffer pair per chunk is created at start. Open for chunk streaming: replace it with load and unload around the camera, and reuse buffers.
- Not checked on screen by the assistant: the app ran 8 s on Wayland without errors, but the picture itself was not inspected.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### Block Placing and Breaking

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
The world could be flown over but not changed. Placing and breaking blocks is the first playable loop, and the first real producer of events for the log that rewind will depend on.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Exact voxel ray cast (DDA) and per-chunk mesh replacement | Exact block and face; small re-mesh cost; reusable for streaming | Changes `Renderer`; border-neighbour logic | Medium |
| B: Fixed-step ray march | Simplest to write | Can skip blocks at grazing angles; face is approximate | Small |
| C: DDA, then re-mesh every chunk per edit | Smallest diff | About 729 chunks rebuilt per click; thrown away by streaming | Small |

#### Decision & Rationale
**Chosen Option:** A. The face the ray crosses decides where a block is placed, and the DDA gives it exactly. Keyed meshes are also what chunk streaming needs.

**Trade-offs accepted:** `Renderer::add_mesh` became `set_mesh(chunk, ..)`, a change to a public function that A required. The window path (clicks, GPU upload) is verified by hand, not by `scripts/check`.

#### Implementation Notes
- `building::raycast::cast` is pure maths over `World::block_at`. It returns the hit block and the cell before it (`None` when the ray starts inside the block), and refuses a zero or non-finite direction, a non-finite reach and an origin beyond one million blocks, so cell arithmetic cannot overflow.
- The ray cast is tested against a slab-test search that shares no code with it. Full-reach miss costs about 81 ns (`--quick` run), the baseline for the 5% gate.
- `building::service` applies the event to the world first and logs it only if the world accepted it, so the log never holds a rejected edit. `SolidBlock` makes placing air unrepresentable.
- Placing is refused in the cell of the viewer's eye. There is no player body yet, so a fuller collision rule waits for physics.
- `chunks_to_remesh(pos)` is the chunk of the block plus the chunks of its six face neighbours. A property test confirms every other chunk's mesh is unchanged by the edit.
- Open for chunk streaming: placing next to an unloaded chunk writes into it through `World::apply`, and `insert_generated` would later refuse that chunk. Streaming must merge generated terrain under existing edits.
- Open for the block registry: hotbar keys 1 to 3 and the block colours are fixed constants.
- Not checked on screen by the assistant: clicking and the picture were not inspected.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### Chunk Streaming

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
The window drew a fixed 9×9 grid of chunks. Streaming loads and unloads chunks around the camera. The open question was what happens to a player's edits in a chunk that unloads: `insert_generated` refuses to overwrite a stored chunk, so edits cannot simply be regenerated over.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Never unload edited chunks | Smallest change; no edit can be lost | Memory grows with edits; edited chunks stay loaded far away | Small |
| B: Park edited chunks in a side store | Bounded loaded set; fast reload | A second copy of the truth beside the log; rewind must patch it | Medium |
| C: Unload all; reload by generating, then replaying the chunk's events | World stays terrain plus events; no extra state; replay and rewind build on it | Needs a per-chunk log index; reload cost grows with that chunk's edits | Medium |

#### Decision & Rationale
**Chosen Option:** C. The world is defined as terrain (a pure function of the seed) plus the event log, and C is the only option that keeps that true. It also covers most of the next roadmap item, event replay wiring.

**Trade-offs accepted:** reload cost grows with the number of edits in one chunk. Snapshots, already planned under time rewind, are the fix if it ever matters.

#### Implementation Notes
- `Streamer` only tracks positions. It keeps its own loaded set because `World` never stores an all-air chunk, so "loaded but empty" and "not loaded" look the same there.
- Distance is square (largest horizontal chunk offset). A chunk unloads one chunk past the load radius, and the starting area loads in full at startup.
- Each `BlockSet` touches one block, so replaying a single chunk's events gives the same blocks the live world held. An integration test unloads and reloads every chunk after random edits and compares the worlds.
- A newly loaded chunk remeshes its loaded neighbours so border faces update. Unloading does not remesh neighbours, so the outer ring keeps hidden faces toward the gap until the chunk reloads.
- Placing a block into an unloaded chunk would log a wrong `old` block. Reach (5 blocks) is far below the load radius (64 blocks), so it cannot happen yet.
- Generation runs on the main thread, capped at 8 chunks per frame. Not checked on screen by the assistant: the streaming picture was not inspected.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |
