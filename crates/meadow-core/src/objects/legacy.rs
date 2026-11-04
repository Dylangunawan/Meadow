//! Pre-1.13 numeric block IDs (MCEdit `.schematic`, `.bo2`) to modern block states.

use meadow_blocks::StateId;

const BASE: &[(u16, &str)] = &[
    (0, "air"), (1, "stone"), (2, "grass_block"), (3, "dirt"), (4, "cobblestone"), (5, "oak_planks"),
    (6, "oak_sapling"), (7, "bedrock"), (8, "water"), (9, "water"), (10, "lava"), (11, "lava"),
    (12, "sand"), (13, "gravel"), (14, "gold_ore"), (15, "iron_ore"), (16, "coal_ore"), (17, "oak_log"),
    (18, "oak_leaves"), (19, "sponge"), (20, "glass"), (21, "lapis_ore"), (22, "lapis_block"),
    (23, "dispenser"), (24, "sandstone"), (25, "note_block"), (26, "red_bed"), (27, "powered_rail"),
    (28, "detector_rail"), (29, "sticky_piston"), (30, "cobweb"), (31, "short_grass"), (32, "dead_bush"),
    (33, "piston"), (35, "white_wool"), (37, "dandelion"), (38, "poppy"), (39, "brown_mushroom"),
    (40, "red_mushroom"), (41, "gold_block"), (42, "iron_block"), (43, "smooth_stone_slab[type=double]"),
    (44, "smooth_stone_slab[type=bottom]"), (45, "bricks"), (46, "tnt"), (47, "bookshelf"),
    (48, "mossy_cobblestone"), (49, "obsidian"), (50, "torch"), (51, "fire"), (52, "spawner"),
    (53, "oak_stairs"), (54, "chest"), (55, "redstone_wire"), (56, "diamond_ore"), (57, "diamond_block"),
    (58, "crafting_table"), (59, "wheat"), (60, "farmland"), (61, "furnace"), (62, "furnace[lit=true]"),
    (63, "oak_sign"), (64, "oak_door"), (65, "ladder"), (66, "rail"), (67, "cobblestone_stairs"),
    (68, "oak_wall_sign"), (69, "lever"), (70, "stone_pressure_plate"), (71, "iron_door"),
    (72, "oak_pressure_plate"), (73, "redstone_ore"), (74, "redstone_ore[lit=true]"),
    (75, "redstone_torch[lit=false]"), (76, "redstone_torch"), (77, "stone_button"), (78, "snow[layers=1]"),
    (79, "ice"), (80, "snow_block"), (81, "cactus"), (82, "clay"), (83, "sugar_cane"), (84, "jukebox"),
    (85, "oak_fence"), (86, "carved_pumpkin"), (87, "netherrack"), (88, "soul_sand"), (89, "glowstone"),
    (90, "nether_portal"), (91, "jack_o_lantern"), (92, "cake"), (93, "repeater"),
    (94, "repeater[powered=true]"), (95, "white_stained_glass"), (96, "oak_trapdoor"),
    (97, "infested_stone"), (98, "stone_bricks"), (99, "brown_mushroom_block"), (100, "red_mushroom_block"),
    (101, "iron_bars"), (102, "glass_pane"), (103, "melon"), (104, "pumpkin_stem"), (105, "melon_stem"),
    (106, "vine"), (107, "oak_fence_gate"), (108, "brick_stairs"), (109, "stone_brick_stairs"),
    (110, "mycelium"), (111, "lily_pad"), (112, "nether_bricks"), (113, "nether_brick_fence"),
    (114, "nether_brick_stairs"), (115, "nether_wart"), (116, "enchanting_table"), (117, "brewing_stand"),
    (118, "cauldron"), (119, "end_portal"), (120, "end_portal_frame"), (121, "end_stone"),
    (122, "dragon_egg"), (123, "redstone_lamp"), (124, "redstone_lamp[lit=true]"),
    (125, "oak_slab[type=double]"), (126, "oak_slab[type=bottom]"), (127, "cocoa"),
    (128, "sandstone_stairs"), (129, "emerald_ore"), (130, "ender_chest"), (131, "tripwire_hook"),
    (132, "tripwire"), (133, "emerald_block"), (134, "spruce_stairs"), (135, "birch_stairs"),
    (136, "jungle_stairs"), (137, "command_block"), (138, "beacon"), (139, "cobblestone_wall"),
    (140, "flower_pot"), (141, "carrots"), (142, "potatoes"), (143, "oak_button"), (144, "skeleton_skull"),
    (145, "anvil"), (146, "trapped_chest"), (147, "light_weighted_pressure_plate"),
    (148, "heavy_weighted_pressure_plate"), (149, "comparator"), (150, "comparator[powered=true]"),
    (151, "daylight_detector"), (152, "redstone_block"), (153, "nether_quartz_ore"), (154, "hopper"),
    (155, "quartz_block"), (156, "quartz_stairs"), (157, "activator_rail"), (158, "dropper"),
    (159, "white_terracotta"), (160, "white_stained_glass_pane"), (161, "acacia_leaves"),
    (162, "acacia_log"), (163, "acacia_stairs"), (164, "dark_oak_stairs"), (165, "slime_block"),
    (166, "barrier"), (167, "iron_trapdoor"), (168, "prismarine"), (169, "sea_lantern"),
    (170, "hay_block"), (171, "white_carpet"), (172, "terracotta"), (173, "coal_block"),
    (174, "packed_ice"), (175, "sunflower"), (176, "white_banner"), (177, "white_wall_banner"),
    (178, "daylight_detector[inverted=true]"), (179, "red_sandstone"), (180, "red_sandstone_stairs"),
    (181, "red_sandstone_slab[type=double]"), (182, "red_sandstone_slab[type=bottom]"),
    (183, "spruce_fence_gate"), (184, "birch_fence_gate"), (185, "jungle_fence_gate"),
    (186, "dark_oak_fence_gate"), (187, "acacia_fence_gate"), (188, "spruce_fence"), (189, "birch_fence"),
    (190, "jungle_fence"), (191, "dark_oak_fence"), (192, "acacia_fence"), (193, "spruce_door"),
    (194, "birch_door"), (195, "jungle_door"), (196, "acacia_door"), (197, "dark_oak_door"),
    (198, "end_rod"), (199, "chorus_plant"), (200, "chorus_flower"), (201, "purpur_block"),
    (202, "purpur_pillar"), (203, "purpur_stairs"), (204, "purpur_slab[type=double]"),
    (205, "purpur_slab[type=bottom]"), (206, "end_stone_bricks"), (207, "beetroots"), (208, "dirt_path"),
    (209, "end_gateway"), (210, "repeating_command_block"), (211, "chain_command_block"),
    (212, "frosted_ice"), (213, "magma_block"), (214, "nether_wart_block"), (215, "red_nether_bricks"),
    (216, "bone_block"), (217, "structure_void"), (218, "observer"), (219, "white_shulker_box"),
    (235, "white_glazed_terracotta"), (251, "white_concrete"), (252, "white_concrete_powder"),
    (255, "structure_block"),
];

