//! Terrain types. Indices follow WorldPainter's `Terrain.VALUES` order so that scripts using
//! numeric terrain indices (`toTerrain(36)` = Beaches) keep working. Meadow-only terrains use
//! indices from 162 upward.

use meadow_blocks::{StateId, common};
use serde::{Deserialize, Serialize};

use crate::layer::PlantEntry;
use crate::noise::{hash3, perlin3, unit};

pub type TerrainId = u8;

pub mod ids {
    pub const GRASS: u8 = 0;
    pub const BARE_GRASS: u8 = 1;
    pub const DIRT: u8 = 2;
    pub const PERMADIRT: u8 = 3;
    pub const PODZOL: u8 = 4;
    pub const SAND: u8 = 5;
    pub const RED_SAND: u8 = 6;
    pub const DESERT: u8 = 7;
    pub const RED_DESERT: u8 = 8;
    pub const MESA: u8 = 9;
    pub const HARDENED_CLAY: u8 = 10;
    pub const SANDSTONE: u8 = 27;
    pub const STONE: u8 = 28;
    pub const ROCK: u8 = 29;
    pub const COBBLESTONE: u8 = 30;
    pub const MOSSY_COBBLESTONE: u8 = 31;
    pub const OBSIDIAN: u8 = 32;
    pub const BEDROCK: u8 = 33;
    pub const GRAVEL: u8 = 34;
    pub const CLAY: u8 = 35;
    pub const BEACHES: u8 = 36;
    pub const WATER: u8 = 37;
    pub const LAVA: u8 = 38;
    pub const SNOW: u8 = 39;
    pub const DEEP_SNOW: u8 = 40;
    pub const NETHERRACK: u8 = 41;
    pub const SOUL_SAND: u8 = 42;
    pub const NETHERLIKE: u8 = 43;
    pub const MYCELIUM: u8 = 44;
    pub const END_STONE: u8 = 45;
    pub const RESOURCES: u8 = 46;
    pub const RED_SANDSTONE: u8 = 71;
    pub const GRANITE: u8 = 72;
    pub const DIORITE: u8 = 73;
    pub const ANDESITE: u8 = 74;
    pub const STONE_MIX: u8 = 75;
    pub const GRASS_PATH: u8 = 100;
    pub const MAGMA: u8 = 101;
    pub const DEEPSLATE: u8 = 150;
    pub const TUFF: u8 = 151;
    pub const BASALT: u8 = 152;
    pub const BLACKSTONE: u8 = 153;
    pub const SOUL_SOIL: u8 = 154;
    pub const WARPED_NYLIUM: u8 = 155;
    pub const CRIMSON_NYLIUM: u8 = 156;
    pub const CALCITE: u8 = 157;
    pub const MUD: u8 = 158;
    pub const BARE_BEACHES: u8 = 159;
    pub const MOSS: u8 = 160;
    pub const PALE_MOSS: u8 = 161;
    // Meadow additions (blocks from recent updates)
    pub const LEAF_LITTER: u8 = 162;
    pub const DRY_GRASSLAND: u8 = 163;
    pub const WILDFLOWER_MEADOW: u8 = 164;
    pub const ROOTED_DIRT: u8 = 165;
    pub const PACKED_MUD: u8 = 166;
    pub const SNOWY_GRASS: u8 = 167;
}

/// Map a WorldPainter custom-terrain slot (0..96) to its terrain index.
pub fn custom_slot_index(slot: usize) -> Option<u8> {
    match slot {
        0..=23 => Some((slot + 47) as u8),
        24..=47 => Some((slot + 52) as u8),
        48..=95 => Some((slot + 54) as u8),
        _ => None,
    }
}

