//! A record of what every actor asked for on every tick.
//!
//! Intents are whole numbers (see [`Intent`]), and bodies consume exactly those numbers, so
//! feeding a track back into a fresh body reproduces the run bit for bit. The log lives in
//! memory; `simulation::persist` saves and loads it.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::shared::clock::Tick;
use crate::shared::intent::Intent;

/// Who an intent belongs to: the player, or one mob. A mob that dies and is replaced gets a
/// new id, so each track follows one body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActorId(pub u32);

impl ActorId {
    pub const PLAYER: Self = Self(0);

    /// The id after this one. It stops at the largest value instead of wrapping onto an id
    /// already used.
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

/// One actor's intent on one tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub tick: Tick,
    pub intent: Intent,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecordError {
    #[error("tick {tick:?} is not after tick {last:?}, already recorded for {actor:?}")]
    NotAfterLast {
        actor: ActorId,
        last: Tick,
        tick: Tick,
    },
}

/// Append-only intents, one track per actor with strictly rising ticks.
///
/// A track never holds two intents for one tick or goes back in time, so reading it in order
/// is always a valid replay. Tracks are separate because actors are stepped one after another
/// within a frame, so ticks are only ordered inside a track.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntentLog {
    tracks: BTreeMap<ActorId, Vec<Entry>>,
}

impl IntentLog {
    /// Adds `intent` for `actor` on `tick`.
    ///
    /// # Errors
    ///
    /// Returns [`RecordError::NotAfterLast`] if `tick` is not after the actor's latest
    /// recorded tick. The log is left unchanged.
    pub fn record(
        &mut self,
        actor: ActorId,
        tick: Tick,
        intent: Intent,
    ) -> Result<(), RecordError> {
        let track = self.tracks.entry(actor).or_default();
        if let Some(last) = track.last().map(|entry| entry.tick)
            && tick <= last
        {
            return Err(RecordError::NotAfterLast { actor, last, tick });
        }
        track.push(Entry { tick, intent });
        Ok(())
    }

    /// Everything recorded for `actor`, oldest first. Empty for an actor never seen.
    pub fn track(&self, actor: ActorId) -> &[Entry] {
        self.tracks.get(&actor).map_or(&[], Vec::as_slice)
    }

    /// Every actor with a track, in id order, each with its entries oldest first.
    pub fn tracks(&self) -> impl Iterator<Item = (ActorId, &[Entry])> {
        self.tracks
            .iter()
            .map(|(actor, track)| (*actor, track.as_slice()))
    }

    /// What `actor` asked for on `tick`, if anything was recorded for that tick.
    pub fn intent_at(&self, actor: ActorId, tick: Tick) -> Option<Intent> {
        let track = self.track(actor);
        track
            .binary_search_by_key(&tick, |entry| entry.tick)
            .ok()
            .map(|index| track[index].intent)
    }

    /// Forgets all but each track's newest entry. The newest stays so the rising-ticks rule
    /// keeps checking the next record; the caller must have saved the rest first.
    pub fn retain_newest(&mut self) {
        for track in self.tracks.values_mut() {
            let keep = track.len().saturating_sub(1);
            track.drain(..keep);
        }
    }

    /// Entries recorded across all actors.
    pub fn len(&self) -> usize {
        self.tracks.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::intent::Walk;
    use proptest::prelude::*;

    const ZOMBIE: ActorId = ActorId(1);

    fn walking(x: i8) -> Intent {
        Intent {
            walk: Walk::new(x, 0),
            jump: false,
        }
    }

    #[test]
    fn a_new_log_is_empty_and_an_unknown_actor_has_no_track() {
        let log = IntentLog::default();
        assert!(log.is_empty());
        assert_eq!(log.track(ZOMBIE), &[]);
        assert_eq!(log.intent_at(ZOMBIE, Tick(0)), None);
    }

    #[test]
    fn a_tick_that_is_not_after_the_last_is_refused_and_changes_nothing() {
        let mut log = IntentLog::default();
        assert_eq!(log.record(ZOMBIE, Tick(5), walking(1)), Ok(()));
        for tick in [Tick(5), Tick(4)] {
            assert_eq!(
                log.record(ZOMBIE, tick, walking(2)),
                Err(RecordError::NotAfterLast {
                    actor: ZOMBIE,
                    last: Tick(5),
                    tick
                })
            );
        }
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn retaining_the_newest_keeps_the_ordering_check_alive() {
        let mut log = IntentLog::default();
        for tick in 1..=3 {
            assert_eq!(log.record(ZOMBIE, Tick(tick), walking(1)), Ok(()));
        }
        log.retain_newest();
        assert_eq!(log.len(), 1);
        assert_eq!(log.intent_at(ZOMBIE, Tick(3)), Some(walking(1)));
        assert!(log.record(ZOMBIE, Tick(3), walking(1)).is_err());
        assert_eq!(log.record(ZOMBIE, Tick(4), walking(1)), Ok(()));
    }

    #[test]
    fn actors_keep_independent_clocks() {
        let mut log = IntentLog::default();
        assert_eq!(log.record(ZOMBIE, Tick(9), walking(1)), Ok(()));
        assert_eq!(log.record(ActorId::PLAYER, Tick(3), walking(2)), Ok(()));
        assert_eq!(log.intent_at(ActorId::PLAYER, Tick(3)), Some(walking(2)));
        assert_eq!(log.intent_at(ZOMBIE, Tick(3)), None);
    }

    #[test]
    fn the_next_id_never_wraps_onto_an_earlier_one() {
        assert_eq!(ActorId(7).next(), ActorId(8));
        assert_eq!(ActorId(u32::MAX).next(), ActorId(u32::MAX));
    }

    proptest! {
        #[test]
        fn every_recorded_intent_reads_back_in_order(
            gaps in prop::collection::vec((1u64..5, any::<i8>(), any::<bool>()), 0..100),
        ) {
            let mut log = IntentLog::default();
            let mut tick = 0;
            let mut expected = Vec::new();
            for (gap, x, jump) in gaps {
                tick += gap;
                let intent = Intent { walk: Walk::new(x, x), jump };
                prop_assert_eq!(log.record(ZOMBIE, Tick(tick), intent), Ok(()));
                expected.push(Entry { tick: Tick(tick), intent });
            }
            prop_assert_eq!(log.track(ZOMBIE), expected.as_slice());
            for entry in &expected {
                prop_assert_eq!(log.intent_at(ZOMBIE, entry.tick), Some(entry.intent));
            }
        }
    }
}
