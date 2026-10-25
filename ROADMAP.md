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

---

## 🔥 High Priority (Critical)

- **Tech stack: Rust + `wgpu` + `winit`, no engine** – Decided. Own game loop, world, meshing and simulation. Native first, with WASM possible later.
- **Deterministic simulation** – Make physics, mob AI and world generation use `FixedClock` and forked `Rng` streams only, so replays always match.
- **Block registry** – Block names, colours and properties in one table, replacing the fixed `BlockId` constants and the colour `match`.
- **Resource management** – Inventory, item stacks, gathering and basic survival stats.
- **Crafting** – Recipe system and crafting interface.
- **Combat** – Health, damage, basic hostile mobs and weapons.

---

## 🟡 Medium Priority (Important)

- **Time rewind** – Scrub backward through the event log to undo builds. Needs periodic snapshots with events in between to keep the log bounded.
- **Local rewind zones** – Rewind only a region, for example to undo a creeper blast or reset a farm.
- **Time as a resource** – Rewinding costs a crafted fuel, so it is a tool and not a free undo button.
- **Code as gameplay (scripting)** – Embed a sandboxed language with an instruction budget per tick, so a bad script cannot freeze the game.
- **Programmable mobs** – Craft a "brain" item and script its behavior: follow, guard, farm, haul. The best first use of scripting.
- **Logic and automation** – Wires, sensors and triggers that scripts can read and write.

---

## 🟢 Low Priority (Nice‑to‑Have)

- **Shareable blueprints** – Export a build or script as a text blob that others can import and watch build step by step.
- **Spectate and replay mode** – Watch a world's history as a timelapse.
- **Script-driven crafting** – Recipes defined as functions, so players can create and share new ones.

---

## 🧭 Long‑Term Vision

- **Timelines and branching** – Fork the world at any past moment and play both versions. Merge or discard a branch.
- **Ghost replays** – Your past self replays a recorded loop as an NPC, for co-op puzzles and automated crews.
- **Paradox rules** – Rewinding an item you have since used creates a conflict you have to resolve.
- **Server rules as scripts** – Servers set world rules (gravity, day length) in a script. Needs multiplayer.

---

## 🎯 Next Actions (Immediate)

1. **Deterministic simulation** – Make physics, mob AI and world generation use `FixedClock` and forked `Rng` streams only.

---

## Notes

- The two headline features pair well. If every edit is a logged event, rewind comes almost free, and scripted blocks can replay deterministically.
- For scripting, embed a sandboxed language (Lua or Rhai in Rust, or a restricted JS runtime in the browser).
- Keep combat and resource management simple until rewind and scripting work.
