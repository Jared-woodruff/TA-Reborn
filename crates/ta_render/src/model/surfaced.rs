//! Models with surfaces of their own (`ta_formats::surface`, beside their 3DOs: Supreme
//! Commander's units as TA Reborn makes them, and any mod's): their vertices' texture coordinates
//! and smooth normals as the surface gives them, their polygons painted with its materials, and
//! nothing of Reborn's kit, caps or rounding (they are whole already).
//!
//! A material's paint is three textures (its colour; its specular and team colour; its normals),
//! each a layer of one of two arrays of BC3 textures with every mip level: one of [`SIDE`] by
//! [`SIDE`] texels, and one of [`FINE_SIDE`] for the textures painted that finely (most of Supreme
//! Commander's buildings', its commanders', its larger ships' and experimentals') while the paint
//! is fine ([`SurfaceTextures::set_fine`]: with HD surfaces; otherwise those take their 512 level,
//! as every texture did before, a quarter of the memory). The paint is asked for the first time a
//! model of its material is drawn: a battle holds the paint of the units in it, not of every unit
//! there is, and lets it go when the next battle starts ([`SurfaceTextures::release`]). Where to
//! find each of a material's textures is a table the shaders read (`surface_slots`, three words a
//! material); until all three are in, its faces are drawn in its colour on average. The files are
//! read and made BC3 levels on workers of their own (a texture of another size, or uncompressed,
//! is decoded and encoded again, which takes tens of milliseconds); the frame puts what they made
//! in the arrays for [`FRAME_SHARE`] of its time at most. Each array grows about 32 MB at a time;
//! when the card has no memory for more fine layers, the fine textures take their 512 level, and
//! when it has none for that either, the materials not in the arrays stay in their colours on
//! average. Without block-compressed textures on the graphics card, the layers are decoded and
//! held as plain colour instead (never fine: four times the memory).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use glam::Vec3;
use ta_formats::dds;
use ta_formats::model3do::{FIXED_ONE, Model3do};
use ta_formats::surface::{Material, Surface};

use super::{FAR_FROM_EDGES, ModelId, ModelInfo, ModelLibrary, ModelVertex, PALETTE_FACE, PieceInfo, TextureRef, f16_bits, face_normal, polygon_uvs, unflagged_ground_plate};
use crate::occlusion::Footprint;

/// The side of the plain array's layers, and of the fine one's.
pub const SIDE: u32 = 512;
pub const FINE_SIDE: u32 = 1024;
/// A model with a lighter one for far off (its surface's `far`) is drawn as itself from this
/// many pixels across on screen, and as its lighter one under [`FAR_PIXELS_BACK`]: between the
/// two it stays as it was drawn the frame before (first seen there, the lighter one), so a model
/// small on the screen does not swap back and forth as the camera shakes or a walker bobs.
pub const FAR_PIXELS: f32 = 48.0;
pub const FAR_PIXELS_BACK: f32 = 42.0;
/// [`ModelLibrary::far_role`]s: a model with a lighter one for far off, and such a lighter one.
pub(super) const FAR_HAS: u8 = 1;
pub(super) const FAR_IS: u8 = 2;
/// A material's textures: colour, specular and team colour, normals.
pub const LAYERS: u32 = 3;
/// A table word for a texture not read; and the bit of one in the fine array (its layer below).
const NOT_READ: u32 = u32::MAX;
pub const FINE: u32 = 1 << 31;
/// The most of a frame's time spent putting the paint the workers made in the arrays (at least one
/// material a frame; the rest the next).
pub const FRAME_SHARE: Duration = Duration::from_millis(2);
/// How much an array grows by at a time (in bytes of BC3: 96 layers of 512, 24 of 1024).
const GROWTH_BYTES: u64 = 32 << 20;
/// Materials made and waiting for the frames, on the way from the workers and taken from them
/// (twice this all told) at most: the workers wait for room (a material's levels are one to four
/// megabytes, four times that as plain colour).
const WAITING: usize = 16;

/// Reads a file of the game's (a texture a surface names), or none.
pub type Loader = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// Whether each frame waits for the paint it asks for ([`set_paint_waits`]).
static WAITS: AtomicBool = AtomicBool::new(false);
/// The longest a frame waits for a material's paint ([`set_paint_waits`]).
const WAIT_AT_MOST: Duration = Duration::from_secs(10);

/// With `waits`, each frame waits for the paint its models ask for, and puts all of it in the
/// array whatever it takes: a capture (`--screenshot`, a film) shows a unit in its paint from the
/// first frame it is in, whatever the workers' pace. Otherwise the paint comes as it is made.
pub fn set_paint_waits(waits: bool) {
    WAITS.store(waits, Ordering::Relaxed);
}

struct Entry {
    material: Material,
    /// Where its three textures are (the table's words: a layer, [`FINE`] for the fine array's),
    /// once read.
    words: Option<[u32; 3]>,
    /// Asked of the workers (its paint on the way).
    asked: bool,
    /// Reading it failed, or the arrays had no room for it: drawn in its colour on average from
    /// then on (until the paint is let go).
    failed: bool,
}

/// A material's paint to read and make: its three textures' paths, and whether finely.
struct Job {
    slot: u32,
    generation: u32,
    paths: [String; 3],
    loader: Loader,
    fine: bool,
}

/// One of a material's textures as the workers made it: its side, and its levels as the arrays
/// hold them (BC3 blocks, or plain colour).
struct Layer {
    side: u32,
    levels: Vec<Vec<u8>>,
}

/// A material's paint as the workers made it, or why it could not be made.
struct Made {
    slot: u32,
    generation: u32,
    layers: Result<Vec<Layer>, String>,
}

/// The workers reading and making paint, and the way to them and back.
struct Workers {
    jobs: mpsc::Sender<Job>,
    made: mpsc::Receiver<Made>,
}

/// One of the two arrays the paint is held in, and how full it is.
struct PaintArray {
    side: u32,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Layers it has room for, and holds.
    capacity: u32,
    held: u32,
    /// The card refused the memory to grow it, or it holds all the layers the card allows: no
    /// more go in until the paint is let go.
    full: bool,
}

impl PaintArray {
    fn empty(device: &wgpu::Device, compressed: bool, side: u32) -> PaintArray {
        let (texture, view) = SurfaceTextures::array(device, compressed, 1, 4, 1);
        PaintArray { side, texture, view, capacity: 0, held: 0, full: false }
    }

    fn levels(&self) -> u32 {
        self.side.ilog2() + 1
    }

    /// Room for `more` layers: made (the array grown, what it held copied over) where there is
    /// none, as far as the card allows (`most` layers); whether there is, and whether the array was
    /// replaced (its bind group to be made again).
    fn room(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, compressed: bool, most: u32, more: u32) -> (bool, bool) {
        if self.held + more <= self.capacity {
            return (true, false);
        }
        if self.full {
            return (false, false);
        }
        if self.held + more > most {
            log::warn!("surface paint: no room for more than {most} layers of {}", self.side);
            self.full = true;
            return (false, false);
        }
        let step = (GROWTH_BYTES / layer_bytes(self.side, compressed)).max(1) as u32;
        if self.grow(device, queue, compressed, (self.capacity + step).max(self.held + more).min(most)) {
            (true, true)
        } else {
            self.full = true;
            (false, false)
        }
    }

