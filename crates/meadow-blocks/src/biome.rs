//! Biomes. Meadow stores biomes per column as WorldPainter-compatible numeric IDs (so old
//! scripts that paint biome numbers keep working); 255 means "automatic".

use std::sync::OnceLock;

const BIOMES_TSV: &str = include_str!("../../../data/vanilla_biomes.tsv");

pub const BIOME_AUTO: u8 = 255;

/// WorldPainter's legacy numeric biome IDs (index) mapped to modern IDs. Several legacy IDs map to
/// the same modern biome; the first index is the canonical one.
const LEGACY: &[(u8, &str)] = &[
    (0, "ocean"), (1, "plains"), (2, "desert"), (3, "windswept_hills"), (4, "forest"), (5, "taiga"),
    (6, "swamp"), (7, "river"), (8, "nether_wastes"), (9, "the_end"), (10, "frozen_ocean"),
    (11, "frozen_river"), (12, "snowy_plains"), (13, "snowy_plains"), (14, "mushroom_fields"),
    (15, "mushroom_fields"), (16, "beach"), (17, "desert"), (18, "forest"), (19, "taiga"),
    (20, "windswept_hills"), (21, "jungle"), (22, "jungle"), (23, "sparse_jungle"), (24, "deep_ocean"),
    (25, "stony_shore"), (26, "snowy_beach"), (27, "birch_forest"), (28, "birch_forest"),
    (29, "dark_forest"), (30, "snowy_taiga"), (31, "snowy_taiga"), (32, "old_growth_pine_taiga"),
    (33, "old_growth_pine_taiga"), (34, "windswept_forest"), (35, "savanna"), (36, "savanna_plateau"),
    (37, "badlands"), (38, "wooded_badlands"), (39, "badlands"), (40, "small_end_islands"),
    (41, "end_midlands"), (42, "end_highlands"), (43, "end_barrens"), (44, "warm_ocean"),
    (45, "lukewarm_ocean"), (46, "cold_ocean"), (47, "warm_ocean"), (48, "deep_lukewarm_ocean"),
    (49, "deep_cold_ocean"), (50, "deep_frozen_ocean"), (127, "the_void"), (129, "sunflower_plains"),
    (130, "desert"), (131, "windswept_gravelly_hills"), (132, "flower_forest"), (133, "taiga"),
    (134, "swamp"), (140, "ice_spikes"), (149, "jungle"), (151, "sparse_jungle"),
    (155, "old_growth_birch_forest"), (156, "old_growth_birch_forest"), (157, "dark_forest"),
    (158, "snowy_taiga"), (160, "old_growth_spruce_taiga"), (161, "old_growth_spruce_taiga"),
    (162, "windswept_gravelly_hills"), (163, "windswept_savanna"), (164, "windswept_savanna"),
    (165, "eroded_badlands"), (166, "wooded_badlands"), (167, "badlands"), (168, "bamboo_jungle"),
    (169, "bamboo_jungle"), (170, "soul_sand_valley"), (171, "crimson_forest"), (172, "warped_forest"),
    (173, "basalt_deltas"), (174, "dripstone_caves"), (175, "lush_caves"), (245, "pale_garden"),
    (246, "cherry_grove"), (247, "mangrove_swamp"), (248, "deep_dark"), (249, "frozen_peaks"),
    (250, "grove"), (251, "jagged_peaks"), (252, "meadow"), (253, "snowy_slopes"), (254, "stony_peaks"),
];

#[derive(Debug, Clone)]
pub struct BiomeInfo {
    pub id: String,
    pub name: String,
    pub dimension: String,
    pub color: [u8; 3],
    /// Canonical numeric ID used in the Biome layer.
    pub numeric: u8,
}

pub struct Biomes {
    pub list: Vec<BiomeInfo>,
    by_numeric: Vec<Option<usize>>,
}

impl Biomes {
    pub fn by_numeric(&self, n: u8) -> Option<&BiomeInfo> {
        self.by_numeric.get(n as usize).copied().flatten().map(|i| &self.list[i])
    }
    pub fn by_id(&self, id: &str) -> Option<&BiomeInfo> {
        let id = id.strip_prefix("minecraft:").unwrap_or(id);
        self.list.iter().find(|b| b.id.strip_prefix("minecraft:") == Some(id))
    }
}

pub fn biomes() -> &'static Biomes {
    static B: OnceLock<Biomes> = OnceLock::new();
    B.get_or_init(|| {
        let mut list = Vec::new();
        let mut by_numeric = vec![None; 256];
        let mut next_free: u8 = 244;
        for line in BIOMES_TSV.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let c: Vec<&str> = line.split('\t').collect();
            let path = c[0];
            let rgb = u32::from_str_radix(c.get(3).unwrap_or(&"808080"), 16).unwrap_or(0x808080);
            let canonical = LEGACY.iter().find(|(_, n)| *n == path).map(|(i, _)| *i);
            let numeric = canonical.unwrap_or_else(|| {
                while by_numeric[next_free as usize].is_some() || LEGACY.iter().any(|(i, _)| *i == next_free) {
                    next_free -= 1;
                }
                next_free
            });
            let idx = list.len();
            list.push(BiomeInfo {
                id: format!("minecraft:{path}"),
                name: crate::registry::title_case(path),
                dimension: c.get(1).unwrap_or(&"overworld").to_string(),
                color: [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8],
                numeric,
            });
            by_numeric[numeric as usize] = Some(idx);
        }
        // Alias legacy IDs that map to an existing modern biome.
        for (n, path) in LEGACY {
            if by_numeric[*n as usize].is_none()
                && let Some(i) = list.iter().position(|b| b.id.ends_with(&format!(":{path}")))
            {
                by_numeric[*n as usize] = Some(i);
            }
        }
        Biomes { list, by_numeric }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_ids() {
        let b = biomes();
        assert_eq!(b.by_numeric(1).unwrap().id, "minecraft:plains");
        assert_eq!(b.by_numeric(4).unwrap().id, "minecraft:forest");
        assert_eq!(b.by_numeric(13).unwrap().id, "minecraft:snowy_plains");
        assert_eq!(b.by_numeric(246).unwrap().id, "minecraft:cherry_grove");
        assert_eq!(b.by_id("pale_garden").unwrap().numeric, 245);
        assert!(b.by_numeric(BIOME_AUTO).is_none());
        assert!(b.list.len() >= 64);
    }
}
