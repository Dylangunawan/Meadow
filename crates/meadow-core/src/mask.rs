//! Custom masks: composable, named selections of the map used as brush filters and for global
//! operations ("fill layer through mask"). They evaluate to a strength in [0, 1] per column.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::dimension::Dimension;
use crate::layer::LayerId;
use crate::noise::fbm2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaskImage {
    pub width: u32,
    pub height: u32,
    /// Greyscale values, row-major.
    pub data: Vec<u8>,
    /// World coordinates of the image's top-left pixel.
    pub origin: (i32, i32),
    /// Blocks per pixel.
    pub scale: f32,
}

impl MaskImage {
    pub fn load(path: &Path, origin: (i32, i32), scale: f32) -> Result<MaskImage> {
        let img = image::open(path).with_context(|| format!("opening {}", path.display()))?.to_luma8();
        Ok(MaskImage { width: img.width(), height: img.height(), data: img.into_raw(), origin, scale: scale.max(0.01) })
    }

    #[inline]
    pub fn sample(&self, x: i32, y: i32) -> f32 {
        let fx = (x - self.origin.0) as f32 / self.scale;
        let fy = (y - self.origin.1) as f32 / self.scale;
        if fx < 0.0 || fy < 0.0 {
            return 0.0;
        }
        let (px, py) = (fx as u32, fy as u32);
        if px >= self.width || py >= self.height {
            return 0.0;
        }
        self.data[(py * self.width + px) as usize] as f32 / 255.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MaskNode {
    All,
    Image(MaskImage),
    /// Height in [min, max] with a soft edge of `feather` blocks.
    Height { min: f32, max: f32, feather: f32 },
    /// Slope in degrees.
    Slope { min: f32, max: f32, feather: f32 },
    Terrain(Vec<u8>),
    /// Layer value >= min.
    Layer { layer: LayerId, min: u8 },
    /// Under water (true) or on land (false).
    Water(bool),
    Biome(Vec<u8>),
    Noise { seed: u64, scale: f32, octaves: u32, threshold: f32, feather: f32 },
    /// Hand-painted mask stored in a `PaintedMask` layer.
    Painted(LayerId),
    /// Columns within `distance` blocks of the world edge (tile border).
    Edge { distance: f32 },
    Not(Box<MaskNode>),
    And(Vec<MaskNode>),
    Or(Vec<MaskNode>),
    /// Multiply strength by a constant.
    Strength(Box<MaskNode>, f32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomMask {
    pub name: String,
    pub node: MaskNode,
    pub color: [u8; 3],
    pub invert: bool,
}

impl CustomMask {
    #[inline]
    pub fn eval(&self, dim: &Dimension, x: i32, y: i32) -> f32 {
        let v = self.node.eval(dim, x, y);
        if self.invert { 1.0 - v } else { v }
    }
}

#[inline]
fn band(v: f32, min: f32, max: f32, feather: f32) -> f32 {
    if feather <= 0.0 {
        return if v >= min && v <= max { 1.0 } else { 0.0 };
    }
    let lo = ((v - min) / feather + 1.0).clamp(0.0, 1.0);
    let hi = ((max - v) / feather + 1.0).clamp(0.0, 1.0);
    lo.min(hi)
}

impl MaskNode {
    pub fn eval(&self, dim: &Dimension, x: i32, y: i32) -> f32 {
        match self {
            MaskNode::All => 1.0,
            MaskNode::Image(img) => img.sample(x, y),
            MaskNode::Height { min, max, feather } => match dim.height(x, y) {
                Some(h) => band(h, *min, *max, *feather),
                None => 0.0,
            },
            MaskNode::Slope { min, max, feather } => band(dim.slope(x, y), *min, *max, *feather),
            MaskNode::Terrain(list) => dim.terrain(x, y).map(|t| list.contains(&t) as u8 as f32).unwrap_or(0.0),
            MaskNode::Layer { layer, min } => (dim.layer_value(*layer, 0, x, y) >= *min.max(&1)) as u8 as f32,
            MaskNode::Water(under) => (dim.height(x, y).is_some() && dim.is_underwater(x, y) == *under) as u8 as f32,
            MaskNode::Biome(list) => list.contains(&dim.layer_value(LayerId::BIOME, 255, x, y)) as u8 as f32,
            MaskNode::Noise { seed, scale, octaves, threshold, feather } => {
                let n = fbm2(*seed, x as f32 / scale.max(1.0), y as f32 / scale.max(1.0), (*octaves).clamp(1, 8));
                let v = (n + 1.0) * 0.5;
                if *feather <= 0.0 {
                    (v >= *threshold) as u8 as f32
                } else {
                    ((v - threshold) / feather + 0.5).clamp(0.0, 1.0)
                }
            }
            MaskNode::Painted(layer) => dim.layer_value(*layer, 0, x, y) as f32 / 255.0,
            MaskNode::Edge { distance } => {
                let mut best = f32::MAX;
                let r = distance.ceil() as i32;
                // Sample a sparse ring of neighbours; cheap enough for brush use.
                for (dx, dy) in [(r, 0), (-r, 0), (0, r), (0, -r), (r, r), (-r, -r), (r, -r), (-r, r)] {
                    if dim.height(x + dx, y + dy).is_none() {
                        best = best.min(((dx * dx + dy * dy) as f32).sqrt());
                    }
                }
                if best == f32::MAX { 0.0 } else { 1.0 }
            }
            MaskNode::Not(n) => 1.0 - n.eval(dim, x, y),
            MaskNode::And(list) => {
                let mut v: f32 = 1.0;
                for n in list {
                    v = v.min(n.eval(dim, x, y));
                    if v <= 0.0 {
                        break;
                    }
                }
                v
            }
            MaskNode::Or(list) => list.iter().map(|n| n.eval(dim, x, y)).fold(0.0, f32::max),
            MaskNode::Strength(n, s) => (n.eval(dim, x, y) * s).clamp(0.0, 1.0),
        }
    }

    /// Short human-readable description for the UI.
    pub fn describe(&self) -> String {
        match self {
            MaskNode::All => "everything".into(),
            MaskNode::Image(i) => format!("image {}x{}", i.width, i.height),
            MaskNode::Height { min, max, .. } => format!("height {min}..{max}"),
            MaskNode::Slope { min, max, .. } => format!("slope {min}°..{max}°"),
            MaskNode::Terrain(t) => format!("terrain in {t:?}"),
            MaskNode::Layer { layer, min } => format!("layer {} >= {min}", layer.0),
            MaskNode::Water(true) => "under water".into(),
            MaskNode::Water(false) => "on land".into(),
            MaskNode::Biome(b) => format!("biome in {b:?}"),
            MaskNode::Noise { scale, threshold, .. } => format!("noise (scale {scale}, threshold {threshold})"),
            MaskNode::Painted(_) => "painted".into(),
            MaskNode::Edge { distance } => format!("within {distance} of edge"),
            MaskNode::Not(n) => format!("not ({})", n.describe()),
            MaskNode::And(l) => l.iter().map(|n| n.describe()).collect::<Vec<_>>().join(" and "),
            MaskNode::Or(l) => l.iter().map(|n| n.describe()).collect::<Vec<_>>().join(" or "),
            MaskNode::Strength(n, s) => format!("{} x{s}", n.describe()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimension::DimensionKind;
    use meadow_blocks::BuildLimits;

    #[test]
    fn combine() {
        let mut d = Dimension::new(DimensionKind::Overworld, BuildLimits::VANILLA);
        d.ensure_tile(0, 0, 64.0);
        d.set_height(1, 1, 100.0);
        let m = CustomMask {
            name: "high land".into(),
            node: MaskNode::And(vec![MaskNode::Height { min: 80.0, max: 300.0, feather: 0.0 }, MaskNode::Water(false)]),
            color: [255, 0, 0],
            invert: false,
        };
        assert_eq!(m.eval(&d, 1, 1), 1.0);
        assert_eq!(m.eval(&d, 2, 2), 0.0);
        let feathered = MaskNode::Height { min: 70.0, max: 300.0, feather: 10.0 };
        let v = feathered.eval(&d, 2, 2);
        assert!(v > 0.0 && v < 1.0, "{v}");
    }
}
