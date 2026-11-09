//! The jukebox (J; the title screen's footer): a record turning as the music plays, the track's
//! name and its sound as a spectrum, how far in (a click or a drag along it goes elsewhere in
//! the track); back, stop or play, next; shuffle, repeat, the music's volume; TA's soundtrack,
//! each track by its part in the music (the battle themes, the peace themes, victory, defeat,
//! the briefing's), and the player's own music by its tags, each with its length, found by name;
//! a click plays one. How the player's music plays with TA's, and where it is kept.

use std::collections::{HashMap, HashSet};
use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use egui::{Align2, Color32, Painter, Pos2, Rect, RichText, Sense, Slider, Stroke, pos2, vec2};
use ta_config::UserMusic;
use ta_ui::i18n::{n, t, tf};
use ta_ui::panel::{self, Exit, Panel};
use ta_ui::scroll;
use ta_ui::widgets::{self as w, ActionButton};

use crate::theme::*;

/// What the jukebox asks of the music.
pub enum JukeboxRequest {
    /// Play this track now (a VFS path, or `file:` and a path on disk).
    Play(String),
    Previous,
    Next,
    Stop,
    Start,
    /// The track playing, from this far in (seconds).
    Seek(f64),
    /// The music's volume (0..1); `done` once the slider is let go (then it is kept).
    Volume { level: f32, done: bool },
    /// The next track at random (the settings change).
    Shuffle(bool),
    /// Each track again once it ends (the settings change).
    Repeat(bool),
    /// The player's music plays so with TA's (the settings change).
    Mode(UserMusic),
    /// Look for the player's music again.
    Rescan,
}

/// The music as the jukebox shows it.
pub struct Music<'a> {
    /// The track playing (a VFS path, or `file:` and a path on disk), and whether music is on.
    pub track: Option<&'a str>,
    pub on: bool,
    /// How far into the track (seconds).
    pub position: Option<f64>,
    /// How loud it is in each of [`BANDS`] bands of pitch (0..1), as it sounds now.
    pub bands: &'a [f32],
}

/// The spectrum's bands.
pub const BANDS: usize = 40;
/// The panel's width (its content's), and where its top is.
const WIDTH: f32 = 460.0;
const TOP: f32 = 60.0;

#[derive(Default)]
pub struct Jukebox {
    pub open: bool,
    /// The player's tracks and their titles (read when the jukebox opens, or on a rescan).
    tracks: Option<Vec<(PathBuf, String)>>,
    /// The track the list last scrolled to (it scrolls to the one playing when that changes,
    /// and when the jukebox opens).
    revealed: Option<String>,
    /// How long each track plays, by its path as `Play` takes it (`None`: it cannot be told),
    /// read with the titles on a worker thread as the list first shows them.
    lengths: HashMap<String, Option<f64>>,
    /// The tracks being read, and the worker's word.
    reading: HashSet<String>,
    read: Option<mpsc::Receiver<TrackMetadata>>,
    /// The spectrum as drawn: each band's bar (risen quickly, fallen slowly), and the mark
    /// over it of its last peak with how long that has held.
    bars: Vec<f32>,
    peaks: Vec<(f32, f32)>,
    /// How far the record has turned (radians).
    spin: f32,
    /// Finding a track by name.
    search: String,
    /// Where along the track the progress bar is being dragged to (a fraction), until let go.
    scrub: Option<f32>,
}

struct TrackMetadata {
    path: String,
    length: Option<f64>,
    title: Option<String>,
}

/// A track of TA's by its part in the music ("music/7.mp3" → "Battle theme 6").
pub fn ta_track_name(path: &str) -> String {
    let n: Option<u32> = path.rsplit('/').next().and_then(|f| f.split('.').next()).and_then(|s| s.parse().ok());
    let Some(n) = n else { return path.to_string() };
    let lists = ta_audio::music::Playlists::default();
    let find = |list: &[ta_audio::music::TrackRef]| list.iter().position(|r| *r == ta_audio::music::TrackRef::Number(n));
    if let Some(i) = find(&lists.battle) {
        tf("Battle theme {n}", &[("n", &(i + 1))])
    } else if let Some(i) = find(&lists.peace) {
        tf("Peace theme {n}", &[("n", &(i + 1))])
    } else if find(&lists.victory).is_some() {
        t("Victory").to_string()
    } else if find(&lists.defeat).is_some() {
        t("Defeat").to_string()
    } else if find(&lists.briefing).is_some() {
        t("Briefing").to_string()
    } else {
        tf("Track {n}", &[("n", &n)])
    }
}

