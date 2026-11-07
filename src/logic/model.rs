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
//!
//! A rule has a [`Trigger`]: it either polls its test once a second, or fires right after the
//! player does something ([`PlayerAction`]), optionally only if its test holds at that moment. A
//! rule never reacts to what a script, a rewind or a replay does, so a rule cannot start itself.

use thiserror::Error;

use crate::scripting::model::{Answer, Denied, Query};
use crate::scripting::program::{Compare, Program, Read, Test};
use crate::shared::voxel::WorldPos;

/// Most rules at once.
pub const MAX_RULES: usize = 8;

/// Rules are looked at once per this many ticks (one second at 20 ticks a second).
pub const CHECK_EVERY: u64 = 20;

/// Ticks a rule rests after it fires (ten seconds).
pub const REST_TICKS: u64 = 200;

/// The test a new rule starts with: act when hunger is low. An action rule that does not use
/// its test keeps this one, and the programs file does not store an unused test.
pub const DEFAULT_TEST: Test = Test {
    read: Read::Hunger,
    compare: Compare::Below,
    than: 10,
};

/// Identifies a rule. IDs are not reused while the game runs, so a stale ID finds nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RuleId(pub u8);

/// What a rule can react to: something the player does, or something that happens to them or
/// near them. The first five are the player's own acts; the last three are what the world did,
/// worked out from the events it logged (see [`crate::logic::react`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerAction {
    Break,
    Place,
    Craft,
    Eat,
    /// A hit on a mob.
    Hit,
    /// The player lost health: a mob's hit, or starving.
    Hurt,
    /// The player died and started over.
    Respawn,
    /// A mob appeared by itself, not by a script's command.
    MobAppears,
}

impl PlayerAction {
    /// Every kind, in the order the rules panel steps through them.
    pub const ALL: [Self; 8] = [
        Self::Break,
        Self::Place,
        Self::Craft,
        Self::Eat,
        Self::Hit,
        Self::Hurt,
        Self::Respawn,
        Self::MobAppears,
    ];

    /// The word the panel and the programs file use.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Break => "break",
            Self::Place => "place",
            Self::Craft => "craft",
            Self::Eat => "eat",
            Self::Hit => "hit",
            Self::Hurt => "hurt",
            Self::Respawn => "respawn",
            Self::MobAppears => "appear",
        }
    }

    /// The kind written as `word`.
    pub fn from_name(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == word)
    }
}

/// When a rule looks at its test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Once a second, firing when the test holds.
    Poll,
    /// Right after the player does `kind`. With `gated` the rule's test must hold at that
    /// moment too; without it the test is kept but ignored.
    On { kind: PlayerAction, gated: bool },
}

impl Trigger {
    /// The next trigger in the cycle the rules panel steps through: poll, then each kind of
    /// action, then poll again. A gate is carried from one action to the next.
    pub fn next(self) -> Self {
        match self {
            Self::Poll => Self::On {
                kind: PlayerAction::ALL[0],
                gated: false,
            },
            Self::On { kind, gated } => {
                let at = PlayerAction::ALL
                    .iter()
                    .position(|k| *k == kind)
                    .unwrap_or(0);
                match PlayerAction::ALL.get(at + 1) {
                    Some(&kind) => Self::On { kind, gated },
                    None => Self::Poll,
                }
            }
        }
    }

    /// The same trigger with its gate flipped; a polling rule has no gate and is unchanged.
    pub fn with_gate_flipped(self) -> Self {
        match self {
            Self::Poll => Self::Poll,
            Self::On { kind, gated } => Self::On {
                kind,
                gated: !gated,
            },
        }
    }

    /// Whether the rule's test is part of when it fires.
    pub const fn uses_test(self) -> bool {
        matches!(self, Self::Poll | Self::On { gated: true, .. })
    }
}

/// What a rule's program and test are anchored on, which is what their offsets count from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// One fixed cell: where the rule was made, or the block it was aimed at.
    Cell(WorldPos),
    /// The cell the player stands in, so the rule follows the player.
    Feet,
    /// The cell of the mob nearest to the player. With no mob in range the rule waits.
    Mob,
}

impl Anchor {
    /// The next kind in the cycle the rules panel steps through. `pin` is the cell a fixed
    /// anchor takes when the cycle comes back to it.
    pub fn next(self, pin: WorldPos) -> Self {
        match self {
            Self::Cell(_) => Self::Feet,
            Self::Feet => Self::Mob,
            Self::Mob => Self::Cell(pin),
        }
    }
}

