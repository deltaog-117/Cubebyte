//! The zombie's rules: chase, hit, take hits, spawn and despawn.
//!
//! Nothing here reads wall-clock time. Spawning draws from a stream the caller forks from
//! the simulation, so the same stream always puts the zombie in the same place. Mobs appear,
//! lose health and leave through events, so the functions that decide those things return
//! the events and leave applying them to the caller.

use std::num::NonZeroU64;

use glam::{Vec2, Vec3};

use crate::blocks::registry::is_solid;
use crate::building::raycast;
use crate::combat::model::Damage;
use crate::mobs::model::{HALF_WIDTH, HEIGHT, MAX_HEALTH, Mobs, Zombie};
use crate::shared::body::{self, Intent};
use crate::shared::event::{Event, MobId, Spot};
use crate::shared::intent::Walk;
use crate::shared::math::ray_box;
use crate::shared::rng::Rng;
use crate::shared::voxel::WorldPos;
use crate::world::model::World;

/// Fraction of the player's walking speed the zombie moves at, so the player can outrun it.
pub const WALK_FRACTION: f32 = 0.5;

/// Beyond this distance the zombie does not notice the player and stands still.
pub const SIGHT: f32 = 24.0;

/// Sideways distance (centre to centre) within which the zombie hits.
pub const REACH: f32 = 1.2;

/// Health points one zombie hit takes. Never zero, so the literal is checked by a test.
const HIT_POINTS: u8 = 2;

/// Steps between hits: one second at 20 steps per second.
pub const HIT_COOLDOWN: u8 = 20;

/// Nearest and farthest distance, in blocks along each axis, a zombie appears from the player.
pub const SPAWN_MIN: u32 = 12;
pub const SPAWN_MAX: u32 = 20;

/// A zombie farther than this from the player is dropped, since its chunks may be unloaded.
pub const DESPAWN_DISTANCE: f32 = 48.0;

/// Lowest height a zombie may fall to before it is dropped.
const VOID_Y: f32 = -256.0;

/// The damage one zombie hit deals.
fn hit_damage() -> Damage {
    Damage::try_new(HIT_POINTS).unwrap_or(Damage::FIST)
}

/// What the zombie sees of the player this tick.
struct Sight {
    offset: Vec2,
    /// Within [`SIGHT`], so the zombie notices the player.
    near: bool,
    /// Within [`REACH`] and level enough to hit.
    close: bool,
}

fn look(zombie: &Zombie, target: Vec3) -> Sight {
    let offset = Vec2::new(
        target.x - zombie.position().x,
        target.z - zombie.position().z,
    );
    let near = offset.is_finite() && offset.length() <= SIGHT;
    let close = near && offset.length() <= REACH && (target.y - zombie.position().y).abs() < HEIGHT;
    Sight {
        offset,
        near,
        close,
    }
}

/// What the zombie wants to do this tick, given `target`, the player's feet.
///
/// It walks straight at the player, and jumps when a block stands in its way. The result is
/// whole numbers, so it can be recorded and fed back through [`step_with`].
pub fn decide(zombie: &Zombie, target: Vec3, world: &World) -> Intent {
    let sight = look(zombie, target);
    let direction = if sight.near && !sight.close {
        sight.offset.normalize_or_zero()
    } else {
        Vec2::ZERO
    };
    Intent {
        walk: Walk::from_direction(direction * WALK_FRACTION),
        jump: blocked_ahead(zombie, direction, world),
    }
}

/// Moves `zombie` through one tick of `dt` seconds towards `target`, the player's feet.
///
/// Returns the damage when the zombie lands a hit this tick. It follows [`decide`], and hits
/// once the player is within [`REACH`] with [`HIT_COOLDOWN`] steps between hits.
pub fn step(zombie: &mut Zombie, target: Vec3, world: &World, dt: f32) -> Option<Damage> {
    let intent = decide(zombie, target, world);
    step_with(zombie, intent, target, world, dt)
}

/// Like [`step`], but moves by the given `intent` instead of deciding one, which is how a
/// recorded run is replayed. `target` still decides whether the zombie hits.
pub fn step_with(
    zombie: &mut Zombie,
    intent: Intent,
    target: Vec3,
    world: &World,
    dt: f32,
) -> Option<Damage> {
    zombie.cooldown = zombie.cooldown.saturating_sub(1);
    // Judged from where the zombie stood before it moved, as a hit is swung before the step.
    let close = look(zombie, target).close;
    body::step(&mut zombie.body, intent, &|cell| solid(world, cell), dt);

    if close && zombie.cooldown == 0 {
        zombie.cooldown = HIT_COOLDOWN;
        return Some(hit_damage());
    }
    None
}

