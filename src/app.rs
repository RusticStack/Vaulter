//! The main window, laid out like a Windows 11 app: a Mica title bar (back
//! button, app identity, caption buttons), a NavigationView-style pane (game
//! picker, mode switch, pages, Play), the active page on the content layer,
//! the Apply bar, status line, toasts, and the overlays (comparison viewer,
//! change review, first-run welcome).

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, Corner, ElementId, Entity, ExternalPaths, FontWeight, IntoElement,
    MouseButton, NavigationDirection, ObjectFit, ParentElement, Render, SharedString, Styled, Subscription, Window,
    WindowControlArea, div, img, prelude::*, px,
};
use gpui_component::Disableable as _;
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use crate::games::{Mode, NavItem, PageKind};
use crate::pages;
use crate::theme::{self, Icon};
use crate::ui::{self, Variant};
use crate::workspace::{LaunchMode, ToastAction, ToastKind, Workspace};

/// Title bar height: the tall variant, which fits the back button.
const TITLE_BAR: f32 = 48.;
/// Open navigation pane width.
const PANE: f32 = 280.;

pub struct Shell {
    ws: Entity<Workspace>,
    /// The page area, rendered as a cached view.
    page_host: Entity<PageHost>,
    /// The status bar's busy spinner, while something runs in the background.
    spinner: Option<Entity<BusySpinner>>,
    focus: gpui::FocusHandle,
    /// Pages visited in this game and mode, for the back button.
    history: Vec<PageKind>,
    /// The page on screen at the last render, to notice navigation.
    shown: Option<(usize, Mode, PageKind)>,
    /// Set while going back so the page left isn't pushed again.
    going_back: bool,
    /// The user's pane choice from the menu button; None follows the window
    /// width (compact below 1008px, like NavigationView).
    pane_open: Option<bool>,
    /// The window is narrower than NavigationView's 1008px breakpoint: the
    /// pane is a compact rail and opens as an overlay.
    narrow: bool,
    /// One focus handle per nav item, so arrow keys can move focus along
    /// the pane (it's a single Tab stop).
    nav_focus: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<PageKind, gpui::FocusHandle>>>,
    /// The pointer is over the Play button (its stripes march).
    play_hovered: bool,
    _subs: Vec<Subscription>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ws = cx.new(Workspace::new);
        let page_host = cx.new(|cx| PageHost::new(ws.clone(), cx));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search settings (Ctrl+F)"));
        let profile = cx.new(|cx| InputState::new(window, cx).placeholder("Profile name"));
        let search_ws = ws.clone();
        // Closing mid-capture or mid-install would leave the game half-changed.
        let close_ws = ws.clone();
        window.on_window_should_close(cx, move |_, cx| close_ws.update(cx, |ws, cx| ws.allow_close(cx)));
        let subs = vec![
            cx.observe(&ws, |_, _, cx| cx.notify()),
            cx.subscribe(&search, move |_, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    search_ws.update(cx, |ws, cx| ws.set_search(text, cx));
                }
            }),
            // Game-running background mode: ends when the window comes back,
            // and the window comes back when the game exits. Activation also
            // picks up an accent color changed in Windows Settings meanwhile.
            cx.observe_window_activation(window, |this, window, cx| {
                let active = window.is_window_active();
                if active && theme::sync() {
                    theme::apply(cx);
                    this.page_host.update(cx, |_, cx| cx.notify());
                }
                this.ws.update(cx, |ws, cx| ws.window_activation_changed(active, cx));
                // Out of focus for a couple of seconds: hand the physical
                // memory we aren't using back to Windows for other apps.
                if !active {
                    cx.spawn_in(window, async move |_, cx| {
                        cx.background_executor().timer(Duration::from_secs(2)).await;
                        if cx.update(|window, _| !window.is_window_active()).unwrap_or(false) {
                            crate::win11::trim_memory();
                        }
                    })
                    .detach();
                }
                cx.notify();
            }),
            // Windows switched between light and dark.
            cx.observe_window_appearance(window, |this, _, cx| {
                if theme::sync() {
                    theme::apply(cx);
                    this.page_host.update(cx, |_, cx| cx.notify());
                }
                cx.notify();
            }),
            cx.subscribe_in(&ws, window, |_, _, _: &crate::workspace::RunEvent, window, _| window.activate_window()),
        ];
        ws.update(cx, |ws, _| {
            ws.search_input = Some(search);
            ws.profile_input = Some(profile);
        });
        Self { ws, focus: cx.focus_handle(), history: Vec::new(), shown: None, going_back: false, pane_open: None, narrow: false, nav_focus: Default::default(), play_hovered: false, page_host, spinner: None, _subs: subs }
    }

    /// Tracks page changes for the back button. Switching game or mode starts
    /// a fresh history, like entering a new top-level section.
    fn track_navigation(&mut self, cx: &Context<Self>) {
        let state = self.ws.read(cx);
        let now = (state.active, state.mode(), state.page);
        if let Some(prev) = self.shown
            && prev != now
        {
            if prev.0 != now.0 || prev.1 != now.1 {
                self.history.clear();
            } else if !self.going_back {
                self.history.push(prev.2);
                // Keep it short; nobody goes back fifty pages.
                if self.history.len() > 32 {
                    self.history.remove(0);
                }
            }
        }
        if self.shown.is_some_and(|prev| prev != now) && self.narrow && self.pane_open == Some(true) {
            self.pane_open = None;
        }
        self.going_back = false;
        self.shown = Some(now);
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.history.pop() {
            self.going_back = true;
            self.ws.update(cx, |ws, cx| ws.navigate(page, cx));
        }
    }

    /// App-wide keys. Esc closes overlays, ←/→ flip the viewer's right side,
    /// Alt+← goes back, Ctrl+S applies, Ctrl+F jumps to search.
    fn on_key(&mut self, ev: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        let ctrl = ev.keystroke.modifiers.control;
        if key == "escape" && self.narrow && self.pane_open == Some(true) {
            self.pane_open = None;
            cx.notify();
            return;
        }
        if ev.keystroke.modifiers.alt && key == "left" {
            self.go_back(cx);
            return;
        }
        if ctrl && key == "f" {
            let input = self.ws.read(cx).search_input.clone();
            let page = self.ws.read(cx).page;
            if !matches!(page, PageKind::Tweaks(_) | PageKind::Quick) {
                let first = self.ws.read(cx).game().def.nav_items(self.ws.read(cx).mode()).find(|n| matches!(n.kind, PageKind::Tweaks(_) | PageKind::Quick)).map(|n| n.kind);
                if let Some(kind) = first {
                    self.ws.update(cx, |ws, cx| ws.navigate(kind, cx));
                }
            }
            if let Some(input) = input {
                input.update(cx, |i, cx| i.focus(window, cx));
            }
            return;
        }
        self.ws.update(cx, |ws, cx| {
            if key == "escape" && ws.review_open {
                ws.set_review(false, cx);
                return;
            }
            if let Some(preview) = ws.preview {
                let count = ws
                    .game()
                    .def
                    .tweak(preview.tweak)
                    .map(|t| crate::compare::images(ws.game().def.id, t).len())
                    .unwrap_or(0)
                    .max(1);
                match key {
                    "escape" => ws.close_preview(cx),
                    "right" => ws.set_preview(|p| p.right = (p.right + 1) % count, cx),
                    "left" => ws.set_preview(|p| p.right = (p.right + count - 1) % count, cx),
                    _ => {}
                }
                return;
            }
            if ctrl && key == "s" && ws.pending_count() > 0 && ws.mode() == Mode::Advanced {
                ws.apply_pending(cx);
            }
        });
    }

    // ---- title bar ---------------------------------------------------------------

    /// Tall title bar over Mica. Everything that isn't a button is a drag
    /// area; the caption buttons are real Windows hit-test regions, so Snap
    /// Layouts, double-click-to-maximize and dragging all behave natively.
    /// Drag areas are siblings of the buttons, never their ancestors: gpui
    /// resolves nested control areas outermost-first.
    fn title_bar(&self, compact: bool, window: &Window, cx: &Context<Self>) -> impl IntoElement {
        let state = self.ws.read(cx);
        let maximized = window.is_maximized();
        let active = window.is_window_active();
        let update = state.updates.app.clone();
        let can_go_back = !self.history.is_empty();
        let back = cx.listener(|this, _, _, cx| this.go_back(cx));
        let drag = |id: &'static str| div().id(id).h_full().window_control_area(WindowControlArea::Drag);
        div()
            .h(px(TITLE_BAR))
            .flex_none()
            .flex()
            .items_center()
            .child(div().w(px(4.)).flex_none())
            .child(
                ui::focusable(div().id("back"))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(40.))
                    .h(px(36.))
                    .rounded(px(theme::RADIUS))
                    .when(can_go_back, |d| d.hover(|s| s.bg(theme::panel_hi())).active(|s| s.bg(theme::panel_pressed())))
                    .child(ui::icon(Icon::Back).text_color(if can_go_back { theme::text() } else { theme::text_disabled() }))
                    .tooltip(ui::tip("Back (Alt+Left)"))
                    .on_click(back),
            )
            .child(
                ui::focusable(div().id("pane-toggle"))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(40.))
                    .h(px(36.))
                    .rounded(px(theme::RADIUS))
                    .hover(|s| s.bg(theme::panel_hi()))
                    .active(|s| s.bg(theme::panel_pressed()))
                    .child(ui::icon(Icon::Hamburger).text_color(theme::text()))
                    .tooltip(ui::tip(if compact { "Open navigation" } else { "Close navigation" }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.pane_open = Some(compact);
                        cx.notify();
                    })),
            )
            .child(
                // The logo takes a 40px cell like the buttons before it, so
                // the three sit on one rhythm; the name is Body Strong so it
                // holds its own next to a 20px mark.
                drag("identity")
                    .flex_none()
                    .flex()
                    .items_center()
                    .pr(px(12.))
                    .child(div().w(px(40.)).flex_none().flex().justify_center().child(app_mark(20.)))
                    .child(
                        div()
                            .ml(px(2.))
                            .text_size(px(14.))
                            .line_height(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if active { theme::text() } else { theme::text_muted() })
                            .child("Vaulter"),
                    ),
            )
            .child(drag("drag").flex_1())
            .when_some(update, |d, (tag, page)| {
                d.child(
                    div()
                        .id("update")
                        .mr(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(32.))
                        .px(px(12.))
                        .rounded(px(theme::RADIUS))
                        .bg(theme::accent())
                        .text_color(theme::accent_ink())
                        .text_size(px(12.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::accent_hi()))
                        .child(ui::icon(Icon::Download).size(px(14.)).text_color(theme::accent_ink()))
                        .child(format!("Update to {tag}"))
                        .tooltip(ui::tip("A newer Vaulter is available. Opens the download page."))
                        .on_click(move |_, _, cx| cx.open_url(&page)),
                )
            })
            .child(caption_button("min", Icon::Minimize, WindowControlArea::Min, active))
            .child(caption_button("max", if maximized { Icon::Restore } else { Icon::Maximize }, WindowControlArea::Max, active))
            .child(caption_button("close", Icon::WindowClose, WindowControlArea::Close, active))
    }

    // ---- navigation pane ---------------------------------------------------------

    /// Current game (its own icon when found) with a menu to switch, shown as
    /// the pane's header.
    fn game_switcher(&self, compact: bool, cx: &Context<Self>) -> impl IntoElement {
        let state = self.ws.read(cx);
        let game = state.game();
        type Entry = (usize, &'static str, &'static str, Option<std::path::PathBuf>, &'static str, bool);
        let games: Vec<Entry> = state
            .games
            .iter()
            .enumerate()
            .map(|(i, g)| (i, g.def.name, g.def.short, g.art.icon.clone(), status_text(g), i == state.active))
            .collect();
        let ws = self.ws.clone();
        let button = Button::new("game-switcher").ghost();
        let button = if compact {
            button.w(px(40.)).h(px(40.)).tooltip(game.def.name).child(game_icon(game.art.icon.clone(), game.def.short, 24.))
        } else {
            button.w_full().h(px(56.)).child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(game_icon(game.art.icon.clone(), game.def.short, 32.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .items_start()
                            .child(div().w_full().truncate().text_size(px(14.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text()).child(game.def.name))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .child(div().size(px(6.)).rounded_full().bg(status_color(game)))
                                    .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_muted()).child(status_text(game))),
                            ),
                    )
                    .child(ui::icon(Icon::ChevronDown).size(px(12.)).text_color(theme::text_muted())),
            )
        };
        button.dropdown_menu(move |mut menu, _, _| {
                for (i, name, short, icon, status, active) in games.clone() {
                    let ws = ws.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, _| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .py(px(4.))
                                .child(game_icon(icon.clone(), short, 24.))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .child(div().text_size(px(14.)).child(name))
                                        .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(status)),
                                )
                        })
                        .checked(active)
                        .on_click(move |_, _, cx| ws.update(cx, |ws, cx| ws.select_game(i, cx))),
                    );
                }
                let ws = ws.clone();
                menu.separator()
                    .item(
                        PopupMenuItem::element(|_, _| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .py(px(4.))
                                .child(div().w(px(24.)).flex().justify_center().child(ui::icon(Icon::Refresh).text_color(theme::text())))
                                .child(div().text_size(px(14.)).child("Search for games again"))
                        })
                        .on_click(move |_, _, cx| ws.update(cx, |ws, cx| ws.search_again(cx))),
                    )
                    .min_w(px(PANE - 16.))
            })
    }

    /// Simple / Advanced switch at the top of the pane; a single icon button
    /// that flips the mode when the pane is compact.
    fn mode_toggle(&self, compact: bool, cx: &Context<Self>) -> AnyElement {
        let mode = self.ws.read(cx).mode();
        let tip = |m: Mode| match m {
            Mode::Simple => "Simple: one-click setup and a few friendly settings, saved as you go",
            Mode::Advanced => "Advanced: every setting, presets, exe patches and the mod manager",
        };
        if compact {
            let other = if mode == Mode::Simple { Mode::Advanced } else { Mode::Simple };
            let ws = self.ws.clone();
            return ui::icon_button("mode-flip", if mode == Mode::Simple { Icon::Wand } else { Icon::Sliders }, theme::text())
                .w(px(40.))
                .h(px(36.))
                .tooltip(ui::tip(if other == Mode::Simple { "Switch to Simple" } else { "Switch to Advanced" }))
                .on_click(move |_, window, cx| switch_mode(&ws, other, window, cx))
                .into_any_element();
        }
        let (wrap, segments) = ui::segmented(vec![
            ("mode-simple".into(), "Simple".into(), mode == Mode::Simple),
            ("mode-advanced".into(), "Advanced".into(), mode == Mode::Advanced),
        ]);
        let mut wrap = wrap.w_full();
        for (seg, m) in segments.into_iter().zip([Mode::Simple, Mode::Advanced]) {
            let ws = self.ws.clone();
            wrap = wrap.child(seg.flex_1().justify_center().tooltip(ui::tip(tip(m))).on_click(move |_, window, cx| switch_mode(&ws, m, window, cx)));
        }
        wrap.into_any_element()
    }

    /// The navigation pane: `compact` is the 48px icon rail; `scroll_id`
    /// keeps each pane variant's scroll position apart.
    fn pane(&self, compact: bool, scroll_id: &'static str, cx: &Context<Self>) -> AnyElement {
        let state = self.ws.read(cx);
        let game = state.game();
        let mut nav = div().flex().flex_col().pb(px(8.));
        // App settings is pinned to the pane's footer, like Windows apps do.
        let mut settings_item = None;
        for (i, group) in game.def.nav_for(state.mode()).iter().enumerate() {
            let items: Vec<&'static NavItem> = group.items.iter().filter(|n| n.kind != PageKind::Settings).collect();
            settings_item = settings_item.or(group.items.iter().find(|n| n.kind == PageKind::Settings));
            if items.is_empty() {
                continue;
            }
            nav = if compact {
                // Compact panes separate groups with a line instead of headers.
                nav.when(i > 0, |d| d.child(ui::divider().mx(px(12.)).my(px(8.))))
            } else {
                nav.child(ui::label(group.title).px(px(16.)).pt(px(16.)).pb(px(4.)))
            };
            for item in items {
                nav = nav.child(self.nav_entry(item, compact, cx));
            }
        }
        let footer_settings = settings_item.map(|item| self.nav_entry(item, compact, cx));

        let play_ws = self.ws.clone();
        let can_play = game.install.is_some();
        let running = game.is_running();
        let short = game.def.short;
        // This game ships a separate launcher (BL2/TPS): the menu offers ways
        // around it. "Through the launcher" additionally needs the file there.
        let launcher_known = game.def.launcher.is_some();
        let launcher_exists = game.launcher_path().is_some();
        let menu_ws = self.ws.clone();
        let reapply_ws = self.ws.clone();
        let current_mode = state.launch_mode();
        let menu_row = |icon: Icon, label: SharedString, detail: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .py(px(4.))
                .child(ui::icon(icon).text_color(theme::text()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(14.)).line_height(px(20.)).child(label))
                        .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_muted()).child(detail)),
                )
        };
        // Clicking an entry makes it the Play button's way of launching (the
        // checkmark) and starts the game that way right now, if it can.
        let launch_item = move |label: String, detail: &'static str, icon: Icon, mode: LaunchMode| {
            let ws = menu_ws.clone();
            let label: SharedString = label.into();
            PopupMenuItem::element(move |_, _| menu_row(icon, label.clone(), detail))
                .checked(mode == current_mode)
                .on_click(move |_, _, cx| {
                    ws.update(cx, |ws, cx| {
                        ws.set_launch_mode(mode, cx);
                        if can_play && !running {
                            ws.launch(mode, cx);
                        }
                    })
                })
        };
        let tip = if can_play {
            match current_mode {
                LaunchMode::Normal => "Start the game with your launch options",
                LaunchMode::Direct => "Start the game exe directly, skipping the launcher",
                LaunchMode::Launcher => "Start the game through its own launcher",
            }
        } else {
            "Game install not found"
        };
        let playable = can_play && !running;
        if compact {
            let (fill_from, fill_to) = theme::play_fill();
        let on_fill = theme::ink_on(&[fill_from, fill_to]);
        let ink = move |a: f32| theme::with_alpha(if playable { on_fill } else { theme::accent_ink_disabled() }, a);
            let play = ui::focusable(div().id("rail-play"))
                .size(px(40.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(theme::RADIUS_LG))
                .when(playable, |d| {
                    d.bg(gpui::linear_gradient(
                        135.,
                        gpui::linear_color_stop(fill_from, 0.),
                        gpui::linear_color_stop(fill_to, 1.),
                    ))
                    .border_1()
                    .border_color(theme::with_alpha(theme::accent_pressed(), 0.6))
                    .shadow(theme::shadow_card())
                    .cursor_pointer()
                    .on_click(move |_, _, cx| play_ws.update(cx, |ws, cx| ws.launch_default(cx)))
                })
                .when(!playable, |d| d.bg(theme::accent_disabled()))
                .tooltip(ui::tip(if running { "Running" } else { tip }))
                .child(ui::icon(if running { Icon::Check } else { Icon::PlaySolid }).size(px(16.)).text_color(ink(1.)));
            return div()
                .w(px(48.))
                .flex_none()
                .h_full()
                .flex()
                .flex_col()
                .items_center()
                .child(div().pb(px(4.)).child(self.game_switcher(true, cx)))
                .child(div().id(scroll_id).w_full().flex_1().min_h_0().overflow_y_scroll().child(nav))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(4.))
                        .pt(px(4.))
                        .pb(px(12.))
                        .child(ui::divider().w(px(32.)).mb(px(4.)))
                        .children(footer_settings)
                        .child(self.mode_toggle(true, cx))
                        .child(play),
                )
                .into_any_element();
        }
        // The launch button: bigger than any other control and the one place
        // with a gradient and texture, since it's what the app builds up to.
        // A SplitButton underneath: Play on the left, ways to start on the right.
        let (fill_from, fill_to) = theme::play_fill();
        let on_fill = theme::ink_on(&[fill_from, fill_to]);
        let ink = move |a: f32| theme::with_alpha(if playable { on_fill } else { theme::accent_ink_disabled() }, a);
        let subtitle: SharedString = if !game.loaded {
            "Looking for the game\u{2026}".into()
        } else if running {
            "Game is running".into()
        } else if !can_play {
            "Install not found".into()
        } else {
            match current_mode {
                LaunchMode::Normal => "With your launch options".into(),
                LaunchMode::Direct => "Skipping the launcher".into(),
                LaunchMode::Launcher => "Through the game launcher".into(),
            }
        };
        let hovered = self.play_hovered && playable;
        // The stripes march while the pointer is on the button.
        // Across the whole button, clipped by its rounded edge: the pattern
        // repeats every period, so a loop ends exactly where it began.
        let stripes_at = move |shift: f32| div().absolute().inset_0().overflow_hidden().child(ui::stripes(ink(0.08), shift));
        let stripes = if hovered && theme::motion() {
            div()
                .absolute()
                .inset_0()
                .with_animation("play-stripes", Animation::new(Duration::from_millis(900)).repeat(), move |d, t| d.child(stripes_at(t)))
                .into_any_element()
        } else {
            stripes_at(0.).into_any_element()
        };
        let main = ui::focusable(div().id("rail-play"))
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .items_center()
            .gap(px(12.))
            .pl(px(12.))
            .pr(px(8.))
            .overflow_hidden()
            .rounded_l(px(theme::RADIUS_LG))
            .when(playable, |d| d.cursor_pointer().hover(|s| s.bg(ink(0.08))).active(|s| s.bg(ink(0.16))))
            .tooltip(ui::tip(if running { "Running" } else { tip }))
            .when(playable, |d| d.on_click(move |_, _, cx| play_ws.update(cx, |ws, cx| ws.launch_default(cx))))
            .child(stripes)
            .child(
                div()
                    .size(px(32.))
                    .flex_none()
                    .rounded_full()
                    .bg(ink(0.16))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(ui::icon(if running { Icon::Check } else { Icon::PlaySolid }).size(px(14.)).text_color(ink(1.))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(16.))
                            .line_height(px(20.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ink(1.))
                            .child(if running { "Running".to_string() } else { format!("Play {short}") }),
                    )
                    .child(div().text_size(px(12.)).line_height(px(16.)).text_color(ink(1.)).truncate().child(subtitle)),
            );
        let chevron = Button::new("rail-play-menu")
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(gpui::transparent_black())
                    .foreground(ink(1.))
                    .border(gpui::transparent_black())
                    .hover(ink(0.08))
                    .active(ink(0.16)),
            )
            .disabled(!playable)
            .h(px(54.))
            .w(px(40.))
            .flex_none()
            .rounded_l(px(0.))
            .rounded_r(px(theme::RADIUS_LG))
            .tooltip("More ways to start the game")
            .child(ui::icon(Icon::ChevronDown).size(px(12.)).text_color(ink(1.)))
                    .dropdown_menu_with_anchor(Corner::BottomLeft, move |mut menu, _, _| {
                        menu = menu.item(launch_item(format!("Play {short}"), "With your launch options", Icon::Play, LaunchMode::Normal));
                        if launcher_known {
                            menu = menu.item(launch_item(
                                "Skip the launcher".into(),
                                "The game exe directly, so nothing rewrites your settings",
                                Icon::Bolt,
                                LaunchMode::Direct,
                            ));
                        }
                        if launcher_exists {
                            menu = menu.item(launch_item(
                                "Through the game launcher".into(),
                                "The game's own menu. It may re-apply its video settings",
                                Icon::Game,
                                LaunchMode::Launcher,
                            ));
                        }
                        let reapply_ws = reapply_ws.clone();
                        menu.separator()
                            .item(
                                PopupMenuItem::element(move |_, _| {
                                    menu_row(Icon::Refresh, "Re-apply settings & upgrades".into(), "The game or its launcher reset things? Put them all back")
                                })
                                .on_click(move |_, _, cx| reapply_ws.update(cx, |ws, cx| ws.reapply_all(cx))),
                            )
                            .min_w(px(320.))
                    });
        let play = div()
            .id("play-launch")
            .flex()
            .items_center()
            .w_full()
            .h(px(56.))
            .rounded(px(theme::RADIUS_LG))
            .when(playable, |d| {
                d.bg(gpui::linear_gradient(
                    100.,
                    gpui::linear_color_stop(fill_from, 0.),
                    gpui::linear_color_stop(fill_to, 1.),
                ))
                .border_1()
                .border_color(theme::with_alpha(theme::accent_pressed(), 0.6))
                .shadow(theme::shadow_card())
            })
            .when(!playable, |d| d.bg(theme::accent_disabled()))
            .relative()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.play_hovered = *hovered;
                cx.notify();
            }))
            .child(main)
            .child(div().w(px(1.)).h(px(28.)).bg(ink(0.2)))
            .child(chevron)
            // While hovered, a light circles the button's edge: one lap every
            // 2.4 s at a constant speed, so there's no restart to see.
            .when(hovered && theme::motion(), |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(-1.))
                        .left(px(-1.))
                        .right(px(-1.))
                        .bottom(px(-1.))
                        .with_animation("play-orbit", Animation::new(Duration::from_millis(2400)).repeat(), move |d, t| {
                            d.child(ui::orbit(gpui::white(), t, theme::RADIUS_LG))
                        }),
                )
            });

        div()
            .w(px(PANE))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .child(div().flex().flex_col().gap(px(8.)).px(px(8.)).pb(px(4.)).child(self.game_switcher(false, cx)).child(div().px(px(4.)).child(self.mode_toggle(false, cx))))
            .child(div().id(scroll_id).flex_1().min_h_0().overflow_y_scroll().child(nav))
            .child(div().pt(px(4.)).child(ui::divider().mx(px(12.)).mb(px(4.))).children(footer_settings))
            .child(div().px(px(12.)).pt(px(8.)).pb(px(12.)).child(play))
            .into_any_element()
    }

    /// A NavigationViewItem: 36px tall, 40px icon column, accent selection pill.
    fn nav_entry(&self, item: &'static NavItem, compact: bool, cx: &Context<Self>) -> impl IntoElement {
        let state = self.ws.read(cx);
        let game = state.game();
        let active = state.page == item.kind;
        let waiting = game
            .pending
            .keys()
            .filter(|id| game.def.tweak(id).is_some_and(|t| item.categories.contains(&t.category)))
            .count();
        let ws = self.ws.clone();
        let kind = item.kind;
        let id = SharedString::from(format!("nav-{}", item.title));
        // The pill settles into a newly selected item from its center, on
        // WinUI's 400 ms entrance spline.
        let pill = div().absolute().left_0().w(px(3.)).rounded(px(2.)).bg(theme::accent());
        let pill = if theme::motion() {
            pill.with_animation(
                ElementId::Name(format!("{id}-pill-{}-{:?}", state.active, state.mode()).into()),
                Animation::new(Duration::from_millis(400)).with_easing(ui::ease_entrance()),
                |d, t| d.top(px(18. - 8. * t)).h(px(16. * t)),
            )
            .into_any_element()
        } else {
            pill.top(px(10.)).h(px(16.)).into_any_element()
        };
        // The pane is one Tab stop (the selected item); arrow keys move along it.
        let focus = self.nav_focus.borrow_mut().entry(kind).or_insert_with(|| cx.focus_handle()).clone();
        let order: Vec<PageKind> = game.def.nav_items(state.mode()).map(|n| n.kind).collect();
        let key_ws = self.ws.clone();
        let key_focus = self.nav_focus.clone();
        div()
            .id(ElementId::Name(id))
            .track_focus(&focus.tab_index(0).tab_stop(active))
            .focus_visible(|s| s.outline(px(2.), theme::focus_stroke(), px(1.), Some(theme::focus_stroke_inner().into())))
            .on_key_down(move |ev, window, cx| {
                let at = order.iter().position(|k| *k == kind).unwrap_or(0);
                let next = match ev.keystroke.key.as_str() {
                    "up" => at.checked_sub(1),
                    "down" => (at + 1 < order.len()).then_some(at + 1),
                    "home" => Some(0),
                    "end" => order.len().checked_sub(1),
                    _ => return,
                };
                cx.stop_propagation();
                if let Some(next) = next.map(|i| order[i]).filter(|k| *k != kind) {
                    key_ws.update(cx, |ws, cx| ws.navigate(next, cx));
                    if let Some(handle) = key_focus.borrow().get(&next) {
                        window.focus(handle);
                    }
                }
            })
            .relative()
            .mx(px(4.))
            .my(px(2.))
            .flex()
            .items_center()
            .h(px(36.))
            .when(!compact, |d| d.pr(px(12.)))
            .rounded(px(theme::RADIUS))
            .when(active, |d| d.bg(theme::selected()))
            .hover(|s| s.bg(theme::panel_hi()))
            .active(|s| s.bg(theme::panel_pressed()))
            .when(active, |d| d.child(pill))
            .child(div().w(px(40.)).flex_none().flex().justify_center().child(ui::icon(item.icon).text_color(theme::text())))
            .when(compact, |d| {
                // Icon only: the title moves to a tooltip, the count to a dot.
                d.tooltip(ui::tip(item.title)).when(waiting > 0, |d| {
                    d.child(div().absolute().top(px(6.)).left(px(26.)).size(px(8.)).rounded_full().bg(theme::accent()))
                })
            })
            .when(!compact, |d| d.child(div().flex_1().min_w_0().truncate().text_size(px(14.)).line_height(px(20.)).text_color(theme::text()).child(item.title)))
            .when(waiting > 0 && !compact, |d| {
                // InfoBadge with a count.
                d.child(
                    div()
                        .min_w(px(16.))
                        .h(px(16.))
                        .px(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme::accent())
                        .text_color(theme::accent_ink())
                        .text_size(px(11.))
                        .child(waiting.to_string()),
                )
            })
            .on_click(move |_, window, cx| {
                // A new page starts with a clear search.
                if let Some(input) = ws.read(cx).search_input.clone() {
                    input.update(cx, |i, cx| i.set_value("", window, cx));
                }
                ws.update(cx, |ws, cx| {
                    ws.set_search(String::new(), cx);
                    ws.navigate(kind, cx)
                })
            })
    }

    // ---- bottom bars --------------------------------------------------------------

    fn pending_bar(ws: &Entity<Workspace>, cx: &gpui::App) -> Option<AnyElement> {
        let state = ws.read(cx);
        let game = state.game();
        // Simple mode saves automatically, so there's nothing to confirm.
        if game.pending.is_empty() || state.mode() == Mode::Simple {
            return None;
        }
        let names: Vec<&str> = game.pending.keys().filter_map(|id| game.def.tweak(id).map(|t| t.label)).take(4).collect();
        let more = game.pending.len().saturating_sub(names.len());
        let review_ws = ws.clone();
        let discard_ws = ws.clone();
        let apply_ws = ws.clone();
        let bar = div()
            .flex_none()
            .h(px(64.))
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(24.))
            // Opaque, so the list it sits under never shows through.
            .bg(theme::dialog())
            .border_t_1()
            .border_color(theme::card_stroke())
            .child(ui::icon(Icon::Info).text_color(theme::accent()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(px(4.))
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(14.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text()).child(format!("{} waiting", crate::pages::tweaks::plural(game.pending.len(), "change"))))
                    .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_muted()).truncate().child(format!(
                        "{}{}",
                        names.join(" · "),
                        if more > 0 { format!(" · +{more} more") } else { String::new() }
                    ))),
            )
            .child(
                ui::button("review", "Review", Some(Icon::Search), Variant::Ghost)
                    .tooltip(ui::tip("See every change before it's written"))
                    .on_click(move |_, _, cx| review_ws.update(cx, |ws, cx| ws.set_review(true, cx))),
            )
            .child(ui::button("discard", "Discard", None, Variant::Secondary).on_click(move |_, _, cx| discard_ws.update(cx, |ws, cx| ws.discard_pending(cx))))
            .child(
                ui::button("apply", "Apply", Some(Icon::Check), Variant::Primary)
                    .tooltip(ui::tip("Write the changes (Ctrl+S). A backup is made first."))
                    .on_click(move |_, _, cx| apply_ws.update(cx, |ws, cx| ws.apply_pending(cx))),
            );
        // Rises from the bottom edge when changes first appear.
        Some(if theme::motion() {
            div()
                .flex_none()
                .relative()
                .child(bar)
                .with_animation("pending-bar", Animation::new(theme::NORMAL).with_easing(ui::ease_decelerate()), |d, t| d.top(px(24. * (1. - t))).opacity(t))
                .into_any_element()
        } else {
            bar.into_any_element()
        })
    }

    /// A quiet status line over Mica: install, SDK, upgrades, sounds, version.
    fn status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        use crate::mods::SdkStatus;
        let state = self.ws.read(cx);
        let game = state.game();
        let where_ = game
            .install
            .as_ref()
            .map(|i| format!("{} · {}", i.store.label(), i.root.display()))
            .unwrap_or_else(|| "Game not found".into());
        let sdk = match &game.sdk {
            SdkStatus::Installed(v) => format!("SDK {v}"),
            SdkStatus::Detected => "SDK installed".into(),
            SdkStatus::Legacy => "Legacy SDK".into(),
            SdkStatus::NotInstalled => "No SDK".into(),
        };
        let active = game.def.setup.iter().filter(|c| state.component_status(c).is_active()).count();
        let muted = state.settings.sound_muted;
        let music = state.settings.music;
        let sounds = crate::sound::available();
        let mute_ws = self.ws.clone();
        let music_ws = self.ws.clone();

        let item = |text: String| div().flex_none().text_size(px(12.)).line_height(px(16.)).text_color(theme::text_muted()).child(text);
        let sep = || div().w(px(1.)).h(px(12.)).bg(theme::line());
        let toggle = |id: &'static str, glyph: Icon, on: bool, tip: &'static str| {
            ui::icon_button_if(sounds, id, glyph, if on { theme::text() } else { theme::text_muted() }).size(px(28.)).tooltip(ui::tip(tip))
        };

        div()
            .h(px(32.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            .child(div().size(px(6.)).flex_none().rounded_full().bg(status_color(game)))
            .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(theme::text_muted()).child(where_))
            .when_some(state.busy.clone(), |d, b| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .children(self.spinner.clone().map(|s| gpui::AnyView::from(s).cached(gpui::StyleRefinement::default().size(px(14.)).flex_none())))
                        .child(item(b).text_color(theme::text())),
                )
                    .child(sep())
            })
            .child(item(sdk))
            .child(sep())
            .child(item(format!("{active}/{} upgrades", game.def.setup.len())))
            .child(sep())
            .child(
                toggle("sb-music", Icon::Music, music && !muted && sounds, "Menu music")
                    .on_click(move |_, _, cx| music_ws.update(cx, |ws, cx| ws.set_music(!music, cx))),
            )
            .child(
                toggle(
                    "sb-mute",
                    if muted { Icon::Mute } else { Icon::Volume },
                    !muted && sounds,
                    if sounds { "Button sounds (from the game's launcher)" } else { "No game sounds found" },
                )
                .on_click(move |_, _, cx| mute_ws.update(cx, |ws, cx| ws.set_muted(!muted, cx))),
            )
            .child(sep())
            .child(item(format!("v{}", env!("CARGO_PKG_VERSION"))))
    }

    /// App notifications: flyout-styled cards that slide in from the right.
    fn toasts(&self, bottom: f32, cx: &Context<Self>) -> impl IntoElement {
        let state = self.ws.read(cx);
        let mut stack = div().absolute().bottom(px(bottom)).right(px(16.)).w(px(364.)).flex().flex_col().gap(px(8.));
        for toast in &state.toasts {
            let severity = match toast.kind {
                ToastKind::Info => ui::Severity::Info,
                ToastKind::Success => ui::Severity::Success,
                ToastKind::Error => ui::Severity::Error,
            };
            let (_, color, icon) = severity.style();
            let ws = self.ws.clone();
            let action_ws = self.ws.clone();
            let id = toast.id;
            let card = div()
                .id(ElementId::Integer(id))
                .flex()
                .items_start()
                .gap(px(12.))
                .p(px(16.))
                .backdrop_blur(px(theme::ACRYLIC_BLUR))
                .bg(theme::acrylic())
                .border_1()
                .border_color(theme::flyout_stroke())
                .rounded(px(theme::RADIUS_LG))
                .shadow(theme::shadow())
                .occlude()
                .child(ui::icon(icon).text_color(color).mt(px(2.)))
                .child(div().flex_1().min_w_0().text_size(px(14.)).line_height(px(20.)).text_color(theme::text()).child(toast.message.clone()))
                .when_some(toast.action, |d, action| {
                    d.child(
                        ui::button(
                            ElementId::Name(format!("toast-action-{id}").into()),
                            match action {
                                ToastAction::Undo => "Undo",
                            },
                            None,
                            Variant::Secondary,
                        )
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            action_ws.update(cx, |ws, cx| {
                                ws.dismiss_toast(id, cx);
                                match action {
                                    ToastAction::Undo => ws.undo_last(cx),
                                }
                            })
                        }),
                    )
                })
                .child(
                    ui::icon_button(ElementId::Name(format!("toast-close-{id}").into()), Icon::Close, theme::text_muted())
                        .size(px(24.))
                        .on_click(move |_, _, cx| ws.update(cx, |ws, cx| ws.dismiss_toast(id, cx))),
                );
            // Flyout entrance: 50 px from the edge it's anchored to.
            stack = stack.child(if theme::motion() {
                div()
                    .relative()
                    .child(card)
                    .with_animation(ElementId::Name(format!("toast-in-{id}").into()), Animation::new(Duration::from_millis(367)).with_easing(ui::ease_decelerate()), |d, t| {
                        d.left(px(50. * (1. - t))).opacity((t * 4.).min(1.))
                    })
                    .into_any_element()
            } else {
                card.into_any_element()
            });
        }
        stack
    }

    // ---- overlays -----------------------------------------------------------------

    /// Every waiting change as "old → new", with per-row removal.
    fn review(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let state = self.ws.read(cx);
        if !state.review_open {
            return None;
        }
        let game = state.game();
        let mut rows = div().flex().flex_col();
        for (i, (id, new)) in game.pending.iter().enumerate() {
            let Some(tweak) = game.def.tweak(id) else { continue };
            let old = game.current(tweak).unwrap_or_else(|| tweak.default.to_value());
            let category = game.def.category(tweak.category).map_or("", |c| c.title);
            let ws = self.ws.clone();
            let id = *id;
            rows = rows.when(i > 0, |d| d.child(ui::divider())).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(24.))
                    .py(px(10.))
                    .child(div().flex_1().min_w_0().flex().flex_col().child(ui::title(tweak.label)).child(ui::caption(category)))
                    .child(div().text_size(px(14.)).text_color(theme::text_muted()).child(old.display(&tweak.control)))
                    .child(ui::icon(Icon::ChevronRight).size(px(12.)).text_color(theme::text_muted()))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::accent_text()).child(new.display(&tweak.control)))
                    .child(
                        ui::icon_button(SharedString::from(format!("unstage-{id}")), Icon::Close, theme::text_muted())
                            .tooltip(ui::tip("Drop this change"))
                            .on_click(move |_, _, cx| ws.update(cx, |ws, cx| ws.unstage(id, cx))),
                    ),
            );
        }
        let close_ws = self.ws.clone();
        let backdrop_ws = self.ws.clone();
        let apply_ws = self.ws.clone();
        let count = game.pending.len();
        Some(dialog(
            "review-modal",
            560.,
            div()
                .flex()
                .flex_col()
                .child(div().px(px(24.)).pt(px(24.)).pb(px(12.)).child(ui::display("Review changes", 20.)))
                .child(div().id("review-scroll").max_h(px(420.)).overflow_y_scroll().pb(px(12.)).child(rows)),
            dialog_buttons()
                .child(ui::caption("A backup of the files is made before writing.").flex_1())
                .child(ui::button("review-close", "Close", None, Variant::Secondary).on_click(move |_, _, cx| close_ws.update(cx, |ws, cx| ws.set_review(false, cx))))
                .child(
                    ui::button("review-apply", format!("Apply {}", crate::pages::tweaks::plural(count, "change")), Some(Icon::Check), Variant::Primary)
                        .on_click(move |_, _, cx| apply_ws.update(cx, |ws, cx| ws.apply_pending(cx))),
                ),
            move |cx| backdrop_ws.update(cx, |ws, cx| ws.set_review(false, cx)),
        ))
    }

    /// First run: what was found, and which mode to start in.
    fn welcome(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let state = self.ws.read(cx);
        if state.settings.welcomed {
            return None;
        }
        let mut found = ui::panel().flex().flex_col();
        for (i, g) in state.games.iter().enumerate() {
            found = found.when(i > 0, |d| d.child(ui::divider())).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(16.))
                    .py(px(10.))
                    .child(game_icon(g.art.icon.clone(), g.def.short, 24.))
                    .child(div().flex_1().text_size(px(14.)).text_color(theme::text()).child(g.def.name))
                    .child(div().size(px(6.)).rounded_full().bg(status_color(g)))
                    .child(div().text_size(px(12.)).text_color(theme::text_muted()).child(status_text(g))),
            );
        }
        let mode_card = |id: &'static str, title: &'static str, text: &'static str, icon: Icon, mode: Mode, recommended: bool| {
            let ws = self.ws.clone();
            div()
                .id(id)
                .flex_1()
                .flex_basis(px(0.))
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(16.))
                .rounded(px(theme::RADIUS))
                .border_1()
                .border_color(if recommended { theme::accent() } else { theme::card_stroke() })
                .bg(theme::panel())
                .cursor_pointer()
                .hover(|s| s.bg(theme::control_hover()))
                .active(|s| s.bg(theme::control_pressed()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(ui::icon(icon).size(px(20.)).text_color(theme::accent_text()))
                        .child(ui::label(title))
                        .when(recommended, |d| d.child(ui::badge("Recommended", theme::accent_text()))),
                )
                .child(ui::caption(text))
                .on_click(move |_, _, cx| ws.update(cx, |ws, cx| ws.welcome_done(mode, cx)))
        };
        Some(dialog(
            "welcome",
            600.,
            div()
                .flex()
                .flex_col()
                .gap(px(20.))
                .p(px(24.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .child(app_mark(48.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(ui::display("Welcome to Vaulter", 28.))
                                .child(ui::body("Fixes, settings and mods for the Borderlands games. Every change is backed up and can be undone.")),
                        ),
                )
                .child(div().flex().flex_col().gap(px(8.)).child(ui::label("Games on this PC")).child(found))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(ui::label("How do you want to start?"))
                        .child(
                            div()
                                .flex()
                                .gap(px(12.))
                                .child(mode_card(
                                    "welcome-simple",
                                    "Simple",
                                    "One-click setup with recommended fixes, plus a short list of friendly settings.",
                                    Icon::Wand,
                                    Mode::Simple,
                                    true,
                                ))
                                .child(mode_card(
                                    "welcome-advanced",
                                    "Advanced",
                                    "Every setting, presets, exe patches and the mod manager.",
                                    Icon::Sliders,
                                    Mode::Advanced,
                                    false,
                                )),
                        ),
                ),
            dialog_buttons().child(ui::caption("You can switch any time at the top of the navigation pane.")),
            |_| {},
        ))
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.track_navigation(cx);
        let running = self.ws.read(cx).show_running_screen();
        // The spinner exists only while something runs in the background.
        match (self.ws.read(cx).busy.is_some(), self.spinner.is_some()) {
            (true, false) => self.spinner = Some(cx.new(|cx| BusySpinner::new(14., cx))),
            (false, true) => self.spinner = None,
            _ => {}
        }

        if window.focused(cx).is_none() {
            window.focus(&self.focus);
        }
        let pending_shown = {
            let state = self.ws.read(cx);
            !running && state.mode() == Mode::Advanced && !state.game().pending.is_empty()
        };
        let toast_bottom = 44. + if pending_shown { 64. } else { 0. };
        let drop_ws = self.ws.clone();
        self.narrow = window.viewport_size().width < px(1008.);
        // Wide: the pane is inline, collapsible to the rail. Narrow: the rail
        // stays and the full pane opens over the content.
        let compact = self.narrow || self.pane_open == Some(false);
        let overlay_pane = self.narrow && self.pane_open == Some(true) && !running;
        let overlays: Vec<AnyElement> = crate::pages::compare::lightbox(&self.ws, window, cx)
            .into_iter()
            .chain(self.review(cx))
            .chain(self.welcome(cx))
            .collect();
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .font_family(theme::font_body())
            .text_color(theme::text())
            .text_size(px(14.))
            // Drop .sdkmod/.zip/.blcm files anywhere to install them as mods.
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(theme::with_alpha(theme::accent(), 0.06)))
            .on_drop(move |paths: &ExternalPaths, _, cx| {
                let files: Vec<_> = paths.paths().to_vec();
                drop_ws.update(cx, |ws, cx| {
                    let mods_page = ws.game().def.nav_items(ws.mode()).any(|n| n.kind == PageKind::Mods);
                    ws.install_mod_files(files, cx);
                    if mods_page {
                        ws.navigate(PageKind::Mods, cx);
                    }
                });
            })
            .child(self.title_bar(compact, window, cx))
            // Focus and keys live below the title bar. A focus-tracking
            // ancestor of the caption buttons would claim their mouse-down,
            // and gpui then never passes the click on to Windows.
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(Self::on_key))
                    .on_mouse_down(MouseButton::Navigate(NavigationDirection::Back), cx.listener(|this, _, _, cx| this.go_back(cx)))
                    .when(!running, |d| d.child(self.pane(compact, if compact { "nav-scroll-rail" } else { "nav-scroll" }, cx)))
                    .child(
                        // The content layer, rounded where it meets the pane,
                        // like NavigationView's content area over Mica.
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .bg(theme::layer())
                            .border_t_1()
                            .when(!running, |d| d.border_l_1().rounded_tl(px(theme::RADIUS_LG)))
                            .border_color(theme::card_stroke())
                            .overflow_hidden()
                            // Cached: animations elsewhere (the spinner, pane
                            // hover fades, the nav pill) don't re-render it.
                            .child(
                                gpui::AnyView::from(self.page_host.clone())
                                    .cached(gpui::StyleRefinement::default().flex_1().min_h_0().w_full()),
                            ),
                    ),
            )
            .child(self.status_bar(cx))
            .when(overlay_pane, |d| {
                // NavigationView's overlay pane: acrylic, over the content;
                // a click outside closes it.
                d.child(
                    div()
                        .id("pane-overlay")
                        .absolute()
                        .top(px(TITLE_BAR))
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .occlude()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.pane_open = None;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .id("pane-overlay-body")
                                .h_full()
                                .w(px(PANE))
                                .backdrop_blur(px(theme::ACRYLIC_BLUR))
                                .bg(theme::acrylic())
                                .border_r_1()
                                .border_color(theme::flyout_stroke())
                                .shadow(theme::shadow())
                                .on_click(|_, _, cx| cx.stop_propagation())
                                .child(self.pane(false, "nav-scroll-overlay", cx)),
                        ),
                )
            })
            // Overlays start below the title bar, so the caption buttons work.
            .when(!overlays.is_empty(), |d| {
                d.child(div().absolute().top(px(TITLE_BAR)).left_0().right_0().bottom_0().children(overlays))
            })
            .child(self.toasts(toast_bottom, cx))
            .children(gpui_component::Root::render_dialog_layer(window, cx))
    }
}