const COLOURS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan",
    "purple", "blue", "brown", "green", "red", "black",
];
const WOODS: [&str; 6] = ["oak", "spruce", "birch", "jungle", "acacia", "dark_oak"];

/// Convert a legacy (id, data) pair to a block state string.
pub fn legacy_name(id: u16, data: u8) -> String {
    let d = (data & 15) as usize;
    let log_axis = |d: usize| match (d >> 2) & 3 {
        1 => "x",
        2 => "z",
        _ => "y",
    };
    match id {
        1 => ["stone", "granite", "polished_granite", "diorite", "polished_diorite", "andesite", "polished_andesite"]
            .get(d)
            .unwrap_or(&"stone")
            .to_string(),
        2 => "grass_block[snowy=false]".into(),
        3 => ["dirt", "coarse_dirt", "podzol[snowy=false]"].get(d).unwrap_or(&"dirt").to_string(),
        5 => format!("{}_planks", WOODS.get(d).unwrap_or(&"oak")),
        6 => format!("{}_sapling", WOODS.get(d & 7).unwrap_or(&"oak")),
        8 | 9 => format!("water[level={}]", d.min(15)),
        10 | 11 => format!("lava[level={}]", d.min(15)),
        12 => if d == 1 { "red_sand".into() } else { "sand".into() },
        17 => format!("{}_log[axis={}]", WOODS[d & 3], log_axis(d)),
        162 => format!("{}_log[axis={}]", if d & 1 == 1 { "dark_oak" } else { "acacia" }, log_axis(d)),
        18 => format!("{}_leaves[distance=7,persistent=true,waterlogged=false]", WOODS[d & 3]),
        161 => format!("{}_leaves[distance=7,persistent=true,waterlogged=false]", if d & 1 == 1 { "dark_oak" } else { "acacia" }),
        24 => ["sandstone", "chiseled_sandstone", "cut_sandstone"].get(d).unwrap_or(&"sandstone").to_string(),
        31 => match d {
            2 => "fern".into(),
            0 => "dead_bush".into(),
            _ => "short_grass".into(),
        },
        35 => format!("{}_wool", COLOURS[d]),
        38 => ["poppy", "blue_orchid", "allium", "azure_bluet", "red_tulip", "orange_tulip", "white_tulip", "pink_tulip", "oxeye_daisy"]
            .get(d)
            .unwrap_or(&"poppy")
            .to_string(),
        43 | 44 => {
            let kind = ["smooth_stone", "sandstone", "oak", "cobblestone", "brick", "stone_brick", "nether_brick", "quartz"][d & 7];
            let t = if id == 43 { "double" } else if d & 8 != 0 { "top" } else { "bottom" };
            format!("{kind}_slab[type={t}]")
        }
        125 | 126 => {
            let t = if id == 125 { "double" } else if d & 8 != 0 { "top" } else { "bottom" };
            format!("{}_slab[type={t}]", WOODS.get(d & 7).unwrap_or(&"oak"))
        }
        53 | 67 | 108 | 109 | 114 | 128 | 134 | 135 | 136 | 156 | 163 | 164 | 180 | 203 => {
            let base = legacy_base(id);
            let facing = ["east", "west", "south", "north"][d & 3];
            let half = if d & 4 != 0 { "top" } else { "bottom" };
            format!("{base}[facing={facing},half={half}]")
        }
        78 => format!("snow[layers={}]", (d & 7) + 1),
        95 => format!("{}_stained_glass", COLOURS[d]),
        160 => format!("{}_stained_glass_pane", COLOURS[d]),
        159 => format!("{}_terracotta", COLOURS[d]),
        171 => format!("{}_carpet", COLOURS[d]),
        251 => format!("{}_concrete", COLOURS[d]),
        252 => format!("{}_concrete_powder", COLOURS[d]),
        98 => ["stone_bricks", "mossy_stone_bricks", "cracked_stone_bricks", "chiseled_stone_bricks"]
            .get(d)
            .unwrap_or(&"stone_bricks")
            .to_string(),
        155 => match d {
            1 => "chiseled_quartz_block".into(),
            2 => "quartz_pillar[axis=y]".into(),
            3 => "quartz_pillar[axis=x]".into(),
            4 => "quartz_pillar[axis=z]".into(),
            _ => "quartz_block".into(),
        },
        168 => ["prismarine", "prismarine_bricks", "dark_prismarine"].get(d).unwrap_or(&"prismarine").to_string(),
        175 => {
            if d & 8 != 0 {
                "tall_grass[half=upper]".into()
            } else {
                let p = ["sunflower", "lilac", "tall_grass", "large_fern", "rose_bush", "peony"].get(d).unwrap_or(&"tall_grass");
                format!("{p}[half=lower]")
            }
        }
        106 => {
            let mut s = String::from("vine[");
            s += &format!(
                "east={},north={},south={},up=false,west={}]",
                d & 8 != 0,
                d & 4 != 0,
                d & 1 != 0,
                d & 2 != 0
            );
            s
        }
        139 => if d == 1 { "mossy_cobblestone_wall".into() } else { "cobblestone_wall".into() },
        170 => format!("hay_block[axis={}]", log_axis(d)),
        _ => legacy_base(id).to_string(),
    }
}

fn legacy_base(id: u16) -> &'static str {
    BASE.iter().find(|(i, _)| *i == id).map(|(_, n)| *n).unwrap_or("air")
}

/// Cached conversion of every legacy id/data combination.
pub fn legacy_state(id: u16, data: u8) -> StateId {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<StateId>> = OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut v = Vec::with_capacity(256 * 16);
        for i in 0..256u16 {
            for d in 0..16u8 {
                v.push(StateId::parse(&legacy_name(i, d)));
            }
        }
        v
    });
    if id < 256 { t[(id as usize) * 16 + (data as usize & 15)] } else { StateId::AIR }
}
