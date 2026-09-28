//! Tweak catalog for Gearbox's "Willow" Unreal Engine 3 branch, used by
//! Borderlands 2 and The Pre-Sequel. Sections and keys were checked against
//! a live Borderlands 2 install; option meanings come from the game's own
//! localization files (`WillowGame.int`). The Pre-Sequel's differences come
//! from OpenBLCMM, the BLCMods wiki, PCGamingWiki and the SDK mod database.

use crate::core::display::DisplayMode;
use crate::setup::{Component, ComponentKind, DxvkTarget, Group, TextSource};
use crate::theme::Rarity;
use crate::tweaks::DefaultValue::{B, C, N};
use crate::tweaks::{
    Category, ConfigSet, Control, EXPERIMENTAL, Impact, Key, MENU, NONE, Opt, Preset, RangePair, TF,
    TF_LOWER, TF_LOWER_INV, TF_UPPER, Tweak, Value, choice, custom, key, match_choice, opt,
    slider, toggle,
};

/// Logical config files, mapped to real names by each game's `ini_files`.
pub const E: &str = "engine";
pub const G: &str = "game";
pub const I: &str = "input";
/// The launcher's private copy of `WillowEngine.ini`; it re-applies these
/// keys on start, so launcher-managed tweaks are written to both.
pub const L: &str = "launcher";

pub const INI_FILES: &[(&str, &str)] = &[
    (E, "WillowEngine.ini"),
    (G, "WillowGame.ini"),
    (I, "WillowInput.ini"),
    (L, "LauncherConfig\\WillowEngine.ini"),
];

const SS: &str = "SystemSettings";
const ENGINE: &str = "Engine.Engine";
const PAWN: &str = "WillowGame.WillowPawn";
const PC: &str = "WillowGame.WillowPlayerController";
const MOVIES: &str = "FullScreenMovie";
const PLAYER_INPUT: &str = "Engine.PlayerInput";

pub const CATEGORIES: &[Category] = &[
    Category { id: "quick", title: "Quick", blurb: "Friendly combined settings used by Simple mode." },
    Category { id: "display", title: "Display", blurb: "Window mode, resolution and sync. The launcher keeps its own copy of these, so Vaulter updates both." },
    Category { id: "framerate", title: "Framerate", blurb: "Frame caps and smoothing. Pick \"Smoothed\" and set the range for a custom cap such as 141–142 FPS." },
    Category { id: "quality", title: "World detail", blurb: "Draw distance, level of detail, foliage and decals." },
    Category { id: "aa", title: "Anti-aliasing & filtering", blurb: "The game is DirectX 9 only. FXAA is the built-in option; for better AA use driver SGSSAA or a ReShade SMAA preset." },
    Category { id: "textures", title: "Textures & streaming", blurb: "Texture resolution, streaming pool and pop-in. The game is a 32-bit exe, so avoid huge pool sizes." },
    Category { id: "outlines", title: "Cel shading & outlines", blurb: "Borderlands' ink outlines come from a Sobel edge-detect post-process. Swap the post-process chain to thin them out or remove them." },
    Category { id: "postfx", title: "Post-processing", blurb: "Bloom, ambient occlusion, depth of field, light shafts and other screen effects." },
    Category { id: "shadows", title: "Shadows", blurb: "Dynamic shadow toggles and shadow map resolution. Dynamic shadows are the single biggest FPS cost." },
    Category { id: "physx", title: "PhysX & destruction", blurb: "GPU particle, cloth and fluid effects. On AMD/Intel GPUs PhysX runs on the CPU and costs a lot of FPS at Medium or High." },
    Category { id: "camera", title: "Field of view", blurb: "The in-game slider stops at 110 and is saved in your profile. A hotkey bind can push the FOV further." },
    Category { id: "input", title: "Console & input", blurb: "Unlock the developer console and clean up bindings. Sensitivity, smoothing and invert live in profile.bin, not the ini files." },
    Category { id: "hud", title: "HUD & menus", blurb: "Main-menu and HUD behavior." },
    Category { id: "gameplay", title: "Gameplay feel", blurb: "Camera shakes, corpse cleanup and other feel tweaks." },
    Category { id: "audio", title: "Audio & focus", blurb: "What happens when you alt-tab, voice chat, and mixer voices." },
    Category { id: "startup", title: "Startup", blurb: "Get to Sanctuary faster: skip the logo movies and the intro-cinematic confirmation." },
    Category { id: "network", title: "Network", blurb: "Bandwidth hints for co-op. Values from community co-op lag guides." },
];

// ---- option tables -------------------------------------------------------------

const FRAMERATE_LOCK: &[Opt] = &[
    opt("0", "Custom range"),
    opt("1", "30"),
    opt("2", "50"),
    opt("3", "60"),
    opt("4", "72"),
    opt("5", "120"),
    opt("6", "Unlimited"),
];
pub(crate) const LOW_MED_HIGH: &[Opt] = &[opt("0", "Low"), opt("1", "Medium"), opt("2", "High")];
const VIEW_DISTANCE: &[Opt] = &[opt("0", "Low"), opt("1", "Medium"), opt("2", "High"), opt("3", "Ultra High")];
/// The engine counts these down (0 is the best), so the values run 2 → 0 to
/// keep the lowest setting on the left like every other control.
pub(crate) const DETAIL_LOW_TO_HIGH: &[Opt] = &[opt("2", "Low"), opt("1", "Medium"), opt("0", "High")];
const DECALS: &[Opt] = &[opt("0", "Off"), opt("1", "Normal"), opt("2", "High")];
pub(crate) const ANISO: &[Opt] = &[opt("1", "Off"), opt("2", "2x"), opt("4", "4x"), opt("8", "8x"), opt("16", "16x")];
const SHADOW_RES: &[Opt] = &[opt("512", "512"), opt("1024", "1024"), opt("2048", "2048"), opt("4096", "4096")];
const SCENE_SHADOW_RES: &[Opt] = &[
    opt("512", "512"),
    opt("1024", "1024"),
    opt("2048", "2048"),
    opt("4096", "4096"),
];
pub(crate) const POST_PROCESS: &[Opt] = &[
    opt("WillowEngineMaterials.WillowScenePostProcess", "Classic outlines"),
    opt("WillowEngineMaterials.RyanScenePostProcess", "No outlines"),
    opt("WillowEngineMaterials.CinematicScenePostProcess", "Cinematic (no outlines or cel)"),
    opt("EngineMaterials.ScenePostProcess", "Stock Unreal"),
];
const CONSOLE_KEYS: &[Opt] = &[
    opt("Undefine", "Disabled"),
    opt("Tilde", "~ Tilde"),
    opt("Backslash", "\\ Backslash"),
    opt("F1", "F1"),
    opt("F6", "F6"),
    opt("Insert", "Insert"),
    opt("Home", "Home"),
];
const FOV_KEYS: &[Opt] = &[
    opt("", "Off"),
    opt("F5", "F5"),
    opt("F6", "F6"),
    opt("F7", "F7"),
    opt("F8", "F8"),
    opt("F10", "F10"),
    opt("MiddleMouseButton", "Middle mouse"),
];
const WINDOW_MODES: &[Opt] = &[
    opt("fullscreen", "Fullscreen"),
    opt("windowed", "Windowed"),
    opt("borderless", "Borderless"),
];
pub(crate) const RESOLUTIONS: &[Opt] = &[
    opt("1280x720", "1280×720"),
    opt("1280x800", "1280×800 (Steam Deck)"),
    opt("1600x900", "1600×900"),
    opt("1920x1080", "1920×1080"),
    opt("2560x1080", "2560×1080 UW"),
    opt("2560x1440", "2560×1440"),
    opt("3440x1440", "3440×1440 UW"),
    opt("3840x2160", "3840×2160"),
    opt("5120x1440", "5120×1440 SUW"),
];
pub(crate) const TEXTURE_BIAS: &[Opt] = &[opt("2", "Quarter"), opt("1", "Half"), opt("0", "Full")];
const CORPSES: &[Opt] = &[opt("fast", "Fast (15 s)"), opt("balanced", "Balanced (60 s)"), opt("vanilla", "Vanilla (10 min)")];

// ---- custom bindings -------------------------------------------------------------

/// Writes the live ini and, when present, the launcher's copy.
pub(crate) fn set_mirrored(config: &mut ConfigSet, section: &'static str, name: &'static str, value: &str) {
    config.set(&key(E, section, name), value);
    if config.file(L).is_some_and(|f| f.exists) {
        config.set(&key(L, section, name), value);
    }
}

fn read_bool(config: &ConfigSet, k: &Key) -> Option<bool> {
    config.get(k).and_then(crate::core::ini::parse_bool)
}

fn read_window_mode(c: &ConfigSet) -> Option<Value> {
    let full = read_bool(c, &key(E, SS, "Fullscreen"))?;
    let borderless = read_bool(c, &key(E, SS, "WindowedFullscreen")).unwrap_or(false);
    Some(Value::Choice(match (full, borderless) {
        (_, true) => "borderless",
        (true, false) => "fullscreen",
        (false, false) => "windowed",
    }))
}

fn write_window_mode(c: &mut ConfigSet, v: &Value) {
    let (full, borderless) = match v {
        Value::Choice("fullscreen") => ("True", "False"),
        Value::Choice("borderless") => ("False", "True"),
        Value::Choice("windowed") => ("False", "False"),
        _ => return,
    };
    set_mirrored(c, SS, "Fullscreen", full);
    set_mirrored(c, SS, "WindowedFullscreen", borderless);
}

pub(crate) fn read_resolution(c: &ConfigSet) -> Option<Value> {
    let x = c.get(&key(E, SS, "ResX"))?;
    let y = c.get(&key(E, SS, "ResY"))?;
    Some(match_choice(RESOLUTIONS, &format!("{x}x{y}")))
}

pub(crate) fn write_resolution(c: &mut ConfigSet, v: &Value) {
    let Value::Choice(res) = v else { return };
    let Some((x, y)) = res.split_once('x') else { return };
    set_mirrored(c, SS, "ResX", x);
    set_mirrored(c, SS, "ResY", y);
}

const LOGO_MOVIES: [&str; 2] = ["2K_logo", "Gearbox_logo"];

fn read_skip_logos(c: &ConfigSet) -> Option<Value> {
    let movies = c.doc(E)?.get_all(MOVIES, "StartupMovies");
    Some(Value::Bool(
        !movies.iter().any(|m| LOGO_MOVIES.iter().any(|l| m.eq_ignore_ascii_case(l))),
    ))
}

