//! Game registry. Each supported title is a static `GameDef`; the rest of the
//! app is generic over it. Adding a game means adding a module here and
//! listing it in `all()`.

pub mod bl1e;
pub mod bl2;
pub mod tps;
pub mod willow;

use std::path::PathBuf;

use crate::core::detect::DetectSpec;
use crate::patches::ExePatch;
use crate::theme::Icon;
use crate::tweaks::{Category, Preset, RangePair, Tweak};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// Tweaks, patches and mods are all wired up.
    Full,
    /// Detected and tweakable, but some pages are not available yet.
    /// Unused while every listed game is Full; new titles start here.
    #[allow(dead_code)]
    Preview,
}

/// A page kind the shell knows how to build. Games choose which ones they
/// show and in what order through `GameDef::nav`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageKind {
    Overview,
    /// A tweak page showing the listed tweak categories.
    Tweaks(&'static str),
    Presets,
    Patches,
    Mods,
    Backups,
    Settings,
    /// Simple mode: the one-click setup page.
    Setup,
    /// Simple mode: a short list of friendly settings.
    Quick,
    /// Advanced: capture comparison screenshots on this PC.
    Capture,
}

/// Simple or Advanced presentation of the whole app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    #[default]
    Simple,
    Advanced,
}

/// A group of tweaks on the Simple-mode Quick Settings page.
#[derive(Clone, Copy)]
pub struct QuickSection {
    pub title: &'static str,
    pub blurb: &'static str,
    pub tweaks: &'static [&'static str],
    /// Show the one-click quality buttons (the game's presets) above the tweaks.
    pub quality_presets: &'static [&'static str],
}

#[derive(Clone, Copy)]
pub struct NavItem {
    pub kind: PageKind,
    pub title: &'static str,
    pub icon: Icon,
    /// For `Tweaks` pages: the categories (in order) the page shows.
    pub categories: &'static [&'static str],
}

#[derive(Clone, Copy)]
pub struct NavGroup {
    pub title: &'static str,
    pub items: &'static [NavItem],
}

/// Information about the game's Python SDK / mod manager support.
pub struct ModSupport {
    /// GitHub `owner/repo` whose latest release provides the mod manager zip.
    pub sdk_repo: &'static str,
    /// Substring identifying the right release asset.
    pub sdk_asset_hint: &'static str,
    pub sdk_name: &'static str,
    /// Files whose presence (relative to the install root) means the SDK is installed.
    pub sdk_markers: &'static [&'static str],
    /// Folder (relative to the install root) where `.sdkmod`/python mods live.
    pub sdk_mods_dir: &'static str,
    /// Folder (relative to the install root) where text mods (.blcm/.txt) live.
    pub text_mods_dir: &'static str,
    /// Mods bundled with the SDK itself; shown as locked "core" entries.
    pub core_mods: &'static [&'static str],
    /// Files that indicate the pre-2024 legacy PythonSDK.
    pub legacy_markers: &'static [&'static str],
    /// Visual C++ runtime the SDK needs.
    pub redist_url: &'static str,
    pub links: &'static [(&'static str, &'static str)],
}

