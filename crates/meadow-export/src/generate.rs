//! Turn a dimension into blocks, one chunk at a time (chunks are independent, so the exporter
//! can run them all in parallel).

use meadow_blocks::{BlockState, StateId, StateInfo, common};
use meadow_core::decorate::{all_placements, plant_at};
use meadow_core::layer::{LayerDef, LayerId, LayerKind, TreeKind};
use meadow_core::noise::{hash2, hash3, perlin2, perlin3, unit};
use meadow_core::terrain::{Material, TerrainDef};
use meadow_core::tile::{TILE_MASK, TILE_SHIFT, local_index};
use meadow_core::{Dimension, World};
use rustc_hash::FxHashMap;

use crate::chunk::ChunkBuffer;

pub struct GenContext<'a> {
    pub world: &'a World,
    pub dim: &'a Dimension,
    plant_layers: Vec<&'a LayerDef>,
    overlays: Vec<(LayerId, StateId, u8)>,
    covers: Vec<(LayerId, StateId, u8)>,
    tree_layers: Vec<(LayerId, TreeKind)>,
    pub infos: Vec<StateInfo>,
    ores: Ores,
    /// Surface blocks that turn to dirt under water.
    grassy: Vec<StateId>,
    seed: u64,
}

struct Ores {
    coal: (StateId, StateId),
    iron: (StateId, StateId),
    copper: (StateId, StateId),
    gold: (StateId, StateId),
    redstone: (StateId, StateId),
    lapis: (StateId, StateId),
    diamond: (StateId, StateId),
    emerald: (StateId, StateId),
    gravel: StateId,
    dirt: StateId,
    lava: StateId,
}

fn ore(n: &str) -> (StateId, StateId) {
    (StateId::parse(&format!("minecraft:{n}_ore")), StateId::parse(&format!("minecraft:deepslate_{n}_ore")))
}

impl<'a> GenContext<'a> {
    pub fn new(world: &'a World, dim: &'a Dimension) -> Self {
        let mut overlays = Vec::new();
        let mut covers = Vec::new();
        let mut tree_layers = Vec::new();
        for l in &world.layers {
            match &l.kind {
                LayerKind::TerrainOverlay(s) => overlays.push((l.id, s.block, s.depth.max(1))),
                LayerKind::GroundCover(s) => covers.push((l.id, s.block, s.thickness.max(1))),
                LayerKind::Trees(k) => tree_layers.push((l.id, *k)),
                _ => {}
            }
        }
        let ores = Ores {
            coal: ore("coal"),
            iron: ore("iron"),
            copper: ore("copper"),
            gold: ore("gold"),
            redstone: ore("redstone"),
            lapis: ore("lapis"),
            diamond: ore("diamond"),
            emerald: ore("emerald"),
            gravel: common::gravel(),
            dirt: common::dirt(),
            lava: common::lava(),
        };
        // Make sure every state we may emit is interned before snapshotting infos: all
        // rotated/mirrored object and tree variants, plus the fixed states used below.
        for l in &world.layers {
            let objs: Vec<std::sync::Arc<meadow_core::objects::CustomObject>> = match &l.kind {
                LayerKind::Trees(k) => meadow_core::trees::variants(*k).to_vec(),
                LayerKind::CustomObjects(s) => s.objects.iter().map(|e| e.object.clone()).collect(),
                _ => continue,
            };
            for o in objs {
                for r in 0..4 {
                    o.variant(r, false);
                    o.variant(r, true);
                }
            }
        }
        let _ = (common::snowy_grass_block(), common::snow_layer(), common::ice(), common::lily_pad(), common::deepslate(), common::bedrock());
        GenContext {
            world,
            dim,
            plant_layers: meadow_core::decorate::plant_layers(world),
            overlays,
            covers,
            tree_layers,
            infos: meadow_blocks::state::info_snapshot(),
            ores,
            grassy: ["grass_block[snowy=false]", "grass_block[snowy=true]", "mycelium[snowy=false]", "podzol[snowy=false]", "moss_block", "pale_moss_block"]
                .iter()
                .map(|b| StateId::parse(b))
                .collect(),
            seed: world.seed,
        }
    }

    #[inline]
    fn info(&self, s: StateId) -> StateInfo {
        match self.infos.get(s.0 as usize) {
            Some(i) => *i,
            None => s.info(),
        }
    }

