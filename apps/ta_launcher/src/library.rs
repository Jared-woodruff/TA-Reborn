//! The library pages' shared parts (Maps first; Mods and Skins follow): a search box that
//! answers Ctrl+F and Escape, a grid of picture cards drawn only where they show, list rows
//! with a picture, an item's context menu ([`menu`]), and pictures made on worker threads
//! (those on show first) and kept on disk, textures held only for as many as can be on show.

pub mod menu;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread::JoinHandle;

use egui::{Color32, CursorIcon, Id, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2, pos2, vec2};

use crate::theme::*;
use crate::widgets;

/// The search box of a library page, `width` wide. Ctrl+F puts the cursor in it when nothing
/// else has the keys (`shortcuts`); Escape clears it, and a second Escape gives the keys back.
/// `hint` is translated.
pub fn search(ui: &mut Ui, query: &mut String, id: Id, hint: &str, width: f32, shortcuts: bool) -> Response {
    let shortcuts = shortcuts && !egui::Popup::is_any_open(ui.ctx());
    let find = shortcuts && ui.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
    if find { ui.memory_mut(|memory| memory.request_focus(id)); }
    if shortcuts && ui.memory(|memory| memory.has_focus(id) || (memory.focused().is_none() && memory.had_focus_last_frame(id)))
        && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        if query.is_empty() { ui.memory_mut(|memory| memory.surrender_focus(id)); }
        else {
            query.clear();
            // (Escape clears egui's focus before widgets run: Find stays ready for typing.)
            ui.memory_mut(|memory| memory.request_focus(id));
        }
    }
    let response = widgets::search_field(ui, query, hint, width, Some(id));
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_label(hint));
    if find || response.gained_focus() { response.scroll_to_me(None); }
    response
}

// ---- Pictures -----------------------------------------------------------------------------------

/// A picture made for a card, and the size of what it shows where the maker learnt it on the
/// way (a map's cells).
pub struct Picture {
    pub image: egui::ColorImage,
    pub size: Option<(u32, u32)>,
}

type Make = Box<dyn FnOnce() -> anyhow::Result<Picture> + Send>;
type Made = (String, Result<Picture, String>);

struct Queue {
    jobs: Mutex<VecDeque<(String, Make)>>,
    ready: Condvar,
    stop: AtomicBool,
}

/// Pictures made on worker threads, those asked for to show ahead of those only looked
/// ahead for (`prefetch`: the maker caches them and their size is kept). A bounded queue;
/// textures for at most `cap` pictures, the least recently shown let go first.
pub struct Pictures {
    name: &'static str,
    threads: usize,
    cap: usize,
    queue: Option<Arc<Queue>>,
    workers: Vec<JoinHandle<()>>,
    results: Option<mpsc::Receiver<Made>>,
    pending: HashSet<String>,
    show: HashSet<String>,
    failed: HashSet<String>,
    textures: HashMap<String, (TextureHandle, u64)>,
    sizes: HashMap<String, (u32, u32)>,
    frame: u64,
    made: u64,
}

const QUEUE_CAP: usize = 512;

impl Pictures {
    pub fn new(name: &'static str, threads: usize, cap: usize) -> Self {
        Self { name, threads, cap, queue: None, workers: Vec::new(), results: None, pending: HashSet::new(), show: HashSet::new(),
            failed: HashSet::new(), textures: HashMap::new(), sizes: HashMap::new(), frame: 0, made: 0 }
    }

    /// The picture for `key`, if made (it counts as shown this frame).
    pub fn get(&mut self, key: &str) -> Option<TextureHandle> {
        let frame = self.frame;
        self.textures.get_mut(key).map(|(texture, used)| { *used = frame; texture.clone() })
    }

    /// Whether `key`'s picture is made, without counting it as shown.
    pub fn has(&self, key: &str) -> bool { self.textures.contains_key(key) }

    /// Whether `key` still needs asking for to show: not made, not on its way to show, not failed.
    pub fn wanted(&self, key: &str) -> bool {
        !self.textures.contains_key(key) && !self.show.contains(key) && !self.failed.contains(key)
    }

    pub fn failed(&self, key: &str) -> bool { self.failed.contains(key) }

    /// What a maker learnt of `key`'s size, kept after its texture is let go.
    pub fn size(&self, key: &str) -> Option<(u32, u32)> { self.sizes.get(key).copied() }

    /// How many pictures (or sizes) have come in: a page's cached order changes with it.
    pub fn generation(&self) -> u64 { self.made }

    /// Asks for `key`'s picture to show: made by `make` on a worker, ahead of everything asked
    /// for before.
    pub fn request(&mut self, key: String, make: impl FnOnce() -> anyhow::Result<Picture> + Send + 'static) {
        if !self.wanted(&key) { return; }
        let Some(queue) = self.queue() else { return };
        let mut jobs = queue.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if self.pending.contains(&key) {
            // (Looked ahead for and not yet started: it moves to the front.)
            if let Some(at) = jobs.iter().position(|(queued, _)| *queued == key)
                && let Some(job) = jobs.remove(at) {
                jobs.push_front(job);
            }
        } else {
            jobs.push_front((key.clone(), Box::new(make)));
            self.pending.insert(key.clone());
        }
        self.show.insert(key);
        self.trim(&mut jobs);
        queue.ready.notify_one();
    }

    /// Drops the pictures still waiting their turn (not those being made, which come in as
    /// usual): their makers let go of what they hold, the maps' mounted archives.
    pub fn drop_queued(&mut self) {
        let Some(queue) = &self.queue else { return };
        let mut jobs = queue.jobs.lock().unwrap_or_else(|e| e.into_inner());
        for (key, _) in jobs.drain(..) {
            self.pending.remove(&key);
            self.show.remove(&key);
        }
    }

