use glam::{Vec2, Vec3};
use minecraft_clone::player::model::Player;
use minecraft_clone::player::service::{Intent, collides, step};
use minecraft_clone::shared::event::Event;
use minecraft_clone::shared::voxel::{BlockId, WorldPos};
use minecraft_clone::world::model::World;
use proptest::prelude::*;

const DT: f32 = 0.05;

/// A stone floor with its top face at y = 1, and `blocks` stone cells scattered at
/// heights 1 to 6 above it, all below where the tests start the player.
fn terrain(blocks: &[(i32, i32, i32)]) -> Result<World, TestCaseError> {
    let mut world = World::default();
    let floor = (-12..12).flat_map(|x| (-12..12).map(move |z| (x, 0, z)));
    for (x, y, z) in floor.chain(blocks.iter().copied()) {
        let pos = WorldPos { x, y, z };
        let event = Event::BlockSet {
            pos,
            old: world.block_at(pos),
            new: BlockId::STONE,
        };
        prop_assert_eq!(world.apply(&event), Ok(()));
    }
    Ok(world)
}

fn intents() -> impl Strategy<Value = Vec<Intent>> {
    prop::collection::vec(
        ((-1.5f32..1.5, -1.5f32..1.5), any::<bool>()).prop_map(|((x, z), jump)| Intent {
            walk: Vec2::new(x, z),
            jump,
        }),
        0..200,
    )
}

fn blocks() -> impl Strategy<Value = Vec<(i32, i32, i32)>> {
    prop::collection::vec((-10i32..10, 1i32..7, -10i32..10), 0..80)
}

fn start(x: f32, y: f32, z: f32) -> Result<Player, TestCaseError> {
    Player::try_new(Vec3::new(x, y, z)).map_err(|error| TestCaseError::fail(error.to_string()))
}

proptest! {
    #[test]
    fn the_player_never_ends_a_step_inside_a_solid_block(
        blocks in blocks(),
        intents in intents(),
        from in (-4.0f32..4.0, 8.0f32..80.0, -4.0f32..4.0),
    ) {
        let world = terrain(&blocks)?;
        let mut player = start(from.0, from.1, from.2)?;
        for intent in intents {
            step(&mut player, intent, &world, DT);
            prop_assert!(!collides(&player, &world), "inside a block at {:?}", player.position());
            prop_assert!(player.position().y >= 1.0 - 1e-3, "fell through the floor");
        }
    }

    #[test]
    fn the_same_start_and_inputs_give_the_same_player(
        blocks in blocks(),
        intents in intents(),
        from in (-4.0f32..4.0, 8.0f32..80.0, -4.0f32..4.0),
    ) {
        let world = terrain(&blocks)?;
        let (mut first, mut second) = (start(from.0, from.1, from.2)?, start(from.0, from.1, from.2)?);
        for intent in intents {
            step(&mut first, intent, &world, DT);
            step(&mut second, intent, &world, DT);
        }
        prop_assert_eq!(first, second);
    }
}
