//! The Mods library's parts for the catalogue's mods: a mod's controls (GET, UPDATE, PLAY, its
//! switch, an import, its pages, its check, removal), its release details, its picture or a
//! plate of its own, the badge on it, progress rails and the celebration of an install.

use ta_ui::tooltip::TooltipExt as _;
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, TextureHandle, Ui, pos2, vec2};
use ta_ui::i18n::{n, t, tf};

use crate::catalogue::{self, Download, Entry, Fit};
use crate::getmods::Stage;
use crate::theme::*;
use crate::widgets::{self, ActionButton};
use ta_ui::metal;

/// What a mod's controls ask for.
pub(super) enum CardAction {
    Get,
    Cancel,
    Play,
    TurnOn,
    TurnOff,
    FromFile,
    Open(String),
    Remove,
    Info,
}

/// How long the celebration of a finished install lasts.
pub(super) const CELEBRATION: f32 = 2.2;

/// An install's progress as a rail shows it: how far (when that is known) and what is going on.
pub(super) fn stage_progress(stage: &Stage) -> (Option<f32>, String) {
    match stage {
        Stage::Queued => (None, t("WAITING FOR THE ONE BEFORE").to_string()),
        Stage::Connecting => (None, t("CONNECTING…").to_string()),
        Stage::Downloading { got, total, since } => {
            let secs = since.elapsed().as_secs_f32();
            let left = if *got > 0 && secs > 1.0 { ((*total - (*got).min(*total)) as f32 * secs / *got as f32).round() } else { f32::NAN };
            let mut text = format!("{} / {}", megabytes(*got), megabytes(*total));
            if left.is_finite() { text.push_str(&format!("  ·  {}", tf("{s} s left", &[("s", &left)]))); }
            (Some(*got as f32 / (*total).max(1) as f32), text)
        }
        Stage::Checking => (None, t("CHECKING THE FILE…").to_string()),
        Stage::Copying { got, total } => (Some(*got as f32 / (*total).max(1) as f32), tf("Copying {done} / {total}", &[("done", &megabytes(*got)), ("total", &megabytes(*total))])),
        Stage::Unpacking => (None, t("UNPACKING AND INSTALLING…").to_string()),
        Stage::Done { .. } | Stage::Cancelled | Stage::Failed(_) | Stage::Replaces { .. } => (None, String::new()),
    }
}

/// Keep release information visible for manual and installed mods too. Empty values
/// are explicit rather than disappearing, and dates retain their month and day.
pub(super) fn release_details(ui: &mut Ui, entry: &Entry) {
    let release = entry.release();
    let version = if release.version.is_empty() { t("Unknown") } else { release.version };
    let date = if release.date.is_empty() { t("Unknown") } else { release.date };
    let size = release.bytes.map(megabytes).unwrap_or_else(|| t("Unknown").to_string());
    ui.horizontal_wrapped(|ui| {
        for label in [tf("Version: {version}", &[("version", &version)]), tf("Released: {date}", &[("date", &date)]), tf("Download: {size}", &[("size", &size)])] {
            widgets::tag(ui, &label, STEEL);
        }
    });
}

