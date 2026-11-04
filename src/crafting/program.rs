//! Recipes a player writes: a short program of `take` and `make` steps.
//!
//! A built-in recipe is a fixed row of inputs and an output. A player recipe is a
//! [`RecipeProgram`]: the same two verbs, plus `repeat` and `if`, so a recipe can give more
//! when the pack holds more, or choose by what the player stands next to. Running it works on
//! a copy of the pack and replaces nothing, so a recipe either does all of its steps or none.
//!
//! Two rules keep a player from minting items out of nothing, and both hold for every path
//! through the program, not only the one a run takes:
//! - **Worth.** However the `if`s go, the worth made never exceeds the worth taken (items have
//!   a `worth` in the item table). Worth is checked when the program is built, so an unfair
//!   recipe cannot exist.
//! - **Size.** At most [`MAX_STEPS`] steps, [`MAX_DEPTH`] levels deep, each repeat 1 to
//!   [`MAX_REPEAT`] times, and a run stops after [`RUN_BUDGET`] visits.

use thiserror::Error;

use crate::inventory::model::{Inventory, NotEnough};
use crate::items::registry::{self as items, ItemId};
use crate::scripting::model::{Answer, Denied, Query};
use crate::scripting::program::Test;
use crate::shared::voxel::WorldPos;

/// Most steps one recipe may hold, nested ones included.
pub const MAX_STEPS: usize = 32;

/// Deepest nesting of repeats and ifs, counting the top level as level 1.
pub const MAX_DEPTH: usize = 3;

/// Most times one repeat may run its body.
pub const MAX_REPEAT: u8 = 16;

/// Most steps one run may visit before it is stopped.
pub const RUN_BUDGET: u32 = 1024;

/// One step of a recipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Spend `count` of `item` from the pack.
    Take { item: ItemId, count: u8 },
    /// Add `count` of `item` to the pack.
    Make { item: ItemId, count: u8 },
    /// Run `body` `times` times in a row.
    Repeat { times: u8, body: Vec<Step> },
    /// Run `then` if the test holds, otherwise `otherwise` (which may be empty). A `count`
    /// read counts what the pack holds at that point of the run; the other reads see the world
    /// as it is.
    If {
        test: Test,
        then: Vec<Step>,
        otherwise: Vec<Step>,
    },
}

/// Why a list of steps is not a recipe. Each message says what to change.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecipeError {
    #[error("a recipe needs at least one step")]
    Empty,
    #[error("a repeat or the then part of an if holds no steps")]
    EmptyBody,
    #[error("steps may sit at most {MAX_DEPTH} levels deep")]
    TooDeep,
    #[error("a recipe may hold at most {MAX_STEPS} steps")]
    TooBig,
    #[error("a repeat must run from 1 to {MAX_REPEAT} times")]
    BadRepeat,
    #[error("a take or make must move at least 1 item")]
    ZeroCount,
    #[error("a step names an item that does not exist")]
    UnknownItem,
    #[error("a recipe must make something")]
    MakesNothing,
    #[error("this recipe can make {excess} more worth than it takes; give back less or take more")]
    MakesWorth { excess: u64 },
}

/// Why a run of a recipe did nothing.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RunError {
    #[error("missing ingredients")]
    Missing(#[from] NotEnough),
    #[error("the inventory has no room for the result")]
    Full,
    #[error("the recipe ran too long")]
    TooLong,
    #[error("a reading the recipe needs was refused")]
    Read(#[from] Denied),
}

/// A checked list of steps: the only way to get one is [`RecipeProgram::try_new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeProgram {
    steps: Vec<Step>,
}

