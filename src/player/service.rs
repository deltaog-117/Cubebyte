//! Player movement: gravity, jumping and collision with solid blocks, one tick at a time.
//!
//! Only `+ - * /` and comparisons touch positions, with no trigonometry, and nothing reads
//! wall-clock time, so the same start and the same [`Intent`]s always give the same result.

use glam::Vec2;

use crate::blocks::registry::is_solid;
use crate::player::model::Player;
use crate::shared::voxel::WorldPos;
use crate::world::model::World;

/// Blocks per second on level ground.
pub const WALK_SPEED: f32 = 4.3;

/// Blocks per second squared.
pub const GRAVITY: f32 = 28.0;

/// Upward speed of a jump: about 1.4 blocks of height, enough to climb one block.
pub const JUMP_SPEED: f32 = 9.0;

/// Fastest fall, in blocks per second. At 20 steps per second it is 2.5 blocks a step.
pub const TERMINAL_SPEED: f32 = 50.0;

/// Gap kept between the box and a block it stops against.
///
/// Rounding can place the box a hair inside the face it was stopped at. Inside a block, the
/// next sweep would no longer see it and the player would fall through. Stopping short by
/// more than the rounding error rules that out.
const SKIN: f32 = 0.005;

/// Beyond this distance from the origin `f32` can no longer resolve [`SKIN`], so the player
/// stops moving instead of risking a fall through the floor.
const COORD_LIMIT: f32 = 8192.0;

/// What the player asks for in one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Intent {
    /// Wanted horizontal direction in world space as x and z. A length above 1 is cut to 1,
    /// and a non-finite value counts as standing still.
    pub walk: Vec2,
    pub jump: bool,
}

/// Moves `player` through one tick of `dt` seconds.
///
/// Gravity pulls first, then the box moves along Y, X and Z in turn and is stopped by solid
/// blocks, so it slides along walls. A jump starts only from the ground. A box that already
/// overlaps a block, for example one placed on top of the player, can still move out of it.
pub fn step(player: &mut Player, intent: Intent, world: &World, dt: f32) {
    player.previous = player.position;
    if !(dt.is_finite() && dt > 0.0) || player.position.abs().max_element() > COORD_LIMIT {
        return;
    }

    let walk = if intent.walk.is_finite() {
        intent.walk.clamp_length_max(1.0)
    } else {
        Vec2::ZERO
    };
    let mut velocity = player.velocity;
    velocity.x = walk.x * WALK_SPEED;
    velocity.z = walk.y * WALK_SPEED;
    if intent.jump && player.on_ground {
        velocity.y = JUMP_SPEED;
    }
    velocity.y = (velocity.y - GRAVITY * dt).max(-TERMINAL_SPEED);

    player.on_ground = false;
    for axis in [1, 0, 2] {
        let wanted = velocity[axis] * dt;
        let allowed = clip(world, player, axis, wanted);
        player.position[axis] += allowed;
        if allowed != wanted {
            velocity[axis] = 0.0;
            player.on_ground |= axis == 1 && wanted < 0.0;
        }
    }
    player.velocity = velocity;
}

/// Whether the player's box overlaps a solid block.
pub fn collides(player: &Player, world: &World) -> bool {
    let (min, max) = player.bounds();
    cell_range(min.x, max.x).any(|x| {
        cell_range(min.y, max.y).any(|y| cell_range(min.z, max.z).any(|z| solid(world, [x, y, z])))
    })
}

/// How far the box can move along `axis` towards `delta` before touching a solid block,
/// never more than `delta` and never in the opposite direction.
fn clip(world: &World, player: &Player, axis: usize, delta: f32) -> f32 {
    let (min, max) = player.bounds();
    let (first, second) = ((axis + 1) % 3, (axis + 2) % 3);
    let blocked = |cell: i32| {
        cell_range(min[first], max[first]).any(|a| {
            cell_range(min[second], max[second]).any(|b| {
                let mut at = [0; 3];
                at[axis] = cell;
                at[first] = a;
                at[second] = b;
                solid(world, at)
            })
        })
    };

    if delta > 0.0 {
        // Cells whose near face lies at or beyond the front of the box.
        let mut cells = max[axis].ceil() as i32..=(max[axis] + delta).floor() as i32;
        cells.find(|&cell| blocked(cell)).map_or(delta, |cell| {
            (cell as f32 - max[axis] - SKIN).clamp(0.0, delta)
        })
    } else if delta < 0.0 {
        // Cells whose near face lies at or behind the back of the box, nearest first.
        let mut cells =
            ((min[axis] + delta).ceil() as i32 - 1..=min[axis].floor() as i32 - 1).rev();
        cells.find(|&cell| blocked(cell)).map_or(delta, |cell| {
            ((cell + 1) as f32 - min[axis] + SKIN).clamp(delta, 0.0)
        })
    } else {
        0.0
    }
}

/// The cells a span from `low` to `high` overlaps. A span that only touches a cell's face
/// does not overlap it.
fn cell_range(low: f32, high: f32) -> std::ops::RangeInclusive<i32> {
    low.floor() as i32..=high.ceil() as i32 - 1
}