impl Jukebox {
    /// Find the player's tracks. Their tags and lengths are read off the UI thread.
    pub fn rescan(&mut self, audio: &ta_config::Audio) -> Vec<PathBuf> {
        let files = ta_config::music_files(audio);
        self.replace_tracks(&files);
        files
    }

    fn replace_tracks(&mut self, files: &[PathBuf]) {
        // A file can have changed in place. Drop the old reader before invalidating its
        // cached lengths, so late results cannot restore the old title or duration.
        self.read = None;
        self.reading.clear();
        self.lengths.retain(|path, _| !path.starts_with("file:"));
        self.tracks = Some(files.iter().map(|p| {
            let title = p.file_stem().map(|s| s.to_string_lossy().replace(['_', '-'], " ").split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
            (p.clone(), title)
        }).collect());
        self.revealed = None;
    }

    /// A track's name: TA's by its part, the player's by its title.
    fn name_of(&self, path: &str) -> String {
        match path.strip_prefix("file:") {
            Some(file) => self.tracks.as_ref().and_then(|ts| ts.iter().find(|(p, _)| p.to_string_lossy() == file)).map_or_else(
                || std::path::Path::new(file).file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
                |(_, title)| title.clone(),
            ),
            None => ta_track_name(path),
        }
    }

    /// The tracks' lengths read since last asked; those not known yet (TA's from `source`,
    /// the player's from disk) sent to be read.
    fn read_metadata(&mut self, wanted: Vec<String>, source: &Arc<dyn ta_audio::SoundSource>) {
        if let Some(read) = &self.read {
            loop {
                match read.try_recv() {
                    Ok(TrackMetadata { path, length, title }) => {
                        if let Some(title) = title
                            && let Some(file) = path.strip_prefix("file:")
                            && let Some((_, shown)) = self.tracks.as_mut().and_then(|tracks| tracks.iter_mut().find(|(p, _)| p == std::path::Path::new(file)))
                        {
                            *shown = title;
                        }
                        self.reading.remove(&path);
                        self.lengths.insert(path, length);
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        // (Not read again, and again: their lengths are unknown.)
                        for path in std::mem::take(&mut self.reading) {
                            self.lengths.insert(path, None);
                        }
                        self.read = None;
                        break;
                    }
                }
            }
        }
        if self.read.is_some() {
            return;
        }
        let wanted: Vec<String> = wanted.into_iter().filter(|p| !self.lengths.contains_key(p)).collect();
        if wanted.is_empty() {
            return;
        }
        let (send, read) = mpsc::channel();
        let source = source.clone();
        let paths = wanted.clone();
        let reader = std::thread::Builder::new().name("ta-jukebox-metadata".into()).spawn(move || {
            for path in paths {
                let (length, title) = match path.strip_prefix("file:") {
                    Some(file) => {
                        let file = std::path::Path::new(file);
                        (ta_audio::length::file_seconds(file), Some(ta_audio::tags::title_of(file)))
                    }
                    None => (source.read(&path).as_deref().and_then(ta_audio::length::seconds), None),
                };
                if send.send(TrackMetadata { path, length, title }).is_err() {
                    return;
                }
            }
        });
        match reader {
            Ok(_) => {
                self.reading.extend(wanted);
                self.read = Some(read);
            }
            Err(e) => {
                log::warn!("could not read the music's metadata: {e}");
                self.lengths.extend(wanted.into_iter().map(|p| (p, None)));
            }
        }
    }

    /// A track's length, once known.
    fn length(&self, path: &str) -> Option<f64> {
        self.lengths.get(path).copied().flatten()
    }

    /// The spectrum's bars and peaks a frame (`dt` seconds) on, towards `bands` (none while
    /// silent).
    fn ease_spectrum(&mut self, bands: &[f32], playing: bool, dt: f32) {
        self.bars.resize(BANDS, 0.0);
        self.peaks.resize(BANDS, (0.0, 0.0));
        for (i, (bar, (peak, held))) in self.bars.iter_mut().zip(self.peaks.iter_mut()).enumerate() {
            let target = if playing { bands.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0) } else { 0.0 };
            *bar = if target > *bar { *bar + (target - *bar) * (1.0 - (-dt * 28.0).exp()) } else { (*bar - dt * 1.6).max(target) };
            if *bar >= *peak {
                (*peak, *held) = (*bar, 0.0);
            } else {
                *held += dt;
                if *held > 0.45 {
                    *peak = (*peak - dt * 0.8).max(*bar);
                }
            }
        }
    }