pub fn is_custom_index(id: u8) -> bool {
    matches!(id, 47..=70 | 76..=99 | 102..=149)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Material {
    Block(StateId),
    /// Per-block weighted random mix.
    Mix(Vec<(StateId, u16)>),
    /// Mix in noise-shaped patches of roughly `scale` blocks.
    Patches { blocks: Vec<(StateId, u16)>, scale: f32 },
    /// Horizontal bands by absolute Y (mesa-style).
    Bands { blocks: Vec<StateId>, thickness: u8 },
}

impl Material {
    #[inline]
    pub fn pick(&self, seed: u64, x: i32, y: i32, z: i32) -> StateId {
        match self {
            Material::Block(b) => *b,
            Material::Mix(list) => pick_weighted(list, unit(hash3(seed, x, y, z))),
            Material::Patches { blocks, scale } => {
                let n = perlin3(seed ^ 0x7a7c, x as f32 / scale, z as f32 / scale, y as f32 / scale);
                pick_weighted(blocks, ((n * 0.9 + 1.0) * 0.5).clamp(0.0, 0.9999))
            }
            Material::Bands { blocks, thickness } => {
                if blocks.is_empty() {
                    return StateId::AIR;
                }
                let t = (*thickness).max(1) as i32;
                // Slight per-column waviness so bands are not perfectly flat.
                let wobble = (perlin3(seed ^ 0xbad, x as f32 / 48.0, 0.0, y as f32 / 48.0) * 3.0) as i32;
                blocks[((z + wobble).div_euclid(t)).rem_euclid(blocks.len() as i32) as usize]
            }
        }
    }

    /// Representative colour for the 2D map.
    pub fn color(&self) -> [u8; 3] {
        let avg = |list: &[(StateId, u16)]| {
            let tot: u32 = list.iter().map(|(_, w)| *w as u32).sum::<u32>().max(1);
            let mut c = [0u32; 3];
            for (b, w) in list {
                let bc = b.info().color;
                for i in 0..3 {
                    c[i] += bc[i] as u32 * *w as u32;
                }
            }
            [(c[0] / tot) as u8, (c[1] / tot) as u8, (c[2] / tot) as u8]
        };
        match self {
            Material::Block(b) => {
                let c = b.info().color;
                [c[0], c[1], c[2]]
            }
            Material::Mix(l) | Material::Patches { blocks: l, .. } => avg(l),
            Material::Bands { blocks, .. } => avg(&blocks.iter().map(|b| (*b, 1)).collect::<Vec<_>>()),
        }
    }

    pub fn blocks(&self) -> Vec<StateId> {
        match self {
            Material::Block(b) => vec![*b],
            Material::Mix(l) | Material::Patches { blocks: l, .. } => l.iter().map(|x| x.0).collect(),
            Material::Bands { blocks, .. } => blocks.clone(),
        }
    }
}

#[inline]
fn pick_weighted(list: &[(StateId, u16)], r: f32) -> StateId {
    let total: u32 = list.iter().map(|(_, w)| *w as u32).sum();
    if total == 0 {
        return StateId::AIR;
    }
    let mut t = (r * total as f32) as u32;
    for (b, w) in list {
        if t < *w as u32 {
            return *b;
        }
        t -= *w as u32;
    }
    list.last().unwrap().0
}

/// One stratum of a terrain, from the surface down.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Stratum {
    /// Thickness in blocks.
    pub depth: u16,
    pub material: Material,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerrainDef {
    pub id: TerrainId,
    pub name: String,
    /// Strata from the top. Below them the dimension's subsurface material is used.
    pub strata: Vec<Stratum>,
    /// Plants placed on top of the surface.
    pub plants: Vec<PlantEntry>,
    /// Chance per column that a plant is placed.
    pub plant_density: f32,
    /// Biome used when the biome is automatic (WorldPainter numeric ID).
    pub auto_biome: u8,
    pub color: [u8; 3],
    pub custom: bool,
}

impl TerrainDef {
    /// Block for a column at `depth` below the surface (0 = surface block). `None` means use
    /// the subsurface material.
    #[inline]
    pub fn block_at(&self, seed: u64, x: i32, y: i32, z: i32, depth: i32) -> Option<StateId> {
        let mut d = depth;
        for s in &self.strata {
            if d < s.depth as i32 {
                return Some(s.material.pick(seed, x, y, z));
            }
            d -= s.depth as i32;
        }
        None
    }

    pub fn surface_color(&self) -> [u8; 3] {
        self.strata.first().map(|s| s.material.color()).unwrap_or([128, 128, 128])
    }

    pub fn new_custom(id: TerrainId, name: &str, top: Material, depth: u16) -> TerrainDef {
        let color = top.color();
        TerrainDef {
            id,
            name: name.into(),
            strata: vec![Stratum { depth, material: top }],
            plants: Vec::new(),
            plant_density: 0.0,
            auto_biome: 1,
            color,
            custom: true,
        }
    }
}

fn s(b: &str) -> StateId {
    StateId::parse(b)
}

fn block(b: &str) -> Material {
    Material::Block(s(b))
}

fn strata(list: &[(u16, Material)]) -> Vec<Stratum> {
    list.iter().map(|(d, m)| Stratum { depth: *d, material: m.clone() }).collect()
}

fn plain(id: u8, name: &str, st: Vec<Stratum>, biome: u8) -> TerrainDef {
    let mut t = TerrainDef {
        id,
        name: name.into(),
        strata: st,
        plants: Vec::new(),
        plant_density: 0.0,
        auto_biome: biome,
        color: [0; 3],
        custom: false,
    };
    t.color = t.surface_color();
    t
}

fn grassy_plants() -> Vec<PlantEntry> {
    vec![
        PlantEntry::single(common::short_grass(), 120),
        PlantEntry::tall(s("tall_grass[half=lower]"), s("tall_grass[half=upper]"), 10),
        PlantEntry::single(s("dandelion"), 6),
        PlantEntry::single(s("poppy"), 6),
        PlantEntry::single(s("azure_bluet"), 3),
        PlantEntry::single(s("oxeye_daisy"), 3),
        PlantEntry::single(s("cornflower"), 2),
    ]
}

/// All built-in terrains (index = id).
pub fn builtin_terrains() -> Vec<TerrainDef> {
    use ids::*;
    let mut v: Vec<TerrainDef> = Vec::new();
    let dirt3 = (3u16, block("dirt"));
    let mut put = |t: TerrainDef| v.push(t);

    let mut grass = plain(GRASS, "Grass", strata(&[(1, block("grass_block[snowy=false]")), dirt3.clone()]), 1);
    grass.plants = grassy_plants();
    grass.plant_density = 0.18;
    put(grass);
    put(plain(BARE_GRASS, "Bare Grass", strata(&[(1, block("grass_block[snowy=false]")), dirt3.clone()]), 1));
    put(plain(DIRT, "Dirt", strata(&[(4, block("dirt"))]), 1));
    put(plain(PERMADIRT, "Permadirt", strata(&[(4, block("coarse_dirt"))]), 1));
    let mut podzol = plain(PODZOL, "Podzol", strata(&[(1, block("podzol[snowy=false]")), dirt3.clone()]), 32);
    podzol.plants = vec![PlantEntry::single(s("fern"), 10), PlantEntry::tall(s("large_fern[half=lower]"), s("large_fern[half=upper]"), 2)];
    podzol.plant_density = 0.06;
    put(podzol);
    put(plain(SAND, "Sand", strata(&[(4, block("sand")), (3, block("sandstone"))]), 16));
    put(plain(RED_SAND, "Red Sand", strata(&[(4, block("red_sand")), (3, block("red_sandstone"))]), 37));
    let mut desert = plain(DESERT, "Desert", strata(&[(4, block("sand")), (3, block("sandstone"))]), 2);
    desert.plants = vec![
        PlantEntry::single(s("dead_bush"), 10),
        PlantEntry { block: s("cactus[age=0]"), upper: None, weight: 3, min_height: 1, max_height: 3 },
        PlantEntry::single(s("short_dry_grass"), 6),
    ];
    desert.plant_density = 0.01;
    put(desert);
    let mut rdesert = plain(RED_DESERT, "Red Desert", strata(&[(4, block("red_sand")), (3, block("red_sandstone"))]), 37);
    rdesert.plants = vec![PlantEntry::single(s("dead_bush"), 10)];
    rdesert.plant_density = 0.01;
    put(rdesert);
    let bands = ["terracotta", "orange_terracotta", "yellow_terracotta", "terracotta", "brown_terracotta", "red_terracotta", "terracotta", "white_terracotta", "light_gray_terracotta", "orange_terracotta"]
        .iter()
        .map(|b| s(b))
        .collect();
    put(plain(MESA, "Mesa", strata(&[(1, block("red_sand")), (40, Material::Bands { blocks: bands, thickness: 2 })]), 37));
    put(plain(HARDENED_CLAY, "Hardened Clay", strata(&[(6, block("terracotta"))]), 37));
    let colours = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"];
    for (i, c) in colours.iter().enumerate() {
        let name = format!("{} Stained Clay", meadow_blocks::registry::title_case(c));
        put(plain(11 + i as u8, &name, strata(&[(6, block(&format!("{c}_terracotta")))]), 37));
    }
    put(plain(SANDSTONE, "Sandstone", strata(&[(6, block("sandstone"))]), 2));
    put(plain(STONE, "Stone", strata(&[(1, block("stone"))]), 25));
    put(plain(ROCK, "Rock", strata(&[(4, Material::Patches { blocks: vec![(s("stone"), 70), (s("cobblestone"), 30)], scale: 6.0 })]), 25));
    put(plain(COBBLESTONE, "Cobblestone", strata(&[(4, block("cobblestone"))]), 25));
    put(plain(MOSSY_COBBLESTONE, "Mossy Cobblestone", strata(&[(4, block("mossy_cobblestone"))]), 25));
    put(plain(OBSIDIAN, "Obsidian", strata(&[(4, block("obsidian"))]), 25));
    put(plain(BEDROCK, "Bedrock", strata(&[(4, block("bedrock"))]), 25));
    put(plain(GRAVEL, "Gravel", strata(&[(4, block("gravel"))]), 131));
    put(plain(CLAY, "Clay", strata(&[(4, block("clay"))]), 7));
    put(plain(BEACHES, "Beaches", strata(&[(4, Material::Patches { blocks: vec![(s("sand"), 75), (s("gravel"), 17), (s("clay"), 8)], scale: 5.0 }), (2, block("sandstone"))]), 16));
    put(plain(WATER, "Water", strata(&[(1, block("water[level=0]")), (3, block("dirt"))]), 0));
    put(plain(LAVA, "Lava", strata(&[(1, block("lava[level=0]")), (3, block("stone"))]), 1));
    let mut snow = plain(SNOW, "Snow", strata(&[(1, block("grass_block[snowy=true]")), dirt3.clone()]), 12);
    snow.plants = vec![PlantEntry::single(s("snow[layers=1]"), 1)];
    snow.plant_density = 1.0;
    snow.color = [0xf4, 0xf8, 0xf8];
    put(snow);
    put(plain(DEEP_SNOW, "Deep Snow", strata(&[(4, block("snow_block")), dirt3.clone()]), 12));
    put(plain(NETHERRACK, "Netherrack", strata(&[(4, block("netherrack"))]), 8));
    put(plain(SOUL_SAND, "Soul Sand", strata(&[(4, block("soul_sand"))]), 170));
    put(plain(
        NETHERLIKE,
        "Netherlike",
        strata(&[(6, Material::Patches { blocks: vec![(s("netherrack"), 80), (s("soul_sand"), 8), (s("magma_block"), 4), (s("nether_quartz_ore"), 5), (s("glowstone"), 3)], scale: 4.0 })]),
        8,
    ));
    let mut myc = plain(MYCELIUM, "Mycelium", strata(&[(1, block("mycelium[snowy=false]")), dirt3.clone()]), 14);
    myc.plants = vec![PlantEntry::single(s("brown_mushroom"), 1), PlantEntry::single(s("red_mushroom"), 1)];
    myc.plant_density = 0.01;
    put(myc);
    put(plain(END_STONE, "End Stone", strata(&[(4, block("end_stone"))]), 9));
    put(plain(
        RESOURCES,
        "Resources",
        strata(&[(6, Material::Mix(vec![(s("stone"), 90), (s("coal_ore"), 5), (s("iron_ore"), 3), (s("gold_ore"), 1), (s("copper_ore"), 1)]))]),
        25,
    ));
    // Custom slots 47..=70 are filled below.
    v.truncate(47);
    let mut v = fill_custom(v, 71);
    let mut put = |t: TerrainDef| v.push(t);
    put(plain(RED_SANDSTONE, "Red Sandstone", strata(&[(6, block("red_sandstone"))]), 37));
    put(plain(GRANITE, "Granite", strata(&[(4, block("granite"))]), 25));
    put(plain(DIORITE, "Diorite", strata(&[(4, block("diorite"))]), 25));
    put(plain(ANDESITE, "Andesite", strata(&[(4, block("andesite"))]), 25));
    put(plain(
        STONE_MIX,
        "Stone Mix",
        strata(&[(6, Material::Patches { blocks: vec![(s("stone"), 55), (s("granite"), 15), (s("diorite"), 15), (s("andesite"), 15)], scale: 7.0 })]),
        25,
    ));
    let mut v = fill_custom(v, 100);
    let mut put = |t: TerrainDef| v.push(t);
    put(plain(GRASS_PATH, "Grass Path", strata(&[(1, block("dirt_path")), (3, block("dirt"))]), 1));
    put(plain(MAGMA, "Magma", strata(&[(4, block("magma_block"))]), 8));
    let mut v = fill_custom(v, 150);
    let mut put = |t: TerrainDef| v.push(t);
    put(plain(DEEPSLATE, "Deepslate", strata(&[(4, block("deepslate[axis=y]"))]), 25));
    put(plain(TUFF, "Tuff", strata(&[(4, block("tuff"))]), 25));
    put(plain(BASALT, "Basalt", strata(&[(4, block("basalt[axis=y]"))]), 173));
    put(plain(BLACKSTONE, "Blackstone", strata(&[(4, block("blackstone"))]), 173));
    put(plain(SOUL_SOIL, "Soul Soil", strata(&[(4, block("soul_soil"))]), 170));
    put(plain(WARPED_NYLIUM, "Warped Nylium", strata(&[(1, block("warped_nylium")), (3, block("netherrack"))]), 172));
    put(plain(CRIMSON_NYLIUM, "Crimson Nylium", strata(&[(1, block("crimson_nylium")), (3, block("netherrack"))]), 171));
    put(plain(CALCITE, "Calcite", strata(&[(4, block("calcite"))]), 254));
    put(plain(MUD, "Mud", strata(&[(4, block("mud"))]), 247));
    put(plain(BARE_BEACHES, "Bare Beaches", strata(&[(4, block("sand")), (2, block("sandstone"))]), 16));
    put(plain(MOSS, "Moss", strata(&[(1, block("moss_block")), dirt3.clone()]), 175));
    let mut pale = plain(PALE_MOSS, "Pale Moss", strata(&[(1, block("pale_moss_block")), dirt3.clone()]), 245);
    pale.plants = vec![PlantEntry::single(s("pale_moss_carpet[bottom=true,east=none,north=none,south=none,west=none]"), 1)];
    pale.plant_density = 0.25;
    put(pale);
    let mut litter = plain(LEAF_LITTER, "Leaf Litter", strata(&[(1, block("grass_block[snowy=false]")), dirt3.clone()]), 4);
    litter.plants = vec![
        PlantEntry::single(s("leaf_litter[facing=north,segment_amount=4]"), 6),
        PlantEntry::single(s("leaf_litter[facing=east,segment_amount=2]"), 4),
        PlantEntry::single(common::short_grass(), 3),
        PlantEntry::single(s("bush"), 1),
    ];
    litter.plant_density = 0.45;
    litter.color = [0x8f, 0x84, 0x45];
    put(litter);
    let mut dry = plain(DRY_GRASSLAND, "Dry Grassland", strata(&[(1, block("grass_block[snowy=false]")), dirt3.clone()]), 35);
    dry.plants = vec![
        PlantEntry::single(s("short_dry_grass"), 10),
        PlantEntry::single(s("tall_dry_grass"), 4),
        PlantEntry::single(s("dead_bush"), 1),
    ];
    dry.plant_density = 0.3;
    dry.color = [0xa8, 0xa4, 0x5c];
    put(dry);
    let mut wild = plain(WILDFLOWER_MEADOW, "Wildflower Meadow", strata(&[(1, block("grass_block[snowy=false]")), dirt3.clone()]), 252);
    wild.plants = vec![
        PlantEntry::single(s("wildflowers[facing=north,flower_amount=4]"), 8),
        PlantEntry::single(s("wildflowers[facing=south,flower_amount=2]"), 5),
        PlantEntry::single(common::short_grass(), 10),
        PlantEntry::single(s("cornflower"), 1),
        PlantEntry::single(s("golden_dandelion"), 1),
    ];
    wild.plant_density = 0.4;
    wild.color = [0x9c, 0xbd, 0x5a];
    put(wild);
    put(plain(ROOTED_DIRT, "Rooted Dirt", strata(&[(3, Material::Patches { blocks: vec![(s("rooted_dirt"), 60), (s("dirt"), 25), (s("coarse_dirt"), 15)], scale: 4.0 })]), 175));
    put(plain(PACKED_MUD, "Packed Mud", strata(&[(4, block("packed_mud"))]), 247));
    put(plain(SNOWY_GRASS, "Snowy Grass", strata(&[(1, block("grass_block[snowy=true]")), dirt3]), 12));
    v
}

fn fill_custom(mut v: Vec<TerrainDef>, until: usize) -> Vec<TerrainDef> {
    while v.len() < until {
        let id = v.len() as u8;
        let mut t = plain(id, &format!("Custom {}", custom_number(id)), strata(&[(1, block("stone"))]), 1);
        t.custom = true;
        v.push(t);
    }
    v
}

fn custom_number(id: u8) -> usize {
    (0..96).find(|s| custom_slot_index(*s) == Some(id)).map(|s| s + 1).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indices_match_worldpainter() {
        let t = builtin_terrains();
        for (i, d) in t.iter().enumerate() {
            assert_eq!(d.id as usize, i, "terrain {} has wrong id", d.name);
        }
        assert_eq!(t[0].name, "Grass");
        assert_eq!(t[3].name, "Permadirt");
        assert_eq!(t[29].name, "Rock");
        assert_eq!(t[36].name, "Beaches");
        assert_eq!(t[47].name, "Custom 1");
        assert_eq!(t[76].name, "Custom 25");
        assert_eq!(t[102].name, "Custom 49");
        assert_eq!(t[160].name, "Moss");
        assert_eq!(t[ids::SNOWY_GRASS as usize].name, "Snowy Grass");
        assert!(is_custom_index(70) && !is_custom_index(71));
    }
}