/// The page area: the active page (or the game-running screen) and the Apply
/// bar. It's its own view so the shell can render it `.cached(..)`: it only
/// re-renders when the workspace changes or its own animations run.
pub struct PageHost {
    ws: Entity<Workspace>,
    /// The ProgressRing shown while startup reads the games.
    spinner: Option<Entity<BusySpinner>>,
    _sub: Subscription,
}

impl PageHost {
    fn new(ws: Entity<Workspace>, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&ws, |_, _, cx| cx.notify());
        Self { ws, spinner: None, _sub: sub }
    }
}

impl Render for PageHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (page_kind, game_id, nav) = {
            let state = self.ws.read(cx);
            let def = state.game().def;
            let mode = state.mode();
            let nav = def
                .nav_items(mode)
                .find(|n| n.kind == state.page)
                .copied()
                .or_else(|| {
                    // Tool pages reached from elsewhere (e.g. App Settings).
                    matches!(state.page, PageKind::Capture).then_some(NavItem {
                        kind: state.page,
                        title: "Comparison Capture",
                        icon: Icon::Camera,
                        categories: &[],
                    })
                })
                .or_else(|| def.nav_items(mode).next().copied());
            (state.page, def.id, nav)
        };
        // The game is running: its own screen replaces the page.
        let running = self.ws.read(cx).show_running_screen();
        let loaded = self.ws.read(cx).game().loaded;
        if loaded {
            self.spinner = None;
        } else if self.spinner.is_none() {
            self.spinner = Some(cx.new(|cx| BusySpinner::new(32., cx)));
        }
        let page = match nav {
            // Startup reads the games in the background; a ring until then.
            _ if !loaded => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .children(self.spinner.clone())
                .into_any_element(),
            _ if running => pages::running::render(&self.ws, window, cx),
            Some(nav) => pages::render(&nav, &self.ws, window, cx),
            None => div().into_any_element(),
        };
        let page_key = SharedString::from(format!("page-{game_id}-{page_kind:?}-{running}"));
        let images_key = ElementId::Name(format!("images-{page_key}").into());
        // Pages enter with Windows' page-refresh motion (rise and fade in).
        let content = if running || pages::fills_height(page_kind) {
            div().flex_1().min_h_0().flex().flex_col().child(ui::entrance_fill(page_key, 140., page))
        } else {
            div().flex_1().min_h_0().child(
                div().id(ElementId::Name(page_key.clone())).size_full().overflow_y_scroll().child(
                    div()
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(div().flex_1().min_w_0().max_w(px(1064. + 72.)).px(px(36.)).pt(px(24.)).pb(px(36.)).child(ui::entrance(page_key, 140., page))),
                ),
            )
        };
        let pending_bar = Shell::pending_bar(&self.ws, cx).filter(|_| !running);
        div()
            // Each page has its own image cache holding only what it just
            // drew: leaving the page drops the cache and frees its images.
            .image_cache(gpui::retain_recent(images_key, 6))
            .size_full()
            .flex()
            .flex_col()
            .child(content)
            .children(pending_bar)
    }
}

