//! Comparison images: the before/after slider (inline in the detail pane and
//! full-window), and the capture tool page.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Bounds, Entity, FontWeight, IntoElement, ObjectFit, ParentElement, Pixels, SharedString, Styled,
    StyledImage, Window, div, img, prelude::*, px,
};

use super::{breadcrumb_header, missing_notice};
use crate::compare;
use crate::games::willow;
use crate::theme::{self, Icon};
use crate::tweaks::Tweak;
use crate::ui::{self, Variant};
use crate::workspace::Workspace;

/// Marker for divider drags; carries the frame id so frames don't cross-talk.
#[derive(Clone)]
struct DividerDrag(SharedString);

/// Two images stacked in one frame, split by a draggable divider.
#[allow(clippy::too_many_arguments)]
fn split_frame(
    id: SharedString,
    (left_label, left_path): (String, PathBuf),
    (right_label, right_path): (String, PathBuf),
    width: f32,
    height: f32,
    split: f32,
    on_split: impl Fn(f32, &mut App) + 'static,
) -> impl IntoElement {
    let split = split.clamp(0., 1.);
    let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let to_split = |x: Pixels, b: Bounds<Pixels>| if b.size.width <= px(0.) { 0.5 } else { ((x - b.left()) / b.size.width).clamp(0., 1.) };
    let on_split = Rc::new(on_split);
    let on_down = on_split.clone();
    let bounds_down = bounds.clone();
    let bounds_canvas = bounds.clone();
    let drag_id = id.clone();
    let small = width < 500.;
    let tag = |text: String, right: bool| {
        div()
            .absolute()
            .top(px(8.))
            .when(right, |d| d.right(px(8.)))
            .when(!right, |d| d.left(px(8.)))
            .px(px(8.))
            .py(px(2.))
            .rounded(px(theme::RADIUS))
            .bg(theme::with_alpha(theme::bg_deep(), 0.82))
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(12.))
            .line_height(px(16.))
            .text_color(theme::text())
            .child(text)
    };
    let knob = if small { 24. } else { 34. };
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .overflow_hidden()
        .rounded(px(theme::RADIUS))
        .bg(theme::panel_lo())
        .cursor(gpui::CursorStyle::ResizeLeftRight)
        .child(gpui::canvas(move |b, _, _| bounds_canvas.set(b), |_, _, _, _| {}).absolute().size_full())
        .child(img(right_path).absolute().top_0().left_0().w(px(width)).h(px(height)).object_fit(ObjectFit::Cover))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .h_full()
                .w(px(width * split))
                .overflow_hidden()
                .child(img(left_path).w(px(width)).h(px(height)).object_fit(ObjectFit::Cover)),
        )
        .child(div().absolute().top_0().bottom_0().left(px(width * split - 1.)).w(px(2.)).bg(theme::accent()))
        .child(
            div()
                .absolute()
                .top(px(height / 2. - knob / 2.))
                .left(px(width * split - knob / 2.))
                .size(px(knob))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::accent())
                .shadow(theme::shadow())
                .child(ui::icon(Icon::Compare).size(px(knob * 0.5)).text_color(theme::accent_ink())),
        )
        .child(tag(left_label, false))
        .child(tag(right_label, true))
        .on_mouse_down(gpui::MouseButton::Left, move |ev, _, cx| {
            cx.stop_propagation();
            on_down(to_split(ev.position.x, bounds_down.get()), cx);
        })
        .on_click(|_, _, cx| cx.stop_propagation())
        .on_drag(DividerDrag(drag_id.clone()), |_, _, _, cx| cx.new(|_| ui::EmptyView))
        .on_drag_move::<DividerDrag>(move |ev, _, cx| {
            if ev.drag(cx).0 == drag_id {
                on_split(to_split(ev.event.position.x, ev.bounds), cx);
            }
        })
}

