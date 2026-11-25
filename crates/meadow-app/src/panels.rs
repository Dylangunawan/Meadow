//! Side panel contents.

use eframe::egui::{self, Color32, RichText};
use meadow_core::layer::{DataSize, GroundCoverSettings, LayerDef, LayerId, LayerKind, ObjectLayerSettings, PlantLayerSettings, TerrainOverlaySettings};
use meadow_core::mask::{CustomMask, MaskNode};
use meadow_core::ops::{BrushShape, Falloff};

use crate::app::{MeadowApp, ToolChoice};

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
        let groups: [(&str, &[ToolChoice]); 4] = [
            ("Height", &[ToolChoice::Raise, ToolChoice::Lower, ToolChoice::Smooth, ToolChoice::Flatten, ToolChoice::SetHeight, ToolChoice::Roughen, ToolChoice::Terrace]),
            ("Paint", &[ToolChoice::Terrain, ToolChoice::Layer, ToolChoice::EraseLayer, ToolChoice::Biome, ToolChoice::Mask]),
            ("Water", &[ToolChoice::WaterLevel, ToolChoice::Flood, ToolChoice::Sponge]),
            ("World", &[ToolChoice::Tiles, ToolChoice::Spawn]),
        ];
        for (g, tools) in groups {
            ui.label(RichText::new(g).small().weak());
            ui.horizontal_wrapped(|ui| {
                for t in tools {
                    let (_, name, tip) = ToolChoice::ALL.iter().find(|x| x.0 == *t).unwrap();
                    if ui.selectable_label(self.tool == *t, *name).on_hover_text(*tip).clicked() {
                        self.tool = *t;
                    }
                }
            });
        }
        ui.separator();
        match self.tool {
            ToolChoice::SetHeight => {
                ui.horizontal(|ui| {
                    ui.label("Target height");
                    let l = self.dim().limits;
                    ui.add(egui::DragValue::new(&mut self.height_level).range(l.min_y as f32..=(l.max_y - 1) as f32));
                    if let Some((x, y)) = self.map.hover
                        && ui.small_button("pick").on_hover_text("Use the height under the cursor (hover the map, then press P)").clicked()
                    {
                        self.height_level = self.dim().height(x, y).unwrap_or(self.height_level);
                    }
                });
            }
            ToolChoice::WaterLevel | ToolChoice::Flood => {
                ui.horizontal(|ui| {
                    ui.label("Water level");
                    let l = self.dim().limits;
                    ui.add(egui::DragValue::new(&mut self.water_level).range(l.min_y..=l.max_y - 1));
                });
                if self.tool == ToolChoice::Flood {
                    ui.label(RichText::new("Click inside a basin to fill it.").small());
                }
            }
            ToolChoice::Tiles => {
                ui.horizontal(|ui| {
                    ui.label("New tile height");
                    ui.add(egui::DragValue::new(&mut self.height_level));
                });
                ui.label(RichText::new("Drag to add tiles, shift-drag removes them.").small());
            }
            ToolChoice::Layer | ToolChoice::EraseLayer => {
                let name = self.layer.and_then(|l| self.world.layer(l)).map(|l| l.name.clone()).unwrap_or_else(|| "none".into());
                ui.label(format!("Layer: {name}"));
            }
            ToolChoice::Terrain => {
                ui.label(format!("Terrain: {}", self.world.terrain(self.terrain).name));
            }
            ToolChoice::Biome => {
                let n = meadow_blocks::biomes().by_numeric(self.biome).map(|b| b.name.clone()).unwrap_or_default();
                ui.label(format!("Biome: {n} (shift resets to automatic)"));
            }
            _ => {}
        }
        ui.label(RichText::new("Shift + paint erases · [ ] brush size · Ctrl+Z undo").small().weak());
    }

    pub fn panel_brush(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("brush").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Radius");
            ui.add(egui::Slider::new(&mut self.brush.radius, 0.5..=300.0).logarithmic(true));
            ui.end_row();
            ui.label("Intensity");
            ui.add(egui::Slider::new(&mut self.brush.intensity, 0.01..=1.0));
            ui.end_row();
            ui.label("Shape");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.brush.shape, BrushShape::Circle, "Circle");
                ui.selectable_value(&mut self.brush.shape, BrushShape::Square, "Square");
            });
            ui.end_row();
            ui.label("Falloff");
            egui::ComboBox::from_id_salt("falloff").selected_text(format!("{:?}", self.brush.falloff)).show_ui(ui, |ui| {
                for f in [Falloff::Smooth, Falloff::Linear, Falloff::Sharp, Falloff::Constant] {
                    ui.selectable_value(&mut self.brush.falloff, f, format!("{f:?}"));
                }
            });
            ui.end_row();
            ui.label("Roughness");
            ui.add(egui::Slider::new(&mut self.brush.roughness, 0.0..=1.0));
            ui.end_row();
            ui.label("Filter");
            let current = self.mask_filter.and_then(|i| self.world.masks.get(i)).map(|m| m.name.clone()).unwrap_or_else(|| "None".into());
            egui::ComboBox::from_id_salt("filter").selected_text(current).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.mask_filter, None, "None");
                for (i, m) in self.world.masks.iter().enumerate() {
                    ui.selectable_value(&mut self.mask_filter, Some(i), &m.name);
                }
            });
            ui.end_row();
        });
        if self.mask_filter.is_some() {
            ui.checkbox(&mut self.mask_overlay, "Highlight filter on the map");
        }
        ui.label(RichText::new("Filters are custom masks: create them in the Custom masks panel.").small().weak());
    }

    pub fn panel_terrain(&mut self, ui: &mut egui::Ui) {
        let mut edit = None;
        ui.horizontal_wrapped(|ui| {
            for t in &self.world.terrains {
                if t.custom && t.name.starts_with("Custom ") && t.strata.len() == 1 && t.strata[0].material == meadow_core::terrain::Material::Block(meadow_blocks::StateId::parse("stone")) {
                    continue; // unused custom slot
                }
                let sel = self.terrain == t.id;
                let c = t.color;
                let (rect, r) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                let stroke = if sel { egui::Stroke::new(2.5, ui.visuals().selection.stroke.color) } else { egui::Stroke::new(1.0, Color32::from_black_alpha(140)) };
                ui.painter().rect_filled(rect.shrink(2.0), 3.0, Color32::from_rgb(c[0], c[1], c[2]));
                ui.painter().rect_stroke(rect.shrink(1.0), 3.0, stroke, egui::StrokeKind::Inside);
                let r = r.on_hover_text(&t.name);
                if r.clicked() {
                    self.terrain = t.id;
                    if !matches!(self.tool, ToolChoice::Terrain) {
                        self.tool = ToolChoice::Terrain;
                    }
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
                let mut shown = self.overlays.contains(&l.id);
                if ui.checkbox(&mut shown, "").on_hover_text("Show on the 2D map").changed() {
                    toggle_overlay = Some(l.id);
                }
                swatch(ui, l.color);
                let sel = self.layer == Some(l.id);
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
            if let Some(i) = self.overlays.iter().position(|x| *x == id) {
                self.overlays.remove(i);
            } else {
                self.overlays.push(id);
            }
        }
        if let Some(id) = select {
            self.layer = Some(id);
            if !matches!(self.tool, ToolChoice::Layer | ToolChoice::EraseLayer) {
                self.tool = ToolChoice::Layer;
            }
            if !self.overlays.contains(&id) {
                self.overlays.push(id);
            }
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
        if let Some(l) = self.layer.and_then(|l| self.world.layer(l))
            && l.data == DataSize::Nibble
        {
            ui.horizontal(|ui| {
                ui.label("Intensity");
                ui.add(egui::Slider::new(&mut self.layer_value, 1..=15));
            });
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
                self.overlays.push(id);
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
                self.overlays.push(id);
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
                    if ui.selectable_label(self.biome == b.numeric, &b.name).clicked() {
                        self.biome = b.numeric;
                        self.tool = ToolChoice::Biome;
                        if !self.overlays.contains(&LayerId::BIOME) {
                            self.overlays.push(LayerId::BIOME);
                        }
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