    /// The jukebox's panel, when open. `ta_tracks`: TA's music files (read from `source`);
    /// `music`: what plays, how far in and how it sounds; `audio`: the settings; `bottom`: the
    /// room to leave under it (the title screen's footer's).
    pub fn draw(&mut self, ctx: &egui::Context, ta_tracks: &[String], music: &Music, source: &Arc<dyn ta_audio::SoundSource>, audio: &ta_config::Audio, bottom: f32) -> Vec<JukeboxRequest> {
        let mut out = Vec::new();
        if !self.open {
            self.revealed = None;
            self.scrub = None;
            return out;
        }
        let (now, playing) = (music.track, music.on);
        let playing_now = now.filter(|_| playing).map(str::to_owned);
        let reveal = playing_now.is_some() && playing_now != self.revealed;
        self.revealed = playing_now;
        if self.tracks.is_none() {
            out.push(JukeboxRequest::Rescan);
        }
        // (TA's two short duplicates, 9 and 10, are left out.)
        let ta_list: Vec<&String> = ta_tracks.iter().filter(|p| !p.contains("/9.") && !p.contains("/10.")).collect();
        let user_keys: Vec<String> = self.tracks.as_deref().unwrap_or_default().iter().map(|(p, _)| format!("file:{}", p.display())).collect();
        if self.tracks.is_some() {
            self.read_metadata(ta_list.iter().map(|p| (*p).clone()).chain(user_keys.iter().cloned()).collect(), source);
        }
        // The record turns, and the spectrum moves, while the music plays (and settles after).
        let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 0.1);
        self.ease_spectrum(music.bands, playing, dt);
        if playing {
            self.spin = (self.spin + dt * TAU * 33.3 / 60.0) % TAU;
            ctx.request_repaint();
        } else if self.bars.iter().chain(self.peaks.iter().map(|p| &p.0)).any(|&b| b > 0.001) {
            ctx.request_repaint();
        } else if !self.reading.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        // How far into the track playing, of how long it is.
        let progress = now.filter(|_| playing).map(|p| (music.position.unwrap_or(0.0), self.length(p)));
        // The track playing: its name, and where it is in its list.
        let card = match now.filter(|_| playing) {
            Some(path) => {
                let place = match path.strip_prefix("file:") {
                    Some(_) => user_keys.iter().position(|k| k == path).map(|i| (n("YOUR MUSIC"), i, user_keys.len())),
                    None => ta_list.iter().position(|p| p.as_str() == path).map(|i| (n("TOTAL ANNIHILATION"), i, ta_list.len())),
                };
                let line = match place {
                    Some((list, i, count)) => tf("{list} · TRACK {n} OF {count}", &[("list", &t(list)), ("n", &(i + 1)), ("count", &count)]),
                    None => t("NOW PLAYING").to_string(),
                };
                (self.name_of(path), line, true)
            }
            None => (t("Silent").to_string(), t("Choose a track, or press play.").to_string(), false),
        };
        let mut close = false;
        // The room under the panel's top: the list as long as the rest leaves room for, and on a
        // short screen the panel's body scrolls over its buttons.
        let room = ctx.content_rect().height() - TOP - bottom;
        let yours = self.tracks.as_ref().map_or(0, |t| t.len());
        let (mut open_folder, mut look_again) = (false, false);
        egui::Area::new(egui::Id::new("jukebox")).anchor(Align2::RIGHT_TOP, [-14.0, TOP]).order(egui::Order::Foreground).show(ctx, |ui| {
            let page = Panel::new("JUKEBOX").width(WIDTH).exit(Exit::Close).max_height(room);
            let footer = |ui: &mut egui::Ui| {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    open_folder = ui.add(ActionButton::secondary("OPEN MUSIC FOLDER")).on_hover_text(t("Your music goes here, or in the folder chosen in the settings")).clicked();
                    look_again = ui.add(ActionButton::secondary("LOOK AGAIN")).clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(tf("{n} of your tracks", &[("n", &yours)])).font(body(11.5)).color(STEEL_DIM));
                    });
                });
            };
            let response = page.show_with_footer(ui, 44.0, |ui| {
                ui.set_width(WIDTH);
                // The record, the track's name, its sound.
                let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, 100.0), Sense::hover());
                deck_card(ui.painter(), rect, &card, self.spin, &self.bars, &self.peaks);
                ui.add_space(4.0);
                if let Some(seconds) = progress_bar(ui, progress, 30.0, &mut self.scrub) {
                    out.push(JukeboxRequest::Seek(seconds));
                }
                ui.add_space(4.0);
                // A player's transport, as the replay bar's: the track before, play or stop (brass,
                // the main one), another track; the order of play; the volume.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let size = vec2(44.0, 32.0);
                    if w::glyph_button(ui, w::Glyph::Previous, size, w::Tone::Secondary, true).on_hover_text(t("The track before")).clicked() {
                        out.push(JukeboxRequest::Previous);
                    }
                    let (glyph, tip, request) = if playing { (w::Glyph::Stop, t("Stop"), JukeboxRequest::Stop) } else { (w::Glyph::Play, t("Play"), JukeboxRequest::Start) };
                    if w::glyph_button(ui, glyph, size, w::Tone::Primary, true).on_hover_text(tip).clicked() {
                        out.push(request);
                    }
                    if w::glyph_button(ui, w::Glyph::Next, size, w::Tone::Secondary, true).on_hover_text(t("Another track")).clicked() {
                        out.push(JukeboxRequest::Next);
                    }
                    ui.add_space(10.0);
                    let toggle = |on: bool| if on { w::Tone::Primary } else { w::Tone::Secondary };
                    let shuffle_tip = if audio.music_shuffle { t("Shuffle: the next track at random") } else { t("Shuffle: off, the tracks in order") };
                    if w::glyph_button(ui, w::Glyph::Shuffle, vec2(38.0, 32.0), toggle(audio.music_shuffle), true).on_hover_text(shuffle_tip).clicked() {
                        out.push(JukeboxRequest::Shuffle(!audio.music_shuffle));
                    }
                    let repeat_tip = if audio.music_repeat { t("Repeat: each track again when it ends") } else { t("Repeat: off") };
                    if w::glyph_button(ui, w::Glyph::Repeat, vec2(38.0, 32.0), toggle(audio.music_repeat), true).on_hover_text(repeat_tip).clicked() {
                        out.push(JukeboxRequest::Repeat(!audio.music_repeat));
                    }
                    // The volume, at the right.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().slider_width = 92.0;
                        let mut level = audio.music.clamp(0.0, 1.0);
                        let slider = ui.add(Slider::new(&mut level, 0.0..=1.0).show_value(false)).on_hover_text(tf("Music volume: {n}%", &[("n", &((level * 100.0).round() as u32))]));
                        if slider.changed() || slider.drag_stopped() {
                            out.push(JukeboxRequest::Volume { level, done: !slider.dragged() });
                        }
                        let (glyph, _) = ui.allocate_exact_size(vec2(18.0, 32.0), Sense::hover());
                        w::paint_glyph(ui.painter(), w::Glyph::Speaker, glyph.center(), 1.0, if level > 0.0 { STEEL } else { STEEL_DIM });
                    });
                });
                ui.add_space(6.0);
                let mut mode = audio.user_music;
                ui.horizontal(|ui| {
                    ui.label(w::caption("PLAYS"));
                    if w::segmented(ui, &mut mode, &[(UserMusic::Off, "TA'S MUSIC"), (UserMusic::Mixed, "BOTH"), (UserMusic::Only, "YOURS")]).changed() {
                        out.push(JukeboxRequest::Mode(mode));
                    }
                });
                ui.add_space(6.0);
                w::search_field(ui, &mut self.search, t("Find a track"), WIDTH, None);
                ui.add_space(4.0);
                let found = |name: &str, search: &str| search.is_empty() || name.to_lowercase().contains(&search.to_lowercase());
                let search = self.search.trim().to_owned();
                let name_of = |p: &str| self.name_of(p);
                let list = (ui.available_height() - 12.0).clamp(100.0, 300.0);
                let area = egui::ScrollArea::vertical().max_height(list).auto_shrink([false, true]);
                scroll::well(ui, area, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    // Numbered, each with its length; the one playing with how far it is in.
                    let detail = |path: &str, on: bool| match (on, progress) {
                        (true, Some((at, length))) => {
                            let text = match length {
                                Some(l) => format!("{} / {}", clock(at.min(l)), clock(l)),
                                None => clock(at),
                            };
                            Some(RichText::new(text).font(strong(12.0)).color(AMBER))
                        }
                        _ => self.length(path).map(|l| RichText::new(clock(l)).font(body(12.5)).color(STEEL_DIM)),
                    };
                    let mut shown = 0;
                    ui.label(section(n("TOTAL ANNIHILATION"), ta_list.iter().map(|p| self.length(p))));
                    for (i, path) in ta_list.iter().enumerate() {
                        let name = name_of(path);
                        if !found(&name, &search) {
                            continue;
                        }
                        shown += 1;
                        let on = now == Some(path.as_str()) && playing;
                        let row = panel::numbered_row(ui, on, true, i + 1, ta_list.len(), RichText::new(name), detail(path, on));
                        if on && reveal {
                            row.scroll_to_me(Some(egui::Align::Center));
                        }
                        if row.clicked() {
                            out.push(JukeboxRequest::Play((*path).clone()));
                        }
                    }
                    ui.add_space(6.0);
                    ui.label(section(n("YOUR MUSIC"), user_keys.iter().map(|k| self.length(k))));
                    match self.tracks.as_deref() {
                        Some(tracks) if !tracks.is_empty() => {
                            for (i, (path, title)) in tracks.iter().enumerate() {
                                if !found(title, &search) {
                                    continue;
                                }
                                shown += 1;
                                let key = format!("file:{}", path.display());
                                let on = now == Some(key.as_str()) && playing;
                                let row = panel::numbered_row(ui, on, true, i + 1, tracks.len(), RichText::new(title), detail(&key, on));
                                if on && reveal {
                                    row.scroll_to_me(Some(egui::Align::Center));
                                }
                                if row.on_hover_text(path.display().to_string()).clicked() {
                                    out.push(JukeboxRequest::Play(key));
                                }
                            }
                        }
                        _ => {
                            ui.add(egui::Label::new(RichText::new(t("Put MP3, Ogg, FLAC, WAV or M4A files in your music folder, then look again.")).color(STEEL)).wrap());
                        }
                    }
                    if shown == 0 && !search.is_empty() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(t("No track is called that.")).color(STEEL));
                    }
                });
            }, footer);
            close = response.exit;
        });
        if open_folder {
            let dir = ta_config::music_folders(audio).into_iter().next_back().unwrap_or_else(|| ta_config::config_dir().join("music"));
            let _ = std::fs::create_dir_all(&dir);
            crate::photo::open_folder(&dir);
        }
        if look_again {
            self.tracks = None;
        }
        if close {
            self.open = false;
        }
        out
    }
}

