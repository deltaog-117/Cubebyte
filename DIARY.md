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
| 2026-10-27 | Combat rules | Damage, death and respawn first, hostile mobs in a separate iteration | ✅ Confirmed |
| 2026-10-27 | Hostile mobs | One zombie on a shared walking body that takes its solid test as a closure | ✅ Confirmed |
| 2026-10-28 | Movement intent | Bodies move by whole-number intent; per-actor in-memory log; no saving yet | ✅ Confirmed |
| 2026-10-28 | World interface for scripts | Every script effect is an event in the one log; checked commands run behind permissions, range and a per-tick limit | ✅ Confirmed |
| 2026-10-29 | Bounded log and redo | Fold old actions into a per-chunk base of final blocks, keep 256 rewindable; redo from an in-memory stack | ✅ Confirmed |
| 2026-10-30 | Saved intent tracks | Own `MCIT` file of runs (one intent held over consecutive ticks); written on exit, replacing the last session's | ✅ Confirmed |
| 2026-10-30 | Streamed intent tracks | Settled entries appended to a numbered session file during play; same v1 format, memory bounded | ✅ Confirmed |
| 2026-10-31 | Kept intent sessions | Keep the newest 20 numbered files, pruned at startup | ✅ Confirmed |
| 2026-10-31 | Recorded actions | Intentions (target block, recipe, eat) in the same session file, version 2 | ✅ Confirmed |
| 2026-11-01 | Meters under events | Delta events with the points really moved; passive changes logged but skipped by rewind; respawn is a barrier | ✅ Confirmed |
| 2026-11-02 | Mobs under events | A `Mobs` store with ids; spawn, remove and health-change events through the one door; position is not checked | ✅ Confirmed |
| 2026-11-02 | Redo across restarts and script reads | Redo is its own action kind and the stack is rebuilt from the log; reads are a separate `ask` call that cannot change the game | ✅ Confirmed |

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

---

### Combat Rules

**Date:** 2026-10-27
**Status:** Confirmed

#### Context / Background
Combat is too big for one iteration: it needs damage rules, something to fight and a way to fight it. Health could not reach zero, because starvation stopped at 1 until death existed.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Rules only (damage, death, respawn, sword) | Small, fully property-testable, nothing renderer-side | Nothing to fight; only starvation can kill | Medium |
| B: Rules plus one hostile mob | Playable fight; proves deterministic mob AI | About 3x the code; touches physics, ray cast and renderer | Hard |

#### Decision & Rationale
**Chosen Option:** A first, then B as the next iteration. B needs the same damage and death rules, and splitting keeps the risky part (mob AI on the tick) in its own change.

**Trade-offs accepted:** the only way to die for now is starvation, and the sword exists but cannot be obtained until a recipe or the mob drops add one.

#### Implementation Notes
- New `combat` feature: `Damage` (never 0), `hit` (reports `Survived` or `Died`), `respawn` and `strike` (best weapon held, else a fist).
- `Stats::hurt` saturates at 0 and `Stats::is_dead` is health 0; `STARVE_FLOOR` was removed, so starvation kills.
- Death resets the stats and empties the inventory, which is saved on exit as empty. The player returns to the spawn point and leaves free flight; the window title counts deaths.
- The sword is appended to the item table (ID 6), so saved inventories stay valid.
- The window was not run by the assistant; the logic is covered by unit and property tests.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-27 | Initial entry | deltaog-117 |

---

### Hostile Mobs

**Date:** 2026-10-27
**Status:** Confirmed

#### Context / Background
The zombie needs a body that falls, walks and stops at blocks, as the player's does. The question was where that movement code lives.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Separate mover for mobs | Fastest, no edits to existing code | Duplicate collision code that can drift from the player's | Small |
| B: Shared body module | One tested set of rules, clean boundaries, more mobs cost a size | Refactors tested player code | Hard |
| C: Zombie is a `Player` | Same physics, no refactor | Mobs import `player` (breaks the Delete Test); every mob is player-sized | Small |

#### Decision & Rationale
**Chosen Option:** B. Correctness and fit with existing code decide it: the collision rules are subtle and already property-tested, and `shared` must not import a feature, so the body takes the solid test as a closure instead of a `World`.

**Trade-offs accepted:** `Player` now wraps a `Body`, so one more layer sits between the player and its position. The player's own unit tests did not change, and a step still costs about 181 ns (181.1 before, 181.8 after).

#### Implementation Notes
- `shared::body` holds `Body`, `Intent`, `step` and `collides`; `player::service` re-exports `Intent` and the speed constants, so its public paths are unchanged.
- The zombie draws no random numbers while it lives. Only its spawn point comes from a stream forked with the tick mixed into the key, so each spawn differs and a replay repeats them.
- It walks straight at the player with no path finding, jumps when a block is in front of its knees, and is dropped when 48 blocks away or below the world, since distant chunks unload and read as air.
- Hits use `ray_box` against the zombie's drawn box, and count only if no block is nearer. A hit does not knock the zombie back.
- Free flight leaves the walking player where it was, so the zombie still chases that spot.
- The window was not run by the assistant; the logic is covered by unit, property and integration tests.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-27 | Initial entry | deltaog-117 |

