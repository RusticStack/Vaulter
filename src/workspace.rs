//! Shared application state. Pages hold an `Entity<Workspace>`, read from it
//! while rendering, and call its methods to change anything; every mutation
//! notifies observers so all pages stay in sync.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use gpui::Context;
use serde::{Deserialize, Serialize};

use crate::core::backup::{self, Backup};
use crate::core::binpatch::PatchState;
use crate::core::detect::{self, Install, Store};
use crate::games::{self, GameDef, Mode, PageKind};
use crate::mods::{self, ModEntry, SdkStatus};
use crate::patches;
use crate::setup::{self, ComponentKind, Status};
use crate::tweaks::{ConfigSet, Preset, Tweak, Value};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub active_game: Option<String>,
    pub manual_installs: HashMap<String, PathBuf>,
    pub manual_config_dirs: HashMap<String, PathBuf>,
    /// Mark ini files read-only after applying so the game can't revert them.
    pub lock_configs_after_apply: bool,
    /// Launch switches the user ticked, per game.
    pub launch_args: HashMap<String, Vec<String>>,
    /// Which way the Play button starts each game, picked in its dropdown.
    pub launch_mode: HashMap<String, LaunchMode>,
    pub max_backups: Option<usize>,
    pub mode: Mode,
    pub sound_muted: bool,
    /// Advanced: show ini file/section/key under each setting.
    pub show_file_details: bool,
    /// Loop the launcher's menu music.
    pub music: bool,
    /// The first-run welcome has been dismissed.
    pub welcomed: bool,
    /// How often Vaulter redraws its own window (not the game).
    pub frame_rate: AppFrameRate,
}

/// Vaulter's own frame rate while it animates. Nothing redraws at all
/// while nothing changes, and input is never capped (see
/// `gpui::set_max_frame_rate`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppFrameRate {
    Fps30,
    /// The even fraction of the display's refresh rate nearest 45 fps.
    #[default]
    Balanced,
    Fps60,
    /// The display's refresh rate.
    Display,
}

impl AppFrameRate {
    pub const ALL: [AppFrameRate; 4] = [AppFrameRate::Fps30, AppFrameRate::Balanced, AppFrameRate::Fps60, AppFrameRate::Display];

    pub fn cap(self) -> Option<u32> {
        match self {
            AppFrameRate::Fps30 => Some(30),
            AppFrameRate::Balanced => Some(balanced_fps(crate::win11::display_refresh_hz().unwrap_or(60))),
            AppFrameRate::Fps60 => Some(60),
            AppFrameRate::Display => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            AppFrameRate::Fps30 => "30",
            AppFrameRate::Balanced => "balanced",
            AppFrameRate::Fps60 => "60",
            AppFrameRate::Display => "display",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }

    pub fn apply(self) {
        gpui::set_max_frame_rate(self.cap());
    }
}

/// The frame rate nearest 45 that divides `refresh_hz` evenly, so every
/// frame lands on a vblank and animations don't judder: 45 at 180 Hz, 48 at
/// 144, 40 at 120, and 60 on a 60 Hz display (already its own rate).
pub fn balanced_fps(refresh_hz: u32) -> u32 {
    let every = ((refresh_hz as f32 / 45.).round() as u32).max(1);
    refresh_hz / every
}

impl AppSettings {
    fn path() -> PathBuf {
        backup::data_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Saves via a temp file and a rename, so a torn write can't reset every
    /// setting to its default on the next start.
    pub fn save(&self) {
        let saved = serde_json::to_vec_pretty(self)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| crate::core::atomic::write(&Self::path(), &bytes));
        if let Err(e) = saved {
            eprintln!("couldn't save app settings: {e:#}");
        }
    }
}

/// The comparison viewer: two options of one setting, split by a slider.
#[derive(Clone, Copy, Debug)]
pub struct Preview {
    pub tweak: &'static str,
    /// Image indices shown left and right of the divider.
    pub left: usize,
    pub right: usize,
    /// Divider position, 0..=1 from the left edge.
    pub split: f32,
}

/// Which tweaks the tweak pages list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TweakFilter {
    All,
    /// Differs from the game's shipped default (on disk or staged).
    Modified,
    Staged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepState {
    Queued,
    Running,
    Done(String),
    Failed(String),
}

/// How the Play button starts the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    /// The game exe with the user's configured launch options.
    Normal,
    /// The game exe, forcing the skip-launcher switch for this run,
    /// so the launcher can't re-apply its copy of the video settings.
    Direct,
    /// The game's own launcher program (BL2/TPS's Launcher.exe).
    Launcher,
}

/// What a setup run is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupGoal {
    /// Install the selected components that aren't active yet.
    Install,
    /// Remove every component Vaulter installed.
    Uninstall,
    /// Put back everything Vaulter wrote: settings the game or its
    /// launcher rewrote, exe patches a file check reverted, missing files.
    Reapply,
}

