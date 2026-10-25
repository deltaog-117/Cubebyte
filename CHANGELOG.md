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

---