    /// Forgets which pictures could not be made, so that they are tried again (the content they
    /// are made from read again).
    pub fn forget_failures(&mut self) { self.failed.clear(); }

    /// Looks ahead for `key`: made after everything asked for to show, its size kept, no texture.
    pub fn prefetch(&mut self, key: String, make: impl FnOnce() -> anyhow::Result<Picture> + Send + 'static) {
        if self.textures.contains_key(&key) || self.pending.contains(&key) || self.failed.contains(&key) || self.sizes.contains_key(&key) { return; }
        let Some(queue) = self.queue() else { return };
        let mut jobs = queue.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.push_back((key.clone(), Box::new(make)));
        self.pending.insert(key);
        self.trim(&mut jobs);
        queue.ready.notify_one();
    }

    /// (A long scroll asks for more than can be made: the last in line give way.)
    fn trim(&mut self, jobs: &mut VecDeque<(String, Make)>) {
        while jobs.len() > QUEUE_CAP {
            if let Some((old, _)) = jobs.pop_back() {
                self.pending.remove(&old);
                self.show.remove(&old);
            }
        }
    }

    fn queue(&mut self) -> Option<Arc<Queue>> {
        if self.workers.is_empty() {
            let queue = Arc::new(Queue { jobs: Mutex::new(VecDeque::new()), ready: Condvar::new(), stop: AtomicBool::new(false) });
            let (send, receive) = mpsc::channel();
            for n in 0..self.threads.max(1) {
                let (queue, send) = (queue.clone(), send.clone());
                match std::thread::Builder::new().name(format!("{} pictures {n}", self.name)).spawn(move || work(&queue, &send)) {
                    Ok(worker) => self.workers.push(worker),
                    Err(error) => log::warn!("could not start a picture worker: {error}"),
                }
            }
            if self.workers.is_empty() { return None; }
            self.queue = Some(queue);
            self.results = Some(receive);
        }
        self.queue.clone()
    }

    /// Takes in the pictures made since the last frame (a few a frame, so that a burst of them
    /// does not stall one) and lets go of the least recently shown beyond the cap.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        let Some(results) = &self.results else { return };
        let arrived: Vec<Made> = std::iter::from_fn(|| results.try_recv().ok()).take(16).collect();
        for (key, result) in arrived {
            self.pending.remove(&key);
            let show = self.show.remove(&key);
            self.made += 1;
            match result {
                Ok(picture) => {
                    if let Some(size) = picture.size { self.sizes.insert(key.clone(), size); }
                    if show {
                        let texture = ctx.load_texture(format!("{}:{key}", self.name), picture.image, egui::TextureOptions::LINEAR);
                        self.textures.insert(key, (texture, self.frame));
                    }
                }
                Err(error) => {
                    log::debug!("{} picture {key}: {error}", self.name);
                    self.failed.insert(key);
                }
            }
        }
        if self.textures.len() > self.cap {
            let mut used: Vec<_> = self.textures.iter().map(|(key, (_, frame))| (*frame, key.clone())).collect();
            used.sort();
            for (_, key) in used.into_iter().take(self.textures.len() - self.cap) { self.textures.remove(&key); }
        }
        if !self.pending.is_empty() { ctx.request_repaint_after(std::time::Duration::from_millis(30)); }
    }

    /// Nothing asked for is still being made.
    pub fn idle(&self) -> bool { self.pending.is_empty() }

    /// Drops the asks not yet started (another picture is wanted instead).
    pub fn cancel_queued(&mut self) {
        if let Some(queue) = &self.queue {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|e| e.into_inner());
            for (key, _) in jobs.drain(..) {
                self.pending.remove(&key);
                self.show.remove(&key);
            }
        }
    }
}

fn work(queue: &Queue, send: &mpsc::Sender<Made>) {
    loop {
        let (key, make) = {
            let mut jobs = queue.jobs.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if queue.stop.load(Ordering::Relaxed) { return; }
                if let Some(job) = jobs.pop_front() { break job; }
                jobs = queue.ready.wait(jobs).unwrap_or_else(|e| e.into_inner());
            }
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(make))
            .unwrap_or_else(|_| Err(anyhow::anyhow!("the picture could not be made")))
            .map_err(|error| format!("{error:#}"));
        if send.send((key, result)).is_err() { return; }
    }
}

impl Drop for Pictures {
    fn drop(&mut self) {
        if let Some(queue) = &self.queue {
            queue.stop.store(true, Ordering::Relaxed);
            queue.jobs.lock().unwrap_or_else(|e| e.into_inner()).clear();
            queue.ready.notify_all();
        }
        self.results = None;
        for worker in self.workers.drain(..) { let _ = worker.join(); }
    }
}

/// The pictures' disk cache folder for `kind` under the launcher's data folder.
pub fn cache_dir(data: &Path, kind: &str) -> PathBuf {
    data.join("cache").join("pictures").join(kind)
}

/// Where the picture cached under `key` (its content's identity) is kept in `dir`.
pub fn cache_file(dir: &Path, key: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    dir.join(format!("{:x}.jpg", Sha256::digest(key.as_bytes())))
}

/// Reads a cached picture, if there is one.
pub fn read_cached(path: &Path) -> Option<egui::ColorImage> {
    read_picture(path, 16 << 20)
}

/// Cached and benchmark pictures are user-writable. Bound the encoded file and
/// decoder allocations before reading pixels, including compressed image bombs.
pub(crate) fn read_picture(path: &Path, max_bytes: u64) -> Option<egui::ColorImage> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > max_bytes { return None; }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > max_bytes { return None; }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 << 20);
    reader.limits(limits);
    let picture = reader.decode().ok()?.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([picture.width() as usize, picture.height() as usize], picture.as_raw()))
}

