//! A mission's briefing as TA gave it, before the mission starts: an incoming transmission.
//! The narrator speaks (the install's `camps/briefs/*.wav`) while the words come up in time
//! with the voice, the voice's trace runs along the foot, and beside them the mission's
//! picture and the theatre of operations (the map, where your forces stand). Then the
//! objectives, and the mission begins when the player says so.

use egui::{Color32, Rect, RichText, Sense, Stroke, pos2, vec2};
use ta_data::mission::{BriefColor, Goal, Mission};
use ta_ui::i18n::{t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::widgets::{self as w, ActionButton};

use crate::setup::{Assets, GameSetup};
use crate::theme::*;

/// Characters a second the words come up at without a narrator.
const READING: f32 = 30.0;
/// The page's height outside its three columns (its header, the title, the foot's row and the
/// margins, with room to spare), and the columns' tallest.
const RESERVED: f32 = 330.0;
const MAX_COLUMN: f32 = 560.0;
/// Seconds before the words start (the transmission's beep).
const LEAD: f32 = 0.7;

/// What the briefing asks of the campaign page.
pub enum BriefingAction {
    None,
    Back,
    /// The player is ready: the mission starts.
    Begin,
    /// Play the narration again (Some) or stop it.
    Narrate(Option<String>),
}

/// The narrator's recording: where it is, how long it lasts and its loudness, sixty times a
/// second (the trace along the foot).
struct Narration {
    path: String,
    seconds: f32,
    envelope: Vec<f32>,
}

pub struct Briefing {
    /// The mission to start (made with the difficulty chosen).
    pub setup: GameSetup,
    side: String,
    number: usize,
    count: usize,
    title: String,
    planet: String,
    brief: Vec<(BriefColor, String)>,
    victory: Vec<Goal>,
    defeat: Vec<Goal>,
    glamour: Option<egui::TextureHandle>,
    /// Where the player's forces stand on the map.
    forces: Vec<(f32, f32)>,
    narration: Option<Narration>,
    /// When the transmission began (the interface's clock), or none until the first frame.
    started: Option<f64>,
    /// The player skipped to the end of the words.
    skipped: bool,
}

impl Briefing {
    #[allow(clippy::too_many_arguments)]
    pub fn new(assets: &Assets, setup: GameSetup, side: &str, number: usize, count: usize, title: &str, mission: &Mission, difficulty: &str, glamour: Option<egui::TextureHandle>) -> Briefing {
        let brief = ta_data::mission::briefing(&assets.vfs, &mission.brief).unwrap_or_default();
        let path = format!("camps/briefs/{}.wav", mission.narration.to_ascii_lowercase());
        let narration = (!mission.narration.is_empty()).then(|| assets.vfs.read_opt(&path)).flatten().and_then(|bytes| wav_envelope(&bytes)).map(|(seconds, envelope)| Narration { path, seconds, envelope });
        // The theatre: the player's units on the map (player 1 of the mission's file), and
        // its start.
        let schema = mission.schema(difficulty);
        let mut forces: Vec<(f32, f32)> = schema.map(|s| s.units.iter().filter(|u| u.player <= 1).map(|u| (u.x, u.z)).collect()).unwrap_or_default();
        if let Some(start) = schema.and_then(|s| s.start) {
            forces.push(start);
        }
        Briefing {
            setup,
            side: side.to_uppercase(),
            number,
            count,
            title: title.to_string(),
            planet: mission.planet.clone(),
            brief,
            victory: mission.victory.clone(),
            defeat: mission.defeat.clone(),
            glamour,
            forces,
            narration,
            started: None,
            skipped: false,
        }
    }

    /// The narration to play as the briefing opens.
    pub fn narration(&self) -> Option<String> {
        self.narration.as_ref().map(|n| n.path.clone())
    }

    /// How many of the words' characters are up `secs` into the transmission.
    fn shown(&self, secs: f32, total: usize) -> usize {
        if self.skipped {
            return total;
        }
        let f = match &self.narration {
            // (Narrators finish a little before their recording ends.)
            Some(n) => (secs - LEAD) / (n.seconds * 0.9 - LEAD).max(1.0),
            None => (secs - LEAD) * READING / total.max(1) as f32,
        };
        (f.clamp(0.0, 1.0) * total as f32) as usize
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, assets: &Assets) -> BriefingAction {
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        let started = *self.started.get_or_insert_with(|| {
            ta_ui::sound::play(&ctx, ta_ui::sound::UiSound::Message);
            // (Captures: `TA_BRIEF_AT=<seconds>` into the transmission.)
            now - std::env::var("TA_BRIEF_AT").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0)
        });
        let secs = (now - started) as f32;
        let total: usize = self.brief.iter().map(|(_, s)| s.chars().count()).sum();
        let shown = self.shown(secs, total);
        let done = shown >= total;
        if !done || self.narration.as_ref().is_some_and(|n| secs < n.seconds) {
            ctx.request_repaint();
        }
        let mut action = BriefingAction::None;
        let command = if self.side.starts_with("COR") { t("CORE COMMAND") } else { t("ARM COMMAND") };
        let subtitle = tf("{command} · MISSION {n} OF {count}", &[("command", &command), ("n", &self.number), ("count", &self.count)]);
        // Short screens get a smaller page and pictures; the columns are as tall as the screen
        // leaves (so the page is shown at its size, not shrunk to fit).
        let short = ctx.content_rect().height() < 1000.0;
        let side = if short { 300.0 } else { 360.0 };
        let budget = (ctx.content_rect().height() - RESERVED).clamp(260.0, MAX_COLUMN);
        let page = Panel::new("INCOMING TRANSMISSION").subtitle(&subtitle).width(if short { 1180.0 } else { panel::fit_width(&ctx, 1400.0) }).exit(Exit::Back);
        let response = panel::page(&ctx, "briefing", page, |ui| {
            // The title, as on the title screen, and where it is.
            ui.horizontal(|ui| {
                let lamp = 0.6 + 0.4 * ((secs * 5.0).sin() * 0.5 + 0.5);
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 30.0), Sense::hover());
                ui.painter().add(ta_ui::metal::lamp(r.center(), 5.0, GREEN, if done { 1.0 } else { lamp }));
                w::fitted_title(ui, &self.title, 30.0, TEXT_BRIGHT);
            });
            if !self.planet.is_empty() {
                ui.label(RichText::new(self.planet.to_uppercase()).font(strong(12.5)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
            }
            ui.add_space(6.0);
            // Three columns of one height: the mission's picture over its objectives, the words
            // as they come, the theatre of operations.
            let map = crate::map_preview::picture(&ctx, assets, &self.setup.map, MAX_COLUMN);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 22.0;
                let columns = if map.is_some() { 3.0 } else { 2.0 };
                let words_width = ui.available_width() - side * (columns - 1.0) - 22.0 * (columns - 1.0);
                let left = ui.vertical(|ui| {
                    ui.set_width(side);
                    let mut picture = 0.0;
                    if let Some(tex) = &self.glamour {
                        let s = tex.size_vec2();
                        let (rect, _) = ui.allocate_exact_size(s * (side / s.x.max(1.0)), Sense::hover());
                        ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                        ui.painter().add(ta_ui::metal::bezel(rect, 5.0, true));
                        ui.add_space(12.0);
                        picture = rect.height() + 12.0;
                    }
                    // The objectives from the start, faint, then bright once the words are done (a
                    // long list scrolls: six goals show whatever the screen).
                    panel::section(ui, "OBJECTIVES");
                    let a = ctx.animate_bool_with_time(egui::Id::new("briefing_objectives"), done, 0.4);
                    let area = egui::ScrollArea::vertical().id_salt("briefing_objectives").max_height((budget - picture - 50.0).max(170.0)).auto_shrink([false, true]);
                    ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Plate, |ui| {
                        ui.multiply_opacity(0.45 + 0.55 * a);
                        crate::campaign::objectives(ui, &self.victory, &self.defeat, assets);
                    });
                });
                // As tall as the left column, or as the map at the side's width needs (to the
                // screen's room).
                let map_need = map.as_ref().map_or(0.0, |p| 24.0 + side / p.aspect() + 22.0);
                let height = left.response.rect.height().max(map_need.min(budget));
                ui.vertical(|ui| {
                    ui.set_width(words_width);
                    panel::well(ui, |ui| {
                        let area = egui::ScrollArea::vertical().id_salt("briefing_words").max_height(height - 12.0).min_scrolled_height(height - 12.0).auto_shrink([false, false]).stick_to_bottom(!done);
                        ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Well, |ui| {
                            egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                if self.brief.is_empty() {
                                    ui.label(RichText::new(t("No briefing for this mission.")).color(STEEL));
                                } else {
                                    words(ui, &self.brief, shown, !done && (secs * 2.5) as i64 % 2 == 0);
                                }
                            });
                        });
                    });
                });
                if let Some(picture) = &map {
                    ui.vertical(|ui| {
                        ui.set_width(side);
                        panel::section(ui, "THEATRE OF OPERATIONS");
                        // (Its bezel 5 points round it.)
                        let tall = ((side - 10.0) / picture.aspect()).min(height - 56.0) + 10.0;
                        let (cell, _) = ui.allocate_exact_size(vec2(side, tall), Sense::hover());
                        let painter = ui.painter();
                        let placed = crate::map_preview::show(painter, picture, cell);
                        let rect = placed.rect;
                        // Your forces: each unit a dot of our colour, a ring pulsing round each
                        // group of them.
                        let ours = crate::palette::team_color(0);
                        let pulse = (secs * 2.2).fract();
                        let points: Vec<egui::Pos2> = self.forces.iter().map(|&(x, z)| placed.at(x, z)).filter(|p| rect.contains(*p)).collect();
                        for (centre, reach) in groups(&points, 16.0) {
                            painter.circle_stroke(centre, reach + 6.0 + pulse * 12.0, Stroke::new(1.5, ours.gamma_multiply(1.0 - pulse)));
                        }
                        for &p in &points {
                            painter.circle(p, 3.2, ours, Stroke::new(1.0, Color32::from_black_alpha(220)));
                        }
                        ui.label(RichText::new(t("Your forces")).font(body(11.5)).color(ours.lerp_to_gamma(Color32::WHITE, 0.35)));
                    });
                }
            });
            ui.add_space(12.0);
            // The voice's trace, the time, and what to do.
            ui.horizontal(|ui| {
                if let Some(n) = &self.narration {
                    trace(ui, &n.envelope, secs, n.seconds);
                    let at = secs.min(n.seconds);
                    ui.label(RichText::new(format!("{}:{:02} / {}:{:02}", at as u32 / 60, at as u32 % 60, n.seconds as u32 / 60, n.seconds as u32 % 60)).font(strong(12.0)).color(STEEL));
                    if ui.add(ActionButton::secondary("REPLAY")).on_hover_text(t("Hear the briefing again")).clicked() {
                        self.started = Some(now);
                        self.skipped = false;
                        action = BriefingAction::Narrate(Some(n.path.clone()));
                    }
                }
                if !done && ui.add(ActionButton::secondary("SKIP")).on_hover_text(t("All the words at once")).clicked() {
                    self.skipped = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if panel::hero_button(ui, "BEGIN MISSION", vec2(300.0, 56.0), true).clicked() {
                        action = BriefingAction::Begin;
                    }
                });
            });
        });
        if response.exit {
            action = BriefingAction::Back;
        }
        // Enter begins; Space shows the words at once.
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            action = BriefingAction::Begin;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Space)) {
            self.skipped = true;
        }
        action
    }
}

