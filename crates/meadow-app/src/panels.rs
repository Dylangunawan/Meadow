//! Side panel contents.

use eframe::egui::{self, Color32, RichText};
use meadow_core::layer::{DataSize, GroundCoverSettings, LayerDef, LayerId, LayerKind, ObjectLayerSettings, PlantLayerSettings, TerrainOverlaySettings};
use meadow_core::mask::{CustomMask, MaskNode};
use meadow_core::ops::{BrushShape, Falloff};

use crate::app::{MeadowApp, PaintKind, ToolChoice};

fn swatch(ui: &mut egui::Ui, c: [u8; 3]) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 2.0, Color32::from_rgb(c[0], c[1], c[2]));
    ui.painter().rect_stroke(r, 2.0, egui::Stroke::new(1.0, Color32::from_black_alpha(120)), egui::StrokeKind::Inside);
}

pub fn new_layer(name: &str, kind: LayerKind, data: DataSize, color: [u8; 3]) -> LayerDef {
    LayerDef { id: LayerId(0), name: name.into(), description: String::new(), kind, data, default: 0, color, priority: 50, hidden: false }
}

impl MeadowApp {
    pub fn panel_tools(&mut self, ui: &mut egui::Ui) {
        let button = |ui: &mut egui::Ui, current: ToolChoice, info: &crate::app::ToolInfo| -> bool {
            let w = (ui.available_width() / 4.0 - 4.0).max(48.0);
            ui.add_sized([w, 24.0], egui::Button::selectable(current == info.tool, info.name)).on_hover_text(info.tip).clicked()
        };
        // WorldPainter's tool grid, 4 columns, in its order.
        egui::Grid::new("wp_tools").num_columns(4).spacing([4.0, 4.0]).show(ui, |ui| {
            for (k, info) in ToolChoice::WORLDPAINTER.iter().enumerate() {
                if button(ui, self.tool, info) {
                    self.tool = info.tool;
                }
                if k % 4 == 3 {
                    ui.end_row();
                }
            }
            if ui.add_sized([(ui.available_width() - 4.0).max(48.0), 24.0], egui::Button::new("Global ops")).on_hover_text("Fill, clear or reset the whole map through a filter (Ctrl+G)").clicked() {
                self.dialogs.fill.open = true;
            }
            ui.end_row();
        });
        ui.label(RichText::new("Meadow extras").small().weak());
        egui::Grid::new("meadow_tools").num_columns(4).spacing([4.0, 4.0]).show(ui, |ui| {
            for (k, info) in ToolChoice::EXTRAS.iter().enumerate() {
                if button(ui, self.tool, info) {
                    self.tool = info.tool;
                }
                if k % 4 == 3 {
                    ui.end_row();
                }
            }
        });
        ui.separator();
        // Tool settings
        match self.tool {
            ToolChoice::SprayPaint | ToolChoice::Pencil | ToolChoice::Fill => {
                ui.label(format!("Paint: {}", self.paint_name()));
                ui.label(RichText::new("Pick a terrain, layer or biome to paint with. Right-click removes it.").small().weak());
            }
            ToolChoice::SetHeight => {
                ui.horizontal(|ui| {
                    ui.label("Target height");
                    let l = self.dim().limits;
                    ui.add(egui::DragValue::new(&mut self.height_level).range(l.min_y as f32..=(l.max_y - 1) as f32));
                    if let Some((x, y)) = self.map.hover
                        && ui.small_button("pick").on_hover_text("Use the height under the cursor").clicked()
                    {
                        self.height_level = self.dim().height(x, y).unwrap_or(self.height_level);
                    }
                });
            }
            ToolChoice::WaterLevel => {
                ui.horizontal(|ui| {
                    ui.label("Water level");
                    let l = self.dim().limits;
                    ui.add(egui::DragValue::new(&mut self.water_level).range(l.min_y..=l.max_y - 1));
                });
            }
            ToolChoice::Tiles => {
                ui.horizontal(|ui| {
                    ui.label("New tile height");
                    ui.add(egui::DragValue::new(&mut self.height_level));
                });
            }
            ToolChoice::None => {
                ui.label(RichText::new("No tool selected (Esc).").small().weak());
            }
            _ => {}
        }
    }