    /// The array made `capacity` layers large, what it held copied over; false when the card
    /// refused the memory (the array as it was).
    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, compressed: bool, capacity: u32) -> bool {
        let scopes = [wgpu::ErrorFilter::OutOfMemory, wgpu::ErrorFilter::Validation, wgpu::ErrorFilter::Internal].map(|f| device.push_error_scope(f));
        let (texture, view) = SurfaceTextures::array(device, compressed, capacity, self.side, self.levels());
        // (Popped innermost first, every one; the cause told, the outermost: a texture refused
        // for want of memory is then invalid to view as well.)
        let errors: Vec<wgpu::Error> = scopes.into_iter().rev().filter_map(|s| pollster::block_on(s.pop())).collect();
        if let Some(e) = errors.last() {
            log::warn!("surface paint: no graphics memory for room for {capacity} layers of {} ({e})", self.side);
            return false;
        }
        if self.held > 0 {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("surface paint grown") });
            for level in 0..self.levels() {
                let side = (self.side >> level).max(1);
                // (Compressed, a level is copied in whole blocks.)
                let side = if compressed { side.max(4) } else { side };
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    wgpu::Extent3d { width: side, height: side, depth_or_array_layers: self.held },
                );
            }
            queue.submit([encoder.finish()]);
        }
        self.texture = texture;
        self.view = view;
        self.capacity = capacity;
        log::info!("surface paint: room for {capacity} layers of {} ({} MB)", self.side, capacity as u64 * layer_bytes(self.side, compressed) / (1 << 20));
        true
    }

    /// One layer's levels (BC3 blocks, or plain colour; as many as the array has), written as the
    /// array holds them, in its next layer: which.
    fn put(&mut self, queue: &wgpu::Queue, compressed: bool, levels: &[Vec<u8>]) -> u32 {
        let layer = self.held;
        self.held += 1;
        for (level, data) in levels.iter().take(self.levels() as usize).enumerate() {
            let side = (self.side >> level).max(1);
            let blocks = side.div_ceil(4);
            let target = wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: level as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect: wgpu::TextureAspect::All };
            if compressed {
                let extent = wgpu::Extent3d { width: blocks * 4, height: blocks * 4, depth_or_array_layers: 1 };
                queue.write_texture(target, data, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(blocks * 16), rows_per_image: Some(blocks) }, extent);
            } else {
                let extent = wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 };
                queue.write_texture(target, data, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(side * 4), rows_per_image: Some(side) }, extent);
            }
        }
        layer
    }
}

/// The materials of the models with surfaces, and their paint as far as it is read.
pub struct SurfaceTextures {
    entries: Vec<Entry>,
    by_textures: HashMap<(String, String, String), u32>,
    loader: Option<Loader>,
    workers: Option<Workers>,
    /// What the workers made, waiting for room in a frame.
    ready: VecDeque<Made>,
    /// The paint's generation: what was asked before it was last let go is not wanted.
    generation: Arc<AtomicU32>,
    compressed: bool,
    /// The textures painted at 1024 or more texels a side are made at 1024 ([`FINE_SIDE`]) and
    /// held in the fine array (else at 512, in the plain one).
    fine: bool,
    plain: PaintArray,
    fine_array: PaintArray,
    /// The most layers an array may hold (the card's limit).
    most: u32,
    pub(super) table: wgpu::Buffer,
    table_slots: u32,
    /// The bind group must be made again (an array or the table replaced since the last frame).
    rebind: bool,
}

impl SurfaceTextures {
    pub fn new(device: &wgpu::Device) -> SurfaceTextures {
        let compressed = device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let table = Self::make_table(device, 16);
        SurfaceTextures {
            entries: Vec::new(),
            by_textures: HashMap::new(),
            loader: None,
            workers: None,
            ready: VecDeque::new(),
            generation: Arc::new(AtomicU32::new(0)),
            compressed,
            fine: compressed,
            plain: PaintArray::empty(device, compressed, SIDE),
            fine_array: PaintArray::empty(device, compressed, FINE_SIDE),
            most: device.limits().max_texture_array_layers,
            table,
            table_slots: 16,
            rebind: false,
        }
    }

    /// The plain array's view, and the fine one's (the shaders' `surface_paint` and
    /// `surface_paint_fine`).
    pub(super) fn views(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
        (&self.plain.view, &self.fine_array.view)
    }

    /// What reads the textures the materials name (the game's files).
    pub fn set_loader(&mut self, loader: Loader) {
        self.loader = Some(loader);
    }

    /// Whether the paint is fine (the textures painted at 1024 texels a side or more made at 1024:
    /// with HD surfaces), or every texture at 512, as small graphics cards want it (a quarter of
    /// the memory for those). A change lets the paint held go, to be made again as it is drawn.
    /// Never fine without block-compressed textures (four times the memory again).
    pub fn set_fine(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, fine: bool) {
        let fine = fine && self.compressed;
        if fine != self.fine {
            self.fine = fine;
            if self.entries.iter().any(|e| e.asked || e.words.is_some()) {
                self.release(device, queue);
            }
        }
    }

    /// The slot of `material` (one for every material of the same three textures): what the
    /// vertices of its faces name.
    pub fn register(&mut self, material: &Material) -> u32 {
        let key = (material.albedo.to_ascii_lowercase(), material.specular.to_ascii_lowercase(), material.normals.to_ascii_lowercase());
        if let Some(&slot) = self.by_textures.get(&key) {
            return slot;
        }
        let slot = self.entries.len() as u32;
        self.entries.push(Entry { material: material.clone(), words: None, asked: false, failed: false });
        self.by_textures.insert(key, slot);
        slot
    }

    /// Asks for `slot`'s paint (a model of its drawn this frame): the workers read and make it,
    /// once.
    pub(super) fn want(&mut self, slot: u32) {
        let Some(e) = self.entries.get_mut(slot as usize) else { return };
        if e.asked || e.failed || e.words.is_some() {
            return;
        }
        let Some(loader) = self.loader.clone() else { return };
        if self.plain.full {
            e.failed = true;
            return;
        }
        e.asked = true;
        let paths = [e.material.albedo.clone(), e.material.specular.clone(), e.material.normals.clone()];
        let job = Job { slot, generation: self.generation.load(Ordering::Relaxed), paths, loader, fine: self.fine };
        let compressed = self.compressed;
        let generation = &self.generation;
        let workers = self.workers.get_or_insert_with(|| Self::start_workers(compressed, generation.clone()));
        let _ = workers.jobs.send(job);
    }

