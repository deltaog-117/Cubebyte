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
- ✅ Reads in the window: `Q` shows a live line in the title (the way you face, the block under the crosshair, the nearest mob and how many are near), made of the same queries a script would send through `ask`; it changes and logs nothing, so it also works during a replay
- ✅ Script runs in intent tracks: every command a script really ran is saved in the session file as its own action (version 4; versions 1 to 3 still load) and a replay performs it again at the same point of the tick, so a replay of a run that used scripts no longer drifts; the script itself is not saved, so edits to the ready-made scripts cannot change an old recording
- ✅ Time as a resource: `Z` costs fuel, a base of 1 plus whatever fuel the undo would hand back, paid inside the rewind's own all-or-nothing batch; undoing a scripted command no longer refunds its fuel, a pack that cannot pay refuses the rewind and the title says what it needs, and `Y` is the exact inverse so it gives the price back with the edit
- ✅ Archive of folded-away history: the log hands over the events it folds, with their action ends, and the game appends them on exit to `world.mcea` (after the world file is saved, so nothing is written twice); a frame cut short by a crash is dropped on the next append
- ✅ Local rewind zones: key `X` puts the blocks in a cube of radius 8 around the player back as they were 1, 3, 10, 30 or 100 block-changing actions ago (`V` picks), as one edit that `Z` takes back whole; it costs 1 fuel plus 1 per started group of 4 blocks restored; blocks only, so the pack and mobs are untouched; saved in intent tracks (version 5) so a replay restores the same region
- ✅ Command line and command dictionary: `/` opens a typed line and `H` opens it with a table of every command (Tab shows or hides the table); clicking a row runs a command that needs no words or starts one that does; commands are `help`, `give`, `go`, `set`, `spawn`, `rewind`, `redo` and `zone`, defined once in a table that the parser, the dictionary and the error messages all read; positions are blocks from the feet; script commands wait for the next tick and keep the script rules (permission, range, fuel, wait), so a replay reproduces them
- ✅ Scripting language, first slice (blocks as data): a script is now a `Program`, a checked tree of blocks (do a command, shift the working cell, repeat 1 to 16 times, if a read is below, equal to or above a number); nesting is at most 4 deep and 64 blocks, every loop has a fixed count, and a stepper looks at no more than 32 blocks per tick and 4096 per run, so a program cannot freeze the game and a run that goes on too long is stopped with a message; the four ready-made scripts were rewritten as programs and still give the same commands; commands still go through the same permission, range, fuel and wait checks, so a replay is unchanged
- ✅ Quick-hacks: a hack is a program anchored on something: the player's feet, the block under the crosshair or the nearest mob; key `G` opens a menu of the hacks that fit what you aim at (`pillar`, `perch`, `dig` for a block, `cage` for a mob), and the new `hack <name>` command runs any script from the command line; the `T` panel lists only the scripts that need no aim, and a `ward` script shows an `if`
- ✅ Rewind of a meal at near-zero hunger: a rewind's hunger and health changes are cut down to what the meters can still move by (`clamp_meters`), so undoing a meal after a drain lands hunger on 0 instead of being refused; what is logged is what really moved, so `Y` still restores a valid state
- ✅ Distance fog: far terrain fades into the sky colour, computed per pixel in the shader from a checked `Fog` range (half the load radius to the full radius), so chunks arrive hidden in the haze; the interface is never fogged. Per-face shading and the sky colour were already in place
- ✅ Vertex ambient occlusion: every corner of a face is darkened by how many solid blocks crowd it (two beside it and one diagonal), as its own value that the shader applies, so strength can be tuned and later lighting can treat it separately; each quad is cut along its more shut-in diagonal, and an edit re-meshes the whole 3×3×3 cube of chunks around it. Meshing a surface chunk got about 44% slower (157 to 227 µs)
- ✅ Starting world saved for replays: each session saves the world, pack and meters it began with in `intents/NNNN.start/`, and `--replay N` begins from that copy, so a replay is exact from any world; a session that began as a replay never writes over the live saves, and sessions recorded before this fall back to the live saves with a warning
- ✅ Rules, first slice: a rule is a test plus a program fixed on a cell; once a second the first rule that is on, resting no longer and true fires as an ordinary script (permission, range, fuel and wait checks, logged, recorded in intent tracks, undone whole by `Z`), then rests 10 seconds; at most 8 rules, never during a replay; `N` opens a panel to switch, edit, remove or add one; rules live for the session
- ✅ Block editor, first slice: `B` opens a ready-made script or a rule's program as an indented list of blocks (a rule's test is its first line); click a line, pick a field, step it less or more, add a block of a chosen kind, delete, run the draft, make a rule of it or save to the rule; the draft can only become a checked program and refuses growth past 64 blocks or 4 levels; edits last for the session

