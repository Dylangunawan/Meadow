//! Height map import/export and whole-world resampling:
//! * import 8/16-bit PNG/TIFF/JPEG (grey or a selected colour channel) and raw `.r16`/`.raw`;
//! * horizontal scale on import, and resize the world horizontally afterwards;
//! * vertical mapping on import (image range -> world range), and vertical rescaling afterwards.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::dimension::Dimension;
use crate::tile::{TILE_SHIFT, TILE_SIZE, Tile, local_index};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    Grey,
    Red,
    Green,
    Blue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resample {
    Nearest,
    Bilinear,
    Bicubic,
}

/// A loaded height map: raw sample values plus their possible range.
#[derive(Clone)]
pub struct HeightMap {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,
    /// Maximum possible sample value (255, 65535, or 1.0 for float images).
    pub max_value: f32,
    /// Raw RGBA (for colour mapping in scripts), if the source was a colour image.
    pub rgb: Option<Vec<[u8; 4]>>,
}

impl HeightMap {
    pub fn load(path: &Path, channel: Channel) -> Result<HeightMap> {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ext == "r16" || ext == "raw" {
            return Self::load_raw16(path);
        }
        let img = image::open(path).with_context(|| format!("opening {}", path.display()))?;
        Ok(Self::from_image(&img, channel))
    }

    pub fn from_image(img: &image::DynamicImage, channel: Channel) -> HeightMap {
        use image::DynamicImage as D;
        let (w, h) = (img.width(), img.height());
        let pick = |c: &[f32]| -> f32 {
            match channel {
                Channel::Grey => {
                    if c.len() >= 3 {
                        0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
                    } else {
                        c[0]
                    }
                }
                Channel::Red => c[0],
                Channel::Green => c[1.min(c.len() - 1)],
                Channel::Blue => c[2.min(c.len() - 1)],
            }
        };
        let rgb = if img.color().has_color() { Some(img.to_rgba8().pixels().map(|p| p.0).collect()) } else { None };
        match img {
            D::ImageLuma16(i) => HeightMap { width: w, height: h, data: i.pixels().map(|p| p.0[0] as f32).collect(), max_value: 65535.0, rgb },
            D::ImageLumaA16(i) => HeightMap { width: w, height: h, data: i.pixels().map(|p| p.0[0] as f32).collect(), max_value: 65535.0, rgb },
            D::ImageRgb16(i) => HeightMap {
                width: w,
                height: h,
                data: i.pixels().map(|p| pick(&p.0.map(|v| v as f32))).collect(),
                max_value: 65535.0,
                rgb,
            },
            D::ImageRgba16(i) => HeightMap {
                width: w,
                height: h,
                data: i.pixels().map(|p| pick(&p.0[..3].iter().map(|v| *v as f32).collect::<Vec<_>>())).collect(),
                max_value: 65535.0,
                rgb,
            },
            D::ImageRgb32F(_) | D::ImageRgba32F(_) => {
                let f = img.to_rgb32f();
                let data: Vec<f32> = f.pixels().map(|p| pick(&p.0)).collect();
                let max = data.iter().copied().fold(1.0f32, f32::max);
                HeightMap { width: w, height: h, data, max_value: max, rgb }
            }
            _ => {
                let i = img.to_rgba8();
                HeightMap {
                    width: w,
                    height: h,
                    data: i.pixels().map(|p| pick(&[p.0[0] as f32, p.0[1] as f32, p.0[2] as f32])).collect(),
                    max_value: 255.0,
                    rgb,
                }
            }
        }
    }

