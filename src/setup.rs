//! One-click setup components (Simple mode). A game lists the components its
//! "Patch" button installs; each knows how to detect, install and remove
//! itself. Components that download run on a background thread.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::core::display::DisplayMode;
use crate::core::manifest::{self, Manifest};
use crate::core::{atomic, backup, binpatch::PatchState, net};
use crate::mods::SdkStatus;
use crate::tweaks::{ConfigSet, DefaultValue};
use crate::workspace::GameState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Essentials,
    Performance,
    Fixes,
    Mods,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Essentials => "Essentials",
            Group::Performance => "Performance",
            Group::Fixes => "Bug fixes",
            Group::Mods => "Mods & quality of life",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DxvkTarget {
    /// 32-bit Direct3D 9 games (BL2, TPS): `x32/d3d9.dll`.
    D3d9Win32,
    /// 64-bit Direct3D 11 games (BL1E): `x64/d3d11.dll` + `x64/dxgi.dll`.
    D3d11Win64,
}

impl DxvkTarget {
    fn dlls(self) -> &'static [&'static str] {
        match self {
            DxvkTarget::D3d9Win32 => &["d3d9.dll"],
            DxvkTarget::D3d11Win64 => &["d3d11.dll", "dxgi.dll"],
        }
    }

    fn archive_dir(self) -> &'static str {
        match self {
            DxvkTarget::D3d9Win32 => "x32",
            DxvkTarget::D3d11Win64 => "x64",
        }
    }
}

/// Writes display-dependent values (native resolution, refresh-rate cap).
pub type DisplayHook = fn(&mut ConfigSet, DisplayMode);

#[derive(Clone, Copy)]
pub enum ComponentKind {
    /// A curated set of tweak values, plus an optional display hook.
    Settings {
        values: &'static [(&'static str, DefaultValue)],
        display: Option<DisplayHook>,
    },
    /// One of the game's `ExePatch`es, by id.
    ExePatch(&'static str),
    /// A launch switch added to the Play button.
    LaunchArg(&'static str),
    /// DXVK from its latest GitHub release, plus a tuned `dxvk.conf`.
    Dxvk {
        target: DxvkTarget,
        /// Folder of the game exe, relative to the install root.
        exe_dir: &'static str,
        conf: &'static str,
    },
    /// The game's Python SDK (see `GameDef::mods`).
    Sdk,
    /// A file downloaded into the install folder (an SDK mod, a text mod, a
    /// proxy DLL fix...). `dest` is relative to the install root.
    File {
        url: &'static str,
        dest: &'static str,
        /// SDK module to switch on after install (new SDK mods start disabled).
        enable: Option<&'static str>,
    },
    /// A zipped SDK mod whose single top-level `folder` goes into `sdk_mods`.
    SdkZip { url: &'static str, folder: &'static str },
    /// A zip extracted into `dest` (relative to the install root), e.g. an
    /// SDK mod zipped without a root folder, or an ASI plugin pack.
    Archive {
        url: &'static str,
        dest: &'static str,
        /// Entries (by top-level name) that aren't installed, e.g. readmes.
        skip: &'static [&'static str],
        /// SDK module to switch on after install.
        enable: Option<&'static str>,
        /// Files in `dest` that other tools also install (a `dxgi.dll`
        /// proxy): never written over someone else's copy.
        exclusive: &'static [&'static str],
    },
    /// A game file moved aside (renamed with `.vp-hidden`), e.g. an in-game
    /// ad. Uninstalling puts it back.
    Hide { path: &'static str },
    /// Text mod sources merged (with every other `TextPatch` component of
    /// the game) into one offline file that Text Mod Loader auto-runs.
    TextPatch {
        /// BLCMM game tag ("BL2" / "TPS").
        game: &'static str,
        /// Gearbox's official hotfixes, which an offline patch must carry.
        gearbox_url: &'static str,
        sources: &'static [TextSource],
    },
}

/// One upstream text mod, downloaded on the user's machine and filtered to
/// the categories we want. Nothing is redistributed by Vaulter.
#[derive(Clone, Copy)]
pub struct TextSource {
    pub title: &'static str,
    pub credit: &'static str,
    pub url: &'static str,
    /// Category paths (`A/B/C`) to keep; empty keeps everything.
    pub include: &'static [&'static str],
    /// Category paths to drop even if included.
    pub exclude: &'static [&'static str],
}

/// Where the merged community patch lives, relative to the install root.
/// Text Mod Loader only scans files directly inside `Binaries`.
pub const TEXT_PATCH_FILE: &str = "Binaries/VaultPatcher.blcm";

#[derive(Clone, Copy)]
pub struct Component {
    pub id: &'static str,
    pub name: &'static str,
    /// One short line shown in lists.
    pub summary: &'static str,
    /// The full explanation, shown when the row is expanded.
    pub description: &'static str,
    pub group: Group,
    /// Pre-selected in the setup list.
    pub recommended: bool,
    pub kind: ComponentKind,
    /// Components that must be installed first (e.g. SDK mods need the SDK).
    pub requires: &'static [&'static str],
}