/// The detail pane's comparison: your current option on the left, any other
/// option on the right, plus a thumbnail per option and the Nvidia link.
pub fn inline_viewer(tweak: &'static Tweak, ws: &Entity<Workspace>, width: f32, cx: &App) -> Option<AnyElement> {
    let state = ws.read(cx);
    let game = state.game();
    let def = game.def;
    let comparison = def.comparison(tweak.id);
    // Small preview copies here; full size only in the full-window viewer.
    let images: Vec<(String, PathBuf)> = compare::images(def.id, tweak).into_iter().map(|(l, p)| (l, compare::preview(&p))).collect();
    let values = compare::captured_values(def.id, tweak);
    let link = comparison.and_then(|c| c.link).map(willow::nvidia_url);
    if images.len() < 2 && link.is_none() {
        return None;
    }
    let mut col = div().flex().flex_col().gap(px(8.));
    if images.len() >= 2 && images.len() == values.len() {
        let current = game.effective(tweak);
        let left = values.iter().position(|v| *v == current).unwrap_or(0);
        let default = tweak.default.to_value();
        let (_, stored_right, split) = state
            .inline_compare
            .filter(|c| c.0 == tweak.id)
            .unwrap_or((tweak.id, usize::MAX, 0.5));
        let right = if stored_right < images.len() && stored_right != left {
            stored_right
        } else {
            values.iter().position(|v| *v == default).filter(|&d| d != left).unwrap_or(if left == 0 { 1 } else { 0 })
        };
        let height = width * 9. / 16.;
        let split_ws = ws.clone();
        let (left_label, left_path) = images[left].clone();
        col = col.child(split_frame(
            SharedString::from(format!("inline-{}", tweak.id)),
            (format!("Now: {left_label}"), left_path),
            images[right].clone(),
            width,
            height,
            split,
            move |s, cx| split_ws.update(cx, |ws, cx| ws.set_inline(tweak.id, Some(right), Some(s), cx)),
        ));
        let mut thumbs = div().flex().gap(px(4.)).overflow_hidden();
        let thumb_w = ((width - 4. * (images.len() as f32 - 1.)) / images.len() as f32 - 0.5).min(96.);
        for (i, (label, path)) in images.iter().enumerate() {
            let ws = ws.clone();
            let is_right = i == right;
            let is_current = i == left;
            // Only the right-hand pick gets the accent border; the current
            // option is marked by its label ("Now: …" on the frame too).
            let thumb = div()
                .id(SharedString::from(format!("thumb-{}-{i}", tweak.id)))
                .w(px(thumb_w))
                .flex()
                .flex_col()
                .gap(px(4.))
                .rounded(px(theme::RADIUS))
                .tooltip(ui::tip(if is_current { format!("{label} (current)") } else { format!("Compare with {label}") }))
                .child(
                    div()
                        .w_full()
                        .h(px(thumb_w * 9. / 16.))
                        .rounded(px(theme::RADIUS))
                        .overflow_hidden()
                        .border_2()
                        .border_color(if is_right { theme::accent().into() } else { gpui::transparent_black() })
                        .child(img(path.clone()).size_full().object_fit(ObjectFit::Cover)),
                )
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .text_ellipsis().line_clamp(1)
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(if is_current { theme::text() } else { theme::text_muted() })
                        .when(is_current, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(label.clone()),
                );
            thumbs = thumbs.child(if is_current {
                thumb
            } else {
                ui::focusable(thumb).cursor_pointer().on_click(move |_, _, cx| ws.update(cx, |ws, cx| ws.set_inline(tweak.id, Some(i), None, cx)))
            });
        }
        let open_ws = ws.clone();
        col = col.child(thumbs).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    ui::button(SharedString::from(format!("big-{}", tweak.id)), "Full screen", Some(Icon::Expand), Variant::Ghost)
                        .on_click(move |_, _, cx| open_ws.update(cx, |ws, cx| ws.open_preview(tweak.id, right, cx))),
                )
                .child(div().flex_1())
                .when_some(link.clone(), |d, url| d.child(nvidia_link(tweak, url, def.id))),
        );
    } else if let Some(url) = link {
        col = col.child(nvidia_link(tweak, url, def.id));
    }
    Some(col.into_any_element())
}