    /// Square little-endian 16-bit raw height map.
    pub fn load_raw16(path: &Path) -> Result<HeightMap> {
        let bytes = std::fs::read(path)?;
        let n = bytes.len() / 2;
        let side = (n as f64).sqrt() as u32;
        if (side as usize) * (side as usize) != n {
            bail!("raw height map is not square ({n} samples)");
        }
        let data = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as f32).collect();
        Ok(HeightMap { width: side, height: side, data, max_value: 65535.0, rgb: None })
    }

    #[inline]
    pub fn get(&self, x: i32, y: i32) -> f32 {
        let x = x.clamp(0, self.width as i32 - 1) as u32;
        let y = y.clamp(0, self.height as i32 - 1) as u32;
        self.data[(y * self.width + x) as usize]
    }

    pub fn rgba(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        self.rgb.as_ref().map(|c| c[(y as u32 * self.width + x as u32) as usize])
    }

    pub fn sample(&self, x: f32, y: f32, mode: Resample) -> f32 {
        match mode {
            Resample::Nearest => self.get(x.floor() as i32, y.floor() as i32),
            Resample::Bilinear => {
                let (x, y) = (x - 0.5, y - 0.5);
                let (x0, y0) = (x.floor() as i32, y.floor() as i32);
                let (fx, fy) = (x - x0 as f32, y - y0 as f32);
                let a = self.get(x0, y0) * (1.0 - fx) + self.get(x0 + 1, y0) * fx;
                let b = self.get(x0, y0 + 1) * (1.0 - fx) + self.get(x0 + 1, y0 + 1) * fx;
                a * (1.0 - fy) + b * fy
            }
            Resample::Bicubic => {
                let (x, y) = (x - 0.5, y - 0.5);
                let (x0, y0) = (x.floor() as i32, y.floor() as i32);
                let (fx, fy) = (x - x0 as f32, y - y0 as f32);
                let mut col = [0f32; 4];
                for (j, c) in col.iter_mut().enumerate() {
                    let yy = y0 - 1 + j as i32;
                    *c = cubic(
                        self.get(x0 - 1, yy),
                        self.get(x0, yy),
                        self.get(x0 + 1, yy),
                        self.get(x0 + 2, yy),
                        fx,
                    );
                }
                cubic(col[0], col[1], col[2], col[3], fy)
            }
        }
    }

    pub fn min_max(&self) -> (f32, f32) {
        self.data.iter().fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)))
    }
}

#[inline]
fn cubic(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    // Catmull-Rom
    0.5 * ((2.0 * p1) + (-p0 + p2) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t * t * t)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSettings {
    /// Horizontal scale in percent (100 = one pixel per block).
    pub scale_percent: f32,
    pub image_low: f32,
    pub image_high: f32,
    pub world_low: f32,
    pub world_high: f32,
    pub water_level: i32,
    pub terrain: u8,
    /// World coordinates of the image's top-left corner.
    pub offset: (i32, i32),
    pub resample: Resample,
    /// Only raise existing terrain when importing into an existing dimension.
    pub only_raise: bool,
    /// Columns at or below this world height become void (tiles not created). None = never.
    pub void_below: Option<f32>,
}

impl ImportSettings {
    pub fn for_map(map: &HeightMap, min_y: i32, max_y: i32) -> ImportSettings {
        let (lo, hi) = map.min_max();
        ImportSettings {
            scale_percent: 100.0,
            image_low: lo,
            image_high: hi.max(lo + 1.0),
            world_low: 40.0f32.max(min_y as f32),
            world_high: (200.0f32).min((max_y - 1) as f32),
            water_level: 62,
            terrain: 0,
            offset: (0, 0),
            resample: Resample::Bicubic,
            only_raise: false,
            void_below: None,
        }
    }

    #[inline]
    pub fn map_value(&self, v: f32) -> f32 {
        let t = (v - self.image_low) / (self.image_high - self.image_low).max(1e-6);
        self.world_low + t * (self.world_high - self.world_low)
    }

    pub fn world_size(&self, map: &HeightMap) -> (i32, i32) {
        let s = self.scale_percent / 100.0;
        (((map.width as f32) * s).round() as i32, ((map.height as f32) * s).round() as i32)
    }
}

/// Import a height map into a dimension, creating tiles as needed. Runs tile generation in
/// parallel. Returns the number of tiles touched.
pub fn import_into(dim: &mut Dimension, map: &HeightMap, s: &ImportSettings) -> usize {
    let (w, h) = s.world_size(map);
    let scale = s.scale_percent / 100.0;
    let (x0, y0) = s.offset;
    let (x1, y1) = (x0 + w, y0 + h);
    let (lo, hi) = (dim.limits.min_y as f32, (dim.limits.max_y - 1) as f32);
    let mut coords = Vec::new();
    for ty in (y0 >> TILE_SHIFT)..=((y1 - 1) >> TILE_SHIFT) {
        for tx in (x0 >> TILE_SHIFT)..=((x1 - 1) >> TILE_SHIFT) {
            coords.push((tx, ty));
        }
    }
    let existing: Vec<Option<Tile>> = coords.iter().map(|(tx, ty)| dim.tile(*tx, *ty).map(|t| (**t).clone())).collect();
    let default_terrain = s.terrain;
    let tiles: Vec<Option<Tile>> = coords
        .par_iter()
        .zip(existing.into_par_iter())
        .map(|(&(tx, ty), old)| {
            let mut t = old.unwrap_or_else(|| Tile::new(tx, ty, lo, default_terrain, s.water_level as i16));
            let fresh = t.height.iter().all(|v| *v == lo);
            let mut any = !fresh;
            for ly in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let (wx, wy) = ((tx << TILE_SHIFT) + lx, (ty << TILE_SHIFT) + ly);
                    if wx < x0 || wy < y0 || wx >= x1 || wy >= y1 {
                        continue;
                    }
                    let v = map.sample((wx - x0) as f32 / scale + 0.5 / scale, (wy - y0) as f32 / scale + 0.5 / scale, s.resample);
                    let hgt = s.map_value(v).clamp(lo, hi);
                    if let Some(vb) = s.void_below
                        && hgt <= vb
                    {
                        continue;
                    }
                    let i = local_index(lx, ly);
                    if s.only_raise && !fresh {
                        t.height[i] = t.height[i].max(hgt);
                    } else {
                        t.height[i] = hgt;
                        t.water[i] = s.water_level as i16;
                        if fresh {
                            t.terrain[i] = default_terrain;
                        }
                    }
                    any = true;
                }
            }
            any.then_some(t)
        })
        .collect();
    let mut n = 0;
    for t in tiles.into_iter().flatten() {
        let mut t = t;
        t.touch();
        dim.add_tile(t);
        n += 1;
    }
    n
}