/// The status bar's indeterminate ProgressRing as its own view, ticking at
/// 30 fps: the rest of the window isn't re-rendered for it, and it doesn't
/// run at the monitor's refresh rate.
pub struct BusySpinner {
    size: f32,
    started: std::time::Instant,
    _tick: gpui::Task<()>,
}

impl BusySpinner {
    fn new(size: f32, cx: &mut Context<Self>) -> Self {
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(33)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        Self { size, started: std::time::Instant::now(), _tick: tick }
    }
}

impl Render for BusySpinner {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // One 2 s cycle, like WinUI's ring; a still arc without motion.
        let t = if theme::motion() { (self.started.elapsed().as_secs_f32() / 2.).fract() } else { 0.2 };
        ui::progress_ring_frame(self.size, t)
    }
}

/// Switches between Simple and Advanced. Simple mode saves instantly, so
/// changes still waiting from Advanced must be dealt with first.
fn switch_mode(ws: &Entity<Workspace>, m: Mode, window: &mut Window, cx: &mut gpui::App) {
    let pending = ws.read(cx).pending_count();
    if m != Mode::Simple || pending == 0 {
        ws.update(cx, |ws, cx| ws.set_mode(m, cx));
        return;
    }
    let answer = window.prompt(
        gpui::PromptLevel::Info,
        &format!("You have {} waiting to be applied.", crate::pages::tweaks::plural(pending, "change")),
        Some("Simple mode saves changes immediately. Apply the waiting changes now, or discard them?"),
        &["Apply and switch", "Discard and switch", "Cancel"],
        cx,
    );
    let ws = ws.clone();
    cx.spawn(async move |cx| {
        let choice = answer.await.ok();
        cx.update(|cx| {
            ws.update(cx, |ws, cx| match choice {
                Some(0) => {
                    ws.apply_pending(cx);
                    if ws.pending_count() == 0 {
                        ws.set_mode(m, cx);
                    }
                }
                Some(1) => {
                    ws.discard_pending(cx);
                    ws.set_mode(m, cx);
                }
                _ => {}
            })
        })
        .ok();
    })
    .detach();
}

