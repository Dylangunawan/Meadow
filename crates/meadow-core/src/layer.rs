//! Layers: per-column data painted on top of terrain (forests, frost, caves, biomes, custom
//! objects...). Built-in layer IDs are stable; custom layers start at `FIRST_CUSTOM`.

use std::sync::Arc;

use meadow_blocks::StateId;
use serde::{Deserialize, Serialize};

use crate::objects::CustomObject;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LayerId(pub u16);

impl LayerId {
    pub const FROST: LayerId = LayerId(1);
    pub const CAVES: LayerId = LayerId(2);
    pub const CAVERNS: LayerId = LayerId(3);
    pub const CHASMS: LayerId = LayerId(4);
    pub const DECIDUOUS: LayerId = LayerId(5);
    pub const PINE: LayerId = LayerId(6);
    pub const SWAMP: LayerId = LayerId(7);
    pub const JUNGLE: LayerId = LayerId(8);
    pub const VOID: LayerId = LayerId(9);
    pub const RESOURCES: LayerId = LayerId(10);
    pub const BIOME: LayerId = LayerId(11);
    pub const POPULATE: LayerId = LayerId(12);
    pub const READ_ONLY: LayerId = LayerId(13);
    pub const ANNOTATIONS: LayerId = LayerId(14);
    pub const LAVA: LayerId = LayerId(15);
    pub const MANGROVE: LayerId = LayerId(16);
    pub const CHERRY: LayerId = LayerId(17);
    pub const PALE_GARDEN: LayerId = LayerId(18);
    pub const FIRST_CUSTOM: u16 = 100;

