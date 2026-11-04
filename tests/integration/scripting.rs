use glam::Vec3;
use minecraft_clone::events::apply::{Stage, StageError, apply, apply_all, charge_events};
use minecraft_clone::events::log::{ActionKind, EventLog};
use minecraft_clone::events::persist::{decode, encode};
use minecraft_clone::events::redo::RedoStack;
use minecraft_clone::events::rewind::{REWIND_BASE_FUEL, log_action, rewind_events, rewind_price};
use minecraft_clone::inventory::model::Inventory;
use minecraft_clone::items::registry::ItemId;
use minecraft_clone::logic::model::{Anchor, CHECK_EVERY, REST_TICKS, Rules};
use minecraft_clone::mobs::model::Mobs;
use minecraft_clone::player::model::{Player, PlayerError};
use minecraft_clone::scripting::edit::Editor;
use minecraft_clone::scripting::library;
use minecraft_clone::scripting::model::{
    Answer, Command, Cooldowns, Facing, Limits, Pace, Permission, Permissions, Query,
};
use minecraft_clone::scripting::program::{Compare, Read, Test};
use minecraft_clone::scripting::runner::{Step, step};
use minecraft_clone::scripting::service::{ask, run};
use minecraft_clone::shared::clock::Tick;
use minecraft_clone::shared::event::{Event, MobId};
use minecraft_clone::shared::voxel::{BlockId, WorldPos};
use minecraft_clone::simulation::action::Action;
use minecraft_clone::simulation::intent_log::IntentLog;
use minecraft_clone::simulation::persist as intent_persist;
use minecraft_clone::survival::model::Stats;
use minecraft_clone::world::model::World;
use proptest::prelude::*;

fn cell(x: i32, y: i32, z: i32) -> WorldPos {
    WorldPos { x, y, z }
}

fn floor() -> World {
    let mut world = World::default();
    for x in -20..20 {
        for z in -20..20 {
            let event = Event::BlockSet {
                pos: cell(x, 0, z),
                old: BlockId::AIR,
                new: BlockId::STONE,
            };
            assert_eq!(world.apply(&event), Ok(()));
        }
    }
    world
}

/// A pack with fuel in it, since every script command burns some.
fn fuelled(fuel: u32) -> Inventory {
    let mut inventory = Inventory::default();
    assert!(
        inventory.add(ItemId::FUEL, fuel).is_ok(),
        "fuel is a defined item"
    );
    inventory
}

/// More fuel than any session below can burn.
const FUEL: u32 = 900;

fn command() -> impl Strategy<Value = Command> {
    prop_oneof![
        (-20i32..20, 1i32..3, -20i32..20)
            .prop_map(|(x, y, z)| Command::Teleport { to: cell(x, y, z) }),
        (1u16..4, 1u16..40).prop_map(|(item, count)| Command::GiveItem {
            item: ItemId(item),
            count
        }),
        ((-20i32..20, 0i32..3, -20i32..20), 0u16..4).prop_map(|((x, y, z), id)| {
            Command::SetBlock {
                pos: cell(x, y, z),
                block: BlockId(id),
            }
        }),
        (-20i32..20, 1i32..3, -20i32..20)
            .prop_map(|(x, y, z)| Command::SpawnMob { at: cell(x, y, z) }),
    ]
}

proptest! {
    #[test]
    fn a_scripted_session_survives_a_save_and_replays_to_the_same_game(
        ticks in prop::collection::vec(prop::collection::vec(command(), 0..10), 0..8),
    ) {
        let granted = Permissions::NONE
            .with(Permission::Move)
            .with(Permission::Give)
            .with(Permission::Build)
            .with(Permission::Spawn);
        let mut world = floor();
        let mut player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap();
        let mut inventory = fuelled(FUEL);
        let mut stats = Stats::default();
        let mut mobs = Mobs::default();
        let mut next_mob = MobId::FIRST;
        let mut log = EventLog::new();
        let mut cooldowns = Cooldowns::default();
        for (index, commands) in ticks.iter().enumerate() {
            let mut stage = Stage {
                world: &mut world,
                player: &mut player,
                inventory: &mut inventory,
                stats: &mut stats,
                mobs: &mut mobs,
            };
            let mut pace = Pace {
                tick: Tick(index as u64 * 60),
                cooldowns: &mut cooldowns,
            };
            run(
                commands,
                granted,
                Limits::standard(),
                &mut pace,
                &mut stage,
                &mut log,
                &mut next_mob,
            );
        }

        // A restart: only the file survives, and the log is replayed onto the baseline.
        let saved = decode(&encode(117, &log)).unwrap();
        prop_assert_eq!(&saved.log, &log);
        let mut replayed_world = floor();
        let mut replayed_player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap();
        let mut replayed_inventory = fuelled(FUEL);
        let mut replayed_stats = Stats::default();
        let mut replayed_mobs = Mobs::default();
        for event in saved.log.iter() {
            let mut stage = Stage {
                world: &mut replayed_world,
                player: &mut replayed_player,
                inventory: &mut replayed_inventory,
                stats: &mut replayed_stats,
                mobs: &mut replayed_mobs,
            };
            prop_assert_eq!(apply(&mut stage, event), Ok(()));
        }

        prop_assert_eq!(replayed_world, world);
        prop_assert_eq!(replayed_player, player);
        prop_assert_eq!(replayed_inventory, inventory);
        prop_assert_eq!(replayed_mobs, mobs);
    }
}