    /// The workers: a few threads of their own (a texture to decode and encode again keeps one
    /// busy for tens of milliseconds), reading and making what they are asked in turn.
    fn start_workers(compressed: bool, generation: Arc<AtomicU32>) -> Workers {
        let (jobs, asked) = mpsc::channel::<Job>();
        let (done, made) = mpsc::sync_channel::<Made>(WAITING);
        let asked = Arc::new(Mutex::new(asked));
        let count = std::thread::available_parallelism().map_or(1, |n| (n.get() / 4).clamp(1, 3));
        for k in 0..count {
            let (asked, done, generation) = (asked.clone(), done.clone(), generation.clone());
            let spawned = std::thread::Builder::new().name(format!("surface paint {k}")).spawn(move || {
                loop {
                    let job = match asked.lock() {
                        Ok(rx) => rx.recv(),
                        Err(_) => return,
                    };
                    let Ok(job) = job else { return };
                    // (Paint let go since it was asked: not wanted.)
                    if job.generation != generation.load(Ordering::Relaxed) {
                        continue;
                    }
                    let layers = make_paint(&job.loader, &job.paths, compressed, job.fine);
                    if done.send(Made { slot: job.slot, generation: job.generation, layers }).is_err() {
                        return;
                    }
                }
            });
            if let Err(e) = spawned {
                log::warn!("surface paint: no worker ({e})");
            }
        }
        Workers { jobs, made }
    }

    /// Puts the paint the workers made in the arrays, for [`FRAME_SHARE`] of the frame's time at
    /// most, making room for it; whether the bind group must be made again (an array or the table
    /// replaced).
    pub(super) fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        self.update_waiting(device, queue, WAITS.load(Ordering::Relaxed))
    }

    /// [`SurfaceTextures::update`], with `waits`: waiting for every material asked for and putting
    /// all of it in the arrays whatever it takes.
    fn update_waiting(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, waits: bool) -> bool {
        let mut rebind = std::mem::take(&mut self.rebind);
        if self.entries.len() as u32 * LAYERS > self.table_slots {
            let slots = (self.entries.len() as u32 * LAYERS).next_power_of_two();
            self.table = Self::make_table(device, slots);
            self.table_slots = slots;
            let table: Vec<u32> = (0..slots).map(|s| self.entries.get((s / LAYERS) as usize).and_then(|e| e.words).map_or(NOT_READ, |w| w[(s % LAYERS) as usize])).collect();
            queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&table));
            rebind = true;
        }
        let generation = self.generation.load(Ordering::Relaxed);
        if let Some(workers) = &self.workers {
            // (What the workers made, as much as there is room for here: a few materials, the
            // workers waiting meanwhile; all of it when the frame waits for it.)
            while waits || self.ready.len() < WAITING {
                match workers.made.try_recv() {
                    Ok(m) => self.ready.push_back(m),
                    Err(_) => break,
                }
            }
            // (Waiting: for every material asked for and not made yet.)
            if waits {
                loop {
                    let made = self.ready.iter().filter(|m| m.generation == generation).count();
                    let on_the_way = self.entries.iter().filter(|e| e.asked && e.words.is_none() && !e.failed).count();
                    if made >= on_the_way {
                        break;
                    }
                    match workers.made.recv_timeout(WAIT_AT_MOST) {
                        Ok(m) => self.ready.push_back(m),
                        Err(_) => break,
                    }
                }
            }
        }
        let started = Instant::now();
        while let Some(made) = self.ready.pop_front() {
            if made.generation != generation {
                continue;
            }
            let slot = made.slot as usize;
            let layers = match made.layers {
                Ok(layers) => layers,
                Err(e) => {
                    log::warn!("{e}");
                    self.entries[slot].failed = true;
                    continue;
                }
            };
            let (words, grown) = self.place(device, queue, &layers);
            rebind |= grown;
            match words {
                Some(words) => {
                    self.entries[slot].words = Some(words);
                    queue.write_buffer(&self.table, slot as u64 * LAYERS as u64 * 4, bytemuck::cast_slice(&words));
                }
                None => self.entries[slot].failed = true,
            }
            if !waits && started.elapsed() >= FRAME_SHARE {
                break;
            }
        }
        rebind
    }

    /// A material's textures put in the arrays: the fine ones in the fine array where it has room
    /// for them all, else their 512 levels in the plain one with the rest; their table words, or
    /// none when the plain array has no room for what goes there (nothing of it put in: the
    /// material in its colour on average); and whether an array was replaced.
    fn place(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, layers: &[Layer]) -> (Option<[u32; 3]>, bool) {
        let layers = &layers[..layers.len().min(LAYERS as usize)];
        let mut grown = false;
        let fine_ones = layers.iter().filter(|l| l.side == FINE_SIDE).count() as u32;
        let fine = fine_ones > 0 && {
            let (room, replaced) = self.fine_array.room(device, queue, self.compressed, self.most, fine_ones);
            grown |= replaced;
            room
        };
        let plain_ones = layers.len() as u32 - if fine { fine_ones } else { 0 };
        if plain_ones > 0 {
            let (room, replaced) = self.plain.room(device, queue, self.compressed, self.most, plain_ones);
            grown |= replaced;
            if !room {
                return (None, grown);
            }
        }
        let mut words = [NOT_READ; 3];
        for (word, layer) in words.iter_mut().zip(layers) {
            *word = if fine && layer.side == FINE_SIDE {
                FINE | self.fine_array.put(queue, self.compressed, &layer.levels)
            } else {
                // (A fine texture without room for it: from its 512 level down.)
                let skip = (layer.side / SIDE).max(1).ilog2() as usize;
                self.plain.put(queue, self.compressed, &layer.levels[skip.min(layer.levels.len())..])
            };
        }
        (Some(words), grown)
    }

    /// Lets the paint go (a battle over, the next starting): the arrays back to nothing, every
    /// material drawn in its colour on average until a model of its is drawn again; what the
    /// workers are making now is not wanted.
    pub fn release(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.ready.clear();
        for e in &mut self.entries {
            e.words = None;
            e.asked = false;
            e.failed = false;
        }
        let held = (self.plain.capacity, self.fine_array.capacity);
        self.plain = PaintArray::empty(device, self.compressed, SIDE);
        self.fine_array = PaintArray::empty(device, self.compressed, FINE_SIDE);
        queue.write_buffer(&self.table, 0, bytemuck::cast_slice(&vec![NOT_READ; self.table_slots as usize]));
        self.rebind = true;
        if held != (0, 0) {
            log::info!("surface paint: let go of room for {} layers of {SIDE} and {} of {FINE_SIDE}", held.0, held.1);
        }
    }

    fn array(device: &wgpu::Device, compressed: bool, layers: u32, side: u32, levels: u32) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("surface paint"),
            size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: layers },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if compressed { wgpu::TextureFormat::Bc3RgbaUnorm } else { wgpu::TextureFormat::Rgba8Unorm },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        (texture, view)
    }

    fn make_table(device: &wgpu::Device, slots: u32) -> wgpu::Buffer {
        use crate::gpu::CheckedBufferInit;
        let table = vec![NOT_READ; slots as usize];
        device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some("surface slots"), contents: bytemuck::cast_slice(&table), usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST })
    }

    /// How many materials there are, and how many are read.
    pub fn counts(&self) -> (usize, u32) {
        (self.entries.len(), self.entries.iter().filter(|e| e.words.is_some()).count() as u32)
    }

    /// The graphics memory the arrays take (the room made in them, held or not), in bytes.
    pub fn memory(&self) -> u64 {
        self.plain.capacity as u64 * layer_bytes(SIDE, self.compressed) + self.fine_array.capacity as u64 * layer_bytes(FINE_SIDE, self.compressed)
    }

    /// Whether every material asked for is in the arrays or given up (none on the way).
    pub fn settled(&self) -> bool {
        self.ready.is_empty() && self.entries.iter().all(|e| !e.asked || e.words.is_some() || e.failed)
    }
}

