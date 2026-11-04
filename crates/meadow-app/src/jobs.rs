//! Long-running work done off the UI thread: exporting, scripts, mod jar imports, saving.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use meadow_blocks::BlockInfo;
use meadow_core::World;
use meadow_export::{ExportOptions, ExportStats, Progress};
use meadow_script::{ParamValue, ScriptOutcome};

pub struct ExportJob {
    pub progress: Arc<Progress>,
    pub rx: Receiver<Result<ExportStats, String>>,
    pub started: Instant,
    pub dir: PathBuf,
}

impl ExportJob {
    pub fn start(world: World, opts: ExportOptions, ctx: eframe::egui::Context) -> ExportJob {
        let progress = Arc::new(Progress::default());
        let (tx, rx) = channel();
        let p = progress.clone();
        let dir = opts.dir.clone();
        std::thread::Builder::new()
            .name("export".into())
            .spawn(move || {
                let r = meadow_export::export_world(&world, &opts, &p).map_err(|e| format!("{e:#}"));
                let _ = tx.send(r);
                ctx.request_repaint();
            })
            .expect("spawn export thread");
        ExportJob { progress, rx, started: Instant::now(), dir }
    }
    pub fn cancel(&self) {
        self.progress.cancel.store(true, Ordering::Relaxed);
    }
}

pub struct ScriptJob {
    pub output_rx: Receiver<String>,
    pub done_rx: Receiver<ScriptOutcome>,
    pub cancel: Arc<AtomicBool>,
    pub output: String,
    pub started: Instant,
    pub name: String,
}

impl ScriptJob {
    pub fn start(path: PathBuf, source: String, world: World, args: Vec<String>, params: std::collections::BTreeMap<String, ParamValue>, ctx: eframe::egui::Context) -> ScriptJob {
        let (otx, orx) = channel();
        let (dtx, drx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        std::thread::Builder::new()
            .name("script".into())
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                let ctx2 = ctx.clone();
                let out = move |s: &str| {
                    let _ = otx.send(s.to_owned());
                    ctx2.request_repaint();
                };
                let r = meadow_script::run_script(&source, &path, Some(world), &args, &params, c, out);
                let _ = dtx.send(r);
                ctx.request_repaint();
            })
            .expect("spawn script thread");
        ScriptJob { output_rx: orx, done_rx: drx, cancel, output: String::new(), started: Instant::now(), name }
    }
}

pub struct JarJob {
    pub rx: Receiver<Result<(PathBuf, Vec<BlockInfo>), String>>,
    pub path: PathBuf,
}

impl JarJob {
    pub fn start(path: PathBuf, ctx: eframe::egui::Context) -> JarJob {
        let (tx, rx) = channel();
        let p = path.clone();
        std::thread::spawn(move || {
            let r = meadow_blocks::modjar::read_jar(&p).map(|b| (p, b)).map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        JarJob { rx, path }
    }
}

/// Cache file for blocks imported from jars (so imports survive restarts).
pub fn imported_blocks_file() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("imported_blocks.tsv"))
}

pub fn load_imported_blocks() -> usize {
    let Some(f) = imported_blocks_file() else { return 0 };
    let Ok(text) = std::fs::read_to_string(f) else { return 0 };
    let blocks = meadow_blocks::modjar::blocks_from_tsv(&text);
    meadow_blocks::modjar::import_into_registry(blocks, &[]).blocks
}

pub fn save_imported_blocks(new: &[BlockInfo]) {
    let Some(f) = imported_blocks_file() else { return };
    let mut existing = std::fs::read_to_string(&f).map(|t| meadow_blocks::modjar::blocks_from_tsv(&t)).unwrap_or_default();
    existing.retain(|b| !new.iter().any(|n| n.id == b.id));
    existing.extend_from_slice(new);
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, meadow_blocks::modjar::blocks_to_tsv(&existing));
}