---

### Movement Intent

**Date:** 2026-10-28
**Status:** Confirmed

#### Context / Background
Replays need each actor's wanted movement per tick stored exactly. The player's was two `f32` values built from the camera's `sin` and `cos`, and the zombie's came from `normalize`; neither is guaranteed to be the same bits on another machine, so a saved replay could drift.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Shadow log of rounded floats | Smallest change, physics untouched | The log is not what ran, so replay is not exact | Small |
| B: Physics consumes whole numbers | Exact replay, one type for the player and mobs | Shared `Intent` changes; direction and speed shift slightly | Medium |
| C: Record keys and an integer view angle | Replay can also drive the camera | Needs an integer sine table and a separate form for mobs | Hard |

#### Decision & Rationale
**Chosen Option:** B. Only B is both exact and shared by the player and the zombie. C's extra reach (replaying the camera) is not needed yet and can be added on top of B later. Saving was left out on purpose: a file format is hard to undo and nothing reads it until replays exist.

**Trade-offs accepted:** walking direction is rounded to 127 steps per axis, about 0.45 degrees, and the zombie's half-speed walk is 64/127 of full speed, not exactly half. A player step costs the same (180.5 ns, no change beyond noise).

#### Implementation Notes
- `Walk` has private fields and `-128` is raised to `-127`, so left and right are symmetric and no out-of-range value exists. `from_direction` cuts length to 1 before rounding, and a non-finite direction stands still.
- A diagonal of whole axes such as `(90, 90)` is a hair above length 1, so the body still cuts it to 1 and a diagonal is never faster.
- The log keeps one track per actor, not one global list. Within a frame the zombie is stepped for all steps and then the player, so ticks are only ordered inside a track; recording refuses a tick that is not after the actor's last one.
- The zombie is split into `decide` (intent from what it sees) and `step_with` (move by a given intent and swing). `step` is the two in a row, so existing callers are unchanged. The hit check still uses where the zombie stood before it moved.
- Each new zombie gets a new `ActorId`, so a track follows one body. The log grows by about 2 MB an hour with one zombie, which is why saving also needs a bound.
- The window was not run by the assistant; replay exactness is covered by property tests for the player and a unit test for the zombie.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-28 | Initial entry | deltaog-117 |

### Crafting Interface

**Date:** 2026-10-28
**Status:** Confirmed

#### Context / Background
Crafting was driven by keys R, C and E with the window title as the only display. The renderer had no way to draw flat shapes or text, so a recipe list needed a 2D drawing path and some way to show words.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Built-in pixel font plus flat quads | No new dependency, readable, layout is pure code | Blocky look, hand-written font | Medium |
| B: Font library (e.g. `glyphon`) | Best-looking text, scales to later UI | New dependency tied to `wgpu` 27, heavy | Medium |
| C: Colour-only panel | Smallest change | Names unreadable, thrown away later | Small |

#### Decision & Rationale
**Chosen Option:** A. It is the only option with readable text and no new dependency, and it keeps placement and click-testing as pure arithmetic that property tests cover without a GPU. B can replace the font later without touching the layout.

**Trade-offs accepted:** only capitals, digits, space, `-` and `,` exist in the font; other characters are drawn as gaps.

#### Implementation Notes
- Tab toggles the panel. While it is open the pointer is released, mouse look and block clicks pause, and a left click on a row crafts that recipe. Escape closes the panel before it quits. R, C and E still work.
- `ui::layout::panel` is pure: window size and inventory in, rows with rectangles out. Text scale follows window height but is capped so the widest line always fits the width; a property test found the first version overflowing a 320-pixel-wide window.
- `ui::draw` writes clip-space vertices with the existing `GpuVertex`, so the renderer only gained a second pipeline (no depth, no culling) and `set_overlay`.
- `can_craft` runs `craft` on a copy, so the panel and the real craft cannot disagree, including the full-pack case.
- The sword recipe already existed, so no recipe was added. It is the last row, not the first: recipe IDs are table positions, and reordering was not worth the churn.
- The window was not run by the assistant; the layout, font and overlay are covered by unit and property tests only.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-28 | Initial entry | deltaog-117 |

### World Interface for Scripts

**Date:** 2026-10-28
**Status:** Confirmed

#### Context / Background
Scripts will be able to move the player, give items, set blocks and later spawn mobs. For rewind and replay to stay exact, each of those must be recorded. Only block edits were: the player, the one zombie and the inventory were loose fields in `app`, and the inventory was saved as a snapshot.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Command layer, only blocks logged | Fastest, no format change | Rewind and replay inexact for everything but blocks | Small |
| B: Extend `Event`, one history | One exact history; supports undo and region rewind | Largest refactor; save format changes | Hard |
| C: Separate per-tick command log | No change to the block log or file | Two rewind mechanisms; no undo of entities in a region | Medium |