/// The whole game state a script command can change, so two runs can be compared.
struct Game {
    world: World,
    player: Player,
    inventory: Inventory,
    stats: Stats,
    mobs: Mobs,
    next_mob: MobId,
    log: EventLog,
    cooldowns: Cooldowns,
}

impl Game {
    fn new() -> Result<Self, PlayerError> {
        Ok(Self {
            world: floor(),
            player: Player::try_new(Vec3::new(0.5, 1.0, 0.5))?,
            inventory: fuelled(FUEL),
            stats: Stats::default(),
            mobs: Mobs::default(),
            next_mob: MobId::FIRST,
            log: EventLog::new(),
            cooldowns: Cooldowns::default(),
        })
    }

    fn apply_all(&mut self, events: &[Event]) -> Result<(), StageError> {
        let mut stage = Stage {
            world: &mut self.world,
            player: &mut self.player,
            inventory: &mut self.inventory,
            stats: &mut self.stats,
            mobs: &mut self.mobs,
        };
        apply_all(&mut stage, events)
    }

    /// Offers `command` on `tick` and reports whether it ran.
    fn offer(&mut self, tick: Tick, command: Command) -> bool {
        let mut stage = Stage {
            world: &mut self.world,
            player: &mut self.player,
            inventory: &mut self.inventory,
            stats: &mut self.stats,
            mobs: &mut self.mobs,
        };
        let mut pace = Pace {
            tick,
            cooldowns: &mut self.cooldowns,
        };
        let results = run(
            &[command],
            Permissions::ALL,
            Limits::standard(),
            &mut pace,
            &mut stage,
            &mut self.log,
            &mut self.next_mob,
        );
        results == [Ok(())]
    }
}

proptest! {
    /// Many commands are refused (cooling down, no room, blocked), and only those that ran are
    /// recorded; offering exactly those to a fresh game reproduces the run.
    #[test]
    fn recorded_script_commands_replay_to_the_same_game(
        offers in prop::collection::vec((0u64..40, command()), 0..60),
    ) {
        let mut live = Game::new().unwrap();
        let mut recording = IntentLog::default();
        let mut tick = 0;
        for (gap, command) in offers {
            tick += gap;
            if live.offer(Tick(tick), command) {
                prop_assert_eq!(
                    recording.record_action(Tick(tick), Action::Script(command)),
                    Ok(())
                );
            }
        }

        let saved = intent_persist::decode(&intent_persist::encode(&recording)).unwrap();
        prop_assert_eq!(&saved, &recording);
        let mut replayed = Game::new().unwrap();
        for entry in saved.actions() {
            let Action::Script(command) = entry.action else {
                panic!("only script commands were recorded");
            };
            prop_assert!(replayed.offer(entry.tick, command), "{:?} was refused", entry);
        }

        prop_assert_eq!(replayed.world, live.world);
        prop_assert_eq!(replayed.player, live.player);
        prop_assert_eq!(replayed.inventory, live.inventory);
        prop_assert_eq!(replayed.mobs, live.mobs);
        prop_assert_eq!(replayed.log, live.log);
    }
}

