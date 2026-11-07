//! What the world did that a rule can react to, worked out from the events it logged.
//!
//! The player's own acts (a break, a craft, a hit) are told apart by the action that was
//! performed: a break and a script's `build` log the same block event, so the events alone
//! could not say which one the player did. What the world does by itself is different. It is
//! logged as a passive action (a mob appearing, a mob's hit, hunger running out) or as a
//! respawn, and the events say what happened. Edits, rewinds and redos never react, which is
//! what keeps a rule's own script, and the undoing of anything, from setting a rule off.

use crate::events::log::ActionKind;
use crate::logic::model::PlayerAction;
use crate::shared::event::Event;

/// The kinds of reaction that an action of `kind` made of `events` sets off, each once, in
/// the order the events show them.
pub fn reactions(kind: ActionKind, events: &[Event]) -> Vec<PlayerAction> {
    match kind {
        ActionKind::Respawn => vec![PlayerAction::Respawn],
        ActionKind::Passive => {
            let mut found = Vec::new();
            for event in events {
                let action = match *event {
                    Event::HealthChanged { by } if by < 0 => PlayerAction::Hurt,
                    Event::MobSpawned { .. } => PlayerAction::MobAppears,
                    _ => continue,
                };
                if !found.contains(&action) {
                    found.push(action);
                }
            }
            found
        }
        ActionKind::Edit | ActionKind::Rewind | ActionKind::Redo => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use proptest::prelude::*;

    use super::*;
    use crate::shared::event::{MobId, Spot};

    fn spawned() -> Event {
        Event::MobSpawned {
            id: MobId::FIRST,
            at: Spot::from_vec3(Vec3::ZERO),
            health: 20,
        }
    }

    #[test]
    fn a_mobs_hit_and_a_new_mob_are_reactions_in_order() {
        let events = [
            spawned(),
            Event::HealthChanged { by: -3 },
            Event::HealthChanged { by: -1 },
        ];
        assert_eq!(
            reactions(ActionKind::Passive, &events),
            [PlayerAction::MobAppears, PlayerAction::Hurt]
        );
    }

    #[test]
    fn healing_and_hunger_are_not_reactions() {
        let events = [
            Event::HealthChanged { by: 2 },
            Event::HungerChanged { by: -1 },
            Event::MobHealthChanged {
                id: MobId::FIRST,
                by: -4,
            },
        ];
        assert!(reactions(ActionKind::Passive, &events).is_empty());
    }

    #[test]
    fn a_respawn_is_a_reaction_whatever_it_holds() {
        assert_eq!(reactions(ActionKind::Respawn, &[]), [PlayerAction::Respawn]);
    }

    proptest! {
        /// A rule's script commits edits, and a rewind or redo only undoes or redoes: none of
        /// them may set a rule off, whatever events they hold.
        #[test]
        fn edits_rewinds_and_redos_never_react(
            by in any::<i8>(),
            kind in prop_oneof![
                Just(ActionKind::Edit),
                Just(ActionKind::Rewind),
                Just(ActionKind::Redo),
            ],
        ) {
            let events = [spawned(), Event::HealthChanged { by }];
            prop_assert!(reactions(kind, &events).is_empty());
        }
    }
}