- ✅ Saved programs, rules and player recipes (text format): `programs.nib` is a readable, versioned file (`nib 1`) holding every rule and every recipe a player wrote; scripts, rules and recipes share one line-based grammar (`build stone 0 1 0`, `repeat 3 {`, `if hunger below 8 {`), read all-or-nothing with the line to fix, and the game leaves a file it refused alone on exit; the file is copied into each session's starting world so replays use the same recipes
- ✅ Rule anchors: a rule is anchored on a fixed cell, the player's feet, or the nearest mob (a rules panel button steps through them); the choice is saved with the rule
- ✅ Script-driven crafting, first slice: a player recipe is a program of `take`, `make`, `repeat` and `if` that works on a copy of the pack and so does all of its steps or none; items have a worth, and the worth rule (every path through a recipe makes no more worth than it takes) is checked when the recipe is built; recipes join the crafting panel and the `R` and `C` keys, are crafted as ordinary slot events (so `Z` undoes them) and recorded in intent tracks by their number

---

## 🔥 High Priority (Critical)

- **Tech stack: Rust + `wgpu` + `winit`, no engine** – Decided. Own game loop, world, meshing and simulation. Native first, with WASM possible later.
- **Deterministic simulation** – The tick loop, player physics and whole-number intent recording are in place; mob AI must run on the tick too and draw from `Simulation::stream` only, so replays always match.
- **Saving intent tracks** – Seven slices done (see Completed): the file format, streaming to numbered session files, replay at startup, a limit of 20 kept sessions, recording actions, recording hits and rewinds, and recording script commands. Each session also saves the world it started from, and a replay begins from it, so a replay is exact from any world (sessions recorded before that fall back to the live saves).

---

## 🟡 Medium Priority (Important)

- **Time rewind** – Done so far (see Completed): `Z` undoes one action at a time for fuel, `Y` brings it back (and still can after a restart), the log keeps the last 256 actions rewindable, edits in unloaded chunks can be rewound, and the history the log folds away is kept in `world.mcea`. Still to do:
  - reading the archive back for a timelapse or timeline forks: the file is written and can be decoded, but nothing plays it yet;
  - the archive is appended on exit only, so a crash loses that session's folded history (the world file is lost with it); appending as it folds needs a counter in the world file to heal a mismatch;
  - a redo of a meal after a restart can still be refused: the meters are saved now, but not the clock that times the next hunger drain;
  - `Z` then `Y` costs nothing, because the redo gives the price back; keeping the price on a redo needs a redo that rebases the slot events it replays;
  - the price is flat plus returned fuel; a price that grows with how old the undone action is would need ticks in the log;
  - undoing what is not an event yet: walking.
- **Local rewind zones** – First slice done (see Completed). Still to do: marking a region by two corners (needs the command line), and zones that also restore the pack and mobs.
- **World interface for scripts** – Five slices done (see Completed): teleport, give item, set block and spawn a mob, each logged with its old value, checked for permission and range, capped per tick, and paid for with fuel and a wait; reads (look, find, count, health, hunger, mobs in range, facing, nearest mob) that change and log nothing; a script panel that plays ready-made scripts; a readout of the reads in the window (`Q`); and script commands recorded in intent tracks. Still to do:
  - the other event-less changes: breaking, placing, crafting, eating, hits, hunger drain, respawn and mob spawns, hits and removals are events now; still without events are walking and mob movement, so rewind cannot yet undo them;
  - more reads: a list of mobs with their kinds (there is only one kind so far), and a read cost tied to time as a resource;
  - scripts cannot use reads: the window shows them to the player, but a script plays a fixed list of commands and cannot branch on an answer until the language exists;
  - grants that survive a restart: the inventory is still saved as a snapshot, so replaying slot events at startup would apply them twice.
