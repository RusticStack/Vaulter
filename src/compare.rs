//! Per-setting comparison images.
//!
//! Images are captured on the user's own PC by the capture tool: for every
//! value of a setting it writes the config, launches the game straight into a
//! save (via the Quick Startup mod's `-Character=` switch), and a tiny helper
//! SDK mod hides the HUD, takes a screenshot and quits. Vaulter never
//! ships third-party screenshots; settings Nvidia covered also link to its
//! interactive comparison page.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use crate::core::backup;
use crate::tweaks::{ConfigSet, Control, Tweak, Value};

#[derive(Clone, Copy)]
pub struct Comparison {
    pub tweak: &'static str,
    /// Nvidia's interactive comparison for this setting, if one exists.
    pub link: Option<&'static str>,
    /// Include this setting in capture runs.
    pub capture: bool,
}

/// How the capture tool gets a game into a save and what it writes around
/// each shot. One per engine family.
pub struct CaptureProfile {
    pub load: CaptureLoad,
    /// Launch switches for every shot.
    pub launch_args: &'static [&'static str],
    /// (file id, section, key, value) written for every shot: borderless so
    /// the window can be read, no subtitles, no launcher or intro movies.
    pub settings: &'static [(&'static str, &'static str, &'static str, &'static str)],
    /// Camera yaw added after spawning (65536 = a full turn), to face a
    /// better view than the save's.
    pub turn: i32,
    /// How the HUD is hidden: the `togglehud` command, or closing the HUD's
    /// Scaleform movie.
    pub hud: HudHide,
    /// Setup component that must be installed first (e.g. Quick Startup).
    pub prerequisite: Option<&'static str>,
    /// Folder holding the saves, relative to the config folder.
    pub saves: &'static str,
    /// Whether saves sit in per-profile subfolders of `saves`.
    pub save_subfolders: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CaptureLoad {
    /// `-Character=<save>` (Quick Startup mod).
    CharacterArg,
    /// The helper drives the title screen: Continue, single player, start.
    /// Loads the most recently played character.
    MainMenu,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HudHide {
    ToggleHud,
    CloseMovie,
}

/// Save files, newest first.
pub fn find_saves(config_dir: &Path, profile: &CaptureProfile) -> Vec<String> {
    let dir = config_dir.join(profile.saves);
    let entries: Vec<fs::DirEntry> = if profile.save_subfolders {
        fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .flat_map(|p| fs::read_dir(p.path()).into_iter().flatten().flatten())
            .collect()
    } else {
        fs::read_dir(&dir).into_iter().flatten().flatten().collect()
    };
    let mut saves: Vec<(std::time::SystemTime, String)> = entries
        .into_iter()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            (lower.starts_with("save") && lower.ends_with(".sav")).then_some(())?;
            Some((e.metadata().ok()?.modified().ok()?, name))
        })
        .collect();
    saves.sort_by_key(|s| std::cmp::Reverse(s.0));
    saves.into_iter().map(|(_, n)| n).collect()
}

pub fn comparisons_dir() -> PathBuf {
    backup::data_dir().join("comparisons")
}

/// Every value a capture run shoots for a tweak, with its display label.
pub fn capture_values(tweak: &Tweak) -> Vec<(Value, String)> {
    match tweak.control {
        Control::Toggle => vec![(Value::Bool(false), "Off".into()), (Value::Bool(true), "On".into())],
        Control::Choice(options) => options
            .iter()
            .map(|o| (Value::Choice(o.value), o.label.to_string()))
            .collect(),
        Control::Slider { .. } => Vec::new(),
    }
}

