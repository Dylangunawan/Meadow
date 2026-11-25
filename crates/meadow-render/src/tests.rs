use std::sync::Arc;

use meadow_core::layer::LayerId;
use meadow_core::world::NewWorld;
use meadow_core::{DimensionKind, World};

use crate::{Renderer, Scene};

/// One shared device for all GPU tests, used by one test at a time. Software drivers on GPU-less
/// CI machines can crash when several devices are created and used concurrently.
fn gpu() -> Option<(Renderer, parking_lot::MutexGuard<'static, ()>)> {
    static RENDERER: std::sync::OnceLock<Option<Renderer>> = std::sync::OnceLock::new();
    static LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    let guard = LOCK.lock();
    let r = RENDERER.get_or_init(|| match Renderer::headless() {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("no GPU adapter available, skipping GPU tests: {e}");
            None
        }
    });
    r.clone().map(|r| (r, guard))
}

fn out_dir() -> std::path::PathBuf {
    let d = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sample_world() -> World {
    let mut w = World::new(&NewWorld { width_tiles: 4, height_tiles: 4, hills: 40.0, base_height: 70.0, ..Default::default() });
    let d = w.surface_mut();
    for z in -256..256 {
        for x in -256..256 {
            let h = d.height(x, z).unwrap();
            if h > 85.0 {
                d.set_terrain(x, z, meadow_core::terrain::ids::ROCK);
            }
            if h > 95.0 {
                d.set_layer_value(LayerId::FROST, true, 0, x, z, 1);
            } else if h > 64.0 && (x + z).rem_euclid(200) < 120 {
                d.set_layer_value(LayerId::DECIDUOUS, false, 0, x, z, 12);
            }
        }
    }
    w
}

#[test]
fn headless_render() {
    let Some((r, _gpu)) = gpu() else { return };
    let world = Arc::new(sample_world());
    let mut scene = Scene::new(&r);
    scene.settings.msaa = 1;
    scene.set_world(world, DimensionKind::Overworld);
    scene.frame_world();
    scene.camera.distance = 260.0;
    scene.camera.pitch = 0.5;
    let (w, h) = (960, 540);
    scene.finish_loading(&r, w as f32 / h as f32);
    scene.render(&r, w, h);
    let img = scene.read_pixels(&r).unwrap();
    img.save(out_dir().join("viewport.png")).unwrap();
    assert!(scene.stats.tiles_drawn > 0, "{:?}", scene.stats);
    // The frame should not be just sky.
    let sky = img.get_pixel(0, 0).0;
    let differing = img.pixels().filter(|p| (p.0[0] as i32 - sky[0] as i32).abs() > 20).count();
    assert!(differing > (w * h / 4) as usize, "mostly empty frame: {differing}");
}

#[test]
fn object_layer_preview_and_lod() {
    use meadow_blocks::StateId;
    use meadow_core::layer::{DataSize, LayerDef, LayerKind, ObjectEntry, ObjectLayerSettings};
    use meadow_core::objects::CustomObject;
    let Some((r, _gpu)) = gpu() else { return };
    // A boulder-with-moss object built in code (stands in for a user's schematic).
    let mut blocks = Vec::new();
    for y in 0..5 {
        for z in -3..=3 {
            for x in -3..=3 {
                let d = (x * x + z * z + (y * y) * 2) as f32;
                if d <= 10.0 {
                    let s = if y >= 3 { "minecraft:moss_block" } else if (x + z) % 3 == 0 { "minecraft:cobblestone" } else { "minecraft:andesite" };
                    blocks.push((x, y, z, StateId::parse(s)));
                }
            }
        }
    }
    let rock = Arc::new(CustomObject::from_positions("boulder", &blocks, [0, 0, 0]));
    let layer = LayerDef {
        id: LayerId(100),
        name: "Boulders".into(),
        description: String::new(),
        kind: LayerKind::CustomObjects(ObjectLayerSettings { objects: vec![ObjectEntry { object: rock, weight: 1 }], spacing: 9, density: 0.7, ..Default::default() }),
        data: DataSize::Nibble,
        default: 0,
        color: [128, 128, 128],
        priority: 50,
        hidden: false,
    };
    let world = crate::preview::preview_world(&layer, &crate::preview::PreviewOptions { radius: 48, ..Default::default() });
    let mut scene = Scene::new(&r);
    scene.settings.msaa = 1;
    scene.set_world(Arc::new(world), DimensionKind::Overworld);
    scene.camera.target = glam::Vec3::new(0.0, 64.0, 0.0);
    scene.camera.distance = 110.0;
    scene.camera.pitch = 0.6;
    scene.finish_loading(&r, 16.0 / 9.0);
    scene.render(&r, 800, 450);
    scene.read_pixels(&r).unwrap().save(out_dir().join("object_preview.png")).unwrap();

    // Far view of a bigger world exercises LOD.
    let big = World::new(&NewWorld { width_tiles: 24, height_tiles: 24, hills: 90.0, base_height: 90.0, ..Default::default() });
    let mut scene = Scene::new(&r);
    scene.settings.msaa = 1;
    scene.set_world(Arc::new(big), DimensionKind::Overworld);
    scene.frame_world();
    scene.camera.pitch = 0.45;
    scene.finish_loading(&r, 16.0 / 9.0);
    scene.render(&r, 960, 540);
    let steps: std::collections::BTreeSet<i32> = scene.tiles_steps();
    scene.read_pixels(&r).unwrap().save(out_dir().join("lod_far.png")).unwrap();
    assert!(steps.len() > 1, "expected several LOD levels, got {steps:?}");
}