fn write_skip_logos(c: &mut ConfigSet, v: &Value) {
    let Value::Bool(skip) = v else { return };
    let Some(doc) = c.doc_mut(E) else { return };
    // "Loading" must stay last: the engine loops the final startup movie while loading.
    if *skip {
        doc.set_all(MOVIES, "StartupMovies", &["Loading"]);
    } else {
        doc.set_all(MOVIES, "StartupMovies", &["2K_logo", "Gearbox_logo", "Loading"]);
    }
}

fn read_skip_intro_confirm(c: &ConfigSet) -> Option<Value> {
    let doc = c.doc(E)?;
    Some(Value::Bool(!doc.get_all(MOVIES, "ConfirmSkipMovies").iter().any(|m| m.eq_ignore_ascii_case("MegaIntro"))))
}

fn write_skip_intro_confirm(c: &mut ConfigSet, v: &Value) {
    let Value::Bool(skip) = v else { return };
    let Some(doc) = c.doc_mut(E) else { return };
    if *skip {
        doc.remove_value(MOVIES, "ConfirmSkipMovies", "MegaIntro");
    } else {
        doc.add_value(MOVIES, "ConfirmSkipMovies", "MegaIntro");
    }
}

const MIP_FADE_KEYS: [&str; 4] = ["MipFadeInSpeed0", "MipFadeOutSpeed0", "MipFadeInSpeed1", "MipFadeOutSpeed1"];
const MIP_FADE_DEFAULTS: [&str; 4] = ["0.3", "0.1", "2.0", "1.0"];

fn read_texture_fade(c: &ConfigSet) -> Option<Value> {
    let doc = c.doc(E)?;
    let zeroed = MIP_FADE_KEYS
        .iter()
        .all(|k| doc.get(ENGINE, k).and_then(|v| v.parse::<f64>().ok()) == Some(0.0));
    Some(Value::Bool(zeroed))
}

fn write_texture_fade(c: &mut ConfigSet, v: &Value) {
    let Value::Bool(disable) = v else { return };
    let Some(doc) = c.doc_mut(E) else { return };
    for (k, default) in MIP_FADE_KEYS.iter().zip(MIP_FADE_DEFAULTS) {
        doc.set(ENGINE, k, if *disable { "0" } else { default });
    }
}

const TEXTURE_GROUPS: [&str; 12] = [
    "TEXTUREGROUP_World",
    "TEXTUREGROUP_WorldNormalMap",
    "TEXTUREGROUP_WorldSpecular",
    "TEXTUREGROUP_Character",
    "TEXTUREGROUP_CharacterNormalMap",
    "TEXTUREGROUP_CharacterSpecular",
    "TEXTUREGROUP_Weapon",
    "TEXTUREGROUP_WeaponNormalMap",
    "TEXTUREGROUP_WeaponSpecular",
    "TEXTUREGROUP_Vehicle",
    "TEXTUREGROUP_VehicleNormalMap",
    "TEXTUREGROUP_VehicleSpecular",
];

/// Rewrites `LODBias=<n>` inside a TEXTUREGROUP struct value.
fn with_lod_bias(value: &str, bias: &str) -> Option<String> {
    let start = value.find("LODBias=")? + "LODBias=".len();
    let end = value[start..]
        .find([',', ')'])
        .map(|i| start + i)
        .unwrap_or(value.len());
    Some(format!("{}{}{}", &value[..start], bias, &value[end..]))
}

pub(crate) fn read_texture_bias(c: &ConfigSet) -> Option<Value> {
    let v = c.get(&key(E, SS, "TEXTUREGROUP_World"))?;
    let start = v.find("LODBias=")? + "LODBias=".len();
    let bias: String = v[start..].chars().take_while(|ch| ch.is_ascii_digit() || *ch == '-').collect();
    Some(match_choice(TEXTURE_BIAS, &bias))
}

pub(crate) fn write_texture_bias(c: &mut ConfigSet, v: &Value) {
    let Value::Choice(bias) = v else { return };
    for file in [E, L] {
        if file == L && !c.file(L).is_some_and(|f| f.exists) {
            continue;
        }
        for group in TEXTURE_GROUPS {
            let k = key(file, SS, group);
            if let Some(new) = c.get(&k).and_then(|old| with_lod_bias(old, bias)) {
                c.set(&k, &new);
            }
        }
    }
}

fn is_fov_bind(binding: &str) -> bool {
    binding.to_ascii_lowercase().contains("command=\"fov ")
}

/// Parses `(Name="F7",Command="fov 120")` into ("F7", 120).
fn parse_fov_bind(binding: &str) -> Option<(String, f64)> {
    let name_at = binding.find("Name=\"")? + 6;
    let name = binding[name_at..].split('"').next()?.to_string();
    let lower = binding.to_ascii_lowercase();
    let fov_at = lower.find("command=\"fov ")? + "command=\"fov ".len();
    let fov = lower[fov_at..].split('"').next()?.trim().parse().ok()?;
    Some((name, fov))
}

fn fov_bind(c: &ConfigSet) -> Option<(String, f64)> {
    c.doc(I)?
        .find_value(PLAYER_INPUT, "Bindings", is_fov_bind)
        .and_then(parse_fov_bind)
}

fn write_fov_bind(c: &mut ConfigSet, name: &str, fov: f64) {
    let Some(doc) = c.doc_mut(I) else { return };
    doc.remove_values_where(PLAYER_INPUT, "Bindings", is_fov_bind);
    if !name.is_empty() {
        doc.add_value(
            PLAYER_INPUT,
            "Bindings",
            &format!("(Name=\"{name}\",Command=\"fov {}\")", fov.round() as i64),
        );
    }
}

fn read_fov_value(c: &ConfigSet) -> Option<Value> {
    Some(Value::Num(fov_bind(c).map(|(_, fov)| fov).unwrap_or(110.0)))
}

/// The value is applied before the key (tweak ids sort that way), so it
/// creates the binding on a default key that the key tweak then moves.
fn write_fov_value(c: &mut ConfigSet, v: &Value) {
    let Value::Num(fov) = v else { return };
    let name = fov_bind(c).map(|(n, _)| n).unwrap_or_else(|| "F7".into());
    write_fov_bind(c, &name, *fov);
}

fn read_fov_key(c: &ConfigSet) -> Option<Value> {
    Some(match fov_bind(c) {
        Some((name, _)) => match_choice(FOV_KEYS, &name),
        None => Value::Choice(""),
    })
}

fn write_fov_key(c: &mut ConfigSet, v: &Value) {
    let Value::Choice(name) = v else { return };
    let fov = fov_bind(c).map(|(_, f)| f).unwrap_or(110.0);
    write_fov_bind(c, name, fov);
}

const REMASTER_BIND: &str = "(Name=\"R\",Command=\"Remaster TOGGLE\",Control=True,Shift=True)";

fn is_remaster_bind(binding: &str) -> bool {
    binding.contains("Remaster TOGGLE")
}

fn read_remaster_unbound(c: &ConfigSet) -> Option<Value> {
    let doc = c.doc(I)?;
    Some(Value::Bool(doc.find_value(PLAYER_INPUT, "Bindings", is_remaster_bind).is_none()))
}

fn write_remaster_unbound(c: &mut ConfigSet, v: &Value) {
    let Value::Bool(unbind) = v else { return };
    let Some(doc) = c.doc_mut(I) else { return };
    if *unbind {
        doc.remove_values_where(PLAYER_INPUT, "Bindings", is_remaster_bind);
    } else if doc.find_value(PLAYER_INPUT, "Bindings", is_remaster_bind).is_none() {
        doc.add_value(PLAYER_INPUT, "Bindings", REMASTER_BIND);
    }
}

const UPSELL_URL: &str = "services.gearboxsoftware.com/api/v2/articles";

fn read_hide_news(c: &ConfigSet) -> Option<Value> {
    let url = c.get(&key(G, "WillowGame.WillowGameInfo", "UpsellNewsURL"))?;
    Some(Value::Bool(url.contains("127.0.0.1")))
}

fn write_hide_news(c: &mut ConfigSet, v: &Value) {
    let Value::Bool(hide) = v else { return };
    c.set(
        &key(G, "WillowGame.WillowGameInfo", "UpsellNewsURL"),
        if *hide { "127.0.0.1" } else { UPSELL_URL },
    );
}

const CORPSE_KEYS: [&str; 3] = [
    "SecondsBeforeConsideringRagdollRemoval",
    "SecondsBeforeVisibleRagdollRemoval",
    "SecondsBetweenRagdollRemovalAttempts",
];

fn read_corpses(c: &ConfigSet) -> Option<Value> {
    let visible: f64 = c.get(&key(G, PAWN, CORPSE_KEYS[1]))?.parse().ok()?;
    Some(Value::Choice(if visible >= 300.0 {
        "vanilla"
    } else if visible >= 45.0 {
        "balanced"
    } else {
        "fast"
    }))
}

fn write_corpses(c: &mut ConfigSet, v: &Value) {
    let values = match v {
        Value::Choice("vanilla") => ["600.0", "600.0", "1.0"],
        Value::Choice("balanced") => ["30.0", "60.0", "1.0"],
        Value::Choice("fast") => ["5.0", "15.0", "1.0"],
        _ => return,
    };
    for (k, val) in CORPSE_KEYS.iter().zip(values) {
        c.set(&key(G, PAWN, k), val);
    }
}

const FPS_TARGETS: &[Opt] = &[
    opt("60", "60"),
    opt("120", "120"),
    opt("144", "144"),
    opt("165", "165"),
    opt("240", "240"),
    opt("0", "Unlimited"),
];

fn read_fps_target(c: &ConfigSet) -> Option<Value> {
    let lock = c.get(&key(E, SS, "FramerateLocking"))?.trim().to_string();
    Some(match lock.as_str() {
        "6" => Value::Choice("0"),
        "3" => Value::Choice("60"),
        "5" => Value::Choice("120"),
        "0" => {
            if read_bool(c, &key(E, ENGINE, "bSmoothFrameRate")) == Some(false) {
                Value::Choice("0")
            } else {
                match c.get(&key(E, ENGINE, "MaxSmoothedFrameRate")).unwrap_or("62").trim() {
                    // The shipped smoothed range (22–62) is the stock ~60 cap:
                    // factory settings (`fps_lock` "0") read as this control's default.
                    "62" => Value::Choice("60"),
                    max => match_choice(FPS_TARGETS, max),
                }
            }
        }
        other => Value::Unknown(format!("mode {other}")),
    })
}

