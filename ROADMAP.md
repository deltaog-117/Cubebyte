# Roadmap

Author: deltaog-117

This document outlines the future direction of the minecraft-clone project: a voxel sandbox with the usual basics (building, crafting, combat, resource management) plus its own unique features, code as gameplay and time rewind.
Items are organized by priority, not by timeline.

---

## ✅ Completed (Milestones Achieved)

- ✅ Feature-first project scaffold (2024-edition module files, tests mirroring `src/`)
- ✅ `Cargo.toml`, lint rules and `scripts/check` (format, clippy with warnings as errors, tests)
- ✅ Voxel types: `BlockId`, `ChunkPos`, `WorldPos`, and `LocalPos` that can only hold in-range values
- ✅ 16×16×16 flat-array `Chunk` and `World` that changes only by applying events
- ✅ `Event::BlockSet` with its inverse, and an append-only `EventLog`
- ✅ Property tests (positions, chunk contents, undo, replay) and a hot-path benchmark
- ✅ Seeded `Rng` (xoshiro256++ with a pinned output stream and per-key forks) and a `FixedClock` with a catch-up cap
- ✅ `README.md` recording the tech stack (Rust + `wgpu` + `winit`)
- ✅ Seeded terrain generation: Perlin heightmap with grass, dirt and stone layers, inserted into the world without logging events
- ✅ Chunk meshing: one quad per exposed block face, as plain vertex and index data with no `wgpu` types
- ✅ Basic renderer: `wgpu` pipeline with depth buffer, free-flying camera (WASD, mouse, Space/Shift) and a `winit` window drawing a fixed grid of terrain chunks
- ✅ Chunk streaming: chunks load and unload around the camera; an unloaded chunk reloads as generated terrain plus its logged edits, replayed from a per-chunk index of the log
- ✅ Block placing and breaking: voxel ray cast, `building/service` edits routed through the event log, and per-chunk mesh replacement (left click breaks, right click places, 1/2/3 pick stone, dirt or grass)
- ✅ Event replay wiring: the event log is saved to `world.mcel` on exit and loaded at startup in a versioned binary format; chunks rebuild from it as they stream in
- ✅ Block registry: `blocks::registry` holds each block's name, colour and solidity in one table indexed by `BlockId`; the renderer and `SolidBlock` read from it
- ✅ Simulation tick loop: `simulation::model::Simulation` owns the `FixedClock` and forks per-system `Rng` streams by key; `app` advances it each frame
- ✅ Resource management: an item registry, a 27-slot stack inventory saved to `inventory.mcin`, breaking yields and placing spends items, and health and hunger meters stepped by the simulation (shown in the window title)
- ✅ Crafting: a shapeless recipe table with an all-or-nothing `craft`, food items and `eat`; keys R, C and E, with the window title standing in for a UI
- ✅ Deterministic player physics: gravity, jumping and swept box collision stepped on the simulation tick, with the view blended between ticks; `F` toggles the old free flight
- ✅ Combat rules: a `Damage` type, health that can reach zero, death and respawn (full meters, empty pack), and a sword item that sets how hard the player hits
- ✅ Hostile mobs: a zombie that chases and hits on the tick, drawn as a box and hit back with the held weapon; it shares one walking-body module with the player, entity hits go through a ray-versus-box test, and a sword recipe exists
- ✅ Whole-number movement intent: walking direction is two integers from -127 to 127, bodies move by exactly those numbers, and the player's and each zombie's intent is recorded per tick in an in-memory `IntentLog`; replaying a track reproduces the run bit for bit
- ✅ Crafting interface: Tab opens an on-screen recipe panel (built-in pixel font, flat quads drawn over the scene); click a row to craft it, rows you cannot afford are greyed
- ✅ World interface for scripts, first slice: `Event` gains player jumps and per-slot inventory changes, one `events::apply` door sends every event to the world, player or inventory, and `scripting` runs commands (teleport, give item, set block) behind permissions, a range and a per-tick limit; every accepted command is logged and replays and undoes exactly (the window does not run scripts yet)
- ✅ Gameplay under events: breaking with its yield, placing with its cost, crafting and eating now work out their block and slot changes as events, apply them all or nothing through the one `events::apply` door, and log them; a property test shows the event path matches the direct functions and undoes exactly
- ✅ Time rewind, first slice: key `Z` steps back one whole action (a break with its yield, a placement, a craft, a meal); the undo is logged as new events, so the log stays append-only and a restart keeps the rewound world; the saved log gains action ends (version 3)
- ✅ Time rewind, second slice: the log folds all but its newest 256 actions into a base of final blocks per chunk (compaction runs at 512), so it stops growing with play; key `Y` redoes the edit `Z` last undid, from an in-memory stack that any new edit clears; the saved log is version 4
- ✅ Saving intent tracks: the intent log has a versioned file format (`MCIT`) that stores runs of one intent, so idle stretches cost almost nothing; the window writes `intents.mcit` on exit and a reloaded track replays the run bit for bit
- ✅ Streaming intent tracks: settled entries are appended to a numbered session file (`intents/0001.mcit`, ...) as play goes on, so memory is bounded, a crash loses little, older sessions are kept, and a file cut short by a crash can be read back with `decode_recovering`
- ✅ Replaying intent tracks: `cargo run -- --replay 3` loads `intents/0003.mcit` and drives the player and the zombie from it; when the recording has no step for a tick, the keys are live again from where the replay stopped, and the new play is recorded to a fresh session file that holds the replayed steps too
- ✅ Limiting kept intent sessions: at startup the game deletes the oldest `intents/NNNN.mcit` files so at most 20 remain, the new session included
- ✅ Recording hits and rewinds in intent tracks: hits on the zombie (named by the zombie's id), `Z` and `Y` are saved in the session file (version 3; versions 1 and 2 still load) and performed again by a replay, so a replay of a run that used them no longer drifts
- ✅ Meters under events: hunger and health changes are events that carry the points really moved (`HungerChanged`, `HealthChanged`), so a meal, a zombie's hit, the hunger that drains on its own and a respawn all go through the one `events::apply` door and the log; the passive ones are logged but `Z` steps over them, so undoing a meal still works after hunger has drained; a respawn is a barrier that `Z` cannot rewind past; the saved log is version 5
- ✅ Recording actions in intent tracks: breaking, placing, crafting and eating are saved in the session file (version 2; version 1 files still load) as the target block, the recipe or the meal, on the tick they happened; a replay performs them again and records them in its own new session, and keys and mouse do not act while it runs
- ✅ Mobs under events: a `Mobs` store with ids, and spawning, hits, removal (a kill, a drop for distance, or a clear at respawn) as events through the one `events::apply` door; they are logged as passive actions, so `Z` steps over them, and undoing them in reverse restores the mobs exactly; the saved log is version 6
- ✅ Redo and unloaded chunks: `Y` is logged as its own kind of action (log version 7), so the redo stack is rebuilt from the log at startup and survives a restart; a rewind or redo in a chunk that was unloaded now loads that chunk first (generated terrain plus its logged edits) instead of being refused
- ✅ World interface for scripts, second slice: a spawn-mob command (new `Spawn` permission, in range, in the open, at most 8 mobs alive, ids never reused) and reads through `scripting::service::ask` (new `Look` permission): look at a block, find the nearest block of a kind within 16, count an item, health, hunger and mobs in range; reads change and log nothing and count toward the per-tick limit
- ✅ Script costs: accepted script commands burn fuel (a craftable item) and start a per-kind wait; refused ones cost nothing; each accepted command is one edit that `Z` takes back whole
- ✅ Scripts in the window: `T` opens a panel of four ready-made scripts with their fuel cost; a click starts one, and a runner plays it out one command per ready tick, waiting out cooldowns and stopping on any other refusal; nothing runs during a replay
- ✅ Saved meters: health and hunger are written to `stats.mcst` on exit and loaded at startup; the drain clock is not saved
- ✅ More script reads: `Facing` (six directions, a whole-number code) and `NearestMob` (the block cell of the closest mob in range)

---

## 🔥 High Priority (Critical)

- **Tech stack: Rust + `wgpu` + `winit`, no engine** – Decided. Own game loop, world, meshing and simulation. Native first, with WASM possible later.
- **Deterministic simulation** – The tick loop, player physics and whole-number intent recording are in place; mob AI must run on the tick too and draw from `Simulation::stream` only, so replays always match.
- **Saving intent tracks** – Six slices done (see Completed): the file format, streaming to numbered session files, replay at startup, a limit of 20 kept sessions, recording actions, and recording hits and rewinds. A replay is exact as long as the world starts as it did when recorded; it does not yet save or restore the starting world itself.

---

## 🟡 Medium Priority (Important)

- **Time rewind** – Done so far (see Completed): `Z` undoes one action at a time, `Y` brings it back (and still can after a restart), the log keeps the last 256 actions rewindable, and edits in unloaded chunks can be rewound. Still to do:
  - keeping the folded-away history (an archive file) for a timelapse or timeline forks; today it is discarded;
  - a redo of a meal after a restart can still be refused: the meters are saved now, but not the clock that times the next hunger drain;
  - a scripted command is undone with its fuel refunded, which makes rewind a free refund until "Time as a resource" gives it a price;
  - undoing what is not an event yet: walking;
  - a rewind of a meal is refused (and logged) when the undo would push a meter out of 0 to 20, for example right after eating at near-zero hunger.
- **Local rewind zones** – Rewind only a region, for example to undo a creeper blast or reset a farm.
- **Time as a resource** – Rewinding costs a crafted fuel, so it is a tool and not a free undo button.
- **World interface for scripts** – Four slices done (see Completed): teleport, give item, set block and spawn a mob, each logged with its old value, checked for permission and range, capped per tick, and paid for with fuel and a wait; reads (look, find, count, health, hunger, mobs in range, facing, nearest mob) that change and log nothing; and a script panel that plays ready-made scripts. Still to do:
  - the other event-less changes: breaking, placing, crafting, eating, hits, hunger drain, respawn and mob spawns, hits and removals are events now; still without events are walking and mob movement, so rewind cannot yet undo them;
  - more reads: a list of mobs with their kinds (there is only one kind so far), and a read cost tied to time as a resource;
  - reads in the window: `run` is called by the script panel, but nothing calls `ask` yet, and the camera's facing is not passed in;
  - scripts are not recorded in intent tracks, so a replay does not run them and drifts from a run that used them;
  - grants that survive a restart: the inventory is still saved as a snapshot, so replaying slot events at startup would apply them twice.
- **Code as gameplay (scripting)** – A beginner-friendly language for players aged about 13–25, many of them non-developers, that can influence almost anything the world interface allows. Decided so far:
  - High-level and domain-specific, with whole numbers only, a step budget per tick, and no way to freeze the game.
  - Our own language, grown in stages, so errors can be friendly and a running script can be stepped through and rewound. An embedded Rhai or Lua engine with generated text from blocks is the fallback if the schedule slips.
  - Blocks first (clickable, no syntax errors), reusing the crafting-panel click pattern. Blocks and text are the same program, so a typed view can come later.
  - Name still open; "Nibble" is the leading candidate.
- **Quick-hacks** – Watch Dogs-style actions. Aim at a thing or a place and pick a hack from a menu (unlock, distract, go to this coordinate). Each hack is a ready-made script a player can open, read and edit, so players learn by using code first and writing it later. Two ways to trigger one:
  - a specific command aimed at a target, which the command line reads as, for example, "go" to that coordinate;
  - the command-line menu, fetching what the player asks for.
- **Command line and command dictionary** – An in-game command line that runs commands and quick-hacks, with a dictionary listing every command. Commands use a vim-like grammar, a verb plus a noun, that can grow beyond verbs and nouns (modifiers, counts, targets) where needed, so commands are easy to remember and combine.
- **Programmable mobs** – Craft a "brain" item and script its behavior: follow, guard, farm, haul. The best first use of scripting.
- **Logic and automation** – Wires, sensors and triggers that scripts can read and write.

---

## 🟢 Low Priority (Nice‑to‑Have)

- **Shareable blueprints** – Export a build or script as a text blob that others can import and watch build step by step.
- **Spectate and replay mode** – Watch a world's history as a timelapse.
- **Shaped crafting grid** – A 2×2 or 3×3 grid with position-dependent recipes, built on the recipe table.
- **Script-driven crafting** – Recipes defined as functions, so players can create and share new ones.

---

## 🧭 Long‑Term Vision

- **Timelines and branching** – Fork the world at any past moment and play both versions. Merge or discard a branch.
- **Ghost replays** – Your past self replays a recorded loop as an NPC, for co-op puzzles and automated crews.
- **Paradox rules** – Rewinding an item you have since used creates a conflict you have to resolve.
- **Server rules as scripts** – Servers set world rules (gravity, day length) in a script. Needs multiplayer.

---

## 🎯 Next Actions (Immediate)

1. **Time rewind, next slice** – Keep the folded-away history in an archive file.
2. **Time as a resource** – Rewinding costs fuel, using the cost model scripts already have.

---

## 📋 Ranked Backlog

Recommended order, row 1 first.

| # | Item | Why now | Difficulty |
|---|---|---|---|
| 1 | Archive file for folded-away history | Unlocks the timelapse and timeline forks | M |
| 2 | Time as a resource (rewind costs fuel) | Uses the fuel and wait model scripts now have; stops rewind refunding scripted fuel | M |
| 3 | Reads in the window (call `ask` with the camera's facing) | Reads exist and nothing calls them | S |
| 4 | Record script runs in intent tracks | A replay of a run that used scripts drifts | M |
| 5 | Local rewind zones | Reuses chunk-level undo, region-scoped | H |
| 6 | Command line and command dictionary | Needed before quick-hacks and the language | H |
| 7 | Scripting language, blocks first | Needs the world interface, cost model and command line | H |
| 8 | Quick-hacks | Ready-made scripts triggered from the command line; the script panel is the base | M |
| 9 | Programmable mobs | Best first use of scripting; needs mob AI on the tick | H |
| 10 | Walking and mob movement as events | Lets rewind undo them; the last gap in events | H |
| 11 | Save the starting world for replays | Makes intent-track replays exact from any world | M |
| 12 | Logic and automation (wires, sensors, triggers) | Scripts need things to read and write | H |
| 13 | Fix: refused meal rewind at near-zero hunger | Small edge case in rewind | S |
| 14 | Shaped crafting grid | Builds on the recipe table | M |
| 15 | Script-driven crafting | Needs the scripting language | M |
| 16 | Shareable blueprints | Needs scripts and the event log | M |
| 17 | Spectate and replay mode | Needs the archive file from item 1 | M |

---

## Notes

- The two headline features pair well. If every edit is a logged event, rewind comes almost free, and scripted blocks can replay deterministically.
- For scripting, build our own beginner-friendly language on top of the world interface; embedding Rhai or Lua is the fallback. The power and the risk live in the world interface, not in the syntax.
- Keep combat and resource management simple until rewind and scripting work.
