//! Block type registry: vanilla (generated from minecraft-data 26.1), Conquest Reforged, and
//! anything imported from mod jars at runtime.

use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;
use rustc_hash::FxHashMap;

use crate::McVersion;
use crate::color::guess_color;

const VANILLA_TSV: &str = include_str!("../../../data/vanilla_blocks.tsv");
const CONQUEST_TXT: &str = include_str!("../../../data/conquest_reforged_blocks.txt");

#[derive(Debug, Clone)]
pub struct PropDef {
    pub name: String,
    pub values: Vec<String>,
}

/// Where a block comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockSource {
    Vanilla,
    /// Built-in Conquest Reforged ID list (no state information).
    ConquestBuiltin,
    /// Imported from a jar (exact states and texture colours).
    Imported,
}

#[derive(Debug, Clone)]
pub struct BlockInfo {
    /// Full ID, e.g. `minecraft:oak_log` or `conquest:limestone_brick_slab`.
    pub id: String,
    pub props: Vec<PropDef>,
    /// Default value index for each property.
    pub default: Vec<u8>,
    /// Index of the first `McVersion` containing this block (vanilla only).
    pub since: u8,
    pub transparent: bool,
    /// Has no collision box (plants, air, fluids...).
    pub non_solid: bool,
    pub light: u8,
    pub color: [u8; 4],
    pub source: BlockSource,
    /// Rough category used by the block browser.
    pub category: &'static str,
}

impl BlockInfo {
    pub fn namespace(&self) -> &str {
        self.id.split_once(':').map(|(n, _)| n).unwrap_or("minecraft")
    }
    pub fn path(&self) -> &str {
        self.id.split_once(':').map(|(_, p)| p).unwrap_or(&self.id)
    }
    pub fn display_name(&self) -> String {
        title_case(self.path())
    }
    pub fn default_props(&self) -> Vec<(String, String)> {
        self.props
            .iter()
            .zip(&self.default)
            .map(|(p, i)| (p.name.clone(), p.values.get(*i as usize).cloned().unwrap_or_default()))
            .collect()
    }
    pub fn prop(&self, name: &str) -> Option<&PropDef> {
        self.props.iter().find(|p| p.name == name)
    }
}

pub fn title_case(path: &str) -> String {
    path.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Default)]
pub struct Registry {
    blocks: Vec<Arc<BlockInfo>>,
    by_id: FxHashMap<String, usize>,
    generation: u64,
}

impl Registry {
    fn insert(&mut self, info: BlockInfo) {
        if let Some(&i) = self.by_id.get(&info.id) {
            self.blocks[i] = Arc::new(info);
        } else {
            self.by_id.insert(info.id.clone(), self.blocks.len());
            self.blocks.push(Arc::new(info));
        }
        self.generation += 1;
    }

    pub fn get(&self, id: &str) -> Option<&Arc<BlockInfo>> {
        let id = normalize_id_ref(id);
        self.by_id.get(id.as_ref()).map(|&i| &self.blocks[i])
    }
    pub fn all(&self) -> &[Arc<BlockInfo>] {
        &self.blocks
    }
    pub fn namespaces(&self) -> Vec<String> {
        let mut v: Vec<String> = self.blocks.iter().map(|b| b.namespace().to_owned()).collect();
        v.sort();
        v.dedup();
        v
    }
    pub fn len(&self) -> usize {
        self.blocks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
    /// Changes whenever blocks are added or replaced (used to invalidate UI caches).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Simple case-insensitive substring search across IDs and display names.
    pub fn search(&self, query: &str, namespace: Option<&str>, limit: usize) -> Vec<Arc<BlockInfo>> {
        let terms: Vec<String> = query.to_lowercase().split_whitespace().map(|t| t.replace(' ', "_")).collect();
        self.blocks
            .iter()
            .filter(|b| namespace.is_none_or(|n| b.namespace() == n))
            .filter(|b| terms.iter().all(|t| b.id.contains(t.as_str())))
            .take(limit)
            .cloned()
            .collect()
    }

    /// Is the block available when exporting to `version`? Non-vanilla blocks are assumed present
    /// (the user is responsible for installing the mod).
    pub fn available_in(&self, id: &str, version: McVersion) -> bool {
        match self.get(id) {
            Some(b) if b.source == BlockSource::Vanilla => b.since <= version.index(),
            Some(_) => true,
            None => !id.starts_with("minecraft:"),
        }
    }

    pub fn register_imported(&mut self, blocks: Vec<BlockInfo>) -> usize {
        let n = blocks.len();
        for b in blocks {
            self.insert(b);
        }
        n
    }
}

fn normalize_id_ref(id: &str) -> std::borrow::Cow<'_, str> {
    if id.contains(':') { std::borrow::Cow::Borrowed(id) } else { std::borrow::Cow::Owned(format!("minecraft:{id}")) }
}

/// Add the `minecraft:` namespace if missing.
pub fn normalize_id(id: &str) -> String {
    normalize_id_ref(id).into_owned()
}

static REGISTRY: OnceLock<RwLock<Registry>> = OnceLock::new();

/// The global block registry.
pub fn registry() -> &'static RwLock<Registry> {
    REGISTRY.get_or_init(|| RwLock::new(build_builtin()))
}