pub struct GameDef {
    pub id: &'static str,
    pub name: &'static str,
    pub short: &'static str,
    pub tagline: &'static str,
    pub support: Support,
    pub steam_app_ids: &'static [u32],
    pub epic_names: &'static [&'static str],
    /// Executable path relative to the install root.
    pub exe: &'static str,
    /// A separate first-run menu the game ships with (BL2/TPS's Launcher.exe).
    /// It rewrites its own copy of the video keys on every run.
    pub launcher: Option<&'static LauncherDef>,
    /// Config folder relative to `Documents\My Games`.
    pub config_subdir: &'static str,
    /// Logical file id -> file name inside the config folder.
    pub ini_files: &'static [(&'static str, &'static str)],
    pub categories: &'static [Category],
    pub tweaks: &'static [Tweak],
    /// Shared-catalog tweaks that don't apply to (or are unsafe for) this game.
    pub hidden_tweaks: &'static [&'static str],
    /// Slider pairs bounding one range (min ≤ max), shown as one control.
    pub ranges: &'static [RangePair],
    pub presets: &'static [Preset],
    pub patches: &'static [ExePatch],
    pub mods: Option<&'static ModSupport>,
    pub launch_args: &'static [LaunchArg],
    /// Advanced-mode navigation.
    pub nav: &'static [NavGroup],
    /// Simple-mode navigation.
    pub simple_nav: &'static [NavGroup],
    pub quick: &'static [QuickSection],
    /// What the Simple-mode "Patch" button can install.
    pub setup: &'static [crate::setup::Component],
    /// Settings with comparison images or links.
    pub comparisons: &'static [crate::compare::Comparison],
    /// How the comparison capture tool runs this game (None: not supported).
    pub capture: Option<&'static crate::compare::CaptureProfile>,
}

/// The separate launcher program a game ships with, sitting next to the
/// game exe.
#[derive(Clone, Copy)]
pub struct LauncherDef {
    /// Launcher exe, relative to the install root.
    pub exe: &'static str,
    /// The game-exe switch that skips the launcher for one run.
    pub skip_arg: &'static str,
}

/// A command-line switch offered on the Launch page.
#[derive(Clone, Copy)]
pub struct LaunchArg {
    pub arg: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub default_on: bool,
}

impl GameDef {
    pub fn detect_spec(&self) -> DetectSpec<'_> {
        DetectSpec {
            steam_app_ids: self.steam_app_ids,
            epic_names: self.epic_names,
            exe: self.exe,
        }
    }

    pub fn default_config_dir(&self) -> Option<PathBuf> {
        let docs = dirs::document_dir()?;
        Some(docs.join("My Games").join(self.config_subdir))
    }

    /// Tweaks this game shows, in catalog order.
    pub fn visible_tweaks(&self) -> impl Iterator<Item = &'static Tweak> + '_ {
        self.tweaks.iter().filter(|t| !self.hidden_tweaks.contains(&t.id))
    }

    pub fn tweak(&self, id: &str) -> Option<&'static Tweak> {
        self.visible_tweaks().find(|t| t.id == id)
    }

    pub fn category(&self, id: &str) -> Option<&'static Category> {
        self.categories.iter().find(|c| c.id == id)
    }

    /// The range `id` is one end of, if any.
    pub fn range_of(&self, id: &str) -> Option<&'static RangePair> {
        self.ranges.iter().find(|r| r.min == id || r.max == id)
    }

    pub fn nav_for(&self, mode: Mode) -> &'static [NavGroup] {
        match mode {
            Mode::Simple => self.simple_nav,
            Mode::Advanced => self.nav,
        }
    }

    pub fn nav_items(&self, mode: Mode) -> impl Iterator<Item = &'static NavItem> {
        self.nav_for(mode).iter().flat_map(|g| g.items.iter())
    }

    pub fn comparison(&self, tweak: &str) -> Option<&'static crate::compare::Comparison> {
        self.comparisons.iter().find(|c| c.tweak == tweak)
    }

    pub fn component(&self, id: &str) -> Option<&'static crate::setup::Component> {
        self.setup.iter().find(|c| c.id == id)
    }
}

static ALL: [&GameDef; 3] = [&bl2::GAME, &tps::GAME, &bl1e::GAME];

pub fn all() -> &'static [&'static GameDef] {
    &ALL
}

/// Simple-mode navigation used by every game.
pub const SIMPLE_NAV: &[NavGroup] = &[
    NavGroup {
        title: "Get started",
        items: &[
            NavItem {
                kind: PageKind::Setup,
                title: "One-click setup",
                icon: Icon::Bolt,
                categories: &[],
            },
            NavItem {
                kind: PageKind::Quick,
                title: "Quick settings",
                icon: Icon::Sliders,
                categories: &[],
            },
        ],
    },
    COMMON_NAV,
];