fn solid(world: &World, [x, y, z]: [i32; 3]) -> bool {
    is_solid(world.block_at(WorldPos { x, y, z }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::event::Event;
    use crate::shared::voxel::BlockId;
    use glam::Vec3;

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
        for x in -8..8 {
            for z in -8..8 {
                set(&mut world, x, 0, z);
            }
        }
        world
    }

    fn player_at(x: f32, y: f32, z: f32) -> Player {
        Player::try_new(Vec3::new(x, y, z)).unwrap()
    }

    fn run(player: &mut Player, intent: Intent, world: &World, steps: usize) {
        for _ in 0..steps {
            step(player, intent, world, DT);
        }
    }

    fn walk_towards(x: f32, z: f32) -> Intent {
        Intent {
            walk: Vec2::new(x, z),
            jump: false,
        }
    }

    #[test]
    fn a_falling_player_lands_on_the_floor_and_stays() {
        let world = floor();
        let mut player = player_at(0.5, 20.0, 0.5);
        run(&mut player, Intent::default(), &world, 200);
        assert!(player.on_ground());
        assert_eq!(player.velocity().y, 0.0);
        assert!((player.position().y - 1.0).abs() <= SKIN + 1e-4);
        assert!(!collides(&player, &world));
    }

    #[test]
    fn walking_into_a_wall_stops_against_it_and_sliding_continues_along_it() {
        let mut world = floor();
        for y in 1..4 {
            for z in -8..8 {
                set(&mut world, 3, y, z);
            }
        }
        let mut player = player_at(0.5, 1.0, 0.5);
        run(&mut player, walk_towards(1.0, 0.5), &world, 50);
        // The wall's west face is at x = 3; the box is 0.3 wide on each side.
        assert!(player.position().x <= 3.0 - 0.3);
        assert!(player.position().x > 3.0 - 0.3 - 0.05);
        assert!(
            player.position().z > 0.5,
            "should keep sliding along the wall"
        );
        assert!(!collides(&player, &world));
    }

    #[test]
    fn a_jump_needs_the_ground_and_clears_one_block() {
        let world = floor();
        let jump = Intent {
            walk: Vec2::ZERO,
            jump: true,
        };

        let mut airborne = player_at(0.5, 5.0, 0.5);
        step(&mut airborne, jump, &world, DT);
        assert!(airborne.velocity().y < 0.0, "no jump in mid-air");

        let mut player = player_at(0.5, 1.0, 0.5);
        run(&mut player, Intent::default(), &world, 5);
        let mut peak = player.position().y;
        step(&mut player, jump, &world, DT);
        for _ in 0..40 {
            step(&mut player, Intent::default(), &world, DT);
            peak = peak.max(player.position().y);
        }
        assert!(peak - 1.0 > 1.0, "peak {peak}");
        assert!(peak - 1.0 < 1.6, "peak {peak}");
        assert!(player.on_ground(), "lands again");
    }

    #[test]
    fn a_ceiling_stops_a_jump() {
        let mut world = floor();
        for x in -2..3 {
            for z in -2..3 {
                set(&mut world, x, 3, z);
            }
        }
        let mut player = player_at(0.5, 1.0, 0.5);
        run(&mut player, Intent::default(), &world, 5);
        let jump = Intent {
            walk: Vec2::ZERO,
            jump: true,
        };
        for _ in 0..30 {
            step(&mut player, jump, &world, DT);
            assert!(
                player.position().y + 1.8 <= 3.0,
                "head went into the ceiling"
            );
        }
    }

    #[test]
    fn a_fast_fall_does_not_tunnel_through_a_single_layer() {
        let world = floor();
        let mut player = player_at(0.5, 400.0, 0.5);
        run(&mut player, Intent::default(), &world, 400);
        assert!(player.position().y >= 1.0 - 1e-3);
    }

    #[test]
    fn a_non_finite_intent_counts_as_standing_still() {
        let world = floor();
        let mut player = player_at(0.5, 1.0, 0.5);
        run(
            &mut player,
            walk_towards(f32::NAN, f32::INFINITY),
            &world,
            10,
        );
        assert_eq!(player.position().x, 0.5);
        assert_eq!(player.position().z, 0.5);
    }

    #[test]
    fn walking_diagonally_is_not_faster_than_straight() {
        let world = floor();
        let mut straight = player_at(0.5, 1.0, 0.5);
        let mut diagonal = straight;
        run(&mut straight, walk_towards(1.0, 0.0), &world, 10);
        run(&mut diagonal, walk_towards(5.0, 5.0), &world, 10);
        let moved = |player: &Player| (player.position() - Vec3::new(0.5, 1.0, 0.5)).length();
        assert!(moved(&diagonal) <= moved(&straight) + 0.02);
    }

    #[test]
    fn a_player_inside_a_block_can_still_walk_out() {
        let mut world = floor();
        set(&mut world, 0, 1, 0);
        let mut player = player_at(0.5, 1.0, 0.5);
        assert!(collides(&player, &world));
        run(&mut player, walk_towards(1.0, 0.0), &world, 20);
        assert!(!collides(&player, &world));
    }

    #[test]
    fn the_view_blends_between_the_last_two_steps() {
        let world = floor();
        let mut player = player_at(0.5, 1.0, 0.5);
        run(&mut player, Intent::default(), &world, 5);
        step(&mut player, walk_towards(1.0, 0.0), &world, DT);
        let (start, end) = (player.eye(0.0).x, player.eye(1.0).x);
        assert!(end > start);
        assert!((player.eye(0.5).x - (start + end) / 2.0).abs() < 1e-5);
    }
}
