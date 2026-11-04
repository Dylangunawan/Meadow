//! Interned block states. World generation works on cheap `StateId`s; the full name and
//! properties are only looked up when writing palettes.

use std::fmt;
use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::registry::{normalize_id, registry};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockState {
    /// Full ID with namespace.
    pub name: String,
    /// Sorted by property name.
    pub props: Vec<(String, String)>,
}

impl BlockState {
    pub fn new(name: &str, props: &[(&str, &str)]) -> Self {
        let mut props: Vec<(String, String)> = props.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        props.sort();
        BlockState { name: normalize_id(name), props }
    }
    pub fn path(&self) -> &str {
        self.name.split_once(':').map(|(_, p)| p).unwrap_or(&self.name)
    }
    pub fn prop(&self, k: &str) -> Option<&str> {
        self.props.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
    /// Parse `namespace:path[key=value,...]`.
    pub fn parse(s: &str) -> Option<BlockState> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let (name, rest) = match s.find('[') {
            Some(i) => (&s[..i], Some(&s[i + 1..])),
            None => (s, None),
        };
        let mut props = Vec::new();
        if let Some(rest) = rest {
            let rest = rest.strip_suffix(']')?;
            for kv in rest.split(',').filter(|x| !x.trim().is_empty()) {
                let (k, v) = kv.split_once('=')?;
                props.push((k.trim().to_owned(), v.trim().to_owned()));
            }
        }
        props.sort();
        Some(BlockState { name: normalize_id(name), props })
    }
}

impl fmt::Display for BlockState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        if !self.props.is_empty() {
            f.write_str("[")?;
            for (i, (k, v)) in self.props.iter().enumerate() {
                if i > 0 {
                    f.write_str(",")?;
                }
                write!(f, "{k}={v}")?;
            }
            f.write_str("]")?;
        }
        Ok(())
    }
}

/// Per-state data needed on hot paths.
#[derive(Debug, Clone, Copy)]
pub struct StateInfo {
    pub color: [u8; 4],
    pub air: bool,
    pub solid: bool,
    pub transparent: bool,
    pub liquid: bool,
    pub leaves: bool,
    pub light: u8,
}

/// Interned block state handle. `StateId::AIR` is always 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct StateId(pub u32);

impl StateId {
    pub const AIR: StateId = StateId(0);

    pub fn of(s: &BlockState) -> StateId {
        intern(s)
    }
    /// Parse and intern; unknown syntax yields air.
    pub fn parse(s: &str) -> StateId {
        BlockState::parse(s).map(|b| intern(&b)).unwrap_or(StateId::AIR)
    }
    pub fn state(self) -> Arc<BlockState> {
        table().read().states[self.0 as usize].clone()
    }
    pub fn info(self) -> StateInfo {
        table().read().info[self.0 as usize]
    }
    pub fn is_air(self) -> bool {
        self.0 == 0 || self.info().air
    }
}

impl Serialize for StateId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.state().to_string())
    }
}

impl<'de> Deserialize<'de> for StateId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(StateId::parse(&s))
    }
}

pub struct StateTable {
    pub states: Vec<Arc<BlockState>>,
    pub info: Vec<StateInfo>,
    map: FxHashMap<Arc<BlockState>, u32>,
}

static TABLE: OnceLock<RwLock<StateTable>> = OnceLock::new();

pub fn table() -> &'static RwLock<StateTable> {
    TABLE.get_or_init(|| {
        let mut t = StateTable { states: Vec::new(), info: Vec::new(), map: FxHashMap::default() };
        let air = Arc::new(BlockState::new("minecraft:air", &[]));
        t.map.insert(air.clone(), 0);
        t.states.push(air);
        t.info.push(StateInfo { color: [0; 4], air: true, solid: false, transparent: true, liquid: false, leaves: false, light: 0 });
        RwLock::new(t)
    })
}

/// Snapshot of all per-state infos, for lock-free lookups in tight loops.
pub fn info_snapshot() -> Vec<StateInfo> {
    table().read().info.clone()
}

pub fn intern(s: &BlockState) -> StateId {
    if let Some(&id) = table().read().map.get(s) {
        return StateId(id);
    }
    let info = compute_info(s);
    let mut t = table().write();
    if let Some(&id) = t.map.get(s) {
        return StateId(id);
    }
    let id = t.states.len() as u32;
    let arc = Arc::new(s.clone());
    t.map.insert(arc.clone(), id);
    t.states.push(arc);
    t.info.push(info);
    StateId(id)
}

