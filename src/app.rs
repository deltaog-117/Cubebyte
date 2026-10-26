//! Wiring only: window, game loop and handing each feature what it needs.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{Vec2, Vec3};
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::error::{EventLoopError, OsError};
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::building::model::{EditError, SolidBlock};
use crate::crafting::registry::{self as recipes, RecipeId};
use crate::crafting::service::craft;
use crate::events::log::EventLog;
use crate::events::persist::{self, LoadError};
use crate::events::replay::replay_chunk;
use crate::inventory::model::Inventory;
use crate::inventory::persist as inventory_persist;
use crate::inventory::service::{StockError, gather_block, place_from_stock};
use crate::items::registry;
use crate::player::input::MoveKeys;
use crate::player::model::{EYE_HEIGHT, Player, PlayerError};
use crate::player::service::{Intent, step};
use crate::rendering::camera::Camera;
use crate::rendering::meshing::{chunks_to_remesh, mesh_chunk};
use crate::rendering::pipeline::{RenderError, Renderer};
use crate::rendering::vertex::gpu_mesh;
use crate::shared::clock::{ClockError, FixedClock};
use crate::shared::voxel::{CHUNK_SIZE, ChunkPos, WorldPos};
use crate::simulation::model::Simulation;
use crate::survival::model::Stats;
use crate::survival::service::eat;
use crate::world::generation::{MAX_HEIGHT, MIN_HEIGHT, TerrainGenerator};
use crate::world::model::World;
use crate::world::streaming::Streamer;

const SEED: u64 = 117;

/// Where the event log is kept, relative to the working directory.
const SAVE_PATH: &str = "world.mcel";

/// Where the inventory snapshot is kept, relative to the working directory.
const INVENTORY_PATH: &str = "inventory.mcin";

/// Chunks kept loaded in each horizontal direction around the camera.
const LOAD_RADIUS: u8 = 4;

/// Most chunks generated in one frame, so crossing into new terrain does not stall.
const MAX_LOADS_PER_FRAME: usize = 8;

/// Blocks per second.
const FLY_SPEED: f32 = 14.0;

/// Longest frame step, so a stall does not throw the camera across the map.
const MAX_FRAME_SECONDS: f32 = 0.1;

/// Length of one simulation step: 20 steps per second.
const TICK_STEP: Duration = Duration::from_millis(50);

