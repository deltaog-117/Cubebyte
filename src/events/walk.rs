//! Walking as events, by checkpoint.
//!
//! Walking is worked out by the physics step from the player's intent, so logging every step
//! would only say again what the intent track already holds. Instead the game logs a
//! checkpoint where a position matters: the player's at each edit (so undoing it can walk
//! them back to where they stood), a mob's when it is hit or leaves. A checkpoint is a move
//! event from the position the log last recorded to the position now; the game has already
//! walked there, so [`checkpoints`] only names the events and the caller logs them without
//! applying them. Undone or redone later, they are ordinary moves.
//!
//! The player may have walked on since the edit, so a rewind starts with a bridge
//! ([`bridged`]): one move from where they stand to where the edit left them, logged inside
//! the rewind. Its inverse ends the matching redo, so a redo is still the exact opposite.

use crate::events::log::{ActionKind, EventLog};
use crate::mobs::model::Mobs;
use crate::player::model::Player;
use crate::shared::event::{Event, MobId, Spot};

/// The checkpoint events for `events`, about to be applied as one action of `kind`.
///
/// - An edit gets one for the player: from where the log last put them (or from where they
///   stand now, when the log has not put them anywhere yet) to where they stand now.
/// - A passive action or a respawn gets one for each mob that `events` hit or remove and
///   that has walked since the log last put it somewhere.
/// - A rewind or a redo gets none: [`bridged`] handles theirs.
///
/// Call it before applying `events`, because applying them can move or remove what it reads.
pub fn checkpoints(
    log: &EventLog,
    player: &Player,
    mobs: &Mobs,
    events: &[Event],
    kind: ActionKind,
) -> Vec<Event> {
    match kind {
        ActionKind::Edit => {
            let now = Spot::from_vec3(player.position());
            vec![Event::PlayerMoved {
                old: log.player_spot().unwrap_or(now),
                new: now,
            }]
        }
        ActionKind::Passive | ActionKind::Respawn => mob_checkpoints(log, mobs, events),
        ActionKind::Rewind | ActionKind::Redo => Vec::new(),
    }
}

/// One move for each mob that `events` hit or remove, in the order they first appear, from
/// where the log last put the mob to where it stands. A mob that has not walked, that the log
/// has not met, or that is not alive gets none.
fn mob_checkpoints(log: &EventLog, mobs: &Mobs, events: &[Event]) -> Vec<Event> {
    let mut seen: Vec<MobId> = Vec::new();
    for event in events {
        if let Event::MobHealthChanged { id, .. } | Event::MobRemoved { id, .. } = *event
            && !seen.contains(&id)
        {
            seen.push(id);
        }
    }
    seen.into_iter()
        .filter_map(|id| {
            let now = Spot::from_vec3(mobs.get(id)?.position());
            let before = log.mob_spot(id)?;
            (before != now).then_some(Event::MobMoved {
                id,
                old: before,
                new: now,
            })
        })
        .collect()
}

/// `events` with a move in front that takes the player from `player`, where they stand, to
/// where the first move in `events` starts from. Without a move in `events`, or when the player
/// already stands there, `events` come back unchanged.
pub fn bridged(mut events: Vec<Event>, player: Spot) -> Vec<Event> {
    let first = events.iter().find_map(|event| match *event {
        Event::PlayerMoved { old, .. } => Some(old),
        _ => None,
    });
    if let Some(start) = first
        && start != player
    {
        events.insert(
            0,
            Event::PlayerMoved {
                old: player,
                new: start,
            },
        );
    }
    events
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::events::rewind::inverse_run;
    use crate::mobs::service::spawn_event;

    fn spot(x: f32) -> Spot {
        Spot::from_vec3(Vec3::new(x, 0.0, 0.0))
    }

    fn player_at(x: f32) -> Player {
        Player::try_new(Vec3::new(x, 0.0, 0.0)).expect("a finite position")
    }

    #[test]
    fn an_edit_checkpoint_starts_where_the_log_last_put_the_player() {
        let mut log = EventLog::new();
        let none = checkpoints(
            &log,
            &player_at(5.0),
            &Mobs::default(),
            &[],
            ActionKind::Edit,
        );
        // Nothing logged yet: the walk is of no length.
        assert_eq!(
            none,
            [Event::PlayerMoved {
                old: spot(5.0),
                new: spot(5.0)
            }]
        );
        log.push(Event::PlayerMoved {
            old: spot(0.0),
            new: spot(2.0),
        });
        let walked = checkpoints(
            &log,
            &player_at(5.0),
            &Mobs::default(),
            &[],
            ActionKind::Edit,
        );
        assert_eq!(
            walked,
            [Event::PlayerMoved {
                old: spot(2.0),
                new: spot(5.0)
            }]
        );
    }

    #[test]
    fn only_edits_checkpoint_the_player() {
        let log = EventLog::new();
        for kind in [
            ActionKind::Passive,
            ActionKind::Respawn,
            ActionKind::Rewind,
            ActionKind::Redo,
        ] {
            let found = checkpoints(&log, &player_at(1.0), &Mobs::default(), &[], kind);
            assert!(found.is_empty(), "{kind:?} got {found:?}");
        }
    }

    #[test]
    fn a_hit_mob_is_checkpointed_once_and_only_if_it_walked() {
        let (id, other) = (MobId::FIRST, MobId::FIRST.next());
        let mut mobs = Mobs::default();
        let mut log = EventLog::new();
        for (mob, x) in [(id, 1.0), (other, 9.0)] {
            let event = spawn_event(mob, Vec3::new(x, 0.0, 0.0));
            let Event::MobSpawned { at, health, .. } = event else {
                unreachable!("spawn_event spawns");
            };
            mobs.spawn(mob, at.to_vec3(), health).expect("a new mob");
            log.push(event);
        }
        // Mob 1 walks on; mob 2 stays put.
        mobs.move_to(id, spot(1.0), spot(4.0))
            .expect("it stands there");
        let hits = [
            Event::MobHealthChanged { id, by: -2 },
            Event::MobHealthChanged { id, by: -2 },
            Event::MobHealthChanged { id: other, by: -2 },
        ];
        let found = checkpoints(&log, &player_at(0.0), &mobs, &hits, ActionKind::Passive);
        assert_eq!(
            found,
            [Event::MobMoved {
                id,
                old: spot(1.0),
                new: spot(4.0)
            }]
        );
    }

    #[test]
    fn a_bridge_walks_the_player_to_where_the_edit_left_them() {
        let edit = [
            Event::PlayerMoved {
                old: spot(0.0),
                new: spot(3.0),
            },
            Event::HungerChanged { by: 1 },
        ];
        let undo = bridged(inverse_run(&edit), spot(8.0));
        assert_eq!(
            undo.first(),
            Some(&Event::PlayerMoved {
                old: spot(8.0),
                new: spot(3.0)
            })
        );
        assert_eq!(undo.len(), 3);
    }

    #[test]
    fn no_bridge_where_the_player_already_stands_or_no_move_is_recorded() {
        let edit = [Event::PlayerMoved {
            old: spot(0.0),
            new: spot(3.0),
        }];
        let undo = inverse_run(&edit);
        assert_eq!(bridged(undo.clone(), spot(3.0)), undo);
        let plain = vec![Event::HungerChanged { by: 1 }];
        assert_eq!(bridged(plain.clone(), spot(8.0)), plain);
    }
}