/// Navigation shared by every game after its own pages.
pub const COMMON_NAV: NavGroup = NavGroup {
    title: "Maintenance",
    items: &[
        NavItem {
            kind: PageKind::Backups,
            title: "Backups",
            icon: Icon::History,
            categories: &[],
        },
        NavItem {
            kind: PageKind::Settings,
            title: "App settings",
            icon: Icon::Settings,
            categories: &[],
        },
    ],
};

#[cfg(test)]
mod slider_data_tests {
    use crate::tweaks::Control;

    /// Sentinel words and recommended bands sit inside each slider's range,
    /// and range pairs join two sliders of one category.
    #[test]
    fn slider_extras_fit_their_sliders() {
        for game in super::all() {
            for tweak in game.visible_tweaks() {
                let Control::Slider { min, max, labels, recommended, .. } = tweak.control else { continue };
                for (v, label) in labels {
                    assert!((min..=max).contains(v), "{}/{}: {label} = {v} is out of range", game.id, tweak.id);
                }
                if let Some((lo, hi)) = recommended {
                    assert!(min <= lo && lo < hi && hi <= max, "{}/{}: recommended {lo}–{hi}", game.id, tweak.id);
                }
            }
            for pair in game.ranges {
                let (lo, hi) = (game.tweak(pair.min).expect("range min"), game.tweak(pair.max).expect("range max"));
                assert!(matches!(lo.control, Control::Slider { .. }) && matches!(hi.control, Control::Slider { .. }));
                assert_eq!(lo.category, hi.category, "{}: {} and {} should share a page", game.id, pair.min, pair.max);
                assert_eq!(game.range_of(pair.max).map(|p| p.min), Some(pair.min));
            }
        }
        // The sentinels the pages rely on.
        let bl2 = &super::bl2::GAME;
        let word = |id: &str, n: f64| bl2.tweak(id).and_then(|t| t.control.slider_label(n));
        assert_eq!(word("particle_cap", 0.0), Some("Unlimited"));
        assert_eq!(word("physx_heap", 0.0), Some("Off"));
        assert_eq!(word("mesh_lod", -1.0), Some("Force high"));
    }
}

#[cfg(test)]
mod ordering_tests {
    use crate::tweaks::Control;

    fn leading_number(label: &str) -> Option<f64> {
        let digits: String = label.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        digits.parse().ok()
    }

    /// Every choice reads lowest → highest, left → right.
    #[test]
    fn choices_run_from_lowest_to_highest() {
        for game in super::all() {
            for tweak in game.visible_tweaks() {
                let Control::Choice(options) = tweak.control else { continue };
                let labels: Vec<&str> = options.iter().map(|o| o.label).collect();
                let at = |l: &str| labels.iter().position(|x| x.eq_ignore_ascii_case(l));
                let ctx = format!("{} / {}: {labels:?}", game.id, tweak.id);

                // Named quality scales.
                let scale: Vec<usize> = ["Low", "Medium", "High", "Ultra High"].iter().filter_map(|l| at(l)).collect();
                assert!(scale.windows(2).all(|w| w[0] < w[1]), "quality scale out of order in {ctx}");

                // Numeric options ascend; "Off" leads and "Unlimited" trails.
                let numbers: Vec<f64> = labels.iter().filter_map(|l| leading_number(l)).collect();
                if numbers.len() >= 2 {
                    assert!(numbers.windows(2).all(|w| w[0] <= w[1]), "numbers not ascending in {ctx}");
                    if let Some(off) = at("Off") {
                        assert_eq!(off, 0, "Off should be leftmost in {ctx}");
                    }
                    if let Some(unlimited) = at("Unlimited") {
                        assert_eq!(unlimited, labels.len() - 1, "Unlimited should be rightmost in {ctx}");
                    }
                }
            }
        }
    }
}
