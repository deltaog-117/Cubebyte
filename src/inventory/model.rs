//! Item stacks and the fixed-size inventory that holds them.

use thiserror::Error;

use crate::items::registry::{self, ItemId};

/// Number of stack slots.
pub const SLOTS: usize = 27;

/// A non-empty pile of one item, never taller than the item's stack limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stack {
    item: ItemId,
    count: u8,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StackError {
    #[error("item {0} is not defined")]
    UnknownItem(u16),
    #[error("a stack of item {item} holds 1..={limit}, not {count}")]
    BadCount { item: u16, count: u8, limit: u8 },
}

impl Stack {
    /// # Errors
    ///
    /// - [`StackError::UnknownItem`] when `item` has no definition.
    /// - [`StackError::BadCount`] when `count` is 0 or above the item's stack limit.
    pub fn try_new(item: ItemId, count: u8) -> Result<Self, StackError> {
        let def = registry::get(item).ok_or(StackError::UnknownItem(item.0))?;
        if count == 0 || count > def.stack_limit {
            return Err(StackError::BadCount {
                item: item.0,
                count,
                limit: def.stack_limit,
            });
        }
        Ok(Self { item, count })
    }

    pub const fn item(self) -> ItemId {
        self.item
    }

    pub const fn count(self) -> u8 {
        self.count
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("item {0} is not defined")]
pub struct UnknownItem(pub u16);

#[derive(Debug, Error, PartialEq, Eq)]
#[error("the inventory holds {held} of the item, not the {wanted} asked for")]
pub struct NotEnough {
    pub held: u32,
    pub wanted: u32,
}

/// A fixed row of slots, each empty or holding one [`Stack`].
///
/// Items fill the lowest slots first, topping up a partial stack before opening an empty
/// slot, so the same sequence of adds always gives the same layout.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    slots: [Option<Stack>; SLOTS],
}

impl Inventory {
    pub fn slots(&self) -> &[Option<Stack>; SLOTS] {
        &self.slots
    }

    /// Rebuilds an inventory from saved slots.
    pub fn from_slots(slots: [Option<Stack>; SLOTS]) -> Self {
        Self { slots }
    }

    pub fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    /// How many of `item` are held across all slots.
    pub fn count_of(&self, item: ItemId) -> u32 {
        self.slots
            .iter()
            .flatten()
            .filter(|stack| stack.item == item)
            .map(|stack| u32::from(stack.count))
            .sum()
    }

    /// How many more of `item` fit, or `None` for an undefined item.
    pub fn room_for(&self, item: ItemId) -> Option<u32> {
        let limit = u32::from(registry::get(item)?.stack_limit);
        let room = self
            .slots
            .iter()
            .map(|slot| match slot {
                None => limit,
                Some(stack) if stack.item == item => limit - u32::from(stack.count),
                Some(_) => 0,
            })
            .sum();
        Some(room)
    }

    /// Adds as many of `count` as fit and returns how many did not.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownItem`] for an undefined item; the inventory is left unchanged.
    pub fn add(&mut self, item: ItemId, count: u32) -> Result<u32, UnknownItem> {
        let limit = registry::get(item).ok_or(UnknownItem(item.0))?.stack_limit;
        let mut left = count;
        // Top up partial stacks first, then open empty slots.
        for stack in self.slots.iter_mut().flatten() {
            if stack.item == item && stack.count < limit {
                let moved = left.min(u32::from(limit - stack.count));
                stack.count += moved as u8;
                left -= moved;
            }
        }
        for slot in self.slots.iter_mut().filter(|slot| slot.is_none()) {
            if left == 0 {
                break;
            }
            let moved = left.min(u32::from(limit));
            *slot = Some(Stack {
                item,
                count: moved as u8,
            });
            left -= moved;
        }
        Ok(left)
    }