/// Progress of a Simple-mode setup (or restore, or re-apply) run.
pub struct SetupRun {
    pub goal: SetupGoal,
    pub steps: Vec<(&'static str, StepState)>,
    pub finished: bool,
}

/// Re-apply's closing steps, listed after the upgrades it re-installs.
const SETTINGS_STEP: &str = "Your settings";
const PATCHES_STEP: &str = "Exe patches";

/// Whether "Re-apply" re-runs a setup component: only one Vaulter
/// installed that has gone missing. Never something the user didn't
/// install or removed, however recommended. Settings bundles and exe
/// patches come back through the re-apply record instead (their values and
/// patches are remembered when applied), and launch switches can't be
/// rewritten behind our back.
fn reapply_wants(kind: &ComponentKind, status: &Status, ours: bool) -> bool {
    match kind {
        ComponentKind::Settings { .. } | ComponentKind::ExePatch(_) | ComponentKind::LaunchArg(_) => false,
        _ => ours && !status.is_active() && !matches!(status, Status::Blocked(_)),
    }
}

/// The launch mode Play really uses: the picked one when it can work,
/// else `Normal`. `Direct` needs a game with a launcher to skip; `Launcher`
/// also needs the launcher's exe on disk (checked only then).
fn usable_launch_mode(picked: Option<LaunchMode>, has_launcher: bool, launcher_found: impl FnOnce() -> bool) -> LaunchMode {
    match picked {
        Some(LaunchMode::Direct) if has_launcher => LaunchMode::Direct,
        Some(LaunchMode::Launcher) if has_launcher && launcher_found() => LaunchMode::Launcher,
        _ => LaunchMode::Normal,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

/// A button on a toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastAction {
    /// Put back the values the last save replaced.
    Undo,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
    pub action: Option<ToastAction>,
}

/// What the last settings write replaced, for "Undo".
pub struct Undo {
    game: usize,
    values: Vec<(&'static str, Value)>,
}

/// Newer versions found online (checked once at startup).
#[derive(Default)]
pub struct Updates {
    /// (tag, release page) of a newer Vaulter.
    pub app: Option<(String, String)>,
    /// Latest mod SDK release tag, per SDK repo (BL1E's SDK isn't BL2's).
    pub sdk: HashMap<&'static str, String>,
}

#[derive(Clone, Debug, Default)]
pub struct ExeInfo {
    pub size: u64,
    pub patch_states: HashMap<&'static str, PatchState>,
}

/// Everything a refresh reads from disk for one game. Built without the UI
/// (so startup can read it on a background thread) and then handed to
/// [`GameState::take_load`].
struct GameLoad {
    install: Option<Install>,
    art: crate::core::art::Art,
    launcher_file: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    config: ConfigSet,
    exe: Option<ExeInfo>,
    sdk: SdkStatus,
    mods: Vec<ModEntry>,
    backups: Vec<Backup>,
    applied: crate::applied::Applied,
}

impl GameLoad {
    fn read(def: &'static GameDef, settings: &AppSettings) -> Self {
        let install = settings
            .manual_installs
            .get(def.id)
            .filter(|p| p.join(def.exe).is_file())
            .map(|p| Install {
                root: p.clone(),
                store: Store::Manual,
            })
            .or_else(|| detect::detect(&def.detect_spec()));
        let exe_path = install.as_ref().map(|i| i.root.join(def.exe));
        let art = crate::core::art::find(def.id, def.steam_app_ids, exe_path.as_deref());
        let launcher_file = def
            .launcher
            .and_then(|l| install.as_ref().map(|i| i.root.join(l.exe)))
            .filter(|p| p.is_file());
        let config_dir = settings
            .manual_config_dirs
            .get(def.id)
            .cloned()
            .or_else(|| def.default_config_dir());
        let config = match &config_dir {
            Some(dir) => ConfigSet::load(dir, def.ini_files),
            None => ConfigSet::default(),
        };
        let exe = exe_path.filter(|p| p.is_file()).and_then(|path| scan_exe(def, &path));
        let (sdk, mods) = match (def.mods, &install) {
            (Some(support), Some(install)) => (mods::sdk_status(def.id, support, &install.root), mods::scan(support, &install.root)),
            _ => (SdkStatus::NotInstalled, Vec::new()),
        };
        // A record made for another settings folder or install doesn't apply here.
        let applied = crate::applied::load(def.id).fit(config_dir.as_deref(), install.as_ref().map(|i| i.root.as_path()));
        Self { install, art, launcher_file, config_dir, config, exe, sdk, mods, backups: backup::list(def.id), applied }
    }
}

pub struct GameState {
    pub def: &'static GameDef,
    /// Read from disk at least once (startup reads in the background).
    pub loaded: bool,
    pub install: Option<Install>,
    /// Logo, banner and icon found on this PC (Steam cache / exe).
    pub art: crate::core::art::Art,
    pub config_dir: Option<PathBuf>,
    pub config: ConfigSet,
    pub pending: BTreeMap<&'static str, Value>,
    pub exe: Option<ExeInfo>,
    pub sdk: SdkStatus,
    pub mods: Vec<ModEntry>,
    pub backups: Vec<Backup>,
    /// Components ticked on the setup page.
    pub setup_selected: BTreeSet<&'static str>,
    /// Setup statuses touch the disk; cached until the next refresh/write.
    status_cache: std::cell::RefCell<HashMap<&'static str, Status>>,
    /// Last "is the game running" answer and when it was taken.
    running_cache: std::cell::Cell<Option<(std::time::Instant, bool)>>,
    /// The same, for the game's own launcher.
    launcher_cache: std::cell::Cell<Option<(std::time::Instant, bool)>>,
    /// The launcher exe, when the game has one and it's there (found on
    /// refresh, so rendering never touches the disk for it).
    launcher_file: Option<PathBuf>,
    /// Saved profiles, re-read when they change.
    pub profiles: Vec<crate::profiles::Entry>,
    /// What Vaulter last wrote, for "Re-apply" (see `applied.rs`).
    pub applied: crate::applied::Applied,
}

impl GameState {
    fn take_load(&mut self, load: GameLoad) {
        let def = self.def;
        self.loaded = true;
        self.install = load.install;
        self.art = load.art;
        self.launcher_file = load.launcher_file;
        self.config_dir = load.config_dir;
        self.config = load.config;
        self.exe = load.exe;
        self.sdk = load.sdk;
        self.mods = load.mods;
        self.backups = load.backups;
        self.applied = load.applied;
        // Drop pending edits that now match the file.
        let config = &self.config;
        self.pending.retain(|id, v| def.tweak(id).is_some_and(|t| t.read(config).as_ref() != Some(v)));
        self.invalidate_statuses();
    }

    fn new(def: &'static GameDef) -> Self {
        Self {
            def,
            loaded: false,
            install: None,
            art: Default::default(),
            config_dir: None,
            config: ConfigSet::default(),
            pending: BTreeMap::new(),
            exe: None,
            sdk: SdkStatus::NotInstalled,
            mods: Vec::new(),
            backups: Vec::new(),
            setup_selected: def.setup.iter().filter(|c| c.recommended).map(|c| c.id).collect(),
            status_cache: Default::default(),
            running_cache: Default::default(),
            launcher_cache: Default::default(),
            launcher_file: None,
            profiles: crate::profiles::list(def.id),
            applied: crate::applied::load(def.id),
        }
    }

    fn invalidate_statuses(&self) {
        self.status_cache.borrow_mut().clear();
    }

    /// Whether the game was running at the last check, for display. Never
    /// touches the process list: the running watch refreshes it off the UI
    /// thread, so rendering stays cheap. Use [`Self::is_running_now`]
    /// before writing files.
    pub fn is_running(&self) -> bool {
        match self.running_cache.get() {
            Some((_, running)) => running,
            None => self.is_running_now(),
        }
    }

    /// True while the game's exe is running (it rewrites its ini files on
    /// exit, and its exe/DLLs are locked). Takes a process snapshot unless
    /// one is under two seconds old.
    pub fn is_running_now(&self) -> bool {
        if let Some((at, running)) = self.running_cache.get()
            && at.elapsed() < Duration::from_secs(2)
        {
            return running;
        }
        let running = self.exe_path().is_some_and(|p| crate::compare::game_running(&p));
        self.running_cache.set(Some((std::time::Instant::now(), running)));
        running
    }

    /// Stores a fresh "is the game running" answer taken elsewhere (the
    /// running watch polls off the UI thread), so `is_running` agrees.
    fn note_running(&self, running: bool) {
        self.running_cache.set(Some((std::time::Instant::now(), running)));
    }

    /// The game's own launcher program, when it has one and it was there at
    /// the last refresh.
    pub fn launcher_path(&self) -> Option<PathBuf> {
        self.launcher_file.clone()
    }

    /// True while the game's launcher is open: it pushes its own copy of
    /// the video settings when it starts the game, undoing a save made
    /// meanwhile.
    pub fn launcher_running(&self) -> bool {
        if let Some((at, running)) = self.launcher_cache.get()
            && at.elapsed() < Duration::from_secs(2)
        {
            return running;
        }
        let running = self.launcher_path().is_some_and(|p| crate::compare::program_running(&p));
        self.launcher_cache.set(Some((std::time::Instant::now(), running)));
        running
    }

    /// Why the game's settings can't be written right now, if they can't:
    /// the game or its launcher is open and would overwrite them.
    pub fn write_blocker(&self) -> Option<String> {
        if self.is_running_now() {
            Some(format!("{} is running. Close it first, or it will overwrite these settings when it exits", self.def.name))
        } else if self.launcher_running() {
            Some(format!("The {} launcher is open. Close it first, or it will put its own video settings back", self.def.short))
        } else {
            None
        }
    }

    /// Changes this game's re-apply record (see `applied.rs`) and saves it.
    fn update_applied(&mut self, f: impl FnOnce(&mut crate::applied::Applied)) -> Result<()> {
        let root = self.install.as_ref().map(|i| i.root.as_path());
        self.applied = crate::applied::update(self.def.id, self.config_dir.as_deref(), root, f)?;
        Ok(())
    }

    fn reload_profiles(&mut self) {
        self.profiles = crate::profiles::list(self.def.id);
    }

    pub fn config_found(&self) -> bool {
        self.config.files().any(|(_, f)| f.exists)
    }

    /// The value currently written in the game's files.
    pub fn current(&self, tweak: &Tweak) -> Option<Value> {
        tweak.read(&self.config)
    }

    /// The value the UI should show: pending edit, else file, else default.
    pub fn effective(&self, tweak: &Tweak) -> Value {
        self.pending
            .get(tweak.id)
            .cloned()
            .or_else(|| self.current(tweak))
            .unwrap_or_else(|| tweak.default.to_value())
    }

    pub fn exe_path(&self) -> Option<PathBuf> {
        self.install.as_ref().map(|i| i.root.join(self.def.exe))
    }
}

pub struct Workspace {
    pub games: Vec<GameState>,
    pub active: usize,
    pub page: PageKind,
    pub settings: AppSettings,
    pub toasts: Vec<Toast>,
    /// Label of a long-running background job, if any.
    pub busy: Option<String>,
    pub tweak_filter: TweakFilter,
    pub setup_run: Option<SetupRun>,
    /// Setup rows showing their full description.
    pub setup_expanded: BTreeSet<&'static str>,
    /// Comparison lightbox: (tweak id, image index).
    pub preview: Option<Preview>,
    /// A running (or finished) comparison capture.
    pub capture: Option<std::sync::Arc<std::sync::Mutex<crate::compare::CaptureProgress>>>,
    pub capture_save: Option<String>,
    pub capture_settle: u32,
    /// Settings search text (from the toolbar search box).
    pub search: String,
    /// The setting shown in the detail pane.
    pub selected_tweak: Option<&'static str>,
    /// The detail pane's comparison: (tweak, image on the right, divider 0..=1).
    pub inline_compare: Option<(&'static str, usize, f32)>,
    /// Advanced: the "review changes" dialog is open.
    pub review_open: bool,
    pub updates: Updates,
    /// Shared text inputs, created by the window (they need one).
    pub search_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    pub profile_input: Option<gpui::Entity<gpui_component::input::InputState>>,
    undo: Option<Undo>,
    next_toast: u64,
    /// Per game: bumped on every Simple-mode edit; a pending auto-apply only
    /// runs if no newer edit for that game arrived in the meantime.
    autoapply_generation: Vec<u64>,
    /// Game-running watch and background mode (see "game running" below).
    run_watch: RunWatch,
    /// A "Search again" for the games is running.
    pub searching: bool,
    /// The watch's timer loop; dropped (and stopped) with the workspace.
    _run_poll: gpui::Task<()>,
}

impl Workspace {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let settings = AppSettings::load();
        settings.frame_rate.apply();
        let games: Vec<GameState> = games::all().iter().map(|d| GameState::new(d)).collect();
        let active = settings
            .active_game
            .as_ref()
            .and_then(|id| games.iter().position(|g| g.def.id == id))
            .unwrap_or(0);
        let page = match settings.mode {
            Mode::Simple => PageKind::Setup,
            Mode::Advanced => PageKind::Overview,
        };
        let mut ws = Self {
            games,
            active,
            page,
            settings,
            toasts: Vec::new(),
            busy: None,
            tweak_filter: TweakFilter::All,
            setup_run: None,
            setup_expanded: BTreeSet::new(),
            preview: None,
            capture: None,
            capture_save: None,
            capture_settle: 25,
            search: String::new(),
            selected_tweak: None,
            inline_compare: None,
            review_open: false,
            updates: Updates::default(),
            search_input: None,
            profile_input: None,
            undo: None,
            next_toast: 0,
            autoapply_generation: vec![0; games::all().len()],
            run_watch: RunWatch::default(),
            searching: false,
            _run_poll: Self::spawn_run_poll(cx),
        };
        // Before reading anything: a capture cut short last time left its shot settings behind.
        ws.recover_captures(cx);
        ws.load_games(cx);
        ws.fetch_comparison_packs(cx);
        ws.check_updates(cx);
        let probe = cx.background_executor().spawn(async { crate::core::gpu::dxvk_ready().is_ok() });
        cx.spawn(async move |this, cx| {
            let ready = probe.await;
            this.update(cx, |ws, cx| {
                for game in &mut ws.games {
                    if !ready {
                        game.setup_selected.remove("dxvk");
                    }
                    game.invalidate_statuses();
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
        ws
    }

    /// Installs comparison images a game is missing: bundled sets are copied
    /// out of the exe, anything else falls back to the published zip. Then
    /// makes the small preview copies. Silent if there's nothing to add.
    fn fetch_comparison_packs(&mut self, cx: &mut Context<Self>) {
        let all: Vec<&'static str> = self.games.iter().filter(|g| !g.def.comparisons.is_empty()).map(|g| g.def.id).collect();
        let task = cx.background_executor().spawn(async move {
            let mut added = 0;
            for id in &all {
                added += crate::compare::extract_pack(id);
                // Games whose set isn't bundled still try the download.
                if !crate::compare::has_local_images(id) {
                    added += crate::compare::download_pack(id).unwrap_or(0);
                }
            }
            // Small copies for the settings pages (only missing ones are made).
            added += all.iter().map(|id| crate::compare::make_previews(id)).sum::<usize>();
            added
        });
        cx.spawn(async move |this, cx| {
            if task.await > 0 {
                this.update(cx, |_, cx| cx.notify()).ok();
            }
        })
        .detach();
    }

    pub fn game(&self) -> &GameState {
        &self.games[self.active]
    }

    pub fn game_mut(&mut self) -> &mut GameState {
        &mut self.games[self.active]
    }

    pub fn select_game(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.games.len() {
            if self.active != index {
                crate::sound::play(crate::sound::Sound::Whoosh);
                // The running watch follows the newly selected game.
                if self.run_watch.reset(self.games[index].is_running_now()) {
                    self.apply_run_audio();
                }
            }
            self.active = index;
            if !self.games[index].def.nav_items(self.settings.mode).any(|n| n.kind == self.page) {
                self.page = self.home_page();
            }
            self.settings.active_game = Some(self.games[index].def.id.to_string());
            self.settings.save();
            cx.notify();
        }
    }

    pub fn navigate(&mut self, page: PageKind, cx: &mut Context<Self>) {
        // Pages that only exist in the other mode (e.g. One-Click Setup from
        // Advanced) switch modes, unless that would strand waiting changes.
        let def = self.game().def;
        let other = match self.settings.mode {
            Mode::Simple => Mode::Advanced,
            Mode::Advanced => Mode::Simple,
        };
        if page != PageKind::Capture && !def.nav_items(self.settings.mode).any(|n| n.kind == page) && def.nav_items(other).any(|n| n.kind == page) {
            if !self.game().pending.is_empty() {
                self.toast(ToastKind::Info, "Apply or discard the waiting changes first", cx);
                return;
            }
            self.settings.mode = other;
            self.settings.save();
        }
        if self.page != page {
            crate::sound::play(crate::sound::Sound::Whoosh);
        }
        self.page = page;
        cx.notify();
    }

    pub fn set_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
        crate::sound::set_muted(muted);
        crate::sound::set_music(self.settings.music && !muted);
        self.settings.sound_muted = muted;
        self.settings.save();
        cx.notify();
    }

    pub fn set_music(&mut self, on: bool, cx: &mut Context<Self>) {
        crate::sound::set_music(on && !self.settings.sound_muted);
        self.settings.music = on;
        self.settings.save();
        cx.notify();
    }

    pub fn mode(&self) -> Mode {
        self.settings.mode
    }

    fn home_page(&self) -> PageKind {
        match self.settings.mode {
            Mode::Simple => PageKind::Setup,
            Mode::Advanced => PageKind::Overview,
        }
    }


    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.settings.mode == mode {
            return;
        }
        self.settings.mode = mode;
        self.settings.save();
        crate::sound::play(crate::sound::Sound::Whoosh);
        if !self.game().def.nav_items(mode).any(|n| n.kind == self.page) {
            self.page = self.home_page();
        }
        cx.notify();
    }

    // ---- detection & loading -------------------------------------------------

    /// Reads every game from disk on a background thread, so the window
    /// shows up before detection, configs, mods and backups are read (about
    /// 130 ms warm, far more on a cold disk). Pages show a ProgressRing
    /// until [`GameState::loaded`].
    fn load_games(&mut self, cx: &mut Context<Self>) {
        let defs: Vec<&'static GameDef> = self.games.iter().map(|g| g.def).collect();
        let settings = self.settings.clone();
        let task = cx.background_executor().spawn(async move {
            defs.into_iter().map(|def| GameLoad::read(def, &settings)).collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let loads = task.await;
            this.update(cx, |ws, cx| {
                // A game refreshed meanwhile (a folder picked) is newer.
                // (Startup only: [`Self::search_again`] takes everything.)
                for (game, load) in ws.games.iter_mut().zip(loads).filter(|(g, _)| !g.loaded) {
                    game.take_load(load);
                }
                ws.run_watch.reset(ws.game().is_running_now());
                // UI sounds come from the first installed Willow game's launcher.
                let audio_dir = ws
                    .games
                    .iter()
                    .filter_map(|g| g.install.as_ref())
                    .map(|i| i.root.join(crate::sound::AUDIO_DIR))
                    .find(|d| d.join("ButtonClick.mp3").is_file());
                crate::sound::init(audio_dir.as_deref(), ws.settings.sound_muted, ws.settings.music);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Looks for every game again (after installing one, moving it or
    /// plugging in a drive), off the UI thread, then says what it found.
    pub fn search_again(&mut self, cx: &mut Context<Self>) {
        if self.searching {
            return;
        }
        self.searching = true;
        cx.notify();
        let defs: Vec<&'static GameDef> = self.games.iter().map(|g| g.def).collect();
        let settings = self.settings.clone();
        let task = cx.background_executor().spawn(async move { defs.into_iter().map(|def| GameLoad::read(def, &settings)).collect::<Vec<_>>() });
        cx.spawn(async move |this, cx| {
            let loads = task.await;
            this.update(cx, |ws, cx| {
                ws.searching = false;
                for (game, load) in ws.games.iter_mut().zip(loads) {
                    game.take_load(load);
                }
                let found: Vec<&str> = ws.games.iter().filter(|g| g.install.is_some()).map(|g| g.def.short).collect();
                let msg = match found.len() {
                    0 => "No games found. Pick the install folder in App settings".to_string(),
                    n if n == ws.games.len() => format!("Found all {n} games"),
                    _ => format!("Found {}", found.join(", ")),
                };
                ws.toast(if found.is_empty() { ToastKind::Info } else { ToastKind::Success }, msg, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn refresh_game(&mut self, index: usize) {
        let load = GameLoad::read(self.games[index].def, &self.settings);
        self.games[index].take_load(load);
    }

    pub fn refresh_active(&mut self, cx: &mut Context<Self>) {
        self.refresh_game(self.active);
        cx.notify();
    }

    pub fn set_manual_install(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let def = self.game().def;
        match detect::validate_manual(&path, def.exe) {
            Some(root) => {
                self.settings.manual_installs.insert(def.id.to_string(), root);
                self.settings.save();
                self.refresh_active(cx);
                self.toast(ToastKind::Success, format!("{} install set", def.short), cx);
            }
            None => self.toast(
                ToastKind::Error,
                format!("Couldn't find {} under that folder", def.exe),
                cx,
            ),
        }
    }

    pub fn set_manual_config_dir(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let def = self.game().def;
        self.settings.manual_config_dirs.insert(def.id.to_string(), path);
        self.settings.save();
        self.refresh_active(cx);
        self.toast(ToastKind::Success, "Config folder set", cx);
    }

    pub fn clear_manual_paths(&mut self, cx: &mut Context<Self>) {
        let id = self.game().def.id;
        self.settings.manual_installs.remove(id);
        self.settings.manual_config_dirs.remove(id);
        self.settings.save();
        self.refresh_active(cx);
        self.toast(ToastKind::Info, "Back to auto-detected paths", cx);
    }

    // ---- tweaks ----------------------------------------------------------------

    pub fn pending_count(&self) -> usize {
        self.game().pending.len()
    }

    pub fn stage(&mut self, tweak: &'static Tweak, value: Value, cx: &mut Context<Self>) {
        let value = tweak.clamp(value);
        let game = self.game_mut();
        let on_disk = game.current(tweak).unwrap_or_else(|| tweak.default.to_value());
        if value == on_disk {
            game.pending.remove(tweak.id);
            // Nothing to write, but picking what's on disk over the value
            // Vaulter remembered accepts it: no more drift warning,
            // and Re-apply won't put the old value back.
            if game.current(tweak).as_ref() == Some(&value)
                && game.applied.differs(tweak, &value)
                && let Err(e) = game.update_applied(|a| a.merge(&[(tweak.id, Some(value))]))
            {
                self.toast(ToastKind::Error, format!("{e:#}"), cx);
            }
        } else {
            game.pending.insert(tweak.id, value);
        }
        cx.notify();
    }

    /// Loads a preset into the pending changes, replacing what was there.
    pub fn stage_preset(&mut self, preset: &Preset, cx: &mut Context<Self>) {
        self.game_mut().pending.clear();
        let def = self.game().def;
        if preset.values.is_empty() {
            // An empty preset means "factory settings" for everything except
            // the monitor-specific display choices.
            for tweak in factory_tweaks(def) {
                self.stage(tweak, tweak.default.to_value(), cx);
            }
        }
        for (id, value) in preset.values {
            if let Some(tweak) = def.tweak(id) {
                self.stage(tweak, value.to_value(), cx);
            }
        }
        let count = self.game().pending.len();
        self.toast(
            ToastKind::Info,
            format!("{} loaded: {count} change(s) waiting. Press Apply to write them.", preset.name),
            cx,
        );
    }

    pub fn stage_defaults(&mut self, categories: &[&str], cx: &mut Context<Self>) {
        let def = self.game().def;
        for tweak in def.visible_tweaks().filter(|t| categories.contains(&t.category)) {
            self.stage(tweak, tweak.default.to_value(), cx);
        }
    }

    pub fn discard_pending(&mut self, cx: &mut Context<Self>) {
        self.game_mut().pending.clear();
        cx.notify();
    }

    pub fn apply_pending(&mut self, cx: &mut Context<Self>) {
        match self.try_apply_pending(self.active, false) {
            Ok(n) => {
                self.review_open = false;
                self.toast_with(ToastKind::Success, format!("Applied {n} change(s)"), Some(ToastAction::Undo), cx);
            }
            Err(e) => self.toast(ToastKind::Error, format!("Apply failed: {e:#}"), cx),
        }
    }

    fn try_apply_pending(&mut self, gi: usize, coalesce: bool) -> Result<usize> {
        let pending = std::mem::take(&mut self.games[gi].pending);
        let def = self.games[gi].def;
        // What's on disk now, so the save can be undone.
        let previous: Vec<(&'static str, Value)> = pending
            .keys()
            .filter_map(|id| def.tweak(id))
            .map(|t| (t.id, self.games[gi].current(t).unwrap_or_else(|| t.default.to_value())))
            .collect();
        let label = if coalesce {
            QUICK_LABEL.to_string()
        } else {
            format!("Before applying {} change(s)", pending.len())
        };
        let result = self.write_configs(gi, &label, coalesce, |config| {
            for (id, value) in &pending {
                if let Some(tweak) = def.tweak(id) {
                    tweak.write(config, value);
                }
            }
        });
        match result {
            Ok(_) => {
                // A burst of Quick Settings saves undoes back to before the burst.
                match &mut self.undo {
                    Some(undo) if coalesce && undo.game == gi => {
                        for (id, v) in previous {
                            if !undo.values.iter().any(|(i, _)| *i == id) {
                                undo.values.push((id, v));
                            }
                        }
                    }
                    _ => self.undo = Some(Undo { game: gi, values: previous }),
                }
                Ok(pending.len())
            }
            Err(e) => {
                self.games[gi].pending = pending;
                Err(e)
            }
        }
    }

    /// Writes back the values the last save replaced.
    pub fn undo_last(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self.undo.take() else {
            return;
        };
        let def = self.games[undo.game].def;
        let result = self.write_configs(undo.game, "Before undo", false, |config| {
            for (id, value) in &undo.values {
                if let Some(tweak) = def.tweak(id) {
                    tweak.write(config, value);
                }
            }
        });
        match result {
            Ok(_) => self.toast(ToastKind::Info, format!("Undone: {} setting(s) put back", undo.values.len()), cx),
            Err(e) => self.toast(ToastKind::Error, format!("Couldn't undo: {e:#}"), cx),
        }
    }

    /// Drops one waiting change (from the review dialog).
    pub fn unstage(&mut self, id: &str, cx: &mut Context<Self>) {
        self.game_mut().pending.remove(id);
        if self.game().pending.is_empty() {
            self.review_open = false;
        }
        cx.notify();
    }

    pub fn set_review(&mut self, open: bool, cx: &mut Context<Self>) {
        self.review_open = open && !self.game().pending.is_empty();
        cx.notify();
    }

    pub fn set_search(&mut self, text: String, cx: &mut Context<Self>) {
        if self.search != text {
            self.search = text;
            cx.notify();
        }
    }

    /// Updates the detail pane's comparison for `tweak`.
    pub fn set_inline(&mut self, tweak: &'static str, right: Option<usize>, split: Option<f32>, cx: &mut Context<Self>) {
        let (_, r, s) = self.inline_compare.filter(|c| c.0 == tweak).unwrap_or((tweak, usize::MAX, 0.5));
        self.inline_compare = Some((tweak, right.unwrap_or(r), split.unwrap_or(s).clamp(0., 1.)));
        cx.notify();
    }

    pub fn select_tweak(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if self.selected_tweak != Some(id) {
            self.selected_tweak = Some(id);
            cx.notify();
        }
    }

    // ---- profiles ------------------------------------------------------------

    /// Saves every setting's current value (including waiting changes) as a
    /// named profile.
    pub fn save_profile(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let name = name.trim();
        if name.is_empty() {
            self.toast(ToastKind::Error, "Give the profile a name first", cx);
            return false;
        }
        let game = self.game();
        let profile = crate::profiles::snapshot(game.def, name, |t| game.effective(t));
        // Saving under an existing name updates that profile, and says so.
        let replaces = crate::profiles::find(game.def.id, name).is_some();
        let path = crate::profiles::path_for(game.def.id, name);
        match crate::profiles::write(&profile, &path) {
            Ok(()) => {
                self.game_mut().reload_profiles();
                let verb = if replaces { "Updated" } else { "Saved" };
                self.toast(ToastKind::Success, format!("{verb} profile \u{201c}{name}\u{201d}"), cx);
                true
            }
            Err(e) => {
                self.toast(ToastKind::Error, format!("Couldn't save the profile: {e:#}"), cx);
                false
            }
        }
    }

    /// Loads a profile: staged in Advanced mode, saved at once in Simple.
    pub fn load_profile(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let def = self.game().def;
        let loaded = crate::profiles::read(path).and_then(|p| crate::profiles::resolve(def, &p).map(|r| (p.name, r)));
        let (name, (values, skipped)) = match loaded {
            Ok(v) => v,
            Err(e) => {
                self.toast(ToastKind::Error, format!("{e:#}"), cx);
                return;
            }
        };
        self.game_mut().pending.clear();
        for (tweak, value) in values {
            self.stage(tweak, value, cx);
        }
        let changes = self.game().pending.len();
        let note = if skipped > 0 { format!(" ({skipped} unknown setting(s) skipped)") } else { String::new() };
        if self.mode() == Mode::Simple {
            self.schedule_autoapply(cx);
            self.toast(ToastKind::Info, format!("Loading {name}: {changes} change(s){note}"), cx);
        } else {
            self.toast(ToastKind::Info, format!("{name} loaded: {changes} change(s) waiting{note}"), cx);
        }
    }

    /// Copies a profile file into this game's profile folder.
    pub fn import_profile(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let def = self.game().def;
        let result = crate::profiles::read(path).and_then(|mut p| {
            crate::profiles::resolve(def, &p)?;
            // Never over a profile already saved: a clashing name becomes "Name (2)".
            p.name = crate::profiles::unused_name(def.id, &p.name);
            crate::profiles::write(&p, &crate::profiles::path_for(def.id, &p.name))?;
            Ok(p.name)
        });
        self.game_mut().reload_profiles();
        match result {
            Ok(name) => self.toast(ToastKind::Success, format!("Imported {name}"), cx),
            Err(e) => self.toast(ToastKind::Error, format!("Import failed: {e:#}"), cx),
        }
    }

    pub fn export_profile(&mut self, from: &std::path::Path, to: &std::path::Path, cx: &mut Context<Self>) {
        match std::fs::copy(from, to) {
            Ok(_) => self.toast(ToastKind::Success, format!("Exported to {}", to.display()), cx),
            Err(e) => self.toast(ToastKind::Error, format!("Export failed: {e}"), cx),
        }
    }

    pub fn delete_profile(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        if let Err(e) = std::fs::remove_file(path) {
            self.toast(ToastKind::Error, format!("{e}"), cx);
        }
        self.game_mut().reload_profiles();
        cx.notify();
    }

    // ---- updates -------------------------------------------------------------

    /// One background check per launch: a newer Vaulter, and the latest
    /// mod SDK. Silent without a network.
    fn check_updates(&mut self, cx: &mut Context<Self>) {
        let mut repos: Vec<&'static str> = self.games.iter().filter_map(|g| g.def.mods).map(|m| m.sdk_repo).collect();
        repos.sort_unstable();
        repos.dedup();
        let task = cx.background_executor().spawn(async move {
            // Only version tags count: the same repo publishes image packs.
            let app = crate::core::net::latest_app_release(crate::compare::IMAGE_REPO)
                .ok()
                .filter(|(tag, _)| crate::core::net::is_newer(tag, env!("CARGO_PKG_VERSION")));
            let sdk = repos
                .into_iter()
                .filter_map(|r| crate::core::net::latest_release(r).ok().map(|(tag, _)| (r, tag)))
                .collect();
            Updates { app, sdk }
        });
        cx.spawn(async move |this, cx| {
            let updates = task.await;
            this.update(cx, |ws, cx| {
                ws.updates = updates;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The newer SDK release, when the installed one is outdated.
    /// An SDK installed from a zip has no known version, so it's never
    /// reported as outdated.
    pub fn sdk_update_available(&self) -> Option<&str> {
        match (&self.game().sdk, self.latest_sdk()) {
            (SdkStatus::Installed(v), Some(latest)) if v != latest && v != "manual" => Some(latest),
            _ => None,
        }
    }

    /// The latest release of the active game's own mod SDK, if checked.
    pub fn latest_sdk(&self) -> Option<&str> {
        self.game().def.mods.and_then(|m| self.updates.sdk.get(m.sdk_repo)).map(String::as_str)
    }

    /// Edits game `gi`'s configs with `edit` and writes them, after:
    /// refusing while the game runs (it rewrites them on exit), re-reading
    /// the files from disk (so changes made in the game's own menus aren't
    /// reverted), keeping a permanent "original settings" snapshot, and
    /// snapshotting the files about to change. With `coalesce`, one snapshot
    /// covers a burst of Quick Settings edits instead of one per edit.
    /// Returns how many settings the write changed.
    fn write_configs(&mut self, gi: usize, label: &str, coalesce: bool, edit: impl FnOnce(&mut ConfigSet)) -> Result<usize> {
        let lock = self.settings.lock_configs_after_apply;
        let max_backups = self.settings.max_backups;
        let game = &mut self.games[gi];
        let def = game.def;
        let Some(dir) = game.config_dir.clone().filter(|_| game.config_found()) else {
            return Err(anyhow!(
                "config files not found: launch the game once or set the folder in App Settings"
            ));
        };
        if let Some(reason) = game.write_blocker() {
            return Err(anyhow!(reason));
        }
        let mut config = ConfigSet::load(&dir, def.ini_files);
        let existing: Vec<PathBuf> = config.files().filter(|(_, f)| f.exists).map(|(_, f)| f.path.clone()).collect();
        if !game.backups.iter().any(backup::is_original) {
            backup::create(def.id, backup::ORIGINAL_LABEL, &existing).context("saving your original settings")?;
        }
        // What every tweak reads before the edit, so afterwards we know
        // which values this write actually changed, those are remembered
        // for "Re-apply".
        let before: Vec<Option<Value>> = def.visible_tweaks().map(|t| t.read(&config)).collect();
        let changes = |config: &ConfigSet| -> Vec<(&'static str, Option<Value>)> {
            def.visible_tweaks()
                .zip(before.iter())
                .filter_map(|(t, was)| {
                    let now = t.read(config);
                    (now != *was).then_some((t.id, now))
                })
                .collect()
        };
        edit(&mut config);
        let dirty = config.dirty_paths();
        if dirty.is_empty() {
            let changed = changes(&config);
            game.config = config;
            game.update_applied(|a| a.merge(&changed))?;
            return Ok(changed.len());
        }
        match snapshot_plan(game.backups.first().filter(|_| coalesce), label, &dirty, chrono::Local::now().naive_local()) {
            SnapshotPlan::New => {
                backup::create(def.id, label, &dirty).context("creating backup")?;
            }
            SnapshotPlan::Extend(missing) => {
                let mut recent = game.backups[0].clone();
                backup::add_files(&mut recent, &missing).context("creating backup")?;
            }
            SnapshotPlan::Covered => {}
        }
        let written = config.save_dirty();
        game.config = ConfigSet::load(&dir, def.ini_files);
        game.invalidate_statuses();
        game.backups = backup::list(def.id);
        // Remember what actually landed on disk, even when only some of
        // the files could be written.
        let changed = changes(&game.config);
        let recorded = game.update_applied(|a| a.merge(&changed));
        let written = written?;
        recorded?;
        if lock {
            for path in &written {
                backup::set_readonly(path, true)?;
            }
        }
        prune_backups(def.id, max_backups);
        game.backups = backup::list(def.id);
        Ok(changed.len())
    }

    /// Simple mode: stage a change and write it shortly after the user stops
    /// adjusting, so a slider drag doesn't produce a snapshot per pixel.
    pub fn set_now(&mut self, tweak: &'static Tweak, value: Value, cx: &mut Context<Self>) {
        self.stage(tweak, value, cx);
        self.schedule_autoapply(cx);
    }

    pub fn preset_now(&mut self, preset: &Preset, cx: &mut Context<Self>) {
        let def = self.game().def;
        for (id, value) in preset.values {
            if let Some(tweak) = def.tweak(id) {
                self.stage(tweak, value.to_value(), cx);
            }
        }
        self.schedule_autoapply(cx);
    }

    fn schedule_autoapply(&mut self, cx: &mut Context<Self>) {
        let gi = self.active;
        self.autoapply_generation[gi] += 1;
        let generation = self.autoapply_generation[gi];
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(700)).await;
            this.update(cx, |ws, cx| {
                // Only this game's edits, and only if no newer edit came in.
                if ws.autoapply_generation[gi] != generation || ws.games[gi].pending.is_empty() {
                    return;
                }
                match ws.try_apply_pending(gi, true) {
                    Ok(_) => {
                        ws.toast_with(ToastKind::Success, "Saved", Some(ToastAction::Undo), cx);
                    }
                    Err(e) => ws.toast(ToastKind::Error, format!("Couldn't save: {e:#}"), cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ---- one-click setup ---------------------------------------------------------

    pub fn component_status(&self, component: &setup::Component) -> Status {
        self.component_status_for(self.active, component)
    }

    pub(crate) fn component_status_for(&self, gi: usize, component: &setup::Component) -> Status {
        let game = &self.games[gi];
        if let Some(s) = game.status_cache.borrow().get(component.id) {
            return s.clone();
        }
        let status = setup::status(component, game, &self.launch_args_for(gi));
        game.status_cache.borrow_mut().insert(component.id, status.clone());
        status
    }

    pub fn toggle_component(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let def = self.game().def;
        let selected = &mut self.game_mut().setup_selected;
        if !selected.remove(id) {
            selected.insert(id);
            // Picking one of two tools that need the same file (Luma and
            // DXVK both use dxgi.dll) unpicks the other.
            if let Some(picked) = def.component(id) {
                selected.retain(|other| def.component(other).is_none_or(|o| !picked.clashes_with(o)));
            }
        }
        cx.notify();
    }

    pub fn toggle_expanded(&mut self, id: &'static str, cx: &mut Context<Self>) {
        if !self.setup_expanded.remove(id) {
            self.setup_expanded.insert(id);
        }
        cx.notify();
    }

    /// Opens the viewer with the clicked option on the right and, on the
    /// left, the game's default (or the first other option).
    pub fn open_preview(&mut self, tweak: &'static str, index: usize, cx: &mut Context<Self>) {
        let def = self.game().def;
        let left = def
            .tweak(tweak)
            .and_then(|t| {
                let default = t.default.to_value();
                crate::compare::images(def.id, t)
                    .iter()
                    .zip(crate::compare::captured_values(def.id, t))
                    .position(|(_, v)| v == default)
            })
            .filter(|&d| d != index)
            .unwrap_or(if index == 0 { 1 } else { 0 });
        self.preview = Some(Preview { tweak, left, right: index, split: 0.5 });
        cx.notify();
    }

    pub fn set_preview(&mut self, f: impl FnOnce(&mut Preview), cx: &mut Context<Self>) {
        if let Some(p) = &mut self.preview {
            f(p);
            cx.notify();
        }
    }

    pub fn close_preview(&mut self, cx: &mut Context<Self>) {
        self.preview = None;
        cx.notify();
        // The viewer's full-size images are freed when gpui drops its element
        // state, a frame or two after it closes; make sure those frames come
        // even if nothing else redraws.
        cx.spawn(async move |this, cx| {
            for _ in 0..2 {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    /// Save files the capture tool can load into, newest first.
    pub fn capture_saves(&self) -> Vec<String> {
        let game = self.game();
        match (game.config_dir.as_ref(), game.def.capture) {
            (Some(dir), Some(profile)) => crate::compare::find_saves(dir, profile),
            _ => Vec::new(),
        }
    }

    pub fn capture_running(&self) -> bool {
        self.capture
            .as_ref()
            .is_some_and(|p| p.lock().is_ok_and(|p| !p.finished))
    }

    pub fn start_capture(&mut self, cx: &mut Context<Self>) {
        use crate::compare::{self, CaptureProgress, CaptureRequest};
        if self.capture_running() || self.refused(self.job_blocker(), cx) {
            return;
        }
        let game = self.game();
        let def = game.def;
        let (Some(install), Some(config_dir)) = (game.install.clone(), game.config_dir.clone()) else {
            self.toast(ToastKind::Error, "Game install or config folder not found", cx);
            return;
        };
        let Some(profile) = def.capture else {
            self.toast(ToastKind::Error, "Comparison capture isn't available for this game", cx);
            return;
        };
        let prerequisite_missing = profile
            .prerequisite
            .and_then(|id| def.component(id))
            .is_some_and(|c| !self.component_status(c).is_active());
        if !matches!(game.sdk, SdkStatus::Installed(_) | SdkStatus::Detected) || prerequisite_missing {
            self.toast(ToastKind::Error, "Capture needs the Python SDK (and Quick Startup for BL2/TPS). Install them from One-Click Setup first", cx);
            return;
        }
        let Some(save) = self.capture_save.clone().or_else(|| self.capture_saves().into_iter().next()) else {
            self.toast(ToastKind::Error, "No save files found to load into", cx);
            return;
        };
        let shots: Vec<_> = def
            .comparisons
            .iter()
            .filter(|c| c.capture)
            .filter_map(|c| def.tweak(c.tweak))
            .flat_map(|t| compare::capture_values(t).into_iter().map(move |(v, l)| (t, v, l)))
            .collect();
        let progress = std::sync::Arc::new(std::sync::Mutex::new(CaptureProgress {
            total: shots.len(),
            ..Default::default()
        }));
        let request = CaptureRequest {
            game_id: def.id,
            exe: install.root.join(def.exe),
            root: install.root,
            config_dir,
            ini_files: def.ini_files,
            save,
            settle_seconds: self.capture_settle,
            shots,
            profile,
        };
        self.capture = Some(progress.clone());
        cx.notify();
        let task = cx.background_executor().spawn({
            let progress = progress.clone();
            async move { compare::run(request, progress) }
        });
        // Repaint periodically so progress shows while the game runs.
        let tick_progress = progress.clone();
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(700)).await;
            let finished = tick_progress.lock().map(|p| p.finished).unwrap_or(true);
            if this.update(cx, |_, cx| cx.notify()).is_err() || finished {
                break;
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |ws, cx| {
                let id = ws.game().def.id;
                ws.game_mut().backups = backup::list(id);
                match result {
                    Ok(n) => ws.toast(ToastKind::Success, format!("Captured {n} image(s). Your settings were restored."), cx),
                    Err(e) => ws.toast(ToastKind::Error, format!("Capture stopped: {e:#}"), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn cancel_capture(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = &self.capture
            && let Ok(mut p) = p.lock() {
                p.cancel = true;
            }
        cx.notify();
    }

    pub fn setup_running(&self) -> bool {
        self.setup_run.as_ref().is_some_and(|r| !r.finished)
    }

    /// Why a job that changes game files can't start now: a setup run, a
    /// Mods page job or a capture is already going (they'd share files and
    /// downloads).
    fn job_blocker(&self) -> Option<&'static str> {
        if self.capture_running() {
            Some("A comparison capture is running. Wait for it to finish")
        } else if self.setup_running() || self.busy.is_some() {
            Some("Still working. Wait for it to finish")
        } else {
            None
        }
    }

    /// `job_blocker`, and also the game running (its files are in use, and
    /// it would half-load a changing mod setup).
    fn mods_blocker(&self) -> Option<String> {
        self.job_blocker().map(str::to_string).or_else(|| {
            self.game().is_running_now().then(|| format!("{} is running. Close it first", self.game().def.name))
        })
    }

    /// Toasts `reason` and returns true when there is one.
    fn refused(&mut self, reason: Option<impl Into<String>>, cx: &mut Context<Self>) -> bool {
        let Some(reason) = reason else { return false };
        self.toast(ToastKind::Info, reason, cx);
        true
    }

    /// Asked when the window is about to close: refuses while a capture or
    /// an install is half-way, since quitting then would leave the game's
    /// settings or files half-changed.
    pub fn allow_close(&mut self, cx: &mut Context<Self>) -> bool {
        let reason = if self.capture_running() {
            "A comparison capture is running. Press Stop and wait for your settings to be put back, then close"
        } else if self.setup_running() || self.busy.is_some() {
            "Still working. Wait for it to finish, then close"
        } else {
            return true;
        };
        self.toast(ToastKind::Info, reason, cx);
        false
    }

    /// Undoes comparison captures the app didn't get to finish last time.
    fn recover_captures(&mut self, cx: &mut Context<Self>) {
        for outcome in crate::compare::recover_interrupted() {
            match outcome {
                Ok(msg) => self.toast(ToastKind::Info, msg, cx),
                Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
            }
        }
    }

    /// Installs the selected components that aren't active yet (in list
    /// order); with `Uninstall` removes every active component, and with
    /// `Reapply` puts back what was installed and rewrites remembered
    /// settings and exe patches.
    pub fn run_setup(&mut self, goal: SetupGoal, cx: &mut Context<Self>) {
        if self.setup_running() {
            return;
        }
        // A Mods page install or a capture would share its files.
        if self.refused(self.job_blocker(), cx) {
            return;
        }
        let game = self.game();
        let def = game.def;
        let launch_args = self.launch_args();
        let wanted = |c: &setup::Component| {
            let status = self.component_status(c);
            let ours = setup::installed_by_us(c, game, &launch_args);
            match goal {
                // Only undo what Vaulter itself installed or applied
                // (an exe patch only while it's actually on).
                SetupGoal::Uninstall => ours && (!matches!(c.kind, ComponentKind::ExePatch(_)) || status.is_active()),
                SetupGoal::Install => {
                    game.setup_selected.contains(c.id) && !status.is_active() && !matches!(status, Status::Blocked(_))
                }
                SetupGoal::Reapply => reapply_wants(&c.kind, &status, ours),
            }
        };
        let active = |id: &str| def.component(id).is_some_and(|d| self.component_status(d).is_active());
        let mut ids: Vec<&'static str> = def.setup.iter().filter(|c| wanted(c)).map(|c| c.id).collect();
        match goal {
            SetupGoal::Install => {
                // Pull in dependencies that aren't installed yet (e.g. the SDK
                // for SDK mods), keeping list order.
                let needed: Vec<&'static str> = ids
                    .iter()
                    .filter_map(|id| def.component(id))
                    .flat_map(|c| c.requires.iter().copied())
                    .filter(|dep| !ids.contains(dep) && !active(dep))
                    .collect();
                if !needed.is_empty() {
                    ids = def
                        .setup
                        .iter()
                        .map(|c| c.id)
                        .filter(|id| ids.contains(id) || needed.contains(id))
                        .collect();
                }
            }
            // "Put back" never installs anything new: a component whose
            // dependency the user removed is left alone.
            SetupGoal::Reapply => {
                let keep: Vec<&'static str> = ids
                    .iter()
                    .copied()
                    .filter(|id| def.component(id).is_none_or(|c| c.requires.iter().all(|dep| ids.contains(dep) || active(dep))))
                    .collect();
                ids = keep;
            }
            // Remove dependents (SDK mods) before what they depend on.
            SetupGoal::Uninstall => ids.reverse(),
        }
        // Re-apply ends by writing back the remembered settings and exe
        // patches; each gets a line in the step list.
        let mut steps: Vec<&'static str> = ids.clone();
        if goal == SetupGoal::Reapply {
            if !game.applied.values.is_empty() {
                steps.push(SETTINGS_STEP);
            }
            if !game.applied.patches.is_empty() {
                steps.push(PATCHES_STEP);
            }
        }
        if steps.is_empty() {
            self.toast(
                ToastKind::Info,
                match goal {
                    SetupGoal::Uninstall => "Nothing to restore: no Vaulter components are active.",
                    SetupGoal::Install => "Everything selected is already installed.",
                    SetupGoal::Reapply => {
                        "Nothing to re-apply: no settings or upgrades have been applied yet."
                    }
                },
                cx,
            );
            return;
        }
        let uninstall = goal == SetupGoal::Uninstall;
        self.setup_run = Some(SetupRun {
            goal,
            steps: steps.iter().map(|id| (*id, StepState::Queued)).collect(),
            finished: false,
        });
        cx.notify();

        let game_index = self.active;
        cx.spawn(async move |this, cx| {
            for (step, id) in ids.iter().enumerate() {
                // Local work runs right here; downloads come back as a job for
                // the background thread so the UI stays responsive.
                let job = match this.update(cx, |ws, cx| {
                    let job = ws.start_component(game_index, id, uninstall);
                    if job.is_some() {
                        ws.set_step(step, StepState::Running, cx);
                    }
                    job
                }) {
                    Ok(job) => job,
                    Err(_) => return,
                };
                let Some(job) = job else { continue };
                let outcome = match job {
                    Ok(Job::Done(msg)) => Ok(msg),
                    Ok(Job::Background(work)) => cx.background_executor().spawn(async move { work() }).await,
                    Err(e) => Err(e),
                };
                let failed = outcome.is_err();
                this.update(cx, |ws, cx| {
                    ws.set_step(
                        step,
                        match outcome {
                            Ok(msg) => StepState::Done(msg),
                            Err(e) => StepState::Failed(format!("{e:#}")),
                        },
                        cx,
                    );
                    ws.refresh_game(game_index);
                    // Don't install mods on top of a failed SDK, etc.
                    if failed && !uninstall {
                        ws.skip_dependents(game_index, id, cx);
                    }
                })
                .ok();
            }
            this.update(cx, |ws, cx| {
                // With the components back in place, re-write the remembered
                // settings and re-patch the exe.
                let (settings, patched) = ws.rewrite_remembered(game_index, cx);
                let failed = ws.setup_run.as_ref().map_or(0, |r| {
                    r.steps.iter().filter(|(_, s)| matches!(s, StepState::Failed(_))).count()
                });
                let done = ws.setup_run.as_ref().map_or(0, |r| {
                    r.steps
                        .iter()
                        .filter(|(id, s)| matches!(s, StepState::Done(_)) && ![SETTINGS_STEP, PATCHES_STEP].contains(id))
                        .count()
                });
                if let Some(run) = &mut ws.setup_run {
                    run.finished = true;
                }
                ws.refresh_game(game_index);
                match (failed, goal) {
                    (0, SetupGoal::Install) => ws.toast(ToastKind::Success, "All set. Enjoy the upgraded game!", cx),
                    (0, SetupGoal::Uninstall) => ws.toast(ToastKind::Success, "Restored to vanilla", cx),
                    (0, SetupGoal::Reapply) => {
                        let mut parts = Vec::new();
                        if settings > 0 {
                            parts.push(format!("{settings} setting(s)"));
                        }
                        if patched > 0 {
                            parts.push(format!("{patched} exe patch(es)"));
                        }
                        if done > 0 {
                            parts.push(format!("{done} upgrade(s)"));
                        }
                        if parts.is_empty() {
                            ws.toast(ToastKind::Info, "Everything was already in place", cx);
                        } else {
                            ws.toast(ToastKind::Success, format!("Re-applied {}", parts.join(" · ")), cx);
                        }
                    }
                    (n, _) => ws.toast(
                        ToastKind::Error,
                        format!("{n} step(s) didn't finish. See the list for details"),
                        cx,
                    ),
                }
            })
            .ok();
        })
        .detach();
    }

    fn set_step(&mut self, step: usize, state: StepState, cx: &mut Context<Self>) {
        if let Some(s) = self.setup_run.as_mut().and_then(|r| r.steps.get_mut(step)) {
            s.1 = state;
        }
        cx.notify();
    }

    fn skip_dependents(&mut self, gi: usize, failed: &str, cx: &mut Context<Self>) {
        let def = self.games[gi].def;
        if let Some(run) = &mut self.setup_run {
            for (id, state) in &mut run.steps {
                let depends = def.component(id).is_some_and(|c| c.requires.contains(&failed));
                if depends && *state == StepState::Queued {
                    *state = StepState::Failed(format!("Skipped: needs {failed}"));
                }
            }
        }
        cx.notify();
    }

    /// One-click recovery for when the game, its launcher or a file check
    /// rewrote what Vaulter set up: re-runs the upgrades that need it,
    /// then puts every remembered setting value and exe patch back.
    pub fn reapply_all(&mut self, cx: &mut Context<Self>) {
        if self.setup_running() {
            self.toast(ToastKind::Info, "Already working. Wait for it to finish", cx);
            return;
        }
        if let Some(reason) = self.game().write_blocker() {
            self.toast(ToastKind::Error, reason, cx);
            return;
        }
        if !self.game().config_found() && self.game().install.is_none() {
            self.toast(
                ToastKind::Error,
                "Game install and settings folder not found, so there's nothing to re-apply to",
                cx,
            );
            return;
        }
        self.run_setup(SetupGoal::Reapply, cx);
    }

    /// Runs a health check's button.
    pub fn health_fix(&mut self, fix: crate::health::Fix, cx: &mut Context<Self>) {
        match fix {
            crate::health::Fix::Go(page) => self.navigate(page, cx),
            crate::health::Fix::Reapply => self.reapply_all(cx),
            crate::health::Fix::KeepCurrent => self.keep_current(cx),
        }
    }

    /// The user changed things outside Vaulter on purpose: what's on
    /// disk now becomes what Re-apply puts back, for everything it already
    /// remembers (settings and exe patches alike).
    pub fn keep_current(&mut self, cx: &mut Context<Self>) {
        self.refresh_game(self.active);
        let game = self.game_mut();
        let (def, config, found) = (game.def, game.config.clone(), game.config_found());
        let states = game.exe.as_ref().map(|e| e.patch_states.clone()).unwrap_or_default();
        let recorded = game.update_applied(|a| {
            if found {
                a.sync_values(def, &config);
            }
            a.sync_patches(&states);
        });
        match recorded {
            Ok(()) => self.toast(ToastKind::Success, "Kept. Re-apply will leave these as they are", cx),
            Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
        }
    }

    /// End of a re-apply run: writes the remembered setting values back and
    /// re-patches any remembered exe patch that was reverted, updating their
    /// lines in the step list. The record is read now, not when the run
    /// started, so a change made meanwhile isn't undone. Returns how many
    /// (settings, patches) actually changed.
    fn rewrite_remembered(&mut self, gi: usize, cx: &mut Context<Self>) -> (usize, usize) {
        let applied = self.games[gi].applied.clone();
        let has_step = |ws: &Self, id| ws.setup_run.as_ref().is_some_and(|r| r.steps.iter().any(|(s, _)| *s == id));
        let mut settings = 0;
        if has_step(self, SETTINGS_STEP) {
            let writes = crate::applied::resolve(self.games[gi].def, &applied);
            let state = match self.write_configs(gi, "Before re-applying settings", false, |config| {
                for (tweak, value) in &writes {
                    tweak.write(config, value);
                }
            }) {
                Ok(0) => StepState::Done("Already in place".into()),
                Ok(n) => {
                    settings = n;
                    StepState::Done(format!("{n} put back"))
                }
                Err(e) => StepState::Failed(format!("{e:#}")),
            };
            self.set_step_named(SETTINGS_STEP, state, cx);
        }
        let mut patched = 0;
        if has_step(self, PATCHES_STEP) {
            let state = match self.repatch(gi, &applied.patches) {
                Ok(0) => StepState::Done("Already applied".into()),
                Ok(n) => {
                    patched = n;
                    StepState::Done(format!("{n} re-patched"))
                }
                Err(e) => StepState::Failed(format!("{e:#}")),
            };
            self.set_step_named(PATCHES_STEP, state, cx);
        }
        (settings, patched)
    }

    fn set_step_named(&mut self, id: &str, state: StepState, cx: &mut Context<Self>) {
        if let Some(step) = self.setup_run.as_ref().and_then(|r| r.steps.iter().position(|(s, _)| *s == id)) {
            self.set_step(step, state, cx);
        }
    }

    /// Re-applies the remembered exe patches that were reverted. Returns how
    /// many it patched.
    fn repatch(&mut self, gi: usize, remembered: &BTreeSet<String>) -> Result<usize> {
        let game = &self.games[gi];
        let def = game.def;
        let path = game.exe_path().filter(|p| p.is_file()).context("game install not found")?;
        if game.is_running_now() {
            return Err(anyhow!("{} is running. Close it first", def.name));
        }
        let mut bytes = patches::read_exe(&path).context("reading the exe")?;
        let mut patched = 0;
        for id in remembered {
            if let Some(p) = def.patches.iter().find(|p| p.id == id.as_str())
                && p.state(&bytes) == PatchState::Unpatched
            {
                p.set(&mut bytes, true)?;
                patched += 1;
            }
        }
        if patched > 0 {
            write_exe(def.id, &path, "Before re-applying exe patches", &bytes)?;
        }
        Ok(patched)
    }

    /// Begins one setup step. `None` means the step was already skipped.
    fn start_component(&mut self, game_index: usize, id: &str, uninstall: bool) -> Option<Result<Job>> {
        let skipped = self.setup_run.as_ref().is_some_and(|run| {
            run.steps.iter().any(|(s, st)| *s == id && matches!(st, StepState::Failed(_)))
        });
        if skipped {
            return None;
        }
        let def = self.games[game_index].def;
        let component = def.component(id)?;
        Some(self.component_job(game_index, component, uninstall))
    }

    fn component_job(&mut self, gi: usize, component: &'static setup::Component, uninstall: bool) -> Result<Job> {
        let def = self.games[gi].def;
        let root = self.games[gi].install.as_ref().map(|i| i.root.clone());
        let touches_game = !matches!(component.kind, ComponentKind::LaunchArg(_));
        if touches_game && self.games[gi].is_running_now() {
            return Err(anyhow!("{} is running. Close it first", def.name));
        }
        let need_root = || root.clone().context("game install not found");
        let (game_id, comp_id) = (def.id, component.id);
        Ok(match (component.kind, uninstall) {
            (ComponentKind::Settings { values, display }, false) => {
                let mode = crate::core::display::primary();
                self.write_configs(gi, &format!("Before {}", component.name), false, |config| {
                    for (id, v) in values {
                        if let Some(t) = def.tweak(id) {
                            t.write(config, &v.to_value());
                        }
                    }
                    if let (Some(hook), Some(mode)) = (display, mode) {
                        hook(config, mode);
                    }
                })?;
                // Remember every value of the bundle, not only the ones that
                // changed, so Re-apply can put any of them back.
                let game = &mut self.games[gi];
                let set: Vec<(&'static str, Option<Value>)> =
                    values.iter().filter_map(|(id, _)| def.tweak(id)).map(|t| (t.id, game.current(t))).collect();
                game.update_applied(|a| a.merge(&set))?;
                setup::record_settings_applied(game_id, comp_id)?;
                Job::Done(match (display, mode) {
                    (Some(_), Some(m)) => format!("Tuned for {}×{} @ {} Hz", m.width, m.height, m.refresh_hz),
                    _ => "Applied".into(),
                })
            }
            (ComponentKind::Settings { values, .. }, true) => {
                self.write_configs(gi, &format!("Before removing {}", component.name), false, |config| {
                    for (id, _) in values {
                        if let Some(t) = def.tweak(id) {
                            t.write(config, &t.default.to_value());
                        }
                    }
                })?;
                // Removed on purpose: Re-apply mustn't bring these back.
                let ids: Vec<&'static str> = values.iter().filter_map(|(id, _)| def.tweak(id)).map(|t| t.id).collect();
                self.games[gi].update_applied(|a| a.forget(ids))?;
                crate::core::manifest::remove(game_id, comp_id);
                Job::Done("Back to game defaults".into())
            }
            (ComponentKind::ExePatch(patch), uninstall) => {
                let p = def.patches.iter().find(|p| p.id == patch).context("unknown patch")?;
                if uninstall && !p.revertible {
                    return Ok(Job::Done("Kept (the game ships with it)".into()));
                }
                let path = self.games[gi].exe_path().context("game install not found")?;
                let mut bytes = patches::read_exe(&path)?;
                p.set(&mut bytes, !uninstall)?;
                write_exe(game_id, &path, &format!("Before {}", p.name), &bytes)?;
                self.games[gi].update_applied(|a| a.set_patch(p.id, !uninstall))?;
                Job::Done(if uninstall { "Reverted" } else { "Patched" }.into())
            }
            (ComponentKind::LaunchArg(arg), remove) => {
                let mut args = self.launch_args_for(gi);
                args.retain(|a| !a.eq_ignore_ascii_case(arg));
                if !remove {
                    args.push(arg.to_string());
                }
                self.settings.launch_args.insert(game_id.to_string(), args);
                self.settings.save();
                Job::Done(if remove { "Removed" } else { "Added to Play" }.into())
            }
            (ComponentKind::Dxvk { target, exe_dir, conf }, false) => {
                let root = need_root()?;
                Job::Background(Box::new(move || {
                    setup::install_dxvk(game_id, comp_id, &root, target, exe_dir, conf).map(|v| format!("DXVK {v}"))
                }))
            }
            (ComponentKind::Sdk, false) => {
                let root = need_root()?;
                let support = def.mods.context("no SDK for this game")?;
                Job::Background(Box::new(move || {
                    mods::install_sdk_latest(game_id, support, &root).map(|v| format!("SDK {v}"))
                }))
            }
            (ComponentKind::Sdk, true) => {
                let support = def.mods.context("no SDK for this game")?;
                mods::uninstall_sdk(game_id, support, &need_root()?)?;
                Job::Done("Removed".into())
            }
            (ComponentKind::File { url, dest, enable }, false) => {
                let root = need_root()?;
                Job::Background(Box::new(move || setup::install_file(game_id, comp_id, &root, url, dest, enable)))
            }
            (ComponentKind::SdkZip { url, folder }, false) => {
                let root = need_root()?;
                Job::Background(Box::new(move || setup::install_sdk_zip(game_id, comp_id, &root, url, folder)))
            }
            (ComponentKind::Archive { .. }, false) => {
                let root = need_root()?;
                Job::Background(Box::new(move || setup::install_archive(game_id, component, &root)))
            }
            (ComponentKind::Hide { path }, false) => Job::Done(setup::hide_file(game_id, comp_id, &need_root()?, path)?),
            (ComponentKind::Hide { path }, true) => {
                setup::unhide_file(game_id, comp_id, &need_root()?, path)?;
                Job::Done("Restored".into())
            }
            (ComponentKind::TextPatch { game, gearbox_url, .. }, uninstall) => {
                // Every text patch component shares one merged file, so
                // adding or removing one rebuilds it from the rest.
                let root = need_root()?;
                let mut ids = setup::text_patch_parts(game_id);
                ids.retain(|id| id != comp_id);
                if !uninstall {
                    ids.push(comp_id.to_string());
                }
                let parts: Vec<(&'static str, &'static [setup::TextSource])> = def
                    .setup
                    .iter()
                    .filter(|c| ids.iter().any(|id| id == c.id))
                    .filter_map(|c| match c.kind {
                        ComponentKind::TextPatch { sources, .. } => Some((c.id, sources)),
                        _ => None,
                    })
                    .collect();
                Job::Background(Box::new(move || setup::rebuild_text_patch(game_id, &root, game, gearbox_url, &parts)))
            }
            (ComponentKind::Dxvk { .. } | ComponentKind::File { .. } | ComponentKind::SdkZip { .. } | ComponentKind::Archive { .. }, true) => {
                setup::uninstall_files(game_id, comp_id, &need_root()?)?;
                Job::Done("Removed".into())
            }
        })
    }

    pub fn set_config_lock(&mut self, locked: bool, cx: &mut Context<Self>) {
        let paths: Vec<PathBuf> = self
            .game()
            .config
            .files()
            .filter(|(_, f)| f.exists)
            .map(|(_, f)| f.path.clone())
            .collect();
        let result: Result<()> = paths.iter().try_for_each(|p| backup::set_readonly(p, locked));
        match result {
            Ok(()) => self.toast(
                ToastKind::Success,
                if locked {
                    "Config files locked (read-only)"
                } else {
                    "Config files unlocked"
                },
                cx,
            ),
            Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
        }
        cx.notify();
    }

    pub fn configs_locked(&self) -> bool {
        let files: Vec<_> = self.game().config.files().filter(|(_, f)| f.exists).collect();
        !files.is_empty() && files.iter().all(|(_, f)| backup::is_readonly(&f.path))
    }

    // ---- exe patches -------------------------------------------------------------

    pub fn set_exe_patch(&mut self, patch_id: &str, enabled: bool, cx: &mut Context<Self>) {
        let result = (|| -> Result<()> {
            let game = self.game();
            if game.is_running_now() {
                return Err(anyhow!("{} is running. Close it first", game.def.name));
            }
            let path = game.exe_path().context("game install not found")?;
            let patch = game
                .def
                .patches
                .iter()
                .find(|p| p.id == patch_id)
                .context("unknown patch")?;
            let mut bytes = patches::read_exe(&path)?;
            patch.set(&mut bytes, enabled)?;
            let verb = if enabled { "Before applying" } else { "Before reverting" };
            write_exe(game.def.id, &path, &format!("{verb} {}", patch.name), &bytes)
        })();
        match result {
            Ok(()) => {
                let recorded = self.game_mut().update_applied(|a| a.set_patch(patch_id, enabled));
                self.refresh_active(cx);
                match recorded {
                    Ok(()) => self.toast(ToastKind::Success, if enabled { "Patch applied" } else { "Patch reverted" }, cx),
                    Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
                }
            }
            Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
        }
    }

    // ---- backups -----------------------------------------------------------------

    pub fn backup_configs_now(&mut self, cx: &mut Context<Self>) {
        let game = self.game();
        let paths: Vec<PathBuf> = game
            .config
            .files()
            .filter(|(_, f)| f.exists)
            .map(|(_, f)| f.path.clone())
            .collect();
        match backup::create(game.def.id, "Manual config snapshot", &paths) {
            Ok(_) => {
                self.refresh_active(cx);
                self.toast(ToastKind::Success, "Snapshot saved", cx);
            }
            Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
        }
    }

    /// Restores a backup (identified by its folder, not a list position),
    /// snapshotting the current files first so the restore can be undone.
    pub fn restore_backup(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let Some(b) = backup::load(&dir) else {
            self.toast(ToastKind::Error, "That backup no longer exists", cx);
            return;
        };
        if let Some(reason) = self.game().write_blocker() {
            self.toast(ToastKind::Error, reason, cx);
            return;
        }
        let current: Vec<PathBuf> = b.files.iter().map(|f| f.original.clone()).collect();
        let result = backup::create(self.game().def.id, &format!("Before restoring \"{}\"", b.label), &current)
            .context("backing up the current files")
            .and_then(|_| backup::restore(&b));
        match result {
            Ok(()) => {
                self.refresh_active(cx);
                // The restored files are the new intent, so the re-apply
                // record takes their values (for the settings it already
                // remembers) and drops exe patches a restored exe undid,
                // rather than resurrecting what the restore replaced.
                let game = self.game_mut();
                let exe = game.exe_path().map(|p| crate::compare::normalize_path(&p));
                let exe_restored = b.files.iter().any(|f| Some(crate::compare::normalize_path(&f.original)) == exe);
                let (def, config, found) = (game.def, game.config.clone(), game.config_found());
                let states = game.exe.as_ref().map(|e| e.patch_states.clone()).unwrap_or_default();
                let recorded = game.update_applied(|a| {
                    if found {
                        a.sync_values(def, &config);
                    }
                    if exe_restored {
                        a.sync_patches(&states);
                    }
                });
                match recorded {
                    Ok(()) => self.toast(ToastKind::Success, format!("Restored \"{}\"", b.label), cx),
                    Err(e) => self.toast(ToastKind::Error, format!("Restored \"{}\", but {e:#}", b.label), cx),
                }
            }
            Err(e) => self.toast(ToastKind::Error, format!("Restore failed: {e:#}"), cx),
        }
    }

    pub fn delete_backup(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
        let Some(b) = backup::load(&dir) else {
            return;
        };
        match backup::delete(&b) {
            Ok(()) => self.refresh_active(cx),
            Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
        }
    }

    // ---- mods --------------------------------------------------------------------

    pub fn install_sdk(&mut self, cx: &mut Context<Self>) {
        if self.refused(self.mods_blocker(), cx) {
            return;
        }
        let game = self.game();
        let (Some(support), Some(install)) = (game.def.mods, game.install.clone()) else {
            return;
        };
        let game_id = game.def.id;
        self.busy = Some(format!("Downloading {}…", support.sdk_name));
        cx.notify();
        let task = cx
            .background_executor()
            .spawn(async move { mods::install_sdk_latest(game_id, support, &install.root) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |ws, cx| {
                ws.busy = None;
                ws.refresh_active(cx);
                match result {
                    Ok(version) => ws.toast(
                        ToastKind::Success,
                        format!("{} {version} installed", support.sdk_name),
                        cx,
                    ),
                    Err(e) => ws.toast(ToastKind::Error, format!("SDK install failed: {e:#}"), cx),
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn install_sdk_from_zip(&mut self, zip: PathBuf, cx: &mut Context<Self>) {
        if self.refused(self.mods_blocker(), cx) {
            return;
        }
        let game = self.game();
        let (Some(support), Some(install)) = (game.def.mods, game.install.clone()) else {
            return;
        };
        let result = mods::install_sdk_zip(game.def.id, support, &install.root, &zip);
        self.refresh_active(cx);
        match result {
            Ok(()) => self.toast(ToastKind::Success, format!("{} installed", support.sdk_name), cx),
            Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
        }
    }

    pub fn uninstall_sdk(&mut self, cx: &mut Context<Self>) {
        if self.refused(self.mods_blocker(), cx) {
            return;
        }
        let game = self.game();
        let (Some(support), Some(install)) = (game.def.mods, game.install.clone()) else {
            return;
        };
        let result = mods::uninstall_sdk(game.def.id, support, &install.root);
        self.refresh_active(cx);
        match result {
            Ok(()) => self.toast(ToastKind::Success, "Mod manager removed (your mods were kept)", cx),
            Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
        }
    }

    pub fn install_mod_files(&mut self, files: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.refused(self.mods_blocker(), cx) {
            return;
        }
        let game = self.game();
        let (Some(support), Some(install)) = (game.def.mods, game.install.clone()) else {
            return;
        };
        let mut ok = 0;
        for file in files {
            match mods::install_mod(support, &install.root, &file) {
                Ok(()) => ok += 1,
                Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
            }
        }
        self.refresh_active(cx);
        if ok > 0 {
            self.toast(ToastKind::Success, format!("Installed {ok} mod(s)"), cx);
        }
    }

    pub fn set_mod_enabled(&mut self, path: PathBuf, enabled: bool, cx: &mut Context<Self>) {
        if self.refused(self.mods_blocker(), cx) {
            return;
        }
        let game = self.game();
        let (Some(support), Some(install)) = (game.def.mods, game.install.clone()) else {
            return;
        };
        let Some(entry) = game.mods.iter().find(|m| m.path == path).cloned() else {
            return;
        };
        if let Err(e) = mods::set_enabled(support, &install.root, &entry, enabled) {
            self.toast(ToastKind::Error, format!("{e:#}"), cx);
        }
        self.refresh_active(cx);
    }

    pub fn remove_mod(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.refused(self.mods_blocker(), cx) {
            return;
        }
        let Some(entry) = self.game().mods.iter().find(|m| m.path == path).cloned() else {
            return;
        };
        match mods::remove(&entry) {
            Ok(()) => self.toast(ToastKind::Success, format!("Removed {}", entry.name), cx),
            Err(e) => self.toast(ToastKind::Error, format!("{e:#}"), cx),
        }
        self.refresh_active(cx);
    }

    // ---- launch ------------------------------------------------------------------

    pub fn launch_args(&self) -> Vec<String> {
        self.launch_args_for(self.active)
    }

    pub(crate) fn launch_args_for(&self, gi: usize) -> Vec<String> {
        let def = self.games[gi].def;
        self.settings
            .launch_args
            .get(def.id)
            .cloned()
            .unwrap_or_else(|| {
                def.launch_args
                    .iter()
                    .filter(|a| a.default_on)
                    .map(|a| a.arg.to_string())
                    .collect()
            })
    }

    pub fn toggle_launch_arg(&mut self, arg: &str, cx: &mut Context<Self>) {
        let mut args = self.launch_args();
        if let Some(i) = args.iter().position(|a| a == arg) {
            args.remove(i);
        } else {
            args.push(arg.to_string());
        }
        let id = self.game().def.id.to_string();
        self.settings.launch_args.insert(id, args);
        self.settings.save();
        self.game().invalidate_statuses();
        cx.notify();
    }

    /// How the Play button launches the current game, the mode last picked
    /// in its dropdown, falling back to `Normal` when it can't be used (no
    /// launcher for this game, or its exe isn't there).
    pub fn launch_mode(&self) -> LaunchMode {
        let game = self.game();
        let picked = self.settings.launch_mode.get(game.def.id).copied();
        usable_launch_mode(picked, game.def.launcher.is_some(), || game.launcher_path().is_some())
    }

    /// Remembers `mode` as the Play button's way of starting this game.
    pub fn set_launch_mode(&mut self, mode: LaunchMode, cx: &mut Context<Self>) {
        let id = self.game().def.id.to_string();
        if mode == LaunchMode::Normal {
            self.settings.launch_mode.remove(&id);
        } else {
            self.settings.launch_mode.insert(id, mode);
        }
        self.settings.save();
        cx.notify();
    }

    /// Play with whatever way is selected in the dropdown (default: launch
    /// options, the real game exe, never a Steam command).
    pub fn launch_default(&mut self, cx: &mut Context<Self>) {
        self.launch(self.launch_mode(), cx);
    }

    pub fn launch(&mut self, mode: LaunchMode, cx: &mut Context<Self>) {
        // The game would lock the files a setup run is writing.
        if self.job_blocker().is_some() {
            self.toast(ToastKind::Info, "Still working. Wait for it to finish, then play", cx);
            return;
        }
        let def = self.game().def;
        let (exe, args) = match mode {
            LaunchMode::Launcher => {
                let path = def
                    .launcher
                    .and_then(|l| self.game().install.as_ref().map(|i| i.root.join(l.exe)))
                    .filter(|p| p.is_file());
                let Some(path) = path else {
                    self.toast(ToastKind::Error, "The game's launcher wasn't found", cx);
                    return;
                };
                (path, Vec::new())
            }
            _ => {
                let Some(exe) = self.game().exe_path() else {
                    self.toast(ToastKind::Error, "Game install not found", cx);
                    return;
                };
                let mut args = self.launch_args();
                if mode == LaunchMode::Direct
                    && let Some(l) = def.launcher
                    && !args.iter().any(|a| a.eq_ignore_ascii_case(l.skip_arg))
                {
                    args.push(l.skip_arg.to_string());
                }
                (exe, args)
            }
        };
        let result = std::process::Command::new(&exe)
            .args(&args)
            .current_dir(exe.parent().unwrap_or(&exe))
            .spawn();
        match result {
            Ok(_) => self.toast(ToastKind::Success, format!("Launching {}…", def.short), cx),
            Err(e) => self.toast(ToastKind::Error, format!("Launch failed: {e}"), cx),
        }
    }

    // ---- game running & background mode -----------------------------------------
    //
    // A timer loop checks whether the active game's exe is running (every 2s,
    // every 10s in background mode) and notifies only when the answer
    // changes. While it runs, the Shell swaps the page for the "game is
    // running" screen (`pages/running.rs`). "Minimize to background" mutes
    // UI audio and slows the loop; it ends when the window is brought back or
    // the game exits.

    /// The watch's loop. It holds only a weak handle, so it ends with the
    /// workspace; the returned task is also kept on the workspace, so there
    /// is exactly one loop, whatever game is selected.
    fn spawn_run_poll(cx: &mut Context<Self>) -> gpui::Task<()> {
        cx.spawn(async move |this, cx| loop {
            let Ok(delay) = this.read_with(cx, |ws, _| ws.run_watch.poll_interval()) else { break };
            cx.background_executor().timer(delay).await;
            let Ok((gi, exe)) = this.read_with(cx, |ws, _| (ws.active, ws.game().exe_path())) else { break };
            // The process snapshot runs off the UI thread.
            let running = match exe {
                Some(exe) => cx.background_executor().spawn(async move { crate::compare::game_running(&exe) }).await,
                None => false,
            };
            if this.update(cx, |ws, cx| ws.apply_run_poll(gi, running, cx)).is_err() {
                break;
            }
        })
    }

    /// Folds a poll of game `gi` in; notifies only if something changed.
    fn apply_run_poll(&mut self, gi: usize, running: bool, cx: &mut Context<Self>) {
        self.games[gi].note_running(running);
        // A result for a game that's no longer selected is stale.
        if gi != self.active {
            return;
        }
        match self.run_watch.observe(running) {
            RunChange::None => {}
            RunChange::Started => cx.notify(),
            // A comparison capture starts and stops the game itself and
            // reports on its own.
            RunChange::Stopped { .. } if self.capture_running() => cx.notify(),
            RunChange::Stopped { background } => {
                let name = self.games[gi].def.name;
                self.game_stopped(gi, background, format!("{name} closed. You're back in control"), cx);
            }
        }
    }

    /// The game exited: restore audio and the window if it was in the
    /// background, re-read everything (the game rewrites its settings on
    /// exit), and say so.
    fn game_stopped(&mut self, gi: usize, was_background: bool, message: String, cx: &mut Context<Self>) {
        if was_background {
            self.apply_run_audio();
            cx.emit(RunEvent::BringToFront);
        }
        // A recovery that had to wait for the game to close can run now.
        if !self.capture_running() {
            self.recover_captures(cx);
        }
        self.refresh_game(gi);
        self.toast(ToastKind::Success, message, cx);
    }

    /// Sets UI audio for the current mode: silent in the background, else
    /// the user's saved choices.
    fn apply_run_audio(&self) {
        let (muted, music) = run_audio(self.run_watch.background, &self.settings);
        crate::sound::set_muted(muted);
        crate::sound::set_music(music);
    }

    /// True while the "game is running" screen should replace the page.
    /// Never over a comparison capture, which runs the game itself.
    pub fn show_running_screen(&self) -> bool {
        self.run_watch.shows_screen() && !self.capture_running()
    }

    /// "Keep working anyway": hide the running screen until the game stops.
    pub fn dismiss_running_screen(&mut self, cx: &mut Context<Self>) {
        self.run_watch.dismissed = true;
        cx.notify();
    }

    /// Enters background mode. Returns false (and changes nothing) when the
    /// game isn't running; the caller minimizes the window on true.
    pub fn enter_background(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.run_watch.enter_background() {
            return false;
        }
        self.apply_run_audio();
        cx.notify();
        // Once the window has minimized, give the game the memory we aren't using.
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            if this.read_with(cx, |ws, _| ws.run_watch.background).unwrap_or(false) {
                crate::win11::trim_memory();
            }
        })
        .detach();
        true
    }

    /// The window gained or lost OS focus. Coming back after being seen
    /// inactive ends background mode.
    pub fn window_activation_changed(&mut self, active: bool, cx: &mut Context<Self>) {
        if !self.run_watch.activation(active) {
            return;
        }
        self.apply_run_audio();
        // Catch up on what was skipped while in the background.
        let gi = self.active;
        self.refresh_game(gi);
        let running = self.games[gi].exe_path().is_some_and(|p| crate::compare::game_running(&p));
        self.apply_run_poll(gi, running, cx);
        cx.notify();
    }

    /// Kills the active game's process (after the page confirmed it), then
    /// re-reads its state.
    pub fn force_close_game(&mut self, cx: &mut Context<Self>) {
        let gi = self.active;
        let game = &self.games[gi];
        let name = game.def.name;
        let Some(exe) = game.exe_path() else {
            self.toast(ToastKind::Error, "Game install not found", cx);
            return;
        };
        self.busy = Some(format!("Closing {name}\u{2026}"));
        cx.notify();
        let executor = cx.background_executor().clone();
        let task = cx.background_executor().spawn(async move {
            crate::compare::kill_game(&exe);
            // taskkill returns once the kill is requested; give the process
            // a moment to actually go.
            for _ in 0..20 {
                if !crate::compare::game_running(&exe) {
                    return false;
                }
                executor.timer(Duration::from_millis(100)).await;
            }
            crate::compare::game_running(&exe)
        });
        cx.spawn(async move |this, cx| {
            let still_running = task.await;
            this.update(cx, |ws, cx| {
                ws.busy = None;
                ws.games[gi].note_running(still_running);
                if still_running {
                    ws.toast(ToastKind::Error, format!("Couldn't close {name}. Try quitting from the game"), cx);
                    return;
                }
                let change = if gi == ws.active { ws.run_watch.observe(false) } else { RunChange::None };
                match change {
                    RunChange::Stopped { background } => ws.game_stopped(gi, background, format!("{name} was closed"), cx),
                    // The poll already noticed, or another game is selected now.
                    _ => {
                        ws.refresh_game(gi);
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    // ---- misc --------------------------------------------------------------------

    pub fn update_settings(&mut self, f: impl FnOnce(&mut AppSettings), cx: &mut Context<Self>) {
        f(&mut self.settings);
        self.settings.frame_rate.apply();
        self.settings.save();
        cx.notify();
    }

    pub fn toast(&mut self, kind: ToastKind, message: impl Into<String>, cx: &mut Context<Self>) {
        self.toast_with(kind, message, None, cx);
    }

    pub fn toast_with(&mut self, kind: ToastKind, message: impl Into<String>, action: Option<ToastAction>, cx: &mut Context<Self>) {
        let message = message.into();
        // Repeated saves replace the previous "Saved" toast instead of stacking.
        self.toasts.retain(|t| !(t.message == message && t.action == action));
        let id = self.next_toast;
        self.next_toast += 1;
        if kind == ToastKind::Success {
            crate::sound::play(crate::sound::Sound::Applied);
        }
        self.toasts.push(Toast { id, kind, message, action });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
        cx.notify();
        let secs = if kind == ToastKind::Error || action.is_some() { 8 } else { 4 };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(secs)).await;
            this.update(cx, |ws, cx| {
                ws.toasts.retain(|t| t.id != id);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn welcome_done(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.settings.welcomed = true;
        self.settings.mode = mode;
        self.page = self.home_page();
        self.settings.save();
        cx.notify();
    }

    pub fn dismiss_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        self.toasts.retain(|t| t.id != id);
        cx.notify();
    }
}

// ---- game running & background mode: state ---------------------------------------

/// Asks the window to come to the front (the game exited while Vault
/// Patcher was minimized to the background).
pub enum RunEvent {
    BringToFront,
}

impl gpui::EventEmitter<RunEvent> for Workspace {}

/// What the running watch knows about the selected game. Plain state, so the
/// transitions are unit-tested.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RunWatch {
    /// Last polled answer.
    running: bool,
    /// "Keep working anyway" was pressed during this run of the game.
    dismissed: bool,
    /// Minimized to the background while the game plays.
    background: bool,
    /// The window was seen inactive since entering background mode. Only an
    /// activation after that ends it, so a focus event still in flight
    /// from before the minimize can't cancel it straight away.
    seen_inactive: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum RunChange {
    None,
    Started,
    /// The game exited; `background` says whether background mode ended with it.
    Stopped { background: bool },
}

impl RunWatch {
    fn poll_interval(&self) -> Duration {
        if self.background { Duration::from_secs(10) } else { Duration::from_secs(2) }
    }

    fn shows_screen(&self) -> bool {
        self.running && !self.dismissed
    }

    fn observe(&mut self, running: bool) -> RunChange {
        if running == self.running {
            return RunChange::None;
        }
        if running {
            self.running = true;
            return RunChange::Started;
        }
        let background = self.background;
        *self = RunWatch::default();
        RunChange::Stopped { background }
    }

    /// Starts over for a newly selected game. Returns true if this ended
    /// background mode (so audio must be restored).
    fn reset(&mut self, running: bool) -> bool {
        let was_background = self.background;
        *self = RunWatch { running, ..RunWatch::default() };
        was_background
    }

    fn enter_background(&mut self) -> bool {
        if !self.running || self.background {
            return false;
        }
        self.background = true;
        self.seen_inactive = false;
        true
    }

    /// Returns true when this activation change ends background mode.
    fn activation(&mut self, active: bool) -> bool {
        if !self.background {
            return false;
        }
        if !active {
            self.seen_inactive = true;
            return false;
        }
        if !self.seen_inactive {
            return false;
        }
        self.background = false;
        self.seen_inactive = false;
        true
    }
}

/// UI audio as (muted, music playing): silent in background mode, otherwise
/// exactly what the user saved, never unconditionally unmuted.
fn run_audio(background: bool, settings: &AppSettings) -> (bool, bool) {
    if background {
        (true, false)
    } else {
        (settings.sound_muted, settings.music && !settings.sound_muted)
    }
}

/// The settings "Factory Settings" puts back. Skips the monitor-specific
/// display choices, and Simple mode's combined "quick" controls: those only
/// read and write keys the plain settings own (the framerate limit is
/// `fps_lock`'s `FramerateLocking`), so resetting both would fight over them.
fn factory_tweaks(def: &'static GameDef) -> impl Iterator<Item = &'static Tweak> {
    def.visible_tweaks()
        .filter(|t| t.category != "quick" && !matches!(t.id, "resolution" | "window_mode" | "fullscreen"))
}

/// Label shared by Quick Settings saves, so a burst of edits shares one backup.
const QUICK_LABEL: &str = "Before Quick Settings changes";

/// How a settings write snapshots the files it's about to change.
#[derive(Debug, PartialEq)]
enum SnapshotPlan {
    /// A new backup of every changed file.
    New,
    /// Add these files to the burst's backup; it holds the others already.
    Extend(Vec<PathBuf>),
    /// The burst's backup already holds every file, as it was before the burst.
    Covered,
}

/// A burst of Quick Settings saves shares one backup (`recent`, the newest
/// one, if it has `label` and is under five minutes old), but every file the
/// burst touches must be in it, a later edit to another file adds that file.
fn snapshot_plan(recent: Option<&Backup>, label: &str, dirty: &[PathBuf], now: chrono::NaiveDateTime) -> SnapshotPlan {
    let fresh = recent.filter(|b| {
        b.label == label
            && chrono::NaiveDateTime::parse_from_str(&b.created_at, "%Y-%m-%d %H:%M:%S")
                .is_ok_and(|at| (chrono::TimeDelta::zero()..chrono::TimeDelta::minutes(5)).contains(&(now - at)))
    });
    match fresh.map(|b| backup::missing_from(b, dirty)) {
        None => SnapshotPlan::New,
        Some(missing) if missing.is_empty() => SnapshotPlan::Covered,
        Some(missing) => SnapshotPlan::Extend(missing),
    }
}

/// Patch states for an exe, cached by size and modification time so the
/// multi-megabyte scan only runs when the exe actually changed. `None` when
/// the exe can't be read right now (a virus scan or Steam update holding
/// it); that isn't cached, so the next refresh tries again.
fn scan_exe(def: &GameDef, path: &std::path::Path) -> Option<ExeInfo> {
    use std::sync::Mutex;
    type Key = (PathBuf, u64, Option<std::time::SystemTime>);
    static CACHE: Mutex<Vec<(Key, ExeInfo)>> = Mutex::new(Vec::new());
    let meta = std::fs::metadata(path).ok();
    let key: Key = (
        path.to_path_buf(),
        meta.as_ref().map_or(0, |m| m.len()),
        meta.and_then(|m| m.modified().ok()),
    );
    if let Some((_, info)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| *k == key) {
        return Some(info.clone());
    }
    // A previous launch may have scanned this exact exe already.
    let info = match exe_scan_store::load(def, &key.0, key.1, key.2) {
        Some(info) => info,
        None => {
            let bytes = patches::read_exe(path).ok()?;
            let info = ExeInfo {
                size: bytes.len() as u64,
                patch_states: def.patches.iter().map(|p| (p.id, p.state(&bytes))).collect(),
            };
            exe_scan_store::save(&key.0, key.1, key.2, &info);
            info
        }
    };
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|(k, _)| k.0 != key.0);
    cache.push((key, info.clone()));
    Some(info)
}

/// Exe scan results kept on disk (`exe-scan.json`), keyed by path, size and
/// modification time, so a launch doesn't re-read a 27 MB exe that hasn't
/// changed. Anything unreadable or stale is simply scanned again.
mod exe_scan_store {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    use serde::{Deserialize, Serialize};

    use super::ExeInfo;
    use crate::core::binpatch::PatchState;
    use crate::games::GameDef;

    #[derive(Serialize, Deserialize, Default)]
    struct Store {
        entries: Vec<Entry>,
    }

    #[derive(Serialize, Deserialize)]
    struct Entry {
        path: PathBuf,
        len: u64,
        /// Modification time in nanoseconds since the epoch.
        modified: Option<u128>,
        states: HashMap<String, String>,
    }

    fn file() -> PathBuf {
        crate::core::backup::data_dir().join("exe-scan.json")
    }

    fn nanos(t: Option<SystemTime>) -> Option<u128> {
        t.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_nanos())
    }

    fn state_name(s: PatchState) -> &'static str {
        match s {
            PatchState::Unpatched => "unpatched",
            PatchState::Patched => "patched",
            PatchState::Unsupported => "unsupported",
        }
    }

    fn read() -> Store {
        std::fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub(super) fn load(def: &GameDef, path: &Path, len: u64, modified: Option<SystemTime>) -> Option<ExeInfo> {
        let modified = nanos(modified)?;
        let store = read();
        let entry = store.entries.iter().find(|e| e.path == path && e.len == len && e.modified == Some(modified))?;
        // Every patch this build knows must have a remembered state.
        let mut patch_states = HashMap::new();
        for patch in def.patches {
            let state = match entry.states.get(patch.id)?.as_str() {
                "unpatched" => PatchState::Unpatched,
                "patched" => PatchState::Patched,
                "unsupported" => PatchState::Unsupported,
                _ => return None,
            };
            patch_states.insert(patch.id, state);
        }
        Some(ExeInfo { size: len, patch_states })
    }

    pub(super) fn save(path: &Path, len: u64, modified: Option<SystemTime>, info: &ExeInfo) {
        let Some(modified) = nanos(modified) else { return };
        let mut store = read();
        store.entries.retain(|e| e.path != path);
        store.entries.push(Entry {
            path: path.to_path_buf(),
            len,
            modified: Some(modified),
            states: info.patch_states.iter().map(|(k, v)| (k.to_string(), state_name(*v).to_string())).collect(),
        });
        if let Ok(bytes) = serde_json::to_vec_pretty(&store) {
            let _ = std::fs::create_dir_all(crate::core::backup::data_dir());
            let _ = crate::core::atomic::write(&file(), &bytes);
        }
    }
}

/// Work for one setup step.
enum Job {
    Done(String),
    Background(Box<dyn FnOnce() -> Result<String> + Send>),
}

/// Snapshots the game exe, replaces it with `bytes` in one step (never a
/// half-written exe) and reads it back to be sure it landed. Old exe
/// snapshots are pruned, keeping the very first one (the exe as it shipped).
fn write_exe(game_id: &str, path: &std::path::Path, label: &str, bytes: &[u8]) -> Result<()> {
    backup::create(game_id, label, &[path.to_path_buf()])?;
    crate::core::atomic::write(path, bytes).with_context(|| format!("is the game running? ({})", path.display()))?;
    if std::fs::read(path).ok().as_deref() != Some(bytes) {
        return Err(anyhow!("{} didn't save correctly; restore it from Backups", path.display()));
    }
    for old in exe_backups_to_prune(&backup::list(game_id), EXE_BACKUPS_KEPT) {
        let _ = backup::delete(old);
    }
    Ok(())
}

/// Recent exe snapshots kept besides the first one.
const EXE_BACKUPS_KEPT: usize = 3;

/// Exe-only snapshots (each a full copy of a 30+ MB exe) past the newest
/// `keep`, never the oldest one, which holds the exe as it shipped.
/// `backups` is newest first, as `backup::list` returns them.
fn exe_backups_to_prune(backups: &[Backup], keep: usize) -> Vec<&Backup> {
    let exe_only: Vec<&Backup> = backups
        .iter()
        .filter(|b| !b.files.is_empty() && b.files.iter().all(|f| f.original.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))))
        .collect();
    let oldest = exe_only.len().saturating_sub(1);
    exe_only.into_iter().enumerate().filter(|&(i, _)| i >= keep && i != oldest).map(|(_, b)| b).collect()
}

/// Keeps the newest `max` config-only snapshots. Snapshots holding anything
/// else (the game exe, mod manager files) and the original-settings snapshot
/// are never pruned automatically.
fn prune_backups(game_id: &str, max: Option<usize>) {
    let max = max.unwrap_or(30);
    let config_only = |b: &Backup| {
        b.files
            .iter()
            .all(|f| f.original.extension().is_some_and(|e| e.eq_ignore_ascii_case("ini")))
    };
    let prunable = |b: &Backup| config_only(b) && !backup::is_original(b);
    for old in backup::list(game_id).into_iter().filter(prunable).skip(max) {
        let _ = backup::delete(&old);
    }
}

#[cfg(test)]
mod frame_rate_tests {
    use super::balanced_fps;

    #[test]
    fn balanced_divides_the_refresh_rate() {
        assert_eq!(balanced_fps(180), 45);
        assert_eq!(balanced_fps(144), 48);
        assert_eq!(balanced_fps(120), 40);
        assert_eq!(balanced_fps(165), 41);
        assert_eq!(balanced_fps(240), 48);
        assert_eq!(balanced_fps(60), 60);
        assert_eq!(balanced_fps(30), 30);
    }
}

#[cfg(test)]
mod run_watch_tests {
    use super::*;

    #[test]
    fn notices_start_and_stop_once() {
        let mut w = RunWatch::default();
        assert_eq!(w.observe(false), RunChange::None);
        assert_eq!(w.observe(true), RunChange::Started);
        assert_eq!(w.observe(true), RunChange::None);
        assert!(w.shows_screen());
        assert_eq!(w.observe(false), RunChange::Stopped { background: false });
        assert_eq!(w.observe(false), RunChange::None);
        assert!(!w.shows_screen());
    }

    #[test]
    fn dismissal_lasts_until_the_game_stops() {
        let mut w = RunWatch::default();
        w.observe(true);
        w.dismissed = true;
        assert!(!w.shows_screen());
        w.observe(false);
        w.observe(true);
        assert!(w.shows_screen());
    }

    #[test]
    fn background_needs_a_running_game() {
        let mut w = RunWatch::default();
        assert!(!w.enter_background());
        w.observe(true);
        assert!(w.enter_background());
        assert!(!w.enter_background());
        assert_eq!(w.poll_interval(), Duration::from_secs(10));
    }

    #[test]
    fn game_exit_ends_background() {
        let mut w = RunWatch::default();
        w.observe(true);
        w.enter_background();
        assert_eq!(w.observe(false), RunChange::Stopped { background: true });
        assert!(!w.background);
        assert_eq!(w.poll_interval(), Duration::from_secs(2));
    }

    #[test]
    fn restore_needs_deactivation_first() {
        let mut w = RunWatch::default();
        w.observe(true);
        w.enter_background();
        // A stale "active" from before the minimize is ignored.
        assert!(!w.activation(true));
        assert!(w.background);
        assert!(!w.activation(false));
        assert!(w.activation(true));
        assert!(!w.background);
        assert!(w.running);
        // Outside background mode activation changes nothing.
        assert!(!w.activation(false));
        assert!(!w.activation(true));
    }

    #[test]
    fn switching_games_reports_background_end() {
        let mut w = RunWatch::default();
        w.observe(true);
        w.enter_background();
        assert!(w.reset(false));
        assert_eq!(w, RunWatch::default());
        assert!(!w.reset(true));
        assert!(w.running);
    }

    #[test]
    fn audio_restores_saved_settings() {
        let mut s = AppSettings::default();
        assert_eq!(run_audio(true, &s), (true, false));
        s.music = true;
        assert_eq!(run_audio(false, &s), (false, true));
        s.sound_muted = true;
        assert_eq!(run_audio(false, &s), (true, false));
        assert_eq!(run_audio(true, &s), (true, false));
    }

    #[test]
    fn reapply_only_restores_what_we_installed() {
        let bl2 = &crate::games::bl2::GAME;
        let kind = |id| bl2.component(id).unwrap().kind;
        let missing = Status::Missing;
        let active = Status::Active(None);
        // Recommended but never installed (or removed with Restore vanilla).
        assert!(!reapply_wants(&kind("dxvk"), &missing, false));
        assert!(!reapply_wants(&kind("sdk"), &missing, false));
        // Installed by us and gone missing: put back.
        assert!(reapply_wants(&kind("dxvk"), &missing, true));
        assert!(!reapply_wants(&kind("dxvk"), &active, true));
        assert!(!reapply_wants(&kind("dxvk"), &Status::Blocked("no".into()), true));
        // Exe patches and settings come back through the record instead.
        assert!(!reapply_wants(&kind("laa"), &missing, true));
        for c in bl2.setup {
            if matches!(c.kind, ComponentKind::Settings { .. } | ComponentKind::LaunchArg(_)) {
                assert!(!reapply_wants(&c.kind, &missing, true), "{}", c.id);
            }
        }
    }

    fn fake_backup(label: &str, at: &str, files: &[&str]) -> Backup {
        Backup {
            label: label.into(),
            created_at: at.into(),
            files: files.iter().map(|f| backup::BackupEntry { stored_as: String::new(), original: PathBuf::from(f), created: false }).collect(),
            dir: PathBuf::from(at),
        }
    }

    #[test]
    fn a_quick_settings_burst_snapshots_every_file_it_touches() {
        let now = chrono::NaiveDateTime::parse_from_str("2026-09-26 12:03:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let engine = PathBuf::from(r"C:\Cfg\WillowEngine.ini");
        let input = PathBuf::from(r"C:\Cfg\WillowInput.ini");
        let recent = fake_backup(QUICK_LABEL, "2026-09-26 12:01:00", &[r"c:\cfg\willowengine.ini"]);
        assert_eq!(snapshot_plan(Some(&recent), QUICK_LABEL, std::slice::from_ref(&engine), now), SnapshotPlan::Covered);
        assert_eq!(snapshot_plan(Some(&recent), QUICK_LABEL, &[engine.clone(), input.clone()], now), SnapshotPlan::Extend(vec![input.clone()]), "a later edit to another file adds it");
        let old = fake_backup(QUICK_LABEL, "2026-09-26 11:50:00", &[r"C:\Cfg\WillowEngine.ini"]);
        assert_eq!(snapshot_plan(Some(&old), QUICK_LABEL, std::slice::from_ref(&engine), now), SnapshotPlan::New);
        let other = fake_backup("Before applying 2 change(s)", "2026-09-26 12:02:00", &[r"C:\Cfg\WillowEngine.ini"]);
        assert_eq!(snapshot_plan(Some(&other), QUICK_LABEL, std::slice::from_ref(&engine), now), SnapshotPlan::New);
        assert_eq!(snapshot_plan(None, QUICK_LABEL, std::slice::from_ref(&engine), now), SnapshotPlan::New);
    }

    #[test]
    fn exe_snapshots_keep_the_first_and_the_newest() {
        let exe = r"D:\Game\Borderlands2.exe";
        // Newest first, as backup::list returns them.
        let list: Vec<Backup> = (0..7).rev().map(|i| fake_backup("Before applying", &format!("{i}"), &[exe])).chain([fake_backup("config", "x", &[r"C:\Cfg\WillowEngine.ini"])]).collect();
        let doomed: Vec<&str> = exe_backups_to_prune(&list, 3).iter().map(|b| b.created_at.as_str()).collect();
        assert_eq!(doomed, ["3", "2", "1"], "keeps 6, 5, 4 and the original 0");
        assert!(exe_backups_to_prune(&list[..2], 3).is_empty());
    }

    #[test]
    fn factory_settings_agree_on_the_framerate_limit() {
        let def = &crate::games::bl2::GAME;
        assert!(factory_tweaks(def).all(|t| t.category != "quick"));
        let dir = std::env::temp_dir().join("vaulter-factory-fps");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("LauncherConfig")).unwrap();
        std::fs::write(dir.join("WillowEngine.ini"), "[SystemSettings]\r\nFramerateLocking=6\r\n[Engine.Engine]\r\nbSmoothFrameRate=FALSE\r\n").unwrap();
        let mut config = ConfigSet::load(&dir, def.ini_files);
        for t in factory_tweaks(def) {
            t.write(&mut config, &t.default.to_value());
        }
        let target = def.tweak("fps_target").unwrap();
        assert_eq!(target.read(&config), Some(target.default.to_value()), "the quick control reads its own default after a reset");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn launch_mode_falls_back_when_it_cant_work() {
        use LaunchMode::*;
        assert_eq!(usable_launch_mode(None, true, || true), Normal);
        assert_eq!(usable_launch_mode(Some(Direct), true, || false), Direct);
        assert_eq!(usable_launch_mode(Some(Launcher), true, || true), Launcher);
        assert_eq!(usable_launch_mode(Some(Launcher), true, || false), Normal, "launcher exe missing");
        assert_eq!(usable_launch_mode(Some(Launcher), false, || true), Normal, "game has no launcher");
        assert_eq!(usable_launch_mode(Some(Direct), false, || true), Normal);
    }
}
