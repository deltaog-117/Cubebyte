//! Wiring only: window, game loop and handing each feature what it needs.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use glam::Vec3;
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::error::{EventLoopError, OsError};
use winit::event::{DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::building::model::{EditError, SolidBlock};
use crate::building::service::{break_block, place_block};
use crate::events::log::EventLog;
use crate::events::replay::replay_chunk;
use crate::player::input::MoveKeys;
use crate::rendering::camera::Camera;
use crate::rendering::meshing::{chunks_to_remesh, mesh_chunk};
use crate::rendering::pipeline::{RenderError, Renderer};
use crate::rendering::vertex::gpu_mesh;
use crate::shared::voxel::{CHUNK_SIZE, ChunkPos, WorldPos};
use crate::world::generation::{MAX_HEIGHT, MIN_HEIGHT, TerrainGenerator};
use crate::world::model::World;
use crate::world::streaming::Streamer;

const SEED: u64 = 117;

/// Chunks kept loaded in each horizontal direction around the camera.
const LOAD_RADIUS: u8 = 4;

/// Most chunks generated in one frame, so crossing into new terrain does not stall.
const MAX_LOADS_PER_FRAME: usize = 8;

/// Blocks per second.
const FLY_SPEED: f32 = 14.0;

/// Longest frame step, so a stall does not throw the camera across the map.
const MAX_FRAME_SECONDS: f32 = 0.1;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("cannot start the event loop")]
    EventLoop(#[from] EventLoopError),
    #[error("cannot create the window")]
    Window(#[from] OsError),
    #[error("rendering failed")]
    Render(#[from] RenderError),
}

/// Opens the window and runs until it is closed.
///
/// # Errors
///
/// Fails if the event loop, window or graphics device cannot be set up, or if drawing a
/// frame fails for good.
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
    last_frame: Instant,
    world: World,
    log: EventLog,
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

        let terrain = TerrainGenerator::new(SEED);
        let eye_height = terrain.height_at(0, 0) as f32 + 6.0;
        let size = i32::from(CHUNK_SIZE);
        let mut running = Running {
            window,
            renderer,
            camera: Camera::new(Vec3::new(0.0, eye_height, 0.0)),
            last_frame: Instant::now(),
            world: World::default(),
            log: EventLog::new(),
            terrain,
            streamer: Streamer::new(
                LOAD_RADIUS,
                MIN_HEIGHT.div_euclid(size)..=MAX_HEIGHT.div_euclid(size),
            ),
        };
        // Show the whole starting area at once instead of growing it over several frames.
        running.stream(usize::MAX);
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
            KeyCode::Escape if down => event_loop.exit(),
            _ => {}
        }
    }

    /// Left click breaks the block under the crosshair, right click places the selected
    /// block against it.
    fn click(&mut self, button: MouseButton) {
        let selected = self.selected;
        let Some(running) = self.running.as_mut() else {
            return;
        };
        let (origin, direction) = (running.camera.position, running.camera.forward());
        let edit = match button {
            MouseButton::Left => {
                break_block(&mut running.world, &mut running.log, origin, direction)
            }
            MouseButton::Right => place_block(
                &mut running.world,
                &mut running.log,
                origin,
                direction,
                selected,
            ),
            _ => return,
        };
        match edit {
            Ok(pos) => running.remesh_around(pos),
            // Clicking at empty air is normal, not a fault.
            Err(EditError::NothingInReach | EditError::NoFace | EditError::Occupied) => {}
            Err(error) => tracing::warn!(%error, "edit rejected"),
        }
    }

    fn redraw(&mut self) -> Result<(), RenderError> {
        let Some(running) = self.running.as_mut() else {
            return Ok(());
        };
        let now = Instant::now();
        let seconds = (now - running.last_frame)
            .as_secs_f32()
            .min(MAX_FRAME_SECONDS);
        running.last_frame = now;
        running.camera.fly(self.keys.axis(), FLY_SPEED * seconds);
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