/// One rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    test: Test,
    trigger: Trigger,
    program: Program,
    anchor: Anchor,
    on: bool,
    /// The first tick at which the rule may fire again.
    rests_until: u64,
}

impl Rule {
    pub fn test(&self) -> Test {
        self.test
    }

    pub fn trigger(&self) -> Trigger {
        self.trigger
    }

    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Whether the rule's own condition holds with its anchor on `cell`: its test when the
    /// trigger uses it (a read that is refused counts as not holding), and always otherwise.
    fn holds(&self, cell: WorldPos, ask: &mut dyn FnMut(Query) -> Result<Answer, Denied>) -> bool {
        !self.trigger.uses_test() || self.test.check(cell, ask).unwrap_or(false)
    }

    /// What the program's working cell starts on, and the test's reads count from.
    pub fn anchor(&self) -> Anchor {
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
        anchor: Anchor,
    ) -> Result<RuleId, RulesError> {
        self.add_with(Trigger::Poll, test, program, anchor)
    }

    /// Like [`Self::add`], with a chosen [`Trigger`]. `test` is what a polling or gated rule
    /// checks.
    ///
    /// # Errors
    ///
    /// Returns [`RulesError::Full`] when [`MAX_RULES`] rules exist.
    pub fn add_with(
        &mut self,
        trigger: Trigger,
        test: Test,
        program: Program,
        anchor: Anchor,
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
                trigger,
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

    /// Moves rule `id` onto `anchor`. It is ready to fire again at once.
    ///
    /// # Errors
    ///
    /// Returns [`RulesError::Missing`] for an ID no rule has.
    pub fn set_anchor(&mut self, id: RuleId, anchor: Anchor) -> Result<(), RulesError> {
        let rule = self.get_mut(id).ok_or(RulesError::Missing)?;
        rule.anchor = anchor;
        rule.rests_until = 0;
        Ok(())
    }

    /// Gives rule `id` another trigger. It is ready to fire again at once.
    ///
    /// # Errors
    ///
    /// Returns [`RulesError::Missing`] for an ID no rule has.
    pub fn set_trigger(&mut self, id: RuleId, trigger: Trigger) -> Result<(), RulesError> {
        let rule = self.get_mut(id).ok_or(RulesError::Missing)?;
        rule.trigger = trigger;
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

    /// The first polling rule that is on, not resting and whose test holds, if `tick` is one of
    /// the ticks rules are looked at. `resolve` turns an anchor into the cell it stands for right
    /// now; a rule whose anchor has no cell (no mob in range) is skipped. A read that is
    /// refused counts as a test that does not hold. Nothing changes: call [`Self::fired`] once
    /// the rule really started.
    pub fn due(
        &self,
        tick: u64,
        resolve: &mut dyn FnMut(Anchor) -> Option<WorldPos>,
        ask: &mut dyn FnMut(Query) -> Result<Answer, Denied>,
    ) -> Option<RuleId> {
        if !tick.is_multiple_of(CHECK_EVERY) {
            return None;
        }
        self.list
            .iter()
            .filter(|(_, rule)| rule.on && !rule.is_resting(tick))
            .filter(|(_, rule)| rule.trigger == Trigger::Poll)
            .find(|(_, rule)| resolve(rule.anchor).is_some_and(|cell| rule.holds(cell, ask)))
            .map(|(id, _)| *id)
    }

    /// The first rule that reacts to the player doing `kind` that is on, not resting and, when
    /// it is gated, whose test holds right now. Called when the action happened; nothing
    /// changes until [`Self::fired`].
    pub fn due_on(
        &self,
        kind: PlayerAction,
        tick: u64,
        resolve: &mut dyn FnMut(Anchor) -> Option<WorldPos>,
        ask: &mut dyn FnMut(Query) -> Result<Answer, Denied>,
    ) -> Option<RuleId> {
        self.list
            .iter()
            .filter(|(_, rule)| rule.on && !rule.is_resting(tick))
            .filter(|(_, rule)| matches!(rule.trigger, Trigger::On { kind: k, .. } if k == kind))
            .find(|(_, rule)| resolve(rule.anchor).is_some_and(|cell| rule.holds(cell, ask)))
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

    fn origin() -> WorldPos {
        WorldPos { x: 0, y: 0, z: 0 }
    }

    fn cell() -> Anchor {
        Anchor::Cell(origin())
    }

    /// Feet and fixed cells are at the origin; no mob is in range.
    fn resolve(anchor: Anchor) -> Option<WorldPos> {
        match anchor {
            Anchor::Cell(cell) => Some(cell),
            Anchor::Feet => Some(origin()),
            Anchor::Mob => None,
        }
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
        assert_eq!(
            rules.due(CHECK_EVERY, &mut resolve, &mut answer(3)),
            Some(id)
        );
        assert_eq!(
            rules.due(CHECK_EVERY + 1, &mut resolve, &mut answer(3)),
            None
        );
    }

    #[test]
    fn a_false_test_or_a_refused_read_is_not_due() {
        let mut rules = Rules::default();
        rules.add(hunger_below(10), program(), cell()).unwrap();
        assert_eq!(rules.due(0, &mut resolve, &mut answer(10)), None);
        assert_eq!(
            rules.due(0, &mut resolve, &mut |_| Err(Denied::OverLimit)),
            None
        );
    }

    #[test]
    fn a_rule_rests_after_firing_and_wakes_after_the_rest() {
        let mut rules = Rules::default();
        let id = rules.add(hunger_below(10), program(), cell()).unwrap();
        rules.fired(id, 0);
        assert_eq!(rules.due(CHECK_EVERY, &mut resolve, &mut answer(0)), None);
        assert_eq!(
            rules.due(REST_TICKS, &mut resolve, &mut answer(0)),
            Some(id)
        );
    }

    #[test]
    fn a_rule_that_is_off_never_fires() {
        let mut rules = Rules::default();
        let id = rules.add(hunger_below(10), program(), cell()).unwrap();
        assert_eq!(rules.switch(id), Some(false));
        assert_eq!(rules.due(0, &mut resolve, &mut answer(0)), None);
        assert_eq!(rules.switch(id), Some(true));
        assert_eq!(rules.due(0, &mut resolve, &mut answer(0)), Some(id));
    }

    #[test]
    fn the_oldest_true_rule_goes_first_and_the_next_gets_its_turn() {
        let mut rules = Rules::default();
        let first = rules.add(hunger_below(10), program(), cell()).unwrap();
        let second = rules.add(hunger_below(10), program(), cell()).unwrap();
        assert_eq!(rules.due(0, &mut resolve, &mut answer(0)), Some(first));
        rules.fired(first, 0);
        assert_eq!(
            rules.due(CHECK_EVERY, &mut resolve, &mut answer(0)),
            Some(second)
        );
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
        let anchor = Anchor::Cell(WorldPos { x: 4, y: 5, z: 6 });
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

    #[test]
    fn a_rule_on_a_mob_waits_while_there_is_none() {
        let mut rules = Rules::default();
        let id = rules.add(hunger_below(10), program(), Anchor::Mob).unwrap();
        assert_eq!(rules.due(0, &mut resolve, &mut answer(0)), None);
        let mut mob_at = |anchor| match anchor {
            Anchor::Mob => Some(WorldPos { x: 3, y: 0, z: 0 }),
            other => resolve(other),
        };
        assert_eq!(rules.due(0, &mut mob_at, &mut answer(0)), Some(id));
    }

    #[test]
    fn reads_count_from_the_resolved_cell() {
        let mut rules = Rules::default();
        rules
            .add(
                Test {
                    read: Read::Look { by: [0, 0, 0] },
                    compare: Compare::Is,
                    than: 7,
                },
                program(),
                Anchor::Feet,
            )
            .unwrap();
        let mut seen = None;
        let mut ask = |query| {
            seen = Some(query);
            Ok(Answer::Number(0))
        };
        let mut feet_at = |_| Some(WorldPos { x: 9, y: 8, z: 7 });
        rules.due(0, &mut feet_at, &mut ask);
        assert_eq!(
            seen,
            Some(Query::Look {
                pos: WorldPos { x: 9, y: 8, z: 7 }
            })
        );
    }

    #[test]
    fn moving_a_rule_to_a_new_anchor_wakes_it() {
        let mut rules = Rules::default();
        let id = rules.add(hunger_below(10), program(), cell()).unwrap();
        rules.fired(id, 0);
        assert_eq!(rules.set_anchor(id, Anchor::Feet), Ok(()));
        assert_eq!(rules.get(id).map(Rule::anchor), Some(Anchor::Feet));
        assert_eq!(
            rules.due(CHECK_EVERY, &mut resolve, &mut answer(0)),
            Some(id)
        );
        assert_eq!(
            rules.set_anchor(RuleId(99), Anchor::Mob),
            Err(RulesError::Missing)
        );
    }

    #[test]
    fn the_anchor_cycle_visits_every_kind_and_returns() {
        let pin = WorldPos { x: 1, y: 2, z: 3 };
        let mut anchor = Anchor::Cell(origin());
        let mut seen = Vec::new();
        for _ in 0..3 {
            anchor = anchor.next(pin);
            seen.push(anchor);
        }
        assert_eq!(seen, [Anchor::Feet, Anchor::Mob, Anchor::Cell(pin)]);
    }

    fn on(kind: PlayerAction, gated: bool) -> Trigger {
        Trigger::On { kind, gated }
    }

    #[test]
    fn an_action_rule_is_not_polled_and_a_polling_rule_is_not_an_action_rule() {
        let mut rules = Rules::default();
        let reacts = rules
            .add_with(
                on(PlayerAction::Break, false),
                hunger_below(10),
                program(),
                cell(),
            )
            .unwrap();
        let polls = rules.add(hunger_below(10), program(), cell()).unwrap();
        assert_eq!(rules.due(0, &mut resolve, &mut answer(0)), Some(polls));
        rules.fired(polls, 0);
        assert_eq!(rules.due(CHECK_EVERY, &mut resolve, &mut answer(0)), None);
        assert_eq!(
            rules.due_on(PlayerAction::Break, 1, &mut resolve, &mut answer(0)),
            Some(reacts)
        );
        assert_eq!(
            rules.due_on(PlayerAction::Place, 1, &mut resolve, &mut answer(0)),
            None
        );
    }

    #[test]
    fn an_ungated_rule_ignores_its_test_and_a_gated_one_checks_it() {
        let mut rules = Rules::default();
        let id = rules
            .add_with(
                on(PlayerAction::Eat, false),
                hunger_below(10),
                program(),
                cell(),
            )
            .unwrap();
        let due = |rules: &Rules, number| {
            rules.due_on(PlayerAction::Eat, 0, &mut resolve, &mut answer(number))
        };
        assert_eq!(due(&rules, 99), Some(id));
        rules.set_trigger(id, on(PlayerAction::Eat, true)).unwrap();
        assert_eq!(due(&rules, 99), None);
        assert_eq!(due(&rules, 3), Some(id));
        // A refused read counts as a test that does not hold.
        assert_eq!(
            rules.due_on(PlayerAction::Eat, 0, &mut resolve, &mut |_| Err(
                Denied::OverLimit
            )),
            None
        );
    }

    #[test]
    fn an_action_rule_rests_like_any_other_and_waits_for_its_anchor() {
        let mut rules = Rules::default();
        let id = rules
            .add_with(
                on(PlayerAction::Hit, false),
                hunger_below(10),
                program(),
                cell(),
            )
            .unwrap();
        rules.fired(id, 5);
        let due = |rules: &Rules, tick| {
            rules.due_on(PlayerAction::Hit, tick, &mut resolve, &mut answer(0))
        };
        assert_eq!(due(&rules, 6), None);
        assert_eq!(due(&rules, 5 + REST_TICKS), Some(id));
        rules.set_anchor(id, Anchor::Mob).unwrap();
        // No mob in range: the anchor has no cell, so the rule is skipped.
        assert_eq!(due(&rules, 5 + REST_TICKS), None);
    }

    #[test]
    fn stepping_the_trigger_visits_every_kind_once_and_comes_back() {
        let mut trigger = Trigger::Poll;
        let mut seen = Vec::new();
        for _ in 0..=PlayerAction::ALL.len() {
            trigger = trigger.next();
            seen.push(trigger);
        }
        assert_eq!(seen.last(), Some(&Trigger::Poll));
        assert_eq!(seen.len(), PlayerAction::ALL.len() + 1);
        assert_eq!(Trigger::Poll.with_gate_flipped(), Trigger::Poll);
        assert!(!on(PlayerAction::Craft, false).uses_test());
        assert!(
            on(PlayerAction::Craft, false)
                .with_gate_flipped()
                .uses_test()
        );
    }

    #[test]
    fn every_kind_is_found_by_its_own_name() {
        for kind in PlayerAction::ALL {
            assert_eq!(PlayerAction::from_name(kind.name()), Some(kind));
        }
        assert_eq!(PlayerAction::from_name("jump"), None);
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
                prop_assert_eq!(rules.due(tick, &mut resolve, &mut answer(0)), None);
            }
        }
    }
}
