//! Turns a laid-out panel into coloured quads the renderer can draw over the scene.

use crate::crafting::registry::RecipeId;
use crate::rendering::vertex::GpuVertex;
use crate::ui::font::{GLYPH_WIDTH, glyph};
use crate::ui::layout::{Label, Panel, Rect};

const BACKDROP: [f32; 3] = [0.10, 0.10, 0.13];
const ROW_READY: [f32; 3] = [0.18, 0.42, 0.24];
const ROW_READY_HOVER: [f32; 3] = [0.26, 0.60, 0.34];
const ROW_BLOCKED: [f32; 3] = [0.22, 0.22, 0.26];
const ROW_BLOCKED_HOVER: [f32; 3] = [0.30, 0.30, 0.35];
const TEXT: [f32; 3] = [0.95, 0.95, 0.95];
const TEXT_BLOCKED: [f32; 3] = [0.55, 0.55, 0.60];

/// Quads collected in screen-space vertices, ready to upload.
struct Quads {
    size: (f32, f32),
    vertices: Vec<GpuVertex>,
    indices: Vec<u32>,
}

impl Quads {
    /// Adds a filled rectangle given in window pixels.
    fn rect(&mut self, rect: Rect, color: [f32; 3]) {
        let (width, height) = self.size;
        let left = rect.x / width * 2.0 - 1.0;
        let right = (rect.x + rect.width) / width * 2.0 - 1.0;
        let top = 1.0 - rect.y / height * 2.0;
        let bottom = 1.0 - (rect.y + rect.height) / height * 2.0;
        let first = self.vertices.len() as u32;
        self.vertices.extend(
            [(left, top), (right, top), (right, bottom), (left, bottom)].map(|(x, y)| GpuVertex {
                position: [x, y, 0.0],
                color,
            }),
        );
        self.indices
            .extend([0, 1, 2, 0, 2, 3].map(|offset| first + offset));
    }

    /// Adds `label` as one square per lit font pixel. Characters without a glyph leave a gap.
    fn text(&mut self, label: &Label, scale: u32, color: [f32; 3]) {
        let unit = scale as f32;
        for (index, c) in label.text.chars().enumerate() {
            let Some(rows) = glyph(c) else { continue };
            let left = label.x + index as f32 * (GLYPH_WIDTH + 1) as f32 * unit;
            for (row, bits) in rows.iter().enumerate() {
                for column in 0..GLYPH_WIDTH {
                    if bits & (1 << (GLYPH_WIDTH - 1 - column)) != 0 {
                        self.rect(
                            Rect {
                                x: left + column as f32 * unit,
                                y: label.y + row as f32 * unit,
                                width: unit,
                                height: unit,
                            },
                            color,
                        );
                    }
                }
            }
        }
    }
}

/// Vertices and indices for `panel` in a window of `size` pixels, positions already in
/// clip space. `hovered` is the recipe under the pointer, drawn brighter.
pub fn overlay(
    panel: &Panel,
    size: (u32, u32),
    hovered: Option<RecipeId>,
) -> (Vec<GpuVertex>, Vec<u32>) {
    let mut quads = Quads {
        size: (size.0.max(1) as f32, size.1.max(1) as f32),
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    quads.rect(panel.rect, BACKDROP);
    quads.text(&panel.title, panel.scale, TEXT);
    for row in &panel.rows {
        let background = match (row.affordable, hovered == Some(row.recipe)) {
            (true, false) => ROW_READY,
            (true, true) => ROW_READY_HOVER,
            (false, false) => ROW_BLOCKED,
            (false, true) => ROW_BLOCKED_HOVER,
        };
        quads.rect(row.rect, background);
        let color = if row.affordable { TEXT } else { TEXT_BLOCKED };
        quads.text(&row.label, panel.scale, color);
    }
    quads.text(&panel.hint, panel.scale, TEXT_BLOCKED);
    (quads.vertices, quads.indices)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::model::Inventory;
    use crate::ui::layout::panel;
    use proptest::prelude::*;

    #[test]
    fn a_rectangle_maps_the_window_corners_to_clip_space() {
        let mut quads = Quads {
            size: (200.0, 100.0),
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        quads.rect(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
            [1.0, 0.0, 0.0],
        );
        let corners: Vec<_> = quads
            .vertices
            .iter()
            .map(|v| (v.position[0], v.position[1]))
            .collect();
        assert_eq!(
            corners,
            [(-1.0, 1.0), (1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)]
        );
        assert_eq!(quads.indices, [0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn a_label_draws_one_square_per_lit_pixel_and_skips_unknown_characters() {
        let mut quads = Quads {
            size: (100.0, 100.0),
            vertices: Vec::new(),
            indices: Vec::new(),
        };
        let lit =
            |c: char| glyph(c).map_or(0, |rows| rows.iter().map(|r| r.count_ones() as usize).sum());
        let label = Label {
            x: 0.0,
            y: 0.0,
            text: "a\u{e9}1".to_string(),
        };
        quads.text(&label, 2, TEXT);
        assert_eq!(quads.vertices.len(), 4 * (lit('a') + lit('1')));
    }

    proptest! {
        // Indices stay inside the vertex list, every vertex lands on screen, and colours are
        // displayable, wherever the pointer is.
        #[test]
        fn the_overlay_is_well_formed(
            width in 320u32..4096,
            height in 240u32..2160,
            hover in prop::option::of(0u8..4),
        ) {
            let panel = panel((width, height), &Inventory::default());
            let (vertices, indices) = overlay(&panel, (width, height), hover.map(RecipeId));
            prop_assert!(!indices.is_empty() && indices.len() % 3 == 0);
            prop_assert!(indices.iter().all(|&i| (i as usize) < vertices.len()));
            for vertex in &vertices {
                prop_assert!(vertex.position[..2].iter().all(|c| (-1.0..=1.0).contains(c)));
                prop_assert!(vertex.color.iter().all(|c| (0.0..=1.0).contains(c)));
            }
        }
    }
}
