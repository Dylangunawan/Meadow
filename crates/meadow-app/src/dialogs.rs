//! Dialog windows.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
use meadow_blocks::{BuildLimits, McVersion, StateId};
use meadow_core::dimension::{Dimension, DimensionKind};
use meadow_core::heightmap::{self, Channel, HeightMap, ImportSettings, Resample};
use meadow_core::layer::{DataSize, LayerKind, ObjectEntry, PlantEntry, WaterMode};
use meadow_core::mask::{CustomMask, MaskImage, MaskNode};
use meadow_core::objects::CustomObject;
use meadow_core::ops::{self, Tool};
use meadow_core::terrain::{Material, Stratum, TerrainDef};
use meadow_core::world::{GameMode, Generator, NewWorld};
use meadow_core::{LayerId, World};
use meadow_script::{ParamType, ParamValue, ScriptDescriptor};

use crate::app::MeadowApp;
use crate::jobs::{ExportJob, ScriptJob};
use crate::settings::{Dock, ThemeChoice};
use crate::view3d::PreviewMode;

#[derive(Default)]
pub struct Dialogs {
    pub open_file: bool,
    pub error: Option<String>,
    pub about: bool,
    pub help: bool,
    pub settings_open: bool,
    /// WorldPainter's "Configure view" (Ctrl+V).
    pub configure_view: bool,
    pub new_world: NewWorldDialog,
    pub new_dimension: Option<DimensionKind>,
    pub import: ImportDialog,
    pub export: ExportDialog,
    pub script: ScriptDialog,
    pub rescale: RescaleDialog,
    pub resize: ResizeDialog,
    pub remap: RemapDialog,
    pub fill: FillDialog,
    pub blocks: BlockBrowser,
    pub layer_editor: LayerEditor,
    pub mask_editor: MaskEditor,
    pub terrain_editor: TerrainEditor,
    pub world_settings: WorldSettingsDialog,
    pub mask_image: Option<(PathBuf, (i32, i32), f32, String)>,
}

impl Dialogs {
    pub fn mask_from_image(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("Image", &["png", "jpg", "jpeg", "tif", "tiff", "bmp"]).pick_file() {
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Image mask".into());
            self.mask_image = Some((p, (0, 0), 1.0, name));
        }
    }
}

// ---------------------------------------------------------------------------------------------

fn limits_picker(ui: &mut egui::Ui, id: &str, limits: &mut BuildLimits) -> bool {
    let mut changed = false;
    let current = BuildLimits::PRESETS.iter().find(|(_, l)| l == limits).map(|(n, _)| n.to_string()).unwrap_or_else(|| "Custom".into());
    egui::ComboBox::from_id_salt(id).selected_text(current).width(260.0).show_ui(ui, |ui| {
        for (n, l) in BuildLimits::PRESETS {
            if ui.selectable_label(*limits == l, n).clicked() {
                *limits = l;
                changed = true;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("min y");
        changed |= ui.add(egui::DragValue::new(&mut limits.min_y).range(BuildLimits::ABS_MIN_Y..=BuildLimits::ABS_MAX_Y - 16).speed(16)).changed();
        ui.label("max y");
        changed |= ui.add(egui::DragValue::new(&mut limits.max_y).range(BuildLimits::ABS_MIN_Y + 16..=BuildLimits::ABS_MAX_Y).speed(16)).changed();
    });
    if !limits.is_vanilla() {
        ui.label(
            RichText::new(format!(
                "{} blocks tall. Needs a height data pack in Minecraft (Meadow can generate one, or bundle the Higher Heights pack on export).",
                limits.height()
            ))
            .small()
            .color(Color32::from_rgb(230, 190, 90)),
        );
    }
    changed
}

fn terrain_combo(ui: &mut egui::Ui, id: &str, world_terrains: &[TerrainDef], value: &mut u8) {
    let name = world_terrains.get(*value as usize).map(|t| t.name.clone()).unwrap_or_default();
    egui::ComboBox::from_id_salt(id).selected_text(name).height(300.0).show_ui(ui, |ui| {
        for t in world_terrains {
            if t.custom && t.name.starts_with("Custom ") {
                continue;
            }
            ui.selectable_value(value, t.id, &t.name);
        }
    });
}

/// Text field + search menu for picking any block (vanilla, Conquest Reforged, imported).
pub fn block_picker(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, state: &mut StateId) -> bool {
    let mut changed = false;
    let key = egui::Id::unique(("blockpick", &id));
    let mut text: String = ui.data_mut(|d| d.get_temp(key)).unwrap_or_else(|| state.state().to_string());
    ui.horizontal(|ui| {
        let c = state.info().color;
        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        ui.painter().rect_filled(r, 2.0, Color32::from_rgb(c[0], c[1], c[2]));
        let resp = ui.add(egui::TextEdit::singleline(&mut text).desired_width(220.0));
        if (resp.lost_focus() || (resp.changed() && text.ends_with(']')))
            && let Some(bs) = meadow_blocks::BlockState::parse(&text) {
                *state = StateId::of(&bs);
                changed = true;
            }
        ui.menu_button("find", |ui| {
            let qk = egui::Id::unique(("blockq", &id));
            let mut q: String = ui.data_mut(|d| d.get_temp(qk)).unwrap_or_default();
            let nk = egui::Id::unique(("blockns", &id));
            let mut ns: String = ui.data_mut(|d| d.get_temp(nk)).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut q).hint_text("search e.g. 'limestone slab'").desired_width(200.0));
                egui::ComboBox::from_id_salt(("ns", &id)).selected_text(if ns.is_empty() { "all".into() } else { ns.clone() }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut ns, String::new(), "all");
                    for n in meadow_blocks::registry().read().namespaces() {
                        ui.selectable_value(&mut ns, n.clone(), n);
                    }
                });
            });
            let results = meadow_blocks::registry().read().search(&q, if ns.is_empty() { None } else { Some(&ns) }, 300);
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for b in results {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 2.0, Color32::from_rgb(b.color[0], b.color[1], b.color[2]));
                        if ui.button(&b.id).on_hover_text(format!("{} · {}", b.display_name(), b.category)).clicked() {
                            let props = b.default_props();
                            let p: Vec<(&str, &str)> = props.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                            *state = StateId::of(&meadow_blocks::BlockState::new(&b.id, &p));
                            text = state.state().to_string();
                            changed = true;
                            ui.close();
                        }
                    });
                }
            });
            ui.data_mut(|d| {
                d.insert_temp(qk, q);
                d.insert_temp(nk, ns);
            });
        });
    });
    if changed {
        text = state.state().to_string();
    }
    ui.data_mut(|d| d.insert_temp(key, text));
    changed
}

// ---------------------------------------------------------------------------------------------

pub struct NewWorldDialog {
    pub open: bool,
    pub params: NewWorld,
}

impl Default for NewWorldDialog {
    fn default() -> Self {
        NewWorldDialog { open: false, params: NewWorld { hills: 20.0, ..Default::default() } }
    }
}

#[derive(Default)]
pub struct ImportDialog {
    pub open: bool,
    pub path: Option<PathBuf>,
    pub map: Option<HeightMap>,
    pub settings: Option<ImportSettings>,
    pub into_current: bool,
    pub limits: BuildLimits,
    pub name: String,
    pub channel: Option<Channel>,
    pub preview: Option<egui::TextureHandle>,
    pub error: Option<String>,
}

impl ImportDialog {
    pub fn pick_file(&mut self) {
        if let Some(p) = rfd::FileDialog::new().add_filter("Height map", &["png", "tif", "tiff", "jpg", "jpeg", "r16", "raw", "bmp"]).pick_file() {
            self.load(p, Channel::Grey);
        }
    }
    fn load(&mut self, p: PathBuf, ch: Channel) {
        self.error = None;
        match HeightMap::load(&p, ch) {
            Ok(m) => {
                if self.settings.is_none() {
                    let mut s = ImportSettings::for_map(&m, self.limits.min_y, self.limits.max_y);
                    s.offset = (-(m.width as i32) / 2, -(m.height as i32) / 2);
                    self.settings = Some(s);
                } else if let Some(s) = &mut self.settings {
                    let (lo, hi) = m.min_max();
                    s.image_low = lo;
                    s.image_high = hi.max(lo + 1.0);
                }
                self.name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.map = Some(m);
                self.path = Some(p);
                self.channel = Some(ch);
                self.preview = None;
                self.open = true;
            }
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                self.open = true;
            }
        }
    }
}

pub struct ExportDialog {
    pub open: bool,
    pub folder: Option<PathBuf>,
    pub name: String,
    pub compression: u32,
    pub dims: [bool; 3],
    pub datapack_mode: u8,
    pub result: Option<meadow_export::ExportStats>,
}

impl Default for ExportDialog {
    fn default() -> Self {
        ExportDialog { open: false, folder: None, name: String::new(), compression: 4, dims: [true; 3], datapack_mode: 0, result: None }
    }
}

#[derive(Default)]
pub struct ScriptDialog {
    pub open: bool,
    pub path: Option<PathBuf>,
    pub source: String,
    pub descriptor: ScriptDescriptor,
    pub values: BTreeMap<String, String>,
    pub args: String,
    pub output: String,
}

impl ScriptDialog {
    fn load(&mut self, p: PathBuf) {
        match std::fs::read_to_string(&p) {
            Ok(src) => {
                self.descriptor = meadow_script::parse_descriptor(&src);
                self.values = self.descriptor.params.iter().map(|p| (p.name.clone(), p.default.clone().unwrap_or_default())).collect();
                self.source = src;
                self.path = Some(p);
            }
            Err(e) => self.output = format!("Could not read script: {e}"),
        }
    }
}

pub struct RescaleDialog {
    pub open: bool,
    pub factor: f32,
    pub base: f32,
    pub water: bool,
}
impl Default for RescaleDialog {
    fn default() -> Self {
        RescaleDialog { open: false, factor: 1.5, base: 62.0, water: false }
    }
}

