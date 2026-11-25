//! Brushes and editing tools.
//!
//! Every tool works in two phases: a read phase computes per-column strengths (brush falloff x
//! mask) and any neighbourhood data (smoothing), then a write phase mutates tiles. That keeps
//! masks and smoothing correct while tiles are being modified copy-on-write.

use std::collections::VecDeque;

use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};

use crate::dimension::Dimension;
use crate::layer::LayerId;
use crate::mask::CustomMask;
use crate::noise::{fbm2, hash2, hash3, unit};
use crate::terrain::TerrainId;
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrushShape {
    Circle,
    Square,
}

/// Brush falloff profiles, as in WorldPainter's brush palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Falloff {
    Constant,
    Linear,
    /// WorldPainter's default ("Sine"): cos(d*pi)/2 + 0.5.
    #[serde(alias = "Smooth")]
    Sine,
    Plateau,
    #[serde(alias = "Sharp")]
    Spike,
    Dome,
}

impl Falloff {
    pub const ALL: [Falloff; 6] = [Falloff::Spike, Falloff::Linear, Falloff::Sine, Falloff::Plateau, Falloff::Dome, Falloff::Constant];

    pub fn name(self) -> &'static str {
        match self {
            Falloff::Constant => "Constant",
            Falloff::Linear => "Linear",
            Falloff::Sine => "Sine",
            Falloff::Plateau => "Plateau",
            Falloff::Spike => "Spike",
            Falloff::Dome => "Dome",
        }
    }

    /// Strength at normalised distance `d` (0 = centre, 1 = edge).
    #[inline]
    pub fn at(self, d: f32) -> f32 {
        use std::f32::consts::PI;
        match self {
            Falloff::Constant => 1.0,
            Falloff::Linear => 1.0 - d,
            Falloff::Sine => (d * PI).cos() / 2.0 + 0.5,
            Falloff::Plateau => {
                if d < 0.5 {
                    1.0
                } else {
                    ((d - 0.5) * 2.0 * PI).cos() / 2.0 + 0.5
                }
            }
            Falloff::Spike => (1.0 - d) * (1.0 - d),
            Falloff::Dome => (1.0 - d * d).max(0.0).sqrt(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Brush {
    pub radius: f32,
    /// 0..1
    pub intensity: f32,
    pub shape: BrushShape,
    pub falloff: Falloff,
    /// 0 = smooth edges, 1 = very noisy edges/strength.
    pub roughness: f32,
    /// Rotation in degrees (affects square brushes).
    #[serde(default)]
    pub rotation: f32,
}

impl Default for Brush {
    fn default() -> Self {
        Brush { radius: 50.0, intensity: 0.5, shape: BrushShape::Circle, falloff: Falloff::Sine, roughness: 0.0, rotation: 0.0 }
    }
}

impl Brush {
    /// Strength in [0, 1] at offset (dx, dy) from the centre, including intensity.
    #[inline]
    pub fn strength(&self, dx: f32, dy: f32, wx: i32, wy: i32) -> f32 {
        self.falloff_strength(dx, dy, wx, wy) * self.intensity
    }

    /// Strength without intensity ("full strength" in WorldPainter terms).
    #[inline]
    pub fn falloff_strength(&self, dx: f32, dy: f32, wx: i32, wy: i32) -> f32 {
        let r = self.radius.max(0.5);
        let (dx, dy) = if self.rotation != 0.0 && self.shape == BrushShape::Square {
            let (s, c) = (-self.rotation.to_radians()).sin_cos();
            (dx * c - dy * s, dx * s + dy * c)
        } else {
            (dx, dy)
        };
        let d = match self.shape {
            BrushShape::Circle => (dx * dx + dy * dy).sqrt() / r,
            BrushShape::Square => dx.abs().max(dy.abs()) / r,
        };
        if d > 1.0 {
            return 0.0;
        }
        let f = self.falloff.at(d);
        if self.roughness > 0.0 {
            let n = (fbm2(0x9e37, wx as f32 / 8.0, wy as f32 / 8.0, 3) + 1.0) * 0.5;
            (f * (1.0 - self.roughness + self.roughness * n * 2.0)).clamp(0.0, 1.0)
        } else {
            f
        }
    }
}

/// Something that can be painted with Spray Paint, Pencil or Fill (WorldPainter's "paint").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Paint {
    Terrain(TerrainId),
    /// On/off or 0..15 layer.
    Layer { layer: LayerId, bits: bool, default: u8 },
    /// A layer whose values are categories (biomes, annotation colours).
    Discrete { layer: LayerId, value: u8, default: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaintMode {
    /// Soft, random application weighted by brush strength (WorldPainter's Spray Paint).
    Spray,
    /// Hard-edged application (WorldPainter's Pencil).
    Pencil,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Tool {
    Raise,
    Lower,
    Smooth,
    /// Flatten towards the height under the cursor when the stroke started.
    Flatten,
    /// Move heights towards a fixed level.
    SetHeight(f32),
    /// Add noise (roughen).
    Roughen,
    /// Erode-like terracing into steps of the given height.
    Terrace(f32),
    PaintTerrain(TerrainId),
    PaintLayer { layer: LayerId, value: u8, bits: bool, default: u8 },
    EraseLayer { layer: LayerId, bits: bool, default: u8 },
    PaintBiome(u8),
    SetWaterLevel(i32),
    /// Remove water (set water level below the surface).
    Sponge,
    PaintMask(LayerId),
    EraseMask(LayerId),
    /// Apply (or with `remove`, take away) a paint.
    Paint { paint: Paint, mode: PaintMode, remove: bool },
    /// Reset terrain (and frost) to the dimension's theme.
    ResetTerrain,
    /// Reset water to the given level and remove lava (Sponge's inverse).
    ResetWater(i32),
}

impl Tool {
    pub fn name(&self) -> &'static str {
        match self {
            Tool::Raise => "Raise",
            Tool::Lower => "Lower",
            Tool::Smooth => "Smooth",
            Tool::Flatten => "Flatten",
            Tool::SetHeight(_) => "Set height",
            Tool::Roughen => "Roughen",
            Tool::Terrace(_) => "Terrace",
            Tool::PaintTerrain(_) => "Paint terrain",
            Tool::PaintLayer { .. } => "Paint layer",
            Tool::EraseLayer { .. } => "Erase layer",
            Tool::PaintBiome(_) => "Paint biome",
            Tool::SetWaterLevel(_) => "Water level",
            Tool::Sponge => "Sponge",
            Tool::PaintMask(_) => "Paint mask",
            Tool::EraseMask(_) => "Erase mask",
            Tool::Paint { mode: PaintMode::Spray, .. } => "Spray paint",
            Tool::Paint { mode: PaintMode::Pencil, .. } => "Pencil",
            Tool::ResetTerrain => "Reset terrain",
            Tool::ResetWater(_) => "Reset water",
        }
    }
    /// Continuous tools apply every frame while the mouse is held; others once per column.
    pub fn is_continuous(&self) -> bool {
        matches!(self, Tool::Raise | Tool::Lower | Tool::Smooth | Tool::Flatten | Tool::SetHeight(_) | Tool::Roughen | Tool::Terrace(_))
    }
}

/// Per-stroke state (e.g. the flatten target height).
#[derive(Debug, Clone, Default)]
pub struct Stroke {
    pub flatten_level: Option<f32>,
    /// Incremented on every application, so spray paint keeps adding coverage while held.
    pub tick: u64,
}

/// Apply a tool at (cx, cy). `dt` is the time step in seconds for continuous tools (use 1.0 for
/// single applications such as scripts).
pub fn apply(dim: &mut Dimension, tool: &Tool, brush: &Brush, cx: f32, cy: f32, dt: f32, mask: Option<&CustomMask>, stroke: &mut Stroke) {
    let r = brush.radius.ceil() as i32 + 1;
    let (x0, y0) = (cx.floor() as i32 - r, cy.floor() as i32 - r);
    let (x1, y1) = (cx.floor() as i32 + r + 1, cy.floor() as i32 + r + 1);
    let w = (x1 - x0) as usize;
    let h = (y1 - y0) as usize;

    // Read phase: strengths and auxiliary values.
    let mut strength = vec![0f32; w * h];
    let mut any = false;
    for yy in 0..h {
        for xx in 0..w {
            let (wx, wy) = (x0 + xx as i32, y0 + yy as i32);
            let s = brush.strength(wx as f32 + 0.5 - cx, wy as f32 + 0.5 - cy, wx, wy);
            if s <= 0.0 {
                continue;
            }
            let m = mask.map(|m| m.eval(dim, wx, wy)).unwrap_or(1.0);
            let v = s * m;
            if v > 0.0 {
                strength[yy * w + xx] = v;
                any = true;
            }
        }
    }
    if !any {
        return;
    }
    let smooth_src: Option<Vec<f32>> = matches!(tool, Tool::Smooth).then(|| {
        let mut out = vec![0f32; w * h];
        for yy in 0..h {
            for xx in 0..w {
                if strength[yy * w + xx] <= 0.0 {
                    continue;
                }
                let (wx, wy) = (x0 + xx as i32, y0 + yy as i32);
                let mut sum = 0.0;
                let mut n = 0.0;
                for dy in -2..=2 {
                    for dx in -2..=2 {
                        if let Some(hh) = dim.height(wx + dx, wy + dy) {
                            let wgt = 1.0 / (1.0 + (dx * dx + dy * dy) as f32);
                            sum += hh * wgt;
                            n += wgt;
                        }
                    }
                }
                out[yy * w + xx] = if n > 0.0 { sum / n } else { dim.height_or_min(wx, wy) };
            }
        }
        out
    });
    if matches!(tool, Tool::Flatten) && stroke.flatten_level.is_none() {
        stroke.flatten_level = dim.height(cx.floor() as i32, cy.floor() as i32);
    }
    let (min_y, max_y) = (dim.limits.min_y as f32, (dim.limits.max_y - 1) as f32);
    let rate = 20.0 * dt; // blocks per second at full strength
    stroke.tick = stroke.tick.wrapping_add(1);
    let tick = stroke.tick;
    let theme = dim.theme();
    let intensity = brush.intensity.max(0.01);

    // Write phase.
    dim.for_each_in_rect_mut(x0, y0, x1, y1, |t, wx, wy, i| {
        let k = ((wy - y0) as usize) * w + (wx - x0) as usize;
        let s = strength[k];
        if s <= 0.0 {
            return;
        }
        match tool {
            Tool::Raise => t.height[i] = (t.height[i] + s * rate).min(max_y),
            Tool::Lower => t.height[i] = (t.height[i] - s * rate).max(min_y),
            Tool::Smooth => {
                let target = smooth_src.as_ref().unwrap()[k];
                let a = (s * dt * 8.0).min(1.0);
                t.height[i] += (target - t.height[i]) * a;
            }
            Tool::Flatten => {
                if let Some(level) = stroke.flatten_level {
                    let a = (s * dt * 8.0).min(1.0);
                    t.height[i] += (level - t.height[i]) * a;
                }
            }
            Tool::SetHeight(level) => {
                let a = (s * dt * 8.0).min(1.0);
                t.height[i] += (level.clamp(min_y, max_y) - t.height[i]) * a;
            }
            Tool::Roughen => {
                let n = unit(hash2(0xabc, wx, wy)) * 2.0 - 1.0;
                t.height[i] = (t.height[i] + n * s * rate * 0.5).clamp(min_y, max_y);
            }
            Tool::Terrace(step) => {
                let step = step.max(1.0);
                let target = (t.height[i] / step).round() * step;
                let a = (s * dt * 6.0).min(1.0);
                t.height[i] += (target - t.height[i]) * a;
            }
            Tool::PaintTerrain(id) => {
                if s >= 0.5 || unit(hash2(0x7e44, wx, wy)) < s * 2.0 {
                    t.terrain[i] = *id;
                }
            }
            Tool::PaintLayer { layer, value, bits, default } => {
                let d = t.layer_mut(*layer, *bits, *default);
                if *bits {
                    if unit(hash2(0x1a7e, wx, wy)) < s.max(0.02) * 1.5 {
                        d.set(i, 1);
                    }
                } else {
                    let target = (*value as f32 * s.min(1.0) / brush.intensity.max(0.01)).round().min(*value as f32) as u8;
                    if target > d.get(i) || *layer == LayerId::BIOME {
                        d.set(i, if *layer == LayerId::BIOME { *value } else { target.max(1) });
                    }
                }
            }
            Tool::EraseLayer { layer, bits, default } => {
                if t.has_layer(*layer) {
                    let d = t.layer_mut(*layer, *bits, *default);
                    if *bits || *layer == LayerId::BIOME {
                        d.set(i, *default);
                    } else {
                        let cur = d.get(i) as f32;
                        d.set(i, (cur * (1.0 - s.min(1.0) / brush.intensity.max(0.01))).floor().max(0.0) as u8);
                    }
                }
            }
            Tool::PaintBiome(b) => t.layer_mut(LayerId::BIOME, false, meadow_blocks::BIOME_AUTO).set(i, *b),
            Tool::SetWaterLevel(level) => t.water[i] = *level as i16,
            Tool::Sponge => t.water[i] = (dim_min_water(min_y)) as i16,
            Tool::PaintMask(layer) => {
                let d = t.layer_mut(*layer, false, 0);
                let v = (d.get(i) as f32 + s * 255.0 * dt * 4.0).min(255.0);
                d.set(i, v as u8);
            }
            Tool::EraseMask(layer) => {
                let d = t.layer_mut(*layer, false, 0);
                let v = (d.get(i) as f32 - s * 255.0 * dt * 4.0).max(0.0);
                d.set(i, v as u8);
            }
            Tool::ResetTerrain => theme.apply(t, i),
            Tool::ResetWater(level) => {
                t.water[i] = *level as i16;
                if t.has_layer(LayerId::LAVA) {
                    t.layer_mut(LayerId::LAVA, true, 0).set(i, 0);
                }
            }
            Tool::Paint { paint, mode, remove } => {
                // WorldPainter: hit if strength > 0.95 or a random draw < strength (fresh draw
                // every tick, so holding the button keeps adding coverage).
                let hit = match mode {
                    PaintMode::Pencil => true,
                    PaintMode::Spray => s > 0.95 || unit(hash3(0x5b7a ^ tick, wx, wy, 0)) < s,
                };
                apply_paint(t, i, paint, *mode, *remove, s, s / intensity, hit, &theme);
            }
        }
    });
}

/// Apply one paint to one column. `s` includes intensity, `full` does not.
#[allow(clippy::too_many_arguments)]
fn apply_paint(t: &mut crate::tile::Tile, i: usize, paint: &Paint, mode: PaintMode, remove: bool, s: f32, full: f32, hit: bool, theme: &Theme) {
    match *paint {
        Paint::Terrain(id) => {
            if hit {
                if remove {
                    theme.apply(t, i);
                } else {
                    t.terrain[i] = id;
                }
            }
        }
        Paint::Layer { layer, bits: true, default } => {
            if hit && (!remove || t.has_layer(layer)) {
                t.layer_mut(layer, true, default).set(i, (!remove) as u8);
            }
        }
        Paint::Layer { layer, bits: false, default } => {
            let s = if mode == PaintMode::Pencil { s.max(0.0) } else { s };
            if remove {
                if t.has_layer(layer) {
                    let d = t.layer_mut(layer, false, default);
                    let target = (14.0 - (full.min(1.0) * 14.0).round()).max(0.0) as u8;
                    if target < d.get(i) {
                        d.set(i, target);
                    }
                }
            } else {
                let target = (1.0 + (s.min(1.0) * 14.0).round()) as u8;
                let d = t.layer_mut(layer, false, default);
                if target > d.get(i) {
                    d.set(i, target);
                }
            }
        }
        Paint::Discrete { layer, value, default } => {
            if hit {
                if remove {
                    if t.has_layer(layer) {
                        t.layer_mut(layer, false, default).set(i, default);
                    }
                } else {
                    t.layer_mut(layer, false, default).set(i, value);
                }
            }
        }
    }
}

/// The value of a paint's channel at a column (used by Fill to find the region to fill).
fn paint_key(dim: &Dimension, paint: &Paint, x: i32, y: i32) -> Option<u32> {
    let _ = dim.height(x, y)?;
    Some(match *paint {
        Paint::Terrain(_) => dim.terrain(x, y)? as u32,
        Paint::Layer { layer, default, .. } | Paint::Discrete { layer, default, .. } => dim.layer_value(layer, default, x, y) as u32,
    })
}

/// WorldPainter's Fill tool: flood fill the connected area that currently has the same value as
/// the clicked column (same terrain, same layer value, same biome) with the paint, or remove the
/// paint from it. Returns the number of columns changed.
pub fn fill_paint(dim: &mut Dimension, x: i32, y: i32, paint: &Paint, remove: bool, max_columns: usize) -> usize {
    let Some(key) = paint_key(dim, paint, x, y) else { return 0 };
    let mut seen: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut queue = VecDeque::new();
    let mut cols = Vec::new();
    queue.push_back((x, y));
    while let Some((cx, cy)) = queue.pop_front() {
        if cols.len() >= max_columns || !seen.insert((cx, cy)) {
            continue;
        }
        if paint_key(dim, paint, cx, cy) != Some(key) {
            continue;
        }
        cols.push((cx, cy));
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            queue.push_back((cx + dx, cy + dy));
        }
    }
    let theme = dim.theme();
    for &(cx, cy) in &cols {
        let (tx, ty) = (cx >> crate::tile::TILE_SHIFT, cy >> crate::tile::TILE_SHIFT);
        let i = crate::tile::local_index(cx & crate::tile::TILE_MASK, cy & crate::tile::TILE_MASK);
        if let Some(t) = dim.tile_mut(tx, ty) {
            apply_paint(t, i, paint, PaintMode::Pencil, remove, 1.0, 1.0, true, &theme);
        }
    }
    cols.len()
}

/// WorldPainter's Flood tool: raise (or with `lower`, drop) the water level by one block over
/// the connected area around (x, y). With `lava`, floods with lava (or converts water to lava).
/// Returns the number of columns changed.
pub fn flood_step(dim: &mut Dimension, x: i32, y: i32, lower: bool, lava: bool, max_columns: usize) -> usize {
    let Some(terrain) = dim.int_height(x, y) else { return 0 };
    let water = dim.water(x, y).unwrap_or(i32::MIN);
    let fluid = water > terrain;
    if lower && !fluid {
        return 0;
    }
    let is_lava = |d: &Dimension, x: i32, y: i32| d.layer_value(LayerId::LAVA, 0, x, y) != 0;
    // Convert water <-> lava in place when clicking a fluid of the other kind.
    if fluid && !lower && lava != is_lava(dim, x, y) {
        let mut seen: FxHashSet<(i32, i32)> = FxHashSet::default();
        let mut queue = VecDeque::from([(x, y)]);
        let mut cols = Vec::new();
        while let Some((cx, cy)) = queue.pop_front() {
            if cols.len() >= max_columns || !seen.insert((cx, cy)) {
                continue;
            }
            let (Some(h), Some(w)) = (dim.int_height(cx, cy), dim.water(cx, cy)) else { continue };
            if w <= h || is_lava(dim, cx, cy) == lava {
                continue;
            }
            cols.push((cx, cy));
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                queue.push_back((cx + dx, cy + dy));
            }
        }
        for (cx, cy) in &cols {
            dim.set_layer_value(LayerId::LAVA, true, 0, *cx, *cy, lava as u8);
        }
        return cols.len();
    }
    let height = terrain.max(water);
    if (lower && height <= dim.limits.min_y) || (!lower && height >= dim.limits.max_y - 1) {
        return 0;
    }
    let target = if lower { height - 1 } else { height + 1 };
    let mut seen: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut queue = VecDeque::from([(x, y)]);
    let mut cols = Vec::new();
    while let Some((cx, cy)) = queue.pop_front() {
        if cols.len() >= max_columns || !seen.insert((cx, cy)) {
            continue;
        }
        let (Some(h), Some(w)) = (dim.int_height(cx, cy), dim.water(cx, cy)) else { continue };
        let boundary = if lower { w <= h || w <= target } else { h >= target || w >= target };
        if boundary {
            continue;
        }
        cols.push((cx, cy));
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            queue.push_back((cx + dx, cy + dy));
        }
    }
    for (cx, cy) in &cols {
        dim.set_water(*cx, *cy, target);
        dim.set_layer_value(LayerId::LAVA, true, 0, *cx, *cy, lava as u8);
    }
    cols.len()
}

fn dim_min_water(min_y: f32) -> i32 {
    min_y as i32 - 1
}

/// Flood fill water (or lava via the Lava layer) from (x, y): every connected column whose
/// surface is below `level` gets that water level. Returns the number of columns changed.
pub fn flood(dim: &mut Dimension, x: i32, y: i32, level: i32, max_columns: usize) -> usize {
    let mut seen: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut queue = VecDeque::new();
    let mut fill = Vec::new();
    queue.push_back((x, y));
    while let Some((cx, cy)) = queue.pop_front() {
        if !seen.insert((cx, cy)) || fill.len() >= max_columns {
            continue;
        }
        match dim.int_height(cx, cy) {
            Some(h) if h < level => {
                fill.push((cx, cy));
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    queue.push_back((cx + dx, cy + dy));
                }
            }
            _ => {}
        }
    }
    for (cx, cy) in &fill {
        dim.set_water(*cx, *cy, level);
    }
    fill.len()
}

