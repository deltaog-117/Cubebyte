//! Stepping the game back one action.
//!
//! A rewind never removes anything from the log. It works out the events that undo the
//! latest edit that is still in effect; the caller applies them like any other action and
//! logs them as a [`ActionKind::Rewind`], so replay and a restart land on the rewound game.
//!
//! The log keeps only the last [`HISTORY_WINDOW`] actions as history; older ones are folded
//! away, so a rewind cannot reach past them.
//!
//! Rewinding costs fuel: [`rewind_price`] is a base price plus whatever fuel the rewind would
//! hand back, so undoing a scripted command does not refund the fuel it burned. The window
//! pays it inside the rewind's own events (see [`apply::charge_events`]), which makes a redo,
//! the exact inverse, give the price back along with the edit.
//!
//! [`apply::charge_events`]: crate::events::apply::charge_events
//!
//! An edit can sit in a chunk that has since been unloaded. [`touched_chunks`] names the chunks
//! a rewind (or a redo) changes, so the caller can load the missing ones first: a block event
//! applied to an unloaded chunk would be refused, or worse, would leave a stray chunk holding
//! only that block.

use std::collections::HashSet;

use crate::events::log::{ActionKind, EventLog};
use crate::items::registry::ItemId;
use crate::shared::event::{Event, Held};
use crate::shared::voxel::ChunkPos;
use crate::survival::model::{METER_MAX, Stats};

/// How many actions of history the log keeps rewindable. Compaction runs once the log holds
/// twice this many, so it happens once per window of play and not on every action.
pub const HISTORY_WINDOW: usize = 256;

/// Fuel every rewind costs, before the fuel it hands back is added.
pub const REWIND_BASE_FUEL: u32 = 1;

/// The fuel `events` put into the pack, net: what slots end up holding minus what they held.
/// A rewind of a change that burned fuel returns it; one of a change that made fuel returns
/// none, since that fuel is taken away instead.
pub fn fuel_returned(events: &[Event]) -> u32 {
    let fuel = |held: Held| i64::from(held.count) * i64::from(held.item == ItemId::FUEL.0);
    let net: i64 = events
        .iter()
        .map(|event| match *event {
            Event::SlotSet { old, new, .. } => fuel(new) - fuel(old),
            _ => 0,
        })
        .sum();
    u32::try_from(net.max(0)).unwrap_or(u32::MAX)
}

/// The fuel a rewind that applies `events` costs: [`REWIND_BASE_FUEL`] plus the fuel it hands
/// back, so the fuel a command burned stays burned.
pub fn rewind_price(events: &[Event]) -> u32 {
    REWIND_BASE_FUEL.saturating_add(fuel_returned(events))
}

/// The events that undo `events`, newest change first. Inverting the result gives `events`
/// back, which is how a rewound edit is replayed.
pub fn inverse_run(events: &[Event]) -> Vec<Event> {
    events.iter().rev().map(|event| event.inverse()).collect()
}

/// `events` with every meter change cut down to what the meters can still move by, so a rewind
/// that would push a meter out of `0..=METER_MAX` lands on the nearest end instead. A change
/// that cannot move the meter at all is dropped.
///
/// Passive drains are skipped by a rewind, so undoing a meal can ask hunger to fall further than
/// it can: the meal gave 4, a drain took 1, and the undo wants -4 from 3. The log records what
/// was really applied, so a redo (the inverse) still lands on a valid state.
pub fn clamp_meters(events: &[Event], stats: Stats) -> Vec<Event> {
    let (mut hunger, mut health) = (stats.hunger().value(), stats.health().value());
    let mut clamped = Vec::with_capacity(events.len());
    for &event in events {
        let (meter, by) = match event {
            Event::HungerChanged { by } => (&mut hunger, by),
            Event::HealthChanged { by } => (&mut health, by),
            other => {
                clamped.push(other);
                continue;
            }
        };
        let next = (i16::from(*meter) + i16::from(by)).clamp(0, i16::from(METER_MAX));
        // `next` and `*meter` are both in `0..=METER_MAX`, so their difference fits an `i8`.
        let moved = i8::try_from(next - i16::from(*meter)).unwrap_or(0);
        *meter = u8::try_from(next).unwrap_or(*meter);
        if moved != 0 {
            clamped.push(match event {
                Event::HungerChanged { .. } => Event::HungerChanged { by: moved },
                _ => Event::HealthChanged { by: moved },
            });
        }
    }
    clamped
}

