use glam::Vec3;
use minecraft_clone::building::model::SolidBlock;
use minecraft_clone::building::service::{break_block, place_block};
use minecraft_clone::combat::model::Damage;
use minecraft_clone::combat::service::{hit_events, respawn_events};
use minecraft_clone::crafting::registry::RecipeId;
use minecraft_clone::crafting::service::{craft, craft_events};
use minecraft_clone::events::apply::{Stage, apply, apply_all, slot_events};
use minecraft_clone::events::log::{ActionKind, EventLog};
use minecraft_clone::events::persist::{decode, encode};
use minecraft_clone::events::redo::RedoStack;
use minecraft_clone::events::replay::replay_chunk;
use minecraft_clone::events::rewind::{inverse_run, log_action, rewind_events};
use minecraft_clone::events::walk::{bridged, checkpoints};
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::inventory::service::{
    gather_block, gather_events, place_events, place_from_stock,
};
use minecraft_clone::items::registry::ItemId;
use minecraft_clone::mobs::model::Mobs;
use minecraft_clone::mobs::service::{clear_events, despawn_event, spawn_event, strike_events};
use minecraft_clone::player::model::Player;
use minecraft_clone::player::service::teleport;
use minecraft_clone::shared::event::{Event, MobId, Spot};
use minecraft_clone::shared::voxel::{BlockId, ChunkPos, WorldPos};
use minecraft_clone::survival::model::{Meter, Stats};
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
        let stats_start = stats_direct;
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
                    (direct.is_some(), gather_events(&world, &pack, eye, dir).ok().map(|(_, e)| e))
                }
                Play::Place(x, y, z) => {
                    let dir = Vec3::new(x, y, z);
                    let direct = place_from_stock(&mut world_direct, &mut log_direct, &mut pack_direct, eye, dir, SolidBlock::STONE).ok();
                    (direct.is_some(), place_events(&world, &pack, eye, dir, SolidBlock::STONE).ok().map(|(_, e)| e))
                }
                Play::Craft(id) => {
                    let direct = craft(&mut pack_direct, RecipeId(id)).ok();
                    (direct.is_some(), craft_events(&pack, RecipeId(id)).ok())
                }
                Play::Eat => {
                    let direct = eat(&mut pack_direct, &mut stats_direct).ok();
                    let meal = eat_events(&pack, &stats_events).ok();
                    (direct.is_some(), meal.map(|m| m.events))
                }
            };
            // Both paths accept or refuse the same steps.
            prop_assert_eq!(planned.0, planned.1.is_some());
            if let Some(events) = planned.1 {
                let mut stage = Stage { world: &mut world, player: &mut player, inventory: &mut pack, stats: &mut stats_events, mobs: &mut Mobs::default() };
                prop_assert_eq!(apply_all(&mut stage, &events), Ok(()));
                applied.extend(events);
            }
            prop_assert_eq!(&world, &world_direct);
            prop_assert_eq!(&pack, &pack_direct);
            prop_assert_eq!(stats_events, stats_direct);
        }

        for event in applied.iter().rev() {
            let mut stage = Stage { world: &mut world, player: &mut player, inventory: &mut pack, stats: &mut stats_events, mobs: &mut Mobs::default() };
            prop_assert_eq!(apply(&mut stage, &event.inverse()), Ok(()));
        }
        prop_assert_eq!(world, start);
        prop_assert_eq!(pack, Inventory::default());
        prop_assert_eq!(stats_events, stats_start);
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

/// The parts of the game events change, with the log they are written to.
struct Game {
    world: World,
    player: Player,
    pack: Inventory,
    stats: Stats,
    mobs: Mobs,
    log: EventLog,
}

impl Game {
    fn with(player: Player) -> Self {
        Self {
            world: World::default(),
            player,
            pack: Inventory::default(),
            stats: Stats::default(),
            mobs: Mobs::default(),
            log: EventLog::new(),
        }
    }

    /// Commits as the game does: the checkpoints are logged but not applied, since the player
    /// and the mobs already stand where they say.
    fn commit(&mut self, events: &[Event], kind: ActionKind) {
        let walked = checkpoints(&self.log, &self.player, &self.mobs, events, kind);
        let mut stage = Stage {
            world: &mut self.world,
            player: &mut self.player,
            inventory: &mut self.pack,
            stats: &mut self.stats,
            mobs: &mut self.mobs,
        };
        assert_eq!(apply_all(&mut stage, events), Ok(()));
        let logged: Vec<Event> = walked.iter().chain(events).copied().collect();
        log_action(&mut self.log, &logged, kind);
    }