    /// Removes exactly `count` of `item`, emptying the highest slots first.
    ///
    /// # Errors
    ///
    /// Returns [`NotEnough`] when fewer than `count` are held; nothing is removed.
    pub fn take(&mut self, item: ItemId, count: u32) -> Result<(), NotEnough> {
        let held = self.count_of(item);
        if held < count {
            return Err(NotEnough {
                held,
                wanted: count,
            });
        }
        let mut left = count;
        for slot in self.slots.iter_mut().rev() {
            let Some(stack) = slot else { continue };
            if stack.item != item || left == 0 {
                continue;
            }
            let moved = left.min(u32::from(stack.count));
            left -= moved;
            // `moved` is at most `stack.count`, which is a u8.
            let remaining = stack.count - moved as u8;
            *slot = (remaining > 0).then_some(Stack {
                item,
                count: remaining,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn item() -> impl Strategy<Value = ItemId> {
        prop_oneof![Just(ItemId::STONE), Just(ItemId::DIRT), Just(ItemId::GRASS)]
    }

    #[test]
    fn a_stack_must_be_between_one_and_the_limit() {
        assert!(Stack::try_new(ItemId::STONE, 1).is_ok());
        assert!(Stack::try_new(ItemId::STONE, 64).is_ok());
        assert!(matches!(
            Stack::try_new(ItemId::STONE, 0),
            Err(StackError::BadCount { .. })
        ));
        assert!(matches!(
            Stack::try_new(ItemId::STONE, 65),
            Err(StackError::BadCount { .. })
        ));
        assert_eq!(
            Stack::try_new(ItemId(0), 1),
            Err(StackError::UnknownItem(0))
        );
    }

    #[test]
    fn adding_tops_up_a_partial_stack_before_opening_a_new_one() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.add(ItemId::STONE, 60), Ok(0));
        assert_eq!(inventory.add(ItemId::DIRT, 1), Ok(0));
        assert_eq!(inventory.add(ItemId::STONE, 10), Ok(0));

        let slots = inventory.slots();
        assert_eq!(slots[0], Stack::try_new(ItemId::STONE, 64).ok());
        assert_eq!(slots[1], Stack::try_new(ItemId::DIRT, 1).ok());
        assert_eq!(slots[2], Stack::try_new(ItemId::STONE, 6).ok());
    }

    #[test]
    fn a_full_inventory_returns_what_did_not_fit() {
        let mut inventory = Inventory::default();
        let capacity = SLOTS as u32 * 64;
        assert_eq!(inventory.add(ItemId::STONE, capacity + 5), Ok(5));
        assert_eq!(inventory.room_for(ItemId::STONE), Some(0));
        assert_eq!(inventory.room_for(ItemId::DIRT), Some(0));
        assert_eq!(inventory.add(ItemId::DIRT, 3), Ok(3));
    }

    #[test]
    fn an_undefined_item_is_refused_and_changes_nothing() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.add(ItemId(0), 1), Err(UnknownItem(0)));
        assert_eq!(inventory.room_for(ItemId(99)), None);
        assert!(inventory.is_empty());
    }

    #[test]
    fn taking_more_than_held_removes_nothing() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.add(ItemId::DIRT, 5), Ok(0));
        assert_eq!(
            inventory.take(ItemId::DIRT, 6),
            Err(NotEnough { held: 5, wanted: 6 })
        );
        assert_eq!(inventory.count_of(ItemId::DIRT), 5);
    }

    #[test]
    fn taking_the_last_of_a_stack_empties_its_slot() {
        let mut inventory = Inventory::default();
        assert_eq!(inventory.add(ItemId::DIRT, 2), Ok(0));
        assert_eq!(inventory.take(ItemId::DIRT, 2), Ok(()));
        assert!(inventory.is_empty());
    }

    proptest! {
        #[test]
        fn add_then_take_conserves_counts(
            ops in prop::collection::vec((item(), 0u32..200, any::<bool>()), 0..60),
        ) {
            let mut inventory = Inventory::default();
            let mut expected = [0u32; 3];
            for (item, count, adding) in ops {
                let slot = usize::from(item.0 - 1);
                if adding {
                    let left = inventory.add(item, count).unwrap();
                    prop_assert!(left <= count);
                    expected[slot] += count - left;
                } else {
                    let held = inventory.count_of(item);
                    let result = inventory.take(item, count);
                    prop_assert_eq!(result.is_ok(), held >= count);
                    if result.is_ok() {
                        expected[slot] -= count;
                    }
                }
                for id in [ItemId::STONE, ItemId::DIRT, ItemId::GRASS] {
                    prop_assert_eq!(inventory.count_of(id), expected[usize::from(id.0 - 1)]);
                }
            }
        }

        #[test]
        fn no_slot_ever_breaks_the_stack_invariant(
            ops in prop::collection::vec((item(), 0u32..200, any::<bool>()), 0..60),
        ) {
            let mut inventory = Inventory::default();
            for (item, count, adding) in ops {
                if adding {
                    inventory.add(item, count).unwrap();
                } else {
                    let _ = inventory.take(item, count);
                }
            }
            for stack in inventory.slots().iter().flatten() {
                prop_assert_eq!(
                    Stack::try_new(stack.item(), stack.count()),
                    Ok(*stack)
                );
            }
        }

        #[test]
        fn room_for_is_exactly_what_add_accepts(
            fill in prop::collection::vec((item(), 0u32..400), 0..12),
            extra in item(),
            count in 0u32..2000,
        ) {
            let mut inventory = Inventory::default();
            for (item, count) in fill {
                inventory.add(item, count).unwrap();
            }
            let room = inventory.room_for(extra).unwrap();
            let left = inventory.add(extra, count).unwrap();
            prop_assert_eq!(count - left, count.min(room));
        }
    }
}
