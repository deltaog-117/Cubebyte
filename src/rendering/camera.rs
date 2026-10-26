//! Free-flying camera.
//!
//! Pure maths on `glam` types, with no `wgpu` or `winit`, so it is tested without a window.

use glam::camera::rh::proj::directx;
use glam::camera::rh::view;
use glam::{Mat4, Vec3};

/// Radians per pixel of mouse movement.
const LOOK_SENSITIVITY: f32 = 0.0025;

/// Pitch stops just short of straight up or down, where the view direction would be
/// parallel to the up vector and the view matrix would degenerate.
const MAX_PITCH: f32 = std::f32::consts::FRAC_PI_2 - 0.01;

const FOV_Y: f32 = std::f32::consts::FRAC_PI_3;
const NEAR: f32 = 0.1;
const FAR: f32 = 1000.0;

/// Where the viewer is and which way it looks.
///
/// `pitch` is private and only [`Camera::look`] writes it, so it always stays within
/// `±MAX_PITCH`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    /// Radians around the Y axis. Zero looks down `-Z`; positive turns left.
    yaw: f32,
    /// Radians above the horizon.
    pitch: f32,
}

impl Camera {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// Turns by a mouse movement in pixels. Moving right turns right, moving down looks down.
    pub fn look(&mut self, dx: f32, dy: f32) {
        self.yaw = (self.yaw - dx * LOOK_SENSITIVITY).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch - dy * LOOK_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Unit vector the camera looks along.
    pub fn forward(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch)
    }

    /// Unit vector to the camera's right, level with the ground.
    pub fn right(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vec3::new(cos_yaw, 0.0, -sin_yaw)
    }

    /// Unit vector the camera faces, level with the ground, for walking.
    pub fn level_forward(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vec3::new(-sin_yaw, 0.0, -cos_yaw)
    }

    /// Moves by `axis` (x = right, y = up, z = forward) scaled by `distance`.
    ///
    /// Forward and right follow the view direction, up is world up, so looking down and
    /// pressing forward descends like a spectator camera.
    pub fn fly(&mut self, axis: Vec3, distance: f32) {
        let step = self.right() * axis.x + Vec3::Y * axis.y + self.forward() * axis.z;
        self.position += step * distance;
    }

    /// World space to clip space, with depth in `0..=1` as `wgpu` expects.
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        let view = view::look_to_mat4(self.position, self.forward(), Vec3::Y);
        directx::perspective(FOV_Y, aspect, NEAR, FAR) * view
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const EPSILON: f32 = 1e-4;

    #[test]
    fn a_new_camera_looks_down_negative_z() {
        let camera = Camera::new(Vec3::ZERO);
        assert!((camera.forward() - Vec3::NEG_Z).length() < EPSILON);
        assert!((camera.right() - Vec3::X).length() < EPSILON);
    }

    #[test]
    fn a_point_ahead_projects_inside_the_view() {
        let camera = Camera::new(Vec3::ZERO);
        let clip = camera.view_projection(1.0) * Vec3::new(0.0, 0.0, -10.0).extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() < EPSILON && ndc.y.abs() < EPSILON);
        assert!((0.0..=1.0).contains(&ndc.z));
    }

    proptest! {
        #[test]
        fn pitch_never_leaves_its_limit(
            moves in prop::collection::vec((-5000.0f32..5000.0, -5000.0f32..5000.0), 0..50),
        ) {
            let mut camera = Camera::new(Vec3::ZERO);
            for (dx, dy) in moves {
                camera.look(dx, dy);
                prop_assert!(camera.pitch().abs() <= MAX_PITCH);
            }
        }

        #[test]
        fn the_view_basis_stays_orthonormal(
            dx in -5000.0f32..5000.0,
            dy in -5000.0f32..5000.0,
        ) {
            let mut camera = Camera::new(Vec3::ZERO);
            camera.look(dx, dy);
            prop_assert!((camera.forward().length() - 1.0).abs() < EPSILON);
            prop_assert!((camera.right().length() - 1.0).abs() < EPSILON);
            prop_assert!(camera.forward().dot(camera.right()).abs() < EPSILON);
        }

        #[test]
        fn the_level_forward_is_a_horizontal_unit_vector_under_the_view(
            dx in -5000.0f32..5000.0,
            dy in -5000.0f32..5000.0,
        ) {
            let mut camera = Camera::new(Vec3::ZERO);
            camera.look(dx, dy);
            let level = camera.level_forward();
            prop_assert_eq!(level.y, 0.0);
            prop_assert!((level.length() - 1.0).abs() < EPSILON);
            prop_assert!(level.dot(camera.right()).abs() < EPSILON);
            prop_assert!(level.dot(camera.forward()) > 0.0);
        }

        #[test]
        fn flying_forward_then_back_returns_to_the_start(
            dx in -5000.0f32..5000.0,
            dy in -5000.0f32..5000.0,
            distance in 0.0f32..100.0,
        ) {
            let mut camera = Camera::new(Vec3::new(3.0, 20.0, -7.0));
            camera.look(dx, dy);
            let start = camera.position;
            camera.fly(Vec3::Z, distance);
            camera.fly(Vec3::NEG_Z, distance);
            prop_assert!((camera.position - start).length() < 1e-3);
        }
    }
}
