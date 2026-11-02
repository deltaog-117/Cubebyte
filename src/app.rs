//! Wiring only: window, game loop and handing each feature what it needs.

use std::collections::HashSet;
use std::env;
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
use crate::building::service::{REACH, break_target, place_target};
use crate::combat::service::{hit_events, respawn, respawn_events, strike};
use crate::crafting::registry::{self as recipes, RecipeId};
use crate::crafting::service::craft_events;
use crate::events::apply::{Stage, StageError, apply_all};
use crate::events::log::{ActionKind, EventLog};
use crate::events::persist::{self, LoadError};
use crate::events::redo::RedoStack;
use crate::events::replay::replay_chunk;
use crate::events::rewind::{log_action, rewind_events, touched_chunks};
use crate::inventory::model::Inventory;
use crate::inventory::persist as inventory_persist;
use crate::inventory::service::{StockError, gather_events_at, place_events_at};
use crate::items::registry;
use crate::mobs::model::Mobs;
use crate::mobs::service as mobs;
use crate::player::input::MoveKeys;
use crate::player::model::{EYE_HEIGHT, Player, PlayerError};
use crate::player::service::{Intent, step};
use crate::rendering::camera::Camera;
use crate::rendering::meshing::{chunks_to_remesh, mesh_chunk};
use crate::rendering::pipeline::{RenderError, Renderer};
use crate::rendering::vertex::{box_mesh, gpu_mesh};
use crate::shared::clock::{ClockError, FixedClock, Tick};
use crate::shared::event::{Event, MobId, Spot};
use crate::shared::intent::Walk;
use crate::shared::voxel::{BlockId, CHUNK_SIZE, ChunkPos, WorldPos};
use crate::simulation::action::Action;
use crate::simulation::intent_log::ActorId;
use crate::simulation::model::Simulation;
use crate::simulation::replay::{ArgsError, Replay, ReplayError, replay_session};
use crate::simulation::stream::{IntentStream, next_session_path, prune_sessions, session_path};
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

/// Where intent tracks are kept, relative to the working directory: one numbered file per
/// session, because ticks restart at zero each session and earlier sessions are kept.
const INTENTS_DIR: &str = "intents";

