//! Procedural trees for the built-in forest layers. Each kind has a fixed set of variants
//! generated deterministically, so export and the 3D preview agree exactly.

use std::sync::{Arc, OnceLock};

use meadow_blocks::{StateId, common};

use crate::layer::TreeKind;
use crate::noise::Rng;
use crate::objects::CustomObject;

pub const VARIANTS: usize = 24;

type Blocks = Vec<(i32, i32, i32, StateId)>;

fn s(x: &str) -> StateId {
    StateId::parse(x)
}

fn blob(out: &mut Blocks, cx: i32, cy: i32, cz: i32, rx: f32, ry: f32, leaves: StateId, rng: &mut Rng) {
    let (ix, iy) = (rx.ceil() as i32, ry.ceil() as i32);
    for y in -iy..=iy {
        for z in -ix..=ix {
            for x in -ix..=ix {
                let d = (x * x) as f32 / (rx * rx) + (y * y) as f32 / (ry * ry) + (z * z) as f32 / (rx * rx);
                if d <= 1.0 && (d < 0.7 || rng.chance(0.6)) {
                    out.push((cx + x, cy + y, cz + z, leaves));
                }
            }
        }
    }
}

fn trunk(out: &mut Blocks, x: i32, z: i32, from: i32, to: i32, log: StateId) {
    for y in from..to {
        out.push((x, y, z, log));
    }
}

/// Logs win over leaves at the same position.
fn dedup(mut v: Blocks, logs: &[StateId]) -> Blocks {
    v.sort_by_key(|b| (b.0, b.1, b.2, !logs.contains(&b.3)));
    v.dedup_by_key(|b| (b.0, b.1, b.2));
    v
}

fn deciduous(rng: &mut Rng) -> Blocks {
    let birch = rng.chance(0.3);
    let (log, leaves) = if birch { (common::birch_log(), common::birch_leaves()) } else { (common::oak_log(), common::oak_leaves()) };
    let h = rng.range(4, if birch { 7 } else { 6 });
    let mut v = Vec::new();
    trunk(&mut v, 0, 0, 0, h, log);
    let r = rng.range(2, 3) as f32 + rng.f32() * 0.5;
    blob(&mut v, 0, h - 1, 0, r, r * 0.8, leaves, rng);
    if !birch && rng.chance(0.3) {
        // A second smaller blob for irregular crowns
        let (dx, dz) = (rng.range(-1, 1), rng.range(-1, 1));
        blob(&mut v, dx, h, dz, r * 0.7, r * 0.6, leaves, rng);
    }
    dedup(v, &[log])
}

fn pine(rng: &mut Rng) -> Blocks {
    let (log, leaves) = (common::spruce_log(), common::spruce_leaves());
    let h = rng.range(7, 13);
    let mut v = Vec::new();
    trunk(&mut v, 0, 0, 0, h, log);
    let start = rng.range(2, 3);
    let mut r = 0.0f32;
    for y in (start..=h).rev() {
        let layer = h - y;
        r = if layer == 0 { 0.0 } else { (r + 0.6).min(1.0 + layer as f32 * 0.35) };
        if layer % 2 == 1 && layer > 2 {
            r -= 0.8;
        }
        let ri = r.max(0.0).ceil() as i32;
        for z in -ri..=ri {
            for x in -ri..=ri {
                if (x * x + z * z) as f32 <= r * r + 0.5 && (x, z) != (0, 0) {
                    v.push((x, y, z, leaves));
                }
            }
        }
    }
    v.push((0, h, 0, leaves));
    v.push((0, h + 1, 0, leaves));
    dedup(v, &[log])
}

fn jungle(rng: &mut Rng) -> Blocks {
    let (log, leaves) = (common::jungle_log(), common::jungle_leaves());
    let giant = rng.chance(0.25);
    let h = if giant { rng.range(14, 22) } else { rng.range(7, 12) };
    let mut v = Vec::new();
    let w = if giant { 2 } else { 1 };
    for dz in 0..w {
        for dx in 0..w {
            trunk(&mut v, dx, dz, 0, h, log);
        }
    }
    let r = if giant { 4.5 } else { 2.8 };
    blob(&mut v, 0, h, 0, r, 2.0, leaves, rng);
    if giant {
        for _ in 0..3 {
            let y = rng.range(h / 2, h - 3);
            let (dx, dz) = (rng.range(-4, 4), rng.range(-4, 4));
            blob(&mut v, dx, y, dz, 2.5, 1.5, leaves, rng);
        }
    }
    // Vines hanging from the outer leaves
    let vine = common::vine();
    let mut extra = Vec::new();
    for b in &v {
        if b.3 == leaves && rng.chance(0.08) {
            for dy in 1..rng.range(2, 6) {
                extra.push((b.0, b.1 - dy, b.2, vine));
            }
        }
    }
    v.extend(extra);
    dedup(v, &[log, leaves])
}