/// Whether a solid block is in front of the knees, which is what makes the zombie jump.
fn blocked_ahead(zombie: &Zombie, direction: Vec2, world: &World) -> bool {
    if direction == Vec2::ZERO {
        return false;
    }
    let probe = zombie.position()
        + Vec3::new(direction.x, 0.0, direction.y) * (HALF_WIDTH + 0.1)
        + Vec3::Y * 0.5;
    let cell = probe.floor().as_ivec3();
    solid(world, [cell.x, cell.y, cell.z])
}

fn solid(world: &World, [x, y, z]: [i32; 3]) -> bool {
    is_solid(world.block_at(WorldPos { x, y, z }))
}

/// The events for `damage` landing on zombie `id`: the health it really loses, and its
/// removal when that leaves it with none. Damage past what is left is not counted, so the
/// events undo to exactly the health it had.
pub fn strike_events(id: MobId, zombie: &Zombie, damage: Damage) -> Vec<Event> {
    let health = zombie.health();
    let taken = damage.points().min(health);
    let mut events = Vec::new();
    if taken > 0 {
        // A zombie has at most `MAX_HEALTH`, so what it can lose fits a signed byte.
        let by = i8::try_from(taken).map_or(i8::MIN, i8::wrapping_neg);
        events.push(Event::MobHealthChanged { id, by });
    }
    if taken == health {
        events.push(removal(id, zombie, 0));
    }
    events
}

/// Whether the zombie's box overlaps a solid block, as when it would be put inside a wall.
pub fn blocked(zombie: &Zombie, world: &World) -> bool {
    body::collides(&zombie.body, &|cell| solid(world, cell))
}

/// The event for a new zombie `id` at `at`, with full health.
pub fn spawn_event(id: MobId, at: Vec3) -> Event {
    Event::MobSpawned {
        id,
        at: Spot::from_vec3(at),
        health: MAX_HEALTH,
    }
}

/// The event that drops zombie `id` as it is.
pub fn despawn_event(id: MobId, zombie: &Zombie) -> Event {
    removal(id, zombie, zombie.health())
}

/// The events that take every mob out of the world.
pub fn clear_events(mobs: &Mobs) -> Vec<Event> {
    mobs.iter()
        .map(|(id, zombie)| despawn_event(id, zombie))
        .collect()
}

fn removal(id: MobId, zombie: &Zombie, health: u8) -> Event {
    Event::MobRemoved {
        id,
        at: Spot::from_vec3(zombie.position()),
        health,
    }
}

/// How far along the ray from `origin` along `direction` it hits the zombie, if it does so
/// within `reach` before it hits any block. `alpha` blends the zombie's box the way it is
/// drawn, so the player hits what they see.
pub fn aim_distance(
    zombie: &Zombie,
    world: &World,
    origin: Vec3,
    direction: Vec3,
    reach: f32,
    alpha: f32,
) -> Option<f32> {
    let (min, max) = zombie.bounds(alpha);
    let distance = ray_box(origin, direction, min, max).filter(|&d| d <= reach)?;
    let Some(hit) = raycast::cast(world, origin, direction, reach) else {
        return Some(distance);
    };
    let cell = Vec3::new(hit.block.x as f32, hit.block.y as f32, hit.block.z as f32);
    ray_box(origin, direction, cell, cell + Vec3::ONE)
        .is_none_or(|block| distance <= block)
        .then_some(distance)
}

/// Whether the zombie should be dropped: too far from the player, or fallen out of the world.
pub fn despawns(zombie: &Zombie, target: Vec3) -> bool {
    let position = zombie.position();
    !(position - target).is_finite()
        || (position - target).length() > DESPAWN_DISTANCE
        || position.y < VOID_Y
}