/// Rescale heights vertically around `base`: h' = base + (h - base) * factor. Water levels are
/// scaled too when `scale_water` is set.
pub fn scale_vertical(dim: &mut Dimension, factor: f32, base: f32, scale_water: bool) {
    let (lo, hi) = (dim.limits.min_y as f32, (dim.limits.max_y - 1) as f32);
    for (tx, ty) in dim.tile_coords() {
        let t = dim.tile_mut(tx, ty).unwrap();
        t.height.par_iter_mut().for_each(|h| *h = (base + (*h - base) * factor).clamp(lo, hi));
        if scale_water {
            for w in t.water.iter_mut() {
                *w = (base + (*w as f32 - base) * factor).round().clamp(lo - 1.0, hi) as i16;
            }
        }
    }
}

/// Linearly remap heights from [from_lo, from_hi] to [to_lo, to_hi].
pub fn remap_heights(dim: &mut Dimension, from_lo: f32, from_hi: f32, to_lo: f32, to_hi: f32) {
    let (lo, hi) = (dim.limits.min_y as f32, (dim.limits.max_y - 1) as f32);
    let k = (to_hi - to_lo) / (from_hi - from_lo).max(1e-6);
    for (tx, ty) in dim.tile_coords() {
        let t = dim.tile_mut(tx, ty).unwrap();
        t.height.par_iter_mut().for_each(|h| *h = (to_lo + (*h - from_lo) * k).clamp(lo, hi));
    }
}

/// Resize the dimension horizontally by `factor` (e.g. 2.0 doubles the width and length).
/// Heights are resampled bicubically, everything else nearest-neighbour.
pub fn resize_horizontal(dim: &mut Dimension, factor: f32) {
    let factor = factor.clamp(0.05, 16.0);
    let src = dim.snapshot();
    let Some((bx0, by0, bx1, by1)) = src.block_bounds() else { return };
    let nx0 = (bx0 as f32 * factor).floor() as i32;
    let ny0 = (by0 as f32 * factor).floor() as i32;
    let nx1 = (bx1 as f32 * factor).ceil() as i32;
    let ny1 = (by1 as f32 * factor).ceil() as i32;
    let mut coords = Vec::new();
    for ty in (ny0 >> TILE_SHIFT)..=((ny1 - 1) >> TILE_SHIFT) {
        for tx in (nx0 >> TILE_SHIFT)..=((nx1 - 1) >> TILE_SHIFT) {
            coords.push((tx, ty));
        }
    }
    let src_ref = &src;
    let new_tiles: Vec<Tile> = coords
        .par_iter()
        .filter_map(|&(tx, ty)| {
            let mut t = Tile::new(tx, ty, src_ref.limits.min_y as f32, src_ref.default_terrain, src_ref.default_water_level as i16);
            let mut any = false;
            let mut layers_seen: Vec<crate::layer::LayerId> = Vec::new();
            for ly in 0..TILE_SIZE {
                for lx in 0..TILE_SIZE {
                    let wx = (tx << TILE_SHIFT) + lx;
                    let wy = (ty << TILE_SHIFT) + ly;
                    let sx = (wx as f32 + 0.5) / factor;
                    let sy = (wy as f32 + 0.5) / factor;
                    let (ix, iy) = (sx.floor() as i32, sy.floor() as i32);
                    let (stx, sty) = (ix >> TILE_SHIFT, iy >> TILE_SHIFT);
                    let Some(st) = src_ref.tile(stx, sty) else { continue };
                    any = true;
                    let si = local_index(ix & (TILE_SIZE - 1), iy & (TILE_SIZE - 1));
                    let i = local_index(lx, ly);
                    t.height[i] = bicubic_height(src_ref, sx - 0.5, sy - 0.5);
                    t.terrain[i] = st.terrain[si];
                    t.water[i] = st.water[si];
                    for (id, d) in &st.layers {
                        let bits = matches!(d, crate::tile::LayerData::Bits(_));
                        if !layers_seen.contains(id) {
                            layers_seen.push(*id);
                        }
                        let v = d.get(si);
                        let def = if *id == crate::layer::LayerId::BIOME { meadow_blocks::BIOME_AUTO } else { 0 };
                        if v != def || t.has_layer(*id) {
                            t.layer_mut(*id, bits, def).set(i, v);
                        }
                    }
                }
            }
            any.then_some(t)
        })
        .collect();
    for c in src.tile_coords() {
        dim.remove_tile(c.0, c.1);
    }
    for t in new_tiles {
        dim.add_tile(t);
    }
}