#[test]
fn a_priced_rewind_keeps_the_fuel_a_command_burned_and_a_redo_gives_the_price_back() {
    let mut game = Game::new().unwrap();
    let command = Command::SetBlock {
        pos: cell(3, 1, 3),
        block: BlockId::STONE,
    };
    assert!(game.offer(Tick(0), command));
    let (world_after, pack_after) = (game.world.clone(), game.inventory.clone());

    let mut undo = rewind_events(&game.log).unwrap();
    let price = rewind_price(&undo);
    assert_eq!(price, REWIND_BASE_FUEL + u32::from(command.fuel_cost()));
    undo.extend(charge_events(&game.inventory, &undo, ItemId::FUEL, price).unwrap());
    assert_eq!(game.apply_all(&undo), Ok(()));
    log_action(&mut game.log, &undo, ActionKind::Rewind);

    // The block is gone, and the pack is poorer than before the script by only the base price:
    // the command's own fuel was not refunded.
    assert_eq!(game.world.block_at(cell(3, 1, 3)), BlockId::AIR);
    assert_eq!(
        game.inventory.count_of(ItemId::FUEL),
        FUEL - u32::from(command.fuel_cost()) - REWIND_BASE_FUEL
    );

    // A redo is the exact inverse, so it lands on the game as it was after the script.
    let mut redo = RedoStack::default();
    redo.record(&undo);
    let again = redo.take().unwrap();
    assert_eq!(game.apply_all(&again), Ok(()));
    assert_eq!(game.world, world_after);
    assert_eq!(game.inventory, pack_after);
}

#[test]
fn a_pack_without_the_price_cannot_rewind() {
    let mut game = Game::new().unwrap();
    game.inventory = Inventory::default();
    let undo = [Event::BlockSet {
        pos: cell(0, 1, 0),
        old: BlockId::AIR,
        new: BlockId::STONE,
    }];
    assert!(charge_events(&game.inventory, &undo, ItemId::FUEL, rewind_price(&undo)).is_err());
}

#[test]
fn a_script_can_find_a_block_and_then_build_next_to_it() {
    let mut world = floor();
    let mut player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap();
    let (mut inventory, mut stats, mut mobs) = (fuelled(10), Stats::default(), Mobs::default());
    let mut log = EventLog::new();
    let mut next_mob = MobId::FIRST;
    let granted = Permissions::NONE
        .with(Permission::Look)
        .with(Permission::Build);
    let marker = Event::BlockSet {
        pos: cell(4, 1, 0),
        old: BlockId::AIR,
        new: BlockId::DIRT,
    };
    assert_eq!(world.apply(&marker), Ok(()));

    let mut stage = Stage {
        world: &mut world,
        player: &mut player,
        inventory: &mut inventory,
        stats: &mut stats,
        mobs: &mut mobs,
    };
    let answers = ask(
        &[Query::Find {
            block: BlockId::DIRT,
        }],
        granted,
        Limits::standard(),
        Facing::East,
        &stage,
    );
    let Ok(Answer::Found(Some(found))) = answers[0] else {
        panic!("the dirt block is in reach, got {answers:?}");
    };
    assert_eq!(found, cell(4, 1, 0));
    assert!(log.is_empty());

    let next_to = cell(found.x + 1, found.y, found.z);
    let mut cooldowns = Cooldowns::default();
    let mut pace = Pace {
        tick: Tick(0),
        cooldowns: &mut cooldowns,
    };
    let results = run(
        &[Command::SetBlock {
            pos: next_to,
            block: BlockId::STONE,
        }],
        granted,
        Limits::standard(),
        &mut pace,
        &mut stage,
        &mut log,
        &mut next_mob,
    );
    assert_eq!(results, [Ok(())]);
    // The fuel the command burned, then the block it set.
    assert_eq!(log.len(), 2);
}

/// A quick-hack anchored on a block plays out over ticks, pays its fuel, logs one edit per
/// command, and the log's inverse puts the world and the pack back exactly.
#[test]
fn a_hack_runs_to_the_end_and_undoes_exactly() {
    let mut world = floor();
    let mut player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).expect("a valid spawn");
    let mut inventory = fuelled(FUEL);
    let (mut stats, mut mobs) = (Stats::default(), Mobs::default());
    let mut log = EventLog::new();
    let mut next_mob = MobId::FIRST;
    let mut cooldowns = Cooldowns::default();
    let before = (world.clone(), inventory.clone());
    let (_, pillar) = library::find("pillar").expect("the pillar hack exists");
    let anchor = cell(3, 0, 3);
    let mut run = Some(pillar.start(anchor));
    let mut finished = false;

    for tick in 0..200 {
        let Some(current) = run.take() else { break };
        let mut stage = Stage {
            world: &mut world,
            player: &mut player,
            inventory: &mut inventory,
            stats: &mut stats,
            mobs: &mut mobs,
        };
        let mut pace = Pace {
            tick: Tick(tick),
            cooldowns: &mut cooldowns,
        };
        match step(
            current,
            Permissions::ALL,
            Limits::standard(),
            &mut pace,
            &mut stage,
            &mut log,
            &mut next_mob,
        ) {
            Step::Ran { rest, .. } => run = rest,
            Step::Waiting { run: waiting, .. } => run = Some(waiting),
            Step::Finished { .. } => finished = true,
            Step::Stopped { reason, .. } => panic!("the hack stopped: {reason}"),
        }
    }

    assert!(finished, "the hack ends");
    for y in 1..=3 {
        assert_eq!(world.block_at(cell(3, y, 3)), BlockId::STONE);
    }
    assert_eq!(inventory.count_of(ItemId::FUEL), FUEL - pillar.fuel_cost());
    assert_eq!(log.actions(), 3);

    let mut undo: Vec<_> = log.iter().map(|event| event.inverse()).collect();
    undo.reverse();
    let mut stage = Stage {
        world: &mut world,
        player: &mut player,
        inventory: &mut inventory,
        stats: &mut stats,
        mobs: &mut mobs,
    };
    assert_eq!(apply_all(&mut stage, &undo), Ok(()));
    assert_eq!((world, inventory), before);
}