fn file_stem(value: &Value) -> String {
    let raw = match value {
        Value::Bool(b) => if *b { "on" } else { "off" }.to_string(),
        Value::Choice("") => "none".into(),
        Value::Choice(c) => c.to_string(),
        Value::Num(n) => format!("{n}"),
        Value::Unknown(s) => s.clone(),
    };
    raw.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

pub fn image_path(game_id: &str, tweak: &str, value: &Value) -> PathBuf {
    comparisons_dir().join(game_id).join(tweak).join(format!("{}.jpg", file_stem(value)))
}

/// Pages ask which images exist on every render, so file checks are cached
/// until something writes images (a pack download, a capture, previews).
static EXISTS: Mutex<Option<std::collections::HashMap<PathBuf, bool>>> = Mutex::new(None);

fn exists(path: &Path) -> bool {
    let mut cache = EXISTS.lock().unwrap_or_else(|e| e.into_inner());
    *cache.get_or_insert_with(Default::default).entry(path.to_path_buf()).or_insert_with(|| path.is_file())
}

/// Forget cached file checks after images were added or removed.
pub fn invalidate_images() {
    *EXISTS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The values whose images exist, in the same order as `images`.
pub fn captured_values(game_id: &str, tweak: &Tweak) -> Vec<Value> {
    capture_values(tweak)
        .into_iter()
        .map(|(v, _)| v)
        .filter(|v| exists(&image_path(game_id, tweak.id, v)))
        .collect()
}

/// Captured images for a tweak, in option order: (label, path).
pub fn images(game_id: &str, tweak: &Tweak) -> Vec<(String, PathBuf)> {
    capture_values(tweak)
        .into_iter()
        .map(|(v, label)| (label, image_path(game_id, tweak.id, &v)))
        .filter(|(_, p)| exists(p))
        .collect()
}

/// Width of the preview copies used in the settings pages. Full-size images
/// (1600px) are only decoded when the viewer is drawn big enough to need them.
const PREVIEW_WIDTH: u32 = 640;
/// Width of the copies the viewer uses at usual window sizes.
const MEDIUM_WIDTH: u32 = 1024;

fn preview_dir() -> PathBuf {
    backup::data_dir().join("previews")
}

fn medium_dir() -> PathBuf {
    backup::data_dir().join("previews-medium")
}

/// The smallest copy of a comparison image that still covers `pixels`
/// device pixels of width: the viewer in a normal window decodes a 1024px
/// copy (2.4 MB in memory) instead of the 1600px original (5.8 MB), and the
/// original only when drawn larger than that.
pub fn sized(full: &Path, pixels: f32) -> PathBuf {
    let copy = |dir: PathBuf| full.strip_prefix(comparisons_dir()).ok().map(|rel| dir.join(rel)).filter(|p| exists(p));
    let chosen = if pixels <= PREVIEW_WIDTH as f32 {
        copy(preview_dir()).or_else(|| copy(medium_dir()))
    } else if pixels <= MEDIUM_WIDTH as f32 {
        copy(medium_dir())
    } else {
        None
    };
    chosen.unwrap_or_else(|| full.to_path_buf())
}

/// The small copy of a comparison image, or the image itself until the
/// preview has been made.
pub fn preview(full: &Path) -> PathBuf {
    let small = full.strip_prefix(comparisons_dir()).map(|rel| preview_dir().join(rel));
    match small {
        Ok(p) if exists(&p) => p,
        _ => full.to_path_buf(),
    }
}

/// Makes any missing preview copies for a game's images. Returns how many
/// were written. Runs off the UI thread.
pub fn make_previews(game_id: &str) -> usize {
    let root = comparisons_dir().join(game_id);
    let mut made = 0;
    for tweak_dir in fs::read_dir(&root).into_iter().flatten().flatten() {
        for entry in fs::read_dir(tweak_dir.path()).into_iter().flatten().flatten() {
            let full = entry.path();
            if full.extension().is_none_or(|e| e != "jpg") {
                continue;
            }
            let Ok(rel) = full.strip_prefix(comparisons_dir()) else { continue };
            let fresh = |p: &Path| p.metadata().and_then(|m| m.modified()).ok();
            let stale: Vec<(PathBuf, u32)> = [(preview_dir().join(rel), PREVIEW_WIDTH), (medium_dir().join(rel), MEDIUM_WIDTH)]
                .into_iter()
                .filter(|(copy, _)| !fresh(copy).is_some_and(|s| fresh(&full).is_some_and(|f| s >= f)))
                .collect();
            if stale.is_empty() {
                continue;
            }
            let Ok(img) = image::open(&full) else { continue };
            for (copy, width) in stale {
                let resized = img.resize(width, u32::MAX, image::imageops::FilterType::Triangle);
                if fs::create_dir_all(copy.parent().expect("has parent")).is_ok()
                    && resized.to_rgb8().save_with_format(&copy, image::ImageFormat::Jpeg).is_ok()
                {
                    made += 1;
                }
            }
        }
    }
    if made > 0 {
        invalidate_images();
    }
    made
}

// ---- capture ---------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct CaptureProgress {
    pub done: usize,
    pub total: usize,
    pub current: String,
    pub log: Vec<String>,
    pub finished: bool,
    pub cancel: bool,
}

pub struct CaptureRequest {
    pub game_id: &'static str,
    pub root: PathBuf,
    pub exe: PathBuf,
    pub config_dir: PathBuf,
    pub ini_files: &'static [(&'static str, &'static str)],
    /// Save file name for Quick Startup, e.g. `Save0001.sav`.
    pub save: String,
    pub settle_seconds: u32,
    pub shots: Vec<(&'static Tweak, Value, String)>,
    pub profile: &'static CaptureProfile,
}

const HELPER_DIR: &str = "sdk_mods/vault_capture";

/// The in-game half of the capture tool. It does nothing unless
/// `job.json` exists next to it, which only happens during a capture run.
const HELPER_PY: &str = r#""""Vaulter comparison capture helper.

Only active while Vaulter is capturing comparison images: it gets the
game into a save (driving the title screen when the game has no load switch),
waits for the world to settle, closes popups, hides the HUD and weapon, tells
Vaulter to grab the frame, then quits. Removed automatically when the
capture run ends.
"""

import json
from pathlib import Path
from typing import Any

import unrealsdk
from mods_base import build_mod, hook
from unrealsdk import logging

_DIR = Path(__file__).parent
_state: dict[str, Any] = {"job": None, "elapsed": 0.0, "stage": 0, "menu": 0, "menu_t": 0.0}


def _live(cls_name: str) -> list[Any]:
    try:
        return [o for o in unrealsdk.find_all(cls_name) if not o.Name.startswith("Default__")]
    except ValueError:
        return []


def _close_dialogs() -> None:
    """Close popups such as the golden keys / SHiFT notice shown on load."""
    for dialog in _live("WillowGFxDialogBox"):
        try:
            dialog.Close()
            logging.info(f"[vault_capture] closed dialog {dialog.Name}")
        except Exception as ex:  # noqa: BLE001 - never let a popup stop the capture
            logging.warning(f"[vault_capture] couldn't close {dialog.Name}: {ex}")


def _load_job() -> None:
    try:
        _state["job"] = json.loads((_DIR / "job.json").read_text())
    except (OSError, ValueError):
        _state["job"] = None
    _state.update(elapsed=0.0, stage=0, menu=0, menu_t=0.0)
    logging.info(f"[vault_capture] enabled, job={_state['job']}")


def _drive_menu(dt: float) -> None:
    """Title screen -> Continue -> single player lobby -> start, a step every few seconds."""
    _state["menu_t"] += dt
    if _state["menu_t"] < 6:
        return
    _state["menu_t"] = 0.0
    step = _state["menu"]
    try:
        if step == 0:
            for movie in _live("WillowGFxMoviePressStart"):
                movie.extContinue()
        elif step == 1:
            for movie in _live("WillowGFxMenuFrontend"):
                movie.OpenSP()
        else:
            lobbies = [lobby for lobby in _live("WillowGFxMenuLobby2") if "Transient" in str(lobby)]
            if lobbies:
                lobbies[-1].DoStartGame()
        logging.info(f"[vault_capture] menu step {step}")
    except Exception as ex:  # noqa: BLE001 - retried on the next step
        logging.warning(f"[vault_capture] menu step {step}: {ex}")
    _state["menu"] = min(step + 1, 2)


def _hide_hud(pc: Any, how: str) -> None:
    if how == "close":
        for movie in _live("WillowHUDGFxMovie"):
            try:
                movie.Close(False)
            except Exception as ex:  # noqa: BLE001
                logging.warning(f"[vault_capture] couldn't close the HUD: {ex}")
        try:
            pc.myHUD.bShowHUD = False
        except Exception:  # noqa: BLE001
            pass
    else:
        pc.ConsoleCommand("togglehud", False)


def _hide_weapon(pc: Any) -> None:
    """Hide the first-person weapon and arms so the scene is unobstructed."""
    pawn = pc.Pawn
    targets = (
        ("arms", lambda: pawn.Arms),
        ("weapon", lambda: pawn.Weapon.FirstPersonMesh),
        ("weapon (3rd person)", lambda: pawn.Weapon.ThirdPersonMesh),
    )
    for label, target in targets:
        try:
            target().SetHidden(True)
        except Exception as ex:  # noqa: BLE001 - cosmetic, never block the shot
            logging.warning(f"[vault_capture] couldn't hide {label}: {ex}")


def _turn(pc: Any, yaw: int) -> None:
    if not yaw:
        return
    rot = unrealsdk.make_struct("Rotator", Pitch=pc.Rotation.Pitch, Yaw=(pc.Rotation.Yaw + yaw) % 65536, Roll=0)
    pc.Rotation = rot
    try:
        pc.ClientSetRotation(rot)
    except Exception as ex:  # noqa: BLE001
        logging.warning(f"[vault_capture] couldn't turn: {ex}")


@hook("Engine.PlayerController:PlayerTick")
def _tick(obj: Any, args: Any, _ret: Any, _func: Any) -> None:
    job = _state["job"]
    if job is None:
        return
    map_name = str(obj.WorldInfo.GetMapName(False)).lower()
    if obj.Pawn is None or map_name in ("loader", "menumap"):
        _state["elapsed"] = 0.0
        if job.get("load") == "menu" and map_name == "menumap":
            _drive_menu(args.DeltaTime)
        return
    if _state["elapsed"] == 0.0 and _state["stage"] == 0:
        logging.info(f"[vault_capture] player spawned in {map_name}")
        _close_dialogs()
    _state["elapsed"] += args.DeltaTime
    stage = _state["stage"]
    token = str(job.get("token", ""))
    if stage == 0 and _state["elapsed"] >= job.get("settle_seconds", 20):
        logging.info("[vault_capture] world settled; hiding HUD and weapon")
        _close_dialogs()
        _turn(obj, int(job.get("turn", 0)))
        _hide_hud(obj, job.get("hud", "toggle"))
        _hide_weapon(obj)
        _state["stage"], _state["elapsed"] = 1, 0.0
    elif stage == 1 and _state["elapsed"] >= 1.5:
        # Vaulter grabs the frame from the window, then answers.
        _close_dialogs()
        (_DIR / "ready.flag").write_text(token)
        logging.info("[vault_capture] ready for capture")
        _state["stage"], _state["elapsed"] = 2, 0.0
    elif stage == 2:
        captured = (_DIR / "captured.flag")
        if (captured.exists() and captured.read_text() == token) or _state["elapsed"] >= 30:
            logging.info("[vault_capture] done, quitting")
            (_DIR / "done.flag").write_text(token)
            _state["stage"] = 3
            obj.ConsoleCommand("exit", False)


build_mod(
    name="Vaulter Capture Helper",
    author="Vaulter",
    version="1.1",
    description="Takes comparison screenshots during a Vaulter capture run. Inactive otherwise.",
    hooks=[_tick],
    auto_enable=True,
    on_enable=_load_job,
)
"#;

/// Whether a process with the exe's file name is running. Uses a Toolhelp
/// snapshot (no child process, well under a millisecond).
#[cfg(windows)]
pub(crate) fn game_running(exe: &Path) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    let Some(name) = exe.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
        return false;
    };
    // SAFETY: standard snapshot walk; the handle is closed before returning.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut ok = Process32FirstW(snap, &mut entry) != 0;
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            if String::from_utf16_lossy(&entry.szExeFile[..len]).to_lowercase() == name {
                found = true;
                break;
            }
            ok = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
        found
    }
}

#[cfg(not(windows))]
pub(crate) fn game_running(_exe: &Path) -> bool {
    false
}

/// Whether the program at exactly `exe` is running. Unlike `game_running`
/// this checks the full path, for generic names like `Launcher.exe` that
/// other apps use too. A process whose path can't be read doesn't count.
#[cfg(windows)]
pub(crate) fn program_running(exe: &Path) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
    };
    let Some(name) = exe.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
        return false;
    };
    let want = normalize_path(exe);
    let utf16 = |s: &[u16]| String::from_utf16_lossy(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]);
    // SAFETY: standard snapshot walks; every handle is closed before returning.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut ok = Process32FirstW(snap, &mut entry) != 0;
        while ok && !found {
            if utf16(&entry.szExeFile).to_lowercase() == name {
                // A process's first module is its exe, with the full path.
                let modules = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, entry.th32ProcessID);
                if modules != INVALID_HANDLE_VALUE {
                    let mut module: MODULEENTRY32W = std::mem::zeroed();
                    module.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
                    found = Module32FirstW(modules, &mut module) != 0
                        && normalize_path(Path::new(&utf16(&module.szExePath))) == want;
                    CloseHandle(modules);
                }
            }
            ok = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
        found
    }
}

#[cfg(not(windows))]
pub(crate) fn program_running(_exe: &Path) -> bool {
    false
}

/// A path in the form Windows compares it: case-insensitive, either slash.
pub(crate) fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

pub(crate) fn kill_game(exe: &Path) {
    if let Some(name) = exe.file_name() {
        let _ = Command::new("taskkill").args(["/IM", &name.to_string_lossy(), "/F"]).output();
    }
}

/// Stores a frame as a 1600px-wide JPEG.
fn store_image(img: image::DynamicImage, dest: &Path) -> Result<()> {
    let img = if img.width() > 1600 {
        img.resize(1600, u32::MAX, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    fs::create_dir_all(dest.parent().expect("has parent"))?;
    img.to_rgb8().save_with_format(dest, image::ImageFormat::Jpeg)?;
    invalidate_images();
    Ok(())
}

/// Marks the run finished however `run` exits (error or panic), so the UI
/// never sticks in "capturing".
struct FinishOnDrop(Arc<Mutex<CaptureProgress>>);

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        let mut p = self.0.lock().unwrap_or_else(|e| e.into_inner());
        p.finished = true;
        p.done = p.total;
        p.current.clear();
    }
}

/// Puts the user's configs and game folder back if a run stops early
/// (error or panic). Closes the game first so it can't write over them.
struct RestoreOnDrop {
    armed: bool,
    exe: PathBuf,
    snapshot: backup::Backup,
    helper: PathBuf,
    helper_settings: PathBuf,
    /// Game whose "capture in progress" marker this run wrote.
    marker_game: String,
}

impl RestoreOnDrop {
    fn restore(&mut self) -> Result<()> {
        self.armed = false;
        kill_game(&self.exe);
        let result = backup::restore(&self.snapshot).context("restoring your settings after capture");
        let _ = fs::remove_dir_all(&self.helper);
        let _ = fs::remove_file(&self.helper_settings);
        // Put back: the next start no longer has anything to recover.
        if result.is_ok() {
            fs::remove_file(marker_path(&self.marker_game)).ok();
        }
        result
    }
}

/// What an unfinished capture changed, written before it touches anything.
/// If the app is closed or killed mid-run (so `RestoreOnDrop` never runs),
/// the next start finds it and puts everything back.
#[derive(serde::Serialize, serde::Deserialize)]
struct CaptureMarker {
    exe: PathBuf,
    snapshot: PathBuf,
    helper: PathBuf,
    helper_settings: PathBuf,
}

fn marker_path(game_id: &str) -> PathBuf {
    backup::data_dir().join("captures").join(format!("{game_id}.json"))
}

/// Undoes every comparison capture that didn't finish last time (the app
/// was closed or crashed mid-run): restores the settings snapshot and removes
/// the helper mod. Returns one line per game, saying what happened.
pub fn recover_interrupted() -> Vec<Result<String>> {
    let Ok(entries) = fs::read_dir(backup::data_dir().join("captures")) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| recover_one(&p))
        .collect()
}

fn recover_one(marker: &Path) -> Result<String> {
    let text = fs::read_to_string(marker).context("reading the unfinished capture's record")?;
    let Ok(m) = serde_json::from_str::<CaptureMarker>(&text) else {
        fs::remove_file(marker).ok();
        bail!("an unfinished comparison capture left a damaged record; restore \"Before comparison capture\" from Backups if your settings look wrong");
    };
    if game_running(&m.exe) {
        bail!("a comparison capture didn't finish; close the game and restart Vaulter to put your settings back");
    }
    let snapshot = backup::load(&m.snapshot).context("the unfinished capture's settings snapshot is gone")?;
    backup::restore(&snapshot).context("putting your settings back after an unfinished capture")?;
    let _ = fs::remove_dir_all(&m.helper);
    let _ = fs::remove_file(&m.helper_settings);
    fs::remove_file(marker).ok();
    Ok("A comparison capture didn't finish last time; your settings were put back".into())
}

impl Drop for RestoreOnDrop {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.restore();
        }
    }
}

pub fn run(req: CaptureRequest, progress: Arc<Mutex<CaptureProgress>>) -> Result<usize> {
    let _finish = FinishOnDrop(progress.clone());
    let log = |msg: String| {
        if let Ok(mut p) = progress.lock() {
            p.log.push(msg);
        }
    };
    if game_running(&req.exe) {
        bail!("close the game before starting a capture");
    }
    let helper = req.root.join(HELPER_DIR);
    let helper_settings = req.root.join("sdk_mods").join("settings").join("vault_capture.json");

    // One snapshot of the configs up front, including files that don't exist
    // yet, so any the capture creates are removed again, restored no matter
    // how the run ends.
    let config_paths: Vec<PathBuf> = req.ini_files.iter().map(|(_, n)| req.config_dir.join(n)).collect();
    let snapshot = backup::create(req.game_id, "Before comparison capture", &config_paths)?;
    let marker = CaptureMarker {
        exe: req.exe.clone(),
        snapshot: snapshot.dir.clone(),
        helper: helper.clone(),
        helper_settings: helper_settings.clone(),
    };
    let mut guard = RestoreOnDrop {
        armed: true,
        exe: req.exe.clone(),
        snapshot,
        helper: helper.clone(),
        helper_settings: helper_settings.clone(),
        marker_game: req.game_id.to_string(),
    };
    crate::core::atomic::write(&marker_path(req.game_id), &serde_json::to_vec_pretty(&marker)?)
        .context("recording the capture so it can be undone")?;
    fs::create_dir_all(&helper)?;
    fs::write(helper.join("__init__.py"), HELPER_PY)?;
    // New SDK mods start disabled; this settings file switches the helper on.
    fs::create_dir_all(helper_settings.parent().expect("has parent"))?;
    fs::write(&helper_settings, "{\n    \"enabled\": true\n}\n")?;
    let original = ConfigSet::load(&req.config_dir, req.ini_files);
    if let Some(f) = original.unreadable().next() {
        bail!("{} couldn't be read; close the game and try again", f.path.display());
    }

    let result = (|| -> Result<usize> {
        let mut captured = 0;
        for (i, (tweak, value, label)) in req.shots.iter().enumerate() {
            if progress.lock().map(|p| p.cancel).unwrap_or(false) {
                log("Cancelled".into());
                break;
            }
            if let Ok(mut p) = progress.lock() {
                p.current = format!("{} ({label})", tweak.label);
                p.done = i;
            }
            let mut config = original.clone();
            tweak.write(&mut config, value);
            // Borderless (so the frame can be read from the window), no
            // subtitles, no launcher or intros. Restored afterwards.
            for &(file, section, name, v) in req.profile.settings {
                if config.file(file).is_some_and(|f| f.exists) {
                    config.set(&crate::tweaks::key(file, section, name), v);
                }
            }
            config.save_dirty()?;

            let token = format!("{}-{i}", std::process::id());
            for flag in ["ready.flag", "captured.flag", "done.flag"] {
                let _ = fs::remove_file(helper.join(flag));
            }
            let job = serde_json::json!({
                "settle_seconds": req.settle_seconds,
                "token": token,
                "load": if req.profile.load == CaptureLoad::MainMenu { "menu" } else { "arg" },
                "turn": req.profile.turn,
                "hud": if req.profile.hud == HudHide::CloseMovie { "close" } else { "toggle" },
            });
            fs::write(helper.join("job.json"), job.to_string())?;

            let mut args: Vec<String> = req.profile.launch_args.iter().map(|a| a.to_string()).collect();
            if req.profile.load == CaptureLoad::CharacterArg {
                args.push(format!("-Character={}", req.save));
            }
            let clock = Instant::now();
            let child = Command::new(&req.exe)
                .args(&args)
                .current_dir(req.exe.parent().unwrap_or(&req.root))
                .spawn()
                .context("launching the game")?;
            let flag_is = |name: &str| fs::read_to_string(helper.join(name)).is_ok_and(|t| t == token);

            // Wait for the helper to say the frame is ready (max 4 minutes).
            let mut frame = None;
            while clock.elapsed() < Duration::from_secs(240) {
                if progress.lock().map(|p| p.cancel).unwrap_or(false) {
                    break;
                }
                if flag_is("ready.flag") {
                    // Steam's overlay shows a one-off "Access Steam features"
                    // pop-up for a few seconds after it starts; let it pass.
                    std::thread::sleep(Duration::from_secs(7));
                    frame = Some(crate::core::winshot::capture_process_window(child.id()));
                    fs::write(helper.join("captured.flag"), &token)?;
                    break;
                }
                std::thread::sleep(Duration::from_millis(300));
            }
            // Let the helper quit the game, then make sure it's gone.
            let quit_by = Instant::now() + Duration::from_secs(25);
            while game_running(&req.exe) && Instant::now() < quit_by {
                std::thread::sleep(Duration::from_millis(500));
            }
            kill_game(&req.exe);
            match frame {
                None => log(format!("{} ({label}): timed out (did the save load?)", tweak.label)),
                Some(Err(e)) => log(format!("{} ({label}): {e:#}", tweak.label)),
                Some(Ok(img)) => {
                    store_image(image::DynamicImage::ImageRgb8(img), &image_path(req.game_id, tweak.id, value))?;
                    captured += 1;
                    log(format!("{} ({label}): captured", tweak.label));
                }
            }
        }
        Ok(captured)
    })();

    // Always put the user's settings and game folder back.
    guard.restore()?;
    result
}

// ---- comparison image packs ------------------------------------------------------------
// Captured image sets ship inside the exe under `assets/comparisons/<game>/`,
// in the same `<tweak>/<value>.jpg` layout `unpack_pack` writes, and are
// copied to the data dir on first run. Packs published on GitHub releases are
// still downloaded as a fallback for games with comparisons but no bundled set.

/// GitHub repository whose releases host the comparison packs.
pub const IMAGE_REPO: &str = "RusticStack/Vaulter";

/// The image sets compiled into the exe, one folder per game id.
#[derive(rust_embed::RustEmbed)]
#[folder = "assets/comparisons"]
struct ComparisonImages;

fn pack_url(game_id: &str) -> String {
    format!("https://github.com/{IMAGE_REPO}/releases/download/comparisons-{game_id}/comparisons-{game_id}.zip")
}

pub fn has_local_images(game_id: &str) -> bool {
    fs::read_dir(comparisons_dir().join(game_id)).is_ok_and(|mut d| d.next().is_some())
}

/// Downloads and unpacks the published image pack for a game. Fallback for
/// games whose set isn't bundled into the exe.
pub fn download_pack(game_id: &str) -> Result<usize> {
    let tmp = std::env::temp_dir().join(format!("vaulter-comparisons-{game_id}.zip"));
    crate::core::net::download(&pack_url(game_id), &tmp)?;
    let count = unpack_pack(&tmp, &comparisons_dir().join(game_id));
    fs::remove_file(&tmp).ok();
    invalidate_images();
    count
}

/// Copies a game's bundled images into `comparisons_dir()`, writing only
/// files that are missing; an image already there may be the user's own
/// re-shot capture, which always wins. Returns how many were written; zero
/// for games with no bundled set.
pub fn extract_pack(game_id: &str) -> usize {
    let written = extract_pack_to(game_id, &comparisons_dir().join(game_id));
    if written > 0 {
        invalidate_images();
    }
    written
}

fn extract_pack_to(game_id: &str, dest: &Path) -> usize {
    let prefix = format!("{game_id}/");
    let mut written = 0;
    for path in ComparisonImages::iter() {
        let Some(rel) = path.strip_prefix(prefix.as_str()) else { continue };
        let parts: Vec<&str> = rel.split('/').collect();
        if !is_pack_entry(&parts) {
            continue;
        }
        let Some(file) = ComparisonImages::get(path.as_ref()) else { continue };
        let out = dest.join(parts[0]).join(parts[1]);
        if out.exists() {
            continue;
        }
        if fs::create_dir_all(out.parent().expect("has parent")).is_ok() && fs::write(&out, &file.data).is_ok() {
            written += 1;
        }
    }
    written
}

/// `true` for `<tweak>/<value>.jpg` entries; anything else is ignored and
/// nothing can escape the destination folder.
fn is_pack_entry(parts: &[&str]) -> bool {
    parts.len() == 2
        && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)) && *p != "..")
        && parts[1].ends_with(".jpg")
}

/// Extracts `<tweak>/<value>.jpg` entries into `dest`; anything else is ignored.
fn unpack_pack(zip_path: &Path, dest: &Path) -> Result<usize> {
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path)?).context("image pack isn't a valid zip")?;
    let mut count = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().replace('\\', "/");
        let parts: Vec<&str> = name.split('/').collect();
        if entry.is_dir() || !is_pack_entry(&parts) {
            continue;
        }
        let out = dest.join(parts[0]).join(parts[1]);
        fs::create_dir_all(out.parent().expect("has parent"))?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes)?;
        fs::write(out, bytes)?;
        count += 1;
    }
    Ok(count)
}

/// Maintainer tool: zips a captured set for publishing.
pub fn package(game_id: &str, out: &Path) -> Result<usize> {
    use std::io::Write as _;
    let src = comparisons_dir().join(game_id);
    let file = fs::File::create(out).with_context(|| format!("creating {}", out.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    // JPEGs are already compressed.
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut count = 0;
    let mut tweaks: Vec<_> = fs::read_dir(&src)?.flatten().filter(|e| e.path().is_dir()).collect();
    tweaks.sort_by_key(|e| e.file_name());
    for tweak in tweaks {
        let mut images: Vec<_> = fs::read_dir(tweak.path())?.flatten().collect();
        images.sort_by_key(|e| e.file_name());
        for image in images {
            let name = format!("{}/{}", tweak.file_name().to_string_lossy(), image.file_name().to_string_lossy());
            zip.start_file(name, options)?;
            zip.write_all(&fs::read(image.path())?)?;
            count += 1;
        }
    }
    zip.finish()?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_frames_as_resized_jpeg() {
        let dir = std::env::temp_dir().join("vaulter-compare-test");
        let _ = fs::remove_dir_all(&dir);
        let frame = image::RgbImage::from_pixel(2560, 1440, image::Rgb([200, 120, 20]));
        let dest = dir.join("out/on.jpg");
        store_image(image::DynamicImage::ImageRgb8(frame), &dest).unwrap();
        let out = image::open(&dest).unwrap();
        assert_eq!((out.width(), out.height()), (1600, 900));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_packs_only_unpack_plain_jpegs() {
        use std::io::Write as _;
        let dir = std::env::temp_dir().join("vaulter-pack-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("pack.zip");
        let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for name in ["ao/on.jpg", "../evil.jpg", "ao/../../evil2.jpg", "ao/script.exe", "deep/a/b.jpg"] {
            zip.start_file(name, opts).unwrap();
            zip.write_all(b"x").unwrap();
        }
        zip.finish().unwrap();
        let dest = dir.join("out");
        assert_eq!(unpack_pack(&zip_path, &dest).unwrap(), 1);
        assert!(dest.join("ao/on.jpg").is_file());
        assert!(!dir.join("evil.jpg").exists() && !dest.join("ao/script.exe").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundled_sets_follow_the_disk_layout() {
        // `assets/comparisons/<game>/<tweak>/<value>.jpg` mirrors what
        // `unpack_pack` writes into the data dir.
        let files: Vec<_> = ComparisonImages::iter().collect();
        assert!(files.iter().any(|f| f.starts_with("bl2/")), "bl2's set is bundled");
        for f in files {
            let parts: Vec<&str> = f.split('/').collect();
            assert_eq!(parts.len(), 3, "{f}");
            assert!(is_pack_entry(&parts[1..]), "{f}");
        }
    }

    #[test]
    fn bundled_images_never_replace_user_captures() {
        let dest = std::env::temp_dir().join("vaulter-extract-test");
        let _ = fs::remove_dir_all(&dest);
        let total = extract_pack_to("bl2", &dest);
        assert!(total > 0);
        assert_eq!(extract_pack_to("bl2", &dest), 0, "nothing missing, nothing written");
        // The user re-shoots one image and another goes missing.
        let mut shots = fs::read_dir(&dest).unwrap().flatten().flat_map(|d| fs::read_dir(d.path()).unwrap().flatten());
        let (mine, gone) = (shots.next().unwrap().path(), shots.next().unwrap().path());
        fs::write(&mine, b"my capture").unwrap();
        fs::remove_file(&gone).unwrap();
        assert_eq!(extract_pack_to("bl2", &dest), 1);
        assert_eq!(fs::read(&mine).unwrap(), b"my capture");
        assert!(gone.is_file());
        assert_eq!(extract_pack_to("bl1e", &dest.join("none")), 0, "no bundled set");
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn program_check_needs_the_exact_path() {
        let me = std::env::current_exe().unwrap();
        assert!(program_running(&me) || !cfg!(windows));
        let upper = PathBuf::from(me.to_string_lossy().to_uppercase());
        assert!(program_running(&upper) || !cfg!(windows), "paths compare case-insensitively");
        let elsewhere = std::env::temp_dir().join(me.file_name().unwrap());
        assert!(!program_running(&elsewhere), "same name in another folder isn't it");
    }

    #[test]
    fn file_names_are_safe_and_distinct() {
        assert_eq!(file_stem(&Value::Choice("WillowEngineMaterials.RyanScenePostProcess")), "WillowEngineMaterials_RyanScenePostProcess");
        assert_eq!(file_stem(&Value::Bool(true)), "on");
        assert_eq!(file_stem(&Value::Choice("")), "none");
    }

    #[test]
    fn every_comparison_points_at_a_real_tweak() {
        for game in crate::games::all() {
            for c in game.comparisons {
                let t = game.tweak(c.tweak).unwrap_or_else(|| panic!("{}: {}", game.id, c.tweak));
                if c.capture {
                    assert!(!capture_values(t).is_empty(), "{} can't be captured", c.tweak);
                }
            }
        }
    }
}

#[cfg(test)]
mod run_tests {
    use super::*;

    /// A run that fails right after starting (no game exe) must still put the
    /// configs back byte for byte, remove its helper and mark itself finished.
    #[test]
    fn failed_capture_restores_everything() {
        let base = std::env::temp_dir().join("vaulter-capture-fail");
        let _ = fs::remove_dir_all(&base);
        let config_dir = base.join("Config");
        let root = base.join("Game");
        fs::create_dir_all(config_dir.join("LauncherConfig")).unwrap();
        fs::create_dir_all(root.join("sdk_mods")).unwrap();
        let engine = "[SystemSettings]\r\nFullscreen=True\r\nBloom=True\r\n[Engine.Engine]\r\nbSubtitlesForcedOff=FALSE\r\n";
        fs::write(config_dir.join("WillowEngine.ini"), engine).unwrap();
        let tweak = crate::games::bl2::GAME.tweak("bloom").unwrap();
        let progress = Arc::new(Mutex::new(CaptureProgress { total: 1, ..Default::default() }));
        let req = CaptureRequest {
            game_id: "test-capture",
            root: root.clone(),
            exe: root.join("Missing.exe"),
            config_dir: config_dir.clone(),
            ini_files: crate::games::willow::INI_FILES,
            save: "Save0001.sav".into(),
            settle_seconds: 1,
            shots: vec![(tweak, Value::Bool(false), "Off".into())],
            profile: &crate::games::willow::CAPTURE,
        };
        assert!(run(req, progress.clone()).is_err());
        assert_eq!(fs::read_to_string(config_dir.join("WillowEngine.ini")).unwrap(), engine);
        assert!(!config_dir.join("WillowGame.ini").exists(), "files the capture created are removed");
        assert!(!root.join(HELPER_DIR).exists());
        assert!(progress.lock().unwrap().finished);
        assert!(!marker_path("test-capture").exists(), "nothing left to recover");
        for b in backup::list("test-capture") {
            let _ = backup::delete(&b);
        }
        let _ = fs::remove_dir_all(&base);
    }

    /// The app died mid-capture: the next start puts the settings back from
    /// the marker and removes the helper.
    #[test]
    fn an_interrupted_capture_is_undone_on_the_next_start() {
        let game = "test-capture-recover";
        let base = std::env::temp_dir().join("vaulter-capture-recover");
        let _ = fs::remove_dir_all(&base);
        let ini = base.join("Config/WillowEngine.ini");
        let helper = base.join("Game").join(HELPER_DIR);
        let helper_settings = base.join("Game/sdk_mods/settings/vault_capture.json");
        fs::create_dir_all(ini.parent().unwrap()).unwrap();
        fs::write(&ini, "[SystemSettings]\r\nFullscreen=True\r\n").unwrap();
        let snapshot = backup::create(game, "Before comparison capture", std::slice::from_ref(&ini)).unwrap();
        let marker = CaptureMarker { exe: base.join("Game/Missing.exe"), snapshot: snapshot.dir.clone(), helper: helper.clone(), helper_settings: helper_settings.clone() };
        crate::core::atomic::write(&marker_path(game), &serde_json::to_vec(&marker).unwrap()).unwrap();
        // Mid-shot state when the process died.
        fs::write(&ini, "[SystemSettings]\r\nFullscreen=False\r\n").unwrap();
        fs::create_dir_all(&helper).unwrap();
        fs::write(helper.join("__init__.py"), "x").unwrap();
        fs::create_dir_all(helper_settings.parent().unwrap()).unwrap();
        fs::write(&helper_settings, "{}").unwrap();

        recover_one(&marker_path(game)).unwrap();
        assert_eq!(fs::read_to_string(&ini).unwrap(), "[SystemSettings]\r\nFullscreen=True\r\n");
        assert!(!helper.exists() && !helper_settings.exists());
        assert!(!marker_path(game).exists());
        for b in backup::list(game) {
            let _ = backup::delete(&b);
        }
        let _ = fs::remove_dir_all(&base);
    }
}