    /// Generate a chunk; `None` if the chunk contains no tiles.
    pub fn chunk(&self, cx: i32, cz: i32) -> Option<ChunkBuffer> {
        let dim = self.dim;
        let (x0, z0) = (cx * 16, cz * 16);
        let tile = dim.tile(x0 >> TILE_SHIFT, z0 >> TILE_SHIFT)?;
        let limits = dim.limits;
        let mut buf = ChunkBuffer::new(cx, cz, limits.min_y, limits.max_y);
        let world = self.world;
        let water_b = common::water();
        let lava_b = common::lava();
        let ice = common::ice();
        let bedrock = common::bedrock();
        let deepslate = common::deepslate();
        let sub = &dim.subsurface;
        let max_y = limits.max_y - 1;
        let mut frost_cols: Vec<(i32, i32)> = Vec::new();
        // Whole sections of plain deepslate under the entire chunk are stored as uniform sections.
        if sub.deepslate {
            let mut deep_top = 0i32;
            for lz in 0..16 {
                for lx in 0..16 {
                    let i = local_index((x0 + lx) & TILE_MASK, (z0 + lz) & TILE_MASK);
                    let t = world.terrain(tile.terrain[i]);
                    let sd: i32 = t.strata.iter().map(|s| s.depth as i32).sum::<i32>() + 16;
                    let top = ((tile.height[i] + 0.5).floor() as i32).clamp(limits.min_y, max_y);
                    if tile.layer_value(LayerId::VOID, i, 0) != 0 {
                        deep_top = limits.min_y;
                    }
                    deep_top = deep_top.min(top - sd);
                }
            }
            let from = limits.min_y + if sub.bedrock_floor { 16 } else { 0 };
            buf.fill_sections(from, deep_top.min(0), deepslate);
        }

        for lz in 0..16 {
            for lx in 0..16 {
                let (wx, wz) = (x0 + lx, z0 + lz);
                let i = local_index(wx & TILE_MASK, wz & TILE_MASK);
                let lv = |id: LayerId, def: u8| tile.layer_value(id, i, def);
                if lv(LayerId::VOID, 0) != 0 {
                    buf.biomes[(lz * 16 + lx) as usize] = 127;
                    continue;
                }
                let top = ((tile.height[i] + 0.5).floor() as i32).clamp(limits.min_y, max_y);
                let water = tile.water[i] as i32;
                let terrain: &TerrainDef = world.terrain(tile.terrain[i]);
                let frost = lv(LayerId::FROST, 0) != 0;
                let flood_lava = lv(LayerId::LAVA, 0) != 0;
                let overlay = self.overlays.iter().find(|(id, _, _)| lv(*id, 0) != 0).map(|(_, b, d)| (*b, *d as i32));

                // Solid column. Plain deepslate/stone below the strata is written with a fast
                // column fill (most of a 4064-block-tall world).
                let strata_depth: i32 = terrain.strata.iter().map(|s| s.depth as i32).sum::<i32>().max(overlay.map(|o| o.1).unwrap_or(0));
                let mut y_start = limits.min_y;
                if sub.bedrock_floor {
                    buf.set(lx, limits.min_y, lz, bedrock);
                    y_start += 1;
                }
                let plain_top = (top - strata_depth).min(if sub.deepslate { 0 } else { top - strata_depth });
                if sub.deepslate && plain_top > y_start {
                    buf.fill_column(lx, lz, y_start, plain_top, deepslate);
                    y_start = plain_top;
                } else if let Material::Block(b) = &sub.material
                    && !sub.deepslate
                    && plain_top > y_start
                {
                    buf.fill_column(lx, lz, y_start, plain_top, *b);
                    y_start = plain_top;
                }
                for y in y_start..=top {
                    let depth = top - y;
                    let b = if sub.bedrock_floor && y == limits.min_y {
                        bedrock
                    } else if let Some((ob, od)) = overlay
                        && depth < od
                    {
                        ob
                    } else if let Some(b) = terrain.block_at(self.seed, wx, wz, y, depth) {
                        if depth == 0 && water > top && self.grassy.contains(&b) { common::dirt() } else { b }
                    } else if sub.deepslate && (y < 0 || (y < 8 && unit(hash3(self.seed ^ 0xdee9, wx, y, wz)) < (8 - y) as f32 / 8.0)) {
                        deepslate
                    } else {
                        sub.material.pick(self.seed, wx, wz, y)
                    };
                    buf.set(lx, y, lz, b);
                }

                // Resources (ores) in the subsurface.
                let res = lv(LayerId::RESOURCES, 0);
                if res > 0 {
                    self.resources(&mut buf, lx, lz, wx, wz, limits.min_y + 1, top - 3, res);
                }

                // Underground carving.
                let caves = lv(LayerId::CAVES, 0);
                let caverns = lv(LayerId::CAVERNS, 0);
                let chasms = lv(LayerId::CHASMS, 0);
                if caves + caverns + chasms > 0 {
                    self.carve(&mut buf, lx, lz, wx, wz, top, water, caves, caverns, chasms);
                }

                // Water / lava above the surface.
                if water > top {
                    let fluid = if flood_lava { lava_b } else { water_b };
                    let wtop = water.min(max_y);
                    for y in top + 1..=wtop {
                        buf.set(lx, y, lz, if frost && y == wtop && !flood_lava { ice } else { fluid });
                    }
                    if !flood_lava && lv(LayerId::SWAMP, 0) > 0 && water - top <= 3 && unit(hash2(self.seed ^ 0x111, wx, wz)) < 0.06 && wtop < max_y {
                        buf.set(lx, wtop + 1, lz, common::lily_pad());
                    }
                } else {
                    // Ground cover then plants.
                    let mut y = top + 1;
                    let mut covered = false;
                    for (id, b, t) in &self.covers {
                        if lv(*id, 0) != 0 {
                            for k in 0..*t as i32 {
                                if y + k <= max_y {
                                    buf.set(lx, y + k, lz, *b);
                                }
                            }
                            y += *t as i32;
                            covered = true;
                            break;
                        }
                    }
                    if !covered
                        && !frost
                        && self.info(buf.get(lx, top, lz)).solid
                        && let Some(p) = plant_at(world, dim, wx, wz, &self.plant_layers)
                    {
                        for k in 0..p.height as i32 {
                            if y + k <= max_y {
                                buf.set(lx, y + k, lz, p.block);
                            }
                        }
                        if let Some(u) = p.upper
                            && y < max_y
                        {
                            buf.set(lx, y + 1, lz, u);
                        }
                    }
                }

                // Biome.
                let biome = lv(LayerId::BIOME, meadow_blocks::BIOME_AUTO);
                buf.biomes[(lz * 16 + lx) as usize] =
                    if biome != meadow_blocks::BIOME_AUTO { biome } else { self.auto_biome(terrain, top, water, frost, &lv) };
                if frost {
                    frost_cols.push((lx, lz));
                }
            }
        }

        // Objects and trees (including parts reaching in from neighbouring chunks).
        let placements = all_placements(world, dim, x0, z0, x0 + 16, z0 + 16);
        if !placements.is_empty() {
            let mut waterlogged: FxHashMap<StateId, StateId> = FxHashMap::default();
            for p in &placements {
                for &(bx, by, bz, s) in &p.variant.blocks {
                    let (lx, y, lz) = (p.x + bx - x0, p.y + by, p.z + bz - z0);
                    if !(0..16).contains(&lx) || !(0..16).contains(&lz) || y < limits.min_y || y > max_y {
                        continue;
                    }
                    let existing = buf.get(lx, y, lz);
                    let ei = self.info(existing);
                    if existing != StateId::AIR && ei.solid && !p.replace_solid {
                        continue;
                    }
                    let mut s = s;
                    if ei.liquid && existing == water_b {
                        s = *waterlogged.entry(s).or_insert_with(|| {
                            let st = s.state();
                            if st.prop("waterlogged").is_some() {
                                let mut ns: BlockState = (*st).clone();
                                for kv in ns.props.iter_mut() {
                                    if kv.0 == "waterlogged" {
                                        kv.1 = "true".into();
                                    }
                                }
                                StateId::of(&ns)
                            } else {
                                s
                            }
                        });
                    }
                    buf.set(lx, y, lz, s);
                }
            }
        }

        // Frost: snow on top of whatever is highest, ice on water.
        if !frost_cols.is_empty() {
            let snow = common::snow_layer();
            let grass = common::grass_block();
            let snowy_grass = common::snowy_grass_block();
            for (lx, lz) in frost_cols {
                let t = buf.top(lx, lz, |_| true);
                if t <= limits.min_y {
                    continue;
                }
                let below = buf.get(lx, t - 1, lz);
                let bi = self.info(below);
                if below == water_b {
                    buf.set(lx, t - 1, lz, ice);
                } else if !bi.solid && !bi.liquid && !bi.air && below != snow {
                    // Replace a plant by snow
                    buf.set(lx, t - 1, lz, snow);
                    if buf.get(lx, t - 2, lz) == grass {
                        buf.set(lx, t - 2, lz, snowy_grass);
                    }
                } else if (bi.solid || bi.leaves) && t <= max_y && below != snow {
                    buf.set(lx, t, lz, snow);
                    if below == grass {
                        buf.set(lx, t - 1, lz, snowy_grass);
                    }
                }
            }
        }
        Some(buf)
    }