/// Writes a picture to the cache (best effort: a picture not kept is made again next time).
pub fn write_cached(path: &Path, picture: &egui::ColorImage) {
    use std::io::Write;
    let Some(folder) = path.parent() else { return };
    if std::fs::create_dir_all(folder).is_err() { return; }
    let rgb: Vec<u8> = picture.pixels.iter().flat_map(|p| { let [r, g, b, _] = p.to_array(); [r, g, b] }).collect();
    // (Written beside and moved in, so that no reader sees half a file.)
    let partial = path.with_extension(format!("{}.part", std::process::id()));
    let written = std::fs::File::create(&partial).ok().and_then(|file| {
        let mut writer = std::io::BufWriter::new(file);
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 90);
        encoder.encode(&rgb, picture.size[0] as u32, picture.size[1] as u32, image::ExtendedColorType::Rgb8).ok()?;
        writer.flush().ok()
    });
    if written.is_none() || std::fs::rename(&partial, path).is_err() { let _ = std::fs::remove_file(&partial); }
}

// ---- The grid ---------------------------------------------------------------------------------

/// How a grid lays out: as many columns as fit cards at least `least` wide (the cards sharing
/// the width), each `ratio` times as tall as wide plus `extra` points for its words.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout { pub columns: usize, pub card: Vec2, pub gap: f32 }

pub fn layout(width: f32, least: f32, ratio: f32, extra: f32, gap: f32) -> Layout {
    let columns = (((width + gap) / (least + gap)).floor() as usize).max(1);
    let card_width = ((width - gap * (columns - 1) as f32) / columns as f32).floor().max(1.0);
    Layout { columns, card: vec2(card_width, (card_width * ratio + extra).round()), gap }
}

/// A list's layout: one column of rows `height` tall.
pub fn rows(width: f32, height: f32, gap: f32) -> Layout {
    Layout { columns: 1, card: vec2(width.max(1.0), height), gap }
}

/// The cards still to come while a library is looked through: the grid's shapes in the dark,
/// each card's picture well and the bars of its words, fading down the page, a faint light
/// passing over them (held still under reduced motion and in the background), `height` tall.
/// `well` is the pictures' height to their width.
pub fn skeleton(ui: &mut Ui, layout: Layout, height: f32, well: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height.max(1.0)), Sense::hover());
    if !ui.is_rect_visible(rect) { return; }
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
    let pitch = layout.card.y + layout.gap;
    let rows = (((rect.height() + layout.gap) / pitch).ceil() as usize).max(1);
    for row in 0..rows {
        let fade = 1.0 - row as f32 / (rows as f32 + 0.5);
        for column in 0..layout.columns {
            let card = Rect::from_min_size(rect.min + vec2(column as f32 * (layout.card.x + layout.gap), 4.0 + row as f32 * pitch), layout.card);
            painter.rect_filled(card, 3.0, PANEL.gamma_multiply(0.8 * fade));
            let picture = Rect::from_min_size(card.min + vec2(6.0, 6.0), vec2(card.width() - 12.0, ((card.width() - 12.0) * well).round()));
            painter.rect_filled(picture, 2.0, INSET.gamma_multiply(fade));
            let y = picture.max.y + 10.0;
            painter.rect_filled(Rect::from_min_size(pos2(card.min.x + 8.0, y), vec2(card.width() * 0.55, 8.0)), 2.0, LINE.gamma_multiply(0.7 * fade));
            painter.rect_filled(Rect::from_min_size(pos2(card.min.x + 8.0, y + 15.0), vec2(card.width() * 0.35, 6.0)), 2.0, LINE.gamma_multiply(0.45 * fade));
        }
    }
    if !ta_ui::motion::still(ui.ctx()) {
        // (A band of light, leaning, crossing every 1.8 s.)
        let k = (ui.input(|i| i.time) % 1.8 / 1.8) as f32;
        let x = rect.min.x - 240.0 + (rect.width() + 480.0) * k;
        let band = vec![pos2(x, rect.min.y), pos2(x + 140.0, rect.min.y), pos2(x + 20.0, rect.max.y), pos2(x - 120.0, rect.max.y)];
        painter.add(egui::Shape::convex_polygon(band, Color32::from_white_alpha(7), Stroke::NONE));
        ta_ui::motion::next_frame(ui.ctx(), std::time::Duration::from_millis(33));
    }
}

/// The width a grid's cards share in `width` (the scroll lane kept beside them).
pub fn grid_width(width: f32) -> f32 { (width - ta_ui::scroll::LANE - 2.0).max(1.0) }

/// A grid of `count` cards in a scroll area `height` high, drawn only where they show (and a
/// row beyond each edge, so that the arrow keys find the next one): `item(ui, index, rect)`
/// draws and senses one. `reveal` brings a card's row to the top. Returns the card with the keys.
pub fn grid(ui: &mut Ui, id: Id, count: usize, layout: Layout, height: f32, reveal: Option<usize>,
    mut item: impl FnMut(&mut Ui, usize, Rect) -> Response) -> Option<usize>
{
    // (Room round the cards for a chosen one's frame and the focus ring.)
    const INSET: f32 = 4.0;
    let rows = count.div_ceil(layout.columns);
    let pitch = layout.card.y + layout.gap;
    let total = (rows as f32 * pitch - layout.gap).max(0.0) + 2.0 * INSET;
    let height = height.max(1.0);
    let mut area = egui::ScrollArea::vertical().id_salt(id).max_height(height).min_scrolled_height(height).auto_shrink([false, false]);
    if let Some(index) = reveal.filter(|index| *index < count) {
        let row = (index / layout.columns) as f32;
        area = area.vertical_scroll_offset((row * pitch).clamp(0.0, (total - height).max(0.0)));
    }
    let mut focused = None;
    ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Backdrop(BG), |ui| {
        let top = ui.max_rect().top() + INSET;
        let left = ui.max_rect().left();
        let clip = ui.clip_rect();
        ui.set_height(total);
        ui.set_width(ui.available_width());
        let first = (((clip.top() - top) / pitch).floor() as isize - 1).max(0) as usize;
        let last = ((((clip.bottom() - top) / pitch).ceil() as isize + 1).max(0) as usize).min(rows);
        for row in first..last {
            for column in 0..layout.columns {
                let index = row * layout.columns + column;
                if index >= count { break; }
                let rect = Rect::from_min_size(pos2(left + column as f32 * (layout.card.x + layout.gap), top + row as f32 * pitch), layout.card);
                if item(ui, index, rect).has_focus() { focused = Some(index); }
            }
        }
    });
    focused
}

