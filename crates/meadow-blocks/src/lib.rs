//! Block, biome and version knowledge for Meadow.
//!
//! * Vanilla blocks and biomes are generated from minecraft-data (up to Minecraft 26.1), with the
//!   first version each block appeared in, so exports can be checked against the target version.
//! * Conquest Reforged's ~14k block IDs are built in (namespace `conquest`).
//! * Any mod jar (or the vanilla client jar) can be imported for exact block states and colours.

pub mod biome;
pub mod color;
pub mod modjar;
pub mod registry;
pub mod state;
mod version;

pub use biome::{BIOME_AUTO, BiomeInfo, biomes};
pub use registry::{BlockInfo, BlockSource, PropDef, normalize_id, registry};
pub use state::{BlockState, StateId, StateInfo, common};
pub use version::{BuildLimits, McVersion};

/// Map a modern block ID to the name used by an older target version (for the few blocks that
/// were renamed), or `None` if the block does not exist there and needs a fallback.
pub fn name_for_version(id: &str, version: McVersion) -> Option<std::borrow::Cow<'_, str>> {
    use std::borrow::Cow;
    match id {
        "minecraft:short_grass" if version < McVersion::V1_20_4 => return Some(Cow::Borrowed("minecraft:grass")),
        "minecraft:iron_chain" if version < McVersion::V1_21_10 => return Some(Cow::Borrowed("minecraft:chain")),
        _ => {}
    }
    if registry().read().available_in(id, version) { Some(Cow::Borrowed(id)) } else { None }
}

/// Reasonable stand-in for a block missing from an older version.
pub fn fallback_block(path: &str) -> &'static str {
    if path.contains("pale_oak") || path.contains("cherry") {
        if path.contains("leaves") { "minecraft:oak_leaves" } else if path.contains("log") { "minecraft:birch_log" } else { "minecraft:birch_planks" }
    } else if path.contains("moss") {
        "minecraft:moss_block"
    } else if path.contains("copper") {
        "minecraft:cut_copper"
    } else if path.contains("tuff") {
        "minecraft:stone"
    } else if path.contains("resin") {
        "minecraft:orange_terracotta"
    } else if crate::registry::is_plant_like(path) || path.contains("grass") || path.contains("leaf_litter") {
        "minecraft:air"
    } else {
        "minecraft:stone"
    }
}