#### Decision & Rationale
**Chosen Option:** B, delivered in slices. A breaks the roadmap rule that every action is logged, and C would have left two ways to rewind. B is the only one that keeps one ordered history and can undo a single region later.

**Trade-offs accepted:** a grant logs one event per slot it touches, so the log grows a little faster than one event per command. This slice covers only teleport, give item and set block. Mobs, reads, costs and the other event-less changes come in later slices, so rewind still cannot undo everything.

#### Implementation Notes
- `Event` gained `PlayerMoved` and `SlotSet`. Positions are stored as `f32` bits (`Spot`), which makes events `Eq` and makes undo restore the exact position. `Event::chunk` is now an `Option`, so events with no chunk stay out of the per-chunk index that streaming replays from.
- Inventory changes first went in as "items added" and "items removed" with a count. A property test found that undo then restores the totals but not which slot holds what, and the shifted layout could make a later undo fail for lack of room. Replacing them with `SlotSet` (slot, old stack, new stack), the same shape as `BlockSet`, makes replay and undo exact. A grant that spans several slots is several events, applied all or none by `apply_all`.
- `events::apply::apply` is the one door. It takes a `Stage` (world, player, inventory) and refuses a stale or impossible event without changing anything: a move from the wrong spot, a stack that does not fit whole, more removed than held.
- `scripting::service::run` checks permission, then the per-tick limit, then range, builds the event, applies it, and only then logs it, so the log never holds a refused event. Denied commands count toward the limit, so they cannot be spammed for free. The range of 32 blocks stays inside the streamed area (radius 4 chunks), because an edit in an unloaded chunk would make streaming refuse to load its terrain later.
- A teleport is checked with a body of the player's size, so it cannot land inside solid blocks, and it drops velocity so the player does not keep falling from where they were.
- The file format is version 2 with a record per new event; version 1 files read unchanged, which keeps existing saves loading.
- Known gap: the inventory is still saved as a snapshot, so replaying slot events at startup would apply a grant twice. Nothing in `app` produces these events yet, so nothing replays them.
- No benchmark was added: there is no hot loop, since a script gets at most 8 commands per tick. `EventLog::push` gained one branch, and the existing chunk benchmarks are unaffected in kind.
- The window was not run by the assistant: nothing in `app` calls the new feature yet. It is covered by unit, property and integration tests, including a save-and-replay of a scripted session.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-28 | Initial entry | deltaog-117 |

### Gameplay Under Events

**Date:** 2026-10-28
**Status:** Confirmed

#### Context / Background
Scripts changed the pack through logged events, but a left click, a craft and a meal changed it directly. Rewind could not see those, so a replayed or rewound world would drift from the pack.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Diff the pack after each action | Smallest change, rules untouched | Log is a copy, no single door for gameplay | Small |
| B: Rules return events, one door applies | One path for clicks and scripts, atomic, property-testable | More code moves, new twin functions | Medium |

#### Decision & Rationale
**Chosen Option:** B. The pack now changes only through `events::apply`, which refuses a stale event and rolls back a half-applied list. A would have left the log as an after-the-fact copy that a new rule could forget to update.

**Trade-offs accepted:** each rule has an event-producing twin beside the old function, kept so public names and their tests stay. Hunger and health are still not events, so rewinding a meal returns the food but not the hunger.

#### Implementation Notes
- `craft_events`, `eat_events`, `gather_events` and `place_events` build their events from a copy of the inventory and `slot_events`, so they cannot disagree with `craft`, `eat` and the rest; `craft` and `eat` now share a private core with their twins.
- `building::service` gained `break_target`, `place_target` and `block_event`, so the reach, face and eye-cell checks live in one place for both paths.
- `app::Running::commit` applies a list through `apply_all` and logs it only if all of it applied. A refusal there means the plan was made from a stale state, which cannot happen on one thread, so it is logged as a warning.
- The log is non-empty after a craft now, so `world.mcel` is written on exit more often. Startup replay still reads only block events per chunk, so nothing is applied twice, but the log grows with every pickup. "Grants that survive a restart" is still open.
- No benchmark: clicks, crafts and meals are not a hot loop.
- The window was not run by the assistant; the change is covered by unit and property tests, including a random run compared against the direct functions and undone in reverse.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-28 | Initial entry | deltaog-117 |

### Time Rewind, First Slice

**Date:** 2026-10-28
**Status:** Confirmed

#### Context / Background
Every change is a logged event that remembers what it replaced, so undoing was solved. The open question was what a rewind does to the log, which is append-only, saved to disk and replayed per chunk. The log is also flat: a break and its yield are two events, and undoing one alone would leave block and pack out of step.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Append inverse events | Log stays append-only, replay and save work unchanged, local zones fit later | No forward step, log grows per rewind | Small |
| B: Movable "now" marker | Two-way scrubbing | Breaks append-only, chunk replay and file must track the marker | Medium |
| C: Snapshots plus replay | Bounded log | New file format and schedule, solves a problem not yet present | High |