fn bicubic_height(d: &Dimension, x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let g = |a: i32, b: i32| d.height(a, b).or_else(|| d.height(x0.max(a.min(x0 + 1)), y0.max(b.min(y0 + 1)))).unwrap_or(d.limits.min_y as f32);
    let mut col = [0f32; 4];
    for (j, c) in col.iter_mut().enumerate() {
        let yy = y0 - 1 + j as i32;
        *c = cubic(g(x0 - 1, yy), g(x0, yy), g(x0 + 1, yy), g(x0 + 2, yy), fx);
    }
    let (lo, hi) = (d.limits.min_y as f32, (d.limits.max_y - 1) as f32);
    cubic(col[0], col[1], col[2], col[3], fy).clamp(lo, hi)
}

/// Export the dimension's heights as a 16-bit greyscale PNG (min_y -> 0, max_y -> 65535).
pub fn export_png16(dim: &Dimension, path: &Path) -> Result<()> {
    let Some((x0, y0, x1, y1)) = dim.block_bounds() else { bail!("empty dimension") };
    let (w, h) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let span = (dim.limits.max_y - dim.limits.min_y) as f32;
    let mut img = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::new(w, h);
    for (px, py, p) in img.enumerate_pixels_mut() {
        let hv = dim.height(x0 + px as i32, y0 + py as i32).unwrap_or(dim.limits.min_y as f32);
        p.0[0] = (((hv - dim.limits.min_y as f32) / span) * 65535.0).clamp(0.0, 65535.0) as u16;
    }
    img.save(path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dimension::DimensionKind;
    use meadow_blocks::BuildLimits;

    fn gradient(w: u32, h: u32) -> HeightMap {
        let data = (0..h).flat_map(|y| (0..w).map(move |x| ((x + y) * 65535 / (w + h - 2)) as f32)).collect();
        HeightMap { width: w, height: h, data, max_value: 65535.0, rgb: None }
    }

    #[test]
    fn import_scale_and_rescale() {
        let map = gradient(200, 100);
        let mut d = Dimension::new(DimensionKind::Overworld, BuildLimits::VANILLA);
        let mut s = ImportSettings::for_map(&map, -64, 320);
        s.world_low = 0.0;
        s.world_high = 200.0;
        s.scale_percent = 200.0;
        let n = import_into(&mut d, &map, &s);
        // 400 x 200 blocks -> 4 x 2 tiles (rounded up)
        assert_eq!(n, 8);
        let h0 = d.height(0, 0).unwrap();
        let h1 = d.height(399, 199).unwrap();
        assert!(h0 < 5.0 && h1 > 195.0, "{h0} {h1}");
        // Vertical rescale: double around 0
        scale_vertical(&mut d, 1.5, 0.0, false);
        assert!((d.height(399, 199).unwrap() - (h1 * 1.5).min(319.0)).abs() < 0.01);
        // Resize horizontally to half
        let before = d.height(200, 100).unwrap();
        resize_horizontal(&mut d, 0.5);
        let after = d.height(100, 50).unwrap();
        assert!((before - after).abs() < 4.0, "{before} vs {after}");
        assert!(d.height(300, 150).is_none());
    }
}
