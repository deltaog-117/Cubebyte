//! Applying any event to the game state it changes.
//!
//! [`apply`] is the one door every change goes through: blocks to the world, jumps to the
//! player, slot changes to the inventory. Each event records what it replaced, so an event
//! applied to a state that has moved on is refused instead of guessed at, and a refused event
//! leaves the state exactly as it was.

use thiserror::Error;

use crate::inventory::model::{Inventory, Stack};
use crate::items::registry::ItemId;
use crate::player::model::{Player, PlayerError};
use crate::player::service::teleport;
use crate::shared::event::{Event, Held, Spot};
use crate::world::model::{ApplyError, World};

/// The parts of the game state events change, borrowed together for one application.
pub struct Stage<'a> {
    pub world: &'a mut World,
    pub player: &'a mut Player,
    pub inventory: &'a mut Inventory,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StageError {
    #[error(transparent)]
    Block(#[from] ApplyError),
    /// The player is not where the move was recorded from.
    #[error("stale move: expected the player at {expected:?} but found {found:?}")]
    StaleMove { expected: Spot, found: Spot },
    #[error(transparent)]
    Player(#[from] PlayerError),
    #[error("slot {0} does not exist")]
    BadSlot(u8),
    /// The slot does not hold what the event was recorded against.
    #[error("stale slot {slot}: expected {expected:?} but found {found:?}")]
    StaleSlot {
        slot: u8,
        expected: Held,
        found: Held,
    },
    /// An item that is not defined, or a count that is zero or above the item's stack limit.
    #[error("{0:?} is not a stack the game can hold")]
    BadHeld(Held),
}

/// Applies `event` to `stage`.
///
/// # Errors
///
/// - [`StageError::Block`] when a block is not the one the event was recorded against.
/// - [`StageError::StaleMove`] when the player is not where the move started.
/// - [`StageError::Player`] when a move ends at a position that is not finite.
/// - [`StageError::BadSlot`] when the slot is past the last one.
/// - [`StageError::StaleSlot`] when the slot holds something other than the event's `old`.
/// - [`StageError::BadHeld`] when the event's `new` is not a stack the game can hold.
///
/// The stage is left unchanged on any error.
pub fn apply(stage: &mut Stage<'_>, event: &Event) -> Result<(), StageError> {
    match *event {
        Event::BlockSet { .. } => Ok(stage.world.apply(event)?),
        Event::PlayerMoved { old, new } => {
            let found = Spot::from_vec3(stage.player.position());
            if found != old {
                tracing::warn!(?old, ?found, "stale move rejected");
                return Err(StageError::StaleMove {
                    expected: old,
                    found,
                });
            }
            Ok(teleport(stage.player, new.to_vec3())?)
        }
        Event::SlotSet { slot, old, new } => {
            let mut slots = *stage.inventory.slots();
            let cell = slots
                .get_mut(usize::from(slot))
                .ok_or(StageError::BadSlot(slot))?;
            let found = held_of(*cell);
            if found != old {
                tracing::warn!(slot, ?old, ?found, "stale slot rejected");
                return Err(StageError::StaleSlot {
                    slot,
                    expected: old,
                    found,
                });
            }
            *cell = stack_of(new)?;
            *stage.inventory = Inventory::from_slots(slots);
            Ok(())
        }
    }
}

/// Applies `events` in order, all or none: if one is refused, the ones before it are undone
/// and the stage is left as it was.
///
/// # Errors
///
/// Returns the first [`StageError`] from [`apply`].
pub fn apply_all(stage: &mut Stage<'_>, events: &[Event]) -> Result<(), StageError> {
    for (done, event) in events.iter().enumerate() {
        if let Err(error) = apply(stage, event) {
            for applied in events[..done].iter().rev() {
                // Each of these was just applied, so its inverse matches the state it left.
                if let Err(rollback) = apply(stage, &applied.inverse()) {
                    tracing::error!(%rollback, "rollback failed; the state is inconsistent");
                }
            }
            return Err(error);
        }
    }
    Ok(())
}

/// The slot events that turn `before` into `after`, lowest slot first.
pub fn slot_events(before: &Inventory, after: &Inventory) -> Vec<Event> {
    before
        .slots()
        .iter()
        .zip(after.slots())
        .enumerate()
        .filter(|(_, (was, now))| was != now)
        // `SLOTS` is far below 256, so a slot number always fits a byte.
        .map(|(slot, (&was, &now))| Event::SlotSet {
            slot: slot as u8,
            old: held_of(was),
            new: held_of(now),
        })
        .collect()
}

fn held_of(slot: Option<Stack>) -> Held {
    slot.map_or(Held::EMPTY, |stack| Held {
        item: stack.item().0,
        count: stack.count(),
    })
}

fn stack_of(held: Held) -> Result<Option<Stack>, StageError> {
    if held == Held::EMPTY {
        return Ok(None);
    }
    Stack::try_new(ItemId(held.item), held.count)
        .map(Some)
        .map_err(|_| StageError::BadHeld(held))
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use proptest::prelude::*;

    use super::*;
    use crate::inventory::model::SLOTS;
    use crate::shared::voxel::{BlockId, WorldPos};

    const ITEMS: [ItemId; 3] = [ItemId::STONE, ItemId::DIRT, ItemId::GRASS];

    struct State {
        world: World,
        player: Player,
        inventory: Inventory,
    }

    impl State {
        fn new() -> Self {
            Self {
                world: World::default(),
                player: Player::try_new(Vec3::new(0.5, 3.0, 0.5)).unwrap(),
                inventory: Inventory::default(),
            }
        }

        fn stage(&mut self) -> Stage<'_> {
            Stage {
                world: &mut self.world,
                player: &mut self.player,
                inventory: &mut self.inventory,
            }
        }

        fn apply(&mut self, event: &Event) -> Result<(), StageError> {
            apply(&mut self.stage(), event)
        }
    }

    #[derive(Clone, Debug)]
    enum Op {
        Block((i32, i32, i32), u16),
        Move((f32, f32, f32)),
        Add(usize, u32),
        Remove(usize, u32),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            ((-20i32..20, -4i32..4, -20i32..20), 0u16..4).prop_map(|(at, id)| Op::Block(at, id)),
            (-500.0f32..500.0, -50.0f32..50.0, -500.0f32..500.0).prop_map(Op::Move),
            (0usize..3, 1u32..200).prop_map(|(item, count)| Op::Add(item, count)),
            (0usize..3, 1u32..200).prop_map(|(item, count)| Op::Remove(item, count)),
        ]
    }

    /// The events `op` would be if made now; none when it would change nothing.
    fn events_for(state: &State, op: &Op) -> Vec<Event> {
        match *op {
            Op::Block((x, y, z), id) => {
                let pos = WorldPos { x, y, z };
                vec![Event::BlockSet {
                    pos,
                    old: state.world.block_at(pos),
                    new: BlockId(id),
                }]
            }
            Op::Move((x, y, z)) => vec![Event::PlayerMoved {
                old: Spot::from_vec3(state.player.position()),
                new: Spot::from_vec3(Vec3::new(x, y, z)),
            }],
            Op::Add(which, count) => {
                let item = ITEMS[which];
                let mut after = state.inventory.clone();
                let fits = after.room_for(item).unwrap_or(0).min(count);
                after.add(item, fits).unwrap();
                slot_events(&state.inventory, &after)
            }
            Op::Remove(which, count) => {
                let item = ITEMS[which];
                let mut after = state.inventory.clone();
                after
                    .take(item, state.inventory.count_of(item).min(count))
                    .unwrap();
                slot_events(&state.inventory, &after)
            }
        }
    }

    proptest! {
        #[test]
        fn undoing_a_run_of_events_in_reverse_restores_the_state_exactly(
            ops in prop::collection::vec(op(), 0..80),
        ) {
            let mut state = State::new();
            let start_world = state.world.clone();
            let start_player = state.player;

            let mut applied = Vec::new();
            for op in &ops {
                for event in events_for(&state, op) {
                    prop_assert_eq!(state.apply(&event), Ok(()));
                    applied.push(event);
                }
            }
            for event in applied.iter().rev() {
                prop_assert_eq!(state.apply(&event.inverse()), Ok(()));
            }

            prop_assert_eq!(&state.world, &start_world);
            prop_assert_eq!(state.player, start_player);
            prop_assert_eq!(state.inventory, Inventory::default());
        }

        #[test]
        fn replaying_a_run_from_the_start_reaches_the_same_state(
            ops in prop::collection::vec(op(), 0..80),
        ) {
            let mut live = State::new();
            let mut applied = Vec::new();
            for op in &ops {
                for event in events_for(&live, op) {
                    prop_assert_eq!(live.apply(&event), Ok(()));
                    applied.push(event);
                }
            }

            let mut replayed = State::new();
            for event in &applied {
                prop_assert_eq!(replayed.apply(event), Ok(()));
            }

            prop_assert_eq!(replayed.world, live.world);
            prop_assert_eq!(replayed.player, live.player);
            prop_assert_eq!(replayed.inventory, live.inventory);
        }

        #[test]
        fn the_slot_events_between_two_inventories_turn_one_into_the_other(
            first in prop::collection::vec((0usize..3, 1u32..100), 0..20),
            second in prop::collection::vec((0usize..3, 1u32..100), 0..20),
        ) {
            let build = |adds: &[(usize, u32)]| {
                let mut inventory = Inventory::default();
                for &(which, count) in adds {
                    inventory.add(ITEMS[which], count).unwrap();
                }
                inventory
            };
            let (before, after) = (build(&first), build(&second));

            let mut state = State::new();
            state.inventory = before.clone();
            prop_assert_eq!(apply_all(&mut state.stage(), &slot_events(&before, &after)), Ok(()));

            prop_assert_eq!(state.inventory, after);
        }
    }

    fn held(item: ItemId, count: u8) -> Held {
        Held {
            item: item.0,
            count,
        }
    }

    #[test]
    fn a_stale_move_is_refused_and_changes_nothing() {
        let mut state = State::new();
        let before = state.player;
        let elsewhere = Spot::from_vec3(Vec3::new(9.0, 9.0, 9.0));

        let result = state.apply(&Event::PlayerMoved {
            old: elsewhere,
            new: elsewhere,
        });

        assert!(matches!(result, Err(StageError::StaleMove { .. })));
        assert_eq!(state.player, before);
    }

    #[test]
    fn a_move_to_a_non_finite_spot_is_refused_and_changes_nothing() {
        let mut state = State::new();
        let before = state.player;

        let result = state.apply(&Event::PlayerMoved {
            old: Spot::from_vec3(before.position()),
            new: Spot::from_bits([f32::NAN.to_bits(), 0, 0]),
        });

        assert_eq!(result, Err(StageError::Player(PlayerError::NonFinite)));
        assert_eq!(state.player, before);
    }

    #[test]
    fn a_slot_that_holds_something_else_is_refused_and_changes_nothing() {
        let mut state = State::new();
        state.inventory.add(ItemId::DIRT, 5).unwrap();
        let before = state.inventory.clone();

        let result = state.apply(&Event::SlotSet {
            slot: 0,
            old: held(ItemId::STONE, 5),
            new: Held::EMPTY,
        });

        assert_eq!(
            result,
            Err(StageError::StaleSlot {
                slot: 0,
                expected: held(ItemId::STONE, 5),
                found: held(ItemId::DIRT, 5),
            })
        );
        assert_eq!(state.inventory, before);
    }

    #[test]
    fn a_slot_past_the_last_one_is_refused() {
        let mut state = State::new();
        let slot = u8::try_from(SLOTS).unwrap();

        let result = state.apply(&Event::SlotSet {
            slot,
            old: Held::EMPTY,
            new: held(ItemId::STONE, 1),
        });

        assert_eq!(result, Err(StageError::BadSlot(slot)));
        assert!(state.inventory.is_empty());
    }

    #[test]
    fn a_stack_the_game_cannot_hold_is_refused_and_changes_nothing() {
        let mut state = State::new();
        for bad in [
            Held {
                item: 999,
                count: 1,
            },
            Held { item: 0, count: 3 },
            held(ItemId::STONE, 0),
            held(ItemId::STONE, 65),
        ] {
            let result = state.apply(&Event::SlotSet {
                slot: 0,
                old: Held::EMPTY,
                new: bad,
            });
            assert_eq!(result, Err(StageError::BadHeld(bad)));
        }
        assert!(state.inventory.is_empty());
    }

    #[test]
    fn apply_all_undoes_the_earlier_events_when_a_later_one_is_refused() {
        let mut state = State::new();
        let good = Event::SlotSet {
            slot: 0,
            old: Held::EMPTY,
            new: held(ItemId::STONE, 8),
        };
        let stale = Event::SlotSet {
            slot: 1,
            old: held(ItemId::DIRT, 1),
            new: Held::EMPTY,
        };

        let result = apply_all(&mut state.stage(), &[good, stale]);

        assert!(matches!(result, Err(StageError::StaleSlot { slot: 1, .. })));
        assert!(state.inventory.is_empty());
    }
}