    /// The player walks to `x`: no event, as the physics step does it.
    fn walk_to(&mut self, x: f32) {
        assert!(
            teleport(&mut self.player, Vec3::new(x, 3.0, 0.5)).is_ok(),
            "{x} is not a position the player can walk to"
        );
    }

    fn spot(&self) -> Spot {
        Spot::from_vec3(self.player.position())
    }

    /// An edit that sets the block at `x` to `new`.
    fn block_edit(&self, x: i32, new: u16) -> Vec<Event> {
        let pos = WorldPos { x, y: 0, z: 0 };
        vec![Event::BlockSet {
            pos,
            old: self.world.block_at(pos),
            new: BlockId(new),
        }]
    }

    /// `Z`: undoes the latest live edit, first walking the player to where it left them.
    fn rewind(&mut self, redo: &mut RedoStack) -> bool {
        let Some(events) = rewind_events(&self.log) else {
            return false;
        };
        let events = bridged(events, self.spot());
        self.commit(&events, ActionKind::Rewind);
        redo.record(&events);
        true
    }

    /// `Y`: brings back the edit `Z` undid last.
    fn redo(&mut self, redo: &mut RedoStack) -> bool {
        let Some(events) = redo.take() else {
            return false;
        };
        let events = bridged(events, self.spot());
        self.commit(&events, ActionKind::Redo);
        true
    }
}

// An edit records where the player stood. Walking on does not change what `Z` does: it walks
// them back to where the log last had them, undoes the edit, and `Y` is the exact opposite,
// including where the player was standing when `Z` was pressed.
#[test]
fn rewind_walks_the_player_back_and_redo_undoes_that_exactly() {
    let mut game = Game::with(Player::try_new(Vec3::new(0.5, 3.0, 0.5)).unwrap());
    let mut redo = RedoStack::default();
    let start = game.spot();
    let wall = WorldPos { x: 1, y: 0, z: 0 };
    let bare = game.world.block_at(wall);

    let first = game.block_edit(0, 1);
    game.commit(&first, ActionKind::Edit);
    game.walk_to(4.5);
    let second = game.block_edit(1, 2);
    game.commit(&second, ActionKind::Edit);
    game.walk_to(9.5);
    let pressed_at = game.spot();

    assert!(game.rewind(&mut redo));
    assert_eq!(game.spot(), start);
    assert_eq!(game.world.block_at(wall), bare);

    game.walk_to(20.5);
    let before_redo = game.spot();
    assert!(game.redo(&mut redo));
    assert_eq!(game.world.block_at(wall), BlockId(2));
    assert_eq!(game.spot(), pressed_at);

    // The redo is an edit like any other, and `Z` is its exact opposite: the block goes and
    // the player is back where they stood before pressing `Y`.
    game.walk_to(1.5);
    assert!(game.rewind(&mut redo));
    assert_eq!(game.spot(), before_redo);
    assert_eq!(game.world.block_at(wall), bare);
}

// A log that was saved and read back puts the redo stack, and the walks inside it, back too.
#[test]
fn the_walk_in_a_rewound_edit_survives_a_save() {
    let mut game = Game::with(Player::try_new(Vec3::new(0.5, 3.0, 0.5)).unwrap());
    let mut redo = RedoStack::default();
    let first = game.block_edit(0, 1);
    game.commit(&first, ActionKind::Edit);
    game.walk_to(6.5);
    let second = game.block_edit(1, 2);
    game.commit(&second, ActionKind::Edit);
    game.walk_to(12.5);
    assert!(game.rewind(&mut redo));

    let saved = decode(&encode(7, &game.log)).unwrap().log;
    assert_eq!(saved, game.log);
    assert_eq!(RedoStack::from_log(&saved), redo);
}