/// A section of a grid in [`sections`]: its heading (translated where it is drawn), a note beside
/// it, how many cards it holds and its colour; a star marks a heading of favourites.
pub struct Section<'a> {
    pub title: &'a str,
    pub note: Option<&'a str>,
    pub count: usize,
    pub accent: Color32,
    pub star: bool,
}

/// The height of a section's heading in [`sections`].
const HEADING: f32 = 42.0;

/// What a row of [`sections`] holds: a section's heading, or cards (the first's index, how many).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Heading(usize),
    Cards(usize, usize),
}

/// The rows of [`sections`] (each with its top, from the grid's top) and the grid's height: under
/// a heading each where two sections or more hold cards (a section of none left out), the cards'
/// indexes running on from one section to the next.
fn section_rows(sections: &[Section<'_>], layout: Layout) -> (Vec<(f32, Row)>, f32) {
    let shown: Vec<usize> = (0..sections.len()).filter(|&k| sections[k].count > 0).collect();
    let headings = shown.len() > 1;
    let pitch = layout.card.y + layout.gap;
    let (mut rows, mut y, mut first) = (Vec::new(), 0.0, 0);
    for k in shown {
        if headings {
            rows.push((y, Row::Heading(k)));
            y += HEADING;
        }
        let count = sections[k].count;
        for row in 0..count.div_ceil(layout.columns) {
            rows.push((y, Row::Cards(first + row * layout.columns, (count - row * layout.columns).min(layout.columns))));
            y += pitch;
        }
        first += count;
        // (A little more room before the next section's heading.)
        if headings {
            y += layout.gap;
        }
    }
    // (Down to the last row's foot.)
    let height = rows.last().map_or(0.0, |(y, row)| y + if matches!(row, Row::Heading(_)) { HEADING } else { layout.card.y });
    (rows, height)
}

/// [`grid`] in sections, each under its heading where two or more hold cards: the cards'
/// indexes run on from one section to the next, and `item` and `reveal` go by them as
/// [`grid`]'s do. Returns the card with the keys.
pub fn sections(ui: &mut Ui, id: Id, sections: &[Section<'_>], layout: Layout, height: f32, reveal: Option<usize>,
    mut item: impl FnMut(&mut Ui, usize, Rect) -> Response) -> Option<usize>
{
    const INSET: f32 = 4.0;
    let (rows, length) = section_rows(sections, layout);
    let total = length + 2.0 * INSET;
    let height = height.max(1.0);
    let mut area = egui::ScrollArea::vertical().id_salt(id).max_height(height).min_scrolled_height(height).auto_shrink([false, false]);
    if let Some(index) = reveal {
        // (The row the card is in, and its section's heading over it where it is the first.)
        let at = rows.iter().position(|(_, row)| matches!(row, Row::Cards(first, count) if (*first..first + count).contains(&index)));
        if let Some(at) = at {
            let top = if at > 0 && matches!(rows[at - 1].1, Row::Heading(_)) { rows[at - 1].0 } else { rows[at].0 };
            area = area.vertical_scroll_offset(top.clamp(0.0, (total - height).max(0.0)));
        }
    }
    let mut focused = None;
    ta_ui::scroll::show(ui, area, ta_ui::scroll::Behind::Backdrop(BG), |ui| {
        let top = ui.max_rect().top() + INSET;
        let left = ui.max_rect().left();
        let width = ui.available_width();
        let clip = ui.clip_rect();
        ui.set_height(total);
        ui.set_width(width);
        // (A row beyond each edge drawn too, so that the arrow keys find the next card.)
        let reach = layout.card.y + layout.gap;
        for &(y, row) in rows.iter().filter(|(y, _)| top + y + reach * 2.0 >= clip.top() && top + y - reach <= clip.bottom()) {
            match row {
                Row::Heading(k) => heading(ui, Rect::from_min_size(pos2(left, top + y), vec2(width, HEADING)), &sections[k]),
                Row::Cards(first, count) => {
                    for column in 0..count {
                        let rect = Rect::from_min_size(pos2(left + column as f32 * (layout.card.x + layout.gap), top + y), layout.card);
                        if item(ui, first + column, rect).has_focus() { focused = Some(first + column); }
                    }
                }
            }
        }
    });
    focused
}

/// A section's heading in `rect`: its title in its colour (a star before a heading of
/// favourites), how many it holds, its note, over a rule led in its colour.
fn heading(ui: &Ui, rect: Rect, section: &Section<'_>) {
    if !ui.is_rect_visible(rect) { return; }
    let painter = ui.painter();
    let base = rect.bottom() - 10.0;
    let mut x = rect.left() + 2.0;
    if section.star {
        star(painter, pos2(x + 7.0, base - 11.0), 6.5, section.accent);
        x += 20.0;
    }
    let title = widgets::galley(ui, RichText::new(ta_ui::i18n::t(section.title)).font(strong(14.5)).color(section.accent).extra_letter_spacing(ta_ui::i18n::tracking(1.6)));
    painter.galley(pos2(x, base - 4.0 - title.size().y), title.clone(), section.accent);
    x += title.size().x + 10.0;
    let count = widgets::galley(ui, RichText::new(section.count.to_string()).font(strong(12.0)).color(STEEL));
    painter.galley(pos2(x, base - 5.0 - count.size().y), count.clone(), STEEL);
    x += count.size().x + 14.0;
    if let Some(note) = section.note {
        let room = rect.right() - x - 4.0;
        if room > 80.0 {
            let mut job = egui::text::LayoutJob::single_section(ta_ui::i18n::t(note).to_string(), egui::TextFormat { font_id: body(12.0), color: STEEL, ..Default::default() });
            job.wrap = egui::text::TextWrapping::truncate_at_width(room);
            let galley = painter.layout_job(job);
            painter.galley(pos2(x, base - 5.0 - galley.size().y), galley, STEEL);
        }
    }
    painter.hline(rect.x_range(), base + 0.5, Stroke::new(1.0, LINE));
    painter.hline(rect.left()..=rect.left() + 64.0, base + 0.5, Stroke::new(2.0, section.accent));
}

#[cfg(test)]
mod section_tests {
    use super::*;

    fn section(count: usize) -> Section<'static> {
        Section { title: "T", note: None, count, accent: Color32::WHITE, star: false }
    }

    #[test]
    fn sections_head_their_cards_and_number_them_on() {
        let layout = Layout { columns: 3, card: vec2(100.0, 80.0), gap: 10.0 };
        // Two sections hold cards (the empty one is left out): each under its heading.
        let (rows, height) = section_rows(&[section(4), section(0), section(2)], layout);
        let kinds: Vec<Row> = rows.iter().map(|r| r.1).collect();
        assert_eq!(kinds, [Row::Heading(0), Row::Cards(0, 3), Row::Cards(3, 1), Row::Heading(2), Row::Cards(4, 2)]);
        assert_eq!(rows[1].0, HEADING);
        assert_eq!(rows[3].0, HEADING + 2.0 * 90.0 + 10.0);
        assert_eq!(height, rows[4].0 + 80.0);
        // One section alone: no heading, the cards from the top.
        let (rows, height) = section_rows(&[section(0), section(5)], layout);
        assert_eq!(rows.iter().map(|r| r.1).collect::<Vec<_>>(), [Row::Cards(0, 3), Row::Cards(3, 2)]);
        assert_eq!((rows[0].0, height), (0.0, 170.0));
        assert!(section_rows(&[section(0)], layout).0.is_empty());
    }
}

/// A card of a grid or a list: a picture (painted by the caller) and its words, lamps for what
/// it is to the player, a star for a favourite; chosen, a brass frame and lead. A `ribbon`
/// (translated where it is drawn) marks a card that stands out: a brass band over its picture's
/// corner, and the card's frame in brass.
pub struct Card<'a> {
    pub title: &'a str,
    pub detail: &'a str,
    pub lamps: &'a [Color32],
    pub star: bool,
    pub selected: bool,
    pub spoken: &'a str,
    pub ribbon: Option<&'a str>,
}

