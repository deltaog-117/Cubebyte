//! Rules: "when this test holds at this spot, run this program".
//!
//! A rule is a [`Test`] and a [`Program`] fixed on a cell. Every [`CHECK_EVERY`] ticks the game
//! asks [`Rules::due`] which rule, if any, should fire; the program then runs as any script
//! does, through the permission, range, fuel and wait checks, so it is logged, recorded in the
//! intent track and undone like a script a player started by hand. The rules themselves hold
//! no history and change nothing in the world, which is why they need no event of their own.
//!
//! A rule rests for [`REST_TICKS`] after it fires, so one that stays true cannot drain the
//! pack every second.

use thiserror::Error;

use crate::scripting::model::{Answer, Denied, Query};
use crate::scripting::program::{Program, Test};
use crate::shared::voxel::WorldPos;

/// Most rules at once.
pub const MAX_RULES: usize = 8;

/// Rules are looked at once per this many ticks (one second at 20 ticks a second).
pub const CHECK_EVERY: u64 = 20;

/// Ticks a rule rests after it fires (ten seconds).
pub const REST_TICKS: u64 = 200;

/// Identifies a rule. IDs are not reused while the game runs, so a stale ID finds nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RuleId(pub u8);

/// One rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    test: Test,
    program: Program,
    anchor: WorldPos,
    on: bool,
    /// The first tick at which the rule may fire again.
    rests_until: u64,
}

impl Rule {
    pub fn test(&self) -> Test {
        self.test
    }

    pub fn program(&self) -> &Program {
        &self.program
    }

