use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use meadow_core::layer::LayerId;
use meadow_core::{NewWorld, World};
use parking_lot::Mutex;

use super::*;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("meadow-script-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&d).ok();
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(src: &str, dir: &Path, world: Option<World>) -> (ScriptOutcome, String) {
    let out = Arc::new(Mutex::new(String::new()));
    let o2 = out.clone();
    let r = run_script(src, &dir.join("test.js"), world, &[], &BTreeMap::new(), Arc::new(AtomicBool::new(false)), move |s| {
        o2.lock().push_str(s)
    });
    let text = out.lock().clone();
    (r, text)
}

fn write_heightmap(path: &Path) {
    // 256x256 radial hill, 8-bit
    let img = image::GrayImage::from_fn(256, 256, |x, y| {
        let d = (((x as f32 - 128.0).powi(2) + (y as f32 - 128.0).powi(2)).sqrt() / 128.0).min(1.0);
        image::Luma([(255.0 * (1.0 - d)) as u8])
    });
    img.save(path).unwrap();
}

#[test]
fn worldpainter_example_script() {
    let dir = tmpdir("example");
    write_heightmap(&dir.join("heightmap.png"));
    // Adapted from WorldPainter's example.js (legacy .heightMap()/.world() aliases included).
    let src = r#"
var heightMap = wp.getHeightMap().fromFile('heightmap.png').go();
var world = wp.createWorld().fromHeightMap(heightMap).go();
wp.applyHeightMap()
        .heightMap(heightMap)
        .world(world)
        .applyToTerrain()
        .fromLevels(0, 64).toTerrain(36)
        .fromLevels(65, 96).toTerrain(0)
        .fromLevels(97, 112).toTerrain(3)
        .fromLevels(113, 255).toTerrain(29)
        .go();
var frostLayer = wp.getLayer().name('Frost').go();
wp.applyHeightMap()
        .heightMap(heightMap)
        .world(world)
        .applyToLayer(frostLayer)
        .fromLevels(0, 119).toLevel(0)
        .fromLevels(120, 255).toLevel(1)
        .go();
var d = world.getDimension(0);
print('centre ' + d.getHeightAt(128, 128) + ' ' + d.getTerrainAt(128, 128).getName() + ' ' + d.getBitLayerValueAt(frostLayer, 128, 128));
print('edge ' + d.getTerrainAt(0, 0) + ' ' + (d.getTerrainAt(0, 0) === Terrain.BEACHES));
wp.saveWorld(world).toFile('output.world').go();
"#;
    let (r, out) = run(src, &dir, None);
    assert!(r.error.is_none(), "{:?}\n{out}", r.error);
    assert!(out.contains("Rock true"), "{out}");
    assert!(out.contains("edge Beaches true"), "{out}");
    let saved = World::load(&dir.join("output.meadow")).unwrap();
    assert_eq!(saved.surface().terrain(128, 128), Some(29));
    assert_eq!(saved.surface().layer_value(LayerId::FROST, 0, 128, 128), 1);
    assert_eq!(saved.surface().layer_value(LayerId::FROST, 0, 10, 10), 0);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn slope_filter_and_current_world() {
    let dir = tmpdir("slope");
    let mut w = World::new(&NewWorld { width_tiles: 2, height_tiles: 2, ..Default::default() });
    for x in -50..50 {
        w.surface_mut().set_height(x, 10, 64.0 + (x.max(0) as f32) * 2.0);
    }
    let src = r#"
var frost = wp.getLayer().withName('Frost').go();
var steep = wp.createFilter().aboveDegrees(30).go();
wp.applyLayer(frost).toWorld(world).withFilter(steep).go();
// Per-column loop through the Java-style API
var n = 0;
for (var x = -64; x < 64; x++) {
    for (var y = -64; y < 64; y++) {
        if (dimension.getHeightAt(x, y) > 100) { dimension.setTerrainAt(x, y, Terrain.STONE); n++; }
    }
}
print('stone ' + n + ' bounds ' + dimension.getLowestX() + '..' + dimension.getHighestX());
"#;
    let (r, out) = run(src, &dir, Some(w));
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(out.contains("bounds -1..0"), "{out}");
    let w = r.world.unwrap();
    let d = w.surface();
    assert_eq!(d.layer_value(LayerId::FROST, 0, 20, 10), 1, "steep column has frost");
    assert_eq!(d.layer_value(LayerId::FROST, 0, -20, -20), 0);
    assert_eq!(d.terrain(40, 10), Some(28));
    // The whole script is one undo step.
    let mut w = w;
    w.surface_mut().undo();
    assert_eq!(w.surface().layer_value(LayerId::FROST, 0, 20, 10), 0);
    assert_eq!(w.surface().terrain(40, 10), Some(0));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn colour_mapping_biomes_java_shims_and_exit() {
    let dir = tmpdir("colour");
    let img = image::RgbImage::from_fn(64, 64, |x, _| if x < 32 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) });
    img.save(dir.join("colours.png")).unwrap();
    let w = World::new(&NewWorld { width_tiles: 2, height_tiles: 2, ..Default::default() });
    let src = r#"
var Constants = Java.type('org.pepsoft.worldpainter.Constants');
var biomes = wp.getLayer().withName('Biomes').go();
var map = wp.getHeightMap().fromFile('colours.png').go();
wp.applyHeightMap(map).shift(-64, -64).toWorld(world).applyToLayer(biomes)
    .fromColour(255, 0, 0).toLevel(wp.getBiomeId().withName('Forest').go())
    .fromColour(0, 0, 255).toLevel(6)
    .go();
var d = world.getDimension(Constants.DIM_NORMAL);
print(d.getLayerValueAt(biomes, -60, -60) + ',' + d.getLayerValueAt(biomes, -10, -60) + ',' + d.getLayerValueAt(biomes, 10, 10));
print(wp.version.indexOf('Meadow') === 0);
exit(3);
print('not reached');
"#;
    let (r, out) = run(src, &dir, Some(w));
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(r.exit_code, 3);
    assert!(out.contains("4,6,255"), "{out}");
    assert!(!out.contains("not reached"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn errors_are_reported_and_cancel_works() {
    let dir = tmpdir("err");
    let (r, _) = run("wp.getLayer().withName('NoSuchLayer').go();", &dir, None);
    assert!(r.error.as_deref().unwrap_or("").contains("No layer named"), "{:?}", r.error);
    let cancel = Arc::new(AtomicBool::new(true));
    let r = run_script("while (true) {}", &dir.join("x.js"), None, &[], &BTreeMap::new(), cancel, |_| {});
    assert_eq!(r.error.as_deref(), Some("Script cancelled"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn descriptor() {
    let src = "// script.name=Test\n// script.description=Line one\\nline two\n// script.param.firstParam.type=integer\n\
               // script.param.anotherParam.type=percentage\n// script.param.anotherParam.default=50\n\
               // script.param.demoParam.type=boolean\n// script.param.demoParam.displayName=Demo parameter\n\
               // script.hideCmdLineParams=true\nvar x = 1;\n// script.param.ignored.type=string\n";
    let d = parse_descriptor(src);
    assert_eq!(d.name.as_deref(), Some("Test"));
    assert_eq!(d.description.as_deref(), Some("Line one\nline two"));
    assert_eq!(d.params.len(), 3);
    assert_eq!(d.params[1].kind, ParamType::Percentage);
    assert_eq!(d.params[1].default.as_deref(), Some("50"));
    assert_eq!(d.params[2].label(), "Demo parameter");
    assert!(d.hide_cmd_line_params);
}