- **Code as gameplay (scripting)** – Saving is done (see Completed: `programs.nib`, and the text grammar is the typed view's start). Under review: the design of the clickable block editor is being thought through again before any code (Kogama-style blocks with a matching text view of the same program, so a player can use either; the text view would round-trip with the blocks). First slice done (see Completed): the program tree, its stepper and the ready-made scripts built on it; the first slice of the clickable block editor is done too. Still to do: edits to the ready-made scripts in the editor are still not saved (rules and recipes are), a typed view inside the game, real drag and drop and typed numbers in the editor, a typed view of the same tree, variables and more reads and commands as the world interface grows. A beginner-friendly language for players aged about 13–25, many of them non-developers, that can influence almost anything the world interface allows. Decided so far:
  - High-level and domain-specific, with whole numbers only, a step budget per tick, and no way to freeze the game.
  - Our own language, grown in stages, so errors can be friendly and a running script can be stepped through and rewound. An embedded Rhai or Lua engine with generated text from blocks is the fallback if the schedule slips.
  - Blocks first (clickable, no syntax errors), reusing the crafting-panel click pattern. Blocks and text are the same program, so a typed view can come later.
  - Name still open; "Nibble" is the leading candidate.
  - Design goal, reach: the one language for everything players can script, doing the work of both Java mods and redstone in Minecraft and of Lua in Roblox Studio. It runs inside the game and reaches everything the world interface exposes; the interface is the real boundary, so a new thing in the game gets reads and commands and the language reaches it with no change to the language. Redstone-style logic (see Logic and automation) is built on top of it.
  - Design goal, frontend: it is also the friendly front end to the deeper game functions, so players who never write code still use it, through blocks, the command line, quick-hacks and ready-made scripts. Each of those is Nibble underneath, so a no-coder can open what they used, read it and edit it, and learn by using.
  - Prerequisite: walking and mob movement must become events first (see the backlog), or scripts cannot drive or undo them.
  - Design principles, more granular than Roblox and easier than Kogama (Kogama places and configures ready-made objects, with a ceiling; Roblox scripts against a huge API in a separate app, with a wall):
    - every action is an event with a fixed shape, and each one is a block, a command and a hook point, so a script can react to or change anything down to one block or one meter point, with the same few verbs meaning the same everywhere;
    - one thing at three zoom levels: click a ready-made hack, open it as blocks, open it as text; there is no ceiling and never a blank page;
    - open the hood on anything: every ready-made object or hack shows its script, so players learn from what they already use;
    - built in, not a separate Studio: players edit while playing, in the same window, and see the change at once;
    - rewind is the debugger: `Z` undoes a script run, and a script can be stepped through and replayed, so trying risky changes is cheap;
    - friendly errors, whole numbers only and a step budget, so scripts cannot freeze the game or fail in confusing ways;
    - scope by region, chunk, mob or block kind (builds on local rewind zones), so "only here" is simple to say;
    - keep early costs (fuel and waits) low and tunable, so the limits keep scripts fair without punishing small experiments.
- **Quick-hacks** – First slice done (see Completed). Still to do: aiming at a mob with the crosshair (a hack uses the nearest mob in range today), opening a hack to read and edit it (waits for the block editor), and more hacks as commands grow. Watch Dogs-style actions. Aim at a thing or a place and pick a hack from a menu (unlock, distract, go to this coordinate). Each hack is a ready-made script a player can open, read and edit, so players learn by using code first and writing it later. Two ways to trigger one:
  - a specific command aimed at a target, which the command line reads as, for example, "go" to that coordinate;
  - the command-line menu, fetching what the player asks for.
- **Command line and command dictionary** – First slice done (see Completed); `hack` was added with quick-hacks. Still to do: more commands as features arrive (craft, eat, hits), command history with the arrow keys, aiming (`go` to the block under the crosshair), and the vim-like grammar growth (counts, modifiers, targets).
- **Programmable mobs** – Craft a "brain" item and script its behavior: follow, guard, farm, haul. The best first use of scripting.
- **Logic and automation** – Two slices done (see Completed): rules, and saved rules with anchors on a cell, the feet or the nearest mob. Still to do: rules that react to an event and not only to a reading polled once a second, a visible form in the world, and wires and sensor blocks if rules prove not enough (they would need a new block kind, a signal pass and a save format).

---

## 🟢 Low Priority (Nice‑to‑Have)

- **Shareable blueprints** – Export a build or script as a text blob that others can import and watch build step by step.
- **Spectate and replay mode** – Watch a world's history as a timelapse.
- **Block shapes (moulding)** – Let players shape blocks beyond cubes. The meaning is still open: preset shapes (slab, stair, slope), stretching a block into a box, or sculpting cells inside a block. It touches storage, meshing and collision, and custom shapes must be saved so old event logs still replay, so it needs its own COAs first.
- **Shaped crafting grid** – A 2×2 or 3×3 grid with position-dependent recipes, built on the recipe table.
- **Script-driven crafting** – First slice done (see Completed). Still to do: making recipes in the game, recipes that read more of the world, and sharing them (see Shareable blueprints).
- **Graphics upgrade** – Today blocks are flat colours with no lighting. Done after the current items, in this order. Light and shading are computed from world state and never logged, so replay and rewind are untouched; the mesher keeps producing plain vertex data with no `wgpu` types, so these stay testable without a GPU.
  - ✅ per-face shading (top brighter, sides darker) and ✅ sky colour and distance fog (the Kogama-style bright, simple look); a sun direction is still open;
  - ✅ vertex ambient occlusion (darkening is applied in the shader and applies to all light for now; when sun and block light exist it should touch only ambient light);
  - texture atlas with nearest-neighbour sampling (the Minecraft pixel look; needs an art style first);
  - sky gradient or skybox with a sun and clouds;
  - translucent water and glass in a separate sorted pass;
  - block light and sky light, flood-filled per chunk (day and night, caves, torches);
  - real-time shadow mapping from the sun (the Roblox look);
  - post-processing: bloom, tone mapping, anti-aliasing;
  - better mobs: models built from parts, with animation.

---

## 🧭 Long‑Term Vision

- **Timelines and branching** – Fork the world at any past moment and play both versions. Merge or discard a branch.
- **Ghost replays** – Your past self replays a recorded loop as an NPC, for co-op puzzles and automated crews.
- **Paradox rules** – Rewinding an item you have since used creates a conflict you have to resolve.
- **Server rules as scripts** – Servers set world rules (gravity, day length) in a script. Needs multiplayer.

---

## 🎯 Next Actions (Immediate)

1. **Making recipes in the game** – Player recipes can only be written in `programs.nib` by hand. A recipe editor (the block editor with `take` and `make`) or a `recipe` command would let a player make one without leaving the window.
2. **Saving edits to ready-made scripts** – The editor's changes to a ready-made script still vanish on exit; `programs.nib` could hold `script <name>` sections.

---

## 📋 Ranked Backlog

Recommended order, row 1 first.

| # | Item | Why now | Difficulty |
|---|---|---|---|
| 1 | Making recipes and saving script edits in the game | The text file exists; players still have to write recipes by hand | M |
| 2 | Programmable mobs | Best first use of scripting; needs mob AI on the tick | H |
| 3 | Walking and mob movement as events | Lets rewind undo them; the last gap in events | H |
| 4 | Logic and automation, next slices | Event-driven rules and a visible form; anchors are done | M |
| 5 | Shareable blueprints | The text grammar is the export format; needs a paste-in import | S |
| 6 | Shaped crafting grid | Builds on the recipe table | M |
| 7 | Block shapes (moulding) | Needs COAs first on what shape means and how custom shapes are saved | H |
| 8 | Spectate and replay mode | The archive file exists now; it needs a viewer that plays it back | M |
| 9 | Graphics: texture atlas | Needs an art style chosen first | M |
| 10 | Graphics: skybox, clouds, translucent water and glass | Builds on the atlas and the sky | M |
| 11 | Graphics: block and sky light | Flood-fill per chunk; unlocks day and night | H |
| 12 | Graphics: shadow mapping and post-processing | Roblox-style polish; costliest, last | H |
| 13 | Graphics: better mobs | Parts-based models and animation | M |

---

## Notes

- The two headline features pair well. If every edit is a logged event, rewind comes almost free, and scripted blocks can replay deterministically.
- For scripting, build our own beginner-friendly language on top of the world interface; embedding Rhai or Lua is the fallback. The power and the risk live in the world interface, not in the syntax.
- Keep combat and resource management simple until rewind and scripting work.
