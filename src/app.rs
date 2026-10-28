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
use crate::building::service::REACH;
use crate::combat::service::{Outcome, hit, respawn, strike};
use crate::crafting::registry::{self as recipes, RecipeId};
use crate::crafting::service::craft_events;
use crate::events::apply::{Stage, StageError, apply_all};
use crate::events::log::EventLog;
use crate::events::persist::{self, LoadError};
use crate::events::replay::replay_chunk;
use crate::inventory::model::Inventory;
use crate::inventory::persist as inventory_persist;
use crate::inventory::service::{StockError, gather_events, place_events};
use crate::items::registry;
use crate::mobs::model::Zombie;
use crate::mobs::service as mobs;
use crate::player::input::MoveKeys;
use crate::player::model::{EYE_HEIGHT, Player, PlayerError};
use crate::player::service::{Intent, step};
use crate::rendering::camera::Camera;
use crate::rendering::meshing::{chunks_to_remesh, mesh_chunk};
use crate::rendering::pipeline::{RenderError, Renderer};
use crate::rendering::vertex::{box_mesh, gpu_mesh};
use crate::shared::clock::{ClockError, FixedClock, Tick};
use crate::shared::event::Event;
use crate::shared::intent::Walk;
use crate::shared::voxel::{CHUNK_SIZE, ChunkPos, WorldPos};
use crate::simulation::intent_log::{ActorId, IntentLog};
use crate::simulation::model::Simulation;
use crate::survival::model::Stats;
use crate::survival::service::eat_events;
use crate::ui::draw::overlay;
use crate::ui::layout::panel;
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

/// Key of the random stream that places zombies; the tick is mixed in so each differs.
const ZOMBIE_STREAM: u64 = 0x5a0b_1e00;

/// Steps between a zombie's death, or the player's, and the next one appearing: 10 seconds.
const ZOMBIE_DELAY_STEPS: u32 = 200;

const ZOMBIE_COLOR: [f32; 3] = [0.25, 0.55, 0.25];

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
        cursor: (0.0, 0.0),
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
    /// Where the player starts, and returns to after dying.
    spawn: Player,
    /// How many times the player has died this session.
    deaths: u32,
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
    /// The one hostile mob, if it is alive.
    zombie: Option<Zombie>,
    /// Steps left before the next zombie appears.
    zombie_timer: u32,
    /// Who the current zombie is in the intent log.
    zombie_actor: ActorId,
    /// The id the next zombie will get.
    next_actor: ActorId,
    /// What the player and each zombie asked for on every step, in memory only.
    intents: IntentLog,
    /// The recipe the craft key makes.
    recipe: RecipeId,
    /// Whether the crafting panel is open: the pointer is free and clicks go to the panel.
    panel_open: bool,
    terrain: TerrainGenerator,
    streamer: Streamer,
}

struct App {
    running: Option<Running>,
    keys: MoveKeys,
    /// The block a right click places.
    selected: SolidBlock,
    /// Where the pointer last was, in window pixels.
    cursor: (f32, f32),
    /// Why the loop was stopped early, handed back by [`run`].
    failure: Option<AppError>,
}

