//! A dimension: a sparse map of tiles with copy-on-write undo.
//!
//! Tiles are `Arc`s. Taking an undo snapshot only clones the pointer; the first edit after that
//! clones the tile data (`Arc::make_mut`). The exporter and renderers also take cheap snapshots
//! of the whole dimension and work on them in the background while editing continues.

use std::sync::Arc;

use meadow_blocks::{BuildLimits, StateId};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};

use crate::layer::LayerId;
use crate::terrain::{Material, TerrainId};
use crate::tile::{TILE_MASK, TILE_SHIFT, TILE_SIZE, Tile, local_index};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DimensionKind {
    Overworld,
    Nether,
    End,
}

impl DimensionKind {
    pub fn index(self) -> i32 {
        match self {
            DimensionKind::Overworld => 0,
            DimensionKind::Nether => 1,
            DimensionKind::End => 2,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            DimensionKind::Overworld => "Surface",
            DimensionKind::Nether => "Nether",
            DimensionKind::End => "End",
        }
    }
    pub fn from_index(i: i32) -> Option<Self> {
        match i {
            0 => Some(DimensionKind::Overworld),
            1 => Some(DimensionKind::Nether),
            2 => Some(DimensionKind::End),
            _ => None,
        }
    }
}

/// What is generated below the terrain strata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subsurface {
    pub material: Material,
    /// Use deepslate below y=0 (with a vanilla-like blend between y=0 and y=8).
    pub deepslate: bool,
    pub bedrock_floor: bool,
}

impl Default for Subsurface {
    fn default() -> Self {
        Subsurface { material: Material::Block(StateId::parse("minecraft:stone")), deepslate: true, bedrock_floor: true }
    }
}

type Snapshot = Vec<((i32, i32), Option<Arc<Tile>>)>;

#[derive(Default, Clone)]
struct UndoState {
    recording: bool,
    pending: FxHashMap<(i32, i32), Option<Arc<Tile>>>,
    undo: Vec<(String, Snapshot)>,
    redo: Vec<(String, Snapshot)>,
    pending_name: String,
}

const MAX_UNDO: usize = 64;

#[derive(Clone, Serialize, Deserialize)]
pub struct Dimension {
    pub kind: DimensionKind,
    pub limits: BuildLimits,
    pub default_water_level: i32,
    pub default_terrain: TerrainId,
    pub subsurface: Subsurface,
    /// Biome used outside painted areas and as automatic fallback.
    pub default_biome: u8,
    #[serde(with = "tile_map")]
    tiles: FxHashMap<(i32, i32), Arc<Tile>>,
    #[serde(skip)]
    undo: UndoState,
    #[serde(skip)]
    rev: u64,
}

mod tile_map {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(m: &FxHashMap<(i32, i32), Arc<Tile>>, s: S) -> Result<S::Ok, S::Error> {
        let mut v: Vec<&Arc<Tile>> = m.values().collect();
        v.sort_by_key(|t| (t.ty, t.tx));
        s.collect_seq(v)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<FxHashMap<(i32, i32), Arc<Tile>>, D::Error> {
        let v: Vec<Tile> = Vec::deserialize(d)?;
        Ok(v.into_iter().map(|t| ((t.tx, t.ty), Arc::new(t))).collect())
    }
}

impl Dimension {
    pub fn new(kind: DimensionKind, limits: BuildLimits) -> Self {
        Dimension {
            kind,
            limits,
            default_water_level: 62,
            default_terrain: 0,
            subsurface: Subsurface::default(),
            default_biome: match kind {
                DimensionKind::Overworld => 1,
                DimensionKind::Nether => 8,
                DimensionKind::End => 9,
            },
            tiles: FxHashMap::default(),
            undo: UndoState::default(),
            rev: 1,
        }
    }

    /// The theme used for automatic terrain (WorldPainter's default theme for this dimension).
    pub fn theme(&self) -> crate::theme::Theme {
        crate::theme::Theme::default_for(self.default_terrain, self.limits.min_y, self.limits.max_y, self.default_water_level)
    }