impl Component {
    /// Install-folder files only one tool can own (`Binaries\Win64\dxgi.dll`),
    /// relative to the install root and lowercased.
    fn slots(&self) -> Vec<String> {
        let join = |dir: &str, f: &str| format!("{dir}\\{f}").to_ascii_lowercase();
        match self.kind {
            ComponentKind::Dxvk { target, exe_dir, .. } => target.dlls().iter().map(|d| join(exe_dir, d)).collect(),
            ComponentKind::Archive { dest, exclusive, .. } => exclusive.iter().map(|f| join(dest, f)).collect(),
            _ => Vec::new(),
        }
    }

    /// Whether the two can't be installed together (they claim the same file).
    pub fn clashes_with(&self, other: &Component) -> bool {
        self.id != other.id && self.slots().iter().any(|s| other.slots().contains(s))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Active(Option<String>),
    Partial,
    Missing,
    /// Can't be installed right now; the string says why.
    Blocked(String),
}

impl Status {
    pub fn is_active(&self) -> bool {
        matches!(self, Status::Active(_))
    }
}

pub fn status(c: &Component, game: &GameState, launch_args: &[String]) -> Status {
    let root = game.install.as_ref().map(|i| i.root.as_path());
    let needs_install = || Status::Blocked("Game install not found".into());
    match c.kind {
        ComponentKind::Settings { .. } => {
            if !game.config_found() {
                return Status::Blocked("Launch the game once to create its config".into());
            }
            // Once applied, a bundle stays "installed" even if the player
            // later fine-tunes the same settings themselves.
            if manifest::read(game.def.id, c.id).is_some() {
                Status::Active(None)
            } else {
                Status::Missing
            }
        }
        ComponentKind::ExePatch(id) => match game.exe.as_ref().and_then(|e| e.patch_states.get(id)) {
            Some(PatchState::Patched) => Status::Active(None),
            Some(PatchState::Unpatched) => Status::Missing,
            Some(PatchState::Unsupported) => Status::Blocked("Unrecognized exe version".into()),
            None if root.is_some_and(|r| r.join(game.def.exe).is_file()) => {
                Status::Blocked("Couldn't read the game exe (is Steam updating it?). Try Refresh in a moment".into())
            }
            None => needs_install(),
        },
        ComponentKind::LaunchArg(arg) => {
            if launch_args.iter().any(|a| a.eq_ignore_ascii_case(arg)) {
                Status::Active(None)
            } else {
                Status::Missing
            }
        }
        ComponentKind::Dxvk { target, exe_dir, .. } => {
            let Some(root) = root else { return needs_install() };
            let dir = root.join(exe_dir);
            let present = target.dlls().iter().all(|d| dir.join(d).is_file());
            match manifest::read(game.def.id, c.id) {
                Some(m) if present => Status::Active(Some(m.version)),
                _ if present => Status::Blocked(format!(
                    "Another {} is already installed (ReShade or a manual DXVK?)",
                    target.dlls()[0]
                )),
                _ => match crate::core::gpu::dxvk_ready() {
                    Ok(()) => Status::Missing,
                    Err(reason) => Status::Blocked(reason),
                },
            }
        }
        ComponentKind::Sdk => match (&game.sdk, root) {
            (_, None) => needs_install(),
            (SdkStatus::Installed(v), _) => Status::Active(Some(v.clone())),
            (SdkStatus::Detected, _) => Status::Active(None),
            (SdkStatus::Legacy, _) => Status::Partial,
            (SdkStatus::NotInstalled, _) => Status::Missing,
        },
        ComponentKind::SdkZip { folder, .. } => {
            let Some(root) = root else { return needs_install() };
            if present_or_parked(root, &root.join("sdk_mods").join(folder), Path::is_dir) {
                Status::Active(None)
            } else {
                Status::Missing
            }
        }
        ComponentKind::Archive { dest, exclusive, .. } => {
            let Some(root) = root else { return needs_install() };
            match manifest::read(game.def.id, c.id) {
                Some(m)
                    if m.files.iter().all(|f| present_or_parked(root, f, Path::is_file))
                        && present_or_parked(root, &root.join(dest), Path::exists) =>
                {
                    Status::Active(None)
                }
                Some(_) => Status::Partial,
                None => match exclusive.iter().find(|f| root.join(dest).join(f).exists()) {
                    Some(f) => Status::Blocked(format!("Another {f} is already installed (DXVK or ReShade?)")),
                    None => Status::Missing,
                },
            }
        }
        ComponentKind::Hide { path } => {
            let Some(root) = root else { return needs_install() };
            let file = root.join(path);
            if hidden_twin(&file).is_file() && !file.exists() {
                Status::Active(None)
            } else if file.is_file() {
                Status::Missing
            } else {
                Status::Blocked("File not found in this version of the game".into())
            }
        }
        ComponentKind::TextPatch { .. } => {
            let Some(root) = root else { return needs_install() };
            let included = text_patch_parts(game.def.id);
            if included.iter().any(|p| p == c.id) && present_or_parked(root, &root.join(TEXT_PATCH_FILE), Path::is_file) {
                Status::Active(None)
            } else {
                Status::Missing
            }
        }
        ComponentKind::File { dest, .. } => {
            let Some(root) = root else { return needs_install() };
            let path = root.join(dest);
            if present_or_parked(root, &path, Path::is_file) {
                Status::Active(None)
            } else {
                Status::Missing
            }
        }
    }
}

/// Whether Vaulter itself installed/applied `c` (so "Restore vanilla"
/// may undo it). Things the player installed by hand are left alone.
pub fn installed_by_us(c: &Component, game: &GameState, launch_args: &[String]) -> bool {
    match c.kind {
        ComponentKind::Settings { .. }
        | ComponentKind::Dxvk { .. }
        | ComponentKind::File { .. }
        | ComponentKind::SdkZip { .. }
        | ComponentKind::Archive { .. }
        | ComponentKind::Hide { .. } => manifest::read(game.def.id, c.id).is_some(),
        ComponentKind::Sdk => manifest::read(game.def.id, "sdk").is_some(),
        ComponentKind::TextPatch { .. } => text_patch_parts(game.def.id).iter().any(|p| p == c.id),
        ComponentKind::LaunchArg(arg) => launch_args.iter().any(|a| a.eq_ignore_ascii_case(arg)),
        // An exe patch can't be told apart from how the game shipped, so
        // only the re-apply record says whether we applied it.
        ComponentKind::ExePatch(id) => game.applied.patches.contains(id),
    }
}

/// Remembers that a settings bundle was applied.
pub fn record_settings_applied(game_id: &str, component: &str) -> Result<()> {
    manifest::write(game_id, component, &Manifest { version: "applied".into(), files: Vec::new(), replaced: None })
}

/// `path` is there, or the user disabled it on the Mods page (which moves it
/// aside, see `mods::disabled_twin`). Either way it's installed.
fn present_or_parked(root: &Path, path: &Path, is: impl Fn(&Path) -> bool) -> bool {
    is(path) || crate::mods::disabled_twin(root, path).is_some_and(|p| is(&p))
}

/// Downloads the latest DXVK, installs its DLLs next to the exe and writes
/// `dxvk.conf`. Returns the installed version.
pub fn install_dxvk(game_id: &str, component: &str, root: &Path, target: DxvkTarget, exe_dir: &str, conf: &str) -> Result<String> {
    let asset = net::latest_asset("doitsujin/dxvk", |n| {
        n.starts_with("dxvk-") && !n.contains("native") && n.ends_with(".tar.gz")
    })?;
    let tmp = atomic::temp_file(&asset.name);
    net::download(&asset.url, &tmp)?;

    let dest_dir = root.join(exe_dir);
    let mut targets: Vec<PathBuf> = target.dlls().iter().map(|d| dest_dir.join(d)).collect();
    targets.push(dest_dir.join("dxvk.conf"));
    let replaced = manifest::plan_replace(game_id, component, "Before installing DXVK", &targets)?;
    // Recorded before extraction, so a half-finished install can still be removed.
    manifest::write(
        game_id,
        component,
        &Manifest { version: asset.tag.clone(), files: targets.clone(), replaced: replaced.clone() },
    )?;

    let mut wanted: Vec<(String, PathBuf)> = target
        .dlls()
        .iter()
        .map(|d| (format!("/{}/{d}", target.archive_dir()), dest_dir.join(d)))
        .collect();
    let archive = flate2::read::GzDecoder::new(fs::File::open(&tmp)?);
    let mut tar = tar::Archive::new(archive);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().replace('\\', "/");
        if let Some(i) = wanted.iter().position(|(suffix, _)| name.ends_with(suffix.as_str())) {
            let (_, out) = wanted.remove(i);
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            backup::clear_readonly(&out);
            atomic::write(&out, &bytes).with_context(|| format!("writing {} (close the game first)", out.display()))?;
        }
    }
    fs::remove_file(&tmp).ok();
    if !wanted.is_empty() {
        bail!("DXVK archive layout changed: missing {}", wanted[0].0);
    }
    atomic::write(&dest_dir.join("dxvk.conf"), conf.as_bytes())?;
    Ok(asset.tag)
}