/// An edited program in a rule: the rule waits for its test, runs what the editor made, rests,
/// and does not fire again once the test no longer holds. Every command it ran is in the log
/// and undoes exactly.
#[test]
fn a_rule_runs_an_edited_program_when_its_test_holds_and_undoes_exactly() {
    let mut world = floor();
    let mut player = Player::try_new(Vec3::new(0.5, 1.0, 0.5)).expect("a valid spawn");
    let mut inventory = fuelled(FUEL);
    let (mut stats, mut mobs) = (Stats::default(), Mobs::default());
    let mut log = EventLog::new();
    let mut next_mob = MobId::FIRST;
    let mut cooldowns = Cooldowns::default();
    let before = (world.clone(), inventory.clone());

    // "stock" gives 10 dirt; the player raises that to 12 in the editor.
    let (_, stock) = library::find("stock").expect("the stock script exists");
    let mut editor = Editor::new(None, stock.program());
    editor.select(0);
    editor.next_field();
    assert_eq!(editor.field_name(), Some("count"));
    assert!(editor.nudge(2));
    let program = editor
        .program()
        .expect("an edited draft that is still valid");

    let no_dirt = Test {
        read: Read::Count { item: ItemId::DIRT },
        compare: Compare::Below,
        than: 1,
    };
    let mut rules = Rules::default();
    let id = rules
        .add(no_dirt, program, Anchor::Cell(cell(0, 1, 0)))
        .expect("room for a rule");

    let mut script = None;
    let mut fired = Vec::new();
    for tick in 0..(REST_TICKS + 2 * CHECK_EVERY) {
        let mut stage = Stage {
            world: &mut world,
            player: &mut player,
            inventory: &mut inventory,
            stats: &mut stats,
            mobs: &mut mobs,
        };
        if script.is_none() {
            let mut read = |query| {
                ask(
                    &[query],
                    Permissions::ALL,
                    Limits::standard(),
                    Facing::North,
                    &stage,
                )
                .into_iter()
                .next()
                .expect("one answer per query")
            };
            let mut resolve = |anchor| match anchor {
                Anchor::Cell(at) => Some(at),
                Anchor::Feet | Anchor::Mob => None,
            };
            if rules.due(tick, &mut resolve, &mut read) == Some(id) {
                rules.fired(id, tick);
                fired.push(tick);
                let rule = rules.get(id).expect("the rule is there");
                script = Some(minecraft_clone::scripting::runner::ScriptRun::from_program(
                    "rule",
                    rule.program(),
                    cell(0, 1, 0),
                ));
            }
        }
        let Some(current) = script.take() else {
            continue;
        };
        let mut pace = Pace {
            tick: Tick(tick),
            cooldowns: &mut cooldowns,
        };
        match step(
            current,
            Permissions::ALL,
            Limits::standard(),
            &mut pace,
            &mut stage,
            &mut log,
            &mut next_mob,
        ) {
            Step::Ran { rest, .. } => script = rest,
            Step::Waiting { run, .. } => script = Some(run),
            Step::Finished { .. } => {}
            Step::Stopped { reason, .. } => panic!("the rule's program stopped: {reason}"),
        }
    }

    // It fired once; the dirt it gave made the test false, so it never fired again.
    assert_eq!(fired, [0]);
    assert_eq!(inventory.count_of(ItemId::DIRT), 12);

    let mut undo: Vec<_> = log.iter().map(|event| event.inverse()).collect();
    undo.reverse();
    let mut stage = Stage {
        world: &mut world,
        player: &mut player,
        inventory: &mut inventory,
        stats: &mut stats,
        mobs: &mut mobs,
    };
    assert_eq!(apply_all(&mut stage, &undo), Ok(()));
    assert_eq!((world, inventory), before);
}
