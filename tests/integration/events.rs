use glam::Vec3;
use minecraft_clone::building::model::SolidBlock;
use minecraft_clone::building::service::{break_block, place_block};
use minecraft_clone::crafting::registry::RecipeId;
use minecraft_clone::crafting::service::{craft, craft_events};
use minecraft_clone::events::apply::{Stage, apply, apply_all};
use minecraft_clone::events::log::{ActionKind, EventLog};
use minecraft_clone::events::persist::{decode, encode};
use minecraft_clone::events::redo::RedoStack;
use minecraft_clone::events::replay::replay_chunk;
use minecraft_clone::events::rewind::{inverse_run, log_action, rewind_events};
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::inventory::service::{
    gather_block, gather_events, place_events, place_from_stock,
};
use minecraft_clone::player::model::Player;
use minecraft_clone::shared::event::Event;
use minecraft_clone::shared::voxel::{BlockId, ChunkPos, WorldPos};
use minecraft_clone::survival::model::Stats;
use minecraft_clone::survival::service::{eat, eat_events};
use minecraft_clone::world::generation::{MAX_HEIGHT, MIN_HEIGHT, TerrainGenerator};
use minecraft_clone::world::model::World;
use proptest::prelude::*;

fn chunk_range() -> impl Iterator<Item = ChunkPos> {
    let (low, high) = (MIN_HEIGHT.div_euclid(16), MAX_HEIGHT.div_euclid(16));
    (-1..=1).flat_map(move |x| {
        (-1..=1).flat_map(move |z| (low..=high).map(move |y| ChunkPos { x, y, z }))
    })
}

fn terrain_world(terrain: &TerrainGenerator) -> World {
    let mut world = World::default();
    for pos in chunk_range() {
        assert_eq!(
            world.insert_generated(pos, terrain.generate_chunk(pos)),
            Ok(())
        );
    }
    world
}

proptest! {
    #[test]
    fn a_saved_log_rebuilds_the_edited_world_chunk_by_chunk(
        seed in any::<u64>(),
        edits in prop::collection::vec(
            (any::<bool>(), (-1.0f32..1.0, -1.0f32..0.0, -1.0f32..1.0)),
            0..40,
        ),
    ) {
        let terrain = TerrainGenerator::new(seed);
        let mut live = terrain_world(&terrain);
        let mut log = EventLog::new();
        let eye = Vec3::new(8.0, terrain.height_at(8, 8) as f32 + 3.0, 8.0);
        for (placing, (x, y, z)) in edits {
            let direction = Vec3::new(x, y, z);
            let _ = if placing {
                place_block(&mut live, &mut log, eye, direction, SolidBlock::STONE)
            } else {
                break_block(&mut live, &mut log, eye, direction)
            };
        }

        // A restart: only the file survives. Streaming then replays each chunk on its own.
        let saved = decode(&encode(seed, &log)).unwrap();
        prop_assert_eq!(saved.seed, seed);
        let terrain = TerrainGenerator::new(saved.seed);
        let mut restored = World::default();
        for pos in chunk_range() {
            prop_assert_eq!(restored.insert_generated(pos, terrain.generate_chunk(pos)), Ok(()));
            prop_assert_eq!(replay_chunk(&mut restored, &saved.log, pos), Ok(()));
        }
        prop_assert_eq!(restored, live);
    }
}

// One step of play, as the event path and the direct path each see it.
#[derive(Clone, Copy, Debug)]
enum Play {
    Gather(f32, f32, f32),
    Place(f32, f32, f32),
    Craft(u8),
    Eat,
}

fn play() -> impl Strategy<Value = Play> {
    prop_oneof![
        (-1.0f32..1.0, -1.0f32..0.0, -1.0f32..1.0).prop_map(|(x, y, z)| Play::Gather(x, y, z)),
        (-1.0f32..1.0, -1.0f32..0.0, -1.0f32..1.0).prop_map(|(x, y, z)| Play::Place(x, y, z)),
        (0u8..4).prop_map(Play::Craft),
        Just(Play::Eat),
    ]
}