/// HyperlinkButton to Nvidia's comparison page. Nvidia's guide covers both
/// Willow games but its screenshots are Borderlands 2's, so TPS says so.
fn nvidia_link(tweak: &Tweak, url: String, game_id: &str) -> impl IntoElement {
    let (label, tip) = if game_id == "bl2" {
        ("Nvidia comparison", "Nvidia's interactive comparison, in your browser")
    } else {
        ("Nvidia comparison (BL2)", "Nvidia's interactive comparison, shot in Borderlands 2 on the same engine, in your browser")
    };
    ui::focusable(div().id(SharedString::from(format!("nv-{}", tweak.id))))
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(32.))
        .px(px(8.))
        .rounded(px(theme::RADIUS))
        .text_size(px(14.))
        .text_color(theme::accent_text())
        .cursor_pointer()
        .hover(|s| s.bg(theme::panel_hi()))
        .active(|s| s.bg(theme::panel_pressed()))
        .tooltip(ui::tip(tip))
        .child(ui::icon(Icon::Link).size(px(12.)).text_color(theme::accent_text()))
        .child(label)
        .on_click(move |_, _, cx| cx.open_url(&url))
}

/// Full-window viewer: two options side by side with a draggable divider.
pub fn lightbox(ws: &Entity<Workspace>, window: &Window, cx: &App) -> Option<AnyElement> {
    let state = ws.read(cx);
    let preview = state.preview?;
    let def = state.game().def;
    let tweak = def.tweak(preview.tweak)?;
    let images = compare::images(def.id, tweak);
    if images.is_empty() {
        return None;
    }
    let pick = |i: usize| images.get(i).or_else(|| images.first()).cloned().expect("non-empty");
    let (left_label, left_path) = pick(preview.left);
    let (right_label, right_path) = pick(preview.right);

    // Fit a 16:9 frame into the window, leaving room for the controls.
    let viewport = window.viewport_size();
    // (The dialog surface adds 24px padding on each side; keep it clear of
    // the 48px title bar too.)
    let max_w = f32::from(viewport.width) * 0.88 - 48.;
    let max_h = f32::from(viewport.height) - 230. - 48. - 48.;
    let frame_w = max_w.min(max_h * 16. / 9.).max(320.);
    let frame_h = frame_w * 9. / 16.;
    // The smallest copies that still fill the frame on this display.
    let pixels = frame_w * window.scale_factor();
    let (left_path, right_path) = (compare::sized(&left_path, pixels), compare::sized(&right_path, pixels));
    let split_ws = ws.clone();
    let frame = split_frame(
        "lightbox-frame".into(),
        (left_label.clone(), left_path),
        (right_label.clone(), right_path),
        frame_w,
        frame_h,
        preview.split,
        move |s, cx| split_ws.update(cx, |ws, cx| ws.set_preview(|p| p.split = s, cx)),
    );

    let side = |side_right: bool| {
        let mut row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(4.))
            .child(
                div()
                    .w(px(48.))
                    .flex_none()
                    .text_size(px(14.))
                    .line_height(px(20.))
                    .text_color(theme::text_muted())
                    .child(if side_right { "Right" } else { "Left" }),
            );
        for (i, (label, _)) in images.iter().enumerate() {
            let ws = ws.clone();
            let selected = if side_right { preview.right == i } else { preview.left == i };
            row = row.child(
                ui::chip(SharedString::from(format!("lb-{}-{i}", if side_right { "r" } else { "l" })), label.clone(), selected).on_click(
                    move |_, _, cx| {
                        cx.stop_propagation();
                        ws.update(cx, |ws, cx| ws.set_preview(|p| if side_right { p.right = i } else { p.left = i }, cx))
                    },
                ),
            );
        }
        row
    };

    let values = compare::captured_values(def.id, tweak);
    let simple = state.mode() == crate::games::Mode::Simple;
    let mut use_row = div().id("lightbox-use").flex().gap(px(8.)).on_click(|_, _, cx| cx.stop_propagation());
    for (which, index, label) in [("left", preview.left, &left_label), ("right", preview.right, &right_label)] {
        let Some(value) = values.get(index).cloned() else { continue };
        let ws = ws.clone();
        use_row = use_row.child(
            ui::button(
                SharedString::from(format!("use-{which}")),
                format!("Use {label}"),
                Some(Icon::Check),
                if which == "right" { Variant::Primary } else { Variant::Secondary },
            )
            .on_click(move |_, _, cx| {
                let value = value.clone();
                ws.update(cx, |ws, cx| {
                    if simple {
                        ws.set_now(tweak, value, cx);
                    } else {
                        ws.stage(tweak, value, cx);
                    }
                    ws.close_preview(cx);
                })
            }),
        );
    }

    let close_ws = ws.clone();
    let close_btn_ws = ws.clone();
    Some(
        div()
            // Only the pair on screen stays decoded; stepping to another
            // pair frees the last one, and closing frees everything.
            .image_cache(gpui::retain_recent("lightbox-images", 2))
            .id("lightbox")
            .absolute()
            .inset_0()
            .occlude()
            // Smoke over the app, and the viewer on an opaque dialog surface.
            .bg(theme::smoke())
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p(px(20.))
            .on_click(move |_, _, cx| close_ws.update(cx, |ws, cx| ws.close_preview(cx)))
            .child(
                div()
                    .id("lightbox-dialog")
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .p(px(24.))
                    .rounded(px(theme::RADIUS_LG))
                    .bg(theme::dialog())
                    .border_1()
                    .border_color(theme::flyout_stroke())
                    .shadow(theme::shadow_dialog())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .w(px(frame_w))
                            .flex()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(ui::display(tweak.label, 20.)))
                            .child(
                                ui::icon_button("lightbox-close", Icon::Close, theme::text_muted())
                                    .tooltip(ui::tip("Close (Esc)"))
                                    .on_click(move |_, _, cx| close_btn_ws.update(cx, |ws, cx| ws.close_preview(cx))),
                            ),
                    )
                    .child(frame)
                    .child(
                        div()
                            .id("lightbox-controls")
                            .w(px(frame_w))
                            .flex()
                            .items_start()
                            .gap(px(16.))
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(8.)).child(side(false)).child(side(true)))
                            .child(use_row),
                    )
                    .child(
                        div()
                            .w(px(frame_w))
                            .text_size(px(12.))
                            .line_height(px(16.))
                            .text_color(theme::text_muted())
                            .child("Drag across the image to compare · ← → switch the right side · Esc closes"),
                    ),
            )
            .into_any_element(),
    )
}

