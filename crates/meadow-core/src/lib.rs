//! Meadow's core: the world model (tiles, dimensions, layers, terrain), editing operations,
//! custom masks, height map import/resampling, custom objects and decoration placement.

pub mod decorate;
pub mod dimension;
pub mod heightmap;
pub mod layer;
pub mod mask;
pub mod noise;
pub mod objects;
pub mod ops;
pub mod terrain;
pub mod theme;
pub mod tile;
pub mod trees;
pub mod world;

pub use dimension::{Dimension, DimensionKind};
pub use layer::{LayerDef, LayerId, LayerKind};
pub use tile::{TILE_SHIFT, TILE_SIZE, Tile};
pub use world::{NewWorld, World};

pub use meadow_blocks as blocks;
