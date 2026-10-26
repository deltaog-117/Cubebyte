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
| 2026-10-25 | Block registry | Static `const` table indexed by `BlockId` | ✅ Confirmed |
| 2026-10-25 | Simulation tick loop | `Simulation` owns the `FixedClock` and forks per-system `Rng` streams; no systems yet | ✅ Confirmed |
| 2026-10-25 | Resource management | Item registry plus a snapshot-saved inventory; block events unchanged; stats on the sim step | ✅ Confirmed |
| 2026-10-26 | Player physics | `f32` swept box on the tick, stopping a skin short of blocks; player only | ✅ Confirmed |

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

---

### Event Replay Wiring

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
Edits lived only in memory, so closing the window lost them. Per-chunk replay already rebuilt a chunk from terrain plus its logged events, so the missing piece was getting the log to disk and back through `app`.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Hand-rolled binary file | No dependency; compact; format fully pinned | Encode, decode and corruption handling are ours | Medium |
| B: Plain-text lines | Human-readable and diffable | Larger; slower to parse; strict parsing needed | Small |
| C: `serde` + `bincode` | Least code | Extra dependencies; format not pinned by us | Small |

#### Decision & Rationale
**Chosen Option:** A. Replay has to stay deterministic, and the project already pins its own random stream for the same reason. An event is small, and the seed in the header catches a save made on different terrain.

**Trade-offs accepted:** the format is ours to maintain; a layout change needs a version bump and a migration.

#### Implementation Notes
- Layout: magic `MCEL`, version `u16`, seed `u64`, then 17-byte records, little-endian. There is no stored count: records run to the end of the file, so a file cut mid-record is rejected instead of silently dropping the last edit.
- `save` writes `world.mcel.tmp` and renames it over `world.mcel`, so a crash cannot leave a half-written save.
- `app` loads the log before the first chunk streams in. A save with a different seed is an error, not overwritten. The log is written on exit, but only if it is non-empty.
- The save is written on exit only. A crash or kill loses the edits since the last clean exit.
- The path is relative to the working directory (`world.mcel`). The repo has no `.gitignore` yet, so that file shows up as untracked after a run.
- Not run in the window by the assistant: save and restart were checked through tests only (round trip, truncation, bad magic, bad version, unknown tag, arbitrary bytes, and a restart simulated from the bytes).

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### Block Registry

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
Block data was spread over `BlockId` constants, a colour `match` in the renderer, `SolidBlock` and the key bindings. Crafting and inventory will need more per-block properties, so they needed one place to live.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Static `const` table | No dependency; no allocation; saved IDs and the `.mcel` format unchanged | New block means editing Rust | Small |
| B: Runtime `BlockRegistry` | Open to data-driven blocks | IDs depend on registration order; passed through many call sites | Medium |
| C: `blocks.toml` loaded at startup | No recompiling to add a block | New dependencies, load failures, save-format change | Large |

#### Decision & Rationale
**Chosen Option:** A. It removes the scattered data without touching the save format. `world.mcel` already stores raw IDs, so the ID stays the stable key. The lookup API (`get`, `color`, `is_solid`) has the shape a runtime or file-backed registry would need, so upgrading later does not change callers.

**Trade-offs accepted:** adding a block is a code change and a recompile; modding is out of scope for now.

#### Implementation Notes
- `blocks/registry.rs` holds `BLOCKS`; an ID is its index. New blocks are only appended, because reordering would change what saved worlds mean.
- `shared` cannot import a feature, so the `BlockId::STONE`-style constants stay in `shared::voxel`. A test checks each one against its table entry.
- An ID outside the table has no definition: `get` returns `None`, `color` returns magenta and `is_solid` is false.
- Behaviour change: `SolidBlock::try_new` used to accept any non-air ID and now accepts only defined solid blocks.
- The 1/2/3 key bindings in `app` still name `SolidBlock` constants directly; they move when the inventory arrives.
- Not run in the window by the assistant: colours are covered by tests, not by looking at the screen.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### Simulation Tick Loop

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
`FixedClock` and `Rng` existed and were tested, but the game loop used only `Instant` and a variable frame time. Physics, mobs and rewind all need one place that owns simulation time and randomness, before any of them exist.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: `Simulation` with clock and forked streams | Smallest change; no behaviour change; the hook later systems plug into | Nothing runs on a step yet | Small |
| B: A plus an ordered pipeline of system functions | System order fixed explicitly | An abstraction with zero systems is speculative | Medium |
| C: A plus a fixed-rate camera with render interpolation | Player motion becomes replayable | Adds render state and changes how the camera feels | Medium |

#### Decision & Rationale
**Chosen Option:** A. It meets the roadmap item without designing structure we cannot validate until a real system exists. When there are two systems to order, the step becomes a pipeline (B).

**Trade-offs accepted:** the tick advances but nothing reads it yet, so the wiring is covered by tests rather than by visible behaviour.

#### Implementation Notes
- `simulation/model.rs` holds `Simulation { clock, root }`. `root` is only ever forked, never drawn from, so `stream(key)` depends on the key alone and not on call order.
- `app` runs at 20 steps per second with at most 5 steps per frame. `Instant` stays in `app` as the source of elapsed time; the camera and streaming still use the variable frame time.
- A property test shows that splitting the same real time into different frames gives the same tick and the same stream for a key.
- Not run in the window by the assistant: the change has no visible effect.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### Resource Management

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
Blocks could be broken and placed for free. Crafting and combat both need items that are not blocks (tools, drops, food), and survival needs something to spend and to lose, so items, a pack to hold them and basic stats had to exist first.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Inventory as plain `player` state, counts adjusted beside the edit | Smallest change; no save-format change | Not in the log; items are just blocks | Small |
| B: Inventory changes as events in the log | Rewind and replay restore items | Inventory events have no chunk, which the per-chunk log index assumes; log format bump | High |
| C: Item registry plus inventory state saved as a snapshot beside the log | Items separate from blocks; log and block events untouched | Inventory and world can drift apart on rewind until snapshots are tied to log positions | Medium |

