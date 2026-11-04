//! CPU meshing of tiles (terrain columns, water, decorations) into compact vertex buffers.

use bytemuck::{Pod, Zeroable};
use meadow_blocks::StateId;
use meadow_core::decorate::{all_placements, plant_at, plant_layers};
use meadow_core::layer::{LayerId, LayerKind};
use meadow_core::noise::hash2;
use meadow_core::tile::{TILE_SHIFT, TILE_SIZE};
use meadow_core::{Dimension, World};
use rustc_hash::FxHashMap;

/// Positions are in 1/8 block units relative to the tile origin; `pos[3]` is the face/normal
/// index (0 +Y, 1 -Y, 2 +X, 3 -X, 4 +Z, 5 -Z, 6 plant).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [i16; 4],
    pub color: [u8; 4],
}

pub const SUB: f32 = 8.0;

#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    #[inline]
    fn quad(&mut self, p: [[f32; 3]; 4], normal: i16, color: [u8; 4]) {
        let base = self.vertices.len() as u32;
        for v in p {
            self.vertices.push(Vertex {
                pos: [(v[0] * SUB) as i16, (v[1] * SUB) as i16, (v[2] * SUB) as i16, normal],
                color,
            });
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

pub struct TileMesh {
    pub tx: i32,
    pub ty: i32,
    pub step: i32,
    pub with_decorations: bool,
    pub opaque: MeshData,
    pub water: MeshData,
    pub min_y: f32,
    pub max_y: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct MeshOptions {
    pub water: bool,
    pub decorations: bool,
    pub plants: bool,
    /// Tint the ground with layer colours where no decorations are drawn (e.g. far forests).
    pub layer_tints: bool,
    /// Draw columns without a tile neighbour down to this many blocks below the surface.
    pub skirt: f32,
}

impl Default for MeshOptions {
    fn default() -> Self {
        MeshOptions { water: true, decorations: true, plants: true, layer_tints: true, skirt: 24.0 }
    }
}

#[inline]
fn shade(c: [u8; 3], f: f32) -> [u8; 3] {
    c.map(|v| (v as f32 * f).clamp(0.0, 255.0) as u8)
}

#[inline]
fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    [0, 1, 2].map(|i| (a[i] as f32 * (1.0 - t) + b[i] as f32 * t) as u8)
}

fn ctop_base(c: &Colors, t: usize, var: f32) -> [u8; 3] {
    shade(c.top.get(t).copied().unwrap_or([128, 128, 128]), var * 0.85)
}

struct Colors {
    top: Vec<[u8; 3]>,
    side: Vec<[u8; 3]>,
}

fn terrain_colors(world: &World) -> Colors {
    let top = world.terrains.iter().map(|t| t.color).collect();
    let side = world
        .terrains
        .iter()
        .map(|t| if t.strata.len() > 1 { t.strata[1].material.color() } else { t.color })
        .collect();
    Colors { top, side }
}

/// Build the mesh for one tile at the given sampling step (1, 2, 4, 8, 16).
pub fn build_tile(world: &World, dim: &Dimension, tx: i32, ty: i32, step: i32, opts: &MeshOptions) -> Option<TileMesh> {
    let tile = dim.tile(tx, ty)?;
    let step = step.clamp(1, TILE_SIZE);
    let (ox, oz) = (tx << TILE_SHIFT, ty << TILE_SHIFT);
    let colors = terrain_colors(world);
    let tints: Vec<(LayerId, [u8; 3], f32)> = if opts.layer_tints {
        world
            .layers
            .iter()
            .filter_map(|l| match l.kind {
                LayerKind::Trees(_) | LayerKind::CustomObjects(_) if step > 1 || !opts.decorations => Some((l.id, l.color, 0.6)),
                LayerKind::Annotations => Some((l.id, l.color, 0.5)),
                LayerKind::Plants(_) if step > 1 => Some((l.id, l.color, 0.25)),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    };
    let min_y = dim.limits.min_y as f32;
    let n = TILE_SIZE / step;
    let mut opaque = MeshData::default();
    let mut water = MeshData::default();
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    let s = step as f32;
    let water_color = [52u8, 96, 210];
    let frost_white = [238u8, 244, 248];

    let col_height = |x: i32, z: i32| -> Option<f32> {
        if dim.layer_value(LayerId::VOID, 0, x, z) != 0 {
            return None;
        }
        dim.int_height(x, z).map(|h| h as f32 + 1.0)
    };

    for cz in 0..n {
        for cx in 0..n {
            let (lx, lz) = (cx * step, cz * step);
            let (wx, wz) = (ox + lx, oz + lz);
            let i = meadow_core::tile::local_index(lx, lz);
            if tile.layer_value(LayerId::VOID, i, 0) != 0 {
                continue;
            }
            let top = (tile.height[i] + 0.5).floor() + 1.0;
            let t = tile.terrain[i] as usize;
            let var = 0.92 + (hash2(7, wx, wz) % 1000) as f32 / 1000.0 * 0.16;
            let mut ctop = shade(colors.top.get(t).copied().unwrap_or([128, 128, 128]), var);
            let cside = shade(colors.side.get(t).copied().unwrap_or([110, 100, 90]), var * 0.95);
            // Coarse LODs show mostly the surface colour on cliffs, which avoids speckle.
            let cside = mix(ctop_base(&colors, t, var), cside, (1.0 / step as f32).max(0.2));
            for (id, c, a) in &tints {
                if tile.layer_value(*id, i, 0) != 0 {
                    ctop = mix(ctop, *c, *a);
                }
            }
            if tile.layer_value(LayerId::FROST, i, 0) != 0 {
                ctop = frost_white;
            }
            let wlevel = tile.water[i] as f32;
            let under = wlevel + 1.0 > top;
            if under && !opts.water {
                ctop = shade(ctop, 0.7);
            }
            let (x0, z0) = (lx as f32, lz as f32);
            let (x1, z1) = (x0 + s, z0 + s);
            opaque.quad([[x0, top, z0], [x0, top, z1], [x1, top, z1], [x1, top, z0]], 0, [ctop[0], ctop[1], ctop[2], 255]);
            lo = lo.min(top);
            hi = hi.max(top);
            // Sides
            for (dx, dz, normal) in [(1, 0, 2i16), (-1, 0, 3), (0, 1, 4), (0, -1, 5)] {
                let (nx, nz) = (wx + dx * step, wz + dz * step);
                let nh = match col_height(nx, nz) {
                    Some(h) => h,
                    None => (top - opts.skirt).max(min_y),
                };
                if nh >= top {
                    continue;
                }
                lo = lo.min(nh);
                let c = [cside[0], cside[1], cside[2], 255];
                let q = match normal {
                    2 => [[x1, nh, z0], [x1, top, z0], [x1, top, z1], [x1, nh, z1]],
                    3 => [[x0, nh, z1], [x0, top, z1], [x0, top, z0], [x0, nh, z0]],
                    4 => [[x1, nh, z1], [x1, top, z1], [x0, top, z1], [x0, nh, z1]],
                    _ => [[x0, nh, z0], [x0, top, z0], [x1, top, z0], [x1, nh, z0]],
                };
                opaque.quad(q, normal, c);
            }
            if opts.water && under {
                let wy = wlevel + 0.875;
                let lava = tile.layer_value(LayerId::LAVA, i, 0) != 0;
                let frost = tile.layer_value(LayerId::FROST, i, 0) != 0;
                let (c, a) = if lava {
                    ([220u8, 90, 20], 255u8)
                } else if frost {
                    ([150u8, 185, 250], 230)
                } else {
                    (water_color, 150)
                };
                water.quad([[x0, wy, z0], [x0, wy, z1], [x1, wy, z1], [x1, wy, z0]], 0, [c[0], c[1], c[2], a]);
                hi = hi.max(wy);
            }
        }
    }

    let with_decorations = step == 1 && opts.decorations;
    if with_decorations {
        let (dlo, dhi) = decorations(world, dim, ox, oz, &mut opaque, opts.plants);
        lo = lo.min(dlo);
        hi = hi.max(dhi);
    }
    if lo == f32::MAX {
        lo = min_y;
        hi = min_y + 1.0;
    }
    Some(TileMesh { tx, ty, step, with_decorations, opaque, water, min_y: lo, max_y: hi })
}

/// Voxel meshing of trees, custom objects and plants inside the tile.
fn decorations(world: &World, dim: &Dimension, ox: i32, oz: i32, out: &mut MeshData, plants: bool) -> (f32, f32) {
    let (x1, z1) = (ox + TILE_SIZE, oz + TILE_SIZE);
    let mut vox: FxHashMap<(i32, i32, i32), StateId> = FxHashMap::default();
    for p in all_placements(world, dim, ox, oz, x1, z1) {
        for &(bx, by, bz, s) in &p.variant.blocks {
            let (x, y, z) = (p.x + bx, p.y + by, p.z + bz);
            if x < ox || z < oz || x >= x1 || z >= z1 {
                continue;
            }
            if !p.replace_solid && dim.int_height(x, z).is_some_and(|h| y <= h) {
                continue;
            }
            vox.entry((x, y, z)).or_insert(s);
        }
    }
    let mut info_cache: FxHashMap<StateId, meadow_blocks::StateInfo> = FxHashMap::default();
    let mut info = |s: StateId| *info_cache.entry(s).or_insert_with(|| s.info());
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    // Plants on top of columns.
    let mut plant_list: Vec<(i32, i32, i32, StateId, u8)> = Vec::new();
    if plants {
        let pl = plant_layers(world);
        for z in oz..z1 {
            for x in ox..x1 {
                if let Some(p) = plant_at(world, dim, x, z, &pl) {
                    let h = dim.int_height(x, z).unwrap_or(0);
                    if dim.layer_value(LayerId::FROST, 0, x, z) == 0 {
                        let hgt = if p.upper.is_some() { 2 } else { p.height };
                        plant_list.push((x, h + 1, z, p.block, hgt));
                    }
                }
            }
        }
    }
    let opaque_at = |vox: &FxHashMap<(i32, i32, i32), StateId>, info: &mut dyn FnMut(StateId) -> meadow_blocks::StateInfo, x: i32, y: i32, z: i32| -> bool {
        if let Some(s) = vox.get(&(x, y, z)) {
            let i = info(*s);
            return i.solid && i.color[3] == 255 && !i.leaves;
        }
        dim.int_height(x, z).is_some_and(|h| y <= h)
    };
    for (&(x, y, z), &s) in &vox {
        let inf = info(s);
        if inf.air || inf.color[3] == 0 {
            continue;
        }
        let c = inf.color;
        let var = 0.9 + (hash2(11, x ^ (y << 10), z) % 1000) as f32 / 1000.0 * 0.2;
        let col = shade([c[0], c[1], c[2]], var);
        let col = [col[0], col[1], col[2], c[3].max(200)];
        let (fx, fy, fz) = ((x - ox) as f32, y as f32, (z - oz) as f32);
        lo = lo.min(fy);
        hi = hi.max(fy + 1.0);
        if !inf.solid && !inf.liquid && !inf.leaves {
            cross(out, fx, fy, fz, 1.0, col);
            continue;
        }
        let faces: [(i32, i32, i32, i16); 6] = [(0, 1, 0, 0), (0, -1, 0, 1), (1, 0, 0, 2), (-1, 0, 0, 3), (0, 0, 1, 4), (0, 0, -1, 5)];
        for (dx, dy, dz, n) in faces {
            if opaque_at(&vox, &mut info, x + dx, y + dy, z + dz) {
                continue;
            }
            let (a, b, cz) = (fx, fy, fz);
            let q = match n {
                0 => [[a, b + 1.0, cz], [a, b + 1.0, cz + 1.0], [a + 1.0, b + 1.0, cz + 1.0], [a + 1.0, b + 1.0, cz]],
                1 => [[a, b, cz], [a + 1.0, b, cz], [a + 1.0, b, cz + 1.0], [a, b, cz + 1.0]],
                2 => [[a + 1.0, b, cz], [a + 1.0, b + 1.0, cz], [a + 1.0, b + 1.0, cz + 1.0], [a + 1.0, b, cz + 1.0]],
                3 => [[a, b, cz + 1.0], [a, b + 1.0, cz + 1.0], [a, b + 1.0, cz], [a, b, cz]],
                4 => [[a + 1.0, b, cz + 1.0], [a + 1.0, b + 1.0, cz + 1.0], [a, b + 1.0, cz + 1.0], [a, b, cz + 1.0]],
                _ => [[a, b, cz], [a, b + 1.0, cz], [a + 1.0, b + 1.0, cz], [a + 1.0, b, cz]],
            };
            out.quad(q, n, col);
        }
    }
    for (x, y, z, s, h) in plant_list {
        if vox.contains_key(&(x, y, z)) {
            continue;
        }
        let c = info(s).color;
        let (fx, fy, fz) = ((x - ox) as f32, y as f32, (z - oz) as f32);
        cross(out, fx, fy, fz, h as f32 * 0.9, [c[0], c[1], c[2], 255]);
        hi = hi.max(fy + h as f32);
    }
    (lo, hi)
}

fn cross(out: &mut MeshData, x: f32, y: f32, z: f32, h: f32, c: [u8; 4]) {
    let (a, b) = (0.15, 0.85);
    // Double-sided: emit both windings.
    for q in [
        [[x + a, y, z + a], [x + a, y + h, z + a], [x + b, y + h, z + b], [x + b, y, z + b]],
        [[x + b, y, z + b], [x + b, y + h, z + b], [x + a, y + h, z + a], [x + a, y, z + a]],
        [[x + a, y, z + b], [x + a, y + h, z + b], [x + b, y + h, z + a], [x + b, y, z + a]],
        [[x + b, y, z + a], [x + b, y + h, z + a], [x + a, y + h, z + b], [x + a, y, z + b]],
    ] {
        out.quad(q, 6, c);
    }
}

/// Mesh a single object (for the object preview window), centred on its origin.
pub fn build_object(obj: &meadow_core::objects::CustomObject, rotation: u8, mirror: bool) -> MeshData {
    let v = obj.variant(rotation, mirror);
    let mut set: FxHashMap<(i32, i32, i32), StateId> = FxHashMap::default();
    for &(x, y, z, s) in &v.blocks {
        set.insert((x, y, z), s);
    }
    let mut out = MeshData::default();
    for (&(x, y, z), &s) in &set {
        let inf = s.info();
        if inf.air || inf.color[3] == 0 {
            continue;
        }
        let col = [inf.color[0], inf.color[1], inf.color[2], 255];
        let (fx, fy, fz) = (x as f32, y as f32, z as f32);
        if !inf.solid && !inf.liquid && !inf.leaves {
            cross(&mut out, fx, fy, fz, 1.0, col);
            continue;
        }
        let solid_at = |x: i32, y: i32, z: i32| set.get(&(x, y, z)).is_some_and(|s| {
            let i = s.info();
            i.solid && !i.leaves && i.color[3] == 255
        });
        let faces: [(i32, i32, i32, i16); 6] = [(0, 1, 0, 0), (0, -1, 0, 1), (1, 0, 0, 2), (-1, 0, 0, 3), (0, 0, 1, 4), (0, 0, -1, 5)];
        for (dx, dy, dz, n) in faces {
            if solid_at(x + dx, y + dy, z + dz) {
                continue;
            }
            let (a, b, c) = (fx, fy, fz);
            let q = match n {
                0 => [[a, b + 1.0, c], [a, b + 1.0, c + 1.0], [a + 1.0, b + 1.0, c + 1.0], [a + 1.0, b + 1.0, c]],
                1 => [[a, b, c], [a + 1.0, b, c], [a + 1.0, b, c + 1.0], [a, b, c + 1.0]],
                2 => [[a + 1.0, b, c], [a + 1.0, b + 1.0, c], [a + 1.0, b + 1.0, c + 1.0], [a + 1.0, b, c + 1.0]],
                3 => [[a, b, c + 1.0], [a, b + 1.0, c + 1.0], [a, b + 1.0, c], [a, b, c]],
                4 => [[a + 1.0, b, c + 1.0], [a + 1.0, b + 1.0, c + 1.0], [a, b + 1.0, c + 1.0], [a, b, c + 1.0]],
                _ => [[a, b, c], [a, b + 1.0, c], [a + 1.0, b + 1.0, c], [a + 1.0, b, c]],
            };
            out.quad(q, n, col);
        }
    }
    out
}