/// Caps the framerate at `hz` (0 = unlimited). 60 uses the game's own
/// limiter; anything else uses the smoothed range, which accepts any value.
pub(crate) fn write_fps_cap(c: &mut ConfigSet, hz: u32) {
    match hz {
        0 => set_mirrored(c, SS, "FramerateLocking", "6"),
        60 => set_mirrored(c, SS, "FramerateLocking", "3"),
        hz => {
            set_mirrored(c, SS, "FramerateLocking", "0");
            c.set(&key(E, ENGINE, "bSmoothFrameRate"), "TRUE");
            c.set(&key(E, ENGINE, "MinSmoothedFrameRate"), "22");
            c.set(&key(E, ENGINE, "MaxSmoothedFrameRate"), &hz.to_string());
        }
    }
}

fn write_fps_target(c: &mut ConfigSet, v: &Value) {
    if let Value::Choice(hz) = v
        && let Ok(hz) = hz.parse() {
            write_fps_cap(c, hz);
        }
}

/// Setup hook: native resolution and a framerate cap matching the display.
pub(crate) fn match_display(c: &mut ConfigSet, mode: DisplayMode) {
    set_mirrored(c, SS, "ResX", &mode.width.to_string());
    set_mirrored(c, SS, "ResY", &mode.height.to_string());
    write_fps_cap(c, mode.refresh_hz.max(60));
}

// ---- the catalog -------------------------------------------------------------------

/// The smoothed framerate range, edited as one RangeSlider.
pub const RANGES: &[RangePair] = &[RangePair {
    min: "smooth_min",
    max: "smooth_max",
    label: "Smoothed range",
    description: "Lower and upper bound of the smoothed framerate. The maximum is effectively your FPS cap in Smoothed mode.",
}];