/// ContentDialog over smoke: the body on the layer color, buttons on the
/// base color below. Clicking the smoke runs `on_backdrop`.
fn dialog(
    id: &'static str,
    width: f32,
    content: impl IntoElement,
    buttons: impl IntoElement,
    on_backdrop: impl Fn(&mut gpui::App) + 'static,
) -> AnyElement {
    let body = div()
        .id(SharedString::from(format!("{id}-body")))
        .w(px(width))
        .max_w_full()
        .flex()
        .flex_col()
        .bg(theme::dialog())
        .border_1()
        .border_color(theme::flyout_stroke())
        .rounded(px(theme::RADIUS_LG))
        .shadow(theme::shadow_dialog())
        .overflow_hidden()
        .on_click(|_, _, cx| cx.stop_propagation())
        .child(content)
        .child(buttons);
    // ContentDialog's entrance: scales 1.05 → 1 over 250 ms while it fades
    // in over the first 83 ms.
    let body: AnyElement = if theme::motion() {
        div()
            .child(body)
            .with_animation(SharedString::from(format!("{id}-in")), Animation::new(theme::NORMAL).with_easing(ui::ease_decelerate()), |d, t| {
                d.transform_scale(1.05 - 0.05 * t).opacity((t * 3.).min(1.))
            })
            .into_any_element()
    } else {
        body.into_any_element()
    };
    let smoke = div()
        .id(id)
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p(px(24.))
        .bg(theme::smoke())
        .occlude()
        .on_click(move |_, _, cx| on_backdrop(cx))
        .child(body);
    if theme::motion() {
        smoke
            .with_animation(SharedString::from(format!("{id}-smoke")), Animation::new(Duration::from_millis(83)), |d, t| d.opacity(t))
            .into_any_element()
    } else {
        smoke.into_any_element()
    }
}