/// A material's paint read and made as the arrays hold it (on a worker): each of its three
/// textures (`paths`, read by `loader`) as BC3 levels (`dds::to_bc3_levels`) of its own side
/// (`dds::paint_side`: 512, or 1024 for one of 1024 texels or more with `fine`), decoded to plain
/// colour where the card reads no BC3 (`compressed` false); or why not, in words for the log.
fn make_paint(loader: &Loader, paths: &[String; 3], compressed: bool, fine: bool) -> Result<Vec<Layer>, String> {
    let mut layers = Vec::with_capacity(3);
    for path in paths {
        let bytes = loader(path).ok_or_else(|| format!("{path}: not in the game's files"))?;
        let header = dds::Header::parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
        let side = dds::paint_side(&header, if fine && compressed { FINE_SIDE } else { SIDE });
        let levels = dds::to_bc3_levels(&bytes, side).map_err(|e| format!("{path}: {e}"))?;
        if levels.len() != (side.ilog2() + 1) as usize {
            return Err(format!("{path}: {} mip levels made of {}", levels.len(), side.ilog2() + 1));
        }
        let levels = if compressed {
            levels
        } else {
            levels.iter().enumerate().map(|(level, data)| {
                let side = (side >> level).max(1);
                dds::decode_bc3(data, side, side).into_iter().flatten().collect()
            }).collect()
        };
        layers.push(Layer { side, levels });
    }
    Ok(layers)
}

/// A layer's bytes of `side` with its mip levels (BC3, or plain colour).
fn layer_bytes(side: u32, compressed: bool) -> u64 {
    (0..=side.ilog2()).map(|l| ((side >> l).max(1) as u64).pow(2) * if compressed { 1 } else { 4 }).sum()
}

impl super::ModelRenderer {
    /// Puts the paint asked for so far in the arrays, waiting for what the workers are still
    /// making of it (at most [`WAIT_AT_MOST`] a material): for a picture drawn once and kept (a
    /// portrait), whose models would otherwise stay in their colours on average (the build menu
    /// showed Forged Alliance's units' portraits as dark silhouettes, their paint not read yet as
    /// they were drawn).
    pub fn wait_for_paint(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        if self.surfaces.update_waiting(device, queue, true) {
            self.bind_group = Self::make_bind_group(device, &self.layout, &self.matrix_buffer, &self.textures, &self.surfaces, &self.sampler);
        }
    }
}

impl ModelLibrary {
    /// A model with a surface of its own, registered under `name`: each piece's vertices as the 3DO
    /// places them with the surface's texture coordinates and normals; a polygon naming one of
    /// its materials painted with it (`slots`: each material's, from
    /// [`SurfaceTextures::register`]), one naming a TA texture (`texture` finds it) painted as TA
    /// lays it on, any other in its palette colour. The selection plate is left out, as from
    /// TA's own models.
    pub fn add_surfaced(&mut self, name: &str, model: &Model3do, surface: &Surface, slots: &[u32], texture: impl Fn(&str) -> Option<TextureRef>, palette: &ta_formats::image::Palette) -> ModelId {
        let key = name.to_ascii_lowercase();
        if let Some(&id) = self.by_name.get(&key) {
            return id;
        }
        let fixed = |v: [i32; 3]| Vec3::new(v[0] as f32, v[1] as f32, -(v[2] as f32)) / FIXED_ONE;
        let base_vertex = self.vertices.len() as i32;
        let first_index = self.indices.len() as u32;
        let mut rest: Vec<Vec3> = Vec::with_capacity(model.pieces.len());
        for p in &model.pieces {
            let at = p.parent.map_or(Vec3::ZERO, |parent| rest[parent]) + fixed(p.offset);
            rest.push(at);
        }
        let (mut max_r2, mut max_h) = (0f32, 0f32);
        let mut polygons: Vec<Vec<Vec3>> = Vec::new();
        let mut ranges = Vec::with_capacity(model.pieces.len());
        for (pi, (piece, faces)) in model.pieces.iter().zip(&surface.pieces).enumerate() {
            let piece_first = self.indices.len() as u32;
            // (A vertex of the library's for each of the 3DO's and what paints it: shared by the
            // polygons round it, as its surface's normal is; a TA texture's laid on each polygon
            // of its own.)
            let mut made: HashMap<(u16, u32, [u8; 4]), u32> = HashMap::new();
            let plate = piece.selection_primitive.or_else(|| unflagged_ground_plate(piece, rest[pi].y));
            for (_, prim) in piece.primitives.iter().enumerate().filter(|&(k, p)| p.indices.len() >= 3 && Some(k) != plate) {
                let at_corners: Vec<Vec3> = prim.indices.iter().map(|&i| fixed(piece.vertices[i as usize])).collect();
                let material = prim.texture.as_deref().and_then(|t| surface.material(t));
                let ta = if material.is_none() { prim.texture.as_deref().and_then(&texture) } else { None };
                let laid = ta.map(|_| polygon_uvs(&at_corners, face_normal(&at_corners)));
                let (layer, color, roughness) = match (material, ta) {
                    (Some(m), _) => {
                        let mean = surface.materials[m].mean;
                        // (A middling roughness: the material's own is in its paint.)
                        (slots[m], [mean[0], mean[1], mean[2], 0], 64)
                    }
                    (None, Some(t)) => (t.layer | (t.frames.min(0xFF) << 16) | ((t.emissive as u32) << 24) | (1 << 28), [255, 255, 255, (t.metallic * 255.0).round() as u8], (t.roughness * 127.0).round() as i8),
                    (None, None) => {
                        let [r, g, b] = palette.rgb((prim.color_index & 0xFF) as u8);
                        (0xFFFF | (1 << 16) | PALETTE_FACE, [r, g, b, 255], 64)
                    }
                };
                let mut corners: Vec<u32> = Vec::with_capacity(prim.indices.len());
                for (c, (&i, &p)) in prim.indices.iter().zip(&at_corners).enumerate() {
                    let mut vertex = || {
                        let v = faces.vertices.get(i as usize).copied().unwrap_or(ta_formats::surface::Vertex { uv: [0.0; 2], normal: [0, 32767, 0] });
                        let n = Vec3::new(v.normal[0] as f32, v.normal[1] as f32, -(v.normal[2] as f32)).normalize_or(Vec3::Y);
                        let uv = laid.as_ref().map_or(v.uv, |uvs| uvs[c]);
                        self.vertices.push(ModelVertex {
                            pos: p.to_array(),
                            normal: [(n.x * 127.0) as i8, (n.y * 127.0) as i8, (n.z * 127.0) as i8, roughness],
                            uv: [(uv[0].clamp(0.0, 1.0) * 65535.0).round() as u16, (uv[1].clamp(0.0, 1.0) * 65535.0).round() as u16],
                            layer,
                            color,
                            piece: pi as u32,
                            edge_dist: [f16_bits(FAR_FROM_EDGES); 4],
                            edge_weight: [0; 4],
                            tread: [0.0; 2],
                        });
                        let world = rest[pi] + p;
                        max_r2 = max_r2.max(world.x * world.x + world.z * world.z);
                        max_h = max_h.max(world.y);
                        (self.vertices.len() as i32 - 1 - base_vertex) as u32
                    };
                    let at = if laid.is_some() { vertex() } else { *made.entry((i, layer, color)).or_insert_with(vertex) };
                    corners.push(at);
                }
                for k in 1..corners.len() - 1 {
                    self.indices.extend_from_slice(&[corners[0], corners[k + 1], corners[k]]);
                }
                polygons.push(at_corners.iter().map(|&p| rest[pi] + p).collect());
            }
            ranges.push((piece_first, self.indices.len() as u32 - piece_first));
        }
        let pieces = model
            .pieces
            .iter()
            .zip(&ranges)
            .map(|(p, &(first_index, index_count))| PieceInfo { name: p.name.clone(), parent: p.parent, offset: fixed(p.offset), first_index, index_count })
            .collect();
        let id = self.models.len() as ModelId;
        self.models.push(ModelInfo {
            name: name.to_string(),
            first_index,
            index_count: self.indices.len() as u32 - first_index,
            base_vertex,
            pieces,
            radius: max_r2.sqrt(),
            height: max_h,
            footprint: Footprint::of(&polygons, rest.first().copied().unwrap_or(Vec3::ZERO)),
        });
        self.by_name.insert(key, id);
        let mut used: Vec<u32> = slots.to_vec();
        used.sort_unstable();
        used.dedup();
        self.surfaced.insert(id, used);
        self.dirty = true;
        id
    }