    #[allow(clippy::too_many_arguments)]
    fn resources(&self, buf: &mut ChunkBuffer, lx: i32, lz: i32, wx: i32, wz: i32, y0: i32, y1: i32, value: u8) {
        let p = value as f32 / 15.0 * 0.025;
        let o = &self.ores;
        let ds = common::deepslate();
        let stone = common::stone();
        for y in y0..y1 {
            let h = hash3(self.seed ^ 0x0e5, wx, y, wz);
            if unit(h) >= p {
                continue;
            }
            let cur = buf.get(lx, y, lz);
            let deep = cur == ds;
            if cur != stone && !deep {
                continue;
            }
            let r = (h >> 8) % 1000;
            let pick = |pair: (StateId, StateId)| if deep { pair.1 } else { pair.0 };
            let b = match r {
                0..=299 if y > 0 => pick(o.coal),
                0..=299 => pick(o.iron),
                300..=499 => pick(o.iron),
                500..=629 => pick(o.copper),
                630..=699 if y < 32 => pick(o.gold),
                700..=779 if y < 16 => pick(o.redstone),
                780..=829 if y < 32 => pick(o.lapis),
                830..=869 if y < 16 => pick(o.diamond),
                870..=874 => pick(o.emerald),
                875..=939 => o.gravel,
                940..=989 => o.dirt,
                990..=999 if y < 10 => o.lava,
                _ => continue,
            };
            buf.set(lx, y, lz, b);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn carve(&self, buf: &mut ChunkBuffer, lx: i32, lz: i32, wx: i32, wz: i32, top: i32, water: i32, caves: u8, caverns: u8, chasms: u8) {
        let min_y = self.dim.limits.min_y;
        let (fx, fz) = (wx as f32, wz as f32);
        let flooded = water > top;
        let fill = |y: i32| -> StateId {
            if y < min_y + 8 {
                common::lava()
            } else if flooded && y <= water {
                common::water()
            } else {
                StateId::AIR
            }
        };
        let s = self.seed;
        // Chasm shape is per column.
        let chasm_w = if chasms > 0 {
            let warp = perlin2(s ^ 0xc4a5, fx / 90.0, fz / 90.0) * 30.0;
            let n = perlin2(s ^ 0xc4a6, (fx + warp) / 70.0, (fz - warp) / 70.0).abs();
            0.015 + chasms as f32 * 0.003 - n
        } else {
            -1.0
        };
        for y in (min_y + 1)..top.saturating_sub(1) {
            let fy = y as f32;
            let mut carve = false;
            if caves > 0 && y < top - 3 {
                let r = 0.03 + caves as f32 * 0.006;
                let a = perlin3(s ^ 0xca7e1, fx / 28.0, fy / 18.0, fz / 28.0);
                if a.abs() < r {
                    let b = perlin3(s ^ 0xca7e2, fx / 28.0, fy / 18.0, fz / 28.0);
                    carve = b.abs() < r;
                }
            }
            if !carve && caverns > 0 && y < top - 6 {
                let n = perlin3(s ^ 0xca7e3, fx / 56.0, fy / 28.0, fz / 56.0);
                carve = n > 0.62 - caverns as f32 * 0.025;
            }
            if !carve && chasm_w > 0.0 {
                // Narrower towards the bottom.
                let depth_frac = ((top - y) as f32 / 60.0).clamp(0.0, 1.0);
                carve = chasm_w > depth_frac * 0.012 && y > min_y + 6;
            }
            if carve && buf.get(lx, y, lz) != common::bedrock() {
                buf.set(lx, y, lz, fill(y));
            }
        }
    }

    fn auto_biome(&self, terrain: &TerrainDef, top: i32, water: i32, frost: bool, lv: &dyn Fn(LayerId, u8) -> u8) -> u8 {
        use meadow_core::DimensionKind;
        match self.dim.kind {
            DimensionKind::Nether => return self.dim.default_biome,
            DimensionKind::End => return self.dim.default_biome,
            DimensionKind::Overworld => {}
        }
        if water > top {
            let deep = water - top > 20;
            return match (deep, frost) {
                (true, true) => 50,
                (true, false) => 24,
                (false, true) => 10,
                (false, false) => {
                    if lv(LayerId::SWAMP, 0) > 0 {
                        6
                    } else if lv(LayerId::MANGROVE, 0) > 0 {
                        247
                    } else {
                        0
                    }
                }
            };
        }
        for (id, kind) in &self.tree_layers {
            if lv(*id, 0) > 0 {
                return match kind {
                    TreeKind::Deciduous => 4,
                    TreeKind::Pine if frost => 30,
                    TreeKind::Pine => 5,
                    TreeKind::Jungle => 21,
                    TreeKind::Swamp => 6,
                    TreeKind::Mangrove => 247,
                    TreeKind::Cherry => 246,
                    TreeKind::PaleOak => 245,
                };
            }
        }
        if frost && matches!(terrain.auto_biome, 1 | 4 | 129) {
            return 12;
        }
        terrain.auto_biome
    }
}
