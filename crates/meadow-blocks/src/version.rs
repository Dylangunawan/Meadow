//! Supported Minecraft Java Edition export targets.

use serde::{Deserialize, Serialize};

/// Export target. Ordering matters: `index()` matches the `since` column in
/// `data/vanilla_blocks.tsv` (see `tools/gen_vanilla_data.py`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum McVersion {
    V1_18_2,
    V1_19_4,
    V1_20_1,
    V1_20_4,
    V1_20_6,
    V1_21_1,
    V1_21_4,
    V1_21_5,
    V1_21_8,
    V1_21_10,
    V1_21_11,
    V26_1,
}

impl McVersion {
    pub const ALL: [McVersion; 12] = [
        McVersion::V1_18_2,
        McVersion::V1_19_4,
        McVersion::V1_20_1,
        McVersion::V1_20_4,
        McVersion::V1_20_6,
        McVersion::V1_21_1,
        McVersion::V1_21_4,
        McVersion::V1_21_5,
        McVersion::V1_21_8,
        McVersion::V1_21_10,
        McVersion::V1_21_11,
        McVersion::V26_1,
    ];
    pub const LATEST: McVersion = McVersion::V26_1;

    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        match self {
            McVersion::V1_18_2 => "1.18.2",
            McVersion::V1_19_4 => "1.19.4",
            McVersion::V1_20_1 => "1.20.1",
            McVersion::V1_20_4 => "1.20.4",
            McVersion::V1_20_6 => "1.20.6",
            McVersion::V1_21_1 => "1.21.1",
            McVersion::V1_21_4 => "1.21.4",
            McVersion::V1_21_5 => "1.21.5",
            McVersion::V1_21_8 => "1.21.8",
            McVersion::V1_21_10 => "1.21.10",
            McVersion::V1_21_11 => "1.21.11",
            McVersion::V26_1 => "26.1",
        }
    }

    /// Human readable label for menus.
    pub fn label(self) -> String {
        match self {
            McVersion::V1_20_1 => "Java 1.20.1 (Conquest Reforged)".into(),
            v => format!("Java {}", v.name()),
        }
    }

    /// The chunk/level `DataVersion` written on export.
    pub fn data_version(self) -> i32 {
        match self {
            McVersion::V1_18_2 => 2975,
            McVersion::V1_19_4 => 3337,
            McVersion::V1_20_1 => 3465,
            McVersion::V1_20_4 => 3700,
            McVersion::V1_20_6 => 3839,
            McVersion::V1_21_1 => 3955,
            McVersion::V1_21_4 => 4189,
            McVersion::V1_21_5 => 4325,
            McVersion::V1_21_8 => 4440,
            McVersion::V1_21_10 => 4556,
            McVersion::V1_21_11 => 4671,
            McVersion::V26_1 => 4786,
        }
    }

    /// Data pack format number.
    pub fn pack_format(self) -> i32 {
        match self {
            McVersion::V1_18_2 => 9,
            McVersion::V1_19_4 => 12,
            McVersion::V1_20_1 => 15,
            McVersion::V1_20_4 => 26,
            McVersion::V1_20_6 => 41,
            McVersion::V1_21_1 => 48,
            McVersion::V1_21_4 => 61,
            McVersion::V1_21_5 => 71,
            McVersion::V1_21_8 => 81,
            McVersion::V1_21_10 => 88,
            McVersion::V1_21_11 => 94,
            McVersion::V26_1 => 101,
        }
    }

    /// Whether Meadow can generate a height-override data pack itself for this version. Newer
    /// versions changed the `dimension_type` format; for those the Higher Heights data pack file
    /// must be supplied (it is copied into the world on export).
    pub fn can_generate_height_datapack(self) -> bool {
        self <= McVersion::V1_21_10
    }

    pub fn from_name(s: &str) -> Option<McVersion> {
        McVersion::ALL.into_iter().find(|v| v.name() == s)
    }
}

/// Build height range presets. Vanilla caps dimension types at 4064 blocks (y -2032..2032),
/// which is what the Higher Heights data pack's largest variant uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildLimits {
    pub min_y: i32,
    /// Exclusive upper bound.
    pub max_y: i32,
}

impl BuildLimits {
    pub const VANILLA: BuildLimits = BuildLimits { min_y: -64, max_y: 320 };
    pub const PRESETS: [(&'static str, BuildLimits); 6] = [
        ("Vanilla (-64 to 320)", BuildLimits::VANILLA),
        ("Higher Heights 512 (-64 to 448)", BuildLimits { min_y: -64, max_y: 448 }),
        ("Higher Heights 1024 (-64 to 960)", BuildLimits { min_y: -64, max_y: 960 }),
        ("Higher Heights 2032 (-64 to 1968)", BuildLimits { min_y: -64, max_y: 1968 }),
        ("WorldPainter-style tall (0 to 2032)", BuildLimits { min_y: 0, max_y: 2032 }),
        ("Higher Heights 4064 (-2032 to 2032)", BuildLimits { min_y: -2032, max_y: 2032 }),
    ];
    /// Vanilla hard limits for any dimension type.
    pub const ABS_MIN_Y: i32 = -2032;
    pub const ABS_MAX_Y: i32 = 2032;

    pub fn height(self) -> i32 {
        self.max_y - self.min_y
    }
    pub fn is_vanilla(self) -> bool {
        self == BuildLimits::VANILLA
    }
    /// Clamp/round to what Minecraft accepts (multiples of 16 within ±2032).
    pub fn sanitized(self) -> BuildLimits {
        let min = (self.min_y.div_euclid(16) * 16).clamp(Self::ABS_MIN_Y, Self::ABS_MAX_Y - 16);
        let max = ((self.max_y + 15).div_euclid(16) * 16).clamp(min + 16, Self::ABS_MAX_Y);
        BuildLimits { min_y: min, max_y: max }
    }
}

impl Default for BuildLimits {
    fn default() -> Self {
        BuildLimits::VANILLA
    }
}