pub struct ResizeDialog {
    pub open: bool,
    pub percent: f32,
}
impl Default for ResizeDialog {
    fn default() -> Self {
        ResizeDialog { open: false, percent: 200.0 }
    }
}

#[derive(Default)]
pub struct RemapDialog {
    pub open: bool,
    pub from: (f32, f32),
    pub to: (f32, f32),
}
impl RemapDialog {
    pub fn open_for(&mut self, d: &Dimension) {
        let (lo, hi) = d.tiles().fold((f32::MAX, f32::MIN), |(a, b), t| (a.min(t.min_height()), b.max(t.max_height())));
        self.from = (lo.min(hi), hi.max(lo));
        self.to = self.from;
        self.open = true;
    }
}

#[derive(Default)]
pub struct FillDialog {
    pub open: bool,
    pub what: u8,
    pub mask: Option<usize>,
    pub value: u8,
    pub layer: Option<LayerId>,
    pub terrain: u8,
    pub biome: u8,
    pub level: i32,
}

#[derive(Default)]
pub struct BlockBrowser {
    pub open: bool,
    pub query: String,
    pub ns: String,
}

#[derive(Default)]
pub struct LayerEditor {
    pub open: Option<LayerId>,
}

#[derive(Clone, PartialEq)]
enum Cond {
    Height(f32, f32, f32),
    Slope(f32, f32),
    Terrain(Vec<u8>),
    Layer(LayerId, u8),
    Water(bool),
    Biome(u8),
    Noise(f32, f32, f32, u64),
    Image(MaskImage),
    Painted(LayerId),
    Other(MaskNode),
}

impl Cond {
    fn label(&self) -> &'static str {
        match self {
            Cond::Height(..) => "Height",
            Cond::Slope(..) => "Slope",
            Cond::Terrain(_) => "Terrain",
            Cond::Layer(..) => "Layer",
            Cond::Water(_) => "Water",
            Cond::Biome(_) => "Biome",
            Cond::Noise(..) => "Noise",
            Cond::Image(_) => "Image",
            Cond::Painted(_) => "Painted",
            Cond::Other(_) => "Custom",
        }
    }
    fn to_node(&self) -> MaskNode {
        match self.clone() {
            Cond::Height(a, b, f) => MaskNode::Height { min: a, max: b, feather: f },
            Cond::Slope(a, b) => MaskNode::Slope { min: a, max: b, feather: 2.0 },
            Cond::Terrain(t) => MaskNode::Terrain(t),
            Cond::Layer(l, v) => MaskNode::Layer { layer: l, min: v },
            Cond::Water(w) => MaskNode::Water(w),
            Cond::Biome(b) => MaskNode::Biome(vec![b]),
            Cond::Noise(s, t, f, seed) => MaskNode::Noise { seed, scale: s, octaves: 4, threshold: t, feather: f },
            Cond::Image(i) => MaskNode::Image(i),
            Cond::Painted(l) => MaskNode::Painted(l),
            Cond::Other(n) => n,
        }
    }
    fn from_node(n: &MaskNode) -> (Cond, bool) {
        match n {
            MaskNode::Not(inner) => (Cond::from_node(inner).0, true),
            MaskNode::Height { min, max, feather } => (Cond::Height(*min, *max, *feather), false),
            MaskNode::Slope { min, max, .. } => (Cond::Slope(*min, *max), false),
            MaskNode::Terrain(t) => (Cond::Terrain(t.clone()), false),
            MaskNode::Layer { layer, min } => (Cond::Layer(*layer, *min), false),
            MaskNode::Water(w) => (Cond::Water(*w), false),
            MaskNode::Biome(b) if b.len() == 1 => (Cond::Biome(b[0]), false),
            MaskNode::Noise { seed, scale, threshold, feather, .. } => (Cond::Noise(*scale, *threshold, *feather, *seed), false),
            MaskNode::Image(i) => (Cond::Image(i.clone()), false),
            MaskNode::Painted(l) => (Cond::Painted(*l), false),
            other => (Cond::Other(other.clone()), false),
        }
    }
}

#[derive(Default)]
pub struct MaskEditor {
    pub open: bool,
    index: Option<usize>,
    name: String,
    color: [u8; 3],
    invert: bool,
    any: bool,
    conds: Vec<(Cond, bool)>,
}

impl MaskEditor {
    pub fn open_for(&mut self, world: &World, index: Option<usize>) {
        self.open = true;
        self.index = index;
        self.conds.clear();
        match index.and_then(|i| world.masks.get(i)) {
            Some(m) => {
                self.name = m.name.clone();
                self.color = m.color;
                self.invert = m.invert;
                match &m.node {
                    MaskNode::And(l) => {
                        self.any = false;
                        self.conds = l.iter().map(Cond::from_node).collect();
                    }
                    MaskNode::Or(l) => {
                        self.any = true;
                        self.conds = l.iter().map(Cond::from_node).collect();
                    }
                    n => self.conds = vec![Cond::from_node(n)],
                }
            }
            None => {
                self.name = format!("Mask {}", world.masks.len() + 1);
                self.color = [255, 200, 40];
                self.invert = false;
                self.any = false;
                self.conds = vec![(Cond::Height(80.0, 400.0, 4.0), false)];
            }
        }
    }
    fn build(&self) -> CustomMask {
        let nodes: Vec<MaskNode> = self.conds.iter().map(|(c, not)| if *not { MaskNode::Not(Box::new(c.to_node())) } else { c.to_node() }).collect();
        let node = if nodes.len() == 1 {
            nodes.into_iter().next().unwrap()
        } else if self.any {
            MaskNode::Or(nodes)
        } else {
            MaskNode::And(nodes)
        };
        CustomMask { name: self.name.clone(), node, color: self.color, invert: self.invert }
    }
}

#[derive(Default)]
pub struct TerrainEditor {
    pub open: bool,
    def: Option<TerrainDef>,
}

impl TerrainEditor {
    pub fn open_for(&mut self, world: &World, id: u8) {
        let def = if id == 255 {
            // First unused custom slot
            let slot = (0..96)
                .filter_map(meadow_core::terrain::custom_slot_index)
                .find(|i| world.terrains.get(*i as usize).is_some_and(|t| t.name.starts_with("Custom ")))
                .unwrap_or(47);
            TerrainDef::new_custom(slot, "My terrain", Material::Block(StateId::parse("minecraft:grass_block[snowy=false]")), 1)
        } else {
            world.terrain(id).clone()
        };
        self.def = Some(def);
        self.open = true;
    }
}

#[derive(Default)]
pub struct WorldSettingsDialog {
    pub open: bool,
    name: String,
    seed: u64,
    limits: BuildLimits,
    water: i32,
    deepslate: bool,
    bedrock: bool,
}

impl WorldSettingsDialog {
    pub fn open_for(&mut self, w: &World) {
        self.open = true;
        self.name = w.name.clone();
        self.seed = w.seed;
        self.limits = w.limits;
        self.water = w.surface().default_water_level;
        self.deepslate = w.surface().subsurface.deepslate;
        self.bedrock = w.surface().subsurface.bedrock_floor;
    }
}

// ---------------------------------------------------------------------------------------------

