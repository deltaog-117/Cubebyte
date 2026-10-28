//! The zombie's rules: chase, hit, take hits, spawn and despawn.
//!
//! Nothing here reads wall-clock time. Spawning draws from a stream the caller forks from
//! the simulation, so the same stream always puts the zombie in the same place.

use std::num::NonZeroU64;

use glam::{Vec2, Vec3};

use crate::blocks::registry::is_solid;
use crate::building::raycast;
use crate::combat::model::Damage;
use crate::combat::service::Outcome;
use crate::mobs::model::{HALF_WIDTH, HEIGHT, Zombie};
use crate::shared::body::{self, Intent};
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

/// Takes `damage` from the zombie's health. Hitting a dead zombie reports `Died` again and
/// changes nothing.
pub fn strike(zombie: &mut Zombie, damage: Damage) -> Outcome {
    zombie.health = zombie.health.saturating_sub(damage.points());
    if zombie.health == 0 {
        Outcome::Died
    } else {
        Outcome::Survived
    }
}

/// Whether the ray from `origin` along `direction` hits the zombie within `reach` before it
/// hits any block. `alpha` blends the zombie's box the way it is drawn, so the player hits
/// what they see.
pub fn aimed_at(
    zombie: &Zombie,
    world: &World,
    origin: Vec3,
    direction: Vec3,
    reach: f32,
    alpha: f32,
) -> bool {
    let (min, max) = zombie.bounds(alpha);
    let Some(distance) = ray_box(origin, direction, min, max).filter(|&d| d <= reach) else {
        return false;
    };
    let Some(hit) = raycast::cast(world, origin, direction, reach) else {
        return true;
    };
    let cell = Vec3::new(hit.block.x as f32, hit.block.y as f32, hit.block.z as f32);
    ray_box(origin, direction, cell, cell + Vec3::ONE).is_none_or(|block| distance <= block)
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

    #[test]
    fn strikes_take_health_and_the_last_one_kills() {
        let mut zombie = zombie_at(0.5, 0.5);
        assert_eq!(strike(&mut zombie, damage(4)), Outcome::Survived);
        assert_eq!(zombie.health(), MAX_HEALTH - 4);
        assert_eq!(strike(&mut zombie, damage(u8::MAX)), Outcome::Died);
        assert_eq!(zombie.health(), 0);
        assert_eq!(strike(&mut zombie, damage(1)), Outcome::Died);
    }

    #[test]
    fn the_player_aims_at_the_zombie_unless_a_block_is_in_the_way() {
        let mut world = floor();
        let zombie = zombie_at(6.5, 0.5);
        let eye = Vec3::new(0.5, 2.62, 0.5);
        assert!(aimed_at(&zombie, &world, eye, Vec3::X, 8.0, 1.0));
        assert!(
            !aimed_at(&zombie, &world, eye, Vec3::X, 4.0, 1.0),
            "out of reach"
        );
        assert!(
            !aimed_at(&zombie, &world, eye, Vec3::Z, 8.0, 1.0),
            "looking away"
        );
        set(&mut world, 3, 2, 0);
        assert!(
            !aimed_at(&zombie, &world, eye, Vec3::X, 8.0, 1.0),
            "wall between"
        );
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