/// Where a new zombie appears: [`SPAWN_MIN`] to [`SPAWN_MAX`] blocks from `around` along one
/// axis and anywhere within [`SPAWN_MAX`] along the other, one block above the ground that
/// `height_at` reports.
pub fn spawn_point(rng: &mut Rng, around: Vec3, height_at: impl Fn(i64, i64) -> i32) -> Vec3 {
    let span = NonZeroU64::MIN.saturating_add(u64::from(SPAWN_MAX - SPAWN_MIN));
    let lateral = NonZeroU64::MIN.saturating_add(2 * u64::from(SPAWN_MAX));
    let far = i64::from(SPAWN_MIN) + rng.below(span) as i64;
    let side = if rng.next_u64() & 1 == 0 { far } else { -far };
    let slide = rng.below(lateral) as i64 - i64::from(SPAWN_MAX);
    let (dx, dz) = if rng.next_u64() & 1 == 0 {
        (side, slide)
    } else {
        (slide, side)
    };
    let (x, z) = (around.x.floor() as i64 + dx, around.z.floor() as i64 + dz);
    Vec3::new(x as f32 + 0.5, height_at(x, z) as f32 + 2.0, z as f32 + 0.5)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::mobs::model::MAX_HEALTH;
    use crate::shared::event::Event;
    use crate::shared::rng::Rng;
    use crate::shared::voxel::BlockId;

    const DT: f32 = 0.05;

    fn set(world: &mut World, x: i32, y: i32, z: i32) {
        let pos = WorldPos { x, y, z };
        let event = Event::BlockSet {
            pos,
            old: world.block_at(pos),
            new: BlockId::STONE,
        };
        world.apply(&event).unwrap();
    }

    /// A stone floor whose top face is at y = 1.
    fn floor() -> World {
        let mut world = World::default();
        for x in -40..40 {
            for z in -40..40 {
                set(&mut world, x, 0, z);
            }
        }
        world
    }

    #[test]
    fn a_zombie_is_blocked_inside_a_wall_and_not_on_open_ground() {
        let world = floor();
        let standing = Zombie::try_new(Vec3::new(0.5, 1.0, 0.5)).unwrap();
        let buried = Zombie::try_new(Vec3::new(0.5, 0.0, 0.5)).unwrap();

        assert!(!blocked(&standing, &world));
        assert!(blocked(&buried, &world));
    }

    fn zombie_at(x: f32, z: f32) -> Zombie {
        Zombie::try_new(Vec3::new(x, 1.0, z)).unwrap()
    }

    fn damage(points: u8) -> Damage {
        Damage::try_new(points).unwrap()
    }

    #[test]
    fn the_hit_literal_is_a_real_hit() {
        assert_eq!(hit_damage().points(), HIT_POINTS);
    }

    #[test]
    fn a_zombie_in_sight_walks_closer_but_slower_than_the_player() {
        let world = floor();
        let mut zombie = zombie_at(0.5, 0.5);
        let target = Vec3::new(10.5, 1.0, 0.5);
        for _ in 0..20 {
            step(&mut zombie, target, &world, DT);
        }
        let walked = zombie.position().x - 0.5;
        assert!(walked > 1.0, "walked {walked}");
        assert!(
            walked < 4.3,
            "a second of player walking covers 4.3, not {walked}"
        );
    }

    #[test]
    fn replaying_the_decided_intents_gives_the_same_zombie_and_the_same_hits() {
        let world = floor();
        let target = Vec3::new(6.5, 1.0, 3.5);
        let mut live = zombie_at(0.5, 0.5);
        let mut decided = Vec::new();
        let mut live_hits = Vec::new();
        for _ in 0..80 {
            let intent = decide(&live, target, &world);
            decided.push(intent);
            live_hits.push(step_with(&mut live, intent, target, &world, DT));
        }

        let mut replayed = zombie_at(0.5, 0.5);
        let replayed_hits: Vec<_> = decided
            .into_iter()
            .map(|intent| step_with(&mut replayed, intent, target, &world, DT))
            .collect();
        assert_eq!(live, replayed);
        assert_eq!(live_hits, replayed_hits);
        assert!(live_hits.iter().any(Option::is_some), "it reached and hit");
    }

    #[test]
    fn a_zombie_out_of_sight_stands_still() {
        let world = floor();
        let mut zombie = zombie_at(0.5, 0.5);
        for _ in 0..20 {
            step(&mut zombie, Vec3::new(35.5, 1.0, 0.5), &world, DT);
        }
        assert_eq!(zombie.position().x, 0.5);
    }

    #[test]
    fn a_zombie_next_to_the_player_hits_once_per_cooldown() {
        let world = floor();
        let mut zombie = zombie_at(0.5, 0.5);
        let target = Vec3::new(1.5, 1.0, 0.5);
        let landed: Vec<usize> = (0..3 * usize::from(HIT_COOLDOWN))
            .filter(|_| step(&mut zombie, target, &world, DT).is_some())
            .collect();
        // The first hit lands at once, then one per cooldown.
        assert_eq!(landed, [0, 20, 40]);
    }

    #[test]
    fn a_zombie_climbs_a_one_block_step_to_reach_the_player() {
        let mut world = floor();
        for z in -5..6 {
            set(&mut world, 3, 1, z);
        }
        let mut zombie = zombie_at(0.5, 0.5);
        for _ in 0..60 {
            step(&mut zombie, Vec3::new(8.5, 2.0, 0.5), &world, DT);
        }
        assert!(zombie.position().x > 4.0, "x = {}", zombie.position().x);
    }

    fn apply_all(mobs: &mut Mobs, events: &[Event]) {
        for event in events {
            match *event {
                Event::MobSpawned { id, at, health } => {
                    mobs.spawn(id, at.to_vec3(), health).unwrap();
                }
                Event::MobRemoved { id, health, .. } => mobs.remove(id, health).unwrap(),
                Event::MobHealthChanged { id, by } => mobs.shift_health(id, by).unwrap(),
                other => panic!("not a mob event: {other:?}"),
            }
        }
    }

    #[test]
    fn strikes_take_health_and_the_last_one_kills() {
        let id = MobId::FIRST;
        let mut mobs = Mobs::default();
        apply_all(&mut mobs, &[spawn_event(id, Vec3::new(0.5, 1.0, 0.5))]);

        let events = strike_events(id, mobs.get(id).unwrap(), damage(4));
        assert_eq!(events, [Event::MobHealthChanged { id, by: -4 }]);
        apply_all(&mut mobs, &events);
        assert_eq!(mobs.get(id).map(Zombie::health), Some(MAX_HEALTH - 4));

        let events = strike_events(id, mobs.get(id).unwrap(), damage(u8::MAX));
        assert_eq!(events.len(), 2, "the last hit takes the rest, then removes");
        assert_eq!(
            events[0],
            Event::MobHealthChanged {
                id,
                by: -((MAX_HEALTH - 4) as i8)
            }
        );
        apply_all(&mut mobs, &events);
        assert!(mobs.is_empty());
    }

    #[test]
    fn striking_a_mob_and_undoing_it_restores_it_exactly() {
        let id = MobId::FIRST;
        let mut mobs = Mobs::default();
        apply_all(&mut mobs, &[spawn_event(id, Vec3::new(0.5, 1.0, 0.5))]);
        let before = mobs.clone();

        let events = strike_events(id, mobs.get(id).unwrap(), damage(u8::MAX));
        apply_all(&mut mobs, &events);
        let undo: Vec<Event> = events.iter().rev().map(|event| event.inverse()).collect();
        apply_all(&mut mobs, &undo);

        assert_eq!(mobs, before);
    }

    #[test]
    fn clearing_removes_every_mob_and_undoes_to_the_same_mobs() {
        let mut mobs = Mobs::default();
        let spawns = [MobId(1), MobId(5)].map(|id| spawn_event(id, Vec3::new(0.5, 1.0, 0.5)));
        apply_all(&mut mobs, &spawns);
        let before = mobs.clone();

        let events = clear_events(&mobs);
        apply_all(&mut mobs, &events);
        assert!(mobs.is_empty());
        let undo: Vec<Event> = events.iter().map(|event| event.inverse()).collect();
        apply_all(&mut mobs, &undo);
        assert_eq!(mobs, before);
    }

    #[test]
    fn the_player_aims_at_the_zombie_unless_a_block_is_in_the_way() {
        let mut world = floor();
        let zombie = zombie_at(6.5, 0.5);
        let eye = Vec3::new(0.5, 2.62, 0.5);
        let aimed = |world: &World, direction, reach| {
            aim_distance(&zombie, world, eye, direction, reach, 1.0)
        };
        let distance = aimed(&world, Vec3::X, 8.0).expect("in reach");
        assert!((distance - 5.7).abs() < 1e-3, "distance {distance}");
        assert_eq!(aimed(&world, Vec3::X, 4.0), None, "out of reach");
        assert_eq!(aimed(&world, Vec3::Z, 8.0), None, "looking away");
        set(&mut world, 3, 2, 0);
        assert_eq!(aimed(&world, Vec3::X, 8.0), None, "wall between");
    }

    #[test]
    fn a_zombie_far_away_or_fallen_is_dropped() {
        let target = Vec3::new(0.5, 1.0, 0.5);
        assert!(!despawns(&zombie_at(20.5, 0.5), target));
        assert!(despawns(&zombie_at(60.5, 0.5), target));
        assert!(despawns(
            &Zombie::try_new(Vec3::new(0.5, -300.0, 0.5)).unwrap(),
            target
        ));
    }

    proptest! {
        #[test]
        fn spawns_land_in_the_ring_and_repeat_for_a_seed(
            seed in any::<u64>(),
            x in -1000.0f32..1000.0,
            z in -1000.0f32..1000.0,
        ) {
            let around = Vec3::new(x, 0.0, z);
            let spawn = |seed| spawn_point(&mut Rng::from_seed(seed), around, |_, _| 10);
            let point = spawn(seed);
            prop_assert_eq!(point, spawn(seed));
            let (dx, dz) = ((point.x - x.floor() - 0.5).abs(), (point.z - z.floor() - 0.5).abs());
            prop_assert!(dx.max(dz) >= SPAWN_MIN as f32 && dx.max(dz) <= SPAWN_MAX as f32);
            prop_assert!(dx.min(dz) <= SPAWN_MAX as f32);
            prop_assert_eq!(point.y, 12.0);
        }
    }
}
