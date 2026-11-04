//! User interface settings: scale, theme, colours, fonts and panel layout. Persisted as JSON in
//! the platform config directory.

use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeChoice {
    Dark,
    Light,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelKind {
    Tools,
    Brush,
    Terrain,
    Layers,
    Masks,
    Biomes,
    Info,
}

impl PanelKind {
    pub const ALL: [PanelKind; 7] =
        [PanelKind::Tools, PanelKind::Brush, PanelKind::Terrain, PanelKind::Layers, PanelKind::Masks, PanelKind::Biomes, PanelKind::Info];
    pub fn title(self) -> &'static str {
        match self {
            PanelKind::Tools => "Tools",
            PanelKind::Brush => "Brush & filter",
            PanelKind::Terrain => "Terrain",
            PanelKind::Layers => "Layers",
            PanelKind::Masks => "Custom masks",
            PanelKind::Biomes => "Biomes",
            PanelKind::Info => "Info",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dock {
    Left,
    Right,
    Hidden,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelPlacement {
    pub panel: PanelKind,
    pub dock: Dock,
    pub open: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Overall UI scale (egui zoom factor).
    pub ui_scale: f32,
    /// Base text size in points.
    pub font_size: f32,
    pub theme: ThemeChoice,
    pub accent: [u8; 3],
    /// Rounded corners on widgets.
    pub rounding: f32,
    pub compact: bool,
    /// Panels in display order.
    pub panels: Vec<PanelPlacement>,
    pub left_width: f32,
    pub right_width: f32,
    // 3D view
    pub msaa: bool,
    pub view_distance: f32,
    pub detail_distance: f32,
    pub decoration_distance: f32,
    pub fog: bool,
    pub show_plants_3d: bool,
    pub fly_speed: f32,
    // 2D view
    pub hillshade: f32,
    pub contour_interval: i32,
    pub show_grid: bool,
    pub show_layer_overlays: bool,
    // Misc
    pub recent_files: Vec<PathBuf>,
    pub recent_scripts: Vec<PathBuf>,
    pub minecraft_saves: Option<PathBuf>,
    pub imported_jars: Vec<PathBuf>,
    pub autosave_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ui_scale: 1.0,
            font_size: 13.5,
            theme: ThemeChoice::Dark,
            accent: [112, 180, 92],
            rounding: 4.0,
            compact: false,
            panels: vec![
                PanelPlacement { panel: PanelKind::Tools, dock: Dock::Left, open: true },
                PanelPlacement { panel: PanelKind::Brush, dock: Dock::Left, open: true },
                PanelPlacement { panel: PanelKind::Info, dock: Dock::Left, open: false },
                PanelPlacement { panel: PanelKind::Terrain, dock: Dock::Right, open: true },
                PanelPlacement { panel: PanelKind::Layers, dock: Dock::Right, open: true },
                PanelPlacement { panel: PanelKind::Masks, dock: Dock::Right, open: false },
                PanelPlacement { panel: PanelKind::Biomes, dock: Dock::Right, open: false },
            ],
            left_width: 250.0,
            right_width: 290.0,
            msaa: true,
            view_distance: 5000.0,
            detail_distance: 384.0,
            decoration_distance: 320.0,
            fog: true,
            show_plants_3d: true,
            fly_speed: 60.0,
            hillshade: 1.0,
            contour_interval: 0,
            show_grid: true,
            show_layer_overlays: true,
            recent_files: Vec::new(),
            recent_scripts: Vec::new(),
            minecraft_saves: default_saves_dir(),
            imported_jars: Vec::new(),
            autosave_minutes: 5,
        }
    }
}

pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("net", "Meadow", "Meadow").map(|d| d.config_dir().to_owned())
}

fn default_saves_dir() -> Option<PathBuf> {
    let home = directories::BaseDirs::new()?;
    let candidates = [
        home.data_dir().join(".minecraft").join("saves"),
        home.home_dir().join(".minecraft").join("saves"),
        home.home_dir().join("Library/Application Support/minecraft/saves"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

impl Settings {
    pub fn load() -> Settings {
        let Some(dir) = config_dir() else { return Settings::default() };
        let mut s: Settings = std::fs::read_to_string(dir.join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        // Make sure every panel appears exactly once (new panels after an upgrade).
        for p in PanelKind::ALL {
            if !s.panels.iter().any(|x| x.panel == p) {
                s.panels.push(PanelPlacement { panel: p, dock: Dock::Right, open: false });
            }
        }
        s
    }

    pub fn save(&self) {
        if let Some(dir) = config_dir() {
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(t) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(dir.join("settings.json"), t);
            }
        }
    }

    pub fn push_recent(&mut self, p: PathBuf) {
        self.recent_files.retain(|x| x != &p);
        self.recent_files.insert(0, p);
        self.recent_files.truncate(10);
    }

    pub fn accent(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.accent[0], self.accent[1], self.accent[2])
    }

    /// Apply scale, theme, colours and fonts to the egui context.
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_zoom_factor(self.ui_scale.clamp(0.5, 3.0));
        ctx.set_theme(match self.theme {
            ThemeChoice::Dark => egui::ThemePreference::Dark,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::System => egui::ThemePreference::System,
        });
        let accent = self.accent();
        let font = self.font_size.clamp(8.0, 32.0);
        let rounding = self.rounding;
        let compact = self.compact;
        ctx.all_styles_mut(|style| {
            use egui::{FontFamily, FontId, TextStyle};
            style.text_styles = [
                (TextStyle::Small, FontId::new(font * 0.75, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(font, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(font, FontFamily::Proportional)),
                (TextStyle::Heading, FontId::new(font * 1.35, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(font * 0.95, FontFamily::Monospace)),
            ]
            .into();
            let v = &mut style.visuals;
            v.selection.bg_fill = accent.linear_multiply(0.6);
            v.selection.stroke.color = accent;
            v.hyperlink_color = accent;
            v.widgets.hovered.bg_stroke.color = accent;
            let r = egui::CornerRadius::same(rounding.clamp(0.0, 12.0) as u8);
            v.widgets.noninteractive.corner_radius = r;
            v.widgets.inactive.corner_radius = r;
            v.widgets.hovered.corner_radius = r;
            v.widgets.active.corner_radius = r;
            v.window_corner_radius = egui::CornerRadius::same((rounding * 1.5).clamp(0.0, 16.0) as u8);
            if compact {
                style.spacing.item_spacing = egui::vec2(4.0, 2.0);
                style.spacing.button_padding = egui::vec2(3.0, 1.0);
            } else {
                style.spacing.item_spacing = egui::vec2(8.0, 4.0);
                style.spacing.button_padding = egui::vec2(6.0, 3.0);
            }
        });
    }
}