#### Decision & Rationale
**Chosen Option:** C. Crafting and combat need items that are not blocks, so the registry was needed anyway, and C avoids B's chunk-less events. Event-sourcing the inventory is left to the rewind milestone, where snapshots will be designed on purpose.

**Trade-offs accepted:** after a crash the world log and the inventory snapshot may disagree, since the inventory is written only on a clean exit. A player can also lose or gain items relative to the world if either file is deleted alone.

#### Implementation Notes
- `items::registry` mirrors `blocks::registry`: a `const` table, append-only IDs, with `ItemId(0)` meaning no item. Each item names the block it places, and `drop_of(block)` is the reverse lookup.
- `Stack` can only be built with 1 to the item's limit, so an empty or overfull stack is unrepresentable. `Inventory` fills lowest slots first and tops up a partial stack before opening a slot, so the layout depends only on the order of the adds.
- `gather_block` and `place_from_stock` check everything that can refuse (no room, no item) before the world changes, so a refused edit leaves the world, log and pack as they were. Placing spends the item only after the placement succeeded.
- Health and hunger step once per simulation step from a step counter, with no randomness and no wall-clock time. Starvation stops at 1 health because nothing can heal or kill the player yet.
- Stats are not saved yet; a restart gives full meters. There is no HUD, so the window title shows the stats and the counts.
- Hunger only drains: there is no food until crafting. The new files are covered by property tests; the window itself was not run by the assistant.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

---

### Crafting

**Date:** 2026-10-25
**Status:** Confirmed

#### Context / Background
Hunger only drained, and the only items were the blocks themselves. Crafting needed items that are not blocks, a way to turn one set of items into another, and a food source so the hunger meter has a purpose.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Recipe table with hotkey crafting | Same shape as the registries; no GUI; easy to test | No on-screen recipe list; shapeless recipes only | Small |
| B: Recipe table plus an in-window crafting panel | Matches "crafting interface" | The renderer has no 2D or text pass yet, so it is a large detour | High |
| C: Recipe table plus a shaped 2x2 or 3x3 grid | Closest to Minecraft; position-dependent recipes | Needs a grid model, a matcher and a UI | High |

#### Decision & Rationale
**Chosen Option:** A. It delivers the recipe system and the food in the smallest change that fits the existing structure. C is on the roadmap for later, once there is a UI pass to build it on.

**Trade-offs accepted:** the player works from the window title instead of a recipe list, and recipes cannot depend on layout.

#### Implementation Notes
- `crafting::registry` is a `const` table like the item and block registries; a recipe's ID is its position, so new recipes are only appended.
- `craft` works on a clone of the inventory and replaces it on success, so a short input or a full pack leaves the inventory exactly as it was. The output may use a slot the inputs just freed.
- Food is `ItemDef::restores_hunger`. `eat` takes the first food item in the lowest slot, and refuses when hunger is full so nothing is wasted.
- The first recipes use only what the terrain gives: grass makes bread, grass and dirt make stew. Values are placeholders to tune by play.
- Keys: `R` next recipe, `C` craft, `E` eat. The window itself was not run by the assistant; the logic is covered by unit and property tests.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-25 | Initial entry | deltaog-117 |

### Player Physics

**Date:** 2026-10-26
**Status:** Confirmed

#### Context / Background
The player was a free-flying camera moved by frame time. Combat and mobs need a body that falls, lands and is stopped by blocks, and replay and rewind need it to move only on the simulation tick.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: `f32` player box | Smallest change, reuses `World` and the block registry | Decimal maths; yaw trigonometry can differ between machines | Small |
| B: Fixed-point player box | Exact positions on every machine | New type and conversions; yaw still needs quantizing | Medium |
| C: General body list | Mobs reuse the step | Entity ordering designed before a second entity exists | Medium |

#### Decision & Rationale
**Chosen Option:** A, with `step` working on the box and a plain `Intent`, so it can grow into C when mobs arrive. Basic `f32` arithmetic is deterministic in practice; the real weak spot, the yaw trigonometry, is the same in B.

**Trade-offs accepted:** the world is walkable only within 8192 blocks of the origin (beyond it the player stops, because `f32` can no longer resolve the collision gap); `Intent` carries a world-space direction made from yaw by the app, so a saved replay will have to record it as whole numbers first. Fixed-point (B) is the way out of both.

#### Implementation Notes
- The box moves along Y, then X, then Z, each clipped to the nearest solid cell in its path, so it slides along walls and a 2.5-block fall step cannot skip a layer.
- It stops 0.005 short of a face. Rounding could otherwise leave it a hair inside, where the next sweep would no longer see the block and the player would fall through.
- Cells are tested with open spans, so a box resting exactly on a face does not overlap the block beside it.
- A box already inside a block (a block placed on the player, or landing from flight inside terrain) is not pushed out but can move out.
- Unloaded chunks read as air, so the player falls through terrain that has not streamed in yet; the start area is loaded up front.
- `Player::eye(alpha)` blends the last two steps, with `alpha` from `Simulation::alpha`, so 20 steps per second do not make the view stutter.
- The window was not run by the assistant; the logic is covered by unit and property tests. One step costs about 210 ns on terrain.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-26 | Initial entry | deltaog-117 |