/// Controls use normal wrapped flow, so buttons never share the progress rail's space.
/// `blocked` is why the mod cannot be removed now (the game running, installs at work), if it
/// cannot; `held`, why an installed mod cannot be updated now (the game running).
pub(super) fn footer_ui(ui: &mut Ui, entry: &'static Entry, installed: Option<&str>, enabled: bool, stage: Option<&Stage>, online: bool, blocked: Option<&str>, held: Option<&str>) -> Option<CardAction> {
    let mut action = None;
    let manual = entry.fit == Fit::Unverified || entry.file.is_some_and(|file| !file.can_download());
    let active = stage.filter(|s| s.active());
    if let Some(stage) = active {
        let (fraction, label) = stage_progress(stage);
        ui.horizontal(|ui| {
            progress(ui, (ui.available_width() - 44.0).max(1.0), fraction, &label);
            if widgets::icon_button(ui, widgets::Glyph::Close, true).hover_tip(t("Cancel")).clicked() { action = Some(CardAction::Cancel); }
        });
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        // Manual-only releases lead with the creator's website, beside local import.
        if manual && installed.is_none()
            && ui.add(ActionButton::primary("WEBSITE ↗").height(34.0)).hover_tip(entry.page()).clicked()
        {
            action = Some(CardAction::Open(entry.page()));
        }
        if active.is_none() && entry.can_import() {
            match (installed, entry.file) {
                // (Over an older copy, never the release over a newer one; and over one of unknown
                // version, as its menu offers it: one added from Total Annihilation's folder may be
                // this release or a later one, so the badge and UPDATE ALL count it waiting for
                // nothing, but it can be replaced by the tested release here.)
                (Some(version), Some(file)) if file.can_download() && crate::mod_library::update(entry, Some(version)).is_some() => {
                    let why = if !online { Some(t("The launcher is offline.")) } else { held };
                    if ui.add_enabled(why.is_none(), ActionButton::primary(&tf("UPDATE · {size}", &[("size", &megabytes(file.bytes))])).height(34.0))
                        .disabled_tip(why.unwrap_or_default()).clicked() { action = Some(CardAction::Get); }
                    if ui.add(ActionButton::secondary("PLAY")).clicked() { action = Some(CardAction::Play); }
                }
                (Some(_), _) => {
                    if ui.add(ActionButton::primary("PLAY").height(34.0)).hover_tip(t("Start the game with this mod (and no other conversion)")).clicked() { action = Some(CardAction::Play); }
                    let mut on = enabled;
                    if widgets::toggle_named(ui, &mut on, entry.name).hover_tip(t("Turn the mod on or off for the next game")).changed() { action = Some(if on { CardAction::TurnOn } else { CardAction::TurnOff }); }
                    ui.label(RichText::new(t(if enabled { "ON" } else { "OFF" })).font(strong(12.0)).color(if enabled { GREEN } else { STEEL }));
                }
                (None, Some(file)) if file.can_download() => {
                    if ui.add_enabled(online, ActionButton::primary(&tf("GET · {size}", &[("size", &megabytes(file.bytes))])).height(34.0))
                        .hover_tip(tf("Download {file} from {source} and install it", &[("file", &file.name), ("source", &entry.source_name())])).clicked() { action = Some(CardAction::Get); }
                }
                _ => {}
            }
            // Unpinned project releases stay importable for future updates; tested releases
            // retain their exact version and checksum checks in the installer.
            if entry.file.is_none_or(|file| installed != Some(file.version)) && (installed.is_none() || held.is_none()) && ui.link(RichText::new(t("Have the file?")).font(body(12.5)))
                .hover_tip(tf("Install {file} from your computer", &[("file", &entry.file.map_or(entry.name, |file| file.name))])).clicked() { action = Some(CardAction::FromFile); }
        }
        let website_label = if entry.project.is_none() && entry.moddb().is_some() { n("MODDB ↗") } else { n("WEBSITE ↗") };
        if (!manual || installed.is_some()) && ui.add(ActionButton::secondary(website_label)).hover_tip(entry.page()).clicked() { action = Some(CardAction::Open(entry.page())); }
        // Keep the tested release's host credited without replacing the creator's page.
        if entry.project.is_some() && entry.moddb().is_some()
            && ui.link(RichText::new(t("MODDB ↗")).font(body(12.5))).hover_tip(entry.source_page()).clicked()
        {
            action = Some(CardAction::Open(entry.source_page()));
        }
        if !manual && (entry.file.is_some_and(|file| matches!(file.download, Download::Direct(_))) || !entry.links.is_empty()) {
            ui.menu_button(t("DOWNLOAD LINKS"), |ui| {
                if let Some(catalogue::File { download: Download::Direct(urls), .. }) = entry.file {
                    for (index, url) in urls.iter().enumerate() {
                        let label = if index == 0 { n("Main download") } else { n("Backup download") };
                        if ui.button(t(label)).hover_tip(*url).clicked() {
                            action = Some(CardAction::Open((*url).into()));
                            ui.close();
                        }
                    }
                    ui.separator();
                    ui.add(egui::Label::new(t("Download in your browser, then use Have the file? to install the main mod.")).wrap());
                }
                if !entry.links.is_empty() {
                    ui.separator();
                    ui.label(RichText::new(t("Optional downloads")).strong());
                    for link in entry.links {
                        if ui.button(t(link.label)).hover_tip(link.url).clicked() {
                            action = Some(CardAction::Open(link.url.into()));
                            ui.close();
                        }
                    }
                }
            });
        }
        if super::mod_report::info_button(ui).clicked() { action = Some(CardAction::Info); }
        if installed.is_some() {
            let trash = widgets::glyph_button(ui, widgets::Glyph::Trash, vec2(34.0, 34.0), widgets::Tone::Secondary, active.is_none() && blocked.is_none());
            let trash = match blocked { Some(why) if active.is_none() => trash.disabled_tip(why), _ => trash.hover_tip(t("Remove this mod (to the Recycle Bin)")) };
            if trash.clicked() { action = Some(CardAction::Remove); }
        }
    });
    if let Some(Stage::Failed(why)) = stage {
        ui.add(egui::Label::new(RichText::new(why).font(body(12.0)).color(RED)).wrap());
    }
    action
}