/// Downloads a single file into the install folder, backing up anything it
/// replaces.
pub fn install_file(
    game_id: &str,
    component: &str,
    root: &Path,
    url: &str,
    dest: &str,
    enable: Option<&str>,
) -> Result<String> {
    let out = root.join(dest);
    let replaced =
        manifest::plan_replace(game_id, component, &format!("Before installing {component}"), std::slice::from_ref(&out))?;
    manifest::write(
        game_id,
        component,
        &Manifest { version: "latest".into(), files: vec![out.clone()], replaced },
    )?;
    // Downloaded outside the game folder, so a failed download leaves no
    // stray file there, then swapped in whole.
    let tmp = atomic::temp_file("download");
    let fetched = net::download(url, &tmp).and_then(|()| Ok(fs::read(&tmp)?));
    fs::remove_file(&tmp).ok();
    atomic::write(&out, &fetched?).with_context(|| format!("close the game first ({})", out.display()))?;
    if let Some(module) = enable {
        enable_sdk_module(root, module)?;
    }
    Ok("latest".into())
}

/// Marks an SDK mod enabled via `sdk_mods/settings/<module>.json`, the file
/// mods_base reads at startup. An existing settings file (the user's own
/// choices) is left alone.
fn enable_sdk_module(root: &Path, module: &str) -> Result<()> {
    let settings = root.join("sdk_mods").join("settings").join(format!("{module}.json"));
    if !settings.exists() {
        fs::create_dir_all(settings.parent().expect("settings path has a parent"))?;
        fs::write(settings, "{\n    \"enabled\": true\n}\n")?;
    }
    Ok(())
}