// A mob that walked on is checkpointed when it is hit, so the log says where it was hit, and
// undoing the whole history in reverse puts it, and then nothing, back exactly.
#[test]
fn a_mob_that_walked_is_checkpointed_when_hit_and_undoes_exactly() {
    let mut game = Game::with(Player::try_new(Vec3::new(0.5, 3.0, 0.5)).unwrap());
    let id = MobId::FIRST;
    let spawn = spawn_event(id, Vec3::new(5.0, 3.0, 5.0));
    game.commit(&[spawn], ActionKind::Passive);
    let from = Spot::from_vec3(Vec3::new(5.0, 3.0, 5.0));
    let to = Spot::from_vec3(Vec3::new(2.0, 3.0, 1.0));
    game.mobs.move_to(id, from, to).unwrap();

    let hit = strike_events(id, game.mobs.get(id).unwrap(), Damage::try_new(4).unwrap());
    game.commit(&hit, ActionKind::Passive);
    assert!(game.log.iter().any(|event| {
        *event
            == Event::MobMoved {
                id,
                old: from,
                new: to,
            }
    }));
    assert_eq!(game.log.mob_spot(id), Some(to));

    let logged: Vec<Event> = game.log.iter().copied().collect();
    for event in inverse_run(&logged) {
        let mut stage = Stage {
            world: &mut game.world,
            player: &mut game.player,
            inventory: &mut game.pack,
            stats: &mut game.stats,
            mobs: &mut game.mobs,
        };
        assert_eq!(apply(&mut stage, &event), Ok(()));
    }
    assert_eq!(game.mobs, Mobs::default());
}

proptest! {
    // However the player walks between edits and between undos, undoing every edit leaves the
    // world as it began and the player where the first edit found them, and redoing every one
    // brings back the world after the last.
    #[test]
    fn walking_between_rewinds_and_redos_never_stops_them_working(
        edits in prop::collection::vec((-30i32..30, 1u16..4, -50.0f32..50.0), 1..8),
        paths in prop::collection::vec(-50.0f32..50.0, 16),
    ) {
        let mut game = Game::with(Player::try_new(Vec3::new(0.5, 3.0, 0.5)).unwrap());
        let mut redo = RedoStack::default();
        let mut first_spot = None;
        for &(x, block, at) in &edits {
            game.walk_to(at);
            first_spot.get_or_insert(game.spot());
            let events = game.block_edit(x, block);
            game.commit(&events, ActionKind::Edit);
        }
        let after = game.world.clone();

        for (index, _) in edits.iter().enumerate() {
            game.walk_to(paths[index]);
            prop_assert!(game.rewind(&mut redo));
        }
        prop_assert_eq!(&game.world, &World::default());
        prop_assert_eq!(Some(game.spot()), first_spot);

        for (index, _) in edits.iter().enumerate() {
            game.walk_to(paths[8 + index]);
            prop_assert!(game.redo(&mut redo));
        }
        prop_assert_eq!(&game.world, &after);
    }
}

// A meal is an edit; the hunger that drains and the zombie hit that land after it are
// passive. `Z` undoes the meal only, leaving both in place, and a respawn closes the history:
// nothing before it can be rewound.
#[test]
fn rewind_undoes_a_meal_across_passive_changes_and_stops_at_a_respawn() {
    let mut game = Game {
        world: World::default(),
        player: Player::try_new(Vec3::new(0.5, 3.0, 0.5)).unwrap(),
        pack: Inventory::default(),
        stats: Stats::default(),
        mobs: Mobs::default(),
        log: EventLog::new(),
    };
    for _ in 0..600 * 10 {
        game.stats.step();
    }
    game.pack.add(ItemId::BREAD, 2).unwrap();
    let (hungry, start_pack) = (game.stats, game.pack.clone());

    let meal = eat_events(&game.pack, &game.stats).unwrap();
    game.commit(&meal.events, ActionKind::Edit);
    game.commit(&[Event::HungerChanged { by: -1 }], ActionKind::Passive);
    let blow = hit_events(&game.stats, Damage::try_new(6).unwrap());
    game.commit(&blow, ActionKind::Passive);

    let undo = rewind_events(&game.log).unwrap();
    game.commit(&undo, ActionKind::Rewind);

    assert_eq!(game.pack, start_pack);
    assert_eq!(game.stats.hunger().value(), hungry.hunger().value() - 1);
    assert_eq!(game.stats.health().value(), hungry.health().value() - 6);
    assert_eq!(rewind_events(&game.log), None);

    let mut more = game.pack.clone();
    more.add(ItemId::DIRT, 3).unwrap();
    let gift = slot_events(&game.pack, &more);
    game.commit(&gift, ActionKind::Edit);
    assert!(rewind_events(&game.log).is_some());
    let over = respawn_events(&game.stats, &game.pack);
    game.commit(&over, ActionKind::Respawn);

    assert_eq!(game.stats.health(), Meter::FULL);
    assert_eq!(game.stats.hunger(), Meter::FULL);
    assert!(game.pack.is_empty());
    assert_eq!(rewind_events(&game.log), None);
}