#### Decision & Rationale
**Chosen Option:** A. A rewind is one more batch through `apply_all`, so it is all or nothing and already covered by the same checks. B's forward step can be added later by remembering undone actions in memory.

**Trade-offs accepted:** the log grows with every rewind, and there is no redo yet.

#### Implementation Notes
- The log records where each action ends (`end_action`). Each rewind cancels the closest edit before it, like matching brackets (`latest_live_edit`), so pressing `Z` twice steps back two edits instead of redoing the first.
- `events::rewind` builds the inverses newest first; `app::Running::rewind` commits them as a rewind and remeshes the changed blocks.
- The saved log is version 3 with two new one-byte records for the ends. Version 1 and 2 files still read, but have no action ends, so their history cannot be rewound.
- Not undone: walking, hunger (rewinding a meal returns the food only), health and respawn. A rewind that no longer matches the pack, for example after a respawn, is refused and logged. Edits in unloaded chunks are refused the same way.
- Property tests: rewinding k actions gives the world after the earlier ones, replaying the whole log gives the same world, and an edit after a rewind can itself be rewound.
- The window was not run by the assistant, so the `Z` key and the remesh are untested by hand.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-28 | Initial entry | deltaog-117 |

### Bounded Log and Redo

**Date:** 2026-10-29
**Status:** Confirmed

#### Context / Background
Every rewind added events to an append-only log that is kept in memory and saved whole, so the log only grew. The first rewind slice also had no way forward again. Snapshots were the fix the earlier entries pointed to.

#### Options Considered

| Option | Advantages | Disadvantages | Difficulty |
|---|---|---|---|
| A: Fold old history into a per-chunk base, drop the old events | Bounds memory and file, log stays append-only | Old history lost, rewind depth capped | Medium |
| B: Remove the rewound action instead of logging an undo | Smallest change, no format change | Does not bound growth, breaks append-only, crash risk | Small |
| C: Fold like A, move the old events to an archive file | Bounded memory, history kept on disk | Two files to keep consistent, disk unbounded | High |

#### Decision & Rationale
**Chosen Option:** A. It is the only simple option that bounds the log and keeps the append-only rule. It fits chunk streaming: a chunk is rebuilt as terrain, then its base, then its events. Redo has one credible design, so it had no options: rewound edits stay in memory and are logged again as a new edit, which the existing bracket matching already understands.

**Trade-offs accepted:** history older than the newest 256 actions is gone for good, so no timelapse or fork can start there; moving to C later means archiving instead of discarding. The base holds one entry per edited position, so it grows with the number of distinct blocks changed, not with the number of actions. The redo stack is lost on restart.

#### Implementation Notes
- `EventLog::compact(keep)` folds all but the newest `keep` actions at an action boundary: each `BlockSet` sets its position in the base to its `new` block, and other events are dropped, since the inventory is saved as its own snapshot. The per-chunk index is rebuilt after the fold.
- `log_action` compacts to 256 once the log holds 512 actions, so the fold runs once per window and not on every action.
- Rewind needs no change: a rewind in the kept part that cancels a folded edit finds nothing live and stops, which is the intended limit.
- Redo: `RedoStack` stores each undone edit; `Y` applies the newest and logs it as an edit. A new edit clears the stack, and a refused redo (the state moved on, for example after a respawn) clears it too.
- The saved log is version 4 with tag 6 for a base block; version 1 to 3 files still read.
- Property tests: a compacted log keeps exactly the newest actions rewindable and rebuilds the world; a compacted log saved and reloaded chunk by chunk over generated terrain equals the live world; rewind and redo move through the history both ways and a replay of the whole log agrees.
- The window was not run by the assistant, so the `Y` key and the remesh are untested by hand.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-29 | Initial entry | deltaog-117 |

### Saved Intent Tracks

**Date:** 2026-10-30
**Status:** Confirmed

#### Context / Background
The intent log, which makes a run replayable bit for bit, lived in memory only. A replay needs it on disk. At one entry per actor per tick it would also cost about 2 MB an hour with one zombie, mostly the same intent repeated while an actor stands or walks straight.

#### Decision & Rationale
The approach was clear, so no options were compared: a new file next to `world.mcel` and `inventory.mcin`, with the same shape (magic, version, little-endian fields, a temporary sibling file renamed into place, a reader that refuses damage). The one real choice was the record. A run (actor, first tick, length, intent) stores an unchanged intent once, so idle time is nearly free and the file follows what the actor did, not how long the session ran. The writer joins every maximal run, so the same log always gives the same bytes. The reader is stricter than the writer needs to be: it rejects a walk axis of -128 instead of quietly turning it into -127, since a replay must not differ from the recording.

**Trade-offs accepted:** the file is bounded by how often intents change, not by a fixed size; an actor that changes every tick still costs 19 bytes per tick, more than a plain entry list would. The in-memory log is still unbounded. Ticks restart at zero each session, so the file holds only the latest session and each exit replaces it.