/// A list's heading in the jukebox: its name, how many tracks, and how long they play when all
/// their lengths are known.
fn section(name: &str, lengths: impl Iterator<Item = Option<f64>>) -> RichText {
    let lengths: Vec<Option<f64>> = lengths.collect();
    let count = if lengths.len() == 1 { t("1 TRACK").to_string() } else { tf("{n} TRACKS", &[("n", &lengths.len())]) };
    let text = match lengths.iter().copied().sum::<Option<f64>>() {
        Some(total) if !lengths.is_empty() => format!("{} · {count} · {}", t(name), clock(total)),
        _ => format!("{} · {count}", t(name)),
    };
    w::caption(&text).color(AMBER)
}

/// The jukebox's deck: a record turning (`spin`) as the music plays, the track's name (`card`:
/// its name, the line over it, and whether it plays) and its sound as a spectrum (`bars`, with
/// `peaks`' marks), in a well.
fn deck_card(painter: &Painter, rect: Rect, card: &(String, String, bool), spin: f32, bars: &[f32], peaks: &[(f32, f32)]) {
    painter.add(ta_ui::metal::well(rect, INSET));
    let r = rect.height() / 2.0 - 9.0;
    let centre = pos2(rect.min.x + 12.0 + r, rect.center().y);
    record(painter, centre, r, spin, card.2);
    let left = centre.x + r + 16.0;
    let (name, line, on) = card;
    let caption = painter.layout_no_wrap(line.clone(), strong(11.5), if *on { AMBER } else { STEEL_DIM });
    painter.galley(pos2(left, rect.min.y + 10.0), caption, AMBER);
    let room = rect.max.x - 12.0 - left;
    let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), strong(19.0), if *on { TEXT_BRIGHT } else { STEEL });
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let title = painter.layout_job(job);
    painter.galley(pos2(left, rect.min.y + 27.0), title, TEXT_BRIGHT);
    spectrum(painter, Rect::from_min_max(pos2(left, rect.min.y + 54.0), pos2(rect.max.x - 12.0, rect.max.y - 9.0)), bars, peaks);
}