fn hidden_twin(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".vp-hidden");
    path.with_file_name(name)
}

/// Moves a game file aside (reversibly).
pub fn hide_file(game_id: &str, component: &str, root: &Path, path: &str) -> Result<String> {
    let file = root.join(path);
    let hidden = hidden_twin(&file);
    if !file.is_file() {
        bail!("{} not found", file.display());
    }
    manifest::write(game_id, component, &Manifest { version: "hidden".into(), files: Vec::new(), replaced: None })?;
    backup::clear_readonly(&file);
    fs::rename(&file, &hidden).with_context(|| format!("moving {} aside (close the game first)", file.display()))?;
    Ok("Done".into())
}

/// Puts a hidden game file back.
pub fn unhide_file(game_id: &str, component: &str, root: &Path, path: &str) -> Result<()> {
    let file = root.join(path);
    let hidden = hidden_twin(&file);
    if hidden.is_file() && !file.exists() {
        fs::rename(&hidden, &file).with_context(|| format!("restoring {}", file.display()))?;
    }
    manifest::remove(game_id, component);
    Ok(())
}

/// Downloads an `Archive` component's zip and extracts it into its `dest`,
/// recording every file so uninstall removes exactly what was added (and
/// restores anything replaced).
pub fn install_archive(game_id: &str, c: &Component, root: &Path) -> Result<String> {
    let ComponentKind::Archive { url, dest, skip, enable, exclusive } = c.kind else {
        bail!("{} isn't an archive", c.id)
    };
    let component = c.id;
    let ours = manifest::read(game_id, component).map(|m| m.files).unwrap_or_default();
    for f in exclusive {
        let path = root.join(dest).join(f);
        if path.exists() && !ours.contains(&path) {
            bail!("Another {f} is already installed (DXVK or ReShade?). Remove it first");
        }
    }
    let tmp = atomic::temp_file(&format!("{component}.zip"));
    net::download(url, &tmp)?;
    let mut archive = zip::ZipArchive::new(fs::File::open(&tmp)?).context("not a valid zip")?;
    let dest_dir = root.join(dest);
    let wanted = |name: &str| {
        !name.split('/').any(|p| p == ".." || p.contains(':'))
            && !name.starts_with('/')
            && !skip.iter().any(|s| name.split('/').next().is_some_and(|top| top.eq_ignore_ascii_case(s)))
    };
    let names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().filter(|e| !e.is_dir()).map(|e| e.name().replace('\\', "/")))
        .filter(|n| wanted(n))
        .collect();
    if names.is_empty() {
        bail!("{url} contains nothing to install");
    }
    let targets: Vec<PathBuf> = names.iter().map(|n| dest_dir.join(n)).collect();
    let replaced = manifest::plan_replace(game_id, component, &format!("Before installing {component}"), &targets)?;
    manifest::write(game_id, component, &Manifest { version: "latest".into(), files: targets.clone(), replaced })?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().replace('\\', "/");
        if entry.is_dir() || !wanted(&name) {
            continue;
        }
        let out = dest_dir.join(&name);
        fs::create_dir_all(out.parent().expect("zip entry has a parent"))?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        backup::clear_readonly(&out);
        atomic::write(&out, &bytes).with_context(|| format!("writing {} (close the game first)", out.display()))?;
    }
    fs::remove_file(&tmp).ok();
    if let Some(module) = enable {
        enable_sdk_module(root, module)?;
    }
    Ok("latest".into())
}

