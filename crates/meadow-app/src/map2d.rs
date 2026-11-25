//! The 2D top-down editor view.

use std::time::Instant;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke as EStroke, Vec2};
use meadow_core::layer::{LayerDef, LayerId, LayerKind};
use meadow_core::mask::CustomMask;
use meadow_core::tile::{TILE_SHIFT, TILE_SIZE, local_index};
use meadow_core::{Dimension, World};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

/// Everything that affects how tiles look (hashed into the texture cache key).
#[derive(Clone)]
pub struct MapStyle {
    pub hillshade: f32,
    pub contour: i32,
    pub overlays: Vec<LayerId>,
    pub mask: Option<CustomMask>,
    pub defs_gen: u64,
}

impl MapStyle {
    fn key(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x100000001b3);
        mix((self.hillshade * 100.0) as u64);
        mix(self.contour as u64);
        for l in &self.overlays {
            mix(l.0 as u64 + 7);
        }
        mix(self.mask.as_ref().map(|m| m.name.len() as u64 + 99).unwrap_or(0));
        mix(self.defs_gen);
        h
    }
}

struct CachedTile {
    rev: u64,
    style: u64,
    tex: egui::TextureHandle,
}

pub struct Map2D {
    /// World coordinates at the centre of the view.
    pub center: (f64, f64),
    /// Screen points per block.
    pub zoom: f32,
    cache: FxHashMap<(i32, i32, u8), CachedTile>,
    pub hover: Option<(i32, i32)>,
    last_paint: Option<(f32, f32)>,
    last_time: Instant,
    pub painting: bool,
}

pub enum MapAction {
    None,
    /// Apply the current brush tool along a segment (world coords) with a time step.
    Paint { from: (f32, f32), to: (f32, f32), dt: f32, first: bool },
    EndPaint,
    Click { x: i32, y: i32, secondary: bool },
}

const ANNOTATION_COLORS: [[u8; 3]; 16] = [
    [255, 255, 255], [255, 128, 0], [255, 0, 255], [64, 160, 255], [255, 255, 0], [128, 255, 0], [255, 128, 192], [128, 128, 128],
    [192, 192, 192], [0, 192, 192], [160, 32, 240], [0, 0, 255], [139, 69, 19], [0, 128, 0], [255, 0, 0], [0, 0, 0],
];

impl Map2D {
    pub fn new() -> Self {
        Map2D { center: (0.0, 0.0), zoom: 1.0, cache: FxHashMap::default(), hover: None, last_paint: None, last_time: Instant::now(), painting: false }
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    pub fn frame_world(&mut self, d: &Dimension, view: Vec2) {
        if let Some((x0, z0, x1, z1)) = d.block_bounds() {
            self.center = ((x0 + x1) as f64 / 2.0, (z0 + z1) as f64 / 2.0);
            let w = (x1 - x0).max(1) as f32;
            let h = (z1 - z0).max(1) as f32;
            self.zoom = (view.x / w).min(view.y / h).clamp(0.02, 32.0) * 0.95;
        }
    }

    fn lod(&self) -> u8 {
        let mut lod = 0u8;
        let mut z = self.zoom;
        while z < 0.5 && lod < 4 {
            z *= 2.0;
            lod += 1;
        }
        lod
    }

    pub fn screen_to_world(&self, rect: Rect, p: Pos2) -> (f32, f32) {
        let c = rect.center();
        (
            (((p.x - c.x) / self.zoom) as f64 + self.center.0) as f32,
            (((p.y - c.y) / self.zoom) as f64 + self.center.1) as f32,
        )
    }

    pub fn world_to_screen(&self, rect: Rect, x: f32, y: f32) -> Pos2 {
        let c = rect.center();
        Pos2::new(c.x + (x - self.center.0 as f32) * self.zoom, c.y + (y - self.center.1 as f32) * self.zoom)
    }

    /// Draw the map and handle navigation. Returns the edit action requested by the user.
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        world: &World,
        dim: &Dimension,
        style: &MapStyle,
        brush_radius: f32,
        square_brush: bool,
        show_grid: bool,
        click_tool: bool,
    ) -> MapAction {
        let (rect, resp) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let dark = ui.visuals().dark_mode;
        painter.rect_filled(rect, 0.0, if dark { Color32::from_gray(24) } else { Color32::from_gray(200) });

        // Navigation: scroll zooms about the cursor, middle/right drag (or space + drag) pans.
        let pointer = resp.hover_pos();
        if let Some(p) = pointer {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().ln() * 200.0);
            if scroll != 0.0 {
                let before = self.screen_to_world(rect, p);
                self.zoom = (self.zoom * (1.0 + scroll * 0.0015)).clamp(0.02, 64.0);
                let after = self.screen_to_world(rect, p);
                self.center.0 += (before.0 - after.0) as f64;
                self.center.1 += (before.1 - after.1) as f64;
            }
        }
        let space = ui.input(|i| i.key_down(egui::Key::Space));
        let panning = resp.dragged_by(egui::PointerButton::Middle)
            || resp.dragged_by(egui::PointerButton::Secondary)
            || (space && resp.dragged_by(egui::PointerButton::Primary));
        if panning {
            let d = resp.drag_delta();
            self.center.0 -= (d.x / self.zoom) as f64;
            self.center.1 -= (d.y / self.zoom) as f64;
        }

