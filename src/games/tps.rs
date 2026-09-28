//! Borderlands: The Pre-Sequel: the same Willow engine branch as Borderlands 2,
//! so it shares BL2's tweak catalog, SDK and pages, with its own one-click
//! bundle. Hex signatures match OpenBLCMM's HexDictionary for TPS.

use super::bl2;
use super::willow;
use super::{GameDef, Support};
use crate::patches::{ExePatch, ExePatchKind};
use crate::theme::Rarity;

pub const PATCHES: &[ExePatch] = &[
    bl2::PATCHES[0],
    bl2::PATCHES[1],
    bl2::PATCHES[2],
    bl2::PATCHES[3],
    ExePatch {
        id: "hex_array_msg",
        name: "Silence the array-limit warning",
        description: "Removes the \"array limit reached\" console spam that accompanies the array-limit edit.",
        rarity: Rarity::Common,
        kind: ExePatchKind::Hex {
            original: "8B 40 04 83 F8 64 7C 7B 8B 8D 94 EE FF FF 83 C0 9D 50 68",
            patched: "8B 40 04 83 F8 64 EB 7B 8B 8D 94 EE FF FF 83 C0 9D 50 68",
        },
        note: Some("Not needed with the Python SDK."),
        revertible: true,
    },
];

pub static GAME: GameDef = GameDef {
    id: "tps",
    name: "Borderlands: The Pre-Sequel",
    short: "TPS",
    tagline: "Elpis. Low gravity. Oz kits and cryo.",
    support: Support::Full,
    steam_app_ids: &[261640],
    epic_names: &["Pre-Sequel"],
    exe: "Binaries\\Win32\\BorderlandsPreSequel.exe",
    launcher: Some(&bl2::LAUNCHER),
    config_subdir: "Borderlands The Pre-Sequel\\WillowGame\\Config",
    ini_files: willow::INI_FILES,
    categories: willow::CATEGORIES,
    tweaks: willow::TWEAKS,
    // bForceNoMovies soft-locks TPS at startup (OpenBLCMM disables it too).
    // The Ctrl+Shift+R bind and the news-feed URL are BL2's; TPS hides its
    // menu ads with the No Ads mod instead.
    hidden_tweaks: &["no_movies", "unbind_remaster", "hide_news"],
    ranges: willow::RANGES,
    presets: willow::TPS_PRESETS,
    patches: PATCHES,
    mods: Some(&bl2::WILLOW2_SDK),
    launch_args: bl2::LAUNCH_ARGS,
    nav: bl2::NAV,
    simple_nav: super::SIMPLE_NAV,
    quick: willow::QUICK,
    setup: willow::TPS_SETUP,
    comparisons: willow::COMPARISONS,
    capture: Some(&willow::CAPTURE),
};