/// Downloads a zipped SDK mod and extracts its folder into `sdk_mods`.
pub fn install_sdk_zip(game_id: &str, component: &str, root: &Path, url: &str, folder: &str) -> Result<String> {
    let tmp = atomic::temp_file(&format!("{component}.zip"));
    net::download(url, &tmp)?;
    let mut archive = zip::ZipArchive::new(fs::File::open(&tmp)?).context("not a valid zip")?;
    let sdk_mods = root.join("sdk_mods");
    let wanted = |name: &str| name.starts_with(&format!("{folder}/")) && !name.split('/').any(|p| p == "..");
    let names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().filter(|e| !e.is_dir()).map(|e| e.name().replace('\\', "/")))
        .filter(|n| wanted(n))
        .collect();
    let targets: Vec<PathBuf> = names.iter().map(|n| sdk_mods.join(n)).collect();
    let replaced = manifest::plan_replace(game_id, component, &format!("Before installing {component}"), &targets)?;
    manifest::write(
        game_id,
        component,
        &Manifest { version: "latest".into(), files: names.iter().map(|n| sdk_mods.join(n)).collect(), replaced },
    )?;
    let mut written = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().replace('\\', "/");
        if entry.is_dir() || !wanted(&name) {
            continue;
        }
        let out = sdk_mods.join(&name);
        fs::create_dir_all(out.parent().expect("zip entry has a parent"))?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        atomic::write(&out, &bytes).with_context(|| format!("writing {} (close the game first)", out.display()))?;
        written.push(out);
    }
    fs::remove_file(&tmp).ok();
    if written.is_empty() {
        manifest::remove(game_id, component);
        bail!("{url} doesn't contain a {folder}/ folder");
    }
    Ok("latest".into())
}

// ---- merged text patch -----------------------------------------------------------

fn parts_path(game_id: &str) -> PathBuf {
    backup::data_dir().join("installs").join(format!("{game_id}-textpatch.json"))
}