pub const TWEAKS: &[Tweak] = &[
    // Simple-mode combined controls
    custom("fps_target", "quick", "Framerate limit", "Cap the game at your monitor's refresh rate for smooth, even frame pacing, or remove the cap entirely.",
        Control::Choice(FPS_TARGETS), read_fps_target, write_fps_target, C("60"), Impact::Medium, MENU),

    // Display
    custom("window_mode", "display", "Window mode", "Fullscreen, windowed, or borderless fullscreen window (instant alt-tab).",
        Control::Choice(WINDOW_MODES), read_window_mode, write_window_mode, C("fullscreen"), Impact::None, MENU),
    custom("resolution", "display", "Resolution", "Render resolution. Custom values already in your file show as Custom and are kept until you pick one.",
        Control::Choice(RESOLUTIONS), read_resolution, write_resolution, C("1920x1080"), Impact::High, MENU),
    toggle("vsync", "display", "Vertical sync", "Sync to the monitor refresh. Adds input lag; prefer a frame cap or driver-level sync.",
        &[key(E, SS, "UseVsync"), key(L, SS, "UseVSync")], TF, false, Impact::Low, MENU),
    slider("gamma", "display", "Display gamma", "Engine gamma. The in-game brightness slider tops out early; this goes further.",
        &[key(E, "Engine.Client", "DisplayGamma")], (1.6, 3.2, 0.05), 2, "", 2.2, Impact::None, NONE),
    slider("screen_pct", "display", "Render scale", "Renders at a lower resolution and upscales. Below 100 trades sharpness for FPS; UE3 doesn't support supersampling here (use DSR/VSR).",
        &[key(E, SS, "ScreenPercentage")], (50.0, 100.0, 5.0), 0, "%", 100.0, Impact::High, NONE),
    toggle("pause_focus", "display", "Pause when alt-tabbed", "Pause the game when the window loses focus in fullscreen.",
        &[key(E, ENGINE, "bPauseOnLossOfFocus")], TF_UPPER, true, Impact::None, NONE),
    toggle("pause_focus_windowed", "display", "Pause when alt-tabbed (windowed)", "Same, for windowed and borderless modes.",
        &[key(E, "WillowGame.WillowGameEngine", "bPauseLostFocusWindowed"), key(L, "WillowGame.WillowGameEngine", "bPauseLostFocusWindowed")], TF_UPPER, false, Impact::None, MENU),

    // Framerate
    choice("fps_lock", "framerate", "Framerate limit", "The in-game limiter. \"Smoothed\" uses the min/max range below; set it to 141/142 (or your refresh − 1) for a custom cap.",
        &[key(E, SS, "FramerateLocking"), key(L, SS, "FramerateLocking")], FRAMERATE_LOCK, "0", Impact::Medium, MENU),
    toggle("smooth_fps", "framerate", "Frame smoothing", "Enables the smoothed range. Used when the limit is set to Smoothed.",
        &[key(E, ENGINE, "bSmoothFrameRate")], TF_UPPER, true, Impact::None, NONE),
    slider("smooth_min", "framerate", "Smoothed minimum", "Lower bound of the smoothed range.",
        &[key(E, ENGINE, "MinSmoothedFrameRate")], (10.0, 360.0, 1.0), 0, "fps", 22.0, Impact::None, NONE),
    slider("smooth_max", "framerate", "Smoothed maximum", "Upper bound: effectively your FPS cap in Smoothed mode.",
        &[key(E, ENGINE, "MaxSmoothedFrameRate")], (30.0, 400.0, 1.0), 0, "fps", 62.0, Impact::Medium, NONE),
    toggle("one_frame_lag", "framerate", "One frame thread lag", "Lets the render thread run a frame behind. Off lowers input latency at some FPS cost.",
        &[key(E, SS, "OneFrameThreadLag"), key(L, SS, "OneFrameThreadLag")], TF, true, Impact::Low, MENU),

    // World detail
    choice("view_distance", "quality", "View distance", "Streaming and draw distance tier. Ultra High doubles the High distance and costs a lot of FPS in open maps.",
        &[key(E, SS, "ViewDistance"), key(L, SS, "ViewDistance")], VIEW_DISTANCE, "2", Impact::High, MENU),
    choice("game_detail", "quality", "Game detail", "What the menu calls Game Detail: population and clutter density.",
        &[key(E, SS, "PopulationAdjustment"), key(L, SS, "PopulationAdjustment")], DETAIL_LOW_TO_HIGH, "0", Impact::Medium, MENU),
    choice("detail_mode", "quality", "Detail mode", "Engine-level world detail (minor meshes and effects). Not exposed in the menu.",
        &[key(E, SS, "DetailMode")], LOW_MED_HIGH, "2", Impact::Low, NONE),
    slider("draw_scale", "quality", "Draw distance scale", "Multiplier on per-object cull distances.",
        &[key(E, SS, "MaxDrawDistanceScale")], (0.5, 3.0, 0.1), 1, "×", 1.0, Impact::Medium, EXPERIMENTAL),
    slider("foliage", "quality", "Foliage distance", "Grass and foliage draw radius. 1.0 is the maximum the engine honors; 0 removes grass.",
        &[key(E, SS, "FoliageDrawRadiusMultiplier"), key(L, SS, "FoliageDrawRadiusMultiplier")], (0.0, 1.0, 0.05), 2, "×", 1.0, Impact::Medium, MENU),
    slider("mesh_lod", "quality", "Character LOD bias", "Higher values use lower-detail models sooner. Negative forces high detail.",
        &[key(E, SS, "SkeletalMeshLODBias")], (-1.0, 4.0, 1.0), 0, "", 0.0, Impact::Low, NONE).labels(&[(-1.0, "Force high")]),
    slider("particle_lod", "quality", "Particle LOD bias", "Higher values use cheaper particle effects.",
        &[key(E, SS, "ParticleLODBias")], (0.0, 4.0, 1.0), 0, "", 0.0, Impact::Low, NONE),
    toggle("dynamic_lights", "quality", "Dynamic lights", "Muzzle flashes, elemental glows and other moving lights. Off is a big FPS gain but flattens the look.",
        &[key(E, SS, "DynamicLights")], TF, true, Impact::High, NONE),
    toggle("speedtree_leaves", "quality", "Tree leaves", "SpeedTree leaf cards.",
        &[key(E, SS, "SpeedTreeLeaves")], TF, true, Impact::Low, NONE),
    toggle("speedtree_fronds", "quality", "Tree fronds", "SpeedTree fronds.",
        &[key(E, SS, "SpeedTreeFronds")], TF, true, Impact::Low, NONE),
    choice("decals", "quality", "Bullet decals", "Bullet holes and blood splats.",
        &[key(E, SS, "NumberOfDecals"), key(L, SS, "NumberOfDecals")], DECALS, "1", Impact::Low, MENU),
    toggle("static_decals", "quality", "Static decals", "Level-placed decals (grime, graffiti).",
        &[key(E, SS, "StaticDecals")], TF, true, Impact::Low, NONE),
    toggle("dynamic_decals", "quality", "Dynamic decals", "Runtime decals from combat.",
        &[key(E, SS, "DynamicDecals")], TF, true, Impact::Low, NONE),
    toggle("no_autodetect", "quality", "Block auto-detect", "Stops the game from auto-detecting and overwriting your settings.",
        &[key(E, SS, "bAutoDetectSettings"), key(L, SS, "bAutoDetectSettings")], crate::tweaks::TF_INV, true, Impact::None, MENU),

    // Anti-aliasing & filtering
    toggle("fxaa", "aa", "FXAA", "Built-in post-process anti-aliasing. Cheap, a little soft.",
        &[key(E, SS, "FXAA"), key(L, SS, "FXAA")], TF, true, Impact::Low, MENU),
    choice("aniso", "aa", "Anisotropic filtering", "Texture sharpness at glancing angles. 16× is essentially free on modern GPUs.",
        &[key(E, SS, "MaxAnisotropy"), key(L, SS, "MaxAnisotropy")], ANISO, "4", Impact::Low, MENU),
    toggle("temporal_aa", "aa", "Temporal AA", "Unreal's early temporal AA. Unsupported by Gearbox; may ghost.",
        &[key(E, SS, "bAllowTemporalAA")], TF, false, Impact::Low, EXPERIMENTAL),

    // Textures & streaming
    choice("texture_quality", "textures", "Texture quality", "Menu texture quality (0 is the highest).",
        &[key(E, SS, "TextureQuality"), key(L, SS, "TextureQuality")], DETAIL_LOW_TO_HIGH, "0", Impact::Medium, MENU),
    custom("texture_bias", "textures", "Texture resolution cap", "Applies an LOD bias to world, character, weapon and vehicle textures. Half or Quarter saves lots of VRAM on old GPUs.",
        Control::Choice(TEXTURE_BIAS), read_texture_bias, write_texture_bias, C("0"), Impact::Medium, NONE),
    slider("pool_size", "textures", "Texture pool size", "Streaming pool in MB. Raising it reduces blurry textures, but the 32-bit exe can run out of memory above ~1000.",
        &[key(E, "TextureStreaming", "PoolSize")], (100.0, 1500.0, 20.0), 0, "MB", 160.0, Impact::Medium, NONE),
    custom("no_texture_fade", "textures", "Disable texture fade-in", "Zeroes the four MipFade speeds so textures appear instantly instead of blending from blurry versions.",
        Control::Toggle, read_texture_fade, write_texture_fade, B(false), Impact::None, NONE),
    toggle("only_stream_in", "textures", "Only stream in textures", "Never drops loaded mips, which fixes close-range blur, but can cause \"out of video memory\" crashes (fine with DXVK).",
        &[key(E, SS, "OnlyStreamInTextures")], TF, false, Impact::Medium, EXPERIMENTAL),

    // Outlines
    choice("post_chain", "outlines", "Outline style", "Classic draws the ink outlines. No Outlines keeps most of the cel look but forces ambient occlusion off. Cinematic drops outlines and color grading.",
        &[key(E, ENGINE, "DefaultPostProcessName")], POST_PROCESS, "WillowEngineMaterials.WillowScenePostProcess", Impact::Low, NONE),

    // Post-processing
    toggle("ao", "postfx", "Ambient occlusion", "Contact shadows in corners and crevices. Costs 10–15 FPS on older GPUs.",
        &[key(E, SS, "AmbientOcclusion"), key(L, SS, "AmbientOcclusion")], TF, true, Impact::High, MENU),
    toggle("bloom", "postfx", "Bloom", "Glow around bright light sources.",
        &[key(E, SS, "Bloom")], TF, true, Impact::Low, NONE),
    toggle("dof", "postfx", "Depth of field", "Background blur, most noticeable when aiming.",
        &[key(E, SS, "DepthOfField"), key(L, SS, "DepthOfField")], TF, true, Impact::Medium, MENU),
    toggle("motion_blur", "postfx", "Motion blur", "Camera and object motion blur.",
        &[key(E, SS, "MotionBlur"), key(L, SS, "MotionBlur")], TF, false, Impact::Low, MENU),
    toggle("lens_flares", "postfx", "Lens flares", "Flares from the sun and bright lights.",
        &[key(E, SS, "LensFlares")], TF, true, Impact::Low, NONE),
    toggle("light_shafts", "postfx", "Light shafts", "God rays. Off gives a big FPS gain outdoors.",
        &[key(E, SS, "bAllowLightShafts")], TF, true, Impact::Medium, NONE),
    toggle("distortion", "postfx", "Heat distortion", "Heat haze around fire, explosions and shields. Off helps in big fights.",
        &[key(E, SS, "Distortion")], TF, true, Impact::Medium, NONE),
    toggle("filtered_distortion", "postfx", "Filtered distortion", "Higher-quality filtering on the distortion effect.",
        &[key(E, SS, "FilteredDistortion")], TF, true, Impact::Low, NONE),
    toggle("drop_distortion", "postfx", "Drop particle distortion", "Skips distortion on particles entirely.",
        &[key(E, SS, "DropParticleDistortion")], TF, false, Impact::Low, NONE),
    toggle("radial_blur", "postfx", "Radial blur", "Screen blur used by some explosions and effects.",
        &[key(E, SS, "AllowRadialBlur")], TF, true, Impact::Low, EXPERIMENTAL),
    toggle("fog_volumes", "postfx", "Fog volumes", "Volumetric fog.",
        &[key(E, SS, "FogVolumes"), key(L, SS, "FogVolumes")], TF, false, Impact::Low, MENU),

    // Shadows
    toggle("dynamic_shadows", "shadows", "Dynamic shadows", "Shadows from characters, vehicles and moving objects. Off can add 40 FPS but scenes look flat and over-bright.",
        &[key(E, SS, "DynamicShadows")], TF, true, Impact::High, NONE),
    toggle("light_env_shadows", "shadows", "Light environment shadows", "Shadows cast by characters onto themselves and nearby geometry.",
        &[key(E, SS, "LightEnvironmentShadows")], TF, true, Impact::Medium, NONE),
    toggle("scene_shadows", "shadows", "Whole-scene shadows", "The sun's cascaded shadow maps.",
        &[key(E, SS, "bAllowWholeSceneDominantShadows")], TF, true, Impact::High, NONE),
    choice("scene_shadow_res", "shadows", "Sun shadow resolution", "Resolution of the main sun shadow map. 4096 is much sharper.",
        &[key(E, SS, "MaxWholeSceneDominantShadowResolution")], SCENE_SHADOW_RES, "2048", Impact::Medium, NONE),
    choice("shadow_res_min", "shadows", "Object shadow resolution (min)", "Smallest per-object shadow map.",
        &[key(E, SS, "MinShadowResolution"), key(L, SS, "MinShadowResolution")], SHADOW_RES, "1024", Impact::Low, MENU),
    choice("shadow_res_max", "shadows", "Object shadow resolution (max)", "Largest per-object shadow map.",
        &[key(E, SS, "MaxShadowResolution"), key(L, SS, "MaxShadowResolution")], SHADOW_RES, "1024", Impact::Medium, MENU),
    slider("shadow_bias", "shadows", "Shadow depth bias", "Raise slightly (0.015–0.02) to fix striped \"shadow acne\"; too high detaches shadows from objects.",
        &[key(E, SS, "SystemShadowDepthBias")], (0.005, 0.03, 0.001), 3, "", 0.012, Impact::None, NONE).recommended(0.015, 0.02),
    toggle("foreground_self_shadow", "shadows", "Weapon self-shadowing", "Lets your first-person weapon shadow itself.",
        &[key(E, SS, "bEnableForegroundSelfShadowing")], TF, false, Impact::Low, EXPERIMENTAL),

    // PhysX
    choice("physx", "physx", "PhysX effects", "Low disables GPU PhysX. Medium/High add debris, cloth and fluids. Higher levels have been linked to loot falling through the floor.",
        &[key(E, SS, "PhysXLevel"), key(L, SS, "PhysXLevel")], LOW_MED_HIGH, "0", Impact::High, MENU),
    toggle("fracture", "physx", "Fractured damage", "Destructible meshes that break into chunks.",
        &[key(E, SS, "bAllowFracturedDamage")], TF, true, Impact::Low, NONE),
    slider("fracture_parts", "physx", "Fracture debris amount", "How many fracture chunks survive.",
        &[key(E, SS, "NumFracturedPartsScale"), key(L, SS, "NumFracturedPartsScale")], (0.0, 1.0, 0.1), 1, "×", 0.0, Impact::Low, MENU),
    slider("physx_heap", "physx", "PhysX GPU heap", "GPU memory reserved for PhysX in MB. Some players cut this to 0 (with PhysX Low) to fix combat stutter.",
        &[key(E, ENGINE, "PhysXGpuHeapSize")], (0.0, 512.0, 32.0), 0, "MB", 128.0, Impact::Low, EXPERIMENTAL).labels(&[(0.0, "Off")]),
    slider("particle_cap", "physx", "Particle resize cap", "Upper limit on particle buffer growth. 0 is unlimited; 5000 is a safe cap for weak CPUs.",
        &[key(E, ENGINE, "MaxParticleResize")], (0.0, 10000.0, 250.0), 0, "", 0.0, Impact::Low, EXPERIMENTAL).labels(&[(0.0, "Unlimited")]),

    // Camera
    custom("fov1_value", "camera", "FOV hotkey value", "Field of view the hotkey sets. Weapons don't scale with it. Around 108 avoids distant fog vanishing while sprinting on ultrawide.",
        Control::Slider { min: 70.0, max: 150.0, step: 1.0, decimals: 0, unit: "°", labels: &[], recommended: None }, read_fov_value, write_fov_value, N(110.0), Impact::Low, NONE),
    custom("fov2_key", "camera", "FOV hotkey", "Adds a keybind that runs `fov <value>`. Press it after loading in; the FOV may reset after respawns or vehicles.",
        Control::Choice(FOV_KEYS), read_fov_key, write_fov_key, C(""), Impact::None, NONE),

    // Input
    choice("console_key", "input", "Console key", "Opens the full developer console (stat fps, fov, gamma, setres…). Required by most text mods.",
        &[key(I, "Engine.Console", "ConsoleKey")], CONSOLE_KEYS, "Undefine", Impact::None, NONE),
    choice("type_key", "input", "Quick-type console key", "Opens the one-line console prompt.",
        &[key(I, "Engine.Console", "TypeKey")], CONSOLE_KEYS, "Undefine", Impact::None, NONE),
    custom("unbind_remaster", "input", "Remove Ctrl+Shift+R remaster toggle", "The UHD pack added a hidden Ctrl+Shift+R bind that swaps HD and original assets, which is easy to hit by accident.",
        Control::Toggle, read_remaster_unbound, write_remaster_unbound, B(false), Impact::None, NONE),
    toggle("mouse_smoothing", "input", "Mouse smoothing (engine)", "Engine-level smoothing. The in-game option in profile.bin usually overrides this, so turn it off there too.",
        &[key(I, PLAYER_INPUT, "bEnableMouseSmoothing")], TF_LOWER, true, Impact::None, EXPERIMENTAL),

    // HUD
    custom("hide_news", "hud", "Hide main-menu news/ads", "Points the main menu's news feed (the Borderlands 3 upsell) at localhost.",
        Control::Toggle, read_hide_news, write_hide_news, B(false), Impact::None, NONE),
    toggle("level_timer", "hud", "Show level timer", "Developer timer on the HUD.",
        &[key(G, "WillowGame.WillowHUDGFxMovie", "bShowLevelTimer")], TF, false, Impact::None, EXPERIMENTAL),
    slider("weapon_card_time", "hud", "Weapon card display time", "How long the pending weapon card stays up.",
        &[key(G, "WillowGame.WillowHUD", "PendingWeaponCardDisplayTime")], (0.5, 10.0, 0.5), 1, "s", 3.0, Impact::None, EXPERIMENTAL),

    // Gameplay
    toggle("landing_shake", "gameplay", "Landing camera shake", "Shake when landing from a jump or fall.",
        &[key(G, PC, "bLandingShake")], TF_LOWER, true, Impact::None, EXPERIMENTAL),
    toggle("reload_dof", "gameplay", "Reload depth of field", "Blurs the background while reloading.",
        &[key(G, PC, "bDoDOFOnReload")], TF_LOWER, false, Impact::None, EXPERIMENTAL),
    toggle("echo_videos", "gameplay", "ECHO video logs", "Play the video part of ECHO recordings.",
        &[key(G, PC, "bDisableEchoVideos")], TF_LOWER_INV, true, Impact::None, EXPERIMENTAL),
    toggle("teleport_tunnel", "gameplay", "Fast-travel tunnel effect", "The digistruct tunnel shown while fast travelling.",
        &[key(G, PC, "bHideTeleportTunnel")], crate::tweaks::TF_INV, true, Impact::None, EXPERIMENTAL),
    custom("corpses", "gameplay", "Corpse cleanup", "How long bodies stay. Faster cleanup helps FPS in long fights. Very fast values can interfere with the Cult Following: Eternal Flame quest.",
        Control::Choice(CORPSES), read_corpses, write_corpses, C("vanilla"), Impact::Low, NONE),

    // Audio
    toggle("mute_unfocused", "audio", "Mute when unfocused", "Silence the game when alt-tabbed.",
        &[key(E, "WillowGame.WillowGameEngine", "bMuteAudioWhenNotInFocus"), key(L, "WillowGame.WillowGameEngine", "bMuteAudioWhenNotInFocus")], TF, true, Impact::None, MENU),
    toggle("voip", "audio", "Voice chat", "Built-in VoIP. Turning it off is a common co-op lag workaround.",
        &[key(E, "VoIP", "bHasVoiceEnabled")], TF_LOWER, true, Impact::None, NONE),
    slider("audio_channels", "audio", "Audio voices", "Maximum simultaneous sounds.",
        &[key(E, "XAudio2.XAudio2Device", "MaxChannels")], (16.0, 128.0, 8.0), 0, "", 32.0, Impact::Low, EXPERIMENTAL),

    // Startup
    custom("skip_logos", "startup", "Skip logo movies", "Removes the 2K and Gearbox logos. The Loading movie stays so loading screens still animate.",
        Control::Toggle, read_skip_logos, write_skip_logos, B(false), Impact::None, NONE),
    custom("skip_intro_confirm", "startup", "Skip intro cinematic without confirmation", "Lets you skip the opening cinematic with one press.",
        Control::Toggle, read_skip_intro_confirm, write_skip_intro_confirm, B(false), Impact::None, NONE),
    toggle("no_movies", "startup", "Disable all fullscreen movies", "Kills every Bink movie, including loading and some cutscenes.",
        &[key(E, MOVIES, "bForceNoMovies")], TF_UPPER, false, Impact::None, EXPERIMENTAL),

    // Network
    slider("net_speed", "network", "Internet speed", "Bandwidth hint for online co-op. Co-op lag guides use 20000–40000.",
        &[key(E, "Engine.Player", "ConfiguredInternetSpeed")], (5000.0, 100000.0, 5000.0), 0, "", 10000.0, Impact::None, NONE).recommended(20000.0, 40000.0),
    slider("lan_speed", "network", "LAN speed", "Bandwidth hint for LAN co-op.",
        &[key(E, "Engine.Player", "ConfiguredLanSpeed")], (5000.0, 100000.0, 5000.0), 0, "", 20000.0, Impact::None, NONE),
];

