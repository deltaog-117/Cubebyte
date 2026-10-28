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
- `events::persist`: a versioned binary save format for the event log (magic, version, seed, then 17-byte records), with `encode`, `decode`, `save` and `load`; `save` writes a sibling file and renames it, so a crash never leaves a half-written save
- Integration test that saves a log, restores it, and checks every chunk replays to the edited world
- `blocks::registry`: one table of block name, colour and solidity indexed by `BlockId`, with `get`, `color` and `is_solid`; undefined IDs have no entry and draw magenta
- Property test that every `u16` ID has a displayable colour, and a test that the `BlockId` constants match their table entries
- `simulation::model::Simulation`: owns the `FixedClock` and a never-drawn root `Rng`, with `advance(elapsed)`, `tick()` and `stream(key)` for per-system random streams
- Property test that splitting the same real time into different frames gives the same tick and the same random stream
- `items::registry`: one table of item names, stack limits and the block each item places, with `drop_of(block)` for what breaking a block yields
- `inventory::model`: `Stack` (1 to the item's limit, checked on construction) and a 27-slot `Inventory` with `add`, `take`, `count_of` and `room_for`
- `inventory::persist`: a versioned `MCIN` snapshot, written atomically to `inventory.mcin` and checked slot by slot on load
- `inventory::service`: `gather_block` and `place_from_stock`, which pair each block edit with its inventory change and refuse before touching the world
- `survival::model`: `Stats` with health and hunger meters stepped by the simulation; hunger drops every 30 seconds and starvation lowers health to a floor of 1
- Property tests for inventory conservation, stack limits, `room_for` against `add`, save round trips and the stats ranges
- `crafting::registry`: a `const` table of shapeless recipes (bread from 3 grass, stew from 4 grass and 2 dirt) with append-only `RecipeId`s
- `crafting::service::craft`: spends a recipe's inputs and adds its output on a copy of the inventory, so a short input or a full pack changes nothing
- Food items `bread` and `stew`, and `ItemDef::restores_hunger`, which marks an item as food
- `survival::service::eat` and `Stats::restore_hunger`: eating spends one food item, refills hunger up to the maximum, and refuses when hunger is already full
- Keys `R` (next recipe), `C` (craft) and `E` (eat); the window title shows every item count and the selected recipe
- Property test that a craft either changes nothing or moves counts by exactly the recipe, and that the result survives a save and load
- `player::model::Player`: a walking box (0.6 wide, 1.8 tall, eyes at 1.62) whose position is finite by construction and changes only through `step`
- `player::service::step`: gravity, jump from the ground, and per-axis swept collision against solid blocks, so the player slides along walls and cannot tunnel through a floor at terminal speed
- `player::service::collides` and `Intent`: the overlap test and the per-tick request (walk direction and jump); a non-finite request counts as standing still
- `FixedClock::alpha` and `Simulation::alpha`, the fraction of the next step already elapsed, so the view can blend between the last two steps
- `MoveKeys::walk` and `Camera::level_forward` for ground movement; key `F` toggles between walking and free flight
- Property tests that the player never ends a step inside a block or below the floor, and that the same start and inputs give an identical player; a benchmark for one step
- `combat::model::Damage`: a hit's size, at least 1 by construction, with `FIST` for bare hands
- `combat::service`: `hit` takes health and reports `Survived` or `Died`, `respawn` restores full meters and empties the inventory, and `strike` gives the best weapon's damage in the pack
- `Stats::hurt` and `Stats::is_dead`; health can now reach 0
- A sword item (`ItemId::SWORD`, damage 4, stack of 1) and `ItemDef::damage`; it cannot be crafted or found yet
- Property tests that health stays in range under any mix of hits, starvation and respawns, and that a sword always out-hits a fist
- `shared::body`: the walking box (gravity, jumping, swept collision against solid cells) split out of the player so any body of any size moves by the same rules; it takes the solid test as a closure and imports no feature
- `mobs` feature with one hostile `Zombie` (20 health): it walks at half the player's speed towards the player within 24 blocks, jumps one-block steps, and hits for 2 once a second within 1.2 blocks. `mobs::service::strike` takes hits, `aimed_at` says whether the crosshair is on it before any block, and `spawn_point` and `despawns` place and drop it
- `shared::math::ray_box`, a ray against a box, used for entity hits
- `rendering::vertex::box_mesh` and `Renderer::set_entities`: the zombie is drawn as a green box
- A sword recipe (6 stone and 2 dirt), so the sword can now be crafted
- Tests: shared-body properties for bodies of any size, zombie chase, hit, climb, strike, aim and spawn tests, and an integration test of a zombie killing the player and a crafted sword killing a zombie
- `shared::intent`: `Walk` (two axes from -127 to 127, private fields so no other value exists) and `Intent`, with `Walk::from_direction` rounding a float direction and treating a non-finite one as standing still
- `simulation::intent_log`: `ActorId` and an in-memory, append-only `IntentLog` with one track per actor and strictly rising ticks; `app` records the player's and each zombie's intent on every step
- `mobs::service::decide` and `step_with`, so a zombie's intent can be recorded and replayed; `step` is now `decide` followed by `step_with`
- Property tests that replaying a recorded player track, and a zombie's decided intents, reproduces the run exactly
- `ui` feature: a crafting panel opened with Tab. `ui::layout` places a centred panel with one clickable row per recipe (rows you can craft now are green, others grey), `ui::font` is a built-in 5×7 pixel font, and `ui::draw` turns the panel into coloured quads; a left click on a row crafts that recipe
- `Renderer::set_overlay` and `Renderer::size`, with a second pipeline that draws the interface over the scene without depth
- `crafting::registry::ids` and `crafting::service::can_craft`, so the panel can list every recipe and mark the affordable ones
- Property tests that panel rows fit the window without overlapping, that a click finds the row under it, and that the overlay's indices and colours are valid

### Changed
- `shared::body::Intent` moved to `shared::intent` (still re-exported from `shared::body` and `player::service`) and its `walk` is now a `Walk` of whole numbers instead of a `Vec2`. Direction is rounded to about 0.45 degrees and speed to under 0.4%; a player step still costs about 180 ns
- `player::model::Player` wraps a `shared::body::Body`; `player::service::step`, `collides`, `Intent` and the speed constants keep their paths and behaviour. A step still costs about 181 ns
- `app` spawns a zombie 12 to 20 blocks away ten seconds after the start, after each kill and after each death; a left click on the zombie hits it instead of breaking the block behind it. Free flight does not stop it
- `app` starts the player walking on the terrain and moves it on the simulation tick; the camera follows its eyes and Space jumps. Free flight remains on `F`
- `app::run` streams chunks around the camera (radius 4, at most 8 loaded per frame) instead of generating a fixed 9×9 grid; edited chunks come back with their edits replayed from the log
- `Renderer::add_mesh` is now `Renderer::set_mesh(chunk, ..)`: meshes are kept per chunk, so one chunk can be replaced or removed
- `app` loads `world.mcel` at startup and saves it on exit when the world was edited; a save made with another seed stops the game instead of being overwritten
- The renderer reads block colours from `blocks::registry` instead of its own `match`
- `app` advances a `Simulation` every frame (20 steps per second, at most 5 per frame); nothing runs on a step yet
- `SolidBlock::try_new` accepts only defined solid blocks, so an ID with no definition is no longer placeable (it used to be accepted when not air)
- Left click now keeps the broken block's item, right click needs one of the selected block in the inventory, and a full inventory leaves the block standing
- `app` loads `inventory.mcin` at startup, saves it on exit when it changed, and shows health, hunger and item counts in the window title
- Starvation now kills: the old floor of 1 health is gone, and dying in `app` respawns the player at the start with full meters and an empty pack; the window title counts deaths
- `items::registry::ItemDef` gains `restores_hunger`, and `items::registry::ids()` lists every item; the window title lists all items instead of a fixed three
- Escape closes the crafting panel first and quits only when it is closed; the pointer is released while the panel is open and mouse look pauses