/// Fill a layer/terrain over the whole dimension through a mask (or everywhere).
pub fn fill_through_mask(dim: &mut Dimension, tool: &Tool, mask: Option<&CustomMask>) {
    let snap = dim.snapshot();
    for (tx, ty) in snap.tile_coords() {
        let (ox, oy) = (tx << crate::tile::TILE_SHIFT, ty << crate::tile::TILE_SHIFT);
        let size = crate::tile::TILE_SIZE;
        let mut strengths = vec![1f32; crate::tile::TILE_AREA];
        if let Some(m) = mask {
            for ly in 0..size {
                for lx in 0..size {
                    strengths[crate::tile::local_index(lx, ly)] = m.eval(&snap, ox + lx, oy + ly);
                }
            }
        }
        dim.for_each_in_rect_mut(ox, oy, ox + size, oy + size, |t, wx, wy, i| {
            let s = strengths[i];
            if s <= 0.0 {
                return;
            }
            match tool {
                Tool::PaintTerrain(id) => {
                    if s >= 0.5 || unit(hash2(0x7e44, wx, wy)) < s * 2.0 {
                        t.terrain[i] = *id;
                    }
                }
                Tool::PaintLayer { layer, value, bits, default } => {
                    let d = t.layer_mut(*layer, *bits, *default);
                    if *bits {
                        if s >= 0.5 {
                            d.set(i, 1);
                        }
                    } else if *layer == LayerId::BIOME {
                        if s >= 0.5 {
                            d.set(i, *value);
                        }
                    } else {
                        d.set(i, ((*value as f32) * s).round() as u8);
                    }
                }
                Tool::EraseLayer { layer, bits, default } => {
                    if s >= 0.5 {
                        t.layer_mut(*layer, *bits, *default).set(i, *default);
                    }
                }
                Tool::PaintBiome(b) => {
                    if s >= 0.5 {
                        t.layer_mut(LayerId::BIOME, false, meadow_blocks::BIOME_AUTO).set(i, *b);
                    }
                }
                Tool::SetWaterLevel(level) => {
                    if s >= 0.5 {
                        t.water[i] = *level as i16;
                    }
                }
                Tool::SetHeight(level) => t.height[i] += (level - t.height[i]) * s,
                _ => {}
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimension::DimensionKind;
    use crate::mask::MaskNode;
    use meadow_blocks::BuildLimits;

    fn dim() -> Dimension {
        let mut d = Dimension::new(DimensionKind::Overworld, BuildLimits::VANILLA);
        for ty in -1..1 {
            for tx in -1..1 {
                d.ensure_tile(tx, ty, 64.0);
            }
        }
        d.clear_history();
        d
    }

    #[test]
    fn raise_and_undo() {
        let mut d = dim();
        d.begin_op("raise");
        let b = Brush { radius: 10.0, intensity: 1.0, ..Default::default() };
        let mut s = Stroke::default();
        for _ in 0..10 {
            apply(&mut d, &Tool::Raise, &b, 0.0, 0.0, 0.1, None, &mut s);
        }
        d.end_op();
        let c = d.height(0, 0).unwrap();
        assert!(c > 70.0, "{c}");
        assert_eq!(d.height(20, 20), Some(64.0));
        d.undo();
        assert_eq!(d.height(0, 0), Some(64.0));
    }

    #[test]
    fn masked_paint_and_flood() {
        let mut d = dim();
        d.set_height(5, 5, 90.0);
        let mask = CustomMask { name: "hi".into(), node: MaskNode::Height { min: 80.0, max: 400.0, feather: 0.0 }, color: [0; 3], invert: false };
        let b = Brush { radius: 20.0, intensity: 1.0, falloff: Falloff::Constant, ..Default::default() };
        apply(&mut d, &Tool::PaintTerrain(28), &b, 5.0, 5.0, 1.0, Some(&mask), &mut Stroke::default());
        assert_eq!(d.terrain(5, 5), Some(28));
        assert_eq!(d.terrain(6, 6), Some(0));
        // Lower an area and flood it.
        for y in -30..-20 {
            for x in -30..-20 {
                d.set_height(x, y, 50.0);
            }
        }
        let n = flood(&mut d, -25, -25, 58, 100_000);
        assert_eq!(n, 100);
        assert_eq!(d.water(-25, -25), Some(58));
        assert!(d.is_underwater(-25, -25));
    }

    #[test]
    fn worldpainter_flood_and_fill() {
        let mut d = dim();
        for y in -20..-10 {
            for x in -20..-10 {
                d.set_height(x, y, 50.0);
            }
        }
        // Water starts at 62 everywhere; lower it in the basin first.
        for y in -20..-10 {
            for x in -20..-10 {
                d.set_water(x, y, 40);
            }
        }
        // Each click raises the basin's water by one block.
        assert_eq!(flood_step(&mut d, -15, -15, false, false, 100_000), 100);
        assert_eq!(d.water(-15, -15), Some(51));
        assert_eq!(flood_step(&mut d, -15, -15, false, false, 100_000), 100);
        assert_eq!(d.water(-12, -12), Some(52));
        // Right-click lowers it again.
        flood_step(&mut d, -15, -15, true, false, 100_000);
        assert_eq!(d.water(-15, -15), Some(51));
        // Lava tool converts the water to lava.
        flood_step(&mut d, -15, -15, false, true, 100_000);
        assert_eq!(d.layer_value(LayerId::LAVA, 0, -15, -15), 1);
        // Fill paints the connected area of the same terrain.
        d.set_terrain(5, 5, 7);
        let n = fill_paint(&mut d, 0, 0, &Paint::Terrain(28), false, 1_000_000);
        assert_eq!(n, 256 * 256 - 1);
        assert_eq!(d.terrain(5, 5), Some(7));
        assert_eq!(d.terrain(0, 0), Some(28));
        // Right-click fill resets terrain to the theme (grass at height 64).
        fill_paint(&mut d, 0, 0, &Paint::Terrain(28), true, 1_000_000);
        assert_eq!(d.terrain(30, 30), Some(crate::terrain::ids::GRASS));
    }

    #[test]
    fn spray_paint_builds_up_and_layers_follow_worldpainter_values() {
        let mut d = dim();
        let b = Brush { radius: 10.0, intensity: 0.3, falloff: Falloff::Constant, ..Default::default() };
        let mut s = Stroke::default();
        let tool = Tool::Paint { paint: Paint::Terrain(28), mode: PaintMode::Spray, remove: false };
        apply(&mut d, &tool, &b, 0.0, 0.0, 1.0, None, &mut s);
        let count = |d: &Dimension| (-8..8).flat_map(|y| (-8..8).map(move |x| (x, y))).filter(|(x, y)| d.terrain(*x, *y) == Some(28)).count();
        let first = count(&d);
        for _ in 0..20 {
            apply(&mut d, &tool, &b, 0.0, 0.0, 1.0, None, &mut s);
        }
        assert!(first > 30 && first < 140, "{first}");
        assert!(count(&d) > 240, "spray keeps adding coverage");
        // Nibble layer: 1 + round(strength * 14)
        let layer = Paint::Layer { layer: LayerId::DECIDUOUS, bits: false, default: 0 };
        apply(&mut d, &Tool::Paint { paint: layer, mode: PaintMode::Spray, remove: false }, &b, 0.0, 0.0, 1.0, None, &mut s);
        assert_eq!(d.layer_value(LayerId::DECIDUOUS, 0, 0, 0), 5);
        // Square brushes rotate.
        let sq = Brush { radius: 10.0, shape: BrushShape::Square, rotation: 45.0, falloff: Falloff::Constant, ..Default::default() };
        assert_eq!(sq.falloff_strength(9.0, 9.0, 0, 0), 0.0);
        assert_eq!(sq.falloff_strength(13.0, 0.0, 0, 0), 1.0);
    }

    #[test]
    fn smoothing_reduces_spike() {
        let mut d = dim();
        d.set_height(0, 0, 120.0);
        let b = Brush { radius: 4.0, intensity: 1.0, ..Default::default() };
        let mut s = Stroke::default();
        for _ in 0..20 {
            apply(&mut d, &Tool::Smooth, &b, 0.5, 0.5, 0.1, None, &mut s);
        }
        assert!(d.height(0, 0).unwrap() < 80.0);
    }
}
