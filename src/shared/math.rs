//! Small generic math helpers.

use glam::Vec3;

/// Distance along a ray to the first point inside the box from `min` to `max`, or `None` if
/// the ray misses it or the box lies behind. A ray that starts inside the box hits at 0.
///
/// `direction` need not be normalized, but the distance is in units of its length. A
/// non-finite input is a miss.
pub fn ray_box(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    if !(origin.is_finite() && direction.is_finite() && min.is_finite() && max.is_finite()) {
        return None;
    }
    let (mut near, mut far) = (0.0_f32, f32::INFINITY);
    for axis in 0..3 {
        if direction[axis] == 0.0 {
            // Parallel to this slab: a hit needs the origin between its planes.
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let (a, b) = (
            (min[axis] - origin[axis]) / direction[axis],
            (max[axis] - origin[axis]) / direction[axis],
        );
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far).then_some(near)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Vec3 = Vec3::new(1.0, 0.0, -1.0);
    const MAX: Vec3 = Vec3::new(2.0, 2.0, 1.0);

    #[test]
    fn a_ray_hits_the_near_face() {
        assert_eq!(
            ray_box(Vec3::new(-3.0, 1.0, 0.0), Vec3::X, MIN, MAX),
            Some(4.0)
        );
    }

    #[test]
    fn a_ray_inside_hits_at_zero_and_one_behind_or_beside_misses() {
        assert_eq!(
            ray_box(Vec3::new(1.5, 1.0, 0.0), Vec3::X, MIN, MAX),
            Some(0.0)
        );
        assert_eq!(ray_box(Vec3::new(5.0, 1.0, 0.0), Vec3::X, MIN, MAX), None);
        assert_eq!(ray_box(Vec3::new(-3.0, 5.0, 0.0), Vec3::X, MIN, MAX), None);
    }

    #[test]
    fn a_non_finite_input_is_a_miss() {
        assert_eq!(ray_box(Vec3::NAN, Vec3::X, MIN, MAX), None);
        assert_eq!(ray_box(Vec3::ZERO, Vec3::NAN, MIN, MAX), None);
        assert_eq!(ray_box(Vec3::ZERO, Vec3::ZERO, MIN, MAX), None);
    }
}