impl MeadowApp {
    pub fn show_dialogs(&mut self, ctx: &egui::Context) {
        self.dlg_error(ctx);
        self.dlg_new_world(ctx);
        self.dlg_new_dimension(ctx);
        self.dlg_import(ctx);
        self.dlg_export(ctx);
        self.dlg_script(ctx);
        self.dlg_rescale(ctx);
        self.dlg_fill(ctx);
        self.dlg_blocks(ctx);
        self.dlg_layer_editor(ctx);
        self.dlg_mask_editor(ctx);
        self.dlg_mask_image(ctx);
        self.dlg_terrain_editor(ctx);
        self.dlg_world_settings(ctx);
        self.dlg_settings(ctx);
        self.dlg_configure_view(ctx);
        self.dlg_about(ctx);
        if let Some(j) = &self.jar_job {
            egui::Window::new("Importing blocks").collapsible(false).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Reading {}…", j.path.display()));
                });
            });
        }
    }

    fn dlg_error(&mut self, ctx: &egui::Context) {
        let Some(msg) = self.dialogs.error.clone() else { return };
        let mut close = false;
        egui::Modal::new(egui::Id::unique("error")).show(ctx, |ui| {
            ui.set_max_width(520.0);
            ui.heading("Something went wrong");
            ui.label(msg);
            if ui.button("OK").clicked() {
                close = true;
            }
        });
        if close {
            self.dialogs.error = None;
        }
    }

    fn dlg_about(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.about;
        egui::Window::new("About Meadow").open(&mut open).collapsible(false).show(ctx, |ui| {
            ui.heading(format!("Meadow {}", env!("CARGO_PKG_VERSION")));
            ui.label("A fast Minecraft terrain editor written in Rust, inspired by WorldPainter.");
            ui.label("Exports Java Edition 1.18.2 – 26.1 worlds with build heights up to 4064 blocks.");
            ui.separator();
            ui.label(RichText::new("Block and biome data: PrismarineJS/minecraft-data (MIT). Conquest Reforged block IDs from its language file. Meadow is an independent implementation and is not affiliated with WorldPainter, Mojang or the Conquest Reforged team.").small());
        });
        self.dialogs.about = open;
        let mut help = self.dialogs.help;
        egui::Window::new("Keyboard & mouse (WorldPainter controls)").open(&mut help).show(ctx, |ui| {
            egui::Grid::new("keys").striped(true).show(ui, |ui| {
                for (k, v) in [
                    ("Left button", "Use the tool"),
                    ("Right button, or Alt + left", "Opposite of the tool (lower, remove paint, lower water…)"),
                    ("Middle drag, or Space + drag", "Pan the map"),
                    ("Mouse wheel", "Brush size (Shift: by one)"),
                    ("Ctrl + wheel, Ctrl + / − / 0", "Zoom in, out, 100 %"),
                    ("Alt + wheel", "Rotate the brush"),
                    ("1 … 9, 0", "Brush intensity 10 % … 90 %, 100 %"),
                    ("Alt + R / P / L", "Spray Paint / Pencil / Fill"),
                    ("Alt + F / H / A / S", "Flood / Height / Flatten / Smooth"),
                    ("R / L", "Rotate the map's light (3D view: rotate the view)"),
                    ("Esc", "Leave the current tool"),
                    ("Ctrl + G / T / P / V", "Global operations / Add tiles / Dimension properties / Configure view"),
                    ("Ctrl + U / H / D", "View Surface / Nether / End"),
                    ("Ctrl + 3", "3D view"),
                    ("3D view: L / R, + / − / 0", "Rotate 90°, zoom, reset"),
                    ("3D view: drag, right-drag, WASD/QE", "Orbit, pan, fly (Meadow extras)"),
                    ("Ctrl + Z / Y", "Undo / redo"),
                    ("Ctrl + N / O / S / Shift+S / E", "New / open / save / save as / export"),
                    ("[ and ], Tab, F", "Brush size, cycle 2D/3D/split, frame world (Meadow extras)"),
                ] {
                    ui.label(RichText::new(k).strong());
                    ui.label(v);
                    ui.end_row();
                }
            });
        });
        self.dialogs.help = help;
    }

    fn dlg_new_world(&mut self, ctx: &egui::Context) {
        if !self.dialogs.new_world.open {
            return;
        }
        let mut open = true;
        let mut create = false;
        let terrains = self.world.terrains.clone();
        egui::Window::new("New world").open(&mut open).collapsible(false).show(ctx, |ui| {
            let p = &mut self.dialogs.new_world.params;
            egui::Grid::new("nw").num_columns(2).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut p.name);
                ui.end_row();
                ui.label("Seed");
                ui.add(egui::DragValue::new(&mut p.seed));
                ui.end_row();
                ui.label("Size (tiles of 128)");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut p.width_tiles).range(1..=512));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut p.height_tiles).range(1..=512));
                    ui.label(format!("= {}×{} blocks", p.width_tiles * 128, p.height_tiles * 128));
                });
                ui.end_row();
                ui.label("Build limits");
                ui.vertical(|ui| {
                    limits_picker(ui, "nw_limits", &mut p.limits);
                });
                ui.end_row();
                ui.label("Base height");
                ui.add(egui::DragValue::new(&mut p.base_height).range(p.limits.min_y as f32..=(p.limits.max_y - 1) as f32));
                ui.end_row();
                ui.label("Water level");
                ui.add(egui::DragValue::new(&mut p.water_level).range(p.limits.min_y..=p.limits.max_y - 1));
                ui.end_row();
                ui.label("Terrain");
                terrain_combo(ui, "nw_terrain", &terrains, &mut p.terrain);
                ui.end_row();
                ui.label("Hills");
                ui.add(egui::Slider::new(&mut p.hills, 0.0..=400.0).text("amplitude"));
                ui.end_row();
            });
            ui.separator();
            create = ui.button("Create").clicked();
        });
        if create {
            let w = World::new(&self.dialogs.new_world.params);
            self.set_world(w);
            open = false;
        }
        self.dialogs.new_world.open = open;
    }

    fn dlg_new_dimension(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.dialogs.new_dimension else { return };
        let mut close = false;
        egui::Modal::new(egui::Id::unique("newdim")).show(ctx, |ui| {
            ui.heading(format!("Create {}", kind.name()));
            ui.label("A new dimension is created with the same size as the surface.");
            ui.horizontal(|ui| {
                if ui.button("Create").clicked() {
                    let surface = self.world.surface();
                    let limits = match kind {
                        DimensionKind::Overworld => surface.limits,
                        _ if self.world.limits.max_y > 256 => BuildLimits { min_y: 0, max_y: self.world.limits.max_y.min(2032) },
                        _ => BuildLimits { min_y: 0, max_y: 256 },
                    };
                    let coords = surface.tile_coords();
                    let mut d = Dimension::new(kind, limits);
                    let (terrain, h, water) = match kind {
                        DimensionKind::Nether => (meadow_core::terrain::ids::NETHERLIKE, 40.0, 31),
                        _ => (meadow_core::terrain::ids::END_STONE, 50.0, 0),
                    };
                    d.default_terrain = terrain;
                    d.default_water_level = water;
                    d.subsurface.material = Material::Block(StateId::parse(if kind == DimensionKind::Nether { "netherrack" } else { "end_stone" }));
                    d.subsurface.deepslate = false;
                    for (tx, ty) in coords {
                        d.ensure_tile(tx, ty, h);
                    }
                    if kind == DimensionKind::Nether {
                        for (tx, ty) in d.tile_coords() {
                            let t = d.tile_mut(tx, ty).unwrap();
                            for w in t.water.iter_mut() {
                                *w = 31;
                            }
                            t.layer_mut(LayerId::LAVA, true, 0);
                            if let Some(meadow_core::tile::LayerData::Bits(b)) = t.layers.iter_mut().find(|(l, _)| *l == LayerId::LAVA).map(|(_, d)| d) {
                                b.iter_mut().for_each(|w| *w = u64::MAX);
                            }
                        }
                    }
                    d.clear_history();
                    self.world.dimensions.push(d);
                    self.dim = kind;
                    self.frame_map = true;
                    self.world.dirty = true;
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if close {
            self.dialogs.new_dimension = None;
        }
    }

    fn dlg_import(&mut self, ctx: &egui::Context) {
        if !self.dialogs.import.open {
            return;
        }
        let mut open = true;
        let mut do_import = false;
        let terrains = self.world.terrains.clone();
        let current_limits = self.world.limits;
        egui::Window::new("Import height map").open(&mut open).default_width(560.0).show(ctx, |ui| {
            let d = &mut self.dialogs.import;
            if let Some(e) = &d.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            ui.horizontal(|ui| {
                ui.label(d.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "No file".into()));
                if ui.button("Choose…").clicked() {
                    d.pick_file();
                }
            });
            let Some(map) = d.map.clone() else { return };
            if d.preview.is_none() {
                let (w, h) = (map.width, map.height);
                let step = ((w.max(h) as f32 / 256.0).ceil() as u32).max(1);
                let (pw, ph) = ((w / step).max(1), (h / step).max(1));
                let mut px = Vec::with_capacity((pw * ph) as usize);
                let (lo, hi) = map.min_max();
                for y in 0..ph {
                    for x in 0..pw {
                        let v = ((map.get((x * step) as i32, (y * step) as i32) - lo) / (hi - lo).max(1e-6) * 255.0) as u8;
                        px.push(Color32::from_gray(v));
                    }
                }
                d.preview = Some(ui.ctx().load_texture("hm_preview", egui::ColorImage::new([pw as usize, ph as usize], px), Default::default()));
            }
            ui.horizontal(|ui| {
                if let Some(t) = &d.preview {
                    ui.image((t.id(), egui::vec2(180.0, 180.0 * t.size()[1] as f32 / t.size()[0] as f32)));
                }
                ui.vertical(|ui| {
                    let (lo, hi) = map.min_max();
                    ui.label(format!("{} × {} pixels, values {lo:.0} – {hi:.0} (max {})", map.width, map.height, map.max_value));
                    ui.horizontal(|ui| {
                        ui.label("Channel");
                        let mut ch = d.channel.unwrap_or(Channel::Grey);
                        let before = ch;
                        for (c, n) in [(Channel::Grey, "Grey"), (Channel::Red, "R"), (Channel::Green, "G"), (Channel::Blue, "B")] {
                            ui.selectable_value(&mut ch, c, n);
                        }
                        if ch != before
                            && let Some(p) = d.path.clone()
                        {
                            d.load(p, ch);
                        }
                    });
                    ui.checkbox(&mut d.into_current, "Import into the current world (otherwise create a new world)");
                    if !d.into_current {
                        ui.horizontal(|ui| {
                            ui.label("World name");
                            ui.text_edit_singleline(&mut d.name);
                        });
                    }
                });
            });
            let limits = if d.into_current { current_limits } else { d.limits };
            if !d.into_current {
                ui.label("Build limits");
                limits_picker(ui, "imp_limits", &mut d.limits);
            }
            let Some(s) = &mut d.settings else { return };
            egui::Grid::new("imp").num_columns(2).show(ui, |ui| {
                ui.label("Horizontal scale");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut s.scale_percent).range(1.0..=1600.0).suffix(" %"));
                    let (w, h) = s.world_size(&map);
                    ui.label(format!("→ {w} × {h} blocks"));
                });
                ui.end_row();
                ui.label("Image values");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut s.image_low));
                    ui.label("to");
                    ui.add(egui::DragValue::new(&mut s.image_high));
                });
                ui.end_row();
                ui.label("map to heights (vertical scale)");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut s.world_low).range(limits.min_y as f32..=(limits.max_y - 1) as f32));
                    ui.label("to");
                    ui.add(egui::DragValue::new(&mut s.world_high).range(limits.min_y as f32..=(limits.max_y - 1) as f32));
                    ui.label(format!("({:.3} blocks per value)", (s.world_high - s.world_low) / (s.image_high - s.image_low).max(1e-6)));
                });
                ui.end_row();
                ui.label("Water level");
                ui.add(egui::DragValue::new(&mut s.water_level).range(limits.min_y..=limits.max_y - 1));
                ui.end_row();
                ui.label("Terrain");
                terrain_combo(ui, "imp_terrain", &terrains, &mut s.terrain);
                ui.end_row();
                ui.label("Top-left corner at");
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut s.offset.0));
                    ui.add(egui::DragValue::new(&mut s.offset.1));
                });
                ui.end_row();
                ui.label("Resampling");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut s.resample, Resample::Bicubic, "Bicubic");
                    ui.selectable_value(&mut s.resample, Resample::Bilinear, "Bilinear");
                    ui.selectable_value(&mut s.resample, Resample::Nearest, "Nearest");
                });
                ui.end_row();
                if d.into_current {
                    ui.label("");
                    ui.checkbox(&mut s.only_raise, "Only raise existing terrain");
                    ui.end_row();
                }
                ui.label("Void below");
                ui.horizontal(|ui| {
                    let mut on = s.void_below.is_some();
                    ui.checkbox(&mut on, "");
                    let mut v = s.void_below.unwrap_or(s.world_low);
                    ui.add_enabled(on, egui::DragValue::new(&mut v));
                    s.void_below = on.then_some(v);
                    ui.label(RichText::new("(no tiles where the map is this low)").small());
                });
                ui.end_row();
            });
            ui.label(RichText::new("You can rescale heights vertically or resize the world later under Tools.").small().weak());
            do_import = ui.button("Import").clicked();
        });
        if do_import {
            let d = &self.dialogs.import;
            if let (Some(map), Some(s)) = (d.map.clone(), d.settings.clone()) {
                let t = std::time::Instant::now();
                if d.into_current {
                    let dim = self.dim_mut();
                    dim.begin_op("Import height map");
                    heightmap::import_into(dim, &map, &s);
                    dim.end_op();
                    self.world.dirty = true;
                } else {
                    let mut w = World::new(&NewWorld { name: d.name.clone(), width_tiles: 0, height_tiles: 0, limits: d.limits, water_level: s.water_level, terrain: s.terrain, ..Default::default() });
                    let dim = w.surface_mut();
                    dim.default_water_level = s.water_level;
                    heightmap::import_into(dim, &map, &s);
                    dim.clear_history();
                    self.set_world(w);
                    self.world.dirty = true;
                }
                self.toast(format!("Imported height map in {:.2}s", t.elapsed().as_secs_f32()));
                self.frame_map = true;
                open = false;
            }
        }
        self.dialogs.import.open = open;
    }

    fn dlg_export(&mut self, ctx: &egui::Context) {
        if !self.dialogs.export.open && self.export_job.is_none() {
            return;
        }
        let mut open = self.dialogs.export.open || self.export_job.is_some();
        let mut start = false;
        egui::Window::new("Export to Minecraft").open(&mut open).default_width(520.0).show(ctx, |ui| {
            if let Some(j) = &self.export_job {
                ui.label(j.progress.stage.lock().clone());
                ui.add(egui::ProgressBar::new(j.progress.fraction()).show_percentage().animate(true));
                ui.label(format!("{:.1}s elapsed", j.started.elapsed().as_secs_f32()));
                if ui.button("Cancel").clicked() {
                    j.cancel();
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            let d = &mut self.dialogs.export;
            if d.name.is_empty() {
                d.name = self.world.name.clone();
            }
            if d.folder.is_none() {
                d.folder = self.world.export.last_directory.clone().or_else(|| self.settings.minecraft_saves.clone());
            }
            let e = &mut self.world.export;
            egui::Grid::new("exp").num_columns(2).show(ui, |ui| {
                ui.label("Saves folder");
                ui.horizontal(|ui| {
                    ui.label(d.folder.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "(choose)".into()));
                    if ui.button("…").clicked()
                        && let Some(p) = rfd::FileDialog::new().pick_folder()
                    {
                        d.folder = Some(p);
                    }
                });
                ui.end_row();
                ui.label("World folder name");
                ui.text_edit_singleline(&mut d.name);
                ui.end_row();
                ui.label("Minecraft version");
                egui::ComboBox::from_id_salt("ver").selected_text(e.version.label()).show_ui(ui, |ui| {
                    for v in McVersion::ALL.iter().rev() {
                        ui.selectable_value(&mut e.version, *v, v.label());
                    }
                });
                ui.end_row();
                ui.label("Outside the map");
                egui::ComboBox::from_id_salt("gen").selected_text(format!("{:?}", e.generator)).show_ui(ui, |ui| {
                    ui.selectable_value(&mut e.generator, Generator::Void, "Void");
                    ui.selectable_value(&mut e.generator, Generator::Flat, "Flat");
                    ui.selectable_value(&mut e.generator, Generator::Default, "Default (Minecraft generates)");
                });
                ui.end_row();
                ui.label("Game mode");
                egui::ComboBox::from_id_salt("gm").selected_text(format!("{:?}", e.game_mode)).show_ui(ui, |ui| {
                    for g in [GameMode::Creative, GameMode::Survival, GameMode::Adventure, GameMode::Spectator] {
                        ui.selectable_value(&mut e.game_mode, g, format!("{g:?}"));
                    }
                });
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut e.allow_cheats, "Allow cheats");
                ui.end_row();
                ui.label("Dimensions");
                ui.horizontal(|ui| {
                    for (i, k) in [DimensionKind::Overworld, DimensionKind::Nether, DimensionKind::End].iter().enumerate() {
                        if self.world.dimensions.iter().any(|x| x.kind == *k) {
                            ui.checkbox(&mut d.dims[i], k.name());
                        }
                    }
                });
                ui.end_row();
                ui.label("Compression");
                ui.add(egui::Slider::new(&mut d.compression, 1..=9).text("(1 = fastest, 6 = Minecraft default)"));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut e.replace_unavailable_blocks, "Replace blocks missing from this version");
                ui.end_row();
            });
            if !self.world.limits.is_vanilla() || e.datapack.is_some() {
                ui.separator();
                ui.label(RichText::new(format!("Build height {}..{} ({} blocks)", self.world.limits.min_y, self.world.limits.max_y, self.world.limits.height())).strong());
                ui.radio_value(&mut d.datapack_mode, 0, "Generate a height data pack (Minecraft 1.18.2 – 1.21.10)");
                ui.radio_value(&mut d.datapack_mode, 1, "Bundle a data pack file (e.g. Higher Heights from Modrinth)");
                ui.radio_value(&mut d.datapack_mode, 2, "None (I will install a data pack myself)");
                if d.datapack_mode == 1 {
                    ui.horizontal(|ui| {
                        ui.label(e.datapack.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "(choose the .zip)".into()));
                        if ui.button("…").clicked()
                            && let Some(p) = rfd::FileDialog::new().add_filter("Data pack", &["zip"]).pick_file()
                        {
                            e.datapack = Some(p);
                        }
                    });
                }
                if d.datapack_mode == 0 && !e.version.can_generate_height_datapack() {
                    ui.colored_label(Color32::from_rgb(230, 190, 90), format!("Minecraft {} changed the dimension type format; bundle the Higher Heights pack for this version instead.", e.version.name()));
                }
            }
            if let Some(r) = &d.result {
                ui.separator();
                ui.label(RichText::new(format!("Last export: {} chunks in {} regions, {:.1}s", r.chunks, r.regions, r.elapsed.as_secs_f32())).strong());
                if !r.datapacks.is_empty() {
                    ui.label(format!("Data packs: {}", r.datapacks.join(", ")));
                }
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    for w in &r.warnings {
                        ui.colored_label(Color32::from_rgb(230, 190, 90), w);
                    }
                });
            }
            ui.separator();
            start = ui.add_enabled(d.folder.is_some() && !d.name.trim().is_empty(), egui::Button::new("Export")).clicked();
        });
        if start {
            let d = &mut self.dialogs.export;
            d.result = None;
            let dir = d.folder.clone().unwrap().join(d.name.trim());
            match d.datapack_mode {
                0 => {
                    self.world.export.generate_height_datapack = true;
                    if self.world.export.datapack.is_some() {
                        self.world.export.datapack = None;
                    }
                }
                1 => {}
                _ => {
                    self.world.export.generate_height_datapack = false;
                    self.world.export.datapack = None;
                }
            }
            self.world.export.last_directory = d.folder.clone();
            let dims: Vec<DimensionKind> = [DimensionKind::Overworld, DimensionKind::Nether, DimensionKind::End]
                .into_iter()
                .enumerate()
                .filter(|(i, _)| d.dims[*i])
                .map(|(_, k)| k)
                .collect();
            let opts = meadow_export::ExportOptions { dir, version: self.world.export.version, dimensions: dims, compression: d.compression, overwrite: true };
            self.export_job = Some(ExportJob::start(self.world.snapshot(), opts, ctx.clone()));
        }
        self.dialogs.export.open = open;
    }

    fn dlg_script(&mut self, ctx: &egui::Context) {
        if !self.dialogs.script.open && self.script_job.is_none() {
            return;
        }
        let mut open = self.dialogs.script.open || self.script_job.is_some();
        let mut run = false;
        egui::Window::new("Run script").open(&mut open).default_width(560.0).show(ctx, |ui| {
            let running = self.script_job.is_some();
            let d = &mut self.dialogs.script;
            ui.horizontal(|ui| {
                ui.label(d.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "No script selected".into()));
                if ui.add_enabled(!running, egui::Button::new("Choose…")).clicked()
                    && let Some(p) = rfd::FileDialog::new().add_filter("Script", &["js"]).pick_file()
                {
                    d.load(p.clone());
                    self.settings.recent_scripts.retain(|x| x != &p);
                    self.settings.recent_scripts.insert(0, p);
                    self.settings.recent_scripts.truncate(10);
                }
                ui.menu_button("Recent", |ui| {
                    for p in self.settings.recent_scripts.clone() {
                        if ui.button(p.display().to_string()).clicked() {
                            d.load(p);
                            ui.close();
                        }
                    }
                });
            });
            if let Some(n) = &d.descriptor.name {
                ui.heading(n);
            }
            if let Some(desc) = &d.descriptor.description {
                ui.label(desc);
            }
            if !d.descriptor.params.is_empty() {
                egui::Grid::new("params").num_columns(2).show(ui, |ui| {
                    for p in d.descriptor.params.clone() {
                        let v = d.values.entry(p.name.clone()).or_default();
                        ui.label(p.label()).on_hover_text(p.description.clone().unwrap_or_default());
                        match p.kind {
                            ParamType::Boolean => {
                                let mut b = v == "true";
                                ui.checkbox(&mut b, "");
                                *v = b.to_string();
                            }
                            ParamType::Integer | ParamType::Percentage => {
                                let mut n: i64 = v.parse().unwrap_or(0);
                                let lo = p.min.unwrap_or(if p.kind == ParamType::Percentage { 0.0 } else { -1e9 }) as i64;
                                let hi = p.max.unwrap_or(if p.kind == ParamType::Percentage { 100.0 } else { 1e9 }) as i64;
                                ui.add(egui::DragValue::new(&mut n).range(lo..=hi).suffix(if p.kind == ParamType::Percentage { " %" } else { "" }));
                                *v = n.to_string();
                            }
                            ParamType::Float => {
                                let mut n: f64 = v.parse().unwrap_or(0.0);
                                ui.add(egui::DragValue::new(&mut n).speed(0.1));
                                *v = n.to_string();
                            }
                            ParamType::File => {
                                ui.horizontal(|ui| {
                                    ui.label(if v.is_empty() { "(none)".to_string() } else { v.clone() });
                                    if ui.button("…").clicked()
                                        && let Some(f) = rfd::FileDialog::new().pick_file()
                                    {
                                        *v = f.display().to_string();
                                    }
                                });
                            }
                            ParamType::String => {
                                ui.text_edit_singleline(v);
                            }
                        }
                        ui.end_row();
                    }
                });
            }
            if !d.descriptor.hide_cmd_line_params {
                ui.label("Arguments (one per line, available as `arguments`)");
                ui.add(egui::TextEdit::multiline(&mut d.args).desired_rows(2).desired_width(f32::INFINITY));
            }
            ui.horizontal(|ui| {
                if running {
                    ui.spinner();
                    ui.label("Running…");
                    if ui.button("Cancel").clicked()
                        && let Some(j) = &self.script_job
                    {
                        j.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                } else {
                    run = ui.add_enabled(d.path.is_some(), egui::Button::new("Run on current world")).clicked();
                }
            });
            let out = self.script_job.as_ref().map(|j| j.output.clone()).unwrap_or_else(|| self.dialogs.script.output.clone());
            ui.separator();
            egui::ScrollArea::vertical().max_height(260.0).stick_to_bottom(true).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(out).monospace()).wrap());
            });
            ui.label(RichText::new("WorldPainter scripts (wp.*, world, dimension, params, arguments, print, Java.type) run unchanged where the API is supported. The whole run is one undo step.").small().weak());
        });
        if run {
            let d = &self.dialogs.script;
            let params: BTreeMap<String, ParamValue> = d
                .descriptor
                .params
                .iter()
                .map(|p| {
                    let v = d.values.get(&p.name).cloned().unwrap_or_default();
                    let pv = match p.kind {
                        ParamType::Boolean => ParamValue::Bool(v == "true"),
                        ParamType::Integer | ParamType::Percentage => v.parse().map(ParamValue::Int).unwrap_or(ParamValue::Null),
                        ParamType::Float => v.parse().map(ParamValue::Float).unwrap_or(ParamValue::Null),
                        _ if v.is_empty() && p.optional => ParamValue::Null,
                        _ => ParamValue::Str(v),
                    };
                    (p.name.clone(), pv)
                })
                .collect();
            let args: Vec<String> = d.args.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect();
            let path = d.path.clone().unwrap();
            let source = d.source.clone();
            self.dialogs.script.output.clear();
            self.script_job = Some(ScriptJob::start(path, source, self.world.clone(), args, params, ctx.clone()));
        }
        self.dialogs.script.open = open;
    }

    fn dlg_rescale(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.rescale.open;
        let mut apply = false;
        egui::Window::new("Rescale heights vertically").open(&mut open).show(ctx, |ui| {
            let d = &mut self.dialogs.rescale;
            ui.label("Stretch or flatten the terrain around a base level, e.g. after importing a height map.");
            egui::Grid::new("rs").show(ui, |ui| {
                ui.label("Factor");
                ui.add(egui::DragValue::new(&mut d.factor).range(0.01..=50.0).speed(0.01));
                ui.end_row();
                ui.label("Base level");
                ui.add(egui::DragValue::new(&mut d.base));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut d.water, "Scale water levels too");
                ui.end_row();
            });
            apply = ui.button("Apply").clicked();
        });
        if apply {
            let (f, b, w) = (self.dialogs.rescale.factor, self.dialogs.rescale.base, self.dialogs.rescale.water);
            let d = self.dim_mut();
            d.begin_op("Rescale heights");
            heightmap::scale_vertical(d, f, b, w);
            d.end_op();
            self.world.dirty = true;
        }
        self.dialogs.rescale.open = open;

        let mut open = self.dialogs.resize.open;
        let mut apply = false;
        egui::Window::new("Resize world horizontally").open(&mut open).show(ctx, |ui| {
            ui.label("Resample the whole dimension (heights bicubic, everything else nearest).");
            ui.add(egui::DragValue::new(&mut self.dialogs.resize.percent).range(5.0..=1600.0).suffix(" %"));
            if let Some((x0, z0, x1, z1)) = self.dim().block_bounds() {
                let f = self.dialogs.resize.percent / 100.0;
                ui.label(format!("{}×{} → {:.0}×{:.0} blocks", x1 - x0, z1 - z0, (x1 - x0) as f32 * f, (z1 - z0) as f32 * f));
            }
            apply = ui.button("Apply").clicked();
        });
        if apply {
            let f = self.dialogs.resize.percent / 100.0;
            let d = self.dim_mut();
            d.begin_op("Resize world");
            heightmap::resize_horizontal(d, f);
            d.end_op();
            self.world.dirty = true;
            self.frame_map = true;
        }
        self.dialogs.resize.open = open;

        let mut open = self.dialogs.remap.open;
        let mut apply = false;
        egui::Window::new("Remap height range").open(&mut open).show(ctx, |ui| {
            let r = &mut self.dialogs.remap;
            egui::Grid::new("rm").show(ui, |ui| {
                ui.label("From");
                ui.add(egui::DragValue::new(&mut r.from.0));
                ui.add(egui::DragValue::new(&mut r.from.1));
                ui.end_row();
                ui.label("To");
                ui.add(egui::DragValue::new(&mut r.to.0));
                ui.add(egui::DragValue::new(&mut r.to.1));
                ui.end_row();
            });
            apply = ui.button("Apply").clicked();
        });
        if apply {
            let r = (self.dialogs.remap.from, self.dialogs.remap.to);
            let d = self.dim_mut();
            d.begin_op("Remap heights");
            heightmap::remap_heights(d, r.0.0, r.0.1, r.1.0, r.1.1);
            d.end_op();
            self.world.dirty = true;
        }
        self.dialogs.remap.open = open;
    }

    fn dlg_fill(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.fill.open;
        let mut apply = false;
        let terrains = self.world.terrains.clone();
        egui::Window::new("Fill / clear through mask").open(&mut open).show(ctx, |ui| {
            let f = &mut self.dialogs.fill;
            ui.horizontal(|ui| {
                for (i, n) in ["Terrain", "Layer", "Clear layer", "Biome", "Water level", "Height"].iter().enumerate() {
                    ui.selectable_value(&mut f.what, i as u8, *n);
                }
            });
            match f.what {
                0 => terrain_combo(ui, "fill_t", &terrains, &mut f.terrain),
                1 | 2 => {
                    let name = f.layer.and_then(|l| self.world.layer(l)).map(|l| l.name.clone()).unwrap_or_else(|| "Select…".into());
                    egui::ComboBox::from_id_salt("fill_l").selected_text(name).show_ui(ui, |ui| {
                        for l in &self.world.layers {
                            ui.selectable_value(&mut f.layer, Some(l.id), &l.name);
                        }
                    });
                    if f.what == 1 {
                        ui.add(egui::Slider::new(&mut f.value, 1..=15).text("value"));
                    }
                }
                3 => {
                    egui::ComboBox::from_id_salt("fill_b")
                        .selected_text(meadow_blocks::biomes().by_numeric(f.biome).map(|b| b.name.clone()).unwrap_or_default())
                        .show_ui(ui, |ui| {
                            for b in &meadow_blocks::biomes().list {
                                ui.selectable_value(&mut f.biome, b.numeric, &b.name);
                            }
                        });
                }
                4 | 5 => {
                    ui.add(egui::DragValue::new(&mut f.level).prefix(if f.what == 4 { "level " } else { "height " }));
                }
                _ => {}
            }
            let mname = f.mask.and_then(|i| self.world.masks.get(i)).map(|m| m.name.clone()).unwrap_or_else(|| "Everywhere".into());
            egui::ComboBox::from_id_salt("fill_m").selected_text(mname).show_ui(ui, |ui| {
                ui.selectable_value(&mut f.mask, None, "Everywhere");
                for (i, m) in self.world.masks.iter().enumerate() {
                    ui.selectable_value(&mut f.mask, Some(i), &m.name);
                }
            });
            apply = ui.button("Apply").clicked();
        });
        if apply {
            let f = &self.dialogs.fill;
            let tool = match f.what {
                0 => Some(Tool::PaintTerrain(f.terrain)),
                1 | 2 => f.layer.and_then(|l| self.world.layer(l)).map(|d| {
                    if f.what == 1 {
                        Tool::PaintLayer { layer: d.id, value: f.value.min(d.data.max()), bits: d.is_bits(), default: d.default }
                    } else {
                        Tool::EraseLayer { layer: d.id, bits: d.is_bits(), default: d.default }
                    }
                }),
                3 => Some(Tool::PaintBiome(f.biome)),
                4 => Some(Tool::SetWaterLevel(f.level)),
                5 => Some(Tool::SetHeight(f.level as f32)),
                _ => None,
            };
            if let Some(tool) = tool {
                let mask = f.mask.and_then(|i| self.world.masks.get(i).cloned());
                let d = self.dim_mut();
                d.begin_op("Fill");
                ops::fill_through_mask(d, &tool, mask.as_ref());
                d.end_op();
                self.world.dirty = true;
            }
        }
        self.dialogs.fill.open = open;
    }

    fn dlg_blocks(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.blocks.open;
        egui::Window::new("Block browser").open(&mut open).default_width(480.0).show(ctx, |ui| {
            let b = &mut self.dialogs.blocks;
            let reg = meadow_blocks::registry().read();
            let counts: Vec<(String, usize)> = reg.namespaces().into_iter().map(|n| {
                let c = reg.all().iter().filter(|x| x.namespace() == n).count();
                (n, c)
            }).collect();
            ui.label(counts.iter().map(|(n, c)| format!("{n}: {c}")).collect::<Vec<_>>().join(" · "));
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut b.query).hint_text("Search"));
                egui::ComboBox::from_id_salt("bb_ns").selected_text(if b.ns.is_empty() { "all" } else { &b.ns }).show_ui(ui, |ui| {
                    ui.selectable_value(&mut b.ns, String::new(), "all");
                    for (n, _) in &counts {
                        ui.selectable_value(&mut b.ns, n.clone(), n);
                    }
                });
            });
            let results = reg.search(&b.query, if b.ns.is_empty() { None } else { Some(&b.ns) }, 500);
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                egui::Grid::new("bb").striped(true).show(ui, |ui| {
                    for r in results {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(r.color[0], r.color[1], r.color[2]));
                        if ui.selectable_label(false, &r.id).on_hover_text("Click to copy the ID").clicked() {
                            ui.ctx().copy_text(r.id.clone());
                        }
                        ui.label(RichText::new(r.category).small());
                        ui.label(RichText::new(format!("{} props", r.props.len())).small().weak());
                        ui.end_row();
                    }
                });
            });
            ui.label(RichText::new("Conquest Reforged blocks are built in. Import the mod jar (File › Import) to get exact block states and texture colours for any mod.").small().weak());
        });
        self.dialogs.blocks.open = open;
    }

    fn dlg_layer_editor(&mut self, ctx: &egui::Context) {
        let Some(id) = self.dialogs.layer_editor.open else { return };
        let Some(mut def) = self.world.layer(id).cloned() else {
            self.dialogs.layer_editor.open = None;
            return;
        };
        let mut open = true;
        let mut changed = false;
        let mut preview_changed = false;
        let terrains = self.world.terrains.clone();
        egui::Window::new(format!("Layer: {}", def.name)).id(egui::Id::unique("layer_editor")).open(&mut open).default_width(860.0).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(380.0);
                    egui::ScrollArea::vertical().id_salt("le_scroll").max_height(560.0).show(ui, |ui| {
                        if def.id.is_custom() {
                            ui.horizontal(|ui| {
                                ui.label("Name");
                                changed |= ui.text_edit_singleline(&mut def.name).changed();
                                changed |= egui::color_picker::color_edit_button_srgb(ui, &mut def.color).changed();
                            });
                        } else {
                            ui.label(&def.description);
                        }
                        match &mut def.kind {
                            LayerKind::CustomObjects(s) => {
                                ui.separator();
                                ui.label(RichText::new("Objects").strong());
                                let mut remove = None;
                                for (i, e) in s.objects.iter_mut().enumerate() {
                                    ui.horizontal(|ui| {
                                        ui.label(&e.object.name).on_hover_text(e.object.source.clone().unwrap_or_default());
                                        ui.label(RichText::new(format!("{}×{}×{} · {} blocks", e.object.size[0], e.object.size[1], e.object.size[2], e.object.block_count())).small().weak());
                                        changed |= ui.add(egui::DragValue::new(&mut e.weight).range(1..=1000).prefix("weight ")).changed();
                                        if ui.small_button("view").on_hover_text("Preview this object alone").clicked()
                                            && let Some(p) = &mut self.preview
                                        {
                                            p.mode = PreviewMode::Single(i);
                                            p.dirty = true;
                                        }
                                        if ui.small_button("x").clicked() {
                                            remove = Some(i);
                                        }
                                    });
                                }
                                if let Some(i) = remove {
                                    s.objects.remove(i);
                                    changed = true;
                                }
                                if ui.button("+ Add objects (.schem, .schematic, .nbt, .bo2)…").clicked()
                                    && let Some(files) = rfd::FileDialog::new().add_filter("Objects", &["schem", "schematic", "nbt", "bo2"]).pick_files()
                                {
                                    let mut errors = Vec::new();
                                    for f in files {
                                        match CustomObject::load(&f) {
                                            Ok(o) => s.objects.push(ObjectEntry { object: Arc::new(o), weight: 10 }),
                                            Err(e) => errors.push(format!("{}: {e:#}", f.display())),
                                        }
                                    }
                                    changed = true;
                                    if !errors.is_empty() {
                                        self.dialogs.error = Some(errors.join("\n"));
                                    }
                                }
                                ui.separator();
                                egui::Grid::new("obj_settings").num_columns(2).show(ui, |ui| {
                                    ui.label("Spacing");
                                    changed |= ui.add(egui::Slider::new(&mut s.spacing, 1..=64).suffix(" blocks")).on_hover_text("At most one object per spacing×spacing cell").changed();
                                    ui.end_row();
                                    ui.label("Density");
                                    changed |= ui.add(egui::Slider::new(&mut s.density, 0.0..=1.0)).changed();
                                    ui.end_row();
                                    ui.label("Randomise");
                                    ui.horizontal(|ui| {
                                        changed |= ui.checkbox(&mut s.random_rotation, "rotation").changed();
                                        changed |= ui.checkbox(&mut s.random_mirror, "mirroring").changed();
                                    });
                                    ui.end_row();
                                    ui.label("Vertical offset");
                                    ui.horizontal(|ui| {
                                        changed |= ui.add(egui::DragValue::new(&mut s.y_offset).range(-64..=64)).changed();
                                        ui.label("sink up to");
                                        changed |= ui.add(egui::DragValue::new(&mut s.y_jitter).range(0..=32)).changed();
                                    });
                                    ui.end_row();
                                    ui.label("Water");
                                    egui::ComboBox::from_id_salt("water_mode").selected_text(format!("{:?}", s.water)).show_ui(ui, |ui| {
                                        for m in [WaterMode::DryOnly, WaterMode::UnderwaterOnly, WaterMode::Anywhere] {
                                            changed |= ui.selectable_value(&mut s.water, m, format!("{m:?}")).changed();
                                        }
                                    });
                                    ui.end_row();
                                    ui.label("Max slope");
                                    changed |= ui.add(egui::Slider::new(&mut s.max_slope, 0.0..=90.0).suffix("°")).changed();
                                    ui.end_row();
                                    ui.label("");
                                    changed |= ui.checkbox(&mut s.replace_solid, "Replace terrain blocks (cliffs, sunken ruins)").changed();
                                    ui.end_row();
                                });
                                ui.collapsing("Only on terrains", |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        for t in terrains.iter().filter(|t| !(t.custom && t.name.starts_with("Custom "))) {
                                            let mut on = s.only_on_terrain.contains(&t.id);
                                            if ui.toggle_value(&mut on, &t.name).changed() {
                                                if on {
                                                    s.only_on_terrain.push(t.id);
                                                } else {
                                                    s.only_on_terrain.retain(|x| *x != t.id);
                                                }
                                                changed = true;
                                            }
                                        }
                                    });
                                });
                            }
                            LayerKind::Plants(s) => {
                                ui.separator();
                                let mut remove = None;
                                for (i, p) in s.plants.iter_mut().enumerate() {
                                    ui.horizontal(|ui| {
                                        changed |= block_picker(ui, ("plant", i), &mut p.block);
                                        changed |= ui.add(egui::DragValue::new(&mut p.weight).range(1..=1000).prefix("w ")).changed();
                                        if ui.small_button("x").clicked() {
                                            remove = Some(i);
                                        }
                                    });
                                }
                                if let Some(i) = remove {
                                    s.plants.remove(i);
                                    changed = true;
                                }
                                if ui.button("+ Plant").clicked() {
                                    s.plants.push(PlantEntry::single(StateId::parse("fern"), 10));
                                    changed = true;
                                }
                                changed |= ui.add(egui::Slider::new(&mut s.density, 0.0..=1.0).text("density")).changed();
                            }
                            LayerKind::GroundCover(s) => {
                                changed |= block_picker(ui, "cover", &mut s.block);
                                changed |= ui.add(egui::Slider::new(&mut s.thickness, 1..=16).text("thickness")).changed();
                            }
                            LayerKind::TerrainOverlay(s) => {
                                changed |= block_picker(ui, "overlay", &mut s.block);
                                changed |= ui.add(egui::Slider::new(&mut s.depth, 1..=16).text("depth")).changed();
                            }
                            _ => {}
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.label(RichText::new("3D preview (as exported)").strong());
                    if let (Some(gpu), Some(p)) = (&self.gpu, &mut self.preview) {
                        ui.horizontal(|ui| {
                            if ui.selectable_label(p.mode == PreviewMode::Placement, "Placement").clicked() {
                                p.mode = PreviewMode::Placement;
                                preview_changed = true;
                            }
                            preview_changed |= ui.add(egui::DragValue::new(&mut p.opts.radius).range(4..=120).prefix("radius ")).changed();
                            if def.data == DataSize::Nibble {
                                preview_changed |= ui.add(egui::DragValue::new(&mut p.opts.intensity).range(1..=15).prefix("intensity ")).changed();
                            }
                            preview_changed |= ui.add(egui::DragValue::new(&mut p.opts.hills).range(0.0..=40.0).prefix("hills ")).changed();
                            if ui.button("reroll").on_hover_text("New random seed").clicked() {
                                p.opts.seed = p.opts.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                                preview_changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Ground");
                            let mut t = p.opts.terrain;
                            terrain_combo(ui, "prev_terrain", &terrains, &mut t);
                            if t != p.opts.terrain {
                                p.opts.terrain = t;
                                preview_changed = true;
                            }
                        });
                        if preview_changed || changed {
                            p.dirty = true;
                        }
                        p.ui(ui, &gpu.rs, &gpu.r, &def, egui::vec2(440.0, 420.0));
                    } else {
                        ui.label("Needs the GPU renderer.");
                    }
                });
            });
        });
        if changed {
            if let Some(l) = self.world.layer_mut(id) {
                *l = def;
            }
            self.defs_changed();
        }
        if !open {
            self.dialogs.layer_editor.open = None;
        }
    }

    fn dlg_mask_editor(&mut self, ctx: &egui::Context) {
        if !self.dialogs.mask_editor.open {
            return;
        }
        let mut open = true;
        let mut save = false;
        let terrains = self.world.terrains.clone();
        let layers: Vec<(LayerId, String)> = self.world.layers.iter().map(|l| (l.id, l.name.clone())).collect();
        egui::Window::new("Custom mask").open(&mut open).default_width(460.0).show(ctx, |ui| {
            let m = &mut self.dialogs.mask_editor;
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut m.name);
                egui::color_picker::color_edit_button_srgb(ui, &mut m.color);
            });
            ui.horizontal(|ui| {
                ui.label("Columns matching");
                ui.selectable_value(&mut m.any, false, "all conditions");
                ui.selectable_value(&mut m.any, true, "any condition");
                ui.checkbox(&mut m.invert, "invert");
            });
            let mut remove = None;
            for (i, (c, not)) in m.conds.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(("cond", i)).selected_text(c.label()).show_ui(ui, |ui| {
                            let options = [
                                Cond::Height(80.0, 400.0, 4.0),
                                Cond::Slope(30.0, 90.0),
                                Cond::Terrain(vec![0]),
                                Cond::Layer(LayerId::DECIDUOUS, 1),
                                Cond::Water(true),
                                Cond::Biome(1),
                                Cond::Noise(64.0, 0.5, 0.05, 7),
                            ];
                            for o in options {
                                let label = o.label();
                                if ui.selectable_label(c.label() == label, label).clicked() {
                                    *c = o;
                                }
                            }
                        });
                        ui.checkbox(not, "not");
                        if ui.small_button("x").clicked() {
                            remove = Some(i);
                        }
                    });
                    match c {
                        Cond::Height(a, b, f) => {
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(a).prefix("from "));
                                ui.add(egui::DragValue::new(b).prefix("to "));
                                ui.add(egui::DragValue::new(f).range(0.0..=64.0).prefix("feather "));
                            });
                        }
                        Cond::Slope(a, b) => {
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(a).range(0.0..=90.0).suffix("°"));
                                ui.label("to");
                                ui.add(egui::DragValue::new(b).range(0.0..=90.0).suffix("°"));
                            });
                        }
                        Cond::Terrain(list) => {
                            ui.horizontal_wrapped(|ui| {
                                for t in terrains.iter().filter(|t| !(t.custom && t.name.starts_with("Custom "))) {
                                    let mut on = list.contains(&t.id);
                                    if ui.toggle_value(&mut on, &t.name).changed() {
                                        if on {
                                            list.push(t.id);
                                        } else {
                                            list.retain(|x| *x != t.id);
                                        }
                                    }
                                }
                            });
                        }
                        Cond::Layer(l, v) => {
                            ui.horizontal(|ui| {
                                egui::ComboBox::from_id_salt(("condl", i)).selected_text(layers.iter().find(|x| x.0 == *l).map(|x| x.1.clone()).unwrap_or_default()).show_ui(ui, |ui| {
                                    for (id, n) in &layers {
                                        ui.selectable_value(l, *id, n);
                                    }
                                });
                                ui.add(egui::DragValue::new(v).range(1..=255).prefix("≥ "));
                            });
                        }
                        Cond::Water(w) => {
                            ui.horizontal(|ui| {
                                ui.selectable_value(w, true, "under water");
                                ui.selectable_value(w, false, "on land");
                            });
                        }
                        Cond::Biome(b) => {
                            egui::ComboBox::from_id_salt(("condb", i)).selected_text(meadow_blocks::biomes().by_numeric(*b).map(|x| x.name.clone()).unwrap_or_default()).show_ui(ui, |ui| {
                                for bi in &meadow_blocks::biomes().list {
                                    ui.selectable_value(b, bi.numeric, &bi.name);
                                }
                            });
                        }
                        Cond::Noise(s, t, f, seed) => {
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(s).range(2.0..=4096.0).prefix("scale "));
                                ui.add(egui::DragValue::new(t).range(0.0..=1.0).speed(0.01).prefix("threshold "));
                                ui.add(egui::DragValue::new(f).range(0.0..=0.5).speed(0.005).prefix("soft "));
                                ui.add(egui::DragValue::new(seed).prefix("seed "));
                            });
                        }
                        Cond::Image(img) => {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}×{} image", img.width, img.height));
                                ui.add(egui::DragValue::new(&mut img.origin.0).prefix("x "));
                                ui.add(egui::DragValue::new(&mut img.origin.1).prefix("z "));
                                ui.add(egui::DragValue::new(&mut img.scale).range(0.05..=64.0).speed(0.05).prefix("blocks/px "));
                            });
                        }
                        Cond::Painted(_) => {
                            ui.label("Paint it with the Paint mask tool.");
                        }
                        Cond::Other(n) => {
                            ui.label(n.describe());
                        }
                    }
                });
            }
            if let Some(i) = remove {
                m.conds.remove(i);
            }
            if ui.button("+ Condition").clicked() {
                m.conds.push((Cond::Slope(30.0, 90.0), false));
            }
            ui.separator();
            save = ui.add_enabled(!m.conds.is_empty(), egui::Button::new("Save mask")).clicked();
        });
        if save {
            let mask = self.dialogs.mask_editor.build();
            match self.dialogs.mask_editor.index {
                Some(i) if i < self.world.masks.len() => self.world.masks[i] = mask,
                _ => {
                    self.world.masks.push(mask);
                    self.mask_filter = Some(self.world.masks.len() - 1);
                }
            }
            self.mask_overlay = true;
            self.defs_changed();
            open = false;
        }
        self.dialogs.mask_editor.open = open;
    }

    fn dlg_mask_image(&mut self, ctx: &egui::Context) {
        let Some((path, mut origin, mut scale, mut name)) = self.dialogs.mask_image.clone() else { return };
        let mut open = true;
        let mut create = false;
        egui::Window::new("Mask from image").open(&mut open).show(ctx, |ui| {
            ui.label(path.display().to_string());
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut name);
            });
            ui.horizontal(|ui| {
                ui.label("Top-left at");
                ui.add(egui::DragValue::new(&mut origin.0));
                ui.add(egui::DragValue::new(&mut origin.1));
            });
            ui.add(egui::DragValue::new(&mut scale).range(0.05..=64.0).speed(0.05).prefix("blocks per pixel "));
            if ui.button("Centre on world").clicked()
                && let Ok(img) = image::image_dimensions(&path)
            {
                origin = (-((img.0 as f32 * scale) as i32) / 2, -((img.1 as f32 * scale) as i32) / 2);
            }
            ui.label(RichText::new("White = fully selected, black = not selected; greys give soft masks.").small().weak());
            create = ui.button("Create mask").clicked();
        });
        if create {
            match MaskImage::load(&path, origin, scale) {
                Ok(img) => {
                    self.world.masks.push(CustomMask { name: name.clone(), node: MaskNode::Image(img), color: [80, 200, 255], invert: false });
                    self.mask_filter = Some(self.world.masks.len() - 1);
                    self.mask_overlay = true;
                    self.defs_changed();
                }
                Err(e) => self.error(format!("{e:#}")),
            }
            open = false;
        }
        self.dialogs.mask_image = if open { Some((path, origin, scale, name)) } else { None };
    }

    fn dlg_terrain_editor(&mut self, ctx: &egui::Context) {
        if !self.dialogs.terrain_editor.open {
            return;
        }
        let Some(mut def) = self.dialogs.terrain_editor.def.clone() else { return };
        let mut open = true;
        let mut save = false;
        egui::Window::new("Custom terrain").open(&mut open).default_width(520.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut def.name);
                ui.label(RichText::new(format!("slot {}", def.id)).small().weak());
            });
            ui.label(RichText::new("Strata from the surface down. Use any block: vanilla, Conquest Reforged (conquest:...) or imported mod blocks.").small().weak());
            let mut remove = None;
            for (si, st) in def.strata.iter_mut().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut st.depth).range(1..=64).prefix("depth "));
                        let kind = match st.material {
                            Material::Block(_) => 0,
                            Material::Mix(_) => 1,
                            Material::Patches { .. } => 2,
                            Material::Bands { .. } => 3,
                        };
                        let mut k = kind;
                        egui::ComboBox::from_id_salt(("matk", si)).selected_text(["Single block", "Random mix", "Patches", "Bands"][k]).show_ui(ui, |ui| {
                            for (i, n) in ["Single block", "Random mix", "Patches", "Bands"].iter().enumerate() {
                                ui.selectable_value(&mut k, i, *n);
                            }
                        });
                        if k != kind {
                            let blocks = st.material.blocks();
                            st.material = match k {
                                0 => Material::Block(blocks[0]),
                                1 => Material::Mix(blocks.iter().map(|b| (*b, 10)).collect()),
                                2 => Material::Patches { blocks: blocks.iter().map(|b| (*b, 10)).collect(), scale: 6.0 },
                                _ => Material::Bands { blocks, thickness: 2 },
                            };
                        }
                        if ui.small_button("x").clicked() {
                            remove = Some(si);
                        }
                    });
                    match &mut st.material {
                        Material::Block(b) => {
                            block_picker(ui, ("tb", si), b);
                        }
                        Material::Mix(list) | Material::Patches { blocks: list, .. } => {
                            let mut rm = None;
                            for (bi, (b, w)) in list.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    block_picker(ui, ("tm", si, bi), b);
                                    ui.add(egui::DragValue::new(w).range(1..=1000).prefix("w "));
                                    if ui.small_button("x").clicked() {
                                        rm = Some(bi);
                                    }
                                });
                            }
                            if let Some(i) = rm
                                && list.len() > 1
                            {
                                list.remove(i);
                            }
                            if ui.small_button("+ block").clicked() {
                                list.push((StateId::parse("stone"), 10));
                            }
                        }
                        Material::Bands { blocks, thickness } => {
                            ui.add(egui::DragValue::new(thickness).range(1..=16).prefix("band "));
                            for (bi, b) in blocks.iter_mut().enumerate() {
                                block_picker(ui, ("tbd", si, bi), b);
                            }
                            if ui.small_button("+ band").clicked() {
                                blocks.push(StateId::parse("terracotta"));
                            }
                        }
                    }
                    if let Material::Patches { scale, .. } = &mut st.material {
                        ui.add(egui::Slider::new(scale, 1.0..=64.0).text("patch size"));
                    }
                });
            }
            if let Some(i) = remove
                && def.strata.len() > 1
            {
                def.strata.remove(i);
            }
            if ui.button("+ Stratum").clicked() {
                def.strata.push(Stratum { depth: 3, material: Material::Block(StateId::parse("dirt")) });
            }
            ui.horizontal(|ui| {
                ui.label("Automatic biome");
                egui::ComboBox::from_id_salt("tbiome").selected_text(meadow_blocks::biomes().by_numeric(def.auto_biome).map(|b| b.name.clone()).unwrap_or_default()).show_ui(ui, |ui| {
                    for b in &meadow_blocks::biomes().list {
                        ui.selectable_value(&mut def.auto_biome, b.numeric, &b.name);
                    }
                });
            });
            def.color = def.surface_color();
            save = ui.button("Save terrain").clicked();
        });
        self.dialogs.terrain_editor.def = Some(def.clone());
        if save {
            def.custom = true;
            if (def.id as usize) < self.world.terrains.len() {
                self.world.terrains[def.id as usize] = def.clone();
            }
            self.terrain = def.id;
            self.defs_changed();
            open = false;
        }
        self.dialogs.terrain_editor.open = open;
    }

    fn dlg_world_settings(&mut self, ctx: &egui::Context) {
        if !self.dialogs.world_settings.open {
            return;
        }
        let mut open = true;
        let mut apply = false;
        egui::Window::new("World settings").open(&mut open).show(ctx, |ui| {
            let d = &mut self.dialogs.world_settings;
            egui::Grid::new("ws").num_columns(2).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut d.name);
                ui.end_row();
                ui.label("Seed");
                ui.add(egui::DragValue::new(&mut d.seed));
                ui.end_row();
                ui.label("Build limits");
                ui.vertical(|ui| {
                    limits_picker(ui, "ws_limits", &mut d.limits);
                });
                ui.end_row();
                ui.label("Default water level");
                ui.add(egui::DragValue::new(&mut d.water));
                ui.end_row();
                ui.label("Underground");
                ui.vertical(|ui| {
                    ui.checkbox(&mut d.deepslate, "Deepslate below y=0");
                    ui.checkbox(&mut d.bedrock, "Bedrock floor");
                });
                ui.end_row();
            });
            apply = ui.button("Apply").clicked();
        });
        if apply {
            let d = &self.dialogs.world_settings;
            self.world.name = d.name.clone();
            self.world.seed = d.seed;
            if d.limits != self.world.limits {
                let l = d.limits;
                self.world.set_limits(l);
            }
            let (w, ds, b) = (d.water, d.deepslate, d.bedrock);
            let s = self.world.surface_mut();
            s.default_water_level = w;
            s.subsurface.deepslate = ds;
            s.subsurface.bedrock_floor = b;
            self.defs_changed();
            open = false;
        }
        self.dialogs.world_settings.open = open;
    }

    /// WorldPainter's Configure View dialog.
    fn dlg_configure_view(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.configure_view;
        if !open {
            return;
        }
        egui::Window::new("Configure view").open(&mut open).collapsible(false).show(ctx, |ui| {
            let s = &mut self.settings;
            egui::Grid::new("cfg_view").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.checkbox(&mut s.show_grid, "Grid");
                ui.horizontal(|ui| {
                    ui.label("size");
                    ui.add(egui::DragValue::new(&mut s.grid_size).range(4..=4096).suffix(" blocks"));
                });
                ui.end_row();
                ui.checkbox(&mut s.contours, "Contour lines");
                ui.horizontal(|ui| {
                    ui.label("separation");
                    ui.add(egui::DragValue::new(&mut s.contour_spacing).range(1..=256).suffix(" blocks"));
                });
                ui.end_row();
                ui.label("Light from");
                egui::ComboBox::from_id_salt("light").selected_text(s.light.name()).show_ui(ui, |ui| {
                    for l in crate::map2d::LightOrigin::ALL {
                        ui.selectable_value(&mut s.light, l, l.name());
                    }
                });
                ui.end_row();
                ui.label("Hill shading");
                ui.add(egui::Slider::new(&mut s.hillshade, 0.0..=3.0).text("× WorldPainter"));
                ui.end_row();
                ui.label("Background");
                ui.horizontal(|ui| {
                    egui::color_picker::color_edit_button_srgb(ui, &mut s.background);
                    if ui.small_button("default").clicked() {
                        s.background = crate::settings::DEFAULT_BACKGROUND;
                    }
                });
                ui.end_row();
                ui.checkbox(&mut s.show_layer_overlays, "Overlays (layers)");
                ui.checkbox(&mut s.show_biomes, "Show biomes");
                ui.end_row();
            });
        });
        if !open {
            self.settings.save();
        }
        self.dialogs.configure_view = open;
    }

    fn dlg_settings(&mut self, ctx: &egui::Context) {
        let mut open = self.dialogs.settings_open;
        let mut changed = false;
        egui::Window::new("Preferences").open(&mut open).default_width(440.0).show(ctx, |ui| {
            let s = &mut self.settings;
            ui.heading("Interface");
            egui::Grid::new("pref_ui").num_columns(2).show(ui, |ui| {
                ui.label("UI scale");
                changed |= ui.add(egui::Slider::new(&mut s.ui_scale, 0.5..=3.0).fixed_decimals(2)).changed();
                ui.end_row();
                ui.label("Text size");
                changed |= ui.add(egui::Slider::new(&mut s.font_size, 9.0..=24.0)).changed();
                ui.end_row();
                ui.label("Theme");
                ui.horizontal(|ui| {
                    changed |= ui.selectable_value(&mut s.theme, ThemeChoice::Dark, "Dark").changed();
                    changed |= ui.selectable_value(&mut s.theme, ThemeChoice::Light, "Light").changed();
                    changed |= ui.selectable_value(&mut s.theme, ThemeChoice::System, "System").changed();
                });
                ui.end_row();
                ui.label("Accent colour");
                changed |= egui::color_picker::color_edit_button_srgb(ui, &mut s.accent).changed();
                ui.end_row();
                ui.label("Corner rounding");
                changed |= ui.add(egui::Slider::new(&mut s.rounding, 0.0..=12.0)).changed();
                ui.end_row();
                ui.label("");
                changed |= ui.checkbox(&mut s.compact, "Compact spacing").changed();
                ui.end_row();
            });
            ui.collapsing("Panel layout", |ui| {
                let n = s.panels.len();
                let mut swap = None;
                egui::Grid::new("panels").show(ui, |ui| {
                    for i in 0..n {
                        let p = &mut s.panels[i];
                        ui.label(p.panel.title());
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut p.dock, Dock::Left, "Left");
                            ui.selectable_value(&mut p.dock, Dock::Right, "Right");
                            ui.selectable_value(&mut p.dock, Dock::Hidden, "Hidden");
                        });
                        ui.checkbox(&mut p.open, "expanded");
                        if ui.small_button("up").clicked() && i > 0 {
                            swap = Some((i, i - 1));
                        }
                        if ui.small_button("down").clicked() && i + 1 < n {
                            swap = Some((i, i + 1));
                        }
                        ui.end_row();
                    }
                });
                if let Some((a, b)) = swap {
                    s.panels.swap(a, b);
                }
            });
            ui.heading("3D view");
            egui::Grid::new("pref_3d").num_columns(2).show(ui, |ui| {
                ui.label("View distance");
                ui.add(egui::Slider::new(&mut s.view_distance, 256.0..=20000.0).logarithmic(true).suffix(" blocks"));
                ui.end_row();
                ui.label("Full detail within");
                ui.add(egui::Slider::new(&mut s.detail_distance, 64.0..=2048.0).logarithmic(true).suffix(" blocks"));
                ui.end_row();
                ui.label("Trees & objects within");
                ui.add(egui::Slider::new(&mut s.decoration_distance, 0.0..=1024.0).suffix(" blocks"));
                ui.end_row();
                ui.label("");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut s.msaa, "Anti-aliasing");
                    ui.checkbox(&mut s.fog, "Fog");
                    ui.checkbox(&mut s.show_plants_3d, "Plants");
                });
                ui.end_row();
                ui.label("Fly speed");
                ui.add(egui::Slider::new(&mut s.fly_speed, 5.0..=500.0).logarithmic(true));
                ui.end_row();
            });
            ui.heading("2D map");
            ui.label(RichText::new("Grid, contours, background and light are under View › Configure view (Ctrl+V).").small().weak());
            ui.heading("Files");
            egui::Grid::new("pref_files").num_columns(2).show(ui, |ui| {
                ui.label("Minecraft saves");
                ui.horizontal(|ui| {
                    ui.label(s.minecraft_saves.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "(not found)".into()));
                    if ui.button("…").clicked()
                        && let Some(p) = rfd::FileDialog::new().pick_folder()
                    {
                        s.minecraft_saves = Some(p);
                    }
                });
                ui.end_row();
                ui.label("Autosave every");
                ui.add(egui::DragValue::new(&mut s.autosave_minutes).range(0..=120).suffix(" min (0 = off)"));
                ui.end_row();
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Reset to defaults").clicked() {
                    *s = crate::settings::Settings { recent_files: s.recent_files.clone(), recent_scripts: s.recent_scripts.clone(), ..Default::default() };
                    changed = true;
                }
                if ui.button("Save").clicked() {
                    s.save();
                }
            });
        });
        if changed {
            self.settings_applied = false;
            self.map.clear_cache();
        }
        if self.dialogs.settings_open && !open {
            self.settings.save();
        }
        self.dialogs.settings_open = open;
    }
}