    // ---- tiles ------------------------------------------------------------------------------

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }
    pub fn tile(&self, tx: i32, ty: i32) -> Option<&Arc<Tile>> {
        self.tiles.get(&(tx, ty))
    }
    pub fn tiles(&self) -> impl Iterator<Item = &Arc<Tile>> {
        self.tiles.values()
    }
    pub fn tile_coords(&self) -> Vec<(i32, i32)> {
        let mut v: Vec<(i32, i32)> = self.tiles.keys().copied().collect();
        v.sort_by_key(|(x, y)| (*y, *x));
        v
    }

    fn record(&mut self, key: (i32, i32)) {
        if self.undo.recording && !self.undo.pending.contains_key(&key) {
            let old = self.tiles.get(&key).cloned();
            self.undo.pending.insert(key, old);
        }
    }

    /// Mutable access to a tile (copy-on-write; recorded for undo).
    pub fn tile_mut(&mut self, tx: i32, ty: i32) -> Option<&mut Tile> {
        if !self.tiles.contains_key(&(tx, ty)) {
            return None;
        }
        self.record((tx, ty));
        self.rev += 1;
        let t = Arc::make_mut(self.tiles.get_mut(&(tx, ty)).unwrap());
        t.touch();
        Some(t)
    }

    pub fn add_tile(&mut self, tile: Tile) {
        let key = (tile.tx, tile.ty);
        self.record(key);
        self.rev += 1;
        self.tiles.insert(key, Arc::new(tile));
    }

    /// Create a tile filled with defaults if it does not exist.
    pub fn ensure_tile(&mut self, tx: i32, ty: i32, height: f32) {
        if !self.tiles.contains_key(&(tx, ty)) {
            let t = Tile::new(tx, ty, height, self.default_terrain, self.default_water_level as i16);
            self.add_tile(t);
        }
    }

    pub fn remove_tile(&mut self, tx: i32, ty: i32) -> bool {
        if self.tiles.contains_key(&(tx, ty)) {
            self.record((tx, ty));
            self.rev += 1;
            self.tiles.remove(&(tx, ty));
            true
        } else {
            false
        }
    }