    /// Draws `far` in `near`'s place far off (`FAR_PIXELS`): both models with surfaces, of the
    /// same pieces in the same order (or nothing is done; whether it was).
    pub fn set_far(&mut self, near: ModelId, far: ModelId) -> bool {
        let (a, b) = (&self.models[near as usize].pieces, &self.models[far as usize].pieces);
        let same = a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.name.eq_ignore_ascii_case(&y.name) && x.parent == y.parent);
        if same {
            self.far.insert(near, far);
            let most = near.max(far) as usize + 1;
            if self.far_roles.len() < most {
                self.far_roles.resize(most, 0);
            }
            self.far_roles[near as usize] = FAR_HAS;
            self.far_roles[far as usize] = FAR_IS;
        }
        same
    }

    /// Whether a model has a lighter one drawn in its place far off ([`FAR_HAS`]), is such a
    /// lighter one ([`FAR_IS`]), or neither (0).
    pub(super) fn far_role(&self, id: ModelId) -> u8 {
        self.far_roles.get(id as usize).copied().unwrap_or(0)
    }

    /// The material slots a model with a surface paints with (none for any other).
    pub(super) fn surface_slots(&self, id: ModelId) -> &[u32] {
        self.surfaced.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Whether a model is painted with a surface of its own (`add_surfaced`).
    pub fn is_surfaced(&self, id: ModelId) -> bool {
        self.surfaced.contains_key(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_formats::model3do::{Piece, Primitive};
    use ta_formats::surface;

    /// A DXT5 of `side` with all its levels (read as it is), as bytes.
    fn whole_paint(side: u32) -> Vec<u8> {
        let levels = side.ilog2() + 1;
        let mut file = dds::header(side, side, levels, Some(b"DXT5"));
        file.extend(std::iter::repeat_n(0u8, (0..levels).map(|l| ((side >> l).max(1).div_ceil(4).pow(2) * 16) as usize).sum()));
        file
    }

    /// An uncompressed texture of 64 x 64 (decoded, grown and encoded again).
    fn small_paint() -> Vec<u8> {
        let mut file = dds::header(64, 64, 1, None);
        file.extend(std::iter::repeat_n(90u8, 64 * 64 * 4));
        file
    }

    fn loader() -> Loader {
        let files: HashMap<String, Vec<u8>> = [("whole.dds".to_string(), whole_paint(SIDE)), ("fine.dds".to_string(), whole_paint(FINE_SIDE)), ("small.dds".to_string(), small_paint())].into();
        Arc::new(move |path: &str| files.get(path).cloned())
    }

    fn material(albedo: &str, specular: &str, normals: &str) -> Material {
        Material { name: "paint".into(), albedo: albedo.into(), specular: specular.into(), normals: normals.into(), mean: [10, 20, 30] }
    }

    /// The bytes of each level of a layer of `side` (BC3, or plain colour).
    fn level_bytes(side: u32, compressed: bool) -> Vec<usize> {
        (0..=side.ilog2()).map(|l| if compressed { ((side >> l).max(1).div_ceil(4).pow(2) * 16) as usize } else { ((side >> l).max(1).pow(2) * 4) as usize }).collect()
    }

    /// What the workers make of a material: its three textures as the arrays hold them, every
    /// level (BC3 blocks; plain colour for a card that reads none), each of its own side (a
    /// texture of 1024 at 1024 when the paint is fine, else at 512; a small one grown to 512), or
    /// why not.
    #[test]
    fn paint_is_made_as_the_arrays_hold_it() {
        let paths = ["fine.dds".to_string(), "small.dds".to_string(), "whole.dds".to_string()];
        let made = make_paint(&loader(), &paths, true, true).unwrap();
        assert_eq!(made.iter().map(|l| l.side).collect::<Vec<_>>(), [FINE_SIDE, SIDE, SIDE]);
        for layer in &made {
            assert_eq!(layer.levels.iter().map(Vec::len).collect::<Vec<_>>(), level_bytes(layer.side, true));
        }
        let coarse = make_paint(&loader(), &paths, true, false).unwrap();
        assert_eq!(coarse.iter().map(|l| l.side).collect::<Vec<_>>(), [SIDE; 3], "not fine: every texture at 512");
        let plain = make_paint(&loader(), &paths, false, true).unwrap();
        assert_eq!(plain[0].side, SIDE, "never fine as plain colour");
        assert_eq!(plain[1].levels.iter().map(Vec::len).collect::<Vec<_>>(), level_bytes(SIDE, false));
        let missing = make_paint(&loader(), &["whole.dds".into(), "gone.dds".into(), "whole.dds".into()], true, true).err().expect("a texture missing");
        assert!(missing.contains("gone.dds"), "{missing}");
    }

    /// A material's paint is made on the workers and put in the arrays within the frame's share of
    /// time, room made a step at a time (a fine texture in the fine array); one whose textures are
    /// not there stays in its colour on average; the paint is let go between battles, and when it
    /// stops or starts being fine, and asked for again; the card refusing the room leaves an array
    /// as it was; with no room for fine layers, a fine texture takes its 512 level in the plain one.
    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn paint_is_made_on_workers_and_let_go() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
        let features = adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { required_features: features, ..Default::default() })).expect("GPU device");
        let compressed = features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let mut paint = SurfaceTextures::new(&device);
        let a = paint.register(&material("fine.dds", "small.dds", "whole.dds"));
        let b = paint.register(&material("whole.dds", "gone.dds", "whole.dds"));
        assert_eq!(paint.register(&material("FINE.dds", "small.dds", "whole.dds")), a, "one slot for the same textures");
        paint.want(a);
        assert!(paint.workers.is_none(), "nothing read without the game's files");
        paint.set_loader(loader());
        let settle = |paint: &mut SurfaceTextures| {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !paint.settled() {
                paint.update(&device, &queue);
                assert!(Instant::now() < deadline, "the paint was never made");
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        paint.want(a);
        paint.want(b);
        settle(&mut paint);
        assert_eq!(paint.counts(), (2, 1));
        let fine = if compressed { [FINE, 0, 1] } else { [0, 1, 2] };
        assert_eq!((paint.entries[a as usize].words, paint.entries[b as usize].failed), (Some(fine), true));
        let step = (GROWTH_BYTES / layer_bytes(SIDE, compressed)) as u32;
        assert_eq!(paint.plain.capacity, step, "room for a step of layers");
        assert_eq!(paint.fine_array.capacity, if compressed { (GROWTH_BYTES / layer_bytes(FINE_SIDE, true)) as u32 } else { 0 });
        // Asked again: read once.
        paint.want(a);
        assert!(paint.settled());
        // Let go: nothing held, the bind group made again, read again when drawn.
        paint.release(&device, &queue);
        assert_eq!((paint.plain.capacity, paint.plain.held, paint.fine_array.capacity, paint.entries[b as usize].failed), (0, 0, 0, false));
        assert!(paint.update(&device, &queue), "the bind group is made again");
        paint.want(a);
        settle(&mut paint);
        assert_eq!(paint.entries[a as usize].words, Some(fine));
        // No longer fine: let go, and made again at 512, all of it in the plain array.
        paint.set_fine(&device, &queue, false);
        assert_eq!((paint.entries[a as usize].words, paint.plain.held), (None, 0));
        paint.want(a);
        settle(&mut paint);
        assert_eq!(paint.entries[a as usize].words, Some([0, 1, 2]));
        paint.set_fine(&device, &queue, true);
        // Room the card cannot give (more layers than it has): refused, the array as it was.
        assert!(!paint.plain.grow(&device, &queue, compressed, device.limits().max_texture_array_layers + 1));
        assert_eq!(paint.plain.capacity, 0);
        // No room in the fine array: its texture at 512 in the plain one.
        if compressed {
            paint.release(&device, &queue);
            paint.fine_array.full = true;
            paint.want(a);
            settle(&mut paint);
            assert_eq!(paint.entries[a as usize].words, Some([0, 1, 2]));
        }
        // Room for three layers at most: what is asked for past it is drawn in its colours on
        // average.
        paint.release(&device, &queue);
        paint.most = 3;
        paint.want(a);
        settle(&mut paint);
        assert!(paint.entries[a as usize].words.is_some());
        let c = paint.register(&material("small.dds", "small.dds", "small.dds"));
        paint.want(c);
        settle(&mut paint);
        assert!(paint.entries[c as usize].failed && paint.plain.full);
        assert_eq!(paint.memory(), 3 * layer_bytes(SIDE, compressed) + paint.fine_array.capacity as u64 * layer_bytes(FINE_SIDE, compressed));
    }

    /// Runs a fixture of unit.wgsl's functions on the GPU: its `main` once a case, `cases` (bytes
    /// of `count` cases) at binding 0, what it writes at binding 1 (`rows` rows of four floats)
    /// read back.
    fn run_fixture(label: &str, source: String, cases: &[u8], count: u32, rows: u64) -> Vec<[f32; 4]> {
        use crate::gpu::CheckedBufferInit;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
        let input = device.create_buffer_checked(&wgpu::util::BufferInitDescriptor { label: Some(label), contents: cases, usage: wgpu::BufferUsages::STORAGE });
        let bytes = rows * 16;
        let output = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let readback = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: None, module: &shader, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let layout = pipeline.get_bind_group_layout(0);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: input.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() }],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(count, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, bytes);
        queue.submit([encoder.finish()]);
        readback.slice(..).map_async(wgpu::MapMode::Read, |result| result.expect("fixture readback"));
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = readback.slice(..).get_mapped_range().unwrap();
        bytemuck::cast_slice::<u8, [f32; 4]>(&data).to_vec()
    }

    /// Reborn's finish of a model's own surface (`surfaced_finish` in unit.wgsl, run as it is in
    /// a compute shader): a blue-grey paint's cast mostly gone and it matte; painted highlights
    /// brought down on a shoulder; the player's colour where the alpha says, a little darker;
    /// strong mirroring polished, bright metal; the gloss map's outlines brighter, smoother metal
    /// edges; burnt out, rough and no metal.
    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn reborn_finishes_a_surfaces_paint() {
        let unit = include_str!("../shaders/unit.wgsl");
        let (start, end) = (unit.find("struct SurfacedFinish {").unwrap(), unit.find("fn model_surface(").unwrap());
        let source = format!(
            "{}\nstruct Case {{ paint: vec4<f32>, finish: vec4<f32>, wide: vec4<f32>, player: vec4<f32> }};
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> results: array<vec4<f32>, 12>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    let c = cases[id.x];
    let f = surfaced_finish(c.paint.rgb, c.finish, c.wide, c.player.rgb, c.player.a);
    results[id.x] = vec4<f32>(f.albedo, 0.0);
    results[id.x + 6u] = vec4<f32>(f.material, 0.0, 0.0);
}}",
            &unit[start..end]
        );
        let grey = [0.10f32, 0.12, 0.16, 0.0];
        let (plain, flat, player) = ([0.13f32, 0.27, 0.0, 0.0], [0.13f32, 0.27, 0.0, 0.0], [0.03f32, 0.1, 0.7, 0.0]);
        let cases: [[[f32; 4]; 4]; 6] = [
            [grey, plain, flat, player],
            [[0.4, 0.4, 0.4, 0.0], plain, flat, player],
            [grey, [0.13, 0.27, 0.0, 1.0], flat, player],
            [grey, [1.0, 0.8, 0.0, 0.0], [1.0, 0.8, 0.0, 0.0], player],
            [grey, [0.13, 0.9, 0.0, 0.0], flat, player],
            [grey, [0.13, 0.9, 0.0, 0.0], flat, [0.03, 0.1, 0.7, 1.0]],
        ];
        let out = run_fixture("surfaced finish fixture", source, bytemuck::cast_slice(&cases), cases.len() as u32, 12);
        let (albedo, material) = (&out[..6], &out[6..]);
        let chroma = |c: [f32; 4]| (c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])) / c[0].max(c[1]).max(c[2]);
        let lum = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        // A blue-grey paint: greyer, as bright, matte, no metal.
        assert!(chroma(albedo[0]) < chroma(grey) * 0.4 && (lum(albedo[0]) - lum(grey)).abs() < 0.02, "{:?}", albedo[0]);
        assert!((0.55..0.75).contains(&material[0][0]) && material[0][1] < 0.01, "{:?}", material[0]);
        // A painted highlight: brought down.
        assert!(lum(albedo[1]) < 0.4 * 0.8, "{:?}", albedo[1]);
        // The player's colour, a little darker.
        assert!((0..3).all(|k| (albedo[2][k] - player[k] * 0.85).abs() < 1e-3), "{:?}", albedo[2]);
        // Polished metal: bright, smooth, metal.
        assert!(lum(albedo[3]) > 0.4 && material[3][0] < 0.2 && material[3][1] > 0.85, "{:?} {:?}", albedo[3], material[3]);
        // An outline: brighter than the paint round it, smoother, metal; burnt out, none of that.
        assert!(lum(albedo[4]) > lum(albedo[0]) && material[4][0] < material[0][0] - 0.2 && material[4][1] > 0.7, "{:?} {:?}", albedo[4], material[4]);
        assert!((material[5][0] - 0.9).abs() < 1e-3 && material[5][1] == 0.0 && lum(albedo[5]) < lum(albedo[4]), "{:?} {:?}", albedo[5], material[5]);
    }

    /// A film of rain water on a model's own surface (`rain_film` in unit.wgsl, run as it is in a
    /// compute shader, under a rainy sky), against TA's models' film (Fresnel's share of the sky,
    /// from the haze at the horizon to the zenith overhead, on the paint's relief): it lies on
    /// the face, so a texel its normal map turns grazing mirrors no more than the face does; a
    /// side seen from above mirrors the ground; all of it thinner.
    #[test]
    #[ignore = "requires a GPU; run with --ignored"]
    fn rain_lies_on_a_surfaces_faces() {
        let unit = include_str!("../shaders/unit.wgsl");
        let (start, end) = (unit.find("// How thin a film of rain water").unwrap(), unit.find("// Lit colour of a model fragment").unwrap());
        let thin: f32 = unit.split("const SURFACED_FILM: f32 = ").nth(1).and_then(|s| s.split(';').next()).unwrap().parse().unwrap();
        let (fog, zenith, ground) = ([0.38f32, 0.42, 0.48], [0.2f32, 0.25, 0.36], [0.19f32, 0.18, 0.17]);
        let source = format!(
            "struct Frame {{ fog: vec4<f32>, sky_zenith: vec4<f32>, sky: vec4<f32> }};
var<private> frame: Frame = Frame(vec4<f32>({:?}, {:?}, {:?}, 0.0), vec4<f32>({:?}, {:?}, {:?}, 0.0), vec4<f32>(0.0, 0.0, 0.0, 1.0));
{}
struct Case {{ face: vec4<f32>, v: vec4<f32>, ground: vec4<f32> }};
@group(0) @binding(0) var<storage, read> cases: array<Case>;
@group(0) @binding(1) var<storage, read_write> results: array<vec4<f32>, 3>;
@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {{
    let c = cases[id.x];
    let film = rain_film(c.face.xyz, c.v.xyz, c.ground.xyz);
    results[id.x] = vec4<f32>(film.rgb * film.a, 0.0);
}}",
            fog[0], fog[1], fog[2], zenith[0], zenith[1], zenith[2], &unit[start..end]
        );
        let unit_vec = |x: f32, y: f32, z: f32| {
            let l = (x * x + y * y + z * z).sqrt();
            [x / l, y / l, z / l, 0.0]
        };
        // The eye 70 degrees up; a top face, a texel of its relief turned 60 degrees away from the
        // eye, and a side facing the eye.
        let eye = unit_vec(0.0, 0.94, 0.34);
        let (top, turned, side) = (unit_vec(0.0, 1.0, 0.0), unit_vec(0.0, 0.5, -0.866), unit_vec(0.0, 0.0, 1.0));
        let at = [ground[0], ground[1], ground[2], 0.0];
        let cases: [[[f32; 4]; 3]; 3] = [[top, eye, at], [side, eye, at], [top, top, at]];
        let out = run_fixture("rain film fixture", source, bytemuck::cast_slice(&cases), cases.len() as u32, 3);
        // TA's film on a shading normal `n` (unit.wgsl's `shade_model`).
        let ta = |n: [f32; 4], v: [f32; 4]| {
            let nv = n[0] * v[0] + n[1] * v[1] + n[2] * v[2];
            let ry = 2.0 * nv * n[1] - v[1];
            let t = (ry * 1.5).clamp(0.0, 1.0);
            let fresnel = 0.03 + 0.97 * (1.0 - nv.max(0.0)).powi(5);
            [0, 1, 2, 3].map(|k| if k < 3 { (fog[k] + (zenith[k] - fog[k]) * t) * fresnel } else { 0.0 })
        };
        let lum = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let near = |a: [f32; 4], b: [f32; 4]| (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5);
        // A texel of the top turned grazing mirrors as the face does, thinner: a tenth of what
        // TA's film on the turned texel mirrors, at the most.
        let face = ta(top, eye).map(|c| c * thin);
        assert!(near(out[0], face), "{:?} against {face:?}", out[0]);
        assert!(lum(out[0]) < 0.1 * lum(ta(turned, eye)), "{:?} against {:?}", out[0], ta(turned, eye));
        // A side seen from above mirrors the ground below the horizon (TA's the haze), thinner.
        let fresnel = 0.03 + 0.97 * (1.0f32 - 0.34 / (0.94f32 * 0.94 + 0.34 * 0.34).sqrt()).powi(5);
        let mirrored = [0, 1, 2, 3].map(|k| if k < 3 { ground[k] * 0.8 * fresnel * thin } else { 0.0 });
        assert!(near(out[1], mirrored), "{:?} against {mirrored:?}", out[1]);
        assert!(lum(out[1]) < 0.5 * thin * lum(ta(side, eye)), "{:?} against {:?}", out[1], ta(side, eye));
        // Looked at straight down, the zenith, thinner.
        let overhead = ta(top, top).map(|c| c * thin);
        assert!(near(out[2], overhead), "{:?} against {overhead:?}", out[2]);
    }

    /// A hull and a turret on it, a triangle each: the turret's painted with the surface's
    /// material, the hull's in its palette colour (it names no material).
    #[test]
    fn a_surfaced_model_takes_its_surfaces_coordinates_normals_and_slots() {
        let tri = |texture: Option<&str>| Primitive { color_index: 7, indices: vec![0, 1, 2], texture: texture.map(str::to_string) };
        let corners = vec![[0, 0, 0], [65536, 0, 0], [0, 0, 65536]];
        let model = Model3do {
            pieces: vec![
                Piece { name: "base".into(), parent: None, offset: [0; 3], vertices: corners.clone(), primitives: vec![tri(None)], selection_primitive: None, children: vec![1] },
                Piece { name: "turret".into(), parent: Some(0), offset: [0, 4 << 16, 0], vertices: corners, primitives: vec![tri(Some("Paint"))], selection_primitive: None, children: vec![] },
            ],
        };
        let vertex = |u: f32| surface::Vertex { uv: [u, 0.5], normal: [0, 0, 32767] };
        let surface = Surface {
            materials: vec![Material { name: "paint".into(), albedo: "a.dds".into(), specular: "s.dds".into(), normals: "n.dds".into(), mean: [10, 20, 30] }],
            pieces: vec![surface::Piece { vertices: vec![vertex(0.0); 3] }, surface::Piece { vertices: vec![vertex(0.0), vertex(1.0), vertex(0.25)] }],
            far: String::new(),
        };
        let palette = ta_formats::image::Palette::parse(&vec![0u8; 1024]).unwrap();
        let none = |_: &str| None;
        let mut library = ModelLibrary::default();
        let id = library.add_surfaced("tank", &model, &surface, &[5], none, &palette);
        assert_eq!(library.add_surfaced("TANK", &model, &surface, &[5], none, &palette), id, "registered once");
        let info = library.info(id).clone();
        assert_eq!(info.pieces.len(), 2);
        assert_eq!(info.index_count, 6);
        assert!((info.height - 4.0).abs() < 1e-5);
        assert_eq!(library.surface_slots(id), [5]);
        let turret = &library.vertices[3..6];
        assert_eq!(turret.iter().map(|v| v.uv[0]).collect::<Vec<_>>(), [0, 65535, 16384]);
        assert!(turret.iter().all(|v| v.layer == 5 && v.color[..3] == [10, 20, 30] && v.piece == 1));
        // (The normal's z the other way round, as the places'.)
        assert_eq!(turret[0].normal[2], -127);
        assert_eq!(library.vertices[0].layer & 0xFFFF, 0xFFFF, "the hull in its palette colour");
        // The triangles wound as the library winds TA's.
        assert_eq!(&library.indices[3..6], &[3, 5, 4]);
        // A second triangle on the turret's corners shares their vertices.
        let mut twice = model.clone();
        twice.pieces[1].primitives.push(tri(Some("paint")));
        let mut library = ModelLibrary::default();
        let near = library.add_surfaced("tank", &twice, &surface, &[5], none, &palette);
        assert_eq!(library.vertices.len(), 6);
        assert_eq!(library.indices.len(), 9);
        // A far model of the same pieces is drawn in its place far off; another's is not taken.
        let far = library.add_surfaced("tank_lod1", &model, &surface, &[5], none, &palette);
        assert!(library.set_far(near, far));
        let root = glam::Mat4::IDENTITY;
        assert_eq!(library.detail_for(near, root, None, true, None), near, "no camera: as itself");
        let mut camera = crate::camera::Camera::new(glam::Vec2::new(1920.0, 1080.0));
        camera.zoom = 0.2;
        assert_eq!(library.detail_for(near, root, Some(&camera), false, None), far, "small on screen");
        camera.zoom = 40.0;
        assert_eq!(library.detail_for(near, root, Some(&camera), false, None), near, "large on screen");
        assert_eq!((library.far_role(near), library.far_role(far), library.far_role(near + 7)), (FAR_HAS, FAR_IS, 0));
        let mut other = model.clone();
        other.pieces[1].name = "gun".into();
        let stranger = library.add_surfaced("other", &other, &surface, &[5], none, &palette);
        assert!(!library.set_far(near, stranger));
        // Between the two thresholds (45 pixels across, the classic camera's 2 x radius x zoom),
        // as it was drawn the frame before: as itself when it was, else its lighter one.
        let info = library.info(near).clone();
        camera.mode = crate::camera::CameraMode::Classic;
        camera.zoom = 45.0 / (2.0 * info.radius.max(info.height * 0.5));
        assert_eq!(library.detail_for(near, root, Some(&camera), false, Some(false)), near, "drawn as itself: it stays so");
        assert_eq!(library.detail_for(near, root, Some(&camera), false, Some(true)), far, "drawn far off: it stays so");
        assert_eq!(library.detail_for(near, root, Some(&camera), false, None), far, "first seen: under 48, far off");
        camera.zoom = 40.0 / (2.0 * info.radius.max(info.height * 0.5));
        assert_eq!(library.detail_for(near, root, Some(&camera), false, Some(false)), far, "under 42: far off");
        camera.zoom = 50.0 / (2.0 * info.radius.max(info.height * 0.5));
        assert_eq!(library.detail_for(near, root, Some(&camera), false, Some(true)), near, "over 48: as itself");
    }

    /// Polygons naming no material: one naming a TA texture painted with it, laid on as TA lays
    /// it (its own corners, its frames' count kept for team colours), palette faces of two
    /// colours not sharing their corners, and the selection plate left out.
    #[test]
    fn a_surfaced_models_other_polygons_are_drawn_as_tas() {
        let prim = |colour: u32, texture: Option<&str>| Primitive { color_index: colour, indices: vec![0, 1, 2], texture: texture.map(str::to_string) };
        let model = Model3do {
            pieces: vec![Piece {
                name: "base".into(),
                parent: None,
                offset: [0; 3],
                vertices: vec![[0, 0, 0], [65536, 0, 0], [0, 0, 65536]],
                primitives: vec![prim(0, Some("armcolour")), prim(1, None), prim(2, None), prim(3, None)],
                selection_primitive: Some(3),
                children: vec![],
            }],
        };
        let surface = Surface { materials: vec![], pieces: vec![surface::Piece { vertices: vec![surface::Vertex { uv: [0.0; 2], normal: [0, 32767, 0] }; 3] }], far: String::new() };
        let mut colours = vec![0u8; 1024];
        colours[4..8].copy_from_slice(&[200, 0, 0, 0]);
        colours[8..12].copy_from_slice(&[0, 200, 0, 0]);
        let palette = ta_formats::image::Palette::parse(&colours).unwrap();
        let team = TextureRef { layer: 7, frames: 10, emissive: false, roughness: 0.5, metallic: 0.2, mean: [128; 3] };
        let mut library = ModelLibrary::default();
        let id = library.add_surfaced("mixed", &model, &surface, &[], |n: &str| n.eq_ignore_ascii_case("armcolour").then_some(team), &palette);
        // Three polygons drawn (the plate left out), three corners each, none shared.
        assert_eq!(library.info(id).index_count, 9);
        assert_eq!(library.vertices.len(), 9);
        let painted = &library.vertices[0..3];
        assert!(painted.iter().all(|v| v.layer & 0xFFFF == 7 && (v.layer >> 16) & 0xFF == 10), "TA's texture, its ten frames");
        assert_ne!(painted[0].uv, painted[1].uv, "laid on as TA lays it");
        assert_eq!(library.vertices[3].color[..3], [200, 0, 0]);
        assert_eq!(library.vertices[6].color[..3], [0, 200, 0]);
    }
}