/// A record on its turntable: black vinyl in grooves, the light catching it (still, as the light
/// is), a brass label with its mark turning `spin` round (turning only while `on`, the music
/// playing).
fn record(painter: &Painter, centre: Pos2, r: f32, spin: f32, on: bool) {
    painter.circle_filled(centre + vec2(1.5, 2.0), r + 1.0, Color32::from_black_alpha(140));
    painter.circle_filled(centre, r, Color32::from_rgb(17, 17, 15));
    for i in 0..10 {
        let ring = r * (0.44 + 0.054 * i as f32);
        painter.circle_stroke(centre, ring, Stroke::new(0.8, Color32::from_white_alpha(if i % 3 == 0 { 16 } else { 8 })));
    }
    // The light on the grooves: two soft wedges, across from each other.
    for at in [-0.85f32, -0.85 + std::f32::consts::PI] {
        let wedge: Vec<Pos2> = std::iter::once(centre)
            .chain((0..=10).map(|i| {
                let a = at + (i as f32 / 10.0 - 0.5) * 0.5;
                centre + vec2(a.cos(), a.sin()) * r * 0.97
            }))
            .collect();
        painter.add(egui::Shape::convex_polygon(wedge, Color32::from_white_alpha(9), Stroke::NONE));
    }
    painter.circle_stroke(centre, r, Stroke::new(1.0, Color32::from_white_alpha(28)));
    // The label, its mark, the spindle.
    let label = r * 0.36;
    painter.circle_filled(centre, label, if on { AMBER_DIM } else { Color32::from_rgb(92, 76, 40) });
    painter.circle_stroke(centre, label * 0.82, Stroke::new(1.0, AMBER_DEEP));
    let mark = centre + vec2(spin.cos(), spin.sin()) * label * 0.6;
    painter.circle_filled(mark, label * 0.13, AMBER_DEEP);
    painter.circle_filled(centre, 2.4, Color32::from_rgb(10, 10, 9));
}

