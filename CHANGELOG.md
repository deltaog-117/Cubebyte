# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Project scaffold with feature-first modules: `world`, `events`, `building`, `player`, `rendering`, `shared`, `app`
- `Cargo.toml` (edition 2024), `clippy.toml` and `scripts/check` (format check, clippy with warnings as errors, tests)
- Voxel types in `shared::voxel`: `BlockId`, `ChunkPos`, `WorldPos` and `LocalPos`, whose constructor rejects out-of-range coordinates
- `WorldPos::split`, using floor division so negative coordinates land in the correct chunk
- `Chunk`: a 16×16×16 flat block array with an O(1) empty check
- `World`: chunk map that changes only through `World::apply(&Event)`, rejecting stale events and never storing all-air chunks
- `shared::event::Event::BlockSet` with `inverse()`, so edits can be undone without a snapshot
- `events::log::EventLog`, an append-only event log
- Property tests for position splitting, chunk contents, undo of every event, and replay of a log into an equal world
- Criterion benchmarks for chunk reads and for placing and removing a block
- `shared::rng::Rng`: seeded xoshiro256++ (SplitMix64 seeding) with unbiased `below(bound)` and `fork(key)` for independent per-key streams
- Tests that pin the generator's output for known seeds, so any change to the stream fails loudly
- `shared::clock::FixedClock` and `Tick`: integer-time fixed timestep with carry-over and a cap on catch-up steps per advance
- Property tests for the clock (frame splitting never changes the tick count) and the generator (determinism, bounds, fork independence)
- Criterion benchmark for `Rng::next_u64` and `Rng::below`
- `README.md` with the project description, status, tech stack (Rust + `wgpu` + `winit`), structure and test commands
- `world::generation::TerrainGenerator`: seeded, deterministic terrain from hand-written 2D Perlin noise (four octaves), with grass, three dirt blocks and stone below; heights stay within `MIN_HEIGHT..=MAX_HEIGHT`
- `World::insert_generated` and `InsertError`: install generated chunks as the baseline without logging events, and refuse to overwrite a loaded chunk
- `BlockId::STONE`, `DIRT` and `GRASS` constants, and `LocalPos::all()` to iterate a chunk in index order
- Tests that pin terrain heights for known seeds, check there are no cliffs between neighbouring columns, and check chunk layers across chunk borders
- Criterion benchmark for generating a surface chunk and a sky chunk

---
- `rendering::meshing::mesh_chunk`: turns a chunk into `MeshData` with one quad per block face that touches air; border faces read the neighbouring chunk through the world, and an unloaded neighbour counts as air
- `Face`, `Vertex` and `MeshData` types: vertices hold chunk-local positions, the face and the block type, and the mesh always holds whole quads with in-range indices
- Tests that check the quad count against an independent count, that indices and positions are in range, and that every triangle faces along its face normal
- Criterion benchmark for meshing a surface chunk and a sky chunk
- `rendering::camera::Camera`: free-flying camera with clamped pitch and a `wgpu`-ready (depth `0..1`) view-projection matrix; property tests for the pitch limit, orthonormal view basis and fly round-trip
- `player::input::MoveKeys`: held movement keys reduced to a normalized direction, so diagonals are not faster
- `rendering::vertex::GpuVertex` and `gpu_mesh`: converts a chunk mesh to world-space vertices with a flat block colour and a fixed per-face shade baked in
- `rendering::pipeline::Renderer`: `wgpu` surface, depth buffer, one pipeline and one draw per chunk mesh; reconfigures a lost or outdated surface
- `app::run`: `winit` window and game loop that generates a fixed 9×9 column of chunks, meshes them and lets the player fly around (WASD, mouse, Space, Left Shift, Esc to quit)
- Dependencies: `wgpu`, `winit`, `glam`, `bytemuck`, `pollster`
- `building::raycast::cast`: voxel ray cast (Amanatides-Woo) returning the first solid block within reach and the empty cell before it; property tests check it against an independent slab-test search
- `building::service::{break_block, place_block}`: edits applied to the world and then logged as `Event::BlockSet`, with a 5-block reach; placing is refused inside the viewer's own cell and from inside a block
- `building::model::SolidBlock` (a block that is never air) and `EditError`
- `rendering::meshing::chunks_to_remesh`: the chunks an edit can change (its own, plus neighbours across a border); a property test checks every other mesh stays identical
- Left click breaks and right click places a block in the window; `1`, `2` and `3` select stone, dirt or grass
- Tests that any run of edits replays and undoes exactly, on a block of stone and on generated terrain
- Criterion benchmark for a full-reach ray cast (about 81 ns)
- `world::streaming::Streamer`: plans which chunk columns to load (nearest first) and unload around the camera, with one chunk of margin so a camera on a border does not thrash; property tests check the plans settle on exactly the square around the centre
- `EventLog::chunk_events`: a per-chunk index into the log, so one chunk's edits replay in log order without scanning everything
- `events::replay::replay_chunk` and `World::unload`: drop a chunk and bring it back as generated terrain plus its logged edits
- `Event::chunk`
- Integration test that unloads and reloads every chunk after random edits and checks the world is unchanged

### Changed
- `app::run` streams chunks around the camera (radius 4, at most 8 loaded per frame) instead of generating a fixed 9×9 grid; edited chunks come back with their edits replayed from the log
- `Renderer::add_mesh` is now `Renderer::set_mesh(chunk, ..)`: meshes are kept per chunk, so one chunk can be replaced or removed
