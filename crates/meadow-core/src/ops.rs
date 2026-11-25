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
use crate::noise::{fbm2, hash2, unit};
use crate::terrain::TerrainId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BrushShape {
    Circle,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Falloff {
    Constant,
    Linear,
    Smooth,
    Sharp,
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
}

impl Default for Brush {
    fn default() -> Self {
        Brush { radius: 25.0, intensity: 0.5, shape: BrushShape::Circle, falloff: Falloff::Smooth, roughness: 0.0 }
    }
}

impl Brush {
    /// Strength in [0, 1] at offset (dx, dy) from the centre.
    #[inline]
    pub fn strength(&self, dx: f32, dy: f32, wx: i32, wy: i32) -> f32 {
        let r = self.radius.max(0.5);
        let d = match self.shape {
            BrushShape::Circle => (dx * dx + dy * dy).sqrt() / r,
            BrushShape::Square => dx.abs().max(dy.abs()) / r,
        };
        if d > 1.0 {
            return 0.0;
        }
        let f = match self.falloff {
            Falloff::Constant => 1.0,
            Falloff::Linear => 1.0 - d,
            Falloff::Smooth => {
                let t = 1.0 - d;
                t * t * (3.0 - 2.0 * t)
            }
            Falloff::Sharp => (1.0 - d) * (1.0 - d),
        };
        let f = if self.roughness > 0.0 {
            let n = (fbm2(0x9e37, wx as f32 / 8.0, wy as f32 / 8.0, 3) + 1.0) * 0.5;
            (f * (1.0 - self.roughness + self.roughness * n * 2.0)).clamp(0.0, 1.0)
        } else {
            f
        };
        f * self.intensity
    }
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
        }
    });
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