fn swamp(rng: &mut Rng) -> Blocks {
    let (log, leaves) = (common::oak_log(), common::oak_leaves());
    let h = rng.range(5, 8);
    let mut v = Vec::new();
    trunk(&mut v, 0, 0, 0, h, log);
    blob(&mut v, 0, h - 1, 0, 3.5, 1.6, leaves, rng);
    let vine = s("vine[east=false,north=true,south=false,up=false,west=false]");
    let mut extra = Vec::new();
    for b in &v {
        if b.3 == leaves && b.1 < h && rng.chance(0.15) {
            for dy in 1..rng.range(2, 4) {
                extra.push((b.0, b.1 - dy, b.2, vine));
            }
        }
    }
    v.extend(extra);
    dedup(v, &[log, leaves])
}

fn mangrove(rng: &mut Rng) -> Blocks {
    let (log, leaves) = (common::mangrove_log(), common::mangrove_leaves());
    let roots = s("mangrove_roots[waterlogged=false]");
    let lift = rng.range(2, 3);
    let h = rng.range(6, 9);
    let mut v = Vec::new();
    trunk(&mut v, 0, 0, lift, lift + h, log);
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (-1, -1)] {
        if rng.chance(0.75) {
            for y in -1..=lift {
                let k = (lift - y + 1).min(2);
                v.push((dx * k.min(2), y, dz * k.min(2), roots));
            }
        }
    }
    blob(&mut v, 0, lift + h - 1, 0, 3.0, 2.0, leaves, rng);
    dedup(v, &[log, roots])
}

fn cherry(rng: &mut Rng) -> Blocks {
    let log = s("cherry_log[axis=y]");
    let leaves = s("cherry_leaves[distance=7,persistent=true,waterlogged=false]");
    let h = rng.range(5, 7);
    let mut v = Vec::new();
    trunk(&mut v, 0, 0, 0, h - 2, log);
    // Two branches leaning out
    let branches = rng.range(1, 3);
    for b in 0..branches {
        let ang = (b as f32 / branches as f32) * std::f32::consts::TAU + rng.f32();
        let (dx, dz) = (ang.cos(), ang.sin());
        let len = rng.range(2, 4);
        let mut top = (0, h - 2, 0);
        for i in 1..=len {
            top = ((dx * i as f32).round() as i32, h - 2 + i.min(2), (dz * i as f32).round() as i32);
            v.push((top.0, top.1, top.2, log));
        }
        blob(&mut v, top.0, top.1 + 1, top.2, 3.5, 1.8, leaves, rng);
    }
    dedup(v, &[log])
}

fn pale_oak(rng: &mut Rng) -> Blocks {
    let log = s("pale_oak_log[axis=y]");
    let leaves = s("pale_oak_leaves[distance=7,persistent=true,waterlogged=false]");
    let moss = s("pale_hanging_moss[tip=true]");
    let h = rng.range(6, 9);
    let mut v = Vec::new();
    for dz in 0..2 {
        for dx in 0..2 {
            trunk(&mut v, dx, dz, 0, h, log);
        }
    }
    blob(&mut v, 0, h, 0, 4.0, 2.2, leaves, rng);
    let mut extra = Vec::new();
    for b in &v {
        if b.3 == leaves && b.1 < h && rng.chance(0.12) {
            extra.push((b.0, b.1 - 1, b.2, moss));
        }
    }
    v.extend(extra);
    dedup(v, &[log, leaves])
}

/// Generate variant `i` of a tree kind.
pub fn generate(kind: TreeKind, i: usize) -> CustomObject {
    let mut rng = Rng::new(0x7EE0_0000 +(kind as u64) * 1000 + i as u64);
    let blocks = match kind {
        TreeKind::Deciduous => deciduous(&mut rng),
        TreeKind::Pine => pine(&mut rng),
        TreeKind::Jungle => jungle(&mut rng),
        TreeKind::Swamp => swamp(&mut rng),
        TreeKind::Mangrove => mangrove(&mut rng),
        TreeKind::Cherry => cherry(&mut rng),
        TreeKind::PaleOak => pale_oak(&mut rng),
    };
    CustomObject::from_positions(format!("{kind:?} #{i}"), &blocks, [0, 0, 0])
}

/// Cached variants for a tree kind.
pub fn variants(kind: TreeKind) -> &'static [Arc<CustomObject>] {
    static CACHE: [OnceLock<Vec<Arc<CustomObject>>>; 7] =
        [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];
    CACHE[kind as usize].get_or_init(|| (0..VARIANTS).map(|i| Arc::new(generate(kind, i))).collect())
}

/// Grid spacing and maximum density for each kind.
pub fn spacing_density(kind: TreeKind) -> (u16, f32) {
    match kind {
        TreeKind::Deciduous => (5, 0.65),
        TreeKind::Pine => (4, 0.7),
        TreeKind::Jungle => (5, 0.8),
        TreeKind::Swamp => (8, 0.45),
        TreeKind::Mangrove => (7, 0.55),
        TreeKind::Cherry => (8, 0.5),
        TreeKind::PaleOak => (7, 0.6),
    }
}