impl App {
    fn start(event_loop: &ActiveEventLoop) -> Result<Running, AppError> {
        let window = Arc::new(
            event_loop.create_window(Window::default_attributes().with_title("minecraft-clone"))?,
        );
        capture_cursor(&window);

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
            spawn: player,
            deaths: 0,
            flying: false,
            last_frame: Instant::now(),
            simulation: Simulation::new(SEED, clock),
            world: World::default(),
            log,
            inventory,
            inventory_dirty: false,
            stats: Stats::default(),
            zombie: None,
            zombie_timer: ZOMBIE_DELAY_STEPS,
            zombie_actor: ActorId::PLAYER.next(),
            next_actor: ActorId::PLAYER.next(),
            intents: IntentLog::default(),
            recipe: RecipeId::FIRST,
            panel_open: false,
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
            KeyCode::Tab if down => self.with_running(Running::toggle_panel),
            // Escape first closes the panel, and only then quits.
            KeyCode::Escape if down => match self.running.as_mut() {
                Some(running) if running.panel_open => running.toggle_panel(),
                _ => event_loop.exit(),
            },
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
        let (selected, cursor) = (self.selected, self.cursor);
        let Some(running) = self.running.as_mut() else {
            return;
        };
        if running.panel_open {
            if button == MouseButton::Left {
                running.click_panel(cursor);
            }
            return;
        }
        let (origin, direction) = (running.camera.position, running.camera.forward());
        if button == MouseButton::Left && running.hit_zombie(origin, direction) {
            return;
        }
        let planned = match button {
            MouseButton::Left => {
                gather_events(&running.world, &running.inventory, origin, direction)
            }
            MouseButton::Right => place_events(
                &running.world,
                &running.inventory,
                origin,
                direction,
                selected,
            ),
            _ => return,
        };
        match planned {
            Ok((pos, events)) => {
                if !running.commit_or_warn(&events) {
                    return;
                }
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
        // The clock has already moved past this frame's steps, so the first one is behind it.
        let first = running.simulation.tick().0.saturating_sub(u64::from(steps));
        if steps > 0 {
            let shown = (running.stats.health(), running.stats.hunger());
            for step_index in 0..u64::from(steps) {
                running.stats.step();
                running.step_zombie(Tick(first + step_index));
            }
            if running.stats.is_dead() {
                running.die();
            } else if (running.stats.health(), running.stats.hunger()) != shown {
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
                walk: Walk::from_direction(Vec2::new(direction.x, direction.z)),
                jump: self.keys.up,
            };
            for step_index in 0..u64::from(steps) {
                running.record(ActorId::PLAYER, Tick(first + step_index), intent);
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
        running.show_zombie();
        running.show_panel(self.cursor);
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

    /// Starts the player over at the spawn point with full meters and an empty pack.
    fn die(&mut self) {
        respawn(&mut self.stats, &mut self.inventory);
        self.inventory_dirty = true;
        self.deaths += 1;
        self.zombie = None;
        self.zombie_timer = ZOMBIE_DELAY_STEPS;
        self.player = self.spawn;
        self.flying = false;
        self.camera.position = self.player.eye(1.0);
        self.refresh_title();
    }

    /// Moves the zombie one step and lets it hit the player, or counts down to the next one.
    fn step_zombie(&mut self, tick: Tick) {
        let target = self.player.position();
        let Some(zombie) = self.zombie.as_mut() else {
            self.zombie_timer = self.zombie_timer.saturating_sub(1);
            if self.zombie_timer == 0 {
                self.spawn_zombie(target);
            }
            return;
        };
        if mobs::despawns(zombie, target) {
            self.zombie = None;
            self.zombie_timer = ZOMBIE_DELAY_STEPS;
        } else {
            let intent = mobs::decide(zombie, target, &self.world);
            if let Some(damage) =
                mobs::step_with(zombie, intent, target, &self.world, TICK_STEP.as_secs_f32())
            {
                hit(&mut self.stats, damage);
            }
            self.record(self.zombie_actor, tick, intent);
        }
    }

    /// Keeps `intent` in the log. A refusal means a step was recorded twice, which loses
    /// nothing of the run itself, so it is reported and play goes on.
    fn record(&mut self, actor: ActorId, tick: Tick, intent: Intent) {
        if let Err(error) = self.intents.record(actor, tick, intent) {
            tracing::warn!(%error, "cannot record an intent");
        }
    }

    fn spawn_zombie(&mut self, target: Vec3) {
        let mut rng = self
            .simulation
            .stream(ZOMBIE_STREAM ^ self.simulation.tick().0);
        let at = mobs::spawn_point(&mut rng, target, |x, z| self.terrain.height_at(x, z));
        match Zombie::try_new(at) {
            Ok(zombie) => {
                self.zombie = Some(zombie);
                self.zombie_actor = self.next_actor;
                self.next_actor = self.next_actor.next();
            }
            Err(error) => tracing::warn!(%error, "cannot place a zombie"),
        }
    }

    /// Hits the zombie if the crosshair is on it, with the best weapon in the pack. Returns
    /// whether it was, so the click does not also break the block behind it.
    fn hit_zombie(&mut self, origin: Vec3, direction: Vec3) -> bool {
        let alpha = self.simulation.alpha();
        let Some(zombie) = self.zombie.as_mut() else {
            return false;
        };
        if !mobs::aimed_at(zombie, &self.world, origin, direction, REACH, alpha) {
            return false;
        }
        if mobs::strike(zombie, strike(&self.inventory)) == Outcome::Died {
            self.zombie = None;
            self.zombie_timer = ZOMBIE_DELAY_STEPS;
        }
        true
    }

    /// Draws the zombie as a green box, or nothing when there is none.
    fn show_zombie(&mut self) {
        let (vertices, indices) = self.zombie.map_or_else(Default::default, |zombie| {
            let (min, max) = zombie.bounds(self.simulation.alpha());
            box_mesh(min, max, ZOMBIE_COLOR)
        });
        self.renderer.set_entities(&vertices, &indices);
    }

    /// Opens or closes the crafting panel, freeing the pointer while it is open.
    fn toggle_panel(&mut self) {
        self.panel_open = !self.panel_open;
        if self.panel_open {
            self.window.set_cursor_visible(true);
            if let Err(error) = self.window.set_cursor_grab(CursorGrabMode::None) {
                tracing::warn!(%error, "cannot release the cursor");
            }
        } else {
            capture_cursor(&self.window);
        }
    }

    /// Crafts the recipe whose row is under `cursor`, if there is one.
    fn click_panel(&mut self, cursor: (f32, f32)) {
        let layout = panel(self.renderer.size(), &self.inventory);
        if let Some(id) = layout.row_at(cursor.0, cursor.1) {
            self.craft_recipe(id);
        }
    }

    /// Draws the crafting panel while it is open, or removes it.
    fn show_panel(&mut self, cursor: (f32, f32)) {
        let (vertices, indices) = if self.panel_open {
            let size = self.renderer.size();
            let layout = panel(size, &self.inventory);
            let hovered = layout.row_at(cursor.0, cursor.1);
            overlay(&layout, size, hovered)
        } else {
            Default::default()
        };
        self.renderer.set_overlay(&vertices, &indices);
    }

    /// Applies `events` to the world, player and inventory together, all or nothing, and logs
    /// them only if they all applied.
    fn commit(&mut self, events: &[Event]) -> Result<(), StageError> {
        let mut stage = Stage {
            world: &mut self.world,
            player: &mut self.player,
            inventory: &mut self.inventory,
        };
        apply_all(&mut stage, events)?;
        events.iter().for_each(|&event| self.log.push(event));
        Ok(())
    }

    /// [`Self::commit`] for events just worked out from the current state, so a refusal is a
    /// bug: it is logged and reported as `false`.
    fn commit_or_warn(&mut self, events: &[Event]) -> bool {
        self.commit(events)
            .inspect_err(|error| tracing::warn!(%error, "change rejected"))
            .is_ok()
    }

    /// Crafts the selected recipe once.
    fn craft_selected(&mut self) {
        self.craft_recipe(self.recipe);
    }

    /// Crafts `id` once. Missing ingredients or a full pack are normal, not a fault.
    fn craft_recipe(&mut self, id: RecipeId) {
        let Ok(events) = craft_events(&self.inventory, id) else {
            return;
        };
        if self.commit_or_warn(&events) {
            self.inventory_dirty = true;
            self.refresh_title();
        }
    }

    /// Eats one food item. Being full or having no food is normal, not a fault.
    fn eat_food(&mut self) {
        let Ok(meal) = eat_events(&self.inventory, &self.stats) else {
            return;
        };
        if self.commit_or_warn(&meal.events) {
            self.stats.restore_hunger(meal.restored);
            self.inventory_dirty = true;
            self.refresh_title();
        }
    }

    /// Shows health, hunger, the carried items and the selected recipe in the window title
    /// until there is a HUD.
    fn refresh_title(&self) {
        let mut title = format!(
            "minecraft-clone | health {} hunger {} deaths {} |",
            self.stats.health().value(),
            self.stats.hunger().value(),
            self.deaths,
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
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
            }
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
            && !running.panel_open
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

/// Hides the pointer and keeps it in the window, for mouse look.
fn capture_cursor(window: &Window) {
    // Locked is unsupported on some platforms; confined still keeps the pointer inside.
    if window
        .set_cursor_grab(CursorGrabMode::Locked)
        .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
        .is_err()
    {
        tracing::warn!("cannot grab the cursor; mouse look may drift");
    }
    window.set_cursor_visible(false);
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