/// Written next to the exe when DXVK is installed.
pub const DXVK_CONF: &str = "# Vaulter DXVK profile for Borderlands 2 / The Pre-Sequel\n\
# Lower latency: at most one frame queued ahead of the GPU.\n\
d3d9.maxFrameLatency = 1\n\
# Force 16x anisotropic filtering on every texture.\n\
d3d9.samplerAnisotropy = 16\n";

/// "Modern Defaults": what a 2026 game ships with out of the box.
pub const MODERN_DEFAULTS: &[(&str, crate::tweaks::DefaultValue)] = &[
    ("window_mode", C("borderless")),
    ("vsync", B(false)),
    ("one_frame_lag", B(false)),
    ("pause_focus_windowed", B(false)),
    ("motion_blur", B(false)),
    ("dof", B(false)),
    ("reload_dof", B(false)),
    ("aniso", C("16")),
    ("fxaa", B(true)),
    ("no_texture_fade", B(true)),
    ("skip_logos", B(true)),
    ("skip_intro_confirm", B(true)),
    ("hide_news", B(true)),
    ("unbind_remaster", B(true)),
    ("no_autodetect", B(true)),
    ("console_key", C("Tilde")),
    ("corpses", C("balanced")),
];

/// "HD Upgrade": visuals beyond the in-game Ultra.
pub const HD_UPGRADE: &[(&str, crate::tweaks::DefaultValue)] = &[
    ("view_distance", C("3")),
    ("game_detail", C("0")),
    ("detail_mode", C("2")),
    ("foliage", N(1.0)),
    ("texture_quality", C("0")),
    ("pool_size", N(600.0)),
    ("scene_shadow_res", C("4096")),
    ("shadow_res_min", C("2048")),
    ("shadow_res_max", C("2048")),
    ("decals", C("2")),
];

const MODERN: Component = Component {
        id: "modern",
        name: "Modern defaults",
        summary: "Native resolution, smooth framerate, no blur, straight to the menu",
        description: "Borderless fullscreen at your native resolution, framerate matched to your monitor, lower input lag, no motion blur or blurry texture pop-in, skipped logos and ads, and the console on ~.",
        group: Group::Essentials,
        recommended: true,
        kind: ComponentKind::Settings { values: MODERN_DEFAULTS, display: Some(match_display) },
        requires: &[],
};
const LAUNCHER: Component = Component {
        id: "launcher",
        name: "Skip the launcher",
        summary: "Play goes straight into the game",
        description: "Play starts the game directly, and the old launcher can no longer overwrite your video settings.",
        group: Group::Essentials,
        recommended: true,
        kind: ComponentKind::LaunchArg("-NoLauncher"),
        requires: &[],
};
const LAA: Component = Component {
        id: "laa",
        name: "4 GB memory patch",
        summary: "Use 4 GB of RAM instead of 2 GB",
        description: "Lets the 32-bit game use 4 GB of RAM instead of 2 GB, preventing out-of-memory crashes with mods and HD textures.",
        group: Group::Essentials,
        recommended: true,
        kind: ComponentKind::ExePatch("laa"),
        requires: &[],
};
const DXVK: Component = Component {
        id: "dxvk",
        name: "DXVK Vulkan renderer",
        summary: "Vulkan renderer for steadier frame times",
        description: "Runs the 2012 DirectX 9 renderer on Vulkan: steadier frame times, fewer 32-bit memory crashes, and grass flicker fixed with 16x filtering. Needs an up-to-date driver with Vulkan 1.4 (NVIDIA GTX 16 / RTX, AMD RX 7000+, Intel Arc). If the game won't start afterwards, press Restore.",
        group: Group::Performance,
        recommended: true,
        kind: ComponentKind::Dxvk { target: DxvkTarget::D3d9Win32, exe_dir: "Binaries\\Win32", conf: DXVK_CONF },
        requires: &[],
};
const HD: Component = Component {
        id: "hd",
        name: "HD visual upgrade",
        summary: "Beyond-Ultra draw distance, shadows and textures",
        description: "Pushes past the in-game Ultra: maximum view distance and detail, 4K-class shadow maps and a bigger texture streaming pool.",
        group: Group::Performance,
        recommended: false,
        kind: ComponentKind::Settings { values: HD_UPGRADE, display: None },
        requires: &[],
};
const SDK: Component = Component {
        id: "sdk",
        name: "Python SDK (mod loader)",
        summary: "The community mod loader",
        description: "The community mod loader: adds a MODS menu to the main menu and powers the quality-of-life mods below. Needs the Visual C++ runtime.",
        group: Group::Mods,
        recommended: true,
        kind: ComponentKind::Sdk,
        requires: &[],
};

/// An SDK mod downloaded straight into `sdk_mods` and switched on.
#[allow(clippy::too_many_arguments)]
const fn sdk_mod(
    id: &'static str,
    name: &'static str,
    summary: &'static str,
    description: &'static str,
    group: Group,
    recommended: bool,
    url: &'static str,
    dest: &'static str,
    module: &'static str,
) -> Component {
    Component {
        id,
        name,
        summary,
        description,
        group,
        recommended,
        kind: ComponentKind::File { url, dest, enable: Some(module) },
        requires: &["sdk"],
    }
}

// Curated from the official SDK mod database; every URL is the one the
// database links (release "nightly" builds or the author's main branch).
const FIRING_FIX: Component = sdk_mod("firing_fix", "Firing Fix", "Guns no longer get stuck unable to fire", "Fixes guns getting stuck at 0 ammo and weapons that won't start firing after a swap. By ZetaDaemon.",
    Group::Fixes, true, "https://github.com/ZetaDaemon/willow2-sdk-mods/releases/download/nightly/firing_fix.sdkmod", "sdk_mods/firing_fix.sdkmod", "firing_fix");
const RELOAD_FIX: Component = sdk_mod("reload_fix", "Automatic Reload Fix", "Automatic reloads trigger reliably", "Fixes automatic reloads not triggering when a magazine runs dry. By RedxYeti.",
    Group::Fixes, true, "https://github.com/RedxYeti/bl2-willow2-sdkmods/raw/refs/heads/main/AutomaticReloadFix/AutomaticReloadFix.sdkmod", "sdk_mods/AutomaticReloadFix.sdkmod", "AutomaticReloadFix");
const QUICK_STARTUP: Component = sdk_mod("quick_startup", "Quick Startup", "Skip every intro straight to the menu", "Straight to the main menu: skips every intro movie and the press-start screen. By juso & mopioid.",
    Group::Mods, true, "https://github.com/juso40/bl2sdk-mods/raw/refs/heads/main/quick_startup/quick_startup.sdkmod", "sdk_mods/quick_startup.sdkmod", "quick_startup");