/// Capture comparison screenshots on this PC.
pub fn capture_page(ws: &Entity<Workspace>, _window: &mut Window, cx: &mut App) -> AnyElement {
    let state = ws.read(cx);
    let game = state.game();
    let def = game.def;
    // Reached from App settings, which has the nav selection: a breadcrumb
    // leads back.
    let back_ws = ws.clone();
    let mut page = div().flex().flex_col().gap(px(20.)).child(breadcrumb_header(
        "App settings",
        move |cx| back_ws.update(cx, |ws, cx| ws.navigate(crate::games::PageKind::Settings, cx)),
        "Comparison capture",
        "Screenshots every option of the listed settings so the settings pages can show what each looks like. Your settings are backed up first and restored when it finishes.",
    ));
    if game.install.is_none() {
        return page.child(missing_notice("Game install not found", "Capturing runs the game.", ws)).into_any_element();
    }

    let targets: Vec<&'static Tweak> = def.comparisons.iter().filter(|c| c.capture).filter_map(|c| def.tweak(c.tweak)).collect();
    let total_shots: usize = targets.iter().map(|t| compare::capture_values(t).len()).sum();
    let have: usize = targets.iter().map(|t| compare::images(def.id, t).len()).sum();
    let minutes = (total_shots as u32 * (state.capture_settle + 30)).div_ceil(60);

    page = page.child(
        ui::panel().p(px(16.)).flex().flex_col().gap(px(8.))
            .child(ui::section_title("How it works", None))
            .child(ui::body(format!("For each of the {total_shots} shots, Vaulter writes that setting and launches the game straight into your chosen save (via the Quick Startup mod). A small helper mod hides the HUD and weapon, the window is captured, and the game quits. Your settings and game folder are restored afterwards.")))
            .child(ui::body(format!("Takes about {minutes} minutes and the game takes over the screen. Save somewhere scenic first; every shot is taken where the save loads."))),
    );

    let saves = state.capture_saves();
    let chosen = state.capture_save.clone().or_else(|| saves.first().cloned());
    let mut save_row = div().flex().flex_wrap().gap(px(8.));
    for s in saves.iter().take(8) {
        let ws = ws.clone();
        let name = s.clone();
        save_row = save_row.child(
            ui::chip(SharedString::from(format!("save-{s}")), s.clone(), chosen.as_deref() == Some(s)).on_click(move |_, _, cx| {
                ws.update(cx, |ws, cx| {
                    ws.capture_save = Some(name.clone());
                    cx.notify();
                })
            }),
        );
    }
    if saves.is_empty() {
        save_row = save_row.child(ui::body("None found"));
        page = page.child(ui::info_bar(
            ui::Severity::Warning,
            "No save files yet",
            "Capturing loads straight into a save. Play the game once and save somewhere scenic first.",
        ));
    }
    let mut settle_row = div().flex().gap(px(8.));
    for secs in [15u32, 25, 40] {
        let ws = ws.clone();
        settle_row = settle_row.child(
            ui::chip(SharedString::from(format!("settle-{secs}")), format!("{secs} s"), state.capture_settle == secs).on_click(move |_, _, cx| {
                ws.update(cx, |ws, cx| {
                    ws.capture_settle = secs;
                    cx.notify();
                })
            }),
        );
    }
    page = page.child(
        ui::panel()
            .flex()
            .flex_col()
            .child(ui::setting_row("Save to load", "The game loads straight into this save.", save_row.into_any_element()))
            .child(ui::divider())
            .child(ui::setting_row("Wait after loading", "Lets textures stream in before each shot.", settle_row.into_any_element()))
            .child(ui::divider())
            .child(div().px(px(16.)).py(px(12.)).flex().flex_col().gap(px(4.)).child(ui::title("Settings captured")).child(ui::body(
                targets.iter().map(|t| t.label).collect::<Vec<_>>().join(" · "),
            ))),
    );

    let running = state.capture_running();
    let no_saves = saves.is_empty();
    let start_ws = ws.clone();
    let cancel_ws = ws.clone();
    let mut run = ui::panel().p(px(16.)).flex().flex_col().gap(px(12.)).child(
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(ui::body(format!("{have} of {total_shots} images captured so far.")).flex_1())
            .child(if running {
                ui::button("capture-cancel", "Stop after this shot", Some(Icon::Close), Variant::Secondary)
                    .on_click(move |_, _, cx| cancel_ws.update(cx, |ws, cx| ws.cancel_capture(cx)))
                    .into_any_element()
            } else {
                ui::button_if(
                    !no_saves,
                    "capture-start",
                    if have >= total_shots { "Recapture" } else { "Start capture" },
                    Some(Icon::Camera),
                    if have >= total_shots { Variant::Secondary } else { Variant::Primary },
                )
                .when(no_saves, |b| b.tooltip(ui::tip("Needs a save file: play the game once and save first")))
                .on_click(move |_, _, cx| {
                    if !no_saves {
                        start_ws.update(cx, |ws, cx| ws.start_capture(cx))
                    }
                })
                .into_any_element()
            }),
    );
    if let Some(progress) = state.capture.as_ref().and_then(|p| p.lock().ok().map(|p| p.clone())) {
        let fraction = if progress.total == 0 { 0. } else { progress.done as f32 / progress.total as f32 };
        run = run
            .child(ui::progress_bar("capture-progress", Some(fraction), theme::accent()))
            .child(ui::caption(if progress.finished {
                "Finished. Settings restored.".to_string()
            } else {
                format!("Shot {} of {}: {}", progress.done + 1, progress.total, progress.current)
            }))
            .children(progress.log.iter().rev().take(8).map(|l| ui::caption(l.clone())));
    }
    page.child(run).into_any_element()
}
