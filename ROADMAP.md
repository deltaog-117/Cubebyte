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

---

## 🔥 High Priority (Critical)

- **Tech stack: Rust + `wgpu` + `winit`, no engine** – Decided. Own game loop, world, meshing and simulation. Native first, with WASM possible later.
- **Deterministic simulation** – The tick loop, player physics and whole-number intent recording are in place; mob AI must run on the tick too and draw from `Simulation::stream` only, so replays always match.
- **Saving intent tracks** – The intent log lives in memory only. Give it a versioned file format like `world.mcel`, and bound its growth (about 2 MB an hour with one zombie), before replays are saved.

---

## 🟡 Medium Priority (Important)

- **Time rewind** – First slice done (see Completed): `Z` undoes one action at a time. Still to do:
  - periodic snapshots with events in between, to keep the log bounded (every rewind adds events);
  - a forward step (redo), which could keep the undone actions in memory;
  - undoing what is not an event yet: walking, hunger, health and respawn (a rewind refused after a respawn, because the pack no longer matches, is logged and skipped);
  - rewinding edits in chunks that are not loaded.
- **Local rewind zones** – Rewind only a region, for example to undo a creeper blast or reset a farm.
- **Time as a resource** – Rewinding costs a crafted fuel, so it is a tool and not a free undo button.
- **World interface for scripts** – First slice done (see Completed): teleport, give item and set block, each logged with its old value, checked for permission and range, and capped per tick. Still to do:
  - spawning mobs, which needs mobs under the event log (today there is one `Option<Zombie>` in `app`);
  - the other event-less changes: breaking, placing, crafting and eating are events now; still without events are the hunger and health meters (eating restores hunger outside the events), respawn, and walking and mob movement, so rewind cannot yet undo them;
  - reads (look, find, query);
  - a cost per action (fuel or energy) and a cooldown, tied to time as a resource;
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

1. **World interface for scripts, next slice** – Put mobs under the event log so scripts can spawn them, add reads and a cost per action, then bring the meters (hunger, health) and respawn under events. The language comes after.
2. **Time rewind, next slice** – Add periodic snapshots to keep the log bounded, then a forward step.

---

## Notes

- The two headline features pair well. If every edit is a logged event, rewind comes almost free, and scripted blocks can replay deterministically.
- For scripting, build our own beginner-friendly language on top of the world interface; embedding Rhai or Lua is the fallback. The power and the risk live in the world interface, not in the syntax.
- Keep combat and resource management simple until rewind and scripting work.