const ALT_USE_VENDORS: Component = sdk_mod("alt_use_vendors", "Quick Vendor Refill", "Top up ammo and health at a vendor in one press", "Borderlands 3 style: hold the alt-use key at a vendor to instantly top up ammo or health, or sell trash-marked gear. By apple1417.",
    Group::Mods, true, "https://github.com/apple1417/willow2-sdk-mods/releases/download/nightly/alt_use_vendors.sdkmod", "sdk_mods/alt_use_vendors.sdkmod", "alt_use_vendors");
const AUTO_PICKUP: Component = sdk_mod("auto_pickup", "Auto Pickup", "Money, health and chest loot picked up automatically", "Modern loot flow: money, health and chest contents are picked up automatically instead of one button press each. Known issue: a chest can occasionally refuse to open after its ammo is auto-collected, so it's opt-in. By RedxYeti.",
    Group::Mods, false, "https://github.com/RedxYeti/bl2-willow2-sdkmods/raw/refs/heads/main/AutoPickupTweaks/AutoPickupTweaks.sdkmod", "sdk_mods/AutoPickupTweaks.sdkmod", "AutoPickupTweaks");
const ITEM_LIGHTS: Component = sdk_mod("item_lights", "Loot Lights", "Loot glows in its rarity color", "Dropped loot glows in its rarity color, like the loot beams of newer Borderlands games. By RedxYeti.",
    Group::Mods, true, "https://github.com/RedxYeti/bl2-willow2-sdkmods/raw/refs/heads/main/ItemLights/ItemLights.sdkmod", "sdk_mods/ItemLights.sdkmod", "ItemLights");
const NO_ADS: Component = sdk_mod("no_ads", "No Ads", "No DLC ads in the menus", "Removes the DLC banners and message-of-the-day ads from the menus. By apple1417.",
    Group::Mods, true, "https://github.com/apple1417/willow2-sdk-mods/releases/download/nightly/no_ads.sdkmod", "sdk_mods/no_ads.sdkmod", "no_ads");
const BETTER_UI: Component = sdk_mod("better_ui", "Better Menu Controls", "Faster inventory and menu controls", "WASD menu navigation, equip without the slot prompt, reload to mark favorite/trash, quick respec. Changes some menu keys, so it's opt-in. By RedxYeti.",
    Group::Mods, false, "https://github.com/RedxYeti/bl2-willow2-sdkmods/raw/refs/heads/main/BetterUIControls/BetterUIControls.sdkmod", "sdk_mods/BetterUIControls.sdkmod", "BetterUIControls");
const INSTA_VEHICLES: Component = sdk_mod("insta_vehicles", "Instant Vehicles", "Summon a vehicle anywhere", "Borderlands 4 style: summon a vehicle anywhere instead of walking to a Catch-A-Ride. Opt-in. By ZetaDaemon.",
    Group::Mods, false, "https://github.com/ZetaDaemon/willow2-sdk-mods/releases/download/nightly/insta_vehicles.sdkmod", "sdk_mods/insta_vehicles.sdkmod", "insta_vehicles");
// The Pre-Sequel only. Insta Vehicles is BL2-only (its database entry says so);
// Alt Use Catch-A-Ride is the TPS way to get a vehicle without walking to a station.
const OZ_KIT_FIX: Component = sdk_mod("ozkit_fix", "Bomber Oz Kit Fix", "Bomber Oz kits throw their free grenade again", "Fixes grenade-throwing skills breaking the Bomber Oz kit's free grenade. By Zazk0u.",
    Group::Fixes, true, "https://raw.githubusercontent.com/Zazk0u/new-bl-sdk-mods/refs/heads/main/bomber_ozkit_fix/bomber_ozkit_fix.sdkmod", "sdk_mods/bomber_ozkit_fix.sdkmod", "bomber_ozkit_fix");
const CATCH_A_RIDE: Component = sdk_mod("catch_a_ride", "Quick Catch-A-Ride", "Deploy a vehicle in one press", "The alt-use key at a Catch-A-Ride deploys a vehicle instantly, skipping the menu. Pick the vehicle and its weapon in the mod's options. Opt-in. By Siggles.",
    Group::Mods, false, "https://github.com/Siggless/bl-sdk-mods/raw/refs/heads/main/AltUseCatchARide/AltUseCatchARide.sdkmod", "sdk_mods/AltUseCatchARide.sdkmod", "AltUseCatchARide");
const HIDE_MISSIONS: Component = sdk_mod("hide_missions", "Tidy Mission Log", "Mission log only shows what you've found", "Hides missions you haven't discovered yet, like the modern games' mission log. By apple1417.",
    Group::Mods, false, "https://github.com/apple1417/willow2-sdk-mods/releases/download/nightly/hide_undiscovered_missions.sdkmod", "sdk_mods/hide_undiscovered_missions.sdkmod", "hide_undiscovered_missions");

// ---- community patch -----------------------------------------------------------
// Sources are pinned to exact upstream commits so the merge is reproducible;
// the files are downloaded and merged on the user's own PC.

const BL2_GEARBOX_HOTFIXES: &str =
    "https://raw.githubusercontent.com/BLCM/OpenBLCMM/7ccf31d7a58e315c1f868ef9d31852a8afd3672c/src/resources/BL2/GBX_hotfixes.blcm";

/// Only UCP's fixes plus its neutral features; everything that changes
/// balance, loot or difficulty is left out.
const COMMUNITY_PATCH_SOURCES: &[TextSource] = &[
    TextSource {
        title: "Unofficial Community Patch (fixes only)",
        credit: "Unofficial Community Patch by shadowevil and the Community Patch Team (55tumbl, Apocalyptech, FromDarkHell, LightChaosman, Our Lord And Savior Gabe Newell and many more).",
        url: concat!("https://raw.githubusercontent.com/BLCM/BLCMods/b512a9b13f94cc11ce7f0b03c7f5080d8612418c", "/Borderlands%202%20mods/Community%20Patch%20Team/Patch.txt"),
        include: &[
            "Unofficial Community Patch 5.0.4/Fixes",
            "Unofficial Community Patch 5.0.4/Features/Automatically pick up currencies",
            "Unofficial Community Patch 5.0.4/Features/Make Gear Skills Tracked",
            "Unofficial Community Patch 5.0.4/Features/Make Infection skill tracked on HUD",
        ],
        exclude: &[
            "Unofficial Community Patch 5.0.4/Fixes/Skill Fixes/Add Melee Damage Buff to Infection Clouds",
            "Unofficial Community Patch 5.0.4/Fixes/Item Fixes/Fix Amp Shield Drain Scale",
            "Unofficial Community Patch 5.0.4/Fixes/Badass Enemy Fixes/Badass Stalker Buff & Loot",
            "Unofficial Community Patch 5.0.4/Fixes/Other Bug Fixes/Remove \"Permaslag\"",
            "Unofficial Community Patch 5.0.4/Fixes/Other Bug Fixes/Fix Voracidous and Hyperius having excess health with all three level cap DLCs",
        ],
    },
    TextSource {
        title: "Text Fixes",
        credit: "Text Fixes by apple1417.",
        url: concat!("https://raw.githubusercontent.com/BLCM/BLCMods/b512a9b13f94cc11ce7f0b03c7f5080d8612418c", "/Borderlands%202%20mods/apple1417/TextFixes.blcm"),
        include: &[],
        exclude: &[],
    },
    TextSource {
        title: "BL2 Sorted Fast Travel",
        credit: "BL2 Sorted Fast Travel by Apocalyptech (CC0).",
        url: concat!("https://raw.githubusercontent.com/BLCM/BLCMods/b512a9b13f94cc11ce7f0b03c7f5080d8612418c", "/Borderlands%202%20mods/Apocalyptech/BL2%20Sorted%20Fast%20Travel/BL2%20Sorted%20Fast%20Travel.blcm"),
        include: &[],
        exclude: &[],
    },
];

const TIMESAVER_SOURCES: &[TextSource] = &[TextSource {
    title: "BL2 Mega TimeSaver XL",
    credit: "BL2 Mega TimeSaver XL by Apocalyptech (CC0).",
    url: concat!("https://raw.githubusercontent.com/BLCM/BLCMods/b512a9b13f94cc11ce7f0b03c7f5080d8612418c", "/Borderlands%202%20mods/Apocalyptech/BL2%20Mega%20TimeSaver%20XL/BL2%20Mega%20TimeSaver%20XL.blcm"),
    include: &[],
    exclude: &[],
}];

const TEXT_MOD_LOADER: Component = Component {
    id: "tml",
    name: "Text Mod Loader",
    summary: "Runs the community patch automatically",
    description: "apple1417's SDK mod that runs text mods (like the community patch below) every time the game starts, without the console. Adds a list of text mods to the MODS menu.",
    group: Group::Fixes,
    recommended: true,
    kind: ComponentKind::SdkZip {
        url: "https://github.com/apple1417/willow2-sdk-mods/releases/download/nightly/text_mod_loader.zip",
        folder: "text_mod_loader",
    },
    requires: &["sdk"],
};

const COMMUNITY_PATCH: Component = Component {
    id: "community_patch",
    name: "Vaulter Community Patch",
    summary: "Hundreds of bug fixes, no balance changes",
    description: "Built on your PC from the Unofficial Community Patch's bug-fix section (rarity, skill, item, skin, audio and enemy loot-table fixes), apple1417's text fixes and Apocalyptech's alphabetical fast travel list, plus auto-pickup of Eridium, Torgue Tokens and Seraph crystals and tracked gear skills on the HUD. Balance, loot and difficulty changes are left out. Gearbox's official hotfixes are included, so it works offline.",
    group: Group::Fixes,
    recommended: true,
    kind: ComponentKind::TextPatch { game: "BL2", gearbox_url: BL2_GEARBOX_HOTFIXES, sources: COMMUNITY_PATCH_SOURCES },
    requires: &["tml"],
};

const TIMESAVER: Component = Component {
    id: "timesaver",
    name: "Mega TimeSaver XL",
    summary: "Chests, doors, lifts and fast travel about 5x faster",
    description: "Apocalyptech's mod that speeds up almost every waiting animation: chests, doors, drawbridges, elevators, fast-travel stations, slot machines and vehicle entry. A few side missions get slightly easier and the Ore Chasm vault symbol can no longer be reached, so it's opt-in.",
    group: Group::Mods,
    recommended: false,
    kind: ComponentKind::TextPatch { game: "BL2", gearbox_url: BL2_GEARBOX_HOTFIXES, sources: TIMESAVER_SOURCES },
    requires: &["tml"],
};