/// The music's sound, band by band: brass bars from a dark well, each with the mark of its last
/// peak over it.
fn spectrum(painter: &Painter, rect: Rect, bars: &[f32], peaks: &[(f32, f32)]) {
    painter.rect_filled(rect, 2.0, Color32::from_black_alpha(90));
    let inner = rect.shrink2(vec2(6.0, 5.0));
    let n = bars.len().max(1);
    let gap = 2.0;
    let width = ((inner.width() - gap * (n - 1) as f32) / n as f32).max(1.0);
    for (i, (&bar, &(peak, _))) in bars.iter().zip(peaks).enumerate() {
        let x = inner.min.x + i as f32 * (width + gap);
        let height = (bar * inner.height()).max(1.0);
        let r = Rect::from_min_max(pos2(x, inner.max.y - height), pos2(x + width, inner.max.y));
        painter.add(ta_ui::metal::vgrad(r, AMBER_BRIGHT, AMBER_DEEP));
        if peak > 0.02 {
            let y = inner.max.y - peak * inner.height();
            painter.rect_filled(Rect::from_min_max(pos2(x, y - 1.5), pos2(x + width, y)), 0.0, TEXT_BRIGHT.gamma_multiply(0.75));
        }
    }
}

/// A time as a clock reads it: "3:07", "1:02:45".
fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).floor() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// How far into the track playing (`progress`: the time in, and its length where known): the
/// time, a groove filled that far in brass, the length; an empty groove while silent. Where the
/// length is known, a click or a drag along it (`scrub`: where to, while dragged) goes elsewhere
/// in the track: the time to go to, once let go.
fn progress_bar(ui: &mut egui::Ui, progress: Option<(f64, Option<f64>)>, height: f32, scrub: &mut Option<f32>) -> Option<f64> {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
    let font = strong(12.0);
    let figures = |text: String, color| ui.painter().layout_no_wrap(text, font.clone(), color);
    // (Room for the longest times either side, so the groove keeps its place as they change.)
    let (at, length) = match progress {
        Some((at, length)) => (Some(length.map_or(at, |l| at.min(l))), length),
        None => (None, None),
    };
    let room = at.into_iter().chain(length).map(|time| figures(clock(time), TEXT).size().x)
        .fold(figures("00:00".into(), TEXT).size().x, f32::max);
    let groove = Rect::from_min_max(pos2(rect.min.x + room + 10.0, rect.center().y - 3.0), pos2(rect.max.x - room - 10.0, rect.center().y + 3.0));
    // Seeking.
    let mut sought = None;
    let length = length.filter(|&l| l > 0.0 && at.is_some());
    match length {
        Some(l) => {
            let along = |x: f32| ((x - groove.min.x) / groove.width().max(1.0)).clamp(0.0, 1.0);
            if (response.dragged() || response.is_pointer_button_down_on())
                && let Some(p) = response.interact_pointer_pos()
            {
                *scrub = Some(along(p.x));
            }
            if response.drag_stopped() || response.clicked() {
                let to = scrub.take().or_else(|| response.interact_pointer_pos().map(|p| along(p.x)));
                sought = to.map(|f| f64::from(f) * l);
            }
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        None => *scrub = None,
    }
    let shown = match (*scrub, length) {
        (Some(f), Some(l)) => Some(f64::from(f) * l),
        _ => at,
    };
    let painter = ui.painter();
    if let Some(at) = shown {
        let colour = if scrub.is_some() { AMBER_BRIGHT } else { TEXT_BRIGHT };
        let time = figures(clock(at), colour);
        painter.galley(pos2(rect.min.x + room - time.size().x, rect.center().y - time.size().y / 2.0), time, colour);
    }
    if let Some(l) = length {
        let total = figures(clock(l), STEEL);
        painter.galley(pos2(rect.max.x - room, rect.center().y - total.size().y / 2.0), total, STEEL);
    }
    painter.add(ta_ui::metal::well(groove, INSET));
    if let (Some(at), Some(l)) = (shown, length) {
        let x = groove.min.x + (groove.width() * (at / l) as f32).clamp(0.0, groove.width());
        let fill = Rect::from_min_max(groove.min + vec2(1.0, 1.0), pos2(x, groove.max.y - 1.0));
        if fill.width() > 0.5 {
            painter.add(ta_ui::metal::vgrad(fill, AMBER_BRIGHT, AMBER_DIM));
        }
        let knob = if response.hovered() || scrub.is_some() { 5.5 } else { 4.5 };
        painter.circle_filled(pos2(x, groove.center().y), knob + 1.0, Color32::from_black_alpha(140));
        painter.circle_filled(pos2(x, groove.center().y), knob, AMBER_BRIGHT);
    }
    sought
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_a_clock() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(64.4), "1:04");
        assert_eq!(clock(204.99), "3:24");
        assert_eq!(clock(3_765.0), "1:02:45");
    }

    /// The spectrum rises at once and falls slowly; its peaks hold, then fall; silent, all
    /// settles to nothing.
    #[test]
    fn the_spectrum_rises_quickly_and_settles_slowly() {
        let mut jukebox = Jukebox::default();
        let mut bands = [0.0f32; BANDS];
        bands[3] = 0.9;
        for _ in 0..10 {
            jukebox.ease_spectrum(&bands, true, 1.0 / 60.0);
        }
        assert!(jukebox.bars[3] > 0.85 && jukebox.bars[4] == 0.0, "{:?}", &jukebox.bars[..6]);
        jukebox.ease_spectrum(&[0.0; BANDS], true, 0.1);
        assert!(jukebox.bars[3] > 0.6 && jukebox.peaks[3].0 > 0.85, "the bar falls slowly, its peak holds");
        for _ in 0..200 {
            jukebox.ease_spectrum(&bands, false, 1.0 / 30.0);
        }
        assert!(jukebox.bars.iter().chain(jukebox.peaks.iter().map(|p| &p.0)).all(|&b| b == 0.0));
    }

    #[test]
    fn sections_count_their_tracks_and_total_their_lengths() {
        assert_eq!(section("YOUR MUSIC", [Some(60.0), Some(125.0)].into_iter()).text(), "YOUR MUSIC · 2 TRACKS · 3:05");
        assert_eq!(section("YOUR MUSIC", [Some(60.0), None].into_iter()).text(), "YOUR MUSIC · 2 TRACKS");
        assert_eq!(section("YOUR MUSIC", [Some(61.0)].into_iter()).text(), "YOUR MUSIC · 1 TRACK · 1:01");
    }

    #[test]
    fn tas_tracks_go_by_their_parts() {
        assert_eq!(ta_track_name("music/2.mp3"), "Battle theme 1");
        assert_eq!(ta_track_name("music/11.mp3"), "Peace theme 1");
        assert_eq!(ta_track_name("music/0.mp3"), "Victory");
        assert_eq!(ta_track_name("music/17.ogg"), "Briefing");
    }

    #[test]
    fn looking_again_replaces_stale_file_metadata_but_keeps_the_soundtrack_cache() {
        let file = PathBuf::from("my_track.wav");
        let key = format!("file:{}", file.display());
        let mut jukebox = Jukebox::default();
        jukebox.replace_tracks(std::slice::from_ref(&file));
        jukebox.lengths.insert(key.clone(), Some(12.0));
        jukebox.lengths.insert("music/2.mp3".into(), Some(90.0));
        let (old_send, old_read) = mpsc::channel();
        jukebox.read = Some(old_read);
        jukebox.reading.insert(key.clone());
        old_send.send(TrackMetadata { path: key.clone(), length: Some(12.0), title: Some("Old title".into()) }).unwrap();

        jukebox.replace_tracks(std::slice::from_ref(&file));
        assert!(jukebox.read.is_none());
        assert!(jukebox.reading.is_empty());
        assert_eq!(jukebox.length(&key), None);
        assert_eq!(jukebox.length("music/2.mp3"), Some(90.0));
        assert_eq!(jukebox.name_of(&key), "my track");
        assert!(old_send.send(TrackMetadata { path: key.clone(), length: Some(12.0), title: None }).is_err());

        let (send, read) = mpsc::channel();
        jukebox.read = Some(read);
        send.send(TrackMetadata { path: key.clone(), length: Some(25.0), title: Some("Updated title".into()) }).unwrap();
        drop(send);
        let source: Arc<dyn ta_audio::SoundSource> = Arc::new(ta_audio::MemorySource::default());
        jukebox.read_metadata(Vec::new(), &source);
        assert_eq!(jukebox.name_of(&key), "Updated title");
        assert_eq!(jukebox.length(&key), Some(25.0));
    }
}