    /// The cell the program's working cell starts on, and the test's reads count from.
    pub fn anchor(&self) -> WorldPos {
        self.anchor
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Whether the rule is still resting at `tick` after a firing.
    pub fn is_resting(&self, tick: u64) -> bool {
        tick < self.rests_until
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RulesError {
    #[error("at most {MAX_RULES} rules at once")]
    Full,
    #[error("no such rule")]
    Missing,
}

/// The rules in play, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    list: Vec<(RuleId, Rule)>,
    next: u8,
}

impl Rules {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (RuleId, &Rule)> {
        self.list.iter().map(|(id, rule)| (*id, rule))
    }

    pub fn get(&self, id: RuleId) -> Option<&Rule> {
        self.list
            .iter()
            .find(|(rule_id, _)| *rule_id == id)
            .map(|(_, rule)| rule)
    }

    fn get_mut(&mut self, id: RuleId) -> Option<&mut Rule> {
        self.list
            .iter_mut()
            .find(|(rule_id, _)| *rule_id == id)
            .map(|(_, rule)| rule)
    }

    /// Adds a rule that starts switched on and ready.
    ///
    /// # Errors
    ///
    /// Returns [`RulesError::Full`] when [`MAX_RULES`] rules exist.
    pub fn add(
        &mut self,
        test: Test,
        program: Program,
        anchor: WorldPos,
    ) -> Result<RuleId, RulesError> {
        if self.list.len() >= MAX_RULES {
            return Err(RulesError::Full);
        }
        let id = RuleId(self.next);
        // Wrapping is harmless: at most eight rules live at once, so a wrapped ID can only
        // meet a live one after 256 rules were made while that one stayed.
        self.next = self.next.wrapping_add(1);
        self.list.push((
            id,
            Rule {
                test,
                program,
                anchor,
                on: true,
                rests_until: 0,
            },
        ));
        Ok(id)
    }

    /// Changes what rule `id` tests and runs, keeping its spot and whether it is on. It is
    /// ready to fire again at once.
    ///
    /// # Errors
    ///
    /// Returns [`RulesError::Missing`] for an ID no rule has.
    pub fn replace(&mut self, id: RuleId, test: Test, program: Program) -> Result<(), RulesError> {
        let rule = self.get_mut(id).ok_or(RulesError::Missing)?;
        rule.test = test;
        rule.program = program;
        rule.rests_until = 0;
        Ok(())
    }

    /// Switches rule `id` on or off and returns the new state, or `None` for a missing rule.
    pub fn switch(&mut self, id: RuleId) -> Option<bool> {
        let rule = self.get_mut(id)?;
        rule.on = !rule.on;
        Some(rule.on)
    }

    /// Removes rule `id`; returns whether there was one.
    pub fn remove(&mut self, id: RuleId) -> bool {
        let before = self.list.len();
        self.list.retain(|(rule_id, _)| *rule_id != id);
        self.list.len() != before
    }

    /// The first rule that is on, not resting and whose test holds, if `tick` is one of the
    /// ticks rules are looked at. A read that is refused counts as a test that does not hold.
    /// Nothing changes: call [`Self::fired`] once the rule really started.
    pub fn due(
        &self,
        tick: u64,
        ask: &mut dyn FnMut(Query) -> Result<Answer, Denied>,
    ) -> Option<RuleId> {
        if !tick.is_multiple_of(CHECK_EVERY) {
            return None;
        }
        self.list
            .iter()
            .filter(|(_, rule)| rule.on && !rule.is_resting(tick))
            .find(|(_, rule)| rule.test.check(rule.anchor, ask).unwrap_or(false))
            .map(|(id, _)| *id)
    }

    /// Makes rule `id` rest from `tick`.
    pub fn fired(&mut self, id: RuleId, tick: u64) {
        if let Some(rule) = self.get_mut(id) {
            rule.rests_until = tick.saturating_add(REST_TICKS);
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::scripting::program::{Block, Compare, Plan, Read};

    fn cell() -> WorldPos {
        WorldPos { x: 0, y: 0, z: 0 }
    }

    fn program() -> Program {
        Program::try_new(vec![Block::Do(Plan::Hop { by: [1, 0, 0] })]).unwrap()
    }

    fn hunger_below(than: u32) -> Test {
        Test {
            read: Read::Hunger,
            compare: Compare::Below,
            than,
        }
    }

    fn answer(number: u32) -> impl FnMut(Query) -> Result<Answer, Denied> {
        move |_| Ok(Answer::Number(number))
    }

    #[test]
    fn a_true_rule_is_due_only_on_a_check_tick() {
        let mut rules = Rules::default();
        let id = rules.add(hunger_below(10), program(), cell()).unwrap();
        assert_eq!(rules.due(CHECK_EVERY, &mut answer(3)), Some(id));
        assert_eq!(rules.due(CHECK_EVERY + 1, &mut answer(3)), None);
    }

    #[test]
    fn a_false_test_or_a_refused_read_is_not_due() {
        let mut rules = Rules::default();
        rules.add(hunger_below(10), program(), cell()).unwrap();
        assert_eq!(rules.due(0, &mut answer(10)), None);
        assert_eq!(rules.due(0, &mut |_| Err(Denied::OverLimit)), None);
    }

    #[test]
    fn a_rule_rests_after_firing_and_wakes_after_the_rest() {
        let mut rules = Rules::default();
        let id = rules.add(hunger_below(10), program(), cell()).unwrap();
        rules.fired(id, 0);
        assert_eq!(rules.due(CHECK_EVERY, &mut answer(0)), None);
        assert_eq!(rules.due(REST_TICKS, &mut answer(0)), Some(id));
    }

    #[test]
    fn a_rule_that_is_off_never_fires() {
        let mut rules = Rules::default();
        let id = rules.add(hunger_below(10), program(), cell()).unwrap();
        assert_eq!(rules.switch(id), Some(false));
        assert_eq!(rules.due(0, &mut answer(0)), None);
        assert_eq!(rules.switch(id), Some(true));
        assert_eq!(rules.due(0, &mut answer(0)), Some(id));
    }

    #[test]
    fn the_oldest_true_rule_goes_first_and_the_next_gets_its_turn() {
        let mut rules = Rules::default();
        let first = rules.add(hunger_below(10), program(), cell()).unwrap();
        let second = rules.add(hunger_below(10), program(), cell()).unwrap();
        assert_eq!(rules.due(0, &mut answer(0)), Some(first));
        rules.fired(first, 0);
        assert_eq!(rules.due(CHECK_EVERY, &mut answer(0)), Some(second));
    }

    #[test]
    fn there_is_a_limit_and_removing_makes_room() {
        let mut rules = Rules::default();
        let ids: Vec<_> = (0..MAX_RULES)
            .map(|_| rules.add(hunger_below(1), program(), cell()).unwrap())
            .collect();
        assert_eq!(
            rules.add(hunger_below(1), program(), cell()),
            Err(RulesError::Full)
        );
        assert!(rules.remove(ids[3]));
        assert!(!rules.remove(ids[3]));
        assert!(rules.add(hunger_below(1), program(), cell()).is_ok());
    }

    #[test]
    fn replacing_keeps_the_spot_and_the_switch() {
        let mut rules = Rules::default();
        let anchor = WorldPos { x: 4, y: 5, z: 6 };
        let id = rules.add(hunger_below(10), program(), anchor).unwrap();
        rules.switch(id);
        rules.replace(id, hunger_below(2), program()).unwrap();
        let rule = rules.get(id).unwrap();
        assert_eq!((rule.anchor(), rule.is_on()), (anchor, false));
        assert_eq!(rule.test(), hunger_below(2));
        assert_eq!(
            rules.replace(RuleId(99), hunger_below(2), program()),
            Err(RulesError::Missing)
        );
    }

    proptest! {
        // However rules come and go, the count stays within the limit and every listed ID
        // can be found again.
        #[test]
        fn the_rule_count_never_passes_the_limit(steps in prop::collection::vec(0u8..3, 0..60)) {
            let mut rules = Rules::default();
            for step in steps {
                match step {
                    0 => { let _ = rules.add(hunger_below(5), program(), cell()); }
                    1 => {
                        let first = rules.iter().next().map(|(id, _)| id);
                        if let Some(id) = first {
                            rules.remove(id);
                        }
                    }
                    _ => {
                        let last = rules.iter().last().map(|(id, _)| id);
                        if let Some(id) = last {
                            rules.switch(id);
                        }
                    }
                }
                prop_assert!(rules.len() <= MAX_RULES);
                prop_assert!(rules.iter().all(|(id, _)| rules.get(id).is_some()));
            }
        }

        // A rule that fired cannot be due again before the rest is over, on any check tick.
        #[test]
        fn a_fired_rule_is_not_due_during_its_rest(at in 0u64..10_000, gap in 0u64..REST_TICKS) {
            let mut rules = Rules::default();
            let id = rules.add(hunger_below(10), program(), cell()).unwrap();
            rules.fired(id, at);
            let tick = (at + gap).next_multiple_of(CHECK_EVERY);
            if tick < at + REST_TICKS {
                prop_assert_eq!(rules.due(tick, &mut answer(0)), None);
            }
        }
    }
}