    /// WorldPainter's brush palette (falloff x shape) and brush settings.
    pub fn panel_brush(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("brushes").num_columns(2).spacing([4.0, 4.0]).show(ui, |ui| {
            for f in Falloff::ALL {
                for shape in [BrushShape::Circle, BrushShape::Square] {
                    let selected = self.brush.falloff == f && self.brush.shape == shape;
                    let label = format!("{} {}", f.name(), if shape == BrushShape::Circle { "circle" } else { "square" });
                    let w = (ui.available_width() / 2.0 - 4.0).max(70.0);
                    let r = ui.add_sized([w, 22.0], egui::Button::selectable(selected, label));
                    if r.clicked() {
                        self.brush.falloff = f;
                        self.brush.shape = shape;
                    }
                }
                ui.end_row();
            }
        });
        ui.separator();
        egui::Grid::new("brush_settings").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Intensity");
            let mut pct = (self.brush.intensity * 100.0).round();
            if ui.add(egui::Slider::new(&mut pct, 2.0..=100.0).suffix(" %")).on_hover_text("Keys 1–9 set 10–90 %, 0 sets 100 %").changed() {
                self.brush.intensity = pct / 100.0;
            }
            ui.end_row();
            ui.label("Rotation");
            ui.add(egui::Slider::new(&mut self.brush.rotation, -180.0..=180.0).suffix("°").step_by(1.0)).on_hover_text("Alt + mouse wheel");
            ui.end_row();
            ui.label("Radius");
            ui.add(egui::Slider::new(&mut self.brush.radius, 0.0..=crate::map2d::MAX_RADIUS).step_by(1.0)).on_hover_text("Mouse wheel (Shift: by one)");
            ui.end_row();
            ui.label("Roughness");
            ui.add(egui::Slider::new(&mut self.brush.roughness, 0.0..=1.0)).on_hover_text("Meadow extra: noisy brush edges");
            ui.end_row();
            ui.label("Only on");
            let current = self.mask_filter.and_then(|i| self.world.masks.get(i)).map(|m| m.name.clone()).unwrap_or_else(|| "everything".into());
            egui::ComboBox::from_id_salt("filter").selected_text(current).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.mask_filter, None, "everything");
                for (i, m) in self.world.masks.iter().enumerate() {
                    ui.selectable_value(&mut self.mask_filter, Some(i), format!("Custom mask: {}", m.name));
                }
            });
            ui.end_row();
        });
        if self.mask_filter.is_some() {
            ui.checkbox(&mut self.mask_overlay, "Highlight filter on the map");
        }
    }

    pub fn panel_terrain(&mut self, ui: &mut egui::Ui) {
        let mut edit = None;
        let mut select = None;
        ui.horizontal_wrapped(|ui| {
            for t in &self.world.terrains {
                if t.custom && t.name.starts_with("Custom ") && t.strata.len() == 1 && t.strata[0].material == meadow_core::terrain::Material::Block(meadow_blocks::StateId::parse("stone")) {
                    continue; // unused custom slot
                }
                let sel = self.terrain == t.id && self.paint_kind == PaintKind::Terrain;
                let c = t.color;
                let (rect, r) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                let stroke = if sel { egui::Stroke::new(2.5, ui.visuals().selection.stroke.color) } else { egui::Stroke::new(1.0, Color32::from_black_alpha(140)) };
                ui.painter().rect_filled(rect.shrink(2.0), 3.0, Color32::from_rgb(c[0], c[1], c[2]));
                ui.painter().rect_stroke(rect.shrink(1.0), 3.0, stroke, egui::StrokeKind::Inside);
                let r = r.on_hover_text(&t.name);
                if r.clicked() {
                    select = Some(t.id);
                }
                if t.custom {
                    r.context_menu(|ui| {
                        if ui.button("Edit custom terrain…").clicked() {
                            edit = Some(t.id);
                            ui.close();
                        }
                    });
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.world.terrain(self.terrain).name).strong());
            if ui.small_button("+ Custom terrain").on_hover_text("Build a terrain from any blocks, including Conquest Reforged and imported mod blocks").clicked() {
                edit = Some(255);
            }
        });
        if let Some(id) = select {
            self.terrain = id;
            self.select_paint(PaintKind::Terrain);
        }
        if let Some(id) = edit {
            self.dialogs.terrain_editor.open_for(&self.world, id);
        }
    }

    pub fn panel_layers(&mut self, ui: &mut egui::Ui) {
        let mut select = None;
        let mut edit = None;
        let mut remove = None;
        let mut toggle_overlay = None;
        for l in &self.world.layers {
            if matches!(l.kind, LayerKind::Biome | LayerKind::PaintedMask | LayerKind::Populate) {
                continue;
            }
            ui.horizontal(|ui| {
                let mut shown = !self.hidden_layers.contains(&l.id);
                if ui.checkbox(&mut shown, "").on_hover_text("Whether or not to display this layer").changed() {
                    toggle_overlay = Some(l.id);
                }
                swatch(ui, l.color);
                let sel = self.layer == Some(l.id) && self.paint_kind == PaintKind::Layer;
                let r = ui.selectable_label(sel, &l.name).on_hover_text(&l.description);
                if r.clicked() {
                    select = Some(l.id);
                }
                if l.id.is_custom() {
                    if ui.small_button("edit").on_hover_text("Edit layer & 3D preview").clicked() {
                        edit = Some(l.id);
                    }
                    r.context_menu(|ui| {
                        if ui.button("Edit…").clicked() {
                            edit = Some(l.id);
                            ui.close();
                        }
                        if ui.button("Delete layer").clicked() {
                            remove = Some(l.id);
                            ui.close();
                        }
                    });
                } else if matches!(l.kind, LayerKind::Trees(_)) && ui.small_button("3D").on_hover_text("Preview in 3D").clicked() {
                    edit = Some(l.id);
                }
            });
        }
        if let Some(id) = toggle_overlay {
            if let Some(i) = self.hidden_layers.iter().position(|x| *x == id) {
                self.hidden_layers.remove(i);
            } else {
                self.hidden_layers.push(id);
            }
        }
        if let Some(id) = select {
            self.layer = Some(id);
            self.select_paint(PaintKind::Layer);
            self.hidden_layers.retain(|x| *x != id);
        }
        if let Some(id) = remove {
            self.world.remove_layer(id);
            if self.layer == Some(id) {
                self.layer = None;
            }
            self.defs_changed();
        }
        if let Some(id) = edit {
            self.dialogs.layer_editor.open = Some(id);
            if let Some(p) = &mut self.preview {
                p.dirty = true;
                p.mode = crate::view3d::PreviewMode::Placement;
            }
        }
        ui.separator();
        ui.menu_button("+ New layer", |ui| {
            let mut added = None;
            if ui.button("Custom objects (schematics: trees, rocks, cliffs…)").clicked() {
                added = Some(new_layer("Objects", LayerKind::CustomObjects(ObjectLayerSettings::default()), DataSize::Nibble, [150, 110, 70]));
            }
            if ui.button("Plants").clicked() {
                let plants = vec![
                    meadow_core::layer::PlantEntry::single(meadow_blocks::StateId::parse("short_grass"), 10),
                    meadow_core::layer::PlantEntry::single(meadow_blocks::StateId::parse("poppy"), 1),
                ];
                added = Some(new_layer("Plants", LayerKind::Plants(PlantLayerSettings { plants, density: 0.5, only_on_terrain: Vec::new() }), DataSize::Nibble, [90, 200, 90]));
            }
            if ui.button("Ground cover (blocks on top)").clicked() {
                added = Some(new_layer(
                    "Ground cover",
                    LayerKind::GroundCover(GroundCoverSettings { block: meadow_blocks::StateId::parse("moss_carpet"), thickness: 1, noisy_edge: false }),
                    DataSize::Bit,
                    [120, 160, 60],
                ));
            }
            if ui.button("Terrain overlay (replace surface blocks)").clicked() {
                added = Some(new_layer(
                    "Overlay",
                    LayerKind::TerrainOverlay(TerrainOverlaySettings { block: meadow_blocks::StateId::parse("coarse_dirt"), depth: 1 }),
                    DataSize::Bit,
                    [140, 100, 60],
                ));
            }
            if let Some(def) = added {
                let id = self.world.add_layer(def);
                self.layer = Some(id);
                self.paint_kind = PaintKind::Layer;
                self.dialogs.layer_editor.open = Some(id);
                self.defs_changed();
                ui.close();
            }
        });
    }

    pub fn panel_masks(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        let mut edit = None;
        for (i, m) in self.world.masks.iter().enumerate() {
            ui.horizontal(|ui| {
                swatch(ui, m.color);
                let r = ui.selectable_label(self.mask_filter == Some(i), &m.name).on_hover_text(m.node.describe());
                if r.clicked() {
                    self.mask_filter = if self.mask_filter == Some(i) { None } else { Some(i) };
                }
                if ui.small_button("edit").clicked() {
                    edit = Some(i);
                }
                if ui.small_button("x").on_hover_text("Delete").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.world.masks.remove(i);
            self.mask_filter = None;
            self.defs_changed();
        }
        if let Some(i) = edit {
            self.dialogs.mask_editor.open_for(&self.world, Some(i));
        }
        if self.world.masks.is_empty() {
            ui.label(RichText::new("Masks restrict brushes and fills: by height, slope, terrain, layers, water, noise, images or hand painting, combined with and/or/not.").small().weak());
        }
        ui.horizontal(|ui| {
            if ui.button("+ New mask…").clicked() {
                self.dialogs.mask_editor.open_for(&self.world, None);
            }
            if ui.button("From image…").clicked() {
                self.dialogs.mask_from_image();
            }
        });
        ui.horizontal(|ui| {
            if ui.button("+ Painted mask").on_hover_text("A mask you paint by hand with the Paint mask tool").clicked() {
                let n = self.world.layers.iter().filter(|l| matches!(l.kind, LayerKind::PaintedMask)).count() + 1;
                let mut def = new_layer(&format!("Painted mask {n}"), LayerKind::PaintedMask, DataSize::Byte, [255, 60, 60]);
                def.hidden = true;
                let id = self.world.add_layer(def);
                self.world.masks.push(CustomMask { name: format!("Painted mask {n}"), node: MaskNode::Painted(id), color: [255, 60, 60], invert: false });
                self.mask_filter = Some(self.world.masks.len() - 1);
                self.paint_mask_layer = Some(id);
                self.tool = ToolChoice::Mask;
                self.defs_changed();
            }
        });
        if self.tool == ToolChoice::Mask {
            let names: Vec<(LayerId, String)> =
                self.world.layers.iter().filter(|l| matches!(l.kind, LayerKind::PaintedMask)).map(|l| (l.id, l.name.clone())).collect();
            egui::ComboBox::from_id_salt("paintmask")
                .selected_text(self.paint_mask_layer.and_then(|id| names.iter().find(|n| n.0 == id)).map(|n| n.1.clone()).unwrap_or_else(|| "Select…".into()))
                .show_ui(ui, |ui| {
                    for (id, n) in &names {
                        ui.selectable_value(&mut self.paint_mask_layer, Some(*id), n);
                    }
                });
        }
    }

    pub fn panel_biomes(&mut self, ui: &mut egui::Ui) {
        let filter_id = egui::Id::unique("biome_filter");
        let mut filter: String = ui.data_mut(|d| d.get_temp(filter_id).unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut filter).hint_text("Search biomes"));
        ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
        let f = filter.to_lowercase();
        egui::ScrollArea::vertical().id_salt("biomes").max_height(260.0).show(ui, |ui| {
            for b in &meadow_blocks::biomes().list {
                if !f.is_empty() && !b.name.to_lowercase().contains(&f) {
                    continue;
                }
                ui.horizontal(|ui| {
                    swatch(ui, b.color);
                    if ui.selectable_label(self.biome == b.numeric && self.paint_kind == PaintKind::Biome, &b.name).clicked() {
                        self.biome = b.numeric;
                        self.select_paint(PaintKind::Biome);
                        self.settings.show_biomes = true;
                    }
                });
            }
        });
    }

    pub fn panel_info(&mut self, ui: &mut egui::Ui) {
        let d = self.dim();
        let (lo, hi) = d.tiles().fold((f32::MAX, f32::MIN), |(a, b), t| (a.min(t.min_height()), b.max(t.max_height())));
        egui::Grid::new("info").num_columns(2).show(ui, |ui| {
            ui.label("World");
            ui.label(&self.world.name);
            ui.end_row();
            ui.label("Tiles");
            ui.label(format!("{} ({} blocks²)", d.tile_count(), d.tile_count() * 128 * 128));
            ui.end_row();
            ui.label("Build limits");
            ui.label(format!("{} .. {}", d.limits.min_y, d.limits.max_y));
            ui.end_row();
            if lo <= hi {
                ui.label("Heights");
                ui.label(format!("{lo:.0} .. {hi:.0}"));
                ui.end_row();
            }
            ui.label("Export target");
            ui.label(self.world.export.version.label());
            ui.end_row();
            ui.label("Blocks known");
            ui.label(format!("{}", meadow_blocks::registry().read().len()));
            ui.end_row();
        });
    }
}