pub(super) fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb < 10.0 { format!("{mb:.1} MB") } else { format!("{mb:.0} MB") }
}

/// A mod's picture, cropped to fill (zooming in a little under the pointer; a `bundled`
/// banner fitted whole instead); without one (before it has come, offline, or a mod of the
/// player's own), a plate of its own: dark metal tinted in a colour of the mod's (`id`), the
/// emblem faint behind its `name`.
pub(super) fn paint_picture(painter: &egui::Painter, rect: Rect, texture: Option<&TextureHandle>, id: &str, name: &str, bundled: bool, hover: f32) {
    match texture {
        Some(texture) => {
            let size = texture.size_vec2();
            if bundled {
                // Composed banners need every ship visible, even in a narrow spotlight.
                // Fit the full illustration and keep hover zoom from clipping its subjects.
                let scale = (rect.width() / size.x.max(1.0)).min(rect.height() / size.y.max(1.0));
                let fitted = Rect::from_center_size(rect.center(), size * scale);
                painter.rect_filled(rect, 0.0, INSET);
                painter.image(texture.id(), fitted, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                let (want, have) = (rect.width() / rect.height(), size.x / size.y.max(1.0));
                let (mut u, mut v) = (1.0, 1.0);
                if have > want {
                    u = want / have;
                } else {
                    v = have / want;
                }
                let zoom = 1.0 - 0.04 * hover;
                let (u, v) = (u * zoom, v * zoom);
                let uv = Rect::from_min_max(pos2((1.0 - u) / 2.0, (1.0 - v) / 2.0), pos2((1.0 + u) / 2.0, (1.0 + v) / 2.0));
                painter.image(texture.id(), rect, uv, Color32::WHITE);
            }
            // Darker towards the edges, so the badges read.
            widgets::gradient_v(painter, Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + 44.0)), Color32::from_black_alpha(150), Color32::TRANSPARENT);
            widgets::gradient_v(painter, Rect::from_min_max(pos2(rect.min.x, rect.max.y - 44.0), rect.max), Color32::TRANSPARENT, Color32::from_black_alpha(130));
        }
        None => {
            // The mod's colour, from its id: the same every time.
            let hash = id.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
            let hues = [Color32::from_rgb(70, 92, 120), Color32::from_rgb(112, 74, 52), Color32::from_rgb(70, 104, 70), Color32::from_rgb(104, 64, 96), Color32::from_rgb(110, 98, 58), Color32::from_rgb(60, 98, 104)];
            let tint = hues[hash as usize % hues.len()];
            painter.add(metal::vgrad(rect, tint.lerp_to_gamma(Color32::BLACK, 0.55), Color32::from_rgb(12, 12, 10)));
            painter.add(metal::grime_over(painter.ctx(), rect, 1.0));
            // The emblem, faint, on the right; the edges darker.
            let side = rect.height() * 0.9;
            let mut faint = painter.clone();
            faint.multiply_opacity(0.1);
            widgets::paint_emblem(&faint, Rect::from_center_size(pos2(rect.max.x - side * 0.42, rect.center().y), vec2(side, side)));
            widgets::gradient_h(painter, Rect::from_min_max(rect.min, pos2(rect.min.x + rect.width() * 0.25, rect.max.y)), Color32::from_black_alpha(90), Color32::TRANSPARENT);
            widgets::gradient_v(painter, Rect::from_min_max(pos2(rect.min.x, rect.max.y - 50.0), rect.max), Color32::TRANSPARENT, Color32::from_black_alpha(120));
            // The name, lettered as TA's titles are, as large as fits.
            let size = (rect.height() * 0.18).clamp(14.0, 44.0);
            let galley = painter.layout_no_wrap(name.to_uppercase(), display(size), TEXT_BRIGHT);
            let galley = if galley.size().x > rect.width() * 0.8 { painter.layout_no_wrap(name.to_uppercase(), display((size * rect.width() * 0.8 / galley.size().x).max(9.0)), TEXT_BRIGHT) } else { galley };
            metal::embossed(painter, rect.center() - galley.size() / 2.0, galley, TEXT_BRIGHT.gamma_multiply(0.9));
        }
    }
}