// ---- The Pre-Sequel's patch ------------------------------------------------------
// TPS's Community Patch has no fixes-only branch (most of it is a balance and
// loot overhaul), so only its bug fixes and neutral features are taken.

const TPS_GEARBOX_HOTFIXES: &str =
    "https://raw.githubusercontent.com/BLCM/OpenBLCMM/7ccf31d7a58e315c1f868ef9d31852a8afd3672c/src/resources/TPS/GBX_hotfixes.blcm";

const TPS_PATCH_SOURCES: &[TextSource] = &[
    TextSource {
        title: "Community Patch 2.3 (fixes only)",
        credit: "The Pre-Sequel Community Patch 2.3 by the Community Patch Team.",
        url: concat!("https://raw.githubusercontent.com/BLCM/BLCMods/b512a9b13f94cc11ce7f0b03c7f5080d8612418c", "/Pre%20Sequel%20Mods/Community%20Patch/Community%20Patch%202.3/patch.txt"),
        include: &[
            "patch/Patch 2.3/Features/No Broken Chests Near Denial Subroutine",
            "patch/Patch 2.3/Features/Fix Maliwan Cryo Pistols to actually Spawn.",
            "patch/Patch 2.3/Features/Moonstone autopickup",
            "patch/Patch 2.3/Features/Enable fast travels",
            "patch/Patch 2.3/Skinpool Reassignments (DO NOT UNCHECK)",
        ],
        exclude: &[],
    },
    TextSource {
        title: "TPS Sorted Fast Travel",
        credit: "TPS Sorted Fast Travel by Apocalyptech (CC0).",
        url: concat!("https://raw.githubusercontent.com/BLCM/BLCMods/b512a9b13f94cc11ce7f0b03c7f5080d8612418c", "/Pre%20Sequel%20Mods/Apocalyptech/TPS%20Sorted%20Fast%20Travel/TPS%20Sorted%20Fast%20Travel.blcm"),
        include: &[],
        exclude: &[],
    },
];

const TPS_TIMESAVER_SOURCES: &[TextSource] = &[TextSource {
    title: "TPS Mega TimeSaver XL",
    credit: "TPS Mega TimeSaver XL by Apocalyptech (CC0).",
    url: concat!("https://raw.githubusercontent.com/BLCM/BLCMods/b512a9b13f94cc11ce7f0b03c7f5080d8612418c", "/Pre%20Sequel%20Mods/Apocalyptech/TPS%20Mega%20TimeSaver%20XL/TPS%20Mega%20TimeSaver%20XL.blcm"),
    include: &[],
    exclude: &[],
}];

const TPS_COMMUNITY_PATCH: Component = Component {
    id: "community_patch",
    name: "Vaulter Community Patch",
    summary: "Bug fixes and Moonstone auto-pickup, no balance changes",
    description: "Built on your PC from the Pre-Sequel Community Patch's bug fixes (unlootable chests at the Denial Subroutine, Maliwan cryo pistols that never spawned, one-way fast-travel stations) and Apocalyptech's alphabetical fast travel list, plus Moonstones picked up automatically. Its weapon, skill, loot and difficulty changes are left out. Gearbox's official hotfixes are included, so it works offline.",
    group: Group::Fixes,
    recommended: true,
    kind: ComponentKind::TextPatch { game: "TPS", gearbox_url: TPS_GEARBOX_HOTFIXES, sources: TPS_PATCH_SOURCES },
    requires: &["tml"],
};

const TPS_TIMESAVER: Component = Component {
    id: "timesaver",
    name: "Mega TimeSaver XL",
    summary: "Containers, doors, lifts and fast travel about 5x faster",
    description: "Apocalyptech's mod that speeds up almost every waiting animation: containers (loot is ready at once), doors, lifts, fast-travel stations, oxygen generators, the Grinder, slot machines and vehicle entry, plus a few slow mission objects. A sped-up door can let enemies through sooner, so it's opt-in.",
    group: Group::Mods,
    recommended: false,
    kind: ComponentKind::TextPatch { game: "TPS", gearbox_url: TPS_GEARBOX_HOTFIXES, sources: TPS_TIMESAVER_SOURCES },
    requires: &["tml"],
};

pub const BL2_SETUP: &[Component] = &[
    MODERN, LAUNCHER, LAA, DXVK, HD, SDK, FIRING_FIX, RELOAD_FIX, TEXT_MOD_LOADER, COMMUNITY_PATCH,
    QUICK_STARTUP, ALT_USE_VENDORS, AUTO_PICKUP, ITEM_LIGHTS, NO_ADS, BETTER_UI, INSTA_VEHICLES,
    HIDE_MISSIONS, TIMESAVER,
];

/// The Pre-Sequel: the same foundation with its own patch, TimeSaver and mods.
pub const TPS_SETUP: &[Component] = &[
    MODERN, LAUNCHER, LAA, DXVK, HD, SDK, FIRING_FIX, RELOAD_FIX, OZ_KIT_FIX, TEXT_MOD_LOADER,
    TPS_COMMUNITY_PATCH, QUICK_STARTUP, ALT_USE_VENDORS, AUTO_PICKUP, ITEM_LIGHTS, NO_ADS, BETTER_UI,
    CATCH_A_RIDE, HIDE_MISSIONS, TPS_TIMESAVER,
];

pub const QUICK: &[crate::games::QuickSection] = &[
    crate::games::QuickSection {
        title: "Look & feel",
        blurb: "The classic comic-book ink, or a cleaner modern look.",
        tweaks: &["post_chain", "motion_blur", "dof"],
        quality_presets: &[],
    },
    crate::games::QuickSection {
        title: "Graphics quality",
        blurb: "Pick a starting point, then fine-tune the essentials.",
        tweaks: &["view_distance", "ao", "dynamic_shadows", "physx"],
        quality_presets: &["potato", "balanced", "ultra"],
    },
    crate::games::QuickSection {
        title: "Display",
        blurb: "How the game fits your screen.",
        tweaks: &["window_mode", "resolution", "fps_target", "vsync"],
        quality_presets: &[],
    },
    crate::games::QuickSection {
        title: "Field of view",
        blurb: "Past the 110 limit: pick a key, then press it after loading in.",
        tweaks: &["fov1_value", "fov2_key"],
        quality_presets: &[],
    },
    crate::games::QuickSection {
        title: "Comfort",
        blurb: "Small things that make the old game feel new.",
        tweaks: &["skip_logos", "console_key", "mute_unfocused", "landing_shake"],
        quality_presets: &[],
    },
];

/// BL2 and TPS load straight into a save with Quick Startup's `-Character=`.
pub const CAPTURE: crate::compare::CaptureProfile = crate::compare::CaptureProfile {
    load: crate::compare::CaptureLoad::CharacterArg,
    launch_args: &["-NoLauncher", "-nostartupmovies"],
    settings: &[
        (E, "Engine.Engine", "bSubtitlesForcedOff", "TRUE"),
        (E, SS, "Fullscreen", "False"),
        (E, SS, "WindowedFullscreen", "True"),
        (L, SS, "Fullscreen", "False"),
        (L, SS, "WindowedFullscreen", "True"),
    ],
    turn: 0,
    hud: crate::compare::HudHide::ToggleHud,
    prerequisite: Some("quick_startup"),
    saves: r"..\SaveData",
    save_subfolders: true,
};

const NV: &str = "https://international.download.nvidia.com/geforce-com/international/comparisons/borderlands-2-tweak-guide/borderlands-2-tweak-guide-";

/// Settings that get comparison images. Links point at Nvidia's official
/// interactive comparisons (viewed in the browser, never bundled).
pub const COMPARISONS: &[crate::compare::Comparison] = &[
    cmp("post_chain", None, true),
    cmp("ao", Some("ambient-occlusion-interactive-comparison.html"), true),
    cmp("light_shafts", Some("light-shafts-interactive-comparison.html"), true),
    cmp("bloom", None, true),
    cmp("dynamic_shadows", Some("dynamic-shadows-interactive-comparison.html"), true),
    cmp("scene_shadow_res", Some("shadow-resolution-interactive-comparison-1.html"), true),
    cmp("shadow_res_max", Some("shadow-resolution-interactive-comparison-2.html"), false),
    cmp("view_distance", Some("view-distance-interactive-comparison.html"), true),
    cmp("foliage", Some("foliage-distance-interactive-comparison.html"), false),
    cmp("detail_mode", Some("detail-mode-interactive-comparison.html"), true),
    cmp("texture_quality", Some("texture-quality-interactive-comparison.html"), true),
    cmp("aniso", Some("anisotropic-filtering-interactive-comparison.html"), true),
    cmp("fxaa", Some("fxaa-anti-aliasing-interactive-comparison.html"), true),
    cmp("dof", Some("depth-of-field-interactive-comparison.html"), false),
    cmp("decals", Some("bullet-decals-interactive-comparison.html"), false),
    cmp("dynamic_lights", None, true),
];

const fn cmp(tweak: &'static str, nv_page: Option<&'static str>, capture: bool) -> crate::compare::Comparison {
    crate::compare::Comparison { tweak, link: nv_page, capture }
}