/// A small brass band lettered `text` (translated), its left end at `at` (centred on it).
pub fn tag(ui: &Ui, at: egui::Pos2, text: &str) {
    let painter = ui.painter();
    let ink = Color32::from_rgb(34, 26, 10);
    let words = widgets::galley(ui, RichText::new(ta_ui::i18n::t(text)).font(strong(9.5)).color(ink).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
    let band = Rect::from_min_size(pos2(at.x, at.y - (words.size().y + 4.0) * 0.5), words.size() + vec2(10.0, 4.0));
    painter.add(ta_ui::metal::vgrad(band, AMBER_BRIGHT, AMBER_DIM));
    painter.rect_stroke(band, 2.0, Stroke::new(1.0, Color32::from_rgb(70, 52, 20)), StrokeKind::Inside);
    painter.galley(band.min + vec2(5.0, 2.0), words, ink);
}

/// A brass band lettered `text` (translated) over a picture's top-left corner, `size` its
/// lettering's.
pub fn ribbon(ui: &Ui, frame: Rect, text: &str, size: f32) {
    let painter = ui.painter();
    let ink = Color32::from_rgb(34, 26, 10);
    let words = widgets::galley(ui, RichText::new(ta_ui::i18n::t(text)).font(strong(size)).color(ink).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
    let band = Rect::from_min_size(frame.min + vec2(5.0, 5.0), words.size() + vec2(14.0, 6.0));
    painter.rect_filled(band.translate(vec2(1.0, 2.0)), 2.0, Color32::from_black_alpha(130));
    painter.add(ta_ui::metal::vgrad(band, AMBER_BRIGHT, AMBER_DIM));
    painter.rect_stroke(band, 2.0, Stroke::new(1.0, Color32::from_rgb(70, 52, 20)), StrokeKind::Inside);
    painter.galley(band.min + vec2(7.0, 3.0), words, ink);
}

/// Draws `card` in `rect` as the widget `id`, sensing clicks: its picture (`picture` paints it
/// in the frame it is given) `well` tall over its words. Under the pointer (or with the keys) it
/// rises a little off the backdrop on a longer shadow, its plate and rim warm, and a gleam
/// crosses its picture once; none of it moves under reduced motion.
pub fn card(ui: &mut Ui, id: Id, rect: Rect, well: f32, card: Card<'_>, picture: impl FnOnce(&Ui, Rect)) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), card.selected, card.spoken));
    widgets::reveal_focus(ui, &response);
    if response.clicked() && !card.selected { ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle); }
    if !ui.is_rect_visible(rect) { return response; }
    let ctx = ui.ctx().clone();
    let pointer = response.hovered() || response.has_focus();
    let reduced = ta_ui::motion::reduced(&ctx);
    let lift = if reduced { if pointer { 1.0 } else { 0.0 } } else { ctx.animate_bool_with_time(id.with("lift"), pointer, 0.14) };
    let rect = rect.translate(vec2(0.0, if reduced { 0.0 } else { -2.0 * lift }));
    let hot = pointer;
    let painter = ui.painter();
    if lift > 0.0 {
        painter.add(egui::Shadow { offset: [0, (1.0 + 5.0 * lift) as i8], blur: (4.0 + 12.0 * lift) as u8, spread: 0, color: Color32::from_black_alpha((150.0 * lift) as u8) }.as_shape(rect, 3));
    }
    painter.add(ta_ui::metal::plate(&ctx, rect, if card.selected { PANEL_HI } else { PANEL.lerp_to_gamma(PANEL_HI, 0.55 * lift) }));
    let frame = Rect::from_min_size(rect.min + vec2(6.0, 6.0), vec2(rect.width() - 12.0, well));
    picture(ui, frame);
    if !reduced && !ta_ui::motion::still(&ctx) {
        gleam(ui, id, frame.shrink(2.0), pointer);
    }
    if let Some(text) = card.ribbon {
        ribbon(ui, frame, text, 10.5);
    }
    let colour = if card.selected { AMBER_BRIGHT } else if hot { TEXT_BRIGHT } else { TEXT };
    let marks = card.lamps.len() as f32 * 14.0 + if card.star { 16.0 } else { 0.0 };
    let line = |text: &str, font: egui::FontId, colour: Color32, room: f32| egui::WidgetText::from(RichText::new(text).font(font).color(colour))
        .into_galley(ui, Some(egui::TextWrapMode::Truncate), room.max(1.0), egui::TextStyle::Body);
    let title = line(card.title, strong(13.5), colour, rect.width() - 20.0 - marks);
    let detail = line(card.detail, body(11.5), STEEL, rect.width() - 20.0);
    let top = frame.bottom() + 6.0;
    let middle = top + title.size().y * 0.5;
    let below = top + title.size().y + 1.0;
    painter.galley(pos2(rect.left() + 10.0, top), title, colour);
    painter.galley(pos2(rect.left() + 10.0, below), detail, STEEL);
    let mut x = rect.right() - 13.0;
    if card.star {
        star(painter, pos2(x, middle), 5.5, AMBER_BRIGHT);
        x -= 16.0;
    }
    for &lamp in card.lamps {
        painter.add(ta_ui::metal::lamp(pos2(x, middle), 3.4, lamp, 1.0));
        x -= 14.0;
    }
    if card.selected {
        painter.rect_stroke(rect, 3.0, Stroke::new(1.5, AMBER), StrokeKind::Inside);
        painter.rect_filled(Rect::from_min_size(pos2(rect.left(), rect.top() + 8.0), vec2(3.0, rect.height() - 16.0)), 1.0, AMBER_BRIGHT);
    } else {
        // (A ribbon's card keeps a faint brass rim; the pointer warms any card's.)
        let rest = if card.ribbon.is_some() { 0.75 } else { 0.0 };
        let rim = rest + (1.0 - rest) * lift;
        if rim > 0.01 {
            painter.rect_stroke(rect, 3.0, Stroke::new(1.0, AMBER_DIM.gamma_multiply(rim)), StrokeKind::Inside);
        }
    }
    widgets::focus_ring(ui, &response, rect.shrink(3.0));
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A band of light crossing `frame` once, leaning, in the moments after the pointer comes to the
/// widget `id` (`pointer`: it is there now).
pub fn gleam(ui: &Ui, id: Id, frame: Rect, pointer: bool) {
    const CROSSING: f64 = 0.7;
    let key = id.with("gleam");
    let now = ui.input(|i| i.time);
    let since = ui.ctx().data_mut(|d| if pointer { Some(*d.get_temp_mut_or_insert_with(key, || now)) } else { d.remove::<f64>(key); None });
    let Some(since) = since else { return };
    let k = ((now - since) / CROSSING) as f32;
    if !(0.0..1.0).contains(&k) || !frame.is_positive() { return; }
    ui.ctx().request_repaint();
    let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(frame));
    // Across with a quick start and a slow end; brightest midway.
    let eased = 1.0 - (1.0 - k).powi(3);
    let (width, lean) = ((frame.width() * 0.32).max(24.0), frame.height() * 0.45);
    let x = frame.min.x - width - lean + (frame.width() + 2.0 * width + lean) * eased;
    let peak = (72.0 * (1.0 - (2.0 * k - 1.0).abs())) as u8;
    let mut mesh = egui::Mesh::default();
    for (dx, alpha) in [(0.0, 0), (width * 0.5, peak), (width, 0)] {
        let colour = Color32::from_white_alpha(alpha);
        mesh.colored_vertex(pos2(x + dx + lean, frame.min.y), colour);
        mesh.colored_vertex(pos2(x + dx, frame.max.y), colour);
    }
    for i in 0..2u32 {
        let a = i * 2;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// A picture in a recessed well filling `frame`, cropped to fill it (a map's art runs to its
/// edges); while it is made, a rail at work, and a quiet cross where it could not be.
pub fn picture(ui: &Ui, frame: Rect, texture: Option<&TextureHandle>, failed: bool) {
    let painter = ui.painter();
    painter.add(ta_ui::metal::well(frame, Color32::from_rgb(9, 10, 8)));
    let inner = frame.shrink(2.0);
    match texture {
        Some(texture) => {
            let size = texture.size_vec2();
            let uv = fill_uv(size.x / size.y.max(1.0), inner.width() / inner.height().max(1.0));
            painter.image(texture.id(), inner, uv, Color32::WHITE);
            // (The picture sits under the well's rim.)
            painter.rect_stroke(inner, 1.0, Stroke::new(1.0, Color32::from_black_alpha(110)), StrokeKind::Inside);
        }
        None if failed => {
            let stroke = Stroke::new(1.4, STEEL_DIM);
            painter.line_segment([inner.center() + vec2(-6.0, -6.0), inner.center() + vec2(6.0, 6.0)], stroke);
            painter.line_segment([inner.center() + vec2(6.0, -6.0), inner.center() + vec2(-6.0, 6.0)], stroke);
        }
        None => {
            let rail = Rect::from_center_size(inner.center(), vec2((inner.width() * 0.45).clamp(24.0, 96.0), 6.0));
            ta_ui::progress::paint(ui, rail, None, true);
        }
    }
}

/// The part of a picture `picture` wide (over its height) that fills a frame `frame` wide,
/// centred.
pub fn fill_uv(picture: f32, frame: f32) -> Rect {
    if !(picture.is_finite() && frame.is_finite() && picture > 0.0 && frame > 0.0) { return Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)); }
    if picture > frame {
        let keep = frame / picture;
        Rect::from_min_max(pos2((1.0 - keep) * 0.5, 0.0), pos2((1.0 + keep) * 0.5, 1.0))
    } else {
        let keep = picture / frame;
        Rect::from_min_max(pos2(0.0, (1.0 - keep) * 0.5), pos2(1.0, (1.0 + keep) * 0.5))
    }
}

/// A list row with a small picture at its left (`picture` paints it), `aspect` times as wide as
/// tall: `title` over `detail`, a lamp and a star at its right; chosen, the brass lead every list
/// marks its choice with.
pub fn picture_row(ui: &mut Ui, id: Id, rect: Rect, aspect: f32, card: Card<'_>, picture: impl FnOnce(&Ui, Rect)) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), card.selected, card.spoken));
    widgets::reveal_focus(ui, &response);
    if response.clicked() && !card.selected { ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle); }
    if !ui.is_rect_visible(rect) { return response; }
    let hot = response.hovered();
    let painter = ui.painter();
    ta_ui::panel::row_state(painter, rect, card.selected, hot);
    painter.add(ta_ui::metal::seam(rect.x_range().shrink(4.0), rect.bottom() - 1.0));
    let side = rect.height() - 10.0;
    let frame = Rect::from_min_size(pos2(rect.left() + 8.0, rect.top() + 5.0), vec2(side * aspect, side));
    picture(ui, frame);
    let left = frame.right() + 12.0;
    let marks = card.lamps.len() as f32 * 14.0 + if card.star { 16.0 } else { 0.0 };
    let room = (rect.right() - left - 12.0 - marks).max(1.0);
    let colour = if card.selected { AMBER_BRIGHT } else if hot { TEXT_BRIGHT } else { TEXT };
    let line = |text: &str, font: egui::FontId, colour: Color32| egui::WidgetText::from(RichText::new(text).font(font).color(colour))
        .into_galley(ui, Some(egui::TextWrapMode::Truncate), room, egui::TextStyle::Body);
    let name = line(card.title, body(14.5), colour);
    let under = line(card.detail, body(11.5), STEEL);
    let top = rect.center().y - (name.size().y + under.size().y + 1.0) * 0.5;
    let below = top + name.size().y + 1.0;
    // (A row's ribbon follows its name, clear of the small picture.)
    let after = pos2(left + name.size().x + 10.0, top + name.size().y * 0.5);
    painter.galley(pos2(left, top), name, colour);
    painter.galley(pos2(left, below), under, STEEL);
    if let Some(text) = card.ribbon
        && after.x + 60.0 < rect.right() - 12.0 - marks
    {
        tag(ui, after, text);
    }
    let mut x = rect.right() - 14.0;
    if card.star { star(painter, pos2(x, rect.center().y), 5.5, AMBER_BRIGHT); x -= 16.0; }
    for &lamp in card.lamps {
        painter.add(ta_ui::metal::lamp(pos2(x, rect.center().y), 3.4, lamp, 1.0));
        x -= 14.0;
    }
    widgets::focus_ring(ui, &response, rect.shrink(4.0));
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A five-pointed star (a favourite's mark), with a dark rim so that it reads on a picture.
pub fn star(painter: &egui::Painter, centre: egui::Pos2, radius: f32, colour: Color32) {
    let points: Vec<_> = (0..10).map(|i| {
        let angle = i as f32 * std::f32::consts::PI / 5.0 - std::f32::consts::FRAC_PI_2;
        centre + vec2(angle.cos(), angle.sin()) * if i % 2 == 0 { radius } else { radius * 0.45 }
    }).collect();
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, colour);
    for point in &points { mesh.colored_vertex(*point, colour); }
    for i in 0..10u32 { mesh.add_triangle(0, i + 1, (i + 1) % 10 + 1); }
    let mut rim = points.clone();
    rim.push(points[0]);
    painter.add(egui::Shape::line(rim, Stroke::new(1.5, Color32::from_black_alpha(150))));
    painter.add(egui::Shape::mesh(mesh));
}

/// A button showing a star, lit when `on`; clicking flips it. `label` (translated) names it.
pub fn star_button(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(widgets::CONTROL_HEIGHT + 2.0, widgets::CONTROL_HEIGHT), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, label));
    widgets::reveal_focus(ui, &response);
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
    }
    if ui.is_rect_visible(rect) {
        let hot = response.hovered();
        let painter = ui.painter();
        painter.add(ta_ui::metal::slab(ui.ctx(), rect, ta_ui::metal::Slab::Slot, if hot { 1.0 } else { 0.0 }, *on));
        star(painter, rect.center(), 7.0, if *on { AMBER_BRIGHT } else if hot { TEXT } else { STEEL_DIM });
        widgets::focus_ring(ui, &response, rect);
    }
    response.on_hover_cursor(CursorIcon::PointingHand)
}

