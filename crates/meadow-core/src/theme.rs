//! Themes: terrain (and frost) chosen automatically by height, like WorldPainter's "simple
//! theme". Used by right-click with a terrain paint, "apply theme" and resets.

use crate::layer::LayerId;
use crate::terrain::{TerrainId, ids};
use crate::tile::Tile;

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// (from height, terrain), sorted by height. The first entry covers everything below.
    pub ranges: Vec<(i32, TerrainId)>,
    /// Beaches from water - 2 to water + 1.
    pub beaches: bool,
    pub water: i32,
    /// Frost from this height upward.
    pub frost_from: Option<i32>,
}

impl Theme {
    /// WorldPainter's default theme: the surface material low down, then permadirt, stone mix
    /// and deep snow higher up, frost on the peaks, beaches along the water line.
    pub fn default_for(top: TerrainId, min_y: i32, max_y: i32, water: i32) -> Theme {
        let f = max_y.min(320) as f32 / 128.0;
        let at = |k: f32| (k * f) as i32 + water;
        Theme {
            ranges: vec![(min_y - 1, top), (at(32.0), ids::PERMADIRT), (at(48.0), ids::STONE_MIX), (at(80.0), ids::DEEP_SNOW)],
            beaches: true,
            water,
            frost_from: Some(at(64.0)),
        }
    }

    pub fn terrain_at(&self, height: i32) -> TerrainId {
        if self.beaches && height >= self.water - 2 && height <= self.water + 1 {
            return ids::BEACHES;
        }
        let mut t = self.ranges.first().map(|r| r.1).unwrap_or(ids::GRASS);
        for &(from, terrain) in &self.ranges {
            if height >= from {
                t = terrain;
            } else {
                break;
            }
        }
        t
    }

    pub fn frost_at(&self, height: i32) -> bool {
        self.frost_from.is_some_and(|f| height >= f)
    }

    /// Re-terrain one column of a tile from its height.
    pub fn apply(&self, t: &mut Tile, i: usize) {
        let h = (t.height[i] + 0.5).floor() as i32;
        t.terrain[i] = self.terrain_at(h);
        let frost = self.frost_at(h);
        if frost || t.has_layer(LayerId::FROST) {
            t.layer_mut(LayerId::FROST, true, 0).set(i, frost as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_matches_worldpainter() {
        // Vanilla: factor 2.5, water 62
        let t = Theme::default_for(ids::GRASS, -64, 320, 62);
        assert_eq!(t.terrain_at(30), ids::GRASS);
        assert_eq!(t.terrain_at(60), ids::BEACHES);
        assert_eq!(t.terrain_at(63), ids::BEACHES);
        assert_eq!(t.terrain_at(64), ids::GRASS);
        assert_eq!(t.terrain_at(142), ids::PERMADIRT);
        assert_eq!(t.terrain_at(182), ids::STONE_MIX);
        assert_eq!(t.terrain_at(262), ids::DEEP_SNOW);
        assert!(!t.frost_at(221));
        assert!(t.frost_at(222));
    }
}