#### Implementation Notes
- `simulation::persist` has `encode`, `decode`, `save` and `load`, written like `events::persist`. Run order inside an actor must rise in ticks; the reader feeds each tick through `IntentLog::record`, so the log's own rule decides, and a violation is reported as out of order.
- `IntentLog::tracks()` walks the actors for the writer.
- `app` saves `intents.mcit` on exit when the log is not empty; a failed save is reported like the other saves.
- Tests: a property that decode inverts encode and bytes are stable, a cut inside a run is rejected, arbitrary bytes never panic, one run covers 10,000 idle ticks, and one integration property that a saved and reloaded player track replays to the same body.
- The window was not run by the assistant, so the save on exit is untested by hand.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-30 | Initial entry | deltaog-117 |

### Streamed Intent Tracks

**Date:** 2026-10-30
**Status:** Confirmed

#### Context / Background
The saved intent track was written once, on exit, and each exit replaced the last session's file. That left three gaps: the in-memory log grew by one entry per actor per tick for as long as the game ran, a crash lost the whole session, and older sessions were gone.

#### Decision & Rationale
Three options were compared. A kept every tick as runs in memory and wrote numbered files on exit: smallest change, but a crash loses everything and busy play still grows memory. B dropped the oldest ticks past a limit: simplest, but a track that no longer starts at tick 0 cannot be replayed from a fresh body. C wrote settled stretches to the file during play. C was chosen for bounded memory and crash survival, with its cost accepted: a flush on the game loop and handling of failed and torn writes.

The design that kept C small: the file stays a plain `MCIT` version 1 file, so the reader and the existing `encode` are unchanged and no version bump was needed. The reader already accepts runs in any order within an actor, so appending runs in flush order is valid. Each track's newest entry stays in memory, so the rising-ticks rule keeps working and a run is never ended early by a flush; an idle hour still costs one run per flush (19 bytes), not one per tick. Memory per flush is a few thousand entries, not the session.

Invariants that make data loss impossible short of a crash: an entry leaves memory only after its run was written; the file is always the header plus whole runs up to `good_len`; a failed write is cut back to `good_len` before the next write, and if the cut-back also fails the file is marked broken and nothing more goes to it, so a torn run can never sit in the middle of a file. After a failed flush the next try waits for another 2048 entries, so a full disk costs one warning per flush, not one per tick.

**Trade-offs accepted:** a crash still loses up to 2048 entries per run of play (a minute or two), and a power loss can leave a torn last run, which `decode_recovering` handles by keeping the whole runs before it (the strict `decode` still refuses such a file). Every launch that plays at least one tick leaves a new file; none are removed, but runs keep them small. Without a replay mode nothing reads these files yet.

#### Implementation Notes
- New `simulation::stream`: `IntentStream` (record, flush at 2048 entries, `finish` that syncs) and `next_session_path`, which picks one more than the highest `NNNN.mcit` in `intents/` and refuses to wrap at `u32::MAX`. The file is created with `create_new` on the first write, so a second game in the same folder never appends to it, and a session that records nothing leaves no file.
- `persist` gained `push_runs` (the run-joining loop pulled out of `encode`, same bytes), a `HEADER` constant and `decode_recovering`; `IntentLog` gained `retain_newest`.
- `app` records through `IntentStream`, calls `finish` on exit, and no longer writes `intents.mcit`. A leftover `intents.mcit` from earlier runs is left alone.
- Tests: finishing gives back exactly the recorded log for any flush size; file plus tail always equals the whole history and the file is a per-actor prefix (the crash case); 100,000 idle ticks stay under 1 KB on disk and 2048 entries in memory; a failed write keeps every entry and retries at the pushed-back mark; a write that cannot be undone stops further writes (`/dev/full`, Linux only); cut files recover; session numbering.
- The window was not run by the assistant, so the stream during real play and the `intents/` folder on exit are untested by hand. Add `intents/` to your ignore rules if you do not want sessions tracked.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-30 | Initial entry | deltaog-117 |

### Replaying Intent Tracks

**Date:** 2026-10-30
**Status:** Confirmed

#### Context / Background
Sessions were written to `intents/NNNN.mcit` but nothing read them. The last open piece of "Saving intent tracks" was a mode that loads one at startup and drives the player and the zombie from it.

#### Decision & Rationale
Three options: A, a command-line flag that replays and then hands the keys back; B, the same flag but the window closes at the end; C, an in-game key that resets the world and replays. C was dropped because the world cannot yet be reset to a run's start (walking, hunger and health are not events). A was chosen over B because the hand-over costs almost nothing: the replay just stops answering. The Forza-style rewind (hold a key, scrub back a few seconds, resume live) was discussed and is left for a later slice built on this one.

Decisions made while building it, which differ from the plan given before coding in one place: the session started from a replay records the replayed steps as well as the new ones. The plan was to record nothing until the hand-over, but that file would not start at tick 0, and a replay needs the player's track to start there. Recording everything keeps every session replayable by itself, at the price of one more file per replay.
- The replay ends at the first tick with no recorded player step. That also covers the end of the file and a flight, which leaves no steps in the track; `F` is ignored during a replay so a flight cannot cut it short by accident.
- A zombie takes its recorded step if it has one, and otherwise decides for itself. After the player's click hits the original zombie, the recording stops but the replayed zombie lives on.
- A file cut short by a crash still plays, up to the cut. A missing, damaged or empty file, or a bad flag, stops the game before the window opens, with a message.
- `Replay::try_new` refuses a track whose first player step is not tick 0, so a replay always lines up with the clock.