/// The events that undo the latest edit still in effect, newest change first, or `None` when
/// nothing is left to undo.
pub fn rewind_events(log: &EventLog) -> Option<Vec<Event>> {
    Some(inverse_run(log.latest_live_edit()?))
}

/// The chunks whose blocks `events` change, each once, in the order they first appear.
pub fn touched_chunks(events: &[Event]) -> Vec<ChunkPos> {
    let mut seen = HashSet::new();
    events
        .iter()
        .filter_map(|event| event.chunk())
        .filter(|&chunk| seen.insert(chunk))
        .collect()
}

/// Logs `events` as one action of `kind`, then folds the oldest history away once it has
/// grown past twice [`HISTORY_WINDOW`].
pub fn log_action(log: &mut EventLog, events: &[Event], kind: ActionKind) {
    events.iter().for_each(|&event| log.push(event));
    log.end_action(kind);
    if log.actions() >= 2 * HISTORY_WINDOW {
        log.compact(HISTORY_WINDOW);
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::events::replay::replay_chunk;
    use crate::shared::voxel::{BlockId, ChunkPos, WorldPos};
    use crate::world::model::World;

    /// One action: a few blocks in one chunk set to new values.
    fn action() -> impl Strategy<Value = Vec<(i32, u16)>> {
        prop::collection::vec((0i32..4, 0u16..4), 1..4)
    }

    /// Records `action` against `world`, applies it and logs it as an edit.
    fn edit(world: &mut World, log: &mut EventLog, action: &[(i32, u16)]) {
        let events: Vec<Event> = action
            .iter()
            .map(|&(x, new)| {
                let pos = WorldPos { x, y: 0, z: 0 };
                let event = Event::BlockSet {
                    pos,
                    old: world.block_at(pos),
                    new: BlockId(new),
                };
                world.apply(&event).unwrap();
                event
            })
            .collect();
        log_action(log, &events, ActionKind::Edit);
    }

    fn rewind(world: &mut World, log: &mut EventLog) -> bool {
        let Some(events) = rewind_events(log) else {
            return false;
        };
        for event in &events {
            world.apply(event).unwrap();
        }
        log_action(log, &events, ActionKind::Rewind);
        true
    }

    fn replayed(log: &EventLog) -> World {
        let mut world = World::default();
        for event in log.iter() {
            world.apply(event).unwrap();
        }
        world
    }

    proptest! {
        #[test]
        fn compacting_keeps_the_newest_actions_rewindable_and_the_world_rebuildable(
            actions in prop::collection::vec(action(), 1..12),
            keep in 0usize..14,
        ) {
            let (mut world, mut log) = (World::default(), EventLog::new());
            let mut after_each = vec![World::default()];
            for step in &actions {
                edit(&mut world, &mut log, step);
                after_each.push(world.clone());
            }
            log.compact(keep);

            // A restart sees the base and the kept events, and lands on the same world.
            let chunk = ChunkPos { x: 0, y: 0, z: 0 };
            let mut restored = World::default();
            prop_assert_eq!(replay_chunk(&mut restored, &log, chunk), Ok(()));
            prop_assert_eq!(&restored, &world);

            let mut stepped = 0;
            while rewind(&mut world, &mut log) {
                stepped += 1;
            }
            prop_assert_eq!(stepped, keep.min(actions.len()));
            prop_assert_eq!(&world, &after_each[actions.len() - stepped]);

            // The rewinds are in the log too, so a restart still lands on the rewound world.
            let mut restored = World::default();
            prop_assert_eq!(replay_chunk(&mut restored, &log, chunk), Ok(()));
            prop_assert_eq!(restored, world);
        }

        #[test]
        fn rewinding_k_actions_restores_the_world_after_the_earlier_ones(
            actions in prop::collection::vec(action(), 1..8),
            back in 0usize..10,
        ) {
            let (mut world, mut log) = (World::default(), EventLog::new());
            let mut after_each = vec![World::default()];
            for step in &actions {
                edit(&mut world, &mut log, step);
                after_each.push(world.clone());
            }

            let mut stepped = 0;
            while stepped < back && rewind(&mut world, &mut log) {
                stepped += 1;
            }

            prop_assert_eq!(stepped, back.min(actions.len()));
            prop_assert_eq!(&world, &after_each[actions.len() - stepped]);
            // The log holds the whole story, so replaying it gives the same world.
            prop_assert_eq!(replayed(&log), world);
        }

        #[test]
        fn edits_after_a_rewind_are_themselves_rewindable(
            before in action(), undone in action(), after in action(),
        ) {
            let (mut world, mut log) = (World::default(), EventLog::new());
            edit(&mut world, &mut log, &before);
            let kept = world.clone();
            edit(&mut world, &mut log, &undone);
            rewind(&mut world, &mut log);
            edit(&mut world, &mut log, &after);

            rewind(&mut world, &mut log);
            prop_assert_eq!(&world, &kept);
        }
    }

    proptest! {
        #[test]
        fn a_rewind_applies_to_a_world_rebuilt_from_only_the_chunks_it_touches(
            actions in prop::collection::vec(
                prop::collection::vec((-40i32..40, 0u16..4), 1..4),
                1..6,
            ),
        ) {
            let (mut world, mut log) = (World::default(), EventLog::new());
            for step in &actions {
                edit(&mut world, &mut log, step);
            }
            let events = rewind_events(&log).unwrap();

            // Every other chunk is unloaded, as after the player walked away.
            let mut partial = World::default();
            let touched = touched_chunks(&events);
            for &chunk in &touched {
                prop_assert_eq!(replay_chunk(&mut partial, &log, chunk), Ok(()));
            }
            for event in &events {
                prop_assert_eq!(partial.apply(event), Ok(()));
                world.apply(event).unwrap();
                if let Event::BlockSet { pos, .. } = *event {
                    prop_assert_eq!(partial.block_at(pos), world.block_at(pos));
                }
            }
            let distinct: HashSet<_> = touched.iter().collect();
            prop_assert_eq!(distinct.len(), touched.len());
        }
    }

    fn slot(slot: u8, old: Held, new: Held) -> Event {
        Event::SlotSet { slot, old, new }
    }

    fn fuel_held(count: u8) -> Held {
        Held {
            item: ItemId::FUEL.0,
            count,
        }
    }

    #[test]
    fn a_rewind_costs_the_base_plus_the_fuel_it_hands_back() {
        let burn = [slot(0, fuel_held(7), fuel_held(2))];
        // The rewind of that burn puts the five back.
        assert_eq!(rewind_price(&inverse_run(&burn)), REWIND_BASE_FUEL + 5);
        // Redoing it takes them away again, which hands nothing back.
        assert_eq!(rewind_price(&burn), REWIND_BASE_FUEL);
        // Other items and other kinds of event never count.
        let stone = Held {
            item: ItemId::STONE.0,
            count: 9,
        };
        let others = [slot(1, Held::EMPTY, stone), Event::HungerChanged { by: 3 }];
        assert_eq!(rewind_price(&others), REWIND_BASE_FUEL);
        assert_eq!(rewind_price(&[]), REWIND_BASE_FUEL);
    }

    proptest! {
        #[test]
        fn the_price_is_the_base_plus_the_net_fuel_given_back(
            changes in prop::collection::vec((0u8..4, 0u8..64, 0u8..64), 0..12),
        ) {
            let events: Vec<Event> = changes
                .iter()
                .map(|&(at, old, new)| slot(at, fuel_held(old), fuel_held(new)))
                .collect();
            let net: i64 = changes.iter().map(|&(_, old, new)| i64::from(new) - i64::from(old)).sum();
            prop_assert_eq!(
                i64::from(rewind_price(&events)),
                i64::from(REWIND_BASE_FUEL) + net.max(0)
            );
            // Pricing the undo of an undo: what one hands back the other takes away.
            let back = inverse_run(&events);
            prop_assert_eq!(
                i64::from(fuel_returned(&events)) - i64::from(fuel_returned(&back)),
                net
            );
        }
    }

    fn stats_at(health: u8, hunger: u8) -> Stats {
        use crate::survival::model::Meter;
        Stats::with_meters(
            Meter::try_new(health).unwrap(),
            Meter::try_new(hunger).unwrap(),
        )
    }

    #[test]
    fn undoing_a_meal_after_a_drain_stops_at_zero() {
        // Hunger 0, a meal of +4, a drain of -1: hunger reads 3 and the undo asks for -4.
        let undo = [Event::HungerChanged { by: -4 }];
        assert_eq!(
            clamp_meters(&undo, stats_at(20, 3)),
            vec![Event::HungerChanged { by: -3 }]
        );
        // Nothing left to take: the change is dropped.
        assert!(clamp_meters(&undo, stats_at(20, 0)).is_empty());
    }

    proptest! {
        #[test]
        fn clamped_meter_events_always_apply_and_match_the_originals_when_they_fit(
            health in 0u8..=METER_MAX,
            hunger in 0u8..=METER_MAX,
            changes in prop::collection::vec((any::<bool>(), -25i8..25), 0..8),
        ) {
            let events: Vec<Event> = changes
                .iter()
                .map(|&(is_hunger, by)| if is_hunger {
                    Event::HungerChanged { by }
                } else {
                    Event::HealthChanged { by }
                })
                .collect();
            let clamped = clamp_meters(&events, stats_at(health, hunger));
            let mut stats = stats_at(health, hunger);
            for event in &clamped {
                match *event {
                    Event::HungerChanged { by } => prop_assert!(stats.shift_hunger(by).is_ok()),
                    Event::HealthChanged { by } => prop_assert!(stats.shift_health(by).is_ok()),
                    _ => {}
                }
            }
            // Events that already fit are left exactly as they were.
            let mut fits = stats_at(health, hunger);
            let all_fit = events.iter().all(|event| match *event {
                Event::HungerChanged { by } => fits.shift_hunger(by).is_ok(),
                Event::HealthChanged { by } => fits.shift_health(by).is_ok(),
                _ => true,
            });
            if all_fit {
                let kept: Vec<Event> = events
                    .into_iter()
                    .filter(|event| !matches!(
                        event,
                        Event::HungerChanged { by: 0 } | Event::HealthChanged { by: 0 }
                    ))
                    .collect();
                prop_assert_eq!(clamped, kept);
            }
        }
    }

    #[test]
    fn events_that_change_no_blocks_touch_no_chunk() {
        let events = [
            Event::HungerChanged { by: -1 },
            Event::HealthChanged { by: 2 },
        ];
        assert!(touched_chunks(&events).is_empty());
    }

    #[test]
    fn an_empty_log_has_nothing_to_rewind() {
        assert_eq!(rewind_events(&EventLog::new()), None);
    }

    #[test]
    fn events_outside_any_action_are_never_rewound() {
        let mut log = EventLog::new();
        log.push(Event::BlockSet {
            pos: WorldPos { x: 0, y: 0, z: 0 },
            old: BlockId::AIR,
            new: BlockId::STONE,
        });
        assert_eq!(rewind_events(&log), None);
    }

    #[test]
    fn rewinding_past_the_first_edit_stops() {
        let (mut world, mut log) = (World::default(), EventLog::new());
        edit(&mut world, &mut log, &[(0, 1)]);
        assert!(rewind(&mut world, &mut log));
        assert!(!rewind(&mut world, &mut log));
    }
}