/// Re-derive cached colours (after importing textures from a jar).
pub fn refresh_infos() {
    let states: Vec<Arc<BlockState>> = table().read().states.clone();
    let infos: Vec<StateInfo> = states.iter().map(|s| compute_info(s)).collect();
    table().write().info = infos;
}

/// Override the colour of a state for this session (used by tests and custom colour maps).
pub fn set_color(id: StateId, color: [u8; 4]) {
    if let Some(i) = table().write().info.get_mut(id.0 as usize) {
        i.color = color;
    }
}

fn compute_info(s: &BlockState) -> StateInfo {
    let path = s.path();
    let air = matches!(path, "air" | "cave_air" | "void_air");
    let liquid = matches!(path, "water" | "lava" | "bubble_column");
    let reg = registry().read();
    let (mut color, solid, transparent, light) = match reg.get(&s.name) {
        Some(b) => (b.color, !b.non_solid && !air, b.transparent, b.light),
        None => (crate::color::guess_color(path), !air && !crate::registry::is_plant_like(path), false, 0),
    };
    // State-dependent tweaks
    if path == "grass_block" && s.prop("snowy") == Some("true") {
        color = [0xf4, 0xf8, 0xf8, 255];
    }
    if s.prop("waterlogged") == Some("true") && color[3] < 255 {
        color = [0x3f, 0x76, 0xe4, 200];
    }
    let leaves = path.contains("leaves") || path.ends_with("_hedge");
    StateInfo { color, air, solid: solid && !liquid, transparent: transparent || liquid, liquid, leaves, light }
}

/// Commonly used states, interned once.
pub mod common {
    use super::*;

    macro_rules! states {
        ($($fn_name:ident => $s:expr),* $(,)?) => {
            $(
                pub fn $fn_name() -> StateId {
                    static ID: OnceLock<StateId> = OnceLock::new();
                    *ID.get_or_init(|| StateId::parse($s))
                }
            )*
        };
    }

    states! {
        stone => "minecraft:stone",
        deepslate => "minecraft:deepslate[axis=y]",
        bedrock => "minecraft:bedrock",
        dirt => "minecraft:dirt",
        grass_block => "minecraft:grass_block[snowy=false]",
        snowy_grass_block => "minecraft:grass_block[snowy=true]",
        sand => "minecraft:sand",
        gravel => "minecraft:gravel",
        water => "minecraft:water[level=0]",
        lava => "minecraft:lava[level=0]",
        ice => "minecraft:ice",
        snow_layer => "minecraft:snow[layers=1]",
        snow_block => "minecraft:snow_block",
        oak_log => "minecraft:oak_log[axis=y]",
        oak_leaves => "minecraft:oak_leaves[distance=7,persistent=true,waterlogged=false]",
        birch_log => "minecraft:birch_log[axis=y]",
        birch_leaves => "minecraft:birch_leaves[distance=7,persistent=true,waterlogged=false]",
        spruce_log => "minecraft:spruce_log[axis=y]",
        spruce_leaves => "minecraft:spruce_leaves[distance=7,persistent=true,waterlogged=false]",
        jungle_log => "minecraft:jungle_log[axis=y]",
        jungle_leaves => "minecraft:jungle_leaves[distance=7,persistent=true,waterlogged=false]",
        dark_oak_log => "minecraft:dark_oak_log[axis=y]",
        dark_oak_leaves => "minecraft:dark_oak_leaves[distance=7,persistent=true,waterlogged=false]",
        mangrove_log => "minecraft:mangrove_log[axis=y]",
        mangrove_leaves => "minecraft:mangrove_leaves[distance=7,persistent=true,waterlogged=false]",
        vine => "minecraft:vine[east=false,north=false,south=false,up=true,west=false]",
        short_grass => "minecraft:short_grass",
        lily_pad => "minecraft:lily_pad",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning() {
        let a = StateId::parse("oak_log[axis=y]");
        let b = StateId::parse("minecraft:oak_log[axis=y]");
        assert_eq!(a, b);
        assert_ne!(a, StateId::AIR);
        assert_eq!(a.state().to_string(), "minecraft:oak_log[axis=y]");
        assert!(StateId::parse("minecraft:air").is_air());
        let s = BlockState::parse("minecraft:oak_stairs[waterlogged=false,facing=east]").unwrap();
        assert_eq!(s.props[0].0, "facing");
        assert!(common::water().info().liquid);
        assert!(!common::short_grass().info().solid);
    }
}