    /// Changes on every mutation; used to invalidate whole-dimension caches.
    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// Tile bounds (inclusive) or `None` if empty.
    pub fn tile_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let mut it = self.tiles.keys();
        let &(x, y) = it.next()?;
        let mut b = (x, y, x, y);
        for &(x, y) in it {
            b.0 = b.0.min(x);
            b.1 = b.1.min(y);
            b.2 = b.2.max(x);
            b.3 = b.3.max(y);
        }
        Some(b)
    }

    /// Block bounds (inclusive min, exclusive max) or `None` if empty.
    pub fn block_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        self.tile_bounds()
            .map(|(a, b, c, d)| (a << TILE_SHIFT, b << TILE_SHIFT, (c + 1) << TILE_SHIFT, (d + 1) << TILE_SHIFT))
    }

    // ---- per-column access ------------------------------------------------------------------

    #[inline]
    fn split(x: i32, y: i32) -> ((i32, i32), usize) {
        ((x >> TILE_SHIFT, y >> TILE_SHIFT), local_index(x & TILE_MASK, y & TILE_MASK))
    }

    #[inline]
    pub fn height(&self, x: i32, y: i32) -> Option<f32> {
        let (k, i) = Self::split(x, y);
        self.tiles.get(&k).map(|t| t.height[i])
    }
    /// Height, or the dimension minimum outside tiles.
    #[inline]
    pub fn height_or_min(&self, x: i32, y: i32) -> f32 {
        self.height(x, y).unwrap_or(self.limits.min_y as f32)
    }
    #[inline]
    pub fn int_height(&self, x: i32, y: i32) -> Option<i32> {
        self.height(x, y).map(|h| (h + 0.5).floor() as i32)
    }
    pub fn set_height(&mut self, x: i32, y: i32, h: f32) {
        let (k, i) = Self::split(x, y);
        let (lo, hi) = (self.limits.min_y as f32, (self.limits.max_y - 1) as f32);
        if let Some(t) = self.tile_mut(k.0, k.1) {
            t.height[i] = h.clamp(lo, hi);
        }
    }
    #[inline]
    pub fn terrain(&self, x: i32, y: i32) -> Option<TerrainId> {
        let (k, i) = Self::split(x, y);
        self.tiles.get(&k).map(|t| t.terrain[i])
    }
    pub fn set_terrain(&mut self, x: i32, y: i32, terrain: TerrainId) {
        let (k, i) = Self::split(x, y);
        if let Some(t) = self.tile_mut(k.0, k.1) {
            t.terrain[i] = terrain;
        }
    }
    #[inline]
    pub fn water(&self, x: i32, y: i32) -> Option<i32> {
        let (k, i) = Self::split(x, y);
        self.tiles.get(&k).map(|t| t.water[i] as i32)
    }
    pub fn set_water(&mut self, x: i32, y: i32, level: i32) {
        let (k, i) = Self::split(x, y);
        if let Some(t) = self.tile_mut(k.0, k.1) {
            t.water[i] = level.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }
    }
    #[inline]
    pub fn layer_value(&self, layer: LayerId, default: u8, x: i32, y: i32) -> u8 {
        let (k, i) = Self::split(x, y);
        self.tiles.get(&k).map(|t| t.layer_value(layer, i, default)).unwrap_or(default)
    }
    pub fn set_layer_value(&mut self, layer: LayerId, bits: bool, default: u8, x: i32, y: i32, v: u8) {
        let (k, i) = Self::split(x, y);
        if let Some(t) = self.tile_mut(k.0, k.1) {
            t.layer_mut(layer, bits, default).set(i, v);
        }
    }
    pub fn is_underwater(&self, x: i32, y: i32) -> bool {
        match (self.water(x, y), self.int_height(x, y)) {
            (Some(w), Some(h)) => w > h,
            _ => false,
        }
    }

    /// Slope in degrees (steepest neighbour difference).
    pub fn slope(&self, x: i32, y: i32) -> f32 {
        let Some(h) = self.height(x, y) else { return 0.0 };
        let mut d: f32 = 0.0;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            if let Some(n) = self.height(x + dx, y + dy) {
                d = d.max((n - h).abs());
            }
        }
        d.atan().to_degrees()
    }

    /// Bilinear height sample at fractional coordinates.
    pub fn height_bilinear(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor() as i32, y.floor() as i32);
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let h = |a, b| self.height_or_min(a, b);
        let top = h(x0, y0) * (1.0 - fx) + h(x0 + 1, y0) * fx;
        let bot = h(x0, y0 + 1) * (1.0 - fx) + h(x0 + 1, y0 + 1) * fx;
        top * (1.0 - fy) + bot * fy
    }

    /// Call `f` for each existing tile overlapping the block rectangle [x0,x1) x [y0,y1), with
    /// the local index range to visit. Tiles are mutated copy-on-write.
    pub fn for_each_in_rect_mut(
        &mut self,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        mut f: impl FnMut(&mut Tile, i32, i32, usize),
    ) {
        let (tx0, ty0, tx1, ty1) = (x0 >> TILE_SHIFT, y0 >> TILE_SHIFT, (x1 - 1) >> TILE_SHIFT, (y1 - 1) >> TILE_SHIFT);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if !self.tiles.contains_key(&(tx, ty)) {
                    continue;
                }
                let ox = tx << TILE_SHIFT;
                let oy = ty << TILE_SHIFT;
                let lx0 = (x0 - ox).max(0);
                let ly0 = (y0 - oy).max(0);
                let lx1 = (x1 - ox).min(TILE_SIZE);
                let ly1 = (y1 - oy).min(TILE_SIZE);
                let t = self.tile_mut(tx, ty).unwrap();
                for ly in ly0..ly1 {
                    for lx in lx0..lx1 {
                        f(t, ox + lx, oy + ly, local_index(lx, ly));
                    }
                }
            }
        }
    }

    // ---- undo -------------------------------------------------------------------------------

    /// Start recording an undoable operation.
    pub fn begin_op(&mut self, name: &str) {
        if self.undo.recording {
            self.end_op();
        }
        self.undo.recording = true;
        self.undo.pending.clear();
        self.undo.pending_name = name.to_owned();
    }

    /// Finish the current operation and push it onto the undo stack.
    pub fn end_op(&mut self) {
        if !self.undo.recording {
            return;
        }
        self.undo.recording = false;
        if self.undo.pending.is_empty() {
            return;
        }
        let snap: Snapshot = self.undo.pending.drain().collect();
        let name = std::mem::take(&mut self.undo.pending_name);
        self.undo.undo.push((name, snap));
        if self.undo.undo.len() > MAX_UNDO {
            self.undo.undo.remove(0);
        }
        self.undo.redo.clear();
    }

    pub fn is_recording(&self) -> bool {
        self.undo.recording
    }

    fn restore(&mut self, snap: Snapshot) -> Snapshot {
        let mut inverse = Vec::with_capacity(snap.len());
        for (k, t) in snap {
            inverse.push((k, self.tiles.get(&k).cloned()));
            match t {
                Some(mut t) => {
                    Arc::make_mut(&mut t).touch();
                    self.tiles.insert(k, t);
                }
                None => {
                    self.tiles.remove(&k);
                }
            }
        }
        self.rev += 1;
        inverse
    }

    pub fn undo(&mut self) -> Option<String> {
        self.end_op();
        let (name, snap) = self.undo.undo.pop()?;
        let inv = self.restore(snap);
        self.undo.redo.push((name.clone(), inv));
        Some(name)
    }

    pub fn redo(&mut self) -> Option<String> {
        self.end_op();
        let (name, snap) = self.undo.redo.pop()?;
        let inv = self.restore(snap);
        self.undo.undo.push((name.clone(), inv));
        Some(name)
    }

    pub fn undo_name(&self) -> Option<&str> {
        self.undo.undo.last().map(|(n, _)| n.as_str())
    }
    pub fn redo_name(&self) -> Option<&str> {
        self.undo.redo.last().map(|(n, _)| n.as_str())
    }
    pub fn clear_history(&mut self) {
        self.undo = UndoState::default();
    }

    /// A cheap read-only copy (shares tile data) without undo history, for background work.
    pub fn snapshot(&self) -> Dimension {
        let mut d = self.clone();
        d.undo = UndoState::default();
        d
    }

    /// Remove layer data that is entirely the default value in all tiles.
    pub fn compact(&mut self, default_of: impl Fn(LayerId) -> u8 + Copy) {
        let keys: Vec<(i32, i32)> = self.tiles.keys().copied().collect();
        for k in keys {
            let needs = self.tiles[&k].layers.iter().any(|(id, d)| d.is_uniform(default_of(*id)));
            if needs {
                Arc::make_mut(self.tiles.get_mut(&k).unwrap()).compact_layers(default_of);
            }
        }
    }

    /// Set of tiles that contain non-default data for `layer`.
    pub fn tiles_with_layer(&self, layer: LayerId) -> FxHashSet<(i32, i32)> {
        self.tiles.iter().filter(|(_, t)| t.has_layer(layer)).map(|(k, _)| *k).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cow_undo_redo() {
        let mut d = Dimension::new(DimensionKind::Overworld, BuildLimits::VANILLA);
        d.ensure_tile(0, 0, 64.0);
        d.clear_history();
        d.begin_op("raise");
        d.set_height(5, 5, 80.0);
        d.end_op();
        assert_eq!(d.height(5, 5), Some(80.0));
        let snap = d.snapshot();
        d.undo();
        assert_eq!(d.height(5, 5), Some(64.0));
        assert_eq!(snap.height(5, 5), Some(80.0), "snapshot unaffected by undo");
        d.redo();
        assert_eq!(d.height(5, 5), Some(80.0));
        d.begin_op("add");
        d.ensure_tile(1, 0, 10.0);
        d.end_op();
        assert!(d.tile(1, 0).is_some());
        d.undo();
        assert!(d.tile(1, 0).is_none());
        assert_eq!(d.height(-1, 0), None);
    }

    #[test]
    fn layers_and_slope() {
        let mut d = Dimension::new(DimensionKind::Overworld, BuildLimits::VANILLA);
        d.ensure_tile(-1, -1, 64.0);
        d.set_layer_value(LayerId::FROST, true, 0, -3, -3, 1);
        assert_eq!(d.layer_value(LayerId::FROST, 0, -3, -3), 1);
        assert_eq!(d.layer_value(LayerId::FROST, 0, -4, -3), 0);
        d.set_height(-10, -10, 65.0);
        assert!((d.slope(-11, -10) - 45.0).abs() < 0.01);
    }
}