        // Visible tiles
        let lod = self.lod();
        let tl = self.screen_to_world(rect, rect.left_top());
        let br = self.screen_to_world(rect, rect.right_bottom());
        let (tx0, ty0) = ((tl.0.floor() as i32) >> TILE_SHIFT, (tl.1.floor() as i32) >> TILE_SHIFT);
        let (tx1, ty1) = ((br.0.floor() as i32) >> TILE_SHIFT, (br.1.floor() as i32) >> TILE_SHIFT);
        let style_key = style.key();
        let mut missing = Vec::new();
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if let Some(t) = dim.tile(tx, ty) {
                    let ok = self.cache.get(&(tx, ty, lod)).is_some_and(|c| c.rev == t.rev && c.style == style_key);
                    if !ok {
                        missing.push((tx, ty, t.rev));
                    }
                }
            }
        }
        // Render missing tile images in parallel (bounded per frame to stay responsive).
        let budget = 96;
        let more = missing.len() > budget;
        missing.truncate(budget);
        let images: Vec<((i32, i32, u64), egui::ColorImage)> =
            missing.par_iter().filter_map(|&(tx, ty, rev)| Some(((tx, ty, rev), render_tile(world, dim, tx, ty, lod, style)?))).collect();
        for ((tx, ty, rev), img) in images {
            let tex = ui.ctx().load_texture(format!("tile{tx},{ty},{lod}"), img, egui::TextureOptions::NEAREST);
            self.cache.insert((tx, ty, lod), CachedTile { rev, style: style_key, tex });
        }
        if more {
            ui.ctx().request_repaint();
        }
        // Evict far-away cache entries occasionally.
        if self.cache.len() > 4000 {
            self.cache.retain(|(x, y, l), _| *l == lod && *x >= tx0 - 4 && *x <= tx1 + 4 && *y >= ty0 - 4 && *y <= ty1 + 4);
        }
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let (ox, oy) = ((tx << TILE_SHIFT) as f32, (ty << TILE_SHIFT) as f32);
                let r = Rect::from_min_max(self.world_to_screen(rect, ox, oy), self.world_to_screen(rect, ox + TILE_SIZE as f32, oy + TILE_SIZE as f32));
                if let Some(c) = self.cache.get(&(tx, ty, lod)) {
                    painter.image(c.tex.id(), r, uv, Color32::WHITE);
                } else if dim.tile(tx, ty).is_some() {
                    painter.rect_filled(r, 0.0, Color32::from_gray(60));
                }
                if show_grid && self.zoom > 0.25 && dim.tile(tx, ty).is_some() {
                    painter.rect_stroke(r, 0.0, EStroke::new(1.0, Color32::from_black_alpha(60)), egui::StrokeKind::Inside);
                }
            }
        }
        // Chunk grid at high zoom
        if show_grid && self.zoom >= 6.0 {
            let (cx0, cy0) = ((tl.0 / 16.0).floor() as i32, (tl.1 / 16.0).floor() as i32);
            let (cx1, cy1) = ((br.0 / 16.0).ceil() as i32, (br.1 / 16.0).ceil() as i32);
            let st = EStroke::new(1.0, Color32::from_black_alpha(35));
            for cx in cx0..=cx1 {
                let x = self.world_to_screen(rect, (cx * 16) as f32, 0.0).x;
                painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], st);
            }
            for cy in cy0..=cy1 {
                let y = self.world_to_screen(rect, 0.0, (cy * 16) as f32).y;
                painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], st);
            }
        }
        // Spawn marker
        let sp = self.world_to_screen(rect, world.spawn.0 as f32 + 0.5, world.spawn.1 as f32 + 0.5);
        painter.circle_stroke(sp, 6.0, EStroke::new(2.0, Color32::from_rgb(255, 60, 60)));
        painter.line_segment([sp - Vec2::new(9.0, 0.0), sp + Vec2::new(9.0, 0.0)], EStroke::new(1.5, Color32::from_rgb(255, 60, 60)));
        painter.line_segment([sp - Vec2::new(0.0, 9.0), sp + Vec2::new(0.0, 9.0)], EStroke::new(1.5, Color32::from_rgb(255, 60, 60)));

        // Brush outline and hover
        self.hover = None;
        if let Some(p) = pointer {
            let (wx, wy) = self.screen_to_world(rect, p);
            self.hover = Some((wx.floor() as i32, wy.floor() as i32));
            let rpx = brush_radius * self.zoom;
            let col = Color32::from_white_alpha(200);
            if square_brush {
                painter.rect_stroke(Rect::from_center_size(p, Vec2::splat(rpx * 2.0)), 0.0, EStroke::new(1.5, col), egui::StrokeKind::Middle);
            } else {
                painter.circle_stroke(p, rpx, EStroke::new(1.5, col));
            }
            painter.circle_stroke(p, rpx.max(4.0) + 1.0, EStroke::new(0.5, Color32::from_black_alpha(160)));
        }

        // Editing
        let now = Instant::now();
        let dt = now.duration_since(self.last_time).as_secs_f32().clamp(0.001, 0.1);
        self.last_time = now;
        if panning {
            return MapAction::None;
        }
        if click_tool {
            if resp.clicked()
                && let Some((x, y)) = self.hover
            {
                return MapAction::Click { x, y, secondary: ui.input(|i| i.modifiers.shift) };
            }
            return MapAction::None;
        }
        let primary_down = ui.input(|i| i.pointer.primary_down());
        if primary_down && (resp.is_pointer_button_down_on() || self.painting)
            && let Some(p) = pointer.or(ui.input(|i| i.pointer.interact_pos()))
        {
            let w = self.screen_to_world(rect, p);
            let first = !self.painting;
            let from = self.last_paint.unwrap_or(w);
            self.painting = true;
            self.last_paint = Some(w);
            ui.ctx().request_repaint();
            return MapAction::Paint { from, to: w, dt, first };
        }
        if self.painting && !primary_down {
            self.painting = false;
            self.last_paint = None;
            return MapAction::EndPaint;
        }
        MapAction::None
    }
}