/// Ids of the `TextPatch` components currently merged into the game's file.
pub fn text_patch_parts(game_id: &str) -> Vec<String> {
    fs::read_to_string(parts_path(game_id))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn download_text(url: &str) -> Result<String> {
    let tmp = atomic::temp_file("textmod.txt");
    net::download(url, &tmp)?;
    let bytes = fs::read(&tmp)?;
    fs::remove_file(&tmp).ok();
    Ok(crate::textmod::decode(&bytes))
}

/// Rebuilds `Binaries/VaultPatcher.blcm` from the given parts (component id
/// + sources) and points Text Mod Loader at it. With no parts, removes it.
pub fn rebuild_text_patch(
    game_id: &str,
    root: &Path,
    game_tag: &str,
    gearbox_url: &str,
    parts: &[(&str, &[TextSource])],
) -> Result<String> {
    use crate::textmod::{self, MergeInfo, Source};
    let out = root.join(TEXT_PATCH_FILE);
    if parts.is_empty() {
        // Also the copy the user disabled on the Mods page, if any.
        for file in [Some(out.clone()), crate::mods::disabled_twin(root, &out)].into_iter().flatten() {
            if file.is_file() {
                backup::clear_readonly(&file);
                fs::remove_file(&file)?;
            }
        }
        set_tml_auto_enable(root, &out, false)?;
        fs::remove_file(parts_path(game_id)).ok();
        return Ok("Removed".into());
    }
    let gearbox = textmod::parse(&download_text(gearbox_url)?);
    let mut sources = Vec::new();
    for (_, list) in parts {
        for s in list.iter() {
            let tree = textmod::parse(&download_text(s.url)?);
            let nodes = textmod::filter(&tree, s.include, s.exclude);
            if nodes.is_empty() {
                bail!("{} changed upstream: none of the expected categories were found", s.title);
            }
            sources.push(Source { title: s.title.into(), credit: s.credit.into(), nodes });
        }
    }
    let text = textmod::merge(
        &MergeInfo {
            game: game_tag,
            title: "Vaulter Community Patch",
            author: "Vaulter (built from community mods)",
            version: env!("CARGO_PKG_VERSION"),
            description: "Balance-neutral bug fixes and quality of life, merged into one file so Text Mod Loader can run them together.",
        },
        &gearbox,
        &sources,
    );
    if out.exists() {
        backup::create(game_id, "Before rebuilding the community patch", std::slice::from_ref(&out))?;
    }
    atomic::write(&out, &textmod::encode(&text)).with_context(|| format!("writing {}", out.display()))?;
    set_tml_auto_enable(root, &out, true)?;
    let ids: Vec<&str> = parts.iter().map(|(id, _)| *id).collect();
    fs::create_dir_all(parts_path(game_id).parent().expect("has parent"))?;
    atomic::write(&parts_path(game_id), &serde_json::to_vec(&ids)?)?;
    Ok(format!("{} mods merged", sources.len()))
}

/// Adds (or removes) `file` in Text Mod Loader's auto-enable list, keeping
/// every other setting the user has.
fn set_tml_auto_enable(root: &Path, file: &Path, enabled: bool) -> Result<()> {
    let settings = root.join("sdk_mods").join("settings").join("text_mod_loader.json");
    let mut json: serde_json::Value = match fs::read_to_string(&settings) {
        Ok(text) => serde_json::from_str(&text).with_context(|| {
            format!("{} isn't valid JSON; fix or delete it so its settings aren't lost", settings.display())
        })?,
        Err(_) => serde_json::json!({}),
    };
    if !json.is_object() {
        json = serde_json::json!({});
    }
    let options = json
        .as_object_mut()
        .expect("object")
        .entry("options")
        .or_insert_with(|| serde_json::json!({"mod_info": {}, "version": 2}));
    let list = options
        .as_object_mut()
        .context("text_mod_loader.json: options isn't an object")?
        .entry("auto_enable")
        .or_insert_with(|| serde_json::json!([]));
    let path = file.to_string_lossy().to_string();
    let arr = list.as_array_mut().context("auto_enable isn't a list")?;
    arr.retain(|v| v.as_str().is_none_or(|s| !s.eq_ignore_ascii_case(&path)));
    if enabled {
        arr.push(serde_json::Value::String(path));
    }
    fs::create_dir_all(settings.parent().expect("has parent"))?;
    atomic::write(&settings, &serde_json::to_vec_pretty(&json)?)?;
    Ok(())
}

/// Removes a component's recorded files, including copies the user
/// disabled on the Mods page, and puts back anything it replaced.
pub fn uninstall_files(game_id: &str, component: &str, root: &Path) -> Result<()> {
    manifest::uninstall_with(game_id, component, root, |f| crate::mods::disabled_twin(root, f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luma_and_dxvk_clash() {
        let game = &crate::games::bl1e::GAME;
        let (luma, dxvk) = (game.component("luma").unwrap(), game.component("dxvk").unwrap());
        assert!(luma.clashes_with(dxvk) && dxvk.clashes_with(luma));
        assert!(!luma.clashes_with(game.component("ultrawide").unwrap()));
        // BL2's DXVK (d3d9) clashes with nothing in its own bundle.
        let bl2 = &crate::games::bl2::GAME;
        let d3d9 = bl2.component("dxvk").unwrap();
        assert!(bl2.setup.iter().all(|c| !d3d9.clashes_with(c)));
    }

    #[test]
    fn component_ids_are_unique_and_dependencies_exist() {
        for game in crate::games::all() {
            let mut seen = std::collections::HashSet::new();
            for (i, c) in game.setup.iter().enumerate() {
                assert!(seen.insert(c.id), "{}: duplicate component {}", game.id, c.id);
                for dep in c.requires {
                    let at = game.setup.iter().position(|d| d.id == *dep);
                    assert!(at.is_some_and(|at| at < i), "{}: {} needs {dep} listed before it", game.id, c.id);
                }
                if let ComponentKind::Settings { values, .. } = c.kind {
                    for (id, _) in values {
                        assert!(game.tweaks.iter().any(|t| t.id == *id), "{}: {} references {id}", game.id, c.id);
                    }
                }
                if let ComponentKind::ExePatch(p) = c.kind {
                    assert!(game.patches.iter().any(|x| x.id == p), "{}: unknown patch {p}", game.id);
                }
            }
        }
    }
}

/// Network tests that install into a temp folder; run with `--ignored`.
#[cfg(test)]
mod live_tests {
    use super::*;

    #[test]
    #[ignore]
    fn live_every_download_url_resolves() {
        for game in crate::games::all() {
            for c in game.setup {
                let url = match c.kind {
                    ComponentKind::File { url, .. } | ComponentKind::SdkZip { url, .. } | ComponentKind::Archive { url, .. } => Some(url),
                    _ => None,
                };
                if let Some(url) = url {
                    let status = ureq::head(url).set("User-Agent", "Vaulter-test").call().map(|r| r.status());
                    println!("{:<5} {:<16} {:?}", game.id, c.id, status);
                    assert_eq!(status.ok(), Some(200), "{} {}", game.id, c.id);
                }
            }
        }
    }

    /// Builds the real community patch (downloads) into a sandbox and checks
    /// it is well formed and excludes the balance changes.
    #[test]
    #[ignore]
    fn live_community_patch_builds() {
        let text = build_live_patch(&crate::games::bl2::GAME);
        assert!(!text.contains("Permaslag"));
        assert!(text.contains("Fixed Moonshiner Audio"));
    }

    /// The Pre-Sequel's patch: its fixes and neutral features only.
    #[test]
    #[ignore]
    fn live_tps_community_patch_builds() {
        let text = build_live_patch(&crate::games::tps::GAME);
        assert!(text.contains("bAutomaticallyPickup True"), "Moonstone auto-pickup");
        assert!(text.contains("bSendOnly False"), "two-way fast travel");
        assert!(!text.contains("Running UCP"), "no Badass Rank branding");
        assert!(!text.contains("BalanceMod_PT3"), "no UVHM balance changes");
    }

    /// Builds every text patch of `game` (downloads) into a sandbox, checks
    /// it is well formed, removes it again and returns its text.
    fn build_live_patch(game: &crate::games::GameDef) -> String {
        let id = format!("sandbox-{}", game.id);
        let root = std::env::temp_dir().join(format!("vaulter-textpatch-{}", game.id));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Binaries")).unwrap();
        let parts: Vec<(&str, &[TextSource])> = game
            .setup
            .iter()
            .filter_map(|c| match c.kind {
                ComponentKind::TextPatch { sources, .. } => Some((c.id, sources)),
                _ => None,
            })
            .collect();
        let ComponentKind::TextPatch { game: tag, gearbox_url, .. } = game.component("community_patch").unwrap().kind else {
            panic!()
        };
        let msg = rebuild_text_patch(&id, &root, tag, gearbox_url, &parts).unwrap();
        println!("{msg}");
        let text = crate::textmod::decode(&fs::read(root.join(TEXT_PATCH_FILE)).unwrap());
        assert!(text.contains(&format!("<type name=\"{tag}\" offline=\"true\"/>")));
        let keys = text.lines().find(|l| l.contains("SparkServiceConfiguration_0 Keys")).unwrap();
        let values = text.lines().find(|l| l.contains("SparkServiceConfiguration_0 Values")).unwrap();
        let n_keys = keys.matches("-BLCMM").count();
        println!("{} commands, {n_keys} hotfixes, {} bytes", text.split("#Commands:").nth(1).unwrap().lines().filter(|l| l.starts_with("set")).count(), text.len());
        assert!(n_keys > 20);
        // Values are a quoted list with escaped quotes inside; count top-level entries.
        let mut entries = 0;
        let (mut in_str, mut escaped) = (false, false);
        for ch in values.split_once('(').unwrap().1.chars() {
            match (in_str, escaped, ch) {
                (true, true, _) => escaped = false,
                (true, false, '\\') => escaped = true,
                (true, false, '"') => in_str = false,
                (false, _, '"') => { in_str = true; entries += 1; }
                _ => {}
            }
        }
        assert_eq!(entries, n_keys, "Keys and Values must line up");
        let tml = fs::read_to_string(root.join("sdk_mods/settings/text_mod_loader.json")).unwrap();
        assert!(tml.contains("VaultPatcher.blcm"));
        assert_eq!(text_patch_parts(&id).len(), parts.len());
        rebuild_text_patch(&id, &root, tag, gearbox_url, &[]).unwrap();
        assert!(!root.join(TEXT_PATCH_FILE).exists());
        let _ = fs::remove_dir_all(&root);
        text
    }

    #[test]
    #[ignore]
    fn live_sdk_mod_installs_and_is_enabled() {
        let root = std::env::temp_dir().join("vaulter-mod-sandbox");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        install_file(
            "sandbox",
            "firing_fix",
            &root,
            "https://github.com/ZetaDaemon/willow2-sdk-mods/releases/download/nightly/firing_fix.sdkmod",
            "sdk_mods/firing_fix.sdkmod",
            Some("firing_fix"),
        )
        .unwrap();
        let archive = zip::ZipArchive::new(fs::File::open(root.join("sdk_mods/firing_fix.sdkmod")).unwrap()).unwrap();
        assert!(archive.file_names().any(|n| n.starts_with("firing_fix/")), "sdkmod root folder must match its stem");
        let settings = fs::read_to_string(root.join("sdk_mods/settings/firing_fix.json")).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&settings).unwrap()["enabled"] == true);
        uninstall_files("sandbox", "firing_fix", &root).unwrap();
        assert!(!root.join("sdk_mods/firing_fix.sdkmod").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore]
    fn live_dxvk_installs_into_sandbox_and_uninstalls() {
        let root = std::env::temp_dir().join("vaulter-dxvk-sandbox");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Binaries/Win32")).unwrap();
        let version = install_dxvk("sandbox", "dxvk", &root, DxvkTarget::D3d9Win32, "Binaries/Win32", "x = 1\n").unwrap();
        println!("installed DXVK {version}");
        let dll = root.join("Binaries/Win32/d3d9.dll");
        let bytes = fs::read(&dll).unwrap();
        assert!(bytes.len() > 100_000 && &bytes[..2] == b"MZ");
        // 32-bit PE: machine type i386 (0x14C)
        let pe = u32::from_le_bytes(bytes[0x3C..0x40].try_into().unwrap()) as usize;
        assert_eq!(u16::from_le_bytes([bytes[pe + 4], bytes[pe + 5]]), 0x14C);
        assert!(root.join("Binaries/Win32/dxvk.conf").is_file());
        uninstall_files("sandbox", "dxvk", &root).unwrap();
        assert!(!dll.exists());
        let _ = fs::remove_dir_all(&root);
    }

    /// Installs every downloadable BL1E component into a temp folder shaped
    /// like the game, checks the layout, then uninstalls and checks it's clean.
    #[test]
    #[ignore]
    fn live_bl1e_components_install_and_uninstall() {
        let game = &crate::games::bl1e::GAME;
        let root = std::env::temp_dir().join("vaulter-bl1e-install-sandbox");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Binaries/Win64")).unwrap();
        let upk = root.join("WillowGame/CookedPC/Packages/Interface/ui_frontend_upsell_PC.upk");
        fs::create_dir_all(upk.parent().unwrap()).unwrap();
        fs::write(&upk, b"upk").unwrap();
        let id = "sandbox-bl1e";

        let version = crate::mods::install_sdk_latest(id, game.mods.unwrap(), &root).unwrap();
        println!("SDK {version}");
        assert!(root.join("Binaries/Win64/dinput8.dll").is_file(), "SDK loader");
        assert!(root.join("Binaries/Win64/Plugins/unrealsdk.dll").is_file());
        assert!(root.join("sdk_mods").is_dir());

        for c in game.setup {
            let result = match c.kind {
                ComponentKind::File { url, dest, enable } => install_file(id, c.id, &root, url, dest, enable),
                ComponentKind::Archive { .. } => install_archive(id, c, &root),
                ComponentKind::Hide { path } => hide_file(id, c.id, &root, path),
                ComponentKind::Dxvk { target, exe_dir, conf } => install_dxvk(id, c.id, &root, target, exe_dir, conf),
                _ => continue,
            };
            println!("{:<18} {:?}", c.id, result);
            if c.id == "luma" {
                assert!(result.is_err(), "Luma must not overwrite DXVK's dxgi.dll");
                continue;
            }
            result.unwrap();
            if let ComponentKind::File { dest, enable: Some(module), .. } = c.kind {
                // The module name we enable must be the .sdkmod's root folder.
                let archive = zip::ZipArchive::new(fs::File::open(root.join(dest)).unwrap()).unwrap();
                assert!(archive.file_names().any(|n| n.starts_with(&format!("{module}/"))), "{} root folder isn't {module}", c.id);
            }
            if let ComponentKind::File { enable: Some(module), .. } | ComponentKind::Archive { enable: Some(module), .. } = c.kind {
                assert!(root.join(format!("sdk_mods/settings/{module}.json")).is_file(), "{} not enabled", c.id);
            }
        }
        assert!(root.join("sdk_mods/BloodwingReturnFix/__init__.py").is_file());
        assert!(root.join("Binaries/Win64/winmm.dll").is_file() && root.join("Binaries/Win64/scripts/BorderlandsGOTYEnhancedFix.asi").is_file());
        assert!(!root.join("Binaries/Win64/README.txt").exists());
        assert!(!upk.exists() && upk.with_file_name("ui_frontend_upsell_PC.upk.vp-hidden").is_file());
        let dxgi = fs::read(root.join("Binaries/Win64/dxgi.dll")).unwrap();
        let pe = u32::from_le_bytes(dxgi[0x3C..0x40].try_into().unwrap()) as usize;
        assert_eq!(u16::from_le_bytes([dxgi[pe + 4], dxgi[pe + 5]]), 0x8664, "DXVK must be 64-bit for BL1E");

        for c in game.setup.iter().filter(|c| c.id != "luma") {
            match c.kind {
                ComponentKind::File { .. } | ComponentKind::Archive { .. } | ComponentKind::Dxvk { .. } => uninstall_files(id, c.id, &root).unwrap(),
                ComponentKind::Hide { path } => unhide_file(id, c.id, &root, path).unwrap(),
                _ => {}
            }
        }
        assert!(upk.is_file(), "ad restored");
        for gone in ["Binaries/Win64/version.dll", "Binaries/Win64/winmm.dll", "Binaries/Win64/dxgi.dll", "Binaries/Win64/d3d11.dll", "sdk_mods/BloodwingReturnFix/__init__.py", "sdk_mods/AutopickupBL1E.sdkmod"] {
            assert!(!root.join(gone).exists(), "{gone} left behind");
        }
        // With DXVK gone, Luma installs into the same slot and comes out cleanly.
        install_archive(id, game.component("luma").unwrap(), &root).unwrap();
        for f in ["Binaries/Win64/dxgi.dll", "Binaries/Win64/Luma-Borderlands GOTY Enhanced.addon", "Binaries/Win64/Luma/d3dcompiler_47.dll"] {
            assert!(root.join(f).is_file(), "{f} missing");
        }
        uninstall_files(id, "luma", &root).unwrap();
        assert!(!root.join("Binaries/Win64/dxgi.dll").exists() && !root.join("Binaries/Win64/Luma/d3dcompiler_47.dll").exists());
        crate::mods::uninstall_sdk(id, game.mods.unwrap(), &root).unwrap();
        assert!(!root.join("Binaries/Win64/dinput8.dll").exists(), "SDK loader left behind");
        let _ = fs::remove_dir_all(&root);
    }
}