/// The ContentDialog button row.
fn dialog_buttons() -> gpui::Div {
    div().flex().items_center().gap(px(8.)).p(px(24.)).bg(theme::bg_deep()).border_t_1().border_color(theme::card_stroke())
}

fn status_color(g: &crate::workspace::GameState) -> gpui::Rgba {
    if g.install.is_some() {
        theme::success()
    } else if g.config_found() {
        theme::warning()
    } else {
        theme::text_muted()
    }
}

fn status_text(g: &crate::workspace::GameState) -> &'static str {
    if !g.loaded {
        return "Looking for the game\u{2026}";
    }
    match (&g.install, g.config_found()) {
        (Some(i), _) => match i.store {
            crate::core::detect::Store::Steam => "Installed · Steam",
            crate::core::detect::Store::Epic => "Installed · Epic Games",
            crate::core::detect::Store::Manual => "Installed · custom folder",
        },
        (None, true) => "Settings found, game not installed",
        (None, false) => "Not found",
    }
}

/// The game's own icon (from its exe or Steam), or its short name.
pub fn game_icon(path: Option<std::path::PathBuf>, short: &'static str, size: f32) -> AnyElement {
    match path {
        Some(p) => img(p).size(px(size)).flex_none().object_fit(ObjectFit::Contain).into_any_element(),
        None => div()
            .size(px(size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(theme::RADIUS))
            .bg(theme::control())
            .border_1()
            .border_color(theme::control_stroke())
            .font_family(theme::font_display())
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(size * 0.38))
            .text_color(theme::text_muted())
            .child(short)
            .into_any_element(),
    }
}

/// A Windows caption button (46px wide, 10px Segoe Fluent glyph). Hover and
/// press colors follow the system: subtle fills, red for Close.
fn caption_button(id: &'static str, glyph: Icon, area: WindowControlArea, active: bool) -> impl IntoElement {
    let close = matches!(area, WindowControlArea::Close);
    let fg = if active { theme::text() } else { theme::text_disabled() };
    div()
        .id(id)
        .group(id)
        .w(px(46.))
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .window_control_area(area)
        .hover(move |s| if close { s.bg(theme::close_hover()) } else { s.bg(theme::panel_hi()) })
        .active(move |s| if close { s.bg(theme::close_pressed()) } else { s.bg(theme::panel_pressed()) })
        .child(ui::icon(glyph).size(px(10.)).text_color(fg).when(close, |i| i.group_hover(id, |s| s.text_color(gpui::white()))))
}

/// The app logo (the vault door).
fn app_mark(size: f32) -> impl IntoElement {
    img(if size < 40. { theme::LOGO_SMALL } else { theme::LOGO }).size(px(size)).flex_none()
}
