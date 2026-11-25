//! 128x128-column tiles: the unit of storage, undo, parallel export and render caching.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::layer::LayerId;

pub const TILE_SHIFT: i32 = 7;
pub const TILE_SIZE: i32 = 1 << TILE_SHIFT;
pub const TILE_MASK: i32 = TILE_SIZE - 1;
pub const TILE_AREA: usize = (TILE_SIZE * TILE_SIZE) as usize;

static REV: AtomicU64 = AtomicU64::new(1);

/// Monotonic revision counter used by caches (2D map textures, 3D meshes) to detect changes.
pub fn next_rev() -> u64 {
    REV.fetch_add(1, Ordering::Relaxed)
}

#[inline]
pub fn local_index(lx: i32, ly: i32) -> usize {
    ((ly << TILE_SHIFT) | lx) as usize
}

/// Per-column layer storage. Nibble layers use one byte per column for speed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum LayerData {
    Bits(Vec<u64>),
    Bytes(Vec<u8>),
}

impl LayerData {
    pub fn new_bits() -> Self {
        LayerData::Bits(vec![0; TILE_AREA / 64])
    }
    pub fn new_bytes(fill: u8) -> Self {
        LayerData::Bytes(vec![fill; TILE_AREA])
    }
    #[inline]
    pub fn get(&self, i: usize) -> u8 {
        match self {
            LayerData::Bits(b) => ((b[i >> 6] >> (i & 63)) & 1) as u8,
            LayerData::Bytes(b) => b[i],
        }
    }
    #[inline]
    pub fn set(&mut self, i: usize, v: u8) {
        match self {
            LayerData::Bits(b) => {
                if v != 0 {
                    b[i >> 6] |= 1 << (i & 63);
                } else {
                    b[i >> 6] &= !(1 << (i & 63));
                }
            }
            LayerData::Bytes(b) => b[i] = v,
        }
    }
    pub fn is_uniform(&self, v: u8) -> bool {
        match self {
            LayerData::Bits(b) => b.iter().all(|w| if v != 0 { *w == u64::MAX } else { *w == 0 }),
            LayerData::Bytes(b) => b.iter().all(|x| *x == v),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tile {
    pub tx: i32,
    pub ty: i32,
    /// Surface height per column (fractional; the exported surface block is `round(height)`).
    pub height: Vec<f32>,
    /// Terrain index per column (see `terrain::TerrainId`).
    pub terrain: Vec<u8>,
    /// Water (or lava) surface level per column. Water is placed when this exceeds the height.
    pub water: Vec<i16>,
    /// Layer data, sorted by layer id.
    pub layers: Vec<(LayerId, LayerData)>,
    #[serde(skip, default = "next_rev")]
    pub rev: u64,
}

impl Tile {
    pub fn new(tx: i32, ty: i32, height: f32, terrain: u8, water: i16) -> Self {
        Tile {
            tx,
            ty,
            height: vec![height; TILE_AREA],
            terrain: vec![terrain; TILE_AREA],
            water: vec![water; TILE_AREA],
            layers: Vec::new(),
            rev: next_rev(),
        }
    }

    #[inline]
    pub fn touch(&mut self) {
        self.rev = next_rev();
    }

    pub fn origin(&self) -> (i32, i32) {
        (self.tx << TILE_SHIFT, self.ty << TILE_SHIFT)
    }

    pub fn layer(&self, id: LayerId) -> Option<&LayerData> {
        self.layers.iter().find(|(l, _)| *l == id).map(|(_, d)| d)
    }

    /// Get mutable layer data, creating it (filled with `default`) if absent.
    pub fn layer_mut(&mut self, id: LayerId, bits: bool, default: u8) -> &mut LayerData {
        self.touch();
        let pos = match self.layers.binary_search_by_key(&id, |(l, _)| *l) {
            Ok(p) => p,
            Err(p) => {
                let d = if bits { LayerData::new_bits() } else { LayerData::new_bytes(default) };
                self.layers.insert(p, (id, d));
                p
            }
        };
        &mut self.layers[pos].1
    }

    #[inline]
    pub fn layer_value(&self, id: LayerId, i: usize, default: u8) -> u8 {
        self.layer(id).map(|d| d.get(i)).unwrap_or(default)
    }

    /// Drop layer data that only holds the default value (keeps saves and exports lean).
    pub fn compact_layers(&mut self, default_of: impl Fn(LayerId) -> u8) {
        self.layers.retain(|(id, d)| !d.is_uniform(default_of(*id)));
    }

    pub fn has_layer(&self, id: LayerId) -> bool {
        self.layer(id).is_some()
    }

    pub fn max_height(&self) -> f32 {
        self.height.iter().copied().fold(f32::MIN, f32::max)
    }
    pub fn min_height(&self) -> f32 {
        self.height.iter().copied().fold(f32::MAX, f32::min)
    }
}