/// `points` gathered into groups: those within `link` of one another, and of those, together.
/// Each group's middle and how far its points reach from it.
fn groups(points: &[egui::Pos2], link: f32) -> Vec<(egui::Pos2, f32)> {
    // (Each point's group: the lowest index of those linked to it.)
    let mut group: Vec<usize> = (0..points.len()).collect();
    fn root(group: &mut [usize], mut i: usize) -> usize {
        while group[i] != i {
            group[i] = group[group[i]];
            i = group[i];
        }
        i
    }
    for i in 0..points.len() {
        for j in i + 1..points.len() {
            if points[i].distance(points[j]) <= link {
                let (a, b) = (root(&mut group, i), root(&mut group, j));
                group[a.max(b)] = a.min(b);
            }
        }
    }
    let mut members: std::collections::BTreeMap<usize, Vec<egui::Pos2>> = Default::default();
    for (i, &p) in points.iter().enumerate() {
        let r = root(&mut group, i);
        members.entry(r).or_default().push(p);
    }
    members
        .into_values()
        .map(|g| {
            let centre = (g.iter().fold(egui::Vec2::ZERO, |s, p| s + p.to_vec2()) / g.len() as f32).to_pos2();
            (centre, g.iter().map(|p| p.distance(centre)).fold(0.0, f32::max))
        })
        .collect()
}

