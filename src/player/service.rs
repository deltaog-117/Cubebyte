//! Player movement: the shared walking-body rules, fed from the player's world.
//!
//! The same start and the same [`Intent`]s always give the same result.

use crate::blocks::registry::is_solid;
use crate::player::model::Player;
use crate::shared::body;
pub use crate::shared::body::{GRAVITY, Intent, JUMP_SPEED, TERMINAL_SPEED, WALK_SPEED};
use crate::shared::voxel::WorldPos;
use crate::world::model::World;

/// Moves `player` through one tick of `dt` seconds.
///
/// Gravity pulls first, then the box moves along Y, X and Z in turn and is stopped by solid
/// blocks, so it slides along walls. A jump starts only from the ground. A box that already
/// overlaps a block, for example one placed on top of the player, can still move out of it.
pub fn step(player: &mut Player, intent: Intent, world: &World, dt: f32) {
    body::step(&mut player.body, intent, &|cell| solid(world, cell), dt);
}

/// Whether the player's box overlaps a solid block.
pub fn collides(player: &Player, world: &World) -> bool {
    body::collides(&player.body, &|cell| solid(world, cell))
}

fn solid(world: &World, [x, y, z]: [i32; 3]) -> bool {
    is_solid(world.block_at(WorldPos { x, y, z }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::body::SKIN;
    use crate::shared::event::Event;
    use crate::shared::voxel::BlockId;
    use glam::{Vec2, Vec3};

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