/// Render one tile to an image at 128 >> lod resolution.
pub fn render_tile(world: &World, dim: &Dimension, tx: i32, ty: i32, lod: u8, style: &MapStyle) -> Option<egui::ColorImage> {
    let tile = dim.tile(tx, ty)?;
    let step = 1i32 << lod;
    let n = (TILE_SIZE / step) as usize;
    let (ox, oy) = (tx << TILE_SHIFT, ty << TILE_SHIFT);
    let overlay_defs: Vec<&LayerDef> = style.overlays.iter().filter_map(|id| world.layer(*id)).collect();
    let mut px = vec![Color32::BLACK; n * n];
    let h_at = |lx: i32, ly: i32| -> f32 {
        if (0..TILE_SIZE).contains(&lx) && (0..TILE_SIZE).contains(&ly) {
            tile.height[local_index(lx, ly)]
        } else {
            dim.height(ox + lx, oy + ly).unwrap_or(tile.height[local_index(lx.clamp(0, TILE_SIZE - 1), ly.clamp(0, TILE_SIZE - 1))])
        }
    };
    for py in 0..n {
        for pxx in 0..n {
            let (lx, ly) = (pxx as i32 * step, py as i32 * step);
            let i = local_index(lx, ly);
            if tile.layer_value(LayerId::VOID, i, 0) != 0 {
                px[py * n + pxx] = Color32::from_rgb(8, 8, 12);
                continue;
            }
            let h = tile.height[i];
            let t = tile.terrain[i];
            let mut c = world.terrain(t).color.map(|v| v as f32);
            if tile.layer_value(LayerId::FROST, i, 0) != 0 {
                c = [240.0, 245.0, 250.0];
            }
            // Hillshade from the slope towards the north-west light.
            let dx = h_at(lx + step, ly) - h_at(lx - step, ly);
            let dy = h_at(lx, ly + step) - h_at(lx, ly - step);
            let shade = (1.0 - (dx + dy) / (step as f32 * 2.0) * 0.18 * style.hillshade).clamp(0.45, 1.5);
            for v in c.iter_mut() {
                *v *= shade;
            }
            let water = tile.water[i] as f32;
            if water > (h + 0.5).floor() {
                let depth = water - h;
                let lava = tile.layer_value(LayerId::LAVA, i, 0) != 0;
                let wc = if lava { [230.0, 100.0, 20.0] } else { [40.0, 80.0, 190.0] };
                let a = if lava { 0.95 } else { (0.45 + depth * 0.03).min(0.88) };
                for k in 0..3 {
                    c[k] = c[k] * (1.0 - a) + wc[k] * a;
                }
            }
            for def in &overlay_defs {
                let v = tile.layer_value(def.id, i, def.default);
                if v == def.default {
                    continue;
                }
                let (col, a) = match def.kind {
                    LayerKind::Annotations => (ANNOTATION_COLORS[(v & 15) as usize].map(|x| x as f32), 0.7),
                    LayerKind::Biome => {
                        let b = meadow_blocks::biomes().by_numeric(v).map(|b| b.color).unwrap_or([255, 0, 255]);
                        (b.map(|x| x as f32), 0.55)
                    }
                    LayerKind::PaintedMask => ([255.0, 60.0, 60.0], v as f32 / 255.0 * 0.6),
                    _ => (def.color.map(|x| x as f32), 0.25 + 0.4 * v as f32 / def.data.max() as f32),
                };
                for k in 0..3 {
                    c[k] = c[k] * (1.0 - a) + col[k] * a;
                }
            }
            if let Some(m) = &style.mask {
                let v = m.eval(dim, ox + lx, oy + ly);
                if v > 0.0 {
                    let col = m.color.map(|x| x as f32);
                    let a = v * 0.45;
                    for k in 0..3 {
                        c[k] = c[k] * (1.0 - a) + col[k] * a;
                    }
                }
            }
            if style.contour > 0 {
                let ci = style.contour as f32;
                let band = (h / ci).floor();
                if (h_at(lx + step, ly) / ci).floor() != band || (h_at(lx, ly + step) / ci).floor() != band {
                    for v in c.iter_mut() {
                        *v *= 0.6;
                    }
                }
            }
            px[py * n + pxx] = Color32::from_rgb(c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8);
        }
    }
    Some(egui::ColorImage::new([n, n], px))
}