**Trade-offs accepted:** the tracks hold walking and jumping only, so a replay matches the original only if the world is the same as when the run began. Block edits, crafting, eating and clicks on the zombie are not replayed, and clicks still work during a replay and can change the world under it. Nothing limits how many session files are kept.

#### Implementation Notes
- New `simulation::replay`: `Replay` (`try_new`, `load`, `intent`), `ReplayError`, `replay_session` and `ArgsError`. `stream::session_path` builds a session path and `next_session_path` now uses it.
- `app` reads the flag and loads the session before opening a window, keeps the replay in `Running`, asks `player_intent` for each player step and the replay for each zombie step, and adds a line to the title while it runs.
- Tests: the flag, a missing, damaged, empty, offset and cut-short file, lookup by tick, and one integration property that a run streamed to a session file and loaded as a replay moves a fresh body to exactly the same state.
- The window was not run by the assistant, so the replay during real play and the hand-over are untested by hand. Try it with `cargo run -- --replay 1` after one recorded session.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-30 | Initial entry | deltaog-117 |

---

### Limiting Kept Intent Sessions

**Date:** 2026-10-31
**Status:** Confirmed

#### Context / Background
Every game start adds a numbered file to `intents/` and nothing deleted them. The replay slice listed this as an open trade-off.

#### Decision & Rationale
One credible approach, so no options were presented: keep the newest 20 files and delete the older ones when the game starts. Session numbers only go up, so "oldest" is simply the lowest numbers. Pruning at startup, not at exit, means a crash cannot skip it, and it leaves room for the new session by keeping 19 old ones. A replayed session is already loaded into memory before the window opens, so pruning cannot take it away.

**Trade-offs accepted:** the limit is a constant, not a setting. Replaying `--replay N` for a session older than the newest 20 will not work after a few more runs. A failed deletion only logs a warning and play goes on.

#### Implementation Notes
- `simulation::stream`: `prune_sessions(dir, keep)`, plus a shared `session_numbers` helper that `next_session_path` now uses too. Files that are not numbered `.mcit` files are never touched.
- `app`: `MAX_SESSIONS = 20`, and the call before the new stream is created.
- Test: pruning removes only the lowest numbers, leaves other files alone, and treats a missing folder as empty. The window was not run by the assistant.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-31 | Initial entry | deltaog-117 |

---

### Recording Actions in Intent Tracks

**Date:** 2026-10-31
**Status:** Confirmed

#### Context / Background
A replay fed back walking and jumping only, so it matched the original run only if the world began exactly as before. Breaking, placing, crafting and eating were not in the file.

#### Decision & Rationale
Three options were presented; the choice was **A: record the player's intention in the same session file**. B (record the exact changes) fails whenever the world differs, because the recorded old values no longer fit. C (a second file) can disagree with the first after a crash and needs its own write and recovery code.
- An action names its target, not the click: the camera's look direction is not recorded, so "break the block at (x, y, z)" is what can be played back. A replay works out the result from the world it finds, so a break with no block there is refused like a click at empty air.
- Live play and replay share one path. A click works out its target block, then calls the same `perform` a replay calls, so what is recorded is what a replay does.
- An action is stamped with the tick it happens before, and a replay performs it before that tick's steps, which is the order it had live.
- Only actions that worked are recorded, so a click at air costs nothing in the file.
- Keys and mouse do not act during a replay, as `F` already did not; otherwise a stray click would change the world under the replay.

**Trade-offs accepted:** a replay in a different world gives a different run, not an identical one. Hits on a mob and rewinds are still not recorded. A craft stores a position in the recipe table, so adding recipes in the middle of the table would change what an old session crafts.

#### Implementation Notes
- File format version 2: a tag byte before each record (1 run, 2 action). Version 1 files are read as untagged runs. A file cut short mid-record is still recovered up to the cut, now for actions too.
- New `simulation::action::Action`; `IntentLog` gains an ordered action list (several actions may share a tick); `IntentStream` writes settled actions with the runs and keeps the newest in memory for the ordering check.
- `inventory::service` gains `gather_events_at` and `place_events_at`, which share their code with the look-direction versions.
- Tests: round trips with random actions, cuts inside an action, bad tags and kinds, actions out of order, a version 1 file, streaming across flushes, and one integration test that a streamed session replays its actions on their ticks. The existing torn-tail replay test now appends a real cut-short run, since a bare byte 7 is now an unknown tag.
- The window was not run by the assistant. Try it by playing a short session with a break, a place, a craft and a meal, then `cargo run -- --replay N`.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-31 | Initial entry | deltaog-117 |


---

