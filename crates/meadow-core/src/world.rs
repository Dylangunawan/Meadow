//! The world: dimensions plus everything that is shared between them (layers, custom terrains,
//! masks, export settings). Saved as a zstd-compressed bincode file (`.meadow`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use meadow_blocks::{BuildLimits, McVersion};
use serde::{Deserialize, Serialize};

use crate::dimension::{Dimension, DimensionKind};
use crate::layer::{LayerDef, LayerId, builtin_layers};
use crate::mask::CustomMask;
use crate::terrain::{TerrainDef, builtin_terrains};
use crate::tile::{TILE_SIZE, Tile};

const MAGIC: &[u8; 8] = b"MEADOW\x00\x01";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Generator {
    /// Nothing outside the painted area.
    #[default]
    Void,
    /// Vanilla superflat grass outside the painted area.
    Flat,
    /// Minecraft's normal generator outside the painted area.
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GameMode {
    Survival,
    #[default]
    Creative,
    Adventure,
    Spectator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSettings {
    pub version: McVersion,
    pub generator: Generator,
    pub game_mode: GameMode,
    pub allow_cheats: bool,
    /// Path of a Higher Heights (or other) data pack zip to bundle with the export.
    pub datapack: Option<PathBuf>,
    /// Generate Meadow's own height data pack when the build limits are not vanilla.
    pub generate_height_datapack: bool,
    pub last_directory: Option<PathBuf>,
    /// Blocks missing from the target version are replaced instead of exported as-is.
    pub replace_unavailable_blocks: bool,
    pub export_dimensions: [bool; 3],
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings {
            version: McVersion::LATEST,
            generator: Generator::Void,
            game_mode: GameMode::Creative,
            allow_cheats: true,
            datapack: None,
            generate_height_datapack: true,
            last_directory: None,
            replace_unavailable_blocks: true,
            export_dimensions: [true, true, true],
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct World {
    pub name: String,
    pub seed: u64,
    pub limits: BuildLimits,
    pub dimensions: Vec<Dimension>,
    pub layers: Vec<LayerDef>,
    pub terrains: Vec<TerrainDef>,
    pub masks: Vec<CustomMask>,
    pub export: ExportSettings,
    pub spawn: (i32, i32),
    #[serde(skip)]
    pub path: Option<PathBuf>,
    #[serde(skip)]
    pub dirty: bool,
}

/// Parameters for creating a new world.
#[derive(Debug, Clone)]
pub struct NewWorld {
    pub name: String,
    pub seed: u64,
    pub limits: BuildLimits,
    /// Size in tiles (128 blocks each).
    pub width_tiles: i32,
    pub height_tiles: i32,
    pub base_height: f32,
    pub water_level: i32,
    pub terrain: u8,
    /// Add gentle noise hills of this amplitude (0 = flat).
    pub hills: f32,
}

impl Default for NewWorld {
    fn default() -> Self {
        NewWorld {
            name: "New World".into(),
            seed: 0x5eed,
            limits: BuildLimits::VANILLA,
            width_tiles: 4,
            height_tiles: 4,
            base_height: 64.0,
            water_level: 62,
            terrain: 0,
            hills: 0.0,
        }
    }
}

impl World {
    pub fn new(p: &NewWorld) -> World {
        let limits = p.limits.sanitized();
        let mut surface = Dimension::new(DimensionKind::Overworld, limits);
        surface.default_water_level = p.water_level;
        surface.default_terrain = p.terrain;
        let x0 = -(p.width_tiles / 2);
        let y0 = -(p.height_tiles / 2);
        for ty in y0..y0 + p.height_tiles {
            for tx in x0..x0 + p.width_tiles {
                let mut t = Tile::new(tx, ty, p.base_height, p.terrain, p.water_level as i16);
                if p.hills > 0.0 {
                    let (ox, oy) = t.origin();
                    for ly in 0..TILE_SIZE {
                        for lx in 0..TILE_SIZE {
                            let (x, y) = ((ox + lx) as f32, (oy + ly) as f32);
                            let n = crate::noise::fbm2(p.seed, x / 256.0, y / 256.0, 5);
                            t.height[crate::tile::local_index(lx, ly)] =
                                (p.base_height + n * p.hills).clamp(limits.min_y as f32, (limits.max_y - 1) as f32);
                        }
                    }
                }
                surface.add_tile(t);
            }
        }
        surface.clear_history();
        World {
            name: p.name.clone(),
            seed: p.seed,
            limits,
            dimensions: vec![surface],
            layers: builtin_layers(),
            terrains: builtin_terrains(),
            masks: Vec::new(),
            export: ExportSettings::default(),
            spawn: (0, 0),
            path: None,
            dirty: false,
        }
    }

    pub fn dimension(&self, kind: DimensionKind) -> Option<&Dimension> {
        self.dimensions.iter().find(|d| d.kind == kind)
    }
    pub fn dimension_mut(&mut self, kind: DimensionKind) -> Option<&mut Dimension> {
        self.dimensions.iter_mut().find(|d| d.kind == kind)
    }
    pub fn surface(&self) -> &Dimension {
        self.dimension(DimensionKind::Overworld).expect("world has no surface dimension")
    }
    pub fn surface_mut(&mut self) -> &mut Dimension {
        self.dimension_mut(DimensionKind::Overworld).expect("world has no surface dimension")
    }

    pub fn layer(&self, id: LayerId) -> Option<&LayerDef> {
        self.layers.iter().find(|l| l.id == id)
    }
    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut LayerDef> {
        self.layers.iter_mut().find(|l| l.id == id)
    }
    pub fn layer_by_name(&self, name: &str) -> Option<&LayerDef> {
        self.layers.iter().find(|l| l.matches_name(name))
    }
    pub fn layer_default(&self, id: LayerId) -> u8 {
        self.layer(id).map(|l| l.default).unwrap_or(0)
    }

    /// Add a custom layer and return its id.
    pub fn add_layer(&mut self, mut def: LayerDef) -> LayerId {
        let next = self.layers.iter().map(|l| l.id.0).filter(|i| *i >= LayerId::FIRST_CUSTOM).max().map(|m| m + 1);
        def.id = LayerId(next.unwrap_or(LayerId::FIRST_CUSTOM));
        let id = def.id;
        self.layers.push(def);
        self.dirty = true;
        id
    }

    pub fn remove_layer(&mut self, id: LayerId) {
        if !id.is_custom() {
            return;
        }
        self.layers.retain(|l| l.id != id);
        for d in &mut self.dimensions {
            let coords: Vec<(i32, i32)> = d.tiles_with_layer(id).into_iter().collect();
            for (tx, ty) in coords {
                if let Some(t) = d.tile_mut(tx, ty) {
                    t.layers.retain(|(l, _)| *l != id);
                }
            }
        }
        self.dirty = true;
    }

    pub fn terrain(&self, id: u8) -> &TerrainDef {
        self.terrains.get(id as usize).unwrap_or(&self.terrains[0])
    }

    /// Change the build limits of the world (and all dimensions), clamping heights.
    pub fn set_limits(&mut self, limits: BuildLimits) {
        let limits = limits.sanitized();
        self.limits = limits;
        for d in &mut self.dimensions {
            d.limits = limits;
            let coords = d.tile_coords();
            for (tx, ty) in coords {
                let t = d.tile_mut(tx, ty).unwrap();
                for h in t.height.iter_mut() {
                    *h = h.clamp(limits.min_y as f32, (limits.max_y - 1) as f32);
                }
            }
        }
        self.dirty = true;
    }

    // ---- persistence ------------------------------------------------------------------------

    pub fn save(&mut self, path: &Path) -> Result<()> {
        for d in &mut self.dimensions {
            let defaults: Vec<(LayerId, u8)> = self.layers.iter().map(|l| (l.id, l.default)).collect();
            d.compact(|id| defaults.iter().find(|(l, _)| *l == id).map(|(_, v)| *v).unwrap_or(0));
        }
        let body = bincode::serialize(self).context("serialising world")?;
        let tmp = path.with_extension("meadow.tmp");
        {
            let f = std::fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
            let mut w = std::io::BufWriter::new(f);
            w.write_all(MAGIC)?;
            let mut enc = zstd::Encoder::new(w, 3)?;
            enc.write_all(&body)?;
            enc.finish()?.flush()?;
        }
        std::fs::rename(&tmp, path)?;
        self.path = Some(path.to_owned());
        self.dirty = false;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<World> {
        let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut r = std::io::BufReader::new(f);
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic)?;
        if &magic != MAGIC {
            bail!("{} is not a Meadow world", path.display());
        }
        let mut body = Vec::new();
        zstd::Decoder::new(r)?.read_to_end(&mut body)?;
        let mut w: World = bincode::deserialize(&body).context("corrupt world file")?;
        // Built-in definitions may have improved since the world was saved; keep custom ones.
        let builtin = builtin_terrains();
        for (i, t) in w.terrains.iter_mut().enumerate() {
            if !t.custom && i < builtin.len() {
                *t = builtin[i].clone();
            }
        }
        if w.terrains.len() < builtin.len() {
            w.terrains.extend_from_slice(&builtin[w.terrains.len()..]);
        }
        for b in builtin_layers() {
            if w.layer(b.id).is_none() {
                w.layers.push(b);
            }
        }
        w.path = Some(path.to_owned());
        Ok(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_roundtrip() {
        let mut w = World::new(&NewWorld { hills: 20.0, width_tiles: 2, height_tiles: 2, ..Default::default() });
        w.surface_mut().set_layer_value(LayerId::FROST, true, 0, 3, 3, 1);
        let dir = std::env::temp_dir().join(format!("meadow-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.meadow");
        w.save(&p).unwrap();
        let back = World::load(&p).unwrap();
        assert_eq!(back.surface().tile_count(), 4);
        assert_eq!(back.surface().height(10, 10), w.surface().height(10, 10));
        assert_eq!(back.surface().layer_value(LayerId::FROST, 0, 3, 3), 1);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tall_limits() {
        let mut w = World::new(&NewWorld { limits: BuildLimits { min_y: -2032, max_y: 2032 }, base_height: 1500.0, ..Default::default() });
        assert_eq!(w.limits.height(), 4064);
        assert_eq!(w.surface().height(0, 0), Some(1500.0));
        w.set_limits(BuildLimits::VANILLA);
        assert_eq!(w.surface().height(0, 0), Some(319.0));
    }
}