// A zombie appears, is hit and is killed as passive events: `Z` steps over them, the saved
// log carries them without a restart bringing the zombie back, and undoing them in reverse
// puts the zombie back exactly as it was.
#[test]
fn mob_events_are_passive_survive_a_save_and_undo_exactly() {
    let mut game = Game {
        world: World::default(),
        player: Player::try_new(Vec3::new(0.5, 3.0, 0.5)).unwrap(),
        pack: Inventory::default(),
        stats: Stats::default(),
        mobs: Mobs::default(),
        log: EventLog::new(),
    };
    let id = MobId::FIRST;
    let mut more = game.pack.clone();
    more.add(ItemId::DIRT, 3).unwrap();
    let gift = slot_events(&game.pack, &more);
    game.commit(&gift, ActionKind::Edit);

    game.commit(
        &[spawn_event(id, Vec3::new(5.5, 1.0, 5.5))],
        ActionKind::Passive,
    );
    let spawned = game.mobs.clone();
    let hurt = strike_events(id, game.mobs.get(id).unwrap(), Damage::try_new(4).unwrap());
    game.commit(&hurt, ActionKind::Passive);
    let killed = strike_events(id, game.mobs.get(id).unwrap(), Damage::try_new(99).unwrap());
    game.commit(&killed, ActionKind::Passive);
    assert!(game.mobs.is_empty());

    // `Z` undoes the gift, not the zombie's life and death. The gift also logged where the
    // player stood (here the player never walked, so the move is of no length), and its undo
    // takes that back too.
    let undo = rewind_events(&game.log).unwrap();
    let stood = Event::PlayerMoved {
        old: game.spot(),
        new: game.spot(),
    };
    let edit: Vec<Event> = std::iter::once(stood).chain(gift.iter().copied()).collect();
    assert_eq!(undo, inverse_run(&edit));

    // A save and a reload keep every record, and only blocks are replayed into chunks.
    let saved = decode(&encode(117, &game.log)).unwrap();
    assert_eq!(saved.log, game.log);
    let mut chunk = World::default();
    assert_eq!(
        replay_chunk(&mut chunk, &saved.log, ChunkPos { x: 0, y: 0, z: 0 }),
        Ok(())
    );

    // Undone newest first, the zombie comes back as it was and then goes again.
    let mut back = hurt.clone();
    back.extend(killed.iter());
    for event in back.iter().rev() {
        game.commit(&[event.inverse()], ActionKind::Rewind);
    }
    assert_eq!(game.mobs.get(id).map(|z| z.health()), Some(20));
    assert_eq!(game.mobs.len(), spawned.len());
    let clear = clear_events(&game.mobs);
    game.commit(&clear, ActionKind::Respawn);
    assert!(game.mobs.is_empty());
    let drop = despawn_event(id, spawned.get(id).unwrap());
    assert!(matches!(drop, Event::MobRemoved { health: 20, .. }));
}

#[test]
fn the_archive_and_the_log_together_hold_the_whole_history_after_compaction() {
    use minecraft_clone::events::archive;
    use minecraft_clone::events::rewind::HISTORY_WINDOW;

    let path = std::env::temp_dir().join(format!("mcea-integration-{}.mcea", std::process::id()));
    let _ = std::fs::remove_file(&path);

    // `log` compacts as the game does; `whole` never does, so it is the history to match.
    let (mut log, mut whole) = (EventLog::new(), EventLog::new());
    let actions = 5 * HISTORY_WINDOW;
    for index in 0..actions {
        let event = Event::BlockSet {
            pos: WorldPos {
                x: i32::try_from(index).unwrap(),
                y: 0,
                z: 0,
            },
            old: BlockId::AIR,
            new: BlockId::STONE,
        };
        log_action(&mut log, &[event], ActionKind::Edit);
        whole.push(event);
        whole.end_action(ActionKind::Edit);
        // A first session ends part-way through; the second picks up from the same log.
        if index == 3 * HISTORY_WINDOW {
            archive::append(&path, 117, &log.take_folded()).unwrap();
        }
    }
    archive::append(&path, 117, &log.take_folded()).unwrap();

    let archived = archive::load(&path).unwrap().unwrap();
    assert_eq!(archived.frames.len(), 2);
    let rejoined: Vec<_> = archived.records().chain(log.records()).collect();
    assert_eq!(rejoined, whole.records().collect::<Vec<_>>());
    assert!(log.actions() < 2 * HISTORY_WINDOW);
    let _ = std::fs::remove_file(&path);
}