/// Most session files kept in [`INTENTS_DIR`], the one being recorded included. A session of
/// idle play is a few bytes, but an active one is not, and nothing else ever deletes them.
const MAX_SESSIONS: usize = 20;

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
    #[error("cannot save the intent track")]
    SaveIntents(#[source] io::Error),
    #[error("bad command line")]
    Args(#[from] ArgsError),
    #[error("cannot replay the session")]
    Replay(#[from] ReplayError),
}

/// Opens the window and runs until it is closed.
///
/// # Errors
///
/// Fails if the event loop, window or graphics device cannot be set up, if the saved world
/// cannot be read or does not match the seed, if the saved inventory is damaged, if drawing
/// a frame fails for good, or if the world or inventory cannot be saved on exit. Also fails
/// before opening a window if `--replay <number>` is malformed or names a session that cannot
/// be loaded.
pub fn run() -> Result<(), AppError> {
    let replay = replay_session(env::args().skip(1))?
        .map(|number| Replay::load(&session_path(Path::new(INTENTS_DIR), number)))
        .transpose()?;
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        running: None,
        keys: MoveKeys::default(),
        selected: SolidBlock::STONE,
        cursor: (0.0, 0.0),
        replay,
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
    /// Edits that `Z` undid and nothing has replaced since, for `Y` to bring back. Rebuilt from
    /// the log at startup, so it survives a restart.
    redo: RedoStack,
    inventory: Inventory,
    /// Whether the inventory differs from what is on disk, so exit knows to save it.
    inventory_dirty: bool,
    stats: Stats,
    /// Every hostile mob that is alive; they come and go through events.
    mobs: Mobs,
    /// Steps left before the next zombie appears, counted while no mob is alive.
    zombie_timer: u32,
    /// The id the next zombie will get. It is also the mob's actor in the intent log.
    next_mob: MobId,
    /// What the player and each zombie asked for on every step; settled steps are written to
    /// this session's file as play goes on.
    intents: IntentStream,
    /// The saved session driving the player and the zombie, until it has no step for a tick.
    replay: Option<Replay>,
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
    /// The session to play back, handed to the game when the window opens.
    replay: Option<Replay>,
    /// Why the loop was stopped early, handed back by [`run`].
    failure: Option<AppError>,
}

impl App {
    fn start(event_loop: &ActiveEventLoop, replay: Option<Replay>) -> Result<Running, AppError> {
        let window = Arc::new(
            event_loop.create_window(Window::default_attributes().with_title("minecraft-clone"))?,
        );
        capture_cursor(&window);

        let size = window.inner_size();
        let renderer =
            pollster::block_on(Renderer::new(window.clone(), (size.width, size.height)))?;

        let log = open_log(Path::new(SAVE_PATH))?;
        let redo = RedoStack::from_log(&log);
        let inventory = inventory_persist::load(Path::new(INVENTORY_PATH))?.unwrap_or_default();
        // Makes room for the new session. A replayed session is already in memory by now.
        if let Err(error) = prune_sessions(Path::new(INTENTS_DIR), MAX_SESSIONS - 1) {
            tracing::warn!(%error, "cannot delete old intent sessions");
        }
        let intents = IntentStream::new(
            next_session_path(Path::new(INTENTS_DIR)).map_err(AppError::SaveIntents)?,
        );
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
            redo,
            inventory,
            inventory_dirty: false,
            stats: Stats::default(),
            mobs: Mobs::default(),
            zombie_timer: ZOMBIE_DELAY_STEPS,
            next_mob: MobId::FIRST,
            intents,
            replay,
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
            KeyCode::KeyZ if down => self.with_running(|running| running.act(Action::Rewind)),
            KeyCode::KeyY if down => self.with_running(|running| running.act(Action::Redo)),
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
        if button == MouseButton::Left
            && let Some(id) = running.aimed_mob(origin, direction)
        {
            running.act(Action::Strike(actor_of(id)));
            return;
        }
        let action = match button {
            MouseButton::Left => break_target(&running.world, origin, direction).map(Action::Break),
            MouseButton::Right => {
                place_target(&running.world, origin, direction).map(|pos| Action::Place {
                    pos,
                    block: selected.id(),
                })
            }
            _ => return,
        };
        // Clicking at empty air, or from inside a block, is normal, not a fault.
        if let Ok(action) = action {
            running.act(action);
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
                let tick = Tick(first + step_index);
                running.replay_actions(tick);
                running.step_stats();
                running.step_zombie(tick);
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
                let tick = Tick(first + step_index);
                let intent = running.player_intent(tick, intent);
                running.record(ActorId::PLAYER, tick, intent);
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
        running.show_mobs();
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
        // A flight leaves no steps in the track, so it would break the replay.
        if self.replay.is_some() {
            return;
        }
        self.flying = !self.flying;
        if !self.flying
            && let Ok(player) = Player::try_new(self.camera.position - Vec3::Y * EYE_HEIGHT)
        {
            self.player = player;
        }
    }

    /// Starts the player over at the spawn point with full meters and an empty pack. The
    /// respawn is logged as a barrier, so `Z` cannot reach back past it.
    fn die(&mut self) {
        let mut events = respawn_events(&self.stats, &self.inventory);
        events.extend(mobs::clear_events(&self.mobs));
        let (from, to) = (
            Spot::from_vec3(self.player.position()),
            Spot::from_vec3(self.spawn.position()),
        );
        if from != to {
            events.push(Event::PlayerMoved { old: from, new: to });
        }
        if let Err(error) = self.commit_as(&events, ActionKind::Respawn) {
            // The events were just worked out from this state, so a refusal is a bug; the
            // player still starts over, only the log misses it.
            tracing::warn!(%error, "respawn rejected");
            respawn(&mut self.stats, &mut self.inventory);
            self.mobs = Mobs::default();
        }
        self.stats.restart_clock();
        self.redo.clear();
        self.inventory_dirty = true;
        self.deaths += 1;
        self.zombie_timer = ZOMBIE_DELAY_STEPS;
        self.player = self.spawn;
        self.flying = false;
        self.camera.position = self.player.eye(1.0);
        self.refresh_title();
    }

    /// Moves every mob one step and lets it hit the player, or counts down to the next
    /// zombie while there is none.
    fn step_zombie(&mut self, tick: Tick) {
        let target = self.player.position();
        if self.mobs.is_empty() {
            self.zombie_timer = self.zombie_timer.saturating_sub(1);
            if self.zombie_timer == 0 {
                self.spawn_zombie(target);
            }
            return;
        }
        let ids: Vec<MobId> = self.mobs.iter().map(|(id, _)| id).collect();
        for id in ids {
            self.step_mob(id, tick, target);
        }
        if self.mobs.is_empty() {
            self.zombie_timer = ZOMBIE_DELAY_STEPS;
        }
    }

    /// Moves mob `id` one step towards `target` and lets it hit the player, or drops it when
    /// it is too far away.
    fn step_mob(&mut self, id: MobId, tick: Tick, target: Vec3) {
        let actor = actor_of(id);
        let Some(zombie) = self.mobs.get_mut(id) else {
            return;
        };
        if mobs::despawns(zombie, target) {
            let event = mobs::despawn_event(id, zombie);
            self.commit_passive(&[event]);
            return;
        }
        // Past the recorded steps, for instance after the original zombie was hit and
        // killed, it decides for itself.
        let intent = self
            .replay
            .as_ref()
            .and_then(|replay| replay.intent(actor, tick))
            .unwrap_or_else(|| mobs::decide(zombie, target, &self.world));
        if let Some(damage) =
            mobs::step_with(zombie, intent, target, &self.world, TICK_STEP.as_secs_f32())
        {
            let events = hit_events(&self.stats, damage);
            self.commit_passive(&events);
        }
        self.record(actor, tick, intent);
    }

    /// What the player does on `tick`: the recorded step while a replay has one, otherwise
    /// `live`. The first tick the replay has no step for ends it and hands control to the keys.
    fn player_intent(&mut self, tick: Tick, live: Intent) -> Intent {
        let recorded = self
            .replay
            .as_ref()
            .map(|replay| replay.intent(ActorId::PLAYER, tick));
        match recorded {
            Some(Some(intent)) => intent,
            Some(None) => {
                self.replay = None;
                tracing::info!(?tick, "replay finished; the keys are live again");
                self.refresh_title();
                live
            }
            None => live,
        }
    }

    /// Keeps `intent` in the log. A refusal means a step was recorded twice, which loses
    /// nothing of the run itself, and a failed write leaves the steps in memory to be
    /// written later, so both are reported and play goes on.
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
        let id = self.next_mob;
        self.commit_passive(&[mobs::spawn_event(id, at)]);
        self.next_mob = id.next();
    }

    /// The mob the crosshair is on, nearest first, so the click hits it and does not also
    /// break the block behind it.
    fn aimed_mob(&self, origin: Vec3, direction: Vec3) -> Option<MobId> {
        self.mobs
            .iter()
            .filter_map(|(id, zombie)| {
                mobs::aim_distance(
                    zombie,
                    &self.world,
                    origin,
                    direction,
                    REACH,
                    self.simulation.alpha(),
                )
                .map(|distance| (id, distance))
            })
            .min_by(|(_, near), (_, far)| near.total_cmp(far))
            .map(|(id, _)| id)
    }

    /// Hits mob `id` with the best weapon in the pack. Reports whether it did: it did not if
    /// that mob is gone. The hit and a kill are logged as passive events, so `Z` does not
    /// undo them.
    fn strike_mob(&mut self, id: MobId) -> bool {
        let damage = strike(&self.inventory);
        let Some(zombie) = self.mobs.get(id) else {
            return false;
        };
        let events = mobs::strike_events(id, zombie, damage);
        self.commit_passive(&events);
        if self.mobs.is_empty() {
            self.zombie_timer = ZOMBIE_DELAY_STEPS;
        }
        true
    }

    /// Draws every mob as a green box, or nothing when there are none.
    fn show_mobs(&mut self) {
        let alpha = self.simulation.alpha();
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        for (_, zombie) in self.mobs.iter() {
            let (min, max) = zombie.bounds(alpha);
            let (more_vertices, more_indices) = box_mesh(min, max, ZOMBIE_COLOR);
            let first = vertices.len() as u32;
            indices.extend(more_indices.into_iter().map(|index| first + index));
            vertices.extend(more_vertices);
        }
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
    /// them as one edit only if they all applied.
    fn commit(&mut self, events: &[Event]) -> Result<(), StageError> {
        self.commit_as(events, ActionKind::Edit)?;
        // A new edit starts a new future, so what was rewound can no longer come back.
        self.redo.clear();
        Ok(())
    }

    /// Applies and logs a change the world made by itself, such as hunger draining or a
    /// zombie's hit. It is not an edit, so `Z` never undoes it and the redo stack stays. The
    /// events were just worked out from the current state, so a refusal is a bug: it is
    /// logged and play goes on.
    fn commit_passive(&mut self, events: &[Event]) {
        if let Err(error) = self.commit_as(events, ActionKind::Passive) {
            tracing::warn!(%error, "passive change rejected");
        }
    }

    /// Counts one step of survival and applies what it drains as passive events.
    fn step_stats(&mut self) {
        let drain = self.stats.advance();
        let mut events = Vec::new();
        if drain.hunger != 0 {
            events.push(Event::HungerChanged { by: drain.hunger });
        }
        if drain.health != 0 {
            events.push(Event::HealthChanged { by: drain.health });
        }
        self.commit_passive(&events);
    }

    /// [`Self::commit`] for an action of any `kind`.
    fn commit_as(&mut self, events: &[Event], kind: ActionKind) -> Result<(), StageError> {
        let mut stage = Stage {
            world: &mut self.world,
            player: &mut self.player,
            inventory: &mut self.inventory,
            stats: &mut self.stats,
            mobs: &mut self.mobs,
        };
        apply_all(&mut stage, events)?;
        log_action(&mut self.log, events, kind);
        Ok(())
    }

    /// Steps back the latest edit still in effect. Reports whether it did. Nothing left to
    /// undo is normal, and so is a respawn in the way; a refusal (the state moved on in a way
    /// no event records, such as walking, or a meter that the undo would push out of range)
    /// is logged.
    fn rewind(&mut self) -> bool {
        let Some(events) = rewind_events(&self.log) else {
            return false;
        };
        self.load_touched(&events);
        if let Err(error) = self.commit_as(&events, ActionKind::Rewind) {
            tracing::warn!(%error, "rewind rejected");
            return false;
        }
        self.redo.record(&events);
        self.finish_history_step(&events);
        true
    }

    /// Brings back the edit `Z` undid last. It is logged as a redo, which a later `Z` can
    /// undo again. If the game has moved on in a way no event records, the edit no longer
    /// fits: it is refused and logged, and the older ones are dropped with it. Reports whether
    /// it brought an edit back.
    fn redo(&mut self) -> bool {
        let Some(events) = self.redo.take() else {
            return false;
        };
        self.load_touched(&events);
        if let Err(error) = self.commit_as(&events, ActionKind::Redo) {
            tracing::warn!(%error, "redo rejected");
            self.redo.clear();
            return false;
        }
        self.finish_history_step(&events);
        true
    }

    /// Brings in every unloaded chunk that `events` change, generated and with its logged
    /// edits, before a rewind or redo applies them: an edit may be in a chunk the player has
    /// walked away from. The streamer unloads it again once it is out of range, and the log
    /// keeps its edits.
    fn load_touched(&mut self, events: &[Event]) {
        for pos in touched_chunks(events) {
            if self.streamer.is_loaded(pos) {
                continue;
            }
            load_chunk(&mut self.world, &self.log, &self.terrain, pos);
            self.streamer.mark_loaded(pos);
            // The new chunk also changes the border faces of the loaded chunks next to it.
            remesh(&mut self.renderer, &self.world, pos);
            for [dx, dy, dz] in NEIGHBOURS {
                let next = ChunkPos {
                    x: pos.x.saturating_add(dx),
                    y: pos.y.saturating_add(dy),
                    z: pos.z.saturating_add(dz),
                };
                if self.streamer.is_loaded(next) {
                    remesh(&mut self.renderer, &self.world, next);
                }
            }
        }
    }

    /// Redraws and flags for saving after a rewind or redo applied `events`.
    fn finish_history_step(&mut self, events: &[Event]) {
        for event in events {
            if let Event::BlockSet { pos, .. } = *event {
                self.remesh_around(pos);
            }
        }
        self.inventory_dirty = true;
        self.refresh_title();
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
        self.act(Action::Craft(id.0));
    }

    /// Eats one food item. Being full or having no food is normal, not a fault.
    fn eat_food(&mut self) {
        self.act(Action::Eat);
    }

    /// Does `action` for the player now. While a replay runs the keys and the mouse do not
    /// act, since the replay supplies every action itself.
    fn act(&mut self, action: Action) {
        if self.replay.is_none() {
            self.perform(self.simulation.tick(), action);
        }
    }

    /// Performs what the replay recorded for `tick`, before that tick's steps, as the player
    /// did it then.
    fn replay_actions(&mut self, tick: Tick) {
        let due: Vec<Action> = self
            .replay
            .as_ref()
            .map(|replay| replay.actions_at(tick).collect())
            .unwrap_or_default();
        for action in due {
            self.perform(tick, action);
        }
    }

    /// Does `action` on `tick`, and keeps it in the intent log if it worked. A refusal (no
    /// block there, nothing to place, a missing ingredient) changes and records nothing.
    fn perform(&mut self, tick: Tick, action: Action) {
        let done = match action {
            Action::Break(pos) => self.break_at(pos),
            Action::Place { pos, block } => self.place_at(pos, block),
            Action::Craft(recipe) => self.craft_once(RecipeId(recipe)),
            Action::Eat => self.eat_once(),
            Action::Strike(actor) => self.strike_mob(mob_of(actor)),
            Action::Rewind => self.rewind(),
            Action::Redo => self.redo(),
        };
        if !done {
            return;
        }
        if let Err(error) = self.intents.record_action(tick, action) {
            tracing::warn!(%error, "cannot record an action");
        }
        self.inventory_dirty = true;
        self.refresh_title();
    }

    /// Breaks the block at `pos` and keeps what it yields. Reports whether it did.
    fn break_at(&mut self, pos: WorldPos) -> bool {
        let planned = gather_events_at(&self.world, &self.inventory, pos);
        let Some(events) = unless_routine(planned) else {
            return false;
        };
        if !self.commit_or_warn(&events) {
            return false;
        }
        self.remesh_around(pos);
        true
    }

    /// Places `block` at `pos`, spending one from the pack. Reports whether it did.
    fn place_at(&mut self, pos: WorldPos, block: BlockId) -> bool {
        let Ok(block) = SolidBlock::try_new(block) else {
            tracing::warn!(?block, "cannot place a block that is not solid");
            return false;
        };
        let planned = place_events_at(&self.world, &self.inventory, pos, block);
        let Some(events) = unless_routine(planned) else {
            return false;
        };
        if !self.commit_or_warn(&events) {
            return false;
        }
        self.remesh_around(pos);
        true
    }

    /// Crafts `id` once. Reports whether it did.
    fn craft_once(&mut self, id: RecipeId) -> bool {
        let Ok(events) = craft_events(&self.inventory, id) else {
            return false;
        };
        self.commit_or_warn(&events)
    }

    /// Eats one food item. Reports whether it did.
    fn eat_once(&mut self) -> bool {
        let Ok(meal) = eat_events(&self.inventory, &self.stats) else {
            return false;
        };
        self.commit_or_warn(&meal.events)
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
                " | recipe {} (R next, C craft, E eat, Z rewind, Y redo)",
                recipe.name
            ));
        }
        if self.replay.is_some() {
            title.push_str(" | replaying a saved run (keys are live when it ends)");
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
        match Self::start(event_loop, self.replay.take()) {
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
        let Some(running) = self.running.take() else {
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
        if let Err(error) = running.intents.finish() {
            self.failure.get_or_insert(AppError::SaveIntents(error));
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(running) = &self.running {
            running.window.request_redraw();
        }
    }
}

/// The mob that is intent-log actor `actor`; the inverse of [`actor_of`].
const fn mob_of(actor: ActorId) -> MobId {
    MobId(actor.0)
}

/// The intent-log actor that is mob `id`: mobs take the actor with their own number.
const fn actor_of(id: MobId) -> ActorId {
    ActorId(id.0)
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

/// The plan, or `None` if the edit was refused. Aiming at air, a full pack and having nothing
/// to place are normal play; anything else is reported.
fn unless_routine<T>(planned: Result<T, StockError>) -> Option<T> {
    planned
        .inspect_err(|error| {
            let routine = matches!(
                error,
                StockError::Edit(
                    EditError::NothingInReach | EditError::NoFace | EditError::Occupied
                ) | StockError::Full
                    | StockError::Missing
            );
            if !routine {
                tracing::warn!(%error, "edit rejected");
            }
        })
        .ok()
}

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