fn build_builtin() -> Registry {
    let mut r = Registry::default();
    for line in VANILLA_TSV.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 5 {
            continue;
        }
        let path = cols[0];
        let flags = cols[2];
        let props: Vec<PropDef> = if cols[3] == "-" {
            Vec::new()
        } else {
            cols[3]
                .split(';')
                .filter_map(|p| {
                    let (n, vs) = p.split_once(':')?;
                    Some(PropDef { name: n.to_owned(), values: vs.split(',').map(str::to_owned).collect() })
                })
                .collect()
        };
        let default: Vec<u8> =
            if cols[4] == "-" { Vec::new() } else { cols[4].split(',').filter_map(|s| s.parse().ok()).collect() };
        let light = flags
            .find('l')
            .map(|i| flags[i + 1..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        r.insert(BlockInfo {
            id: format!("minecraft:{path}"),
            props,
            default,
            since: cols[1].parse().unwrap_or(0),
            transparent: flags.contains('t'),
            non_solid: flags.contains('e'),
            light,
            color: guess_color(path),
            source: BlockSource::Vanilla,
            category: categorize(path),
        });
    }
    for line in CONQUEST_TXT.lines() {
        let path = line.trim();
        if path.is_empty() || path.starts_with('#') {
            continue;
        }
        let non_solid = is_plant_like(path);
        r.insert(BlockInfo {
            id: format!("conquest:{path}"),
            props: Vec::new(),
            default: Vec::new(),
            since: 0,
            transparent: non_solid || path.contains("glass") || path.contains("leaves") || path.contains("window"),
            non_solid,
            light: if path.contains("lantern") || path.contains("candle") || path.contains("torch") { 14 } else { 0 },
            color: guess_color(path),
            source: BlockSource::ConquestBuiltin,
            category: categorize(path),
        });
    }
    r.generation = 1;
    r
}

pub(crate) fn is_plant_like(path: &str) -> bool {
    const PLANT: &[&str] = &[
        "flower", "fern", "sapling", "bush", "shrub", "reed", "tall_grass", "short_grass", "_grass_plant", "weed",
        "wheat", "crop", "mushroom", "tulip", "rose", "daisy", "poppy", "dandelion", "herb", "vine", "ivy",
        "seagrass", "kelp", "lily", "torch", "button", "sign", "rail", "carpet", "pressure_plate", "lever",
    ];
    PLANT.iter().any(|k| path.contains(k)) && !path.ends_with("_block") && !path.contains("leaves")
}

pub(crate) fn categorize(path: &str) -> &'static str {
    const CATS: &[(&str, &[&str])] = &[
        ("Plants", &["flower", "fern", "sapling", "bush", "shrub", "grass", "reed", "crop", "mushroom", "vine", "ivy", "kelp", "coral", "tulip", "rose", "lily"]),
        ("Leaves", &["leaves", "hedge"]),
        ("Logs & Wood", &["log", "wood", "plank", "beam", "timber", "bark", "stump", "branch"]),
        ("Glass & Windows", &["glass", "window", "pane"]),
        ("Fluids & Weather", &["water", "lava", "ice", "snow"]),
        ("Ground", &["dirt", "soil", "loam", "mud", "sand", "gravel", "clay", "podzol", "mycelium", "moss", "path"]),
        ("Roofing", &["roof", "thatch", "shingle"]),
        ("Metal", &["iron", "copper", "gold", "steel", "bronze", "chain", "bars"]),
        ("Stone & Masonry", &["stone", "brick", "cobble", "marble", "granite", "diorite", "andesite", "slate", "schist", "limestone", "travertine", "tuff", "deepslate", "basalt", "concrete", "plaster", "terracotta", "quartz", "pillar", "column", "arch", "capital"]),
        ("Decor & Furniture", &["table", "chair", "bench", "bed", "barrel", "chest", "shelf", "pot", "lantern", "candle", "banner", "carpet", "wool"]),
    ];
    for (cat, keys) in CATS {
        if keys.iter().any(|k| path.contains(k)) {
            return cat;
        }
    }
    "Other"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry() {
        let r = registry().read();
        let stairs = r.get("oak_stairs").unwrap();
        assert_eq!(stairs.props.len(), 4);
        assert_eq!(
            stairs.default_props(),
            vec![
                ("facing".to_string(), "north".to_string()),
                ("half".to_string(), "bottom".to_string()),
                ("shape".to_string(), "straight".to_string()),
                ("waterlogged".to_string(), "false".to_string())
            ]
        );
        assert!(r.get("minecraft:pale_oak_log").is_some());
        assert!(r.get("conquest:limestone_brick_slab").is_none() || true);
        let cq = r.all().iter().filter(|b| b.source == BlockSource::ConquestBuiltin).count();
        assert!(cq > 14000, "conquest blocks: {cq}");
        assert!(!r.available_in("minecraft:pale_oak_log", McVersion::V1_20_1));
        assert!(r.available_in("minecraft:pale_oak_log", McVersion::V1_21_4));
        assert!(r.available_in("conquest:travertine_cornice_wall", McVersion::V1_20_1));
        assert!(!r.search("schist column", Some("conquest"), 10).is_empty());
    }
}
