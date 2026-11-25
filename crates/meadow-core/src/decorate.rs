//! Deterministic placement of objects, trees and plants. Used by the exporter (which writes the
//! blocks into chunks) and by the 3D viewport / object preview (which meshes them), so the
//! preview shows what will actually be exported.

use std::sync::Arc;

use meadow_blocks::StateId;

use crate::dimension::Dimension;
use crate::layer::{LayerDef, LayerId, LayerKind, ObjectEntry, PlantEntry, WaterMode};
use crate::noise::{hash2, hash64, unit};
use crate::objects::{CustomObject, Variant};
use crate::trees;
use crate::world::World;

/// An object instance. `x`/`z` are the map column of the object origin, `y` the block height
/// of the origin.
#[derive(Clone)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub variant: Arc<Variant>,
    pub replace_solid: bool,
    pub layer: LayerId,
}

impl Placement {
    /// Inclusive horizontal bounds (x0, z0, x1, z1).
    pub fn bounds(&self) -> (i32, i32, i32, i32) {
        (self.x + self.variant.min[0], self.z + self.variant.min[2], self.x + self.variant.max[0], self.z + self.variant.max[2])
    }
}

/// Rules shared by object and tree layers.
struct Rules<'a> {
    spacing: i32,
    density: f32,
    max_value: f32,
    rotate: bool,
    mirror: bool,
    y_offset: i32,
    y_jitter: i32,
    water: WaterMode,
    only_on: &'a [u8],
    max_slope: f32,
    replace_solid: bool,
}

fn layer_seed(world_seed: u64, id: LayerId) -> u64 {
    hash64(world_seed ^ (id.0 as u64).wrapping_mul(0x2545f4914f6cdd1d))
}

/// Object placements of one layer whose footprint intersects [x0,x1) x [z0,z1).
pub fn placements_for_layer(world: &World, dim: &Dimension, layer: &LayerDef, x0: i32, z0: i32, x1: i32, z1: i32) -> Vec<Placement> {
    match &layer.kind {
        LayerKind::CustomObjects(s) if !s.objects.is_empty() => {
            let rules = Rules {
                spacing: s.spacing.max(1) as i32,
                density: s.density,
                max_value: layer.data.max() as f32,
                rotate: s.random_rotation,
                mirror: s.random_mirror,
                y_offset: s.y_offset,
                y_jitter: s.y_jitter as i32,
                water: s.water,
                only_on: &s.only_on_terrain,
                max_slope: s.max_slope,
                replace_solid: s.replace_solid,
            };
            let radius = s.objects.iter().map(|e| e.object.radius()).max().unwrap_or(1);
            place(world, dim, layer.id, &rules, radius, x0, z0, x1, z1, |h| pick_entry(&s.objects, h))
        }
        LayerKind::Trees(kind) => {
            let (spacing, density) = trees::spacing_density(*kind);
            let vars = trees::variants(*kind);
            let water = if matches!(kind, crate::layer::TreeKind::Mangrove | crate::layer::TreeKind::Swamp) {
                WaterMode::Anywhere
            } else {
                WaterMode::DryOnly
            };
            let rules = Rules {
                spacing: spacing as i32,
                density,
                max_value: 15.0,
                rotate: true,
                mirror: true,
                y_offset: 0,
                y_jitter: 0,
                water,
                only_on: &[],
                max_slope: 60.0,
                replace_solid: false,
            };
            let radius = vars.iter().map(|o| o.radius()).max().unwrap_or(1);
            place(world, dim, layer.id, &rules, radius, x0, z0, x1, z1, |h| Some(&vars[(h % vars.len() as u64) as usize]))
        }
        _ => Vec::new(),
    }
}

