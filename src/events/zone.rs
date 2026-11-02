//! Rewinding one region of blocks to how it stood some actions ago.
//!
//! [`zone_events`] looks at the newest actions in the log and works out, for every block of the
//! region they changed, the events that put it back as it was before the first of them. Blocks
//! outside the region are never touched. The caller applies the events like any other edit and
//! logs them as one [`ActionKind::Edit`], so `Z` undoes the whole zone rewind exactly, the log
//! stays append-only, and a restart lands on the same world.
//!
//! Only blocks come back: the pack, the meters and the mobs stay as they are. To keep a zone
//! rewind from being a free way to duplicate the yield of a broken block, [`zone_price`] grows
//! with the number of blocks it restores.
//!
//! [`ActionKind::Edit`]: crate::events::log::ActionKind::Edit

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use crate::events::log::EventLog;
use crate::shared::event::Event;
use crate::shared::voxel::{BlockId, WorldPos};

/// Half the side of the cube around the player that a zone rewind covers, in blocks.
pub const ZONE_RADIUS: u8 = 8;

/// The "how far back" choices, in actions that changed blocks.
pub const ZONE_BACK_STEPS: [u16; 5] = [1, 3, 10, 30, 100];

/// Fuel every zone rewind costs, before the per-block part.
pub const ZONE_BASE_FUEL: u32 = 1;

/// How many restored blocks cost one more fuel.
pub const BLOCKS_PER_FUEL: u32 = 4;

/// A box of blocks, both corners included. `min` never exceeds `max` on any axis, because the
/// only way to build one is [`Zone::around`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    min: WorldPos,
    max: WorldPos,
}

impl Zone {
    /// The cube of blocks within `radius` of `center` on every axis. At the edge of the
    /// coordinate range the cube is cut short instead of wrapping.
    pub fn around(center: WorldPos, radius: u8) -> Self {
        let radius = i32::from(radius);
        Self {
            min: WorldPos {
                x: center.x.saturating_sub(radius),
                y: center.y.saturating_sub(radius),
                z: center.z.saturating_sub(radius),
            },
            max: WorldPos {
                x: center.x.saturating_add(radius),
                y: center.y.saturating_add(radius),
                z: center.z.saturating_add(radius),
            },
        }
    }

    pub fn contains(self, pos: WorldPos) -> bool {
        (self.min.x..=self.max.x).contains(&pos.x)
            && (self.min.y..=self.max.y).contains(&pos.y)
            && (self.min.z..=self.max.z).contains(&pos.z)
    }
}

/// The events that put every block of `zone` that the newest `back` block-changing actions
/// changed back to what it was before them, in the order the blocks were last changed, newest
/// first. Empty when nothing in the zone differs.
pub fn zone_events(log: &EventLog, zone: Zone, back: u16) -> Vec<Event> {
    // Per block: what it is now (the newest event's value), then what it was before the span
    // (the oldest event's old value). Walking backwards, the first sight fixes `now` and every
    // later one moves `then` further back.
    let mut spans: HashMap<WorldPos, (BlockId, BlockId)> = HashMap::new();
    let mut order = Vec::new();
    for event in log.recent_block_events(usize::from(back)).iter().rev() {
        let Event::BlockSet { pos, old, new } = *event else {
            continue;
        };
        if !zone.contains(pos) {
            continue;
        }
        match spans.entry(pos) {
            Entry::Occupied(mut span) => span.get_mut().1 = old,
            Entry::Vacant(span) => {
                order.push(pos);
                span.insert((new, old));
            }
        }
    }
    order
        .into_iter()
        .filter_map(|pos| {
            let (now, then) = spans[&pos];
            (now != then).then_some(Event::BlockSet {
                pos,
                old: now,
                new: then,
            })
        })
        .collect()
}