impl RecipeProgram {
    /// # Errors
    ///
    /// Returns a [`RecipeError`] when `steps` is empty, nests deeper than [`MAX_DEPTH`], holds
    /// more than [`MAX_STEPS`] steps, repeats outside `1..=`[`MAX_REPEAT`], moves zero items,
    /// names an unknown item, makes nothing, has an empty body, or can make more worth than it
    /// takes on any path.
    pub fn try_new(steps: Vec<Step>) -> Result<Self, RecipeError> {
        if steps.is_empty() {
            return Err(RecipeError::Empty);
        }
        check(&steps, 1, &mut 0)?;
        if !makes_something(&steps) {
            return Err(RecipeError::MakesNothing);
        }
        let gain = best_gain(&steps);
        if gain > 0 {
            return Err(RecipeError::MakesWorth {
                excess: gain.unsigned_abs(),
            });
        }
        Ok(Self { steps })
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The pack after one run, or why the recipe cannot run. Nothing is changed: the caller
    /// decides what to do with the result. `cell` is where `look` reads count from, and `ask`
    /// answers the reads the pack cannot.
    ///
    /// # Errors
    ///
    /// - [`RunError::Missing`] when a take finds too little.
    /// - [`RunError::Full`] when a make has no room.
    /// - [`RunError::TooLong`] after [`RUN_BUDGET`] visits.
    /// - [`RunError::Read`] when an `if` reads something that is refused or not a number.
    pub fn run(
        &self,
        pack: &Inventory,
        cell: WorldPos,
        ask: &mut dyn FnMut(Query) -> Result<Answer, Denied>,
    ) -> Result<Inventory, RunError> {
        let mut after = pack.clone();
        let mut spent = 0;
        walk(&self.steps, &mut after, cell, ask, &mut spent)?;
        Ok(after)
    }
}

fn check(steps: &[Step], depth: usize, count: &mut usize) -> Result<(), RecipeError> {
    if depth > MAX_DEPTH {
        return Err(RecipeError::TooDeep);
    }
    for step in steps {
        *count += 1;
        if *count > MAX_STEPS {
            return Err(RecipeError::TooBig);
        }
        match step {
            Step::Take { item, count } | Step::Make { item, count } => {
                if *count == 0 {
                    return Err(RecipeError::ZeroCount);
                }
                if items::get(*item).is_none() {
                    return Err(RecipeError::UnknownItem);
                }
            }
            Step::Repeat { times, body } => {
                if !(1..=MAX_REPEAT).contains(times) {
                    return Err(RecipeError::BadRepeat);
                }
                if body.is_empty() {
                    return Err(RecipeError::EmptyBody);
                }
                check(body, depth + 1, count)?;
            }
            Step::If {
                test,
                then,
                otherwise,
            } => {
                if let crate::scripting::program::Read::Count { item } = test.read
                    && items::get(item).is_none()
                {
                    return Err(RecipeError::UnknownItem);
                }
                if then.is_empty() {
                    return Err(RecipeError::EmptyBody);
                }
                check(then, depth + 1, count)?;
                check(otherwise, depth + 1, count)?;
            }
        }
    }
    Ok(())
}

fn makes_something(steps: &[Step]) -> bool {
    steps.iter().any(|step| match step {
        Step::Make { .. } => true,
        Step::Take { .. } => false,
        Step::Repeat { body, .. } => makes_something(body),
        Step::If {
            then, otherwise, ..
        } => makes_something(then) || makes_something(otherwise),
    })
}

fn worth_of(item: ItemId, count: u8) -> i64 {
    items::get(item).map_or(0, |def| i64::from(def.worth) * i64::from(count))
}

/// The most worth, made minus taken, that any path through `steps` can end with. A path may
/// take a different branch on every pass of a repeat, so a repeat counts its body's best path
/// each time. Negative means every path takes more than it makes.
fn best_gain(steps: &[Step]) -> i64 {
    steps
        .iter()
        .map(|step| match step {
            Step::Take { item, count } => -worth_of(*item, *count),
            Step::Make { item, count } => worth_of(*item, *count),
            Step::Repeat { times, body } => i64::from(*times) * best_gain(body),
            Step::If {
                then, otherwise, ..
            } => best_gain(then).max(best_gain(otherwise)),
        })
        .sum()
}

fn walk(
    steps: &[Step],
    pack: &mut Inventory,
    cell: WorldPos,
    ask: &mut dyn FnMut(Query) -> Result<Answer, Denied>,
    spent: &mut u32,
) -> Result<(), RunError> {
    for step in steps {
        *spent += 1;
        if *spent > RUN_BUDGET {
            return Err(RunError::TooLong);
        }
        match step {
            Step::Take { item, count } => pack.take(*item, u32::from(*count))?,
            Step::Make { item, count } => {
                // An unknown item cannot get here (the program was checked), so the error arm
                // only guards the type.
                let left = pack
                    .add(*item, u32::from(*count))
                    .map_err(|_| RunError::Full)?;
                if left > 0 {
                    return Err(RunError::Full);
                }
            }
            Step::Repeat { times, body } => {
                for _ in 0..*times {
                    walk(body, pack, cell, ask, spent)?;
                }
            }
            Step::If {
                test,
                then,
                otherwise,
            } => {
                let holds = {
                    let held = &*pack;
                    test.check(cell, &mut |query| match query {
                        Query::Count { item } => Ok(Answer::Number(held.count_of(item))),
                        other => ask(other),
                    })?
                };
                walk(if holds { then } else { otherwise }, pack, cell, ask, spent)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::scripting::program::{Compare, Read};

    fn take(item: ItemId, count: u8) -> Step {
        Step::Take { item, count }
    }

    fn make(item: ItemId, count: u8) -> Step {
        Step::Make { item, count }
    }

    fn origin() -> WorldPos {
        WorldPos { x: 0, y: 0, z: 0 }
    }

    fn no_world(_: Query) -> Result<Answer, Denied> {
        Err(Denied::NothingToDo)
    }

    fn pack(item: ItemId, count: u32) -> Inventory {
        let mut inventory = Inventory::default();
        inventory.add(item, count).unwrap();
        inventory
    }

    fn count_at_least(item: ItemId, than: u32) -> Test {
        Test {
            read: Read::Count { item },
            compare: Compare::Above,
            than,
        }
    }

    #[test]
    fn a_fair_recipe_runs() {
        let recipe =
            RecipeProgram::try_new(vec![take(ItemId::GRASS, 3), make(ItemId::BREAD, 1)]).unwrap();
        let after = recipe
            .run(&pack(ItemId::GRASS, 5), origin(), &mut no_world)
            .unwrap();
        assert_eq!(after.count_of(ItemId::GRASS), 2);
        assert_eq!(after.count_of(ItemId::BREAD), 1);
    }

    #[test]
    fn a_recipe_that_makes_worth_from_nothing_cannot_be_built() {
        assert_eq!(
            RecipeProgram::try_new(vec![make(ItemId::SWORD, 1)]),
            Err(RecipeError::MakesWorth { excess: 16 })
        );
        // One grass is worth 2; a bread is worth 6.
        assert_eq!(
            RecipeProgram::try_new(vec![take(ItemId::GRASS, 1), make(ItemId::BREAD, 1)]),
            Err(RecipeError::MakesWorth { excess: 4 })
        );
    }

    #[test]
    fn the_worth_rule_holds_on_every_branch() {
        // The else path makes bread from one grass, so the whole recipe is refused even
        // though the then path is fair.
        let recipe = RecipeProgram::try_new(vec![Step::If {
            test: count_at_least(ItemId::DIRT, 5),
            then: vec![take(ItemId::GRASS, 3), make(ItemId::BREAD, 1)],
            otherwise: vec![take(ItemId::GRASS, 1), make(ItemId::BREAD, 1)],
        }]);
        assert!(matches!(recipe, Err(RecipeError::MakesWorth { .. })));
    }

    #[test]
    fn a_repeat_cannot_hide_a_gain_that_adds_up() {
        let recipe = RecipeProgram::try_new(vec![Step::Repeat {
            times: 16,
            body: vec![
                take(ItemId::GRASS, 3),
                make(ItemId::BREAD, 1),
                make(ItemId::DIRT, 1),
            ],
        }]);
        assert_eq!(recipe, Err(RecipeError::MakesWorth { excess: 32 }));
    }

    #[test]
    fn an_if_reads_the_pack_as_the_run_has_changed_it() {
        // Spends two dirt, then makes stone only if fewer than three dirt remain.
        let recipe = RecipeProgram::try_new(vec![
            take(ItemId::DIRT, 2),
            Step::If {
                test: Test {
                    read: Read::Count { item: ItemId::DIRT },
                    compare: Compare::Below,
                    than: 3,
                },
                then: vec![make(ItemId::STONE, 1)],
                otherwise: vec![],
            },
        ])
        .unwrap();
        let with_four = recipe
            .run(&pack(ItemId::DIRT, 4), origin(), &mut no_world)
            .unwrap();
        assert_eq!(with_four.count_of(ItemId::STONE), 1);
        let with_six = recipe
            .run(&pack(ItemId::DIRT, 6), origin(), &mut no_world)
            .unwrap();
        assert_eq!(with_six.count_of(ItemId::STONE), 0);
    }

    #[test]
    fn a_world_read_goes_to_the_asker_with_the_cell() {
        let recipe = RecipeProgram::try_new(vec![
            take(ItemId::GRASS, 3),
            Step::If {
                test: Test {
                    read: Read::Look { by: [0, -1, 0] },
                    compare: Compare::Is,
                    than: 3,
                },
                then: vec![make(ItemId::BREAD, 1)],
                otherwise: vec![make(ItemId::GRASS, 1)],
            },
        ])
        .unwrap();
        let mut seen = None;
        let mut ask = |query| {
            seen = Some(query);
            Ok(Answer::Block(crate::shared::voxel::BlockId::GRASS))
        };
        let cell = WorldPos { x: 5, y: 6, z: 7 };
        let after = recipe.run(&pack(ItemId::GRASS, 3), cell, &mut ask).unwrap();
        assert_eq!(after.count_of(ItemId::BREAD), 1);
        assert_eq!(
            seen,
            Some(Query::Look {
                pos: WorldPos { x: 5, y: 5, z: 7 }
            })
        );
    }

    #[test]
    fn a_failed_run_changes_nothing() {
        let recipe =
            RecipeProgram::try_new(vec![take(ItemId::GRASS, 3), make(ItemId::BREAD, 1)]).unwrap();
        let short = pack(ItemId::GRASS, 2);
        let before = short.clone();
        assert!(matches!(
            recipe.run(&short, origin(), &mut no_world),
            Err(RunError::Missing(_))
        ));
        assert_eq!(short, before);
    }

    #[test]
    fn a_result_with_no_room_is_refused() {
        let recipe =
            RecipeProgram::try_new(vec![take(ItemId::GRASS, 1), make(ItemId::DIRT, 1)]).unwrap();
        let mut full = Inventory::default();
        full.add(
            ItemId::STONE,
            (crate::inventory::model::SLOTS as u32 - 1) * 64,
        )
        .unwrap();
        full.add(ItemId::GRASS, 2).unwrap();
        assert_eq!(
            recipe.run(&full, origin(), &mut no_world),
            Err(RunError::Full)
        );
    }

    #[test]
    fn the_shape_rules_are_enforced() {
        use RecipeError as E;
        let ok = || take(ItemId::GRASS, 3);
        assert_eq!(RecipeProgram::try_new(vec![]), Err(E::Empty));
        assert_eq!(
            RecipeProgram::try_new(vec![ok(), take(ItemId::GRASS, 0)]),
            Err(E::ZeroCount)
        );
        assert_eq!(
            RecipeProgram::try_new(vec![ok(), make(ItemId(99), 1)]),
            Err(E::UnknownItem)
        );
        assert_eq!(RecipeProgram::try_new(vec![ok()]), Err(E::MakesNothing));
        assert_eq!(
            RecipeProgram::try_new(vec![Step::Repeat {
                times: 0,
                body: vec![ok()]
            }]),
            Err(E::BadRepeat)
        );
        assert_eq!(
            RecipeProgram::try_new(vec![Step::Repeat {
                times: 2,
                body: vec![]
            }]),
            Err(E::EmptyBody)
        );
        let mut nested = vec![ok(), make(ItemId::BREAD, 1)];
        for _ in 0..MAX_DEPTH {
            nested = vec![Step::Repeat {
                times: 1,
                body: nested,
            }];
        }
        assert_eq!(RecipeProgram::try_new(nested), Err(E::TooDeep));
        let wide: Vec<_> = (0..=MAX_STEPS).map(|_| take(ItemId::GRASS, 1)).collect();
        assert_eq!(RecipeProgram::try_new(wide), Err(E::TooBig));
    }

    fn arb_item() -> impl Strategy<Value = ItemId> {
        (1u16..=7).prop_map(ItemId)
    }

    fn arb_steps() -> impl Strategy<Value = Vec<Step>> {
        let leaf = prop_oneof![
            (arb_item(), 1u8..8).prop_map(|(item, count)| Step::Take { item, count }),
            (arb_item(), 1u8..8).prop_map(|(item, count)| Step::Make { item, count }),
        ];
        let step = leaf.prop_recursive(2, 12, 3, |inner| {
            let body = prop::collection::vec(inner, 1..3);
            prop_oneof![
                (1u8..=4, body.clone()).prop_map(|(times, body)| Step::Repeat { times, body }),
                (arb_item(), body.clone(), body).prop_map(|(item, then, otherwise)| Step::If {
                    test: Test {
                        read: Read::Count { item },
                        compare: Compare::Above,
                        than: 3,
                    },
                    then,
                    otherwise,
                }),
            ]
        });
        prop::collection::vec(step, 1..6)
    }

    fn worth_in(pack: &Inventory) -> i64 {
        items::ids()
            .map(|item| worth_of(item, 1) * i64::from(pack.count_of(item)))
            .sum()
    }

    fn rich_pack() -> Inventory {
        let mut inventory = Inventory::default();
        for item in items::ids() {
            let _ = inventory.add(item, 20);
        }
        inventory
    }

    proptest! {
        // A recipe that was accepted can never leave the pack worth more than it found it,
        // whichever way its ifs go. This is the reason the worth rule exists.
        #[test]
        fn an_accepted_recipe_never_adds_worth(steps in arb_steps()) {
            let Ok(recipe) = RecipeProgram::try_new(steps) else { return Ok(()); };
            let before = rich_pack();
            if let Ok(after) = recipe.run(&before, origin(), &mut no_world) {
                prop_assert!(worth_in(&after) <= worth_in(&before));
            }
        }

        // A run that fails leaves the pack it was given exactly as it was.
        #[test]
        fn a_run_never_touches_the_pack_it_was_given(steps in arb_steps()) {
            let Ok(recipe) = RecipeProgram::try_new(steps) else { return Ok(()); };
            let before = rich_pack();
            let copy = before.clone();
            let _ = recipe.run(&before, origin(), &mut no_world);
            prop_assert_eq!(before, copy);
        }
    }
}