/// A drop-down of a toolbar `width` wide showing `label` (translated, as a caption) and the
/// choice; `choices` as (value, translated name).
pub fn choice<T: PartialEq + Copy>(ui: &mut Ui, id: &str, width: f32, label: &str, value: &mut T, choices: &[(T, String)]) -> Response {
    let shown = choices.iter().find(|(v, _)| v == value).map_or("", |(_, name)| name.as_str());
    let mut job = egui::text::LayoutJob::default();
    job.append(&format!("{label}  "), 0.0, egui::TextFormat::simple(strong(10.5), STEEL));
    job.append(shown, 0.0, egui::TextFormat::simple(body(13.0), TEXT));
    let mut changed = false;
    let mut response = egui::ComboBox::from_id_salt(id).width(width).wrap_mode(egui::TextWrapMode::Truncate)
        .selected_text(job)
        .show_ui(ui, |ui| {
            for (option, name) in choices {
                if ui.selectable_label(value == option, name).clicked() && value != option {
                    *value = *option;
                    changed = true;
                }
            }
        }).response;
    if changed {
        response.mark_changed();
        ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::Toggle);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, format!("{label} {shown}")));
    response
}

/// A fact of a details pane, `width` wide: a small caption (translated) over its value.
pub fn fact(ui: &mut Ui, width: f32, caption: &str, value: &str) -> Response {
    let value_galley = egui::WidgetText::from(RichText::new(value).font(body(14.5)).color(TEXT_BRIGHT))
        .into_galley(ui, Some(egui::TextWrapMode::Truncate), width, egui::TextStyle::Body);
    let caption_galley = widgets::galley(ui, widgets::caption(caption));
    let caption_height = caption_galley.size().y;
    let (rect, response) = ui.allocate_exact_size(vec2(width, caption_height + value_galley.size().y + 2.0), Sense::hover());
    let painter = ui.painter();
    painter.galley(rect.min, caption_galley, STEEL);
    painter.galley(rect.min + vec2(0.0, caption_height + 2.0), value_galley, TEXT_BRIGHT);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("{}: {value}", ta_ui::i18n::t(caption))));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picture_files_are_bounded_before_decoding_and_valid_cache_images_roundtrip() {
        let root = std::env::temp_dir().join(format!("ta-picture-limits-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("picture.jpg");
        let picture = egui::ColorImage::filled([16, 8], Color32::RED);
        write_cached(&path, &picture);
        assert_eq!(read_cached(&path).unwrap().size, [16, 8]);
        let length = std::fs::metadata(&path).unwrap().len();
        assert!(read_picture(&path, length - 1).is_none(), "encoded bytes are bounded");
        // A tiny compressed file with an out-of-range dimension is refused by
        // the decoder's limits, before allocating its pixel buffer.
        image::RgbImage::new(4097, 1).save(&path).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() < 1 << 20);
        assert!(read_cached(&path).is_none());
        assert!(read_picture(&path, 2 << 20).is_none(), "benchmark pictures share the same limits");
        std::fs::write(&path, b"damaged image").unwrap();
        assert!(read_cached(&path).is_none());
        std::fs::File::create(&path).unwrap().set_len((16 << 20) + 1).unwrap();
        assert!(read_cached(&path).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cards_share_the_width_and_never_go_below_their_least() {
        let grid = layout(916.0, 180.0, 0.75, 46.0, 12.0);
        assert_eq!(grid.columns, 4);
        assert!(grid.card.x >= 180.0 && grid.card.x * 4.0 + 36.0 <= 916.0);
        assert_eq!(layout(120.0, 180.0, 0.75, 46.0, 12.0).columns, 1, "a narrow page still has a column");
        let uv = fill_uv(2.0, 1.0);
        assert!((uv.width() - 0.5).abs() < 1e-6 && (uv.height() - 1.0).abs() < 1e-6, "a wide picture is cropped at its sides");
    }

    #[test]
    fn pictures_to_show_come_before_looking_ahead_and_textures_are_capped() {
        let ctx = egui::Context::default();
        let mut pictures = Pictures::new("test", 1, 2);
        let (hold, wait) = mpsc::channel::<()>();
        let order = Arc::new(Mutex::new(Vec::new()));
        let make = |name: &'static str, order: Arc<Mutex<Vec<&'static str>>>| move || {
            order.lock().unwrap().push(name);
            Ok(Picture { image: egui::ColorImage::filled([2, 2], Color32::RED), size: Some((64, 32)) })
        };
        // (The worker is held on the first job while the rest queue up behind it.)
        pictures.request("first".into(), move || { wait.recv().ok(); Ok(Picture { image: egui::ColorImage::filled([2, 2], Color32::RED), size: None }) });
        std::thread::sleep(std::time::Duration::from_millis(50));
        for name in ["ahead-a", "ahead-b"] { pictures.prefetch(name.into(), make(name, order.clone())); }
        for name in ["shown-a", "shown-b", "shown-c"] { pictures.request(name.into(), make(name, order.clone())); }
        hold.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !pictures.idle() && std::time::Instant::now() < deadline {
            pictures.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        pictures.poll(&ctx);
        assert_eq!(*order.lock().unwrap(), ["shown-c", "shown-b", "shown-a", "ahead-a", "ahead-b"], "the newest asked for to show is made first");
        assert_eq!(pictures.size("ahead-a"), Some((64, 32)), "looking ahead keeps the size");
        assert!(!pictures.has("ahead-a"), "and makes no texture");
        assert!(pictures.textures.len() <= 2, "textures beyond the cap are let go");
        assert!(pictures.size("shown-a").is_some(), "a size outlives its texture");
    }
}