fn pick_entry(list: &[ObjectEntry], h: u64) -> Option<&Arc<CustomObject>> {
    let total: u64 = list.iter().map(|e| e.weight.max(1) as u64).sum();
    if total == 0 {
        return None;
    }
    let mut t = h % total;
    for e in list {
        let w = e.weight.max(1) as u64;
        if t < w {
            return Some(&e.object);
        }
        t -= w;
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn place<'o>(
    world: &World,
    dim: &Dimension,
    layer: LayerId,
    r: &Rules<'_>,
    radius: i32,
    x0: i32,
    z0: i32,
    x1: i32,
    z1: i32,
    pick: impl Fn(u64) -> Option<&'o Arc<CustomObject>>,
) -> Vec<Placement> {
    let seed = layer_seed(world.seed, layer);
    let cell = r.spacing;
    let (cx0, cz0) = ((x0 - radius).div_euclid(cell), (z0 - radius).div_euclid(cell));
    let (cx1, cz1) = ((x1 + radius).div_euclid(cell), (z1 + radius).div_euclid(cell));
    let mut out = Vec::new();
    for cz in cz0..=cz1 {
        for cx in cx0..=cx1 {
            let h = hash2(seed, cx, cz);
            let px = cx * cell + (h % cell as u64) as i32;
            let pz = cz * cell + ((h >> 16) % cell as u64) as i32;
            let v = dim.layer_value(layer, 0, px, pz);
            if v == 0 {
                continue;
            }
            let p = r.density * (v as f32 / r.max_value).min(1.0);
            if unit(hash64(h ^ 0xd1ce)) >= p {
                continue;
            }
            let Some(height) = dim.int_height(px, pz) else { continue };
            if dim.layer_value(LayerId::VOID, 0, px, pz) != 0 {
                continue;
            }
            let under = dim.is_underwater(px, pz);
            match r.water {
                WaterMode::DryOnly if under => continue,
                WaterMode::UnderwaterOnly if !under => continue,
                _ => {}
            }
            if !r.only_on.is_empty() && !dim.terrain(px, pz).is_some_and(|t| r.only_on.contains(&t)) {
                continue;
            }
            if r.max_slope < 90.0 && dim.slope(px, pz) > r.max_slope {
                continue;
            }
            // Trees don't grow out of water/lava terrains or bare stone faces.
            if layer.0 < LayerId::FIRST_CUSTOM {
                let t = dim.terrain(px, pz).unwrap_or(0);
                if matches!(t, crate::terrain::ids::WATER | crate::terrain::ids::LAVA) {
                    continue;
                }
                let _ = world;
            }
            let Some(obj) = pick(hash64(h ^ 0x0b1)) else { continue };
            let hr = hash64(h ^ 0x707);
            let rot = if r.rotate { (hr & 3) as u8 } else { 0 };
            let mirror = r.mirror && (hr >> 2) & 1 == 1;
            let variant = obj.variant(rot, mirror);
            let jitter = if r.y_jitter > 0 { ((hr >> 8) % (r.y_jitter as u64 + 1)) as i32 } else { 0 };
            let pl = Placement { x: px, y: height + 1 + r.y_offset - jitter, z: pz, variant, replace_solid: r.replace_solid, layer };
            let (bx0, bz0, bx1, bz1) = pl.bounds();
            if bx1 >= x0 && bz1 >= z0 && bx0 < x1 && bz0 < z1 {
                out.push(pl);
            }
        }
    }
    out
}

/// All object/tree placements intersecting the rectangle, in layer priority order.
pub fn all_placements(world: &World, dim: &Dimension, x0: i32, z0: i32, x1: i32, z1: i32) -> Vec<Placement> {
    let mut layers: Vec<&LayerDef> =
        world.layers.iter().filter(|l| matches!(l.kind, LayerKind::CustomObjects(_) | LayerKind::Trees(_))).collect();
    layers.sort_by_key(|l| l.priority);
    let mut out = Vec::new();
    for l in layers {
        // Skip layers not present anywhere near the rectangle.
        let pad = 48;
        let present = (((x0 - pad) >> 7)..=((x1 + pad) >> 7))
            .any(|tx| (((z0 - pad) >> 7)..=((z1 + pad) >> 7)).any(|tz| dim.tile(tx, tz).is_some_and(|t| t.has_layer(l.id))));
        if present {
            out.extend(placements_for_layer(world, dim, l, x0, z0, x1, z1));
        }
    }
    out
}

/// A plant on a single column: (block, optional upper half, stack height).
#[derive(Clone, Copy, Debug)]
pub struct PlantPick {
    pub block: StateId,
    pub upper: Option<StateId>,
    pub height: u8,
}

fn pick_plant(list: &[PlantEntry], h: u64) -> Option<PlantPick> {
    let total: u64 = list.iter().map(|p| p.weight as u64).sum();
    if total == 0 {
        return None;
    }
    let mut t = h % total;
    for p in list {
        if t < p.weight as u64 {
            let span = (p.max_height.max(p.min_height) - p.min_height) as u64 + 1;
            let height = p.min_height.max(1) + ((h >> 20) % span) as u8;
            return Some(PlantPick { block: p.block, upper: p.upper, height });
        }
        t -= p.weight as u64;
    }
    None
}

/// The plant (if any) generated on top of column (x, z). Plants layers take precedence over the
/// terrain's own vegetation.
pub fn plant_at(world: &World, dim: &Dimension, x: i32, z: i32, plant_layers: &[&LayerDef]) -> Option<PlantPick> {
    if dim.is_underwater(x, z) {
        return None;
    }
    let terrain = dim.terrain(x, z)?;
    let h = hash2(world.seed ^ 0x91a47, x, z);
    for l in plant_layers {
        if let LayerKind::Plants(s) = &l.kind {
            let v = dim.layer_value(l.id, 0, x, z);
            if v == 0 || (!s.only_on_terrain.is_empty() && !s.only_on_terrain.contains(&terrain)) {
                continue;
            }
            let p = s.density * v as f32 / l.data.max() as f32;
            if unit(hash64(h ^ l.id.0 as u64)) < p {
                return pick_plant(&s.plants, hash64(h ^ 0x51));
            }
        }
    }
    let t = world.terrain(terrain);
    if t.plant_density > 0.0 && !t.plants.is_empty() && unit(h) < t.plant_density {
        return pick_plant(&t.plants, hash64(h ^ 0x77));
    }
    None
}

/// Plant layers of the world, ready for `plant_at`.
pub fn plant_layers(world: &World) -> Vec<&LayerDef> {
    world.layers.iter().filter(|l| matches!(l.kind, LayerKind::Plants(_))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{DataSize, ObjectLayerSettings};
    use crate::world::NewWorld;

    #[test]
    fn deterministic_and_bounded() {
        let mut w = World::new(&NewWorld { width_tiles: 2, height_tiles: 2, ..Default::default() });
        let obj = Arc::new(CustomObject::from_positions(
            "rock",
            &[(0, 0, 0, StateId::parse("stone")), (1, 0, 0, StateId::parse("stone")), (0, 1, 0, StateId::parse("cobblestone"))],
            [0, 0, 0],
        ));
        let id = w.add_layer(LayerDef {
            id: LayerId(0),
            name: "Rocks".into(),
            description: String::new(),
            kind: LayerKind::CustomObjects(ObjectLayerSettings {
                objects: vec![ObjectEntry { object: obj, weight: 1 }],
                spacing: 6,
                density: 1.0,
                ..Default::default()
            }),
            data: DataSize::Nibble,
            default: 0,
            color: [128, 128, 128],
            priority: 50,
            hidden: false,
        });
        for z in -64..64 {
            for x in -64..64 {
                w.surface_mut().set_layer_value(id, false, 0, x, z, 15);
                w.surface_mut().set_layer_value(LayerId::DECIDUOUS, false, 0, x, z, 15);
            }
        }
        let d = w.surface().snapshot();
        let a = all_placements(&w, &d, -64, -64, 64, 64);
        let b = all_placements(&w, &d, -64, -64, 64, 64);
        assert!(a.len() > 100, "{}", a.len());
        assert_eq!(a.len(), b.len());
        // Splitting the area must yield the same objects (needed for chunk-parallel export).
        let left = all_placements(&w, &d, -64, -64, 0, 64);
        let right = all_placements(&w, &d, 0, -64, 64, 64);
        let mut keys: Vec<_> = a.iter().map(|p| (p.layer, p.x, p.z)).collect();
        let mut split: Vec<_> = left.iter().chain(right.iter()).map(|p| (p.layer, p.x, p.z)).collect();
        keys.sort();
        split.sort();
        split.dedup();
        assert_eq!(keys, split);
        // Trees exist and have logs
        assert!(a.iter().any(|p| p.layer == LayerId::DECIDUOUS && p.variant.blocks.len() > 10));
    }
}