/// Full URL of a comparison's Nvidia page.
pub fn nvidia_url(page: &str) -> String {
    format!("{NV}{page}")
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "fixes",
        name: "Community Essentials",
        rarity: Rarity::Pearlescent,
        description: "The fixes nearly every guide recommends: skip logos, console on ~, no texture fade, hide the menu ad, remove the accidental remaster bind, block auto-detect.",
        values: &[
            ("skip_logos", B(true)),
            ("skip_intro_confirm", B(true)),
            ("console_key", C("Tilde")),
            ("no_texture_fade", B(true)),
            ("hide_news", B(true)),
            ("unbind_remaster", B(true)),
            ("no_autodetect", B(true)),
        ],
    },
    Preset {
        id: "clean",
        name: "Clean Look",
        rarity: Rarity::Epic,
        description: "No ink outlines, no motion blur or depth of field, instant textures and 16× filtering, for a cleaner, sharper image.",
        values: &[
            ("post_chain", C("WillowEngineMaterials.RyanScenePostProcess")),
            ("motion_blur", B(false)),
            ("dof", B(false)),
            ("reload_dof", B(false)),
            ("no_texture_fade", B(true)),
            ("aniso", C("16")),
            ("fxaa", B(true)),
        ],
    },
    Preset {
        id: "potato",
        name: "Potato Mode",
        rarity: Rarity::Common,
        description: "For old laptops and handhelds: lowest detail, no dynamic shadows or lights, quarter-res textures, no grass.",
        values: &[
            ("view_distance", C("0")),
            ("game_detail", C("2")),
            ("detail_mode", C("0")),
            ("foliage", N(0.0)),
            ("texture_quality", C("2")),
            ("texture_bias", C("2")),
            ("pool_size", N(160.0)),
            ("aniso", C("4")),
            ("ao", B(false)),
            ("bloom", B(false)),
            ("light_shafts", B(false)),
            ("distortion", B(false)),
            ("dof", B(false)),
            ("motion_blur", B(false)),
            ("lens_flares", B(false)),
            ("dynamic_shadows", B(false)),
            ("dynamic_lights", B(false)),
            ("decals", C("0")),
            ("physx", C("0")),
            ("mesh_lod", N(2.0)),
            ("particle_lod", N(2.0)),
            ("corpses", C("fast")),
        ],
    },
    Preset {
        id: "performance",
        name: "Competitive FPS",
        rarity: Rarity::Uncommon,
        description: "Maximum framerate while keeping the art style: uncapped FPS, no light shafts, distortion, AO or DOF, PhysX Low, fast corpse cleanup.",
        values: &[
            ("fps_lock", C("6")),
            ("vsync", B(false)),
            ("one_frame_lag", B(false)),
            ("ao", B(false)),
            ("light_shafts", B(false)),
            ("distortion", B(false)),
            ("dof", B(false)),
            ("motion_blur", B(false)),
            ("lens_flares", B(false)),
            ("physx", C("0")),
            ("foliage", N(0.5)),
            ("view_distance", C("2")),
            ("corpses", C("balanced")),
        ],
    },
    Preset {
        id: "balanced",
        name: "Balanced",
        rarity: Rarity::Rare,
        description: "High quality with the costliest effects trimmed: High view distance, 2048 shadows, no motion blur or DOF, PhysX Low.",
        values: &[
            ("view_distance", C("2")),
            ("game_detail", C("0")),
            ("foliage", N(0.75)),
            ("aniso", C("16")),
            ("ao", B(true)),
            ("light_shafts", B(true)),
            ("dof", B(false)),
            ("motion_blur", B(false)),
            ("scene_shadow_res", C("2048")),
            ("physx", C("0")),
            ("pool_size", N(400.0)),
        ],
    },
    ULTRA_BL2,
    Preset {
        id: "vanilla",
        name: "Factory Settings",
        rarity: Rarity::Seraph,
        description: "Every tweak back to the game's shipped default. Resolution and window mode are left alone.",
        values: &[],
    },
];


/// The top preset's settings; each game names it after its own world.
const ULTRA_VALUES: &[(&str, crate::tweaks::DefaultValue)] = &[
        ("view_distance", C("3")),
        ("game_detail", C("0")),
        ("detail_mode", C("2")),
        ("foliage", N(1.0)),
        ("texture_quality", C("0")),
        ("texture_bias", C("0")),
        ("pool_size", N(600.0)),
        ("aniso", C("16")),
        ("ao", B(true)),
        ("bloom", B(true)),
        ("light_shafts", B(true)),
        ("dynamic_shadows", B(true)),
        ("scene_shadow_res", C("4096")),
        ("shadow_res_min", C("2048")),
        ("shadow_res_max", C("2048")),
        ("decals", C("2")),
        ("dynamic_lights", B(true)),
    ];

const ULTRA_BL2: Preset = Preset {
    id: "ultra",
    name: "Pandora Ultra",
    rarity: Rarity::Legendary,
    description: "Everything maxed and a bit beyond: Ultra High view distance, 4096 sun shadows, a larger texture pool and every effect on. Needs a strong GPU.",
    values: ULTRA_VALUES,
};

/// The Pre-Sequel: the same presets, with the top one named for Elpis.
pub const TPS_PRESETS: &[Preset] = &[
    PRESETS[0],
    PRESETS[1],
    PRESETS[2],
    PRESETS[3],
    PRESETS[4],
    Preset { name: "Elpis Ultra", ..ULTRA_BL2 },
    PRESETS[6],
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ini::IniDoc;

    #[test]
    fn every_tweak_has_a_known_category_and_unique_id() {
        let mut ids = std::collections::HashSet::new();
        for t in TWEAKS {
            assert!(ids.insert(t.id), "duplicate tweak id {}", t.id);
            assert!(CATEGORIES.iter().any(|c| c.id == t.category), "{} has unknown category", t.id);
        }
        for p in PRESETS {
            for (id, _) in p.values {
                assert!(TWEAKS.iter().any(|t| t.id == *id), "preset {} references {id}", p.id);
            }
        }
    }

    #[test]
    fn lod_bias_rewrite() {
        let v = "(MinLODSize=1,MaxLODSize=4096,LODBias=0,MinMagFilter=Aniso)";
        assert_eq!(with_lod_bias(v, "2").unwrap(), "(MinLODSize=1,MaxLODSize=4096,LODBias=2,MinMagFilter=Aniso)");
    }

    #[test]
    fn fov_bind_round_trip() {
        let mut config = ConfigSet::from_docs(&[(I, IniDoc::parse("[Engine.PlayerInput]\r\nBindings=(Name=\"F9\",Command=\"shot\")\r\n"))]);
        write_fov_value(&mut config, &Value::Num(120.0));
        assert_eq!(fov_bind(&config), Some(("F7".into(), 120.0)));
        write_fov_key(&mut config, &Value::Choice("F8"));
        assert_eq!(fov_bind(&config), Some(("F8".into(), 120.0)));
        write_fov_key(&mut config, &Value::Choice(""));
        assert_eq!(fov_bind(&config), None);
        assert_eq!(config.doc(I).unwrap().get_all("Engine.PlayerInput", "Bindings").len(), 1);
    }
}

/// Read-only checks against a real install; run with `cargo test -- --ignored`.
#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::core::ini::IniDoc;

    fn config_dir() -> Option<std::path::PathBuf> {
        let dir = dirs::document_dir()?.join(r"My Games\Borderlands 2\WillowGame\Config");
        dir.is_dir().then_some(dir)
    }

    #[test]
    #[ignore]
    fn live_files_round_trip_and_every_tweak_reads() {
        let Some(dir) = config_dir() else { return };
        for (_, name) in INI_FILES {
            let path = dir.join(name);
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(IniDoc::from_bytes(&bytes).to_bytes(), bytes, "{name} did not round-trip");
        }
        let config = ConfigSet::load(&dir, INI_FILES);
        for t in TWEAKS {
            let v = t.read(&config);
            println!("{:<22} {:?}", t.id, v);
            assert!(!matches!(v, Some(Value::Unknown(_))) || t.id == "resolution", "{} read an unknown value", t.id);
        }
    }

    /// Copies the real configs to a temp folder, flips every tweak, saves,
    /// reloads and checks each value reads back. Never touches the originals.
    #[test]
    #[ignore]
    fn live_every_tweak_writes_and_reads_back() {
        let Some(dir) = config_dir() else { return };
        let sandbox = std::env::temp_dir().join("vaulter-sandbox");
        let _ = std::fs::remove_dir_all(&sandbox);
        std::fs::create_dir_all(sandbox.join("LauncherConfig")).unwrap();
        for (_, name) in INI_FILES {
            std::fs::copy(dir.join(name), sandbox.join(name)).unwrap();
        }
        let original = ConfigSet::load(&sandbox, INI_FILES);
        let mut config = original.clone();
        let mut expected = Vec::new();
        // Combined Simple-mode controls share keys with the tweaks below;
        // check them in isolation first.
        for t in TWEAKS.iter().filter(|t| t.category == "quick") {
            if let Control::Choice(options) = t.control {
                for o in options {
                    let mut scratch = original.clone();
                    t.write(&mut scratch, &Value::Choice(o.value));
                    assert_eq!(t.read(&scratch), Some(Value::Choice(o.value)), "{} = {}", t.id, o.value);
                }
            }
        }
        for t in TWEAKS.iter().filter(|t| t.category != "quick") {
            let current = t.read(&config);
            let next = match t.control {
                Control::Toggle => Value::Bool(!matches!(current, Some(Value::Bool(true)))),
                Control::Slider { min, max, .. } => {
                    if current.as_ref().and_then(Value::as_num) == Some(max) { Value::Num(min) } else { Value::Num(max) }
                }
                Control::Choice(options) => {
                    let o = options.iter().find(|o| current != Some(Value::Choice(o.value)) && !o.value.is_empty()).unwrap();
                    Value::Choice(o.value)
                }
            };
            t.write(&mut config, &next);
            expected.push((t, next));
        }
        // The FOV key write runs after the value write, as in a real apply.
        config.save_dirty().unwrap();
        let reloaded = ConfigSet::load(&sandbox, INI_FILES);
        for (t, want) in &expected {
            assert_eq!(t.read(&reloaded).as_ref(), Some(want), "{} did not read back", t.id);
        }
        // Only a modest number of lines should differ per file.
        for (_, name) in INI_FILES {
            let before = std::fs::read_to_string(dir.join(name)).unwrap();
            let after = std::fs::read_to_string(sandbox.join(name)).unwrap();
            let changed = before.lines().zip(after.lines()).filter(|(a, b)| a != b).count();
            println!("{name}: {} -> {} lines, {changed} differ positionally", before.lines().count(), after.lines().count());
        }
        let _ = std::fs::remove_dir_all(&sandbox);
    }

    #[test]
    #[ignore]
    fn live_exe_patches_are_recognized() {
        let game = &crate::games::bl2::GAME;
        let Some(install) = crate::core::detect::detect(&game.detect_spec()) else { return };
        let Ok(bytes) = std::fs::read(install.root.join(game.exe)) else { return };
        for p in crate::games::bl2::PATCHES {
            println!("{:<14} {:?}", p.id, p.state(&bytes));
            assert_ne!(p.state(&bytes), crate::core::binpatch::PatchState::Unsupported, "{}", p.id);
        }
    }
}