/// The fuel a zone rewind that applies `events` costs: [`ZONE_BASE_FUEL`] plus
/// [`BLOCKS_PER_FUEL`]-th of the blocks it restores, rounded up.
pub fn zone_price(events: &[Event]) -> u32 {
    let blocks = u32::try_from(events.len()).unwrap_or(u32::MAX);
    ZONE_BASE_FUEL.saturating_add(blocks.div_ceil(BLOCKS_PER_FUEL))
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::events::log::ActionKind;
    use crate::events::rewind::{log_action, rewind_events};
    use crate::world::model::World;

    /// One action: a few blocks of a 6 by 1 by 1 strip set to new values.
    fn action() -> impl Strategy<Value = Vec<(i32, u16)>> {
        prop::collection::vec((0i32..6, 0u16..4), 1..4)
    }

    fn at(x: i32) -> WorldPos {
        WorldPos { x, y: 0, z: 0 }
    }

    fn edit(world: &mut World, log: &mut EventLog, action: &[(i32, u16)]) {
        let events: Vec<Event> = action
            .iter()
            .map(|&(x, new)| {
                let event = Event::BlockSet {
                    pos: at(x),
                    old: world.block_at(at(x)),
                    new: BlockId(new),
                };
                world.apply(&event).unwrap();
                event
            })
            .collect();
        log_action(log, &events, ActionKind::Edit);
    }

    proptest! {
        #[test]
        fn the_zone_goes_back_and_everything_else_stays(
            actions in prop::collection::vec(action(), 1..8),
            back in 0u16..10,
            low in 0i32..6,
            width in 0u8..4,
        ) {
            let (mut world, mut log) = (World::default(), EventLog::new());
            let mut after_each = vec![World::default()];
            for step in &actions {
                edit(&mut world, &mut log, step);
                after_each.push(world.clone());
            }
            let before = world.clone();
            let zone = Zone::around(at(low + i32::from(width)), width);

            let events = zone_events(&log, zone, back);
            for event in &events {
                world.apply(event).unwrap();
            }

            let reached = actions.len() - usize::from(back).min(actions.len());
            for x in 0..6 {
                let expected = if zone.contains(at(x)) {
                    after_each[reached].block_at(at(x))
                } else {
                    before.block_at(at(x))
                };
                prop_assert_eq!(world.block_at(at(x)), expected);
            }

            // Logged as one edit, `Z` takes the whole zone rewind back exactly.
            if !events.is_empty() {
                log_action(&mut log, &events, ActionKind::Edit);
                let undo = rewind_events(&log).expect("the zone rewind is an edit in effect");
                for event in &undo {
                    world.apply(event).unwrap();
                }
                prop_assert_eq!(&world, &before);
            }
        }
    }

    #[test]
    fn a_zone_around_a_point_holds_its_corners_and_not_the_next_block() {
        let zone = Zone::around(WorldPos { x: 0, y: 0, z: 0 }, 2);
        assert!(zone.contains(WorldPos { x: 2, y: -2, z: 2 }));
        assert!(!zone.contains(WorldPos { x: 3, y: 0, z: 0 }));
    }

    #[test]
    fn a_zone_at_the_edge_of_the_range_is_cut_short() {
        let zone = Zone::around(
            WorldPos {
                x: i32::MAX,
                y: i32::MIN,
                z: 0,
            },
            8,
        );
        assert!(zone.contains(WorldPos {
            x: i32::MAX,
            y: i32::MIN,
            z: 0
        }));
    }

    #[test]
    fn the_price_counts_the_base_and_every_started_group_of_blocks() {
        let one = Event::BlockSet {
            pos: at(0),
            old: BlockId(0),
            new: BlockId(1),
        };
        assert_eq!(zone_price(&[]), ZONE_BASE_FUEL);
        assert_eq!(zone_price(&[one]), ZONE_BASE_FUEL + 1);
        assert_eq!(zone_price(&[one; 4]), ZONE_BASE_FUEL + 1);
        assert_eq!(zone_price(&[one; 5]), ZONE_BASE_FUEL + 2);
    }
}