### Recording Hits and Rewinds in Intent Tracks

**Date:** 2026-10-31
**Status:** Confirmed

#### Context / Background
A replay performed breaks, placements, crafts and meals, but not hits on the zombie or the `Z`/`Y` rewinds. A run that used them drifted: the zombie lived when it had died, and the world kept edits the player had undone.

#### Decision & Rationale
The approach was clear (new action kinds in the existing record), so no options were compared.
- A hit names the zombie by its intent-log id, not the aim. The look direction is not recorded, so a replay cannot re-aim; it hits "zombie 3" with the best weapon in the pack, and is refused if that zombie is already gone.
- Rewind and redo are actions with no fields. The rewind history and the redo stack are rebuilt by performing the same actions in order, so nothing else needs saving.
- Live play and replay share one path: `Z`, `Y` and the click call `act`, which calls `perform`, which records only what worked. A rewind with nothing to undo, or a redo with nothing undone, costs nothing in the file.
- The aim test is split from the hit: the click checks the crosshair, then performs a hit that works from the mob's id alone.

**Trade-offs accepted:** the replay does not check that the zombie is within reach, since the hit was in reach when it was recorded; in a different world it may land when the live one would have missed. The starting world is still not saved, so exactness depends on starting from the same world.

#### Implementation Notes
- File format version 3: action kinds 5 (strike, actor id), 6 (rewind) and 7 (redo). Versions 1 and 2 are still read.
- `Running::hit_zombie` became `aimed_at_zombie` plus `strike_zombie`; `rewind` and `redo` now report whether they did anything.
- Tests: the random round-trip and torn-tail tests now draw the new kinds, plus a version 2 file and the three new kinds sharing one tick.
- The window was not run by the assistant. Try it by hitting the zombie, pressing `Z` and `Y`, then `cargo run -- --replay N`.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-10-31 | Initial entry | deltaog-117 |

---

### Meters Under Events

**Date:** 2026-11-01
**Status:** Confirmed

#### Context / Background
Hunger, health and respawn changed outside the event log, so rewind could not undo them and scripts could not see them. The difficulty is that meters also change on their own (hunger drains every 30 s, starvation hits every 5 s), so an event that says "from X to Y" goes stale within seconds and would block `Z` on the newest edit for good.

