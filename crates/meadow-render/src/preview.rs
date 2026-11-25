//! Sample worlds for the object/layer preview: a small patch of terrain with one layer painted
//! on it, decorated by the same placement engine the exporter uses.

use meadow_core::layer::{LayerDef, LayerKind};
use meadow_core::world::NewWorld;
use meadow_core::{LayerId, World};

#[derive(Debug, Clone, Copy)]
pub struct PreviewOptions {
    /// Radius of the painted area in blocks.
    pub radius: i32,
    /// Layer intensity (0..15 for nibble layers).
    pub intensity: u8,
    pub terrain: u8,
    /// Hilly preview ground instead of flat.
    pub hills: f32,
    pub seed: u64,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        PreviewOptions { radius: 40, intensity: 15, terrain: 0, hills: 0.0, seed: 12345 }
    }
}

/// A one-tile world with `layer` painted in a circle around the origin.
pub fn preview_world(layer: &LayerDef, opts: &PreviewOptions) -> World {
    let mut w = World::new(&NewWorld {
        name: "Preview".into(),
        seed: opts.seed,
        width_tiles: 2,
        height_tiles: 2,
        base_height: 64.0,
        water_level: 50,
        terrain: opts.terrain,
        hills: opts.hills,
        ..Default::default()
    });
    let mut def = layer.clone();
    if !def.id.is_custom() {
        // Built-in layer: already present in the world.
        def = w.layer(layer.id).cloned().unwrap_or(def);
    } else {
        w.layers.retain(|l| l.id != def.id);
        w.layers.push(def.clone());
    }
    let max = def.data.max();
    let v = opts.intensity.min(max).max(1);
    let r = opts.radius;
    let d = w.surface_mut();
    for z in -r..=r {
        for x in -r..=r {
            if x * x + z * z <= r * r {
                d.set_layer_value(def.id, def.is_bits(), def.default, x, z, v);
            }
        }
    }
    d.clear_history();
    w
}

/// A layer wrapping a single object (for "show this one object" previews).
pub fn single_object_layer(obj: std::sync::Arc<meadow_core::objects::CustomObject>) -> LayerDef {
    LayerDef {
        id: LayerId(60000),
        name: obj.name.clone(),
        description: String::new(),
        kind: LayerKind::CustomObjects(meadow_core::layer::ObjectLayerSettings {
            objects: vec![meadow_core::layer::ObjectEntry { object: obj, weight: 1 }],
            spacing: 1000,
            density: 1.0,
            random_rotation: false,
            random_mirror: false,
            ..Default::default()
        }),
        data: meadow_core::layer::DataSize::Nibble,
        default: 0,
        color: [200, 200, 200],
        priority: 50,
        hidden: false,
    }
}