/// Most steps run after one slow frame; the rest of the backlog is dropped.
const MAX_STEPS_PER_FRAME: u32 = 5;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("cannot start the event loop")]
    EventLoop(#[from] EventLoopError),
    #[error("cannot set up the simulation clock")]
    Clock(#[from] ClockError),
    #[error("cannot place the player")]
    Spawn(#[from] PlayerError),
    #[error("cannot create the window")]
    Window(#[from] OsError),
    #[error("rendering failed")]
    Render(#[from] RenderError),
    #[error("cannot load the saved world")]
    Load(#[from] LoadError),
    #[error("the saved world was made with seed {found}, but this build uses seed {expected}")]
    SeedMismatch { found: u64, expected: u64 },
    #[error("cannot save the world")]
    Save(#[source] io::Error),
    #[error("cannot load the saved inventory")]
    LoadInventory(#[from] inventory_persist::LoadError),
    #[error("cannot save the inventory")]
    SaveInventory(#[source] io::Error),
}

/// Opens the window and runs until it is closed.
///
/// # Errors
///
/// Fails if the event loop, window or graphics device cannot be set up, if the saved world
/// cannot be read or does not match the seed, if the saved inventory is damaged, if drawing
/// a frame fails for good, or if the world or inventory cannot be saved on exit.
pub fn run() -> Result<(), AppError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        running: None,
        keys: MoveKeys::default(),
        selected: SolidBlock::STONE,
        failure: None,
    };
    event_loop.run_app(&mut app)?;
    app.failure.map_or(Ok(()), Err)
}

struct Running {
    window: Arc<Window>,
    renderer: Renderer,
    camera: Camera,
    player: Player,
    /// Whether the camera flies freely instead of following the walking player.
    flying: bool,
    last_frame: Instant,
    simulation: Simulation,
    world: World,
    log: EventLog,
    inventory: Inventory,
    /// Whether the inventory differs from what is on disk, so exit knows to save it.
    inventory_dirty: bool,
    stats: Stats,
    /// The recipe the craft key makes.
    recipe: RecipeId,
    terrain: TerrainGenerator,
    streamer: Streamer,
}

struct App {
    running: Option<Running>,
    keys: MoveKeys,
    /// The block a right click places.
    selected: SolidBlock,
    /// Why the loop was stopped early, handed back by [`run`].
    failure: Option<AppError>,
}

impl App {
    fn start(event_loop: &ActiveEventLoop) -> Result<Running, AppError> {
        let window = Arc::new(
            event_loop.create_window(Window::default_attributes().with_title("minecraft-clone"))?,
        );
        // Locked is unsupported on some platforms; confined still keeps the pointer inside.
        if window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
            .is_err()
        {
            tracing::warn!("cannot grab the cursor; mouse look may drift");
        }
        window.set_cursor_visible(false);

        let size = window.inner_size();
        let renderer =
            pollster::block_on(Renderer::new(window.clone(), (size.width, size.height)))?;

        let log = open_log(Path::new(SAVE_PATH))?;
        let inventory = inventory_persist::load(Path::new(INVENTORY_PATH))?.unwrap_or_default();
        let clock = FixedClock::try_new(TICK_STEP, MAX_STEPS_PER_FRAME)?;
        let terrain = TerrainGenerator::new(SEED);
        // Two blocks above the surface, so the player drops onto the ground.
        let spawn = Vec3::new(0.5, terrain.height_at(0, 0) as f32 + 2.0, 0.5);
        let player = Player::try_new(spawn)?;
        let size = i32::from(CHUNK_SIZE);
        let mut running = Running {
            window,
            renderer,
            camera: Camera::new(player.eye(1.0)),
            player,
            flying: false,
            last_frame: Instant::now(),
            simulation: Simulation::new(SEED, clock),
            world: World::default(),
            log,
            inventory,
            inventory_dirty: false,
            stats: Stats::default(),
            recipe: RecipeId::FIRST,
            terrain,
            streamer: Streamer::new(
                LOAD_RADIUS,
                MIN_HEIGHT.div_euclid(size)..=MAX_HEIGHT.div_euclid(size),
            ),
        };
        // Show the whole starting area at once instead of growing it over several frames.
        running.stream(usize::MAX);
        running.refresh_title();
        Ok(running)
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: AppError) {
        self.failure = Some(error);
        event_loop.exit();
    }

    fn set_key(&mut self, event_loop: &ActiveEventLoop, event: &KeyEvent) {
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        let down = event.state == ElementState::Pressed;
        match code {
            KeyCode::KeyW => self.keys.forward = down,
            KeyCode::KeyS => self.keys.back = down,
            KeyCode::KeyA => self.keys.left = down,
            KeyCode::KeyD => self.keys.right = down,
            KeyCode::Space => self.keys.up = down,
            KeyCode::ShiftLeft => self.keys.down = down,
            KeyCode::Digit1 if down => self.selected = SolidBlock::STONE,
            KeyCode::Digit2 if down => self.selected = SolidBlock::DIRT,
            KeyCode::Digit3 if down => self.selected = SolidBlock::GRASS,
            KeyCode::KeyF if down => self.with_running(Running::toggle_flying),
            KeyCode::KeyR if down => self.with_running(|running| {
                running.recipe = recipes::next(running.recipe);
                running.refresh_title();
            }),
            KeyCode::KeyC if down => self.with_running(Running::craft_selected),
            KeyCode::KeyE if down => self.with_running(Running::eat_food),
            KeyCode::Escape if down => event_loop.exit(),
            _ => {}
        }
    }

    fn with_running(&mut self, act: impl FnOnce(&mut Running)) {
        if let Some(running) = self.running.as_mut() {
            act(running);
        }
    }

    /// Left click breaks the block under the crosshair and keeps what it yields, right
    /// click places the selected block against it, spending one from the inventory.
    fn click(&mut self, button: MouseButton) {
        let selected = self.selected;
        let Some(running) = self.running.as_mut() else {
            return;
        };
        let (origin, direction) = (running.camera.position, running.camera.forward());
        let edit = match button {
            MouseButton::Left => gather_block(
                &mut running.world,
                &mut running.log,
                &mut running.inventory,
                origin,
                direction,
            ),
            MouseButton::Right => place_from_stock(
                &mut running.world,
                &mut running.log,
                &mut running.inventory,
                origin,
                direction,
                selected,
            ),
            _ => return,
        };
        match edit {
            Ok(pos) => {
                running.inventory_dirty = true;
                running.remesh_around(pos);
                running.refresh_title();
            }
            // Clicking at empty air, with nothing to place, or with a full pack is normal,
            // not a fault.
            Err(
                StockError::Edit(
                    EditError::NothingInReach | EditError::NoFace | EditError::Occupied,
                )
                | StockError::Full
                | StockError::Missing,
            ) => {}
            Err(error) => tracing::warn!(%error, "edit rejected"),
        }
    }

    fn redraw(&mut self) -> Result<(), RenderError> {
        let Some(running) = self.running.as_mut() else {
            return Ok(());
        };
        let now = Instant::now();
        let elapsed = now - running.last_frame;
        let seconds = elapsed.as_secs_f32().min(MAX_FRAME_SECONDS);
        running.last_frame = now;
        // Later systems read the tick and draw from streams; survival needs neither.
        let steps = running.simulation.advance(elapsed);
        if steps > 0 {
            let shown = (running.stats.health(), running.stats.hunger());
            for _ in 0..steps {
                running.stats.step();
            }
            if (running.stats.health(), running.stats.hunger()) != shown {
                running.refresh_title();
            }
        }
        if running.flying {
            running.camera.fly(self.keys.axis(), FLY_SPEED * seconds);
        } else {
            let walk = self.keys.walk();
            let direction =
                running.camera.right() * walk.x + running.camera.level_forward() * walk.y;
            let intent = Intent {
                walk: Vec2::new(direction.x, direction.z),
                jump: self.keys.up,
            };
            for _ in 0..steps {
                step(
                    &mut running.player,
                    intent,
                    &running.world,
                    TICK_STEP.as_secs_f32(),
                );
            }
            running.camera.position = running.player.eye(running.simulation.alpha());
        }
        running.stream(MAX_LOADS_PER_FRAME);
        running.renderer.render(&running.camera)
    }
}

impl Running {
    /// Unloads chunks that fell out of range and loads up to `budget` of the nearest
    /// missing ones, then rebuilds the meshes that changed.
    fn stream(&mut self, budget: usize) {
        let camera = self.camera.position.floor().as_ivec3();
        let centre = WorldPos {
            x: camera.x,
            y: camera.y,
            z: camera.z,
        }
        .split()
        .0;
        let plan = self.streamer.plan(centre.x, centre.z);

        for pos in plan.unload {
            self.world.unload(pos);
            self.streamer.mark_unloaded(pos);
            self.renderer.set_mesh(pos, &[], &[]);
        }

        // A new chunk also changes the border faces of the chunks next to it.
        let mut stale = HashSet::new();
        for &pos in plan.load.iter().take(budget) {
            load_chunk(&mut self.world, &self.log, &self.terrain, pos);
            self.streamer.mark_loaded(pos);
            stale.insert(pos);
            for [dx, dy, dz] in NEIGHBOURS {
                stale.insert(ChunkPos {
                    x: pos.x.saturating_add(dx),
                    y: pos.y.saturating_add(dy),
                    z: pos.z.saturating_add(dz),
                });
            }
        }
        for pos in stale {
            if self.streamer.is_loaded(pos) {
                remesh(&mut self.renderer, &self.world, pos);
            }
        }
    }

    /// Switches between walking and free flight. Landing from a flight stands the player
    /// where the camera is, even if that is inside a block; the player can walk out of it.
    fn toggle_flying(&mut self) {
        self.flying = !self.flying;
        if !self.flying
            && let Ok(player) = Player::try_new(self.camera.position - Vec3::Y * EYE_HEIGHT)
        {
            self.player = player;
        }
    }

    /// Crafts the selected recipe once. Missing ingredients or a full pack are normal, not
    /// a fault.
    fn craft_selected(&mut self) {
        if craft(&mut self.inventory, self.recipe).is_ok() {
            self.inventory_dirty = true;
            self.refresh_title();
        }
    }

    /// Eats one food item. Being full or having no food is normal, not a fault.
    fn eat_food(&mut self) {
        if eat(&mut self.inventory, &mut self.stats).is_ok() {
            self.inventory_dirty = true;
            self.refresh_title();
        }
    }

    /// Shows health, hunger, the carried items and the selected recipe in the window title
    /// until there is a HUD.
    fn refresh_title(&self) {
        let mut title = format!(
            "minecraft-clone | health {} hunger {} |",
            self.stats.health().value(),
            self.stats.hunger().value(),
        );
        for id in registry::ids() {
            if let Some(def) = registry::get(id) {
                title.push_str(&format!(" {} {}", def.name, self.inventory.count_of(id)));
            }
        }
        if let Some(recipe) = recipes::get(self.recipe) {
            title.push_str(&format!(
                " | recipe {} (R next, C craft, E eat)",
                recipe.name
            ));
        }
        self.window.set_title(&title);
    }

    /// Rebuilds the meshes an edit at `pos` can have changed.
    fn remesh_around(&mut self, pos: WorldPos) {
        for chunk in chunks_to_remesh(pos) {
            remesh(&mut self.renderer, &self.world, chunk);
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        match Self::start(event_loop) {
            Ok(running) => self.running = Some(running),
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(running) = self.running.as_mut() {
                    running.renderer.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.set_key(event_loop, &event),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => self.click(button),
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw() {
                    self.fail(event_loop, error.into());
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let (DeviceEvent::MouseMotion { delta: (dx, dy) }, Some(running)) =
            (event, self.running.as_mut())
        {
            running.camera.look(dx as f32, dy as f32);
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        let Some(running) = &self.running else {
            return;
        };
        // An empty log was never edited, so there is nothing worth writing.
        if !running.log.is_empty()
            && let Err(error) = persist::save(Path::new(SAVE_PATH), SEED, &running.log)
        {
            self.failure.get_or_insert(AppError::Save(error));
        }
        if running.inventory_dirty
            && let Err(error) =
                inventory_persist::save(Path::new(INVENTORY_PATH), &running.inventory)
        {
            self.failure.get_or_insert(AppError::SaveInventory(error));
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}

/// Chunk offsets that share a face with a chunk.
const NEIGHBOURS: [[i32; 3]; 6] = [
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

/// The saved edits at `path`, or an empty log when there is no save yet. Chunks replay
/// their own edits as they stream in, so this is all a restart needs.
fn open_log(path: &Path) -> Result<EventLog, AppError> {
    match persist::load(path)? {
        Some(saved) if saved.seed == SEED => {
            tracing::info!(events = saved.log.len(), "loaded saved world");
            Ok(saved.log)
        }
        Some(saved) => Err(AppError::SeedMismatch {
            found: saved.seed,
            expected: SEED,
        }),
        None => Ok(EventLog::new()),
    }
}

fn remesh(renderer: &mut Renderer, world: &World, pos: ChunkPos) {
    let (vertices, indices) = gpu_mesh(&mesh_chunk(world, pos), pos);
    renderer.set_mesh(pos, &vertices, &indices);
}

/// Installs the generated terrain for `pos`, then the player's edits to it from the log.
fn load_chunk(world: &mut World, log: &EventLog, terrain: &TerrainGenerator, pos: ChunkPos) {
    if let Err(error) = world.insert_generated(pos, terrain.generate_chunk(pos)) {
        tracing::error!(?pos, %error, "cannot load generated chunk");
        return;
    }
    if let Err(error) = replay_chunk(world, log, pos) {
        tracing::error!(?pos, %error, "cannot replay edits onto chunk");
    }
}