    pub fn is_custom(self) -> bool {
        self.0 >= Self::FIRST_CUSTOM
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataSize {
    /// On/off.
    Bit,
    /// 0..=15 intensity.
    Nibble,
    /// 0..=255 (biomes, annotations).
    Byte,
}

impl DataSize {
    pub fn max(self) -> u8 {
        match self {
            DataSize::Bit => 1,
            DataSize::Nibble => 15,
            DataSize::Byte => 255,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TreeKind {
    Deciduous,
    Pine,
    Swamp,
    Jungle,
    Mangrove,
    Cherry,
    PaleOak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WaterMode {
    /// Don't place where the surface is under water.
    #[default]
    DryOnly,
    /// Only place under water (coral, kelp, ruins...).
    UnderwaterOnly,
    Anywhere,
}

/// One object in an object layer, with its relative frequency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectEntry {
    pub object: Arc<CustomObject>,
    pub weight: u16,
}

/// Settings for a "custom objects" layer (WorldPainter's bo2/schematic layer).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectLayerSettings {
    pub objects: Vec<ObjectEntry>,
    /// Grid cell size in blocks; at most one object per cell. Controls minimum spacing.
    pub spacing: u16,
    /// Probability (0..1) that a cell at full layer intensity gets an object.
    pub density: f32,
    pub random_rotation: bool,
    pub random_mirror: bool,
    /// Vertical offset from the block above the surface (negative sinks objects into the ground).
    pub y_offset: i32,
    /// Random extra vertical offset range [0, n].
    pub y_jitter: u16,
    pub water: WaterMode,
    /// Only place on these terrains (empty = any).
    pub only_on_terrain: Vec<u8>,
    /// Object blocks replace solid terrain (true) or only air/plants/water (false).
    pub replace_solid: bool,
    /// Skip placement on slopes steeper than this (degrees, 90 = no limit).
    pub max_slope: f32,
}

impl Default for ObjectLayerSettings {
    fn default() -> Self {
        ObjectLayerSettings {
            objects: Vec::new(),
            spacing: 8,
            density: 0.5,
            random_rotation: true,
            random_mirror: true,
            y_offset: 0,
            y_jitter: 0,
            water: WaterMode::DryOnly,
            only_on_terrain: Vec::new(),
            replace_solid: false,
            max_slope: 90.0,
        }
    }
}

/// A plant or small decoration placed on single columns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlantEntry {
    pub block: StateId,
    /// Upper half for two-tall plants (tall grass, sunflowers...).
    pub upper: Option<StateId>,
    pub weight: u16,
    /// Stack height range (cactus, sugar cane, bamboo).
    pub min_height: u8,
    pub max_height: u8,
}

impl PlantEntry {
    pub fn single(block: StateId, weight: u16) -> Self {
        PlantEntry { block, upper: None, weight, min_height: 1, max_height: 1 }
    }
    pub fn tall(lower: StateId, upper: StateId, weight: u16) -> Self {
        PlantEntry { block: lower, upper: Some(upper), weight, min_height: 1, max_height: 1 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlantLayerSettings {
    pub plants: Vec<PlantEntry>,
    /// Chance per column at full intensity.
    pub density: f32,
    pub only_on_terrain: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundCoverSettings {
    /// Blocks placed on top of the surface, bottom first.
    pub block: StateId,
    pub thickness: u8,
    /// Use a noisy edge rather than a hard one.
    pub noisy_edge: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerrainOverlaySettings {
    /// Replace the terrain's surface blocks with this material where painted.
    pub block: StateId,
    pub depth: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LayerKind {
    Frost,
    Caves,
    Caverns,
    Chasms,
    Trees(TreeKind),
    Void,
    Resources,
    Biome,
    Populate,
    ReadOnly,
    Annotations,
    FloodWithLava,
    CustomObjects(ObjectLayerSettings),
    Plants(PlantLayerSettings),
    GroundCover(GroundCoverSettings),
    TerrainOverlay(TerrainOverlaySettings),
    /// Hand-painted mask data used by custom masks; never exported.
    PaintedMask,
}

impl LayerKind {
    /// Whether the layer affects exported chunks.
    pub fn is_exported(&self) -> bool {
        !matches!(self, LayerKind::Annotations | LayerKind::PaintedMask | LayerKind::ReadOnly | LayerKind::Populate)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerDef {
    pub id: LayerId,
    pub name: String,
    pub description: String,
    pub kind: LayerKind,
    pub data: DataSize,
    pub default: u8,
    /// Colour used to draw the layer on the 2D map.
    pub color: [u8; 3],
    /// Export order: lower first.
    pub priority: i32,
    pub hidden: bool,
}

impl LayerDef {
    pub fn is_bits(&self) -> bool {
        self.data == DataSize::Bit
    }

    /// Name aliases understood by scripts (WorldPainter names).
    pub fn matches_name(&self, n: &str) -> bool {
        let n = n.trim();
        if self.name.eq_ignore_ascii_case(n) {
            return true;
        }
        let aliases: &[&str] = match self.id {
            LayerId::DECIDUOUS => &["Deciduous", "Deciduous forest", "org.pepsoft.Deciduous"],
            LayerId::PINE => &["Pine", "Pine forest", "org.pepsoft.Pine"],
            LayerId::SWAMP => &["Swamp", "Swamp land", "Swampland"],
            LayerId::BIOME => &["Biome", "Biomes"],
            LayerId::CAVES => &["org.pepsoft.Caves"],
            LayerId::CHASMS => &["org.pepsoft.Chasms"],
            LayerId::LAVA => &["Lava", "FloodWithLava", "Flood with lava"],
            LayerId::READ_ONLY => &["ReadOnly", "Read only"],
            LayerId::ANNOTATIONS => &["org.pepsoft.Annotations"],
            _ => &[],
        };
        aliases.iter().any(|a| a.eq_ignore_ascii_case(n))
    }
}

fn def(id: LayerId, name: &str, desc: &str, kind: LayerKind, data: DataSize, color: u32, priority: i32) -> LayerDef {
    LayerDef {
        id,
        name: name.into(),
        description: desc.into(),
        kind,
        data,
        default: if id == LayerId::BIOME { meadow_blocks::BIOME_AUTO } else { 0 },
        color: [(color >> 16) as u8, (color >> 8) as u8, color as u8],
        priority,
        hidden: false,
    }
}

/// The built-in layers present in every world.
pub fn builtin_layers() -> Vec<LayerDef> {
    use DataSize::*;
    use LayerKind as K;
    vec![
        def(LayerId::FROST, "Frost", "Cover the ground with snow and turn water to ice", K::Frost, Bit, 0xffffff, 60),
        def(LayerId::CAVES, "Caves", "Underground tunnel-like caves of varying size", K::Caves, Nibble, 0x6e5a46, 23),
        def(LayerId::CAVERNS, "Caverns", "Large underground caverns", K::Caverns, Nibble, 0x8c6e50, 20),
        def(LayerId::CHASMS, "Chasms", "Ravine-like chasms", K::Chasms, Nibble, 0x5a3c28, 21),
        def(LayerId::DECIDUOUS, "Deciduous", "A deciduous (oak and birch) forest", K::Trees(TreeKind::Deciduous), Nibble, 0x2e8b22, 40),
        def(LayerId::PINE, "Pine", "A pine (spruce) forest", K::Trees(TreeKind::Pine), Nibble, 0x1f5a2a, 41),
        def(LayerId::SWAMP, "Swamp", "Swamp land with vines and lily pads", K::Trees(TreeKind::Swamp), Nibble, 0x4c6b2a, 42),
        def(LayerId::JUNGLE, "Jungle", "Dense jungle", K::Trees(TreeKind::Jungle), Nibble, 0x2fa52f, 43),
        def(LayerId::MANGROVE, "Mangroves", "Mangrove trees with roots", K::Trees(TreeKind::Mangrove), Nibble, 0x5d3a2a, 44),
        def(LayerId::CHERRY, "Cherry grove", "Cherry blossom trees", K::Trees(TreeKind::Cherry), Nibble, 0xf0a8c8, 45),
        def(LayerId::PALE_GARDEN, "Pale garden", "Pale oak trees with hanging moss", K::Trees(TreeKind::PaleOak), Nibble, 0xb8bcb4, 46),
        def(LayerId::VOID, "Void", "Just the long drop into nothingness", K::Void, Bit, 0x000000, 0),
        def(LayerId::RESOURCES, "Resources", "Underground ores, gravel, dirt and pockets", K::Resources, Nibble, 0xc0a040, 10),
        def(LayerId::BIOME, "Biome", "Biome to set (automatic when not painted)", K::Biome, Byte, 0xff00ff, 70),
        def(LayerId::POPULATE, "Populate", "Let Minecraft decorate these chunks", K::Populate, Bit, 0x808000, 0),
        def(LayerId::READ_ONLY, "Read Only", "Chunks that will not be changed when merging", K::ReadOnly, Bit, 0x808080, 90),
        def(LayerId::ANNOTATIONS, "Annotations", "Coloured notes; not exported", K::Annotations, Byte, 0xff8000, 65),
        def(LayerId::LAVA, "Lava", "Flood with lava instead of water", K::FloodWithLava, Bit, 0xff6000, 0),
    ]
}