/// The briefing's words up to `shown` characters, in TA's colours, with a cursor where they
/// stop while more are coming.
fn words(ui: &mut egui::Ui, brief: &[(BriefColor, String)], shown: usize, cursor: bool) {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let mut left = shown;
    for (color, text) in brief {
        if left == 0 {
            break;
        }
        let c = match color {
            BriefColor::Plain => TEXT,
            BriefColor::Yellow => AMBER_BRIGHT,
            BriefColor::Red => RED,
        };
        let part: String = text.chars().take(left).collect();
        left -= part.chars().count();
        job.append(&part, 0.0, egui::TextFormat { font_id: body(16.0), color: c, line_height: Some(23.0), ..Default::default() });
    }
    if cursor {
        job.append("▌", 0.0, egui::TextFormat { font_id: body(16.0), color: GREEN, line_height: Some(23.0), ..Default::default() });
    }
    ui.label(job);
}

/// The voice's trace: three seconds of its loudness running past a line, as a radio's scope
/// shows it, green where it has been heard.
fn trace(ui: &mut egui::Ui, envelope: &[f32], secs: f32, seconds: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(360.0, 40.0), Sense::hover());
    let painter = ui.painter();
    painter.add(ta_ui::metal::well(rect, INSET));
    let mid = rect.center().y;
    painter.line_segment([pos2(rect.min.x + 4.0, mid), pos2(rect.max.x - 4.0, mid)], Stroke::new(1.0, GREEN_DEEP));
    let bars = 90;
    let window = 3.0;
    for i in 0..bars {
        let t = secs - window + (i as f32 + 0.5) / bars as f32 * window;
        let v = if (0.0..seconds).contains(&t) { envelope.get((t * 60.0) as usize).copied().unwrap_or(0.0) } else { 0.0 };
        let x = rect.min.x + 6.0 + (rect.width() - 12.0) * i as f32 / bars as f32;
        let h = (v * (rect.height() * 0.42)).max(0.6);
        let a = 0.35 + 0.65 * (i as f32 / bars as f32);
        painter.line_segment([pos2(x, mid - h), pos2(x, mid + h)], Stroke::new(2.0, GREEN.gamma_multiply(a)));
    }
}