#### Decision & Rationale
Three options were compared: A, absolute set events like blocks and slots; B, change events with everything logged; C, change events for deliberate changes only. B was chosen.
- An event carries the points the meter really moved (a meal at 18 of 20 logs +2, not the food's 4). Its inverse is the opposite change, so undoing a meal after a drain still leaves the drain in place: the meter lands where it would have without the meal.
- The hunger drain, starvation and a mob's hit are logged too, as a new passive action kind. Rewind steps over them, so `Z` never undoes damage and the log still describes every change to the state.
- A respawn is one action of its own kind that rewind cannot cross: what came before no longer matches the pack and the meters. `Z` now reports nothing to undo there, where it used to be refused and logged.

**Trade-offs accepted:** an undo that would push a meter out of 0 to 20 is refused (for example undoing a meal when hunger has since drained below what the meal gave), which is rare but can leave `Z` waiting on that meal. Passive actions share the 256-action history window with edits. The meters are still not saved or replayed from the log at startup, as before.

#### Implementation Notes
- `Stats::advance` returns what a step drains without applying it; `Stats::step` is built on it, and a property test shows both reach the same stats. The step count is a clock, not a meter: a respawn restarts it directly, as before.
- Log format version 5: tags 7 and 8 for the meter events, 9 and 10 for the passive and respawn action ends. Older files are still read.
- `Stage` gained the stats; `eat_events` now returns the hunger change with the slot changes, and `Running` no longer restores hunger on the side.
- Tests: a property test that a run of mixed events undoes in reverse and replays to the same state, the meter events in the event and file round trips, and an integration test that rewinds a meal across a drain and a hit and stops at a respawn.
- The window was not run by the assistant. Try it by eating, waiting, taking a hit and pressing `Z`.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-11-01 | Initial entry | deltaog-117 |

---

### Mobs Under Events

**Date:** 2026-11-02
**Status:** Confirmed

#### Context / Background
The game held one `Option<Zombie>` in `app`, changed by direct calls. Scripts cannot spawn what the log does not describe, and rewind cannot undo what the log does not record. The hard part is that walking is not an event yet, so a mob's position when an event is applied is not the one it had when the event was made.

#### Decision & Rationale
Three options were compared: A, a store of mobs with spawn, removal and health-change events; B, the single `Option<Zombie>` with spawn and remove events; C, A plus a move event on every tick. A was chosen.
- A spawn and a removal are each other's inverse, and a hit is a change in health by the points really lost, like the meters. A kill is the health change followed by a removal at zero health, so undoing it in reverse puts the zombie back at the health it had.
- Applying a removal checks the mob's id and health but not its position. Checking position would make undoing a spawn fail for any mob that has walked, and walking is not an event. The removal still records where the mob stood, so an undo puts it back there.
- Spawns, hits, kills, drops for distance and the clearing at a respawn are all passive actions (the clear joins the respawn action). `Z` steps over them, as it did when hits were direct calls, and the redo stack is kept.
- A mob's id is also its actor id in the intent log, so recorded hits still find their target and a replay spawns the same ids in the same order.

**Trade-offs accepted:** the store holds only zombies, so another kind of mob will need either a second store or a kind field. Mob events are logged but never replayed: a restart starts with no mobs, as before, and the log keeps them as history. They share the 256-action window with edits and meters, and a zombie's life costs three or four records. The game still spawns one zombie at a time; the store and the events allow many, but no rule asks for more yet.

#### Implementation Notes
- `Mobs::spawn`, `remove` and `shift_health` are the only ways to change the store; `get_mut` hands out a mob to step, whose health only the `mobs` feature can write, and nothing there does. `mobs::service::strike` was removed for `strike_events`, and `aimed_at` became `aim_distance` so a click picks the nearest of several mobs.
- Log format version 6: tags 11 to 13. Older files are still read; the unknown-tag test now uses tag 14.
- Tests: a run of mixed block, player, pack, meter and mob events undoes in reverse and replays to the same state; stale mob events are refused and change nothing; the new records round-trip through a file and are refused in a version 5 file; an integration test kills a zombie through the door, and another follows a zombie's life through a save, a rewind that steps over it, and an exact undo.
- The window was not run by the assistant. Try it by waiting ten seconds for a zombie, hitting it until it dies, and dying to one, then watching the next one appear.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-11-02 | Initial entry | deltaog-117 |

---

### Redo Across Restarts, Script Spawns and Reads, Rewind in Unloaded Chunks

**Date:** 2026-11-02
**Status:** Confirmed

#### Context / Background
Four roadmap items were done in one cycle. `Y` forgot everything on a restart. Scripts could write to the world but not read it, and could not add a mob now that mobs are events. A rewind in a chunk the player had walked away from was refused, and an undo of a break there would even have left a one-block chunk behind, because applying a block event to an unloaded chunk creates one, and the real chunk then fails to stream in.

#### Decision & Rationale
**Redo stack.** Three options were compared: A, a separate small file; B, a section inside `world.mcel`; C, mark a redo in the log and rebuild the stack from it. C was chosen.
- A redo is logged as its own action kind. Rewind treats it like an edit, so it can be undone again, but a replay of the marks can tell it from a new edit: a rewind pushes, a redo pops, an edit or a respawn empties the stack.
- Nothing extra is stored, so the stack and the world cannot come from different runs. This was the deciding point over A.

**Script reads.** Three options were compared: A, a separate `Query` call; B, reads inside `Command`; C, a frozen view per tick. A was chosen.
- `ask` takes the stage by shared reference, so the type system says it cannot change the game, and the rule "every accepted command is logged and undoes exactly" stays true as written.
- Reads need their own `Look` permission, count toward the per-tick limit (refused ones too), and a `Find` searches a 16-block cube, about 36 thousand lookups at most.

**Spawn command and unloaded chunks.** One credible approach each, so no options were compared.
- The spawn command follows the other commands: permission, range, a check that the zombie's box is in the open, and a cap of 8 live mobs. Ids are never reused, so `run` takes the app's id counter and moves it on only when a spawn is accepted.
- A rewind or redo now loads every unloaded chunk it touches (terrain plus logged edits) before applying, and hands it to the streamer, which unloads it again when it is out of range. The log keeps the edits either way.

**Trade-offs accepted:** the redo stack only reaches back as far as the 256 kept actions, like `Z`. The meters are not saved, so a redo of a meal after a restart is refused and the stack is cleared. A `Find` that misses the 16-block cube answers "nothing", even if one exists farther away. Nothing in the window runs scripts yet, so the spawn command and the reads are tested but not played.

#### Implementation Notes
- Log format version 7: tag 14 ends a redo. Older files are still read, and a redo in them reads as a plain edit, which leaves the stack empty. The unknown-tag test moved to tag 15.
- `RedoStack::from_log` replays the action marks. A property test runs random edits, rewinds, redos, passive changes and respawns, and checks the rebuilt stack against the one kept live, including after a save and load.
- `touched_chunks` names the chunks a rewind changes. A property test rebuilds a world from only those chunks and shows the rewind applies cleanly and lands on the same blocks.
- `Zombie` boxes are checked with `mobs::service::blocked`, which uses the same collision test as the player.
- Tests: spawn, refused spawn, cap, every read, ties in `Find`, the radius limit, read permission and limit, and a property test that queries never change the game or the log. The scripted-session property test now spawns mobs and checks they replay and undo too.
- The window was not run by the assistant. Try it by breaking blocks, pressing `Z` twice, quitting and reopening, then `Y`; and by breaking a block, walking away until its chunk unloads, coming back far enough to see it gone from view and pressing `Z`.

#### Review / Update Log

| Date | Update | Author |
|------|--------|--------|
| 2026-11-02 | Initial entry | deltaog-117 |