proptest! {
    // Breaking, placing, crafting and eating through events reach exactly the state the
    // direct functions reach, and undoing the logged events in reverse restores the start.
    #[test]
    fn gameplay_through_events_matches_the_direct_path_and_undoes_exactly(
        seed in any::<u64>(),
        steps in prop::collection::vec(play(), 0..40),
    ) {
        let terrain = TerrainGenerator::new(seed);
        let start = terrain_world(&terrain);
        let eye = Vec3::new(0.5, 40.0, 0.5);
        let mut stats_direct = Stats::default();
        for _ in 0..600 * 10 {
            stats_direct.step();
        }
        let mut stats_events = stats_direct;
        let (mut world_direct, mut log_direct, mut pack_direct) =
            (start.clone(), EventLog::new(), Inventory::default());
        let mut world = start.clone();
        let mut pack = Inventory::default();
        let mut player = Player::try_new(eye).unwrap();
        let mut applied: Vec<Event> = Vec::new();

        for step in steps {
            let planned = match step {
                Play::Gather(x, y, z) => {
                    let dir = Vec3::new(x, y, z);
                    let direct = gather_block(&mut world_direct, &mut log_direct, &mut pack_direct, eye, dir).ok();
                    (direct.is_some(), gather_events(&world, &pack, eye, dir).ok().map(|(_, e)| (e, 0)))
                }
                Play::Place(x, y, z) => {
                    let dir = Vec3::new(x, y, z);
                    let direct = place_from_stock(&mut world_direct, &mut log_direct, &mut pack_direct, eye, dir, SolidBlock::STONE).ok();
                    (direct.is_some(), place_events(&world, &pack, eye, dir, SolidBlock::STONE).ok().map(|(_, e)| (e, 0)))
                }
                Play::Craft(id) => {
                    let direct = craft(&mut pack_direct, RecipeId(id)).ok();
                    (direct.is_some(), craft_events(&pack, RecipeId(id)).ok().map(|e| (e, 0)))
                }
                Play::Eat => {
                    let direct = eat(&mut pack_direct, &mut stats_direct).ok();
                    let meal = eat_events(&pack, &stats_events).ok();
                    let restored = meal.as_ref().map_or(0, |m| m.restored);
                    (direct.is_some(), meal.map(|m| (m.events, restored)))
                }
            };
            // Both paths accept or refuse the same steps.
            prop_assert_eq!(planned.0, planned.1.is_some());
            if let Some((events, restored)) = planned.1 {
                let mut stage = Stage { world: &mut world, player: &mut player, inventory: &mut pack };
                prop_assert_eq!(apply_all(&mut stage, &events), Ok(()));
                stats_events.restore_hunger(restored);
                applied.extend(events);
            }
            prop_assert_eq!(&world, &world_direct);
            prop_assert_eq!(&pack, &pack_direct);
            prop_assert_eq!(stats_events, stats_direct);
        }

        for event in applied.iter().rev() {
            let mut stage = Stage { world: &mut world, player: &mut player, inventory: &mut pack };
            prop_assert_eq!(apply(&mut stage, &event.inverse()), Ok(()));
        }
        prop_assert_eq!(world, start);
        prop_assert_eq!(pack, Inventory::default());
    }
}

/// One action: a few blocks set to new values, each remembering what it replaced.
fn action_events(world: &mut World, action: &[(i32, i32, i32, u16)]) -> Vec<Event> {
    action
        .iter()
        .map(|&(x, y, z, new)| {
            let pos = WorldPos { x, y, z };
            let event = Event::BlockSet {
                pos,
                old: world.block_at(pos),
                new: BlockId(new),
            };
            assert_eq!(world.apply(&event), Ok(()));
            event
        })
        .collect()
}

fn action() -> impl Strategy<Value = Vec<(i32, i32, i32, u16)>> {
    prop::collection::vec(
        (-16i32..32, MIN_HEIGHT..=MAX_HEIGHT, -16i32..32, 0u16..4),
        1..4,
    )
}

proptest! {
    // A long game that keeps folding its oldest history away still restarts into the same
    // world, chunk by chunk, through a save.
    #[test]
    fn a_compacted_saved_log_rebuilds_the_edited_world(
        seed in any::<u64>(),
        actions in prop::collection::vec(action(), 0..30),
        keep in 0usize..32,
    ) {
        let terrain = TerrainGenerator::new(seed);
        let mut live = terrain_world(&terrain);
        let mut log = EventLog::new();
        for step in &actions {
            let events = action_events(&mut live, step);
            log_action(&mut log, &events, ActionKind::Edit);
        }
        log.compact(keep);

        let saved = decode(&encode(seed, &log)).unwrap();
        prop_assert_eq!(&saved.log, &log);
        let mut restored = World::default();
        for pos in chunk_range() {
            prop_assert_eq!(restored.insert_generated(pos, terrain.generate_chunk(pos)), Ok(()));
            prop_assert_eq!(replay_chunk(&mut restored, &saved.log, pos), Ok(()));
        }
        prop_assert_eq!(restored, live);
    }

    // Rewind and redo walk one line of history both ways: the world always matches the
    // state after the same number of actions, and a restart from the log agrees.
    #[test]
    fn rewind_and_redo_walk_the_history_both_ways(
        actions in prop::collection::vec(action(), 1..8),
        moves in prop::collection::vec(any::<bool>(), 0..24),
    ) {
        let mut world = World::default();
        let (mut log, mut redo) = (EventLog::new(), RedoStack::default());
        let mut after_each = vec![World::default()];
        for step in &actions {
            let events = action_events(&mut world, step);
            log_action(&mut log, &events, ActionKind::Edit);
            after_each.push(world.clone());
        }
        let mut at = actions.len();

        for rewinding in moves {
            if rewinding {
                if let Some(events) = rewind_events(&log) {
                    for event in &events {
                        prop_assert_eq!(world.apply(event), Ok(()));
                    }
                    log_action(&mut log, &events, ActionKind::Rewind);
                    redo.record(&events);
                    at -= 1;
                } else {
                    prop_assert_eq!(at, 0);
                }
            } else if let Some(events) = redo.take() {
                for event in &events {
                    prop_assert_eq!(world.apply(event), Ok(()));
                }
                log_action(&mut log, &events, ActionKind::Edit);
                at += 1;
            } else {
                prop_assert_eq!(at, actions.len());
            }
            prop_assert_eq!(&world, &after_each[at]);
        }

        let mut replayed = World::default();
        for event in log.iter() {
            prop_assert_eq!(replayed.apply(event), Ok(()));
        }
        prop_assert_eq!(replayed, world);
    }
}

#[test]
fn inverting_a_run_twice_gives_the_run_back() {
    let mut world = World::default();
    let events = action_events(&mut world, &[(0, 0, 0, 1), (0, 0, 0, 2), (1, 0, 0, 3)]);

    assert_eq!(inverse_run(&inverse_run(&events)), events);
}