/// A WAV recording's length in seconds and its loudness sixty times a second (0..1, the
/// loudest 1). Uses the same decoder and channels as the narration that is played.
pub fn wav_envelope(bytes: &[u8]) -> Option<(f32, Vec<f32>)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let sound = ta_audio::decode::decode(bytes).ok()?.sound;
    let seconds = sound.duration() as f32;
    let per = (sound.sample_rate() as usize / 60).max(1) * sound.channels() as usize;
    let mut envelope: Vec<f32> = sound.samples().chunks(per)
        .map(|samples| (samples.iter().map(|sample| sample.powi(2)).sum::<f32>() / samples.len() as f32).sqrt())
        .collect();
    let loudest = envelope.iter().copied().fold(0.0f32, f32::max).max(1e-4);
    for v in &mut envelope {
        *v = (*v / loudest).sqrt();
    }
    Some((seconds, envelope))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
        let mut out = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        out.extend(16u32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(rate.to_le_bytes());
        out.extend((rate * 2).to_le_bytes());
        out.extend(2u16.to_le_bytes());
        out.extend(16u16.to_le_bytes());
        out.extend(b"data");
        out.extend((samples.len() as u32 * 2).to_le_bytes());
        for s in samples {
            out.extend(s.to_le_bytes());
        }
        out
    }

    #[test]
    fn forces_gather_into_groups() {
        let p = |x: f32, y: f32| egui::pos2(x, y);
        // A line of units each near the next is one group; one far off another.
        let g = groups(&[p(0.0, 0.0), p(10.0, 0.0), p(20.0, 0.0), p(30.0, 0.0), p(200.0, 0.0)], 16.0);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0], (p(15.0, 0.0), 15.0));
        assert_eq!(g[1], (p(200.0, 0.0), 0.0));
    }

    #[test]
    fn a_recording_has_its_length_and_its_loudness() {
        // One second: quiet, then loud.
        let samples: Vec<i16> = (0..22_050).map(|i| if i < 11_025 { (i % 7) as i16 * 10 } else { if i % 2 == 0 { 20_000 } else { -20_000 } }).collect();
        let (seconds, env) = wav_envelope(&wav(&samples, 22_050)).unwrap();
        assert!((seconds - 1.0).abs() < 0.01);
        // (Sixty a second, the last part-filled.)
        assert!((60..=61).contains(&env.len()), "{}", env.len());
        assert!(env[50] > 0.95 && env[5] < 0.2, "{} {}", env[50], env[5]);
    }

    #[test]
    fn a_briefing_trace_hears_both_stereo_channels() {
        let samples: Vec<i16> = (0..120).flat_map(|_| [0, 20_000]).collect();
        let mut bytes = wav(&samples, 120);
        bytes[22..24].copy_from_slice(&2u16.to_le_bytes());
        bytes[28..32].copy_from_slice(&480u32.to_le_bytes());
        bytes[32..34].copy_from_slice(&4u16.to_le_bytes());
        let (seconds, envelope) = wav_envelope(&bytes).unwrap();
        assert_eq!(seconds, 1.0);
        assert!(envelope.iter().all(|level| *level > 0.95), "a narrator in the right channel is audible");
    }

    #[test]
    fn malformed_narration_headers_do_not_crash_the_campaign_briefing() {
        // Twelve significant bits in a two-byte container used to index past the
        // final sample. The playback decoder can read that layout safely.
        let mut packed = wav(&[1], 120);
        packed[34..36].copy_from_slice(&12u16.to_le_bytes());
        assert!(wav_envelope(&packed).is_some());
        let mut unsupported = wav(&[1], 120);
        unsupported[20..22].copy_from_slice(&u16::MAX.to_le_bytes());
        let mut missing_format = b"RIFF\0\0\0\0WAVEdata".to_vec();
        missing_format.extend(2u32.to_le_bytes());
        missing_format.extend([0, 0]);
        for bytes in [unsupported, missing_format] {
            assert!(wav_envelope(&bytes).is_none(), "unreadable narration must fall back to text");
        }
    }

    #[test]
    fn the_install_has_its_narrations() {
        let Ok(install) = ta_vfs::install::discover(None) else {
            eprintln!("TA install not found; skipping");
            return;
        };
        let mut vfs = ta_vfs::Vfs::new();
        vfs.mount_game_dir(&install.root, "base").unwrap();
        let narrations: std::collections::BTreeSet<String> = ta_data::mission::campaigns(&vfs).into_iter()
            .flat_map(|campaign| campaign.missions).filter_map(|entry| Mission::load(&vfs, &entry.file))
            .filter(|mission| !mission.narration.is_empty()).map(|mission| mission.narration).collect();
        assert!(!narrations.is_empty());
        let (mut decoded, mut missing) = (0, 0);
        for name in &narrations {
            let path = format!("camps/briefs/{}.wav", name.to_ascii_lowercase());
            let Some(bytes) = vfs.read_opt(&path) else {
                // A few Battle Tactics entries name a recording that wasn't shipped.
                // Briefing::new keeps their text and uses its unvoiced reading pace.
                missing += 1;
                continue;
            };
            let (seconds, envelope) = wav_envelope(&bytes).unwrap_or_else(|| panic!("{path} must decode"));
            assert!(seconds > 0.0 && !envelope.is_empty(), "{path} has no narration");
            assert!(envelope.iter().all(|level| level.is_finite() && (0.0..=1.0).contains(level)), "{path} has an invalid trace");
            decoded += 1;
        }
        assert!(decoded > 0);
        println!("{decoded} campaign narrations have readable lengths and traces; {missing} absent recordings use text");
    }
}
