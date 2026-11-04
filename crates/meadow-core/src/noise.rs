//! Deterministic hashing and gradient noise. Export and preview share these so that what the
//! 3D view shows is exactly what gets exported.

#[inline]
pub fn hash64(mut x: u64) -> u64 {
    // SplitMix64 finaliser
    x = x.wrapping_add(0x9e3779b97f4a7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}

#[inline]
pub fn hash2(seed: u64, x: i32, y: i32) -> u64 {
    hash64(seed ^ (x as u32 as u64).wrapping_mul(0x9e3779b97f4a7c15) ^ (y as u32 as u64).wrapping_mul(0xc2b2ae3d27d4eb4f))
}

#[inline]
pub fn hash3(seed: u64, x: i32, y: i32, z: i32) -> u64 {
    hash64(hash2(seed, x, y) ^ (z as u32 as u64).wrapping_mul(0x165667b19e3779f9))
}

/// Uniform float in [0, 1).
#[inline]
pub fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u64 << 24) as f32
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn grad2(h: u64, x: f32, y: f32) -> f32 {
    match h & 7 {
        0 => x + y,
        1 => x - y,
        2 => -x + y,
        3 => -x - y,
        4 => x,
        5 => -x,
        6 => y,
        _ => -y,
    }
}

/// 2D Perlin noise in roughly [-1, 1].
pub fn perlin2(seed: u64, x: f32, y: f32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let (xi, yi) = (x0 as i32, y0 as i32);
    let (xf, yf) = (x - x0, y - y0);
    let u = fade(xf);
    let v = fade(yf);
    let g00 = grad2(hash2(seed, xi, yi), xf, yf);
    let g10 = grad2(hash2(seed, xi + 1, yi), xf - 1.0, yf);
    let g01 = grad2(hash2(seed, xi, yi + 1), xf, yf - 1.0);
    let g11 = grad2(hash2(seed, xi + 1, yi + 1), xf - 1.0, yf - 1.0);
    let a = g00 + u * (g10 - g00);
    let b = g01 + u * (g11 - g01);
    (a + v * (b - a)) * 0.7
}

#[inline]
fn grad3(h: u64, x: f32, y: f32, z: f32) -> f32 {
    match h % 12 {
        0 => x + y,
        1 => -x + y,
        2 => x - y,
        3 => -x - y,
        4 => x + z,
        5 => -x + z,
        6 => x - z,
        7 => -x - z,
        8 => y + z,
        9 => -y + z,
        10 => y - z,
        _ => -y - z,
    }
}

/// 3D Perlin noise in roughly [-1, 1].
pub fn perlin3(seed: u64, x: f32, y: f32, z: f32) -> f32 {
    let (x0, y0, z0) = (x.floor(), y.floor(), z.floor());
    let (xi, yi, zi) = (x0 as i32, y0 as i32, z0 as i32);
    let (xf, yf, zf) = (x - x0, y - y0, z - z0);
    let (u, v, w) = (fade(xf), fade(yf), fade(zf));
    let g = |dx: i32, dy: i32, dz: i32| {
        grad3(hash3(seed, xi + dx, yi + dy, zi + dz), xf - dx as f32, yf - dy as f32, zf - dz as f32)
    };
    let lerp = |a: f32, b: f32, t: f32| a + t * (b - a);
    let x00 = lerp(g(0, 0, 0), g(1, 0, 0), u);
    let x10 = lerp(g(0, 1, 0), g(1, 1, 0), u);
    let x01 = lerp(g(0, 0, 1), g(1, 0, 1), u);
    let x11 = lerp(g(0, 1, 1), g(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// Fractal (fBm) 2D noise in roughly [-1, 1].
pub fn fbm2(seed: u64, x: f32, y: f32, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for o in 0..octaves {
        sum += amp * perlin2(seed.wrapping_add(o as u64 * 0x51ed27), x * freq, y * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    sum / norm
}

/// Small deterministic PRNG for per-object randomness.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(hash64(seed))
    }
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        hash64(self.0)
    }
    #[inline]
    pub fn f32(&mut self) -> f32 {
        unit(self.next_u64())
    }
    /// Uniform integer in `[lo, hi]`.
    #[inline]
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % (hi - lo + 1) as u64) as i32
    }
    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }
}