/// A small label on a dark plate, with a lit lamp.
pub(super) fn badge(ui: &Ui, painter: &egui::Painter, at: Pos2, text: &str, color: Color32, align: Align2) {
    let galley = widgets::galley(ui, RichText::new(text).font(strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(1.2)));
    let rect = align.anchor_size(at, galley.size() + vec2(24.0, 8.0));
    painter.rect(rect, CornerRadius::same(2), Color32::from_black_alpha(200), Stroke::new(1.0, metal::STEEL_SHADE), StrokeKind::Inside);
    painter.add(metal::lamp(pos2(rect.min.x + 10.0, rect.center().y), 2.8, color, 1.0));
    painter.galley(pos2(rect.min.x + 18.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// A progress rail: filled to `fraction`, or with stripes marching along it when there is
/// nothing to count; `label` over it.
pub(super) fn progress(ui: &mut Ui, width: f32, fraction: Option<f32>, label: &str) {
    let width = width.min(ui.available_width()).max(1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 42.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, ui.is_enabled(), label));
    response.hover_tip(label);
    let rail = Rect::from_min_max(pos2(rect.left(), rect.top() + 20.0), rect.max);
    ta_ui::progress::paint(ui, rail, fraction, true);
    let text = egui::WidgetText::from(RichText::new(label).font(strong(12.0)).extra_letter_spacing(ta_ui::i18n::tracking(0.6)))
        .into_galley(ui, Some(egui::TextWrapMode::Truncate), (width - 12.0).max(1.0), egui::TextStyle::Body);
    // Keep the readable value clear of the moving construction head at every
    // fraction, especially the middle of a download.
    ui.painter().galley(pos2(rect.center().x - text.size().x / 2.0, rect.top()), text, TEXT_BRIGHT);
}

/// Bits of colour bursting up from `origin` and falling, `age` seconds in.
pub(super) fn confetti(painter: &egui::Painter, origin: Pos2, age: f32, seed: &str) {
    let mut state = seed.bytes().fold(0x9e37_79b9u32, |h, b| h.rotate_left(5) ^ b as u32).wrapping_mul(2_654_435_761);
    let mut random = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state >> 8) as f32 / (1u32 << 24) as f32
    };
    let colors = [AMBER, AMBER_BRIGHT, GREEN, SKY, TEXT_BRIGHT];
    let fade = (1.0 - (age - CELEBRATION + 0.6).max(0.0) / 0.6).clamp(0.0, 1.0);
    for i in 0..42 {
        let angle = std::f32::consts::PI * (1.1 + 0.8 * random());
        let speed = 160.0 + 260.0 * random();
        let spin = (random() - 0.5) * 14.0;
        let size = 3.0 + 3.5 * random();
        let p = origin + vec2(angle.cos(), angle.sin()) * speed * age + vec2(0.0, 420.0 * age * age);
        let a = spin * age + i as f32;
        let (s, c) = a.sin_cos();
        let corner = |dx: f32, dy: f32| p + vec2(dx * c - dy * s, dx * s + dy * c) * size;
        let color = colors[i % colors.len()].gamma_multiply(fade);
        painter.add(Shape::convex_polygon(vec![corner(-1.0, -0.5), corner(1.0, -0.5), corner(1.0, 0.5), corner(-1.0, 0.5)], color, Stroke::NONE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect_text(shape: &Shape, text: &mut String) {
        match shape {
            Shape::Text(item) => { text.push_str(&item.galley.job.text); text.push('\n'); }
            Shape::Vec(items) => for item in items { collect_text(item, text); },
            _ => {}
        }
    }

    #[test]
    fn failed_mod_updates_keep_their_error_and_manual_import_visible() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let entry = &catalogue::MODS[0];
        for installed in [None, Some("old version")] {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
            let mut bounds = Rect::NOTHING;
            let mut output = ctx.run_ui(input, |ui| {
                footer_ui(ui, entry, installed, false, Some(&Stage::Failed("Connection interrupted".into())), true, None, None);
                bounds = ui.min_rect();
            });
            output.textures_delta.clear();
            let mut text = String::new();
            for shape in output.shapes { collect_text(&shape.shape, &mut text); }
            assert!(text.contains("Connection interrupted"), "failed upgrades must explain the failure");
            assert!(text.contains("Have the file?"), "manual recovery remains available after a failed download");
            assert!(bounds.max.x <= 301.0, "wrapped controls fit a compact card: {bounds:?}");
        }
    }

    #[test]
    fn project_and_download_host_links_open_their_own_pages() {
        fn text_center(shape: &Shape, label: &str) -> Option<Pos2> {
            match shape {
                Shape::Text(text) if text.galley.job.text == label => {
                    // Wrapped inline links include leading space from preceding controls.
                    // Click visible text, not the galley center (which may be another button).
                    let row = text.galley.rows.iter().find(|row| !row.glyphs.is_empty())?;
                    let at = text.pos + row.rect_without_leading_space().center().to_vec2();
                    Some(at)
                }
                Shape::Vec(items) => items.iter().find_map(|shape| text_center(shape, label)),
                _ => None,
            }
        }
        let entry = catalogue::get("totalmayhem").unwrap();
        for (label, expected) in [("WEBSITE ↗", entry.page()), ("MODDB ↗", entry.source_page())] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            let input = || egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
            let mut bounds = Rect::NOTHING;
            let mut output = ctx.run_ui(input(), |ui| {
                footer_ui(ui, entry, None, false, None, true, None, None);
                bounds = ui.min_rect();
            });
            output.textures_delta.clear();
            let mut text = String::new();
            for shape in &output.shapes { collect_text(&shape.shape, &mut text); }
            assert!(text.contains("GET ·") && text.contains("Have the file?"), "project links retain download and import controls");
            assert!(bounds.max.x <= 301.0, "project and provider controls wrap within compact cards: {bounds:?}");
            let at = output.shapes.iter().find_map(|shape| text_center(&shape.shape, label)).expect("link is visible");
            let mut action = None;
            for pressed in [true, false] {
                let mut raw = input();
                raw.events = vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }];
                ctx.run_ui(raw, |ui| { action = footer_ui(ui, entry, None, false, None, true, None, None); }).drop_without_applying_deltas();
            }
            assert!(matches!(action, Some(CardAction::Open(ref url)) if url == &expected), "{label} opens its own page");
        }
    }

    #[test]
    fn manual_only_mod_card_keeps_website_and_import_without_download_controls() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let entry = catalogue::get("finalfrontier").unwrap();
        for installed in [None, Some("old version"), Some("7.4")] {
            for online in [false, true] {
                let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
                let mut bounds = Rect::NOTHING;
                let mut output = ctx.run_ui(input, |ui| {
                    footer_ui(ui, entry, installed, false, None, online, None, None);
                    bounds = ui.min_rect();
                });
                output.textures_delta.clear();
                let mut text = String::new();
                for shape in output.shapes { collect_text(&shape.shape, &mut text); }
                assert!(text.contains("WEBSITE"));
                assert_eq!(text.contains("Have the file?"), installed != Some("7.4"));
                assert_eq!(text.contains("PLAY"), installed.is_some());
                for unwanted in ["GET ·", "UPDATE ·", "DOWNLOAD LINKS", "Main download", "Backup download", "Optional downloads"] {
                    assert!(!text.contains(unwanted), "manual-only cards must not offer {unwanted}");
                }
                assert!(bounds.max.x <= 301.0, "manual controls fit compact cards: {bounds:?}");
            }
        }
    }

    #[test]
    fn unverified_project_keeps_import_and_installed_controls_without_stale_updates() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let entry = Box::leak(Box::new(Entry { file: None, fit: Fit::Unverified, ..*catalogue::get("escalation").unwrap() }));
        for installed in [None, Some("Gold 9.4.0"), Some("Gold 10.2.0"), Some("unverified")] {
            for online in [false, true] {
                let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
                let mut bounds = Rect::NOTHING;
                let mut output = ctx.run_ui(input, |ui| {
                    footer_ui(ui, entry, installed, true, None, online, None, None);
                    bounds = ui.min_rect();
                });
                output.textures_delta.clear();
                let mut text = String::new();
                for shape in output.shapes { collect_text(&shape.shape, &mut text); }
                assert!(text.contains("WEBSITE") && text.contains("Have the file?"));
                assert_eq!(text.contains("PLAY"), installed.is_some());
                assert_eq!(text.lines().any(|line| line == "ON"), installed.is_some(), "installed imports retain the enabled control");
                for unwanted in ["GET ·", "UPDATE ·", "MODDB", "DOWNLOAD LINKS"] {
                    assert!(!text.contains(unwanted), "unverified project cards must not offer {unwanted}");
                }
                assert!(bounds.max.x <= 301.0, "project controls fit compact cards: {bounds:?}");
            }
        }
        assert!(!crate::mod_library::plays_well(entry, false, &crate::validation::ValidationStatus::Unknown));
    }

    /// The card offers UPDATE only for an older copy than the catalogue's release: never to put
    /// the release over a newer copy, nor over the same written with a "v".
    #[test]
    fn update_is_offered_only_over_an_older_copy() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let entry = catalogue::get("totalmayhem").unwrap();
        let file = entry.file.unwrap();
        for (installed, offered) in [("0.9", true), (&*format!("v{}", file.version), false), ("99.0.0", false)] {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| { footer_ui(ui, entry, Some(installed), true, None, true, None, None); });
            output.textures_delta.clear();
            let mut text = String::new();
            for shape in output.shapes { collect_text(&shape.shape, &mut text); }
            assert_eq!(text.contains("UPDATE ·"), offered, "{installed} against {}: {text}", file.version);
        }
    }

    #[test]
    fn install_stages_read_the_same_everywhere() {
        let (fraction, label) = stage_progress(&Stage::Copying { got: 1 << 20, total: 4 << 20 });
        assert_eq!((fraction, label.as_str()), (Some(0.25), "Copying 1.0 MB / 4.0 MB"));
        assert_eq!(stage_progress(&Stage::Queued), (None, "WAITING FOR THE ONE BEFORE".to_string()));
        assert!(stage_progress(&Stage::Failed("x".into())).1.is_empty());
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(megabytes(26_035_480), "25 MB");
        assert_eq!(megabytes(899_690), "0.9 MB");
    }

    #[test]
    fn release_metadata_is_visible_for_manual_and_unavailable_mods() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        for (id, version, date, size) in [("escalation", "Gold 10.2.0", "2026-08-08", "335 MB"),
            ("totalmayhem", "11.3.0", "2024-06-20", "25 MB"), ("finalfrontier", "7.4", "Unknown", "8.2 MB"),
            ("czta", "Unknown", "Unknown", "Unknown")] {
            let mut bounds = Rect::NOTHING;
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 700.0))), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| { release_details(ui, catalogue::get(id).unwrap()); bounds = ui.min_rect(); });
            output.textures_delta.clear();
            let mut text = String::new();
            for shape in output.shapes { collect_text(&shape.shape, &mut text); }
            for wanted in [format!("Version: {version}"), format!("Released: {date}"), format!("Download: {size}")] {
                assert!(text.contains(&wanted), "{id}: missing {wanted}");
            }
            assert!(bounds.max.x <= 281.0, "metadata fits compact cards: {bounds:?}");
        }
    }
}
