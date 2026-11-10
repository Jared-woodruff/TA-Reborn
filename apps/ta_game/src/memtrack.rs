//! A heap profiler for leak hunts (cargo feature `memtrack`, Windows only, never in a release):
//! about one allocation in every [`SAMPLE`] bytes allocated is recorded with the call stack that
//! made it (each standing for that many bytes), and [`report`] logs the places in the game's own
//! code holding the most of what is still allocated, with what each gained since the report
//! before: a leak is a place that keeps gaining. `--demo spy` reports every ten seconds of
//! battle (`app/spy.rs`). Build it in a target folder of its own (`tools/soak/README.md`), so
//! that the game's own build stays as it is.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

/// Bytes allocated, on average, between two allocations recorded.
const SAMPLE: isize = 128 * 1024;
/// Frames of a call stack kept.
const DEPTH: usize = 40;
/// Frames of the game's own code a place is told by (the first, nearest the allocation).
const PLACE: usize = 3;
const SHARDS: usize = 64;

/// The allocator: the system's, with the sampling.
pub struct Tracker;

#[derive(Clone, Copy)]
struct Sample {
    /// Bytes it stands for.
    weight: usize,
    depth: u16,
    stack: [usize; DEPTH],
}

type Shard = Mutex<Option<HashMap<usize, Sample>>>;
/// The live samples, by address.
static LIVE: [Shard; SHARDS] = [const { Mutex::new(None) }; SHARDS];
/// Each place's bytes at the last report, and every address's name.
static LAST: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);
static NAMES: Mutex<Option<HashMap<usize, String>>> = Mutex::new(None);

thread_local! {
    /// In the profiler's own work: this thread's allocations are left alone.
    static BUSY: Cell<bool> = const { Cell::new(false) };
    /// Bytes still to be allocated before the next sample.
    static UNTIL: Cell<isize> = const { Cell::new(SAMPLE) };
}

fn shard(p: usize) -> &'static Shard {
    &LIVE[((p as u64 >> 4).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 58) as usize]
}

/// `f`, with this thread's allocations left alone meanwhile (none when it is in here already).
fn alone<R>(f: impl FnOnce() -> R) -> Option<R> {
    BUSY.try_with(|busy| {
        if busy.replace(true) {
            return None;
        }
        let r = f();
        busy.set(false);
        Some(r)
    })
    .ok()
    .flatten()
}

fn note_alloc(p: *mut u8, size: usize) {
    let due = UNTIL
        .try_with(|until| {
            let left = until.get() - size as isize;
            if left > 0 {
                until.set(left);
                false
            } else {
                until.set(SAMPLE);
                true
            }
        })
        .unwrap_or(false);
    if !due {
        return;
    }
    alone(|| {
        let mut frames = [std::ptr::null_mut::<c_void>(); DEPTH];
        // SAFETY: fills the frames given with this thread's return addresses.
        let depth = unsafe { windows::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace(2, &mut frames, None) };
        let mut stack = [0usize; DEPTH];
        for (s, f) in stack.iter_mut().zip(&frames) {
            *s = *f as usize;
        }
        let sample = Sample { weight: size.max(SAMPLE as usize), depth, stack };
        shard(p as usize).lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(p as usize, sample);
    });
}

fn note_free(p: *mut u8) {
    alone(|| {
        if let Some(map) = shard(p as usize).lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            map.remove(&(p as usize));
        }
    });
}

// SAFETY: every call goes on to the system's allocator as it came, its result given back as it
// is; what is recorded is kept in allocations made while this thread is `alone` (not recorded).
unsafe impl GlobalAlloc for Tracker {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as the caller's.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            note_alloc(p, layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as the caller's.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            note_alloc(p, layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        note_free(p);
        // SAFETY: as the caller's.
        unsafe { System.dealloc(p, layout) }
    }

    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: as the caller's.
        let q = unsafe { System.realloc(p, layout, new_size) };
        if !q.is_null() {
            note_free(p);
            note_alloc(q, new_size);
        }
        q
    }
}

/// An address's function and line, from the program's debug information.
fn name_of(addr: usize, names: &mut HashMap<usize, String>) -> String {
    use windows::Win32::System::Diagnostics::Debug::{
        IMAGEHLP_LINEW64, SYMBOL_INFOW, SYMOPT_DEFERRED_LOADS, SYMOPT_LOAD_LINES, SYMOPT_UNDNAME, SymFromAddrW, SymGetLineFromAddrW64, SymInitializeW, SymSetOptions,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        // SAFETY: the symbol handler of this process, set up once.
        unsafe {
            SymSetOptions(SYMOPT_UNDNAME | SYMOPT_DEFERRED_LOADS | SYMOPT_LOAD_LINES);
            // (The program's folder first: its .pdb is there.)
            let dir = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_string_lossy().into_owned())).unwrap_or_default();
            let wide: Vec<u16> = dir.encode_utf16().chain(Some(0)).collect();
            if let Err(e) = SymInitializeW(GetCurrentProcess(), windows::core::PCWSTR(wide.as_ptr()), true) {
                log::warn!("memtrack: the symbol handler did not start ({e}); addresses are shown as they are");
            }
        }
    });
    names
        .entry(addr)
        .or_insert_with(|| {
            #[repr(C)]
            struct Info {
                info: SYMBOL_INFOW,
                name: [u16; 512],
            }
            // SAFETY: plain data, zero is a valid value of each field.
            let mut buf: Info = unsafe { std::mem::zeroed() };
            buf.info.SizeOfStruct = std::mem::size_of::<SYMBOL_INFOW>() as u32;
            buf.info.MaxNameLen = 512;
            let mut displacement = 0u64;
            // SAFETY: a structure of the size it says, with room for the name after it.
            let function = if unsafe { SymFromAddrW(GetCurrentProcess(), addr as u64, Some(&mut displacement), &mut buf.info) }.is_ok() {
                let len = (buf.info.NameLen as usize).min(511);
                // SAFETY: the name, `len` units long, starts at `Name` and runs on into `name`.
                let units = unsafe { std::slice::from_raw_parts(std::ptr::addr_of!(buf.info.Name).cast::<u16>(), len) };
                String::from_utf16_lossy(units)
            } else {
                static TOLD: std::sync::Once = std::sync::Once::new();
                TOLD.call_once(|| log::warn!("memtrack: no symbol for {addr:#x}: {}", windows::core::Error::from_thread()));
                format!("{addr:#x}")
            };
            let mut line = IMAGEHLP_LINEW64 { SizeOfStruct: std::mem::size_of::<IMAGEHLP_LINEW64>() as u32, ..Default::default() };
            let mut column = 0u32;
            // SAFETY: a structure of the size it says.
            let at = if unsafe { SymGetLineFromAddrW64(GetCurrentProcess(), addr as u64, &mut column, &mut line) }.is_ok() {
                // SAFETY: the symbol handler's own string, valid until the next call.
                let file = unsafe { line.FileName.to_string() }.unwrap_or_default();
                format!(" ({}:{})", file.rsplit(['\\', '/']).next().unwrap_or(""), line.LineNumber)
            } else {
                String::new()
            };
            format!("{function}{at}")
        })
        .clone()
}

/// Logs the places holding the most of what is still allocated, and what each gained since the
/// last report.
pub fn report(label: &str) {
    let lines = alone(|| {
        let mut stacks: HashMap<Vec<usize>, (usize, usize)> = HashMap::new();
        for shard in &LIVE {
            let live = shard.lock().unwrap_or_else(|e| e.into_inner());
            for s in live.iter().flat_map(|m| m.values()) {
                let e = stacks.entry(s.stack[..s.depth as usize].to_vec()).or_default();
                e.0 += s.weight;
                e.1 += 1;
            }
        }
        let mut names_guard = NAMES.lock().unwrap_or_else(|e| e.into_inner());
        let names = names_guard.get_or_insert_with(HashMap::new);
        // (A place: the first frames of the game's own crates, nearest the allocation; what has
        // none, the first frames past the allocator's.)
        let mut places: HashMap<String, (usize, usize)> = HashMap::new();
        let mut total = 0usize;
        for (stack, (bytes, count)) in stacks {
            total += bytes;
            let frames: Vec<String> = stack.iter().map(|&a| name_of(a, names)).collect();
            let own: Vec<&String> = frames.iter().filter(|f| f.starts_with("ta_")).take(PLACE).collect();
            let place = if own.is_empty() {
                frames.iter().filter(|f| !(f.contains("alloc::") || f.contains("memtrack") || f.contains("hashbrown") || f.starts_with("core::"))).take(PLACE).cloned().collect::<Vec<_>>().join(" <- ")
            } else {
                own.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" <- ")
            };
            let e = places.entry(place).or_default();
            e.0 += bytes;
            e.1 += count;
        }
        let mut last_guard = LAST.lock().unwrap_or_else(|e| e.into_inner());
        let last = last_guard.get_or_insert_with(HashMap::new);
        let mut ranked: Vec<(&String, &(usize, usize))> = places.iter().collect();
        ranked.sort_by_key(|(_, (bytes, _))| std::cmp::Reverse(*bytes));
        let mb = |b: usize| b as f64 / (1 << 20) as f64;
        let mut out = vec![format!("memtrack {label}: {:.0} MB live, estimated from samples", mb(total))];
        for &(place, &(bytes, count)) in ranked.iter().take(18) {
            let before = last.get(place).copied().unwrap_or(0);
            out.push(format!("  {:8.1} MB ({:+7.1}) {:>6} samples  {place}", mb(bytes), mb(bytes) - mb(before), count));
        }
        // (And the places gaining most, wherever they rank.)
        let mut gaining: Vec<(&String, f64)> = places.iter().map(|(p, &(b, _))| (p, mb(b) - mb(last.get(p).copied().unwrap_or(0)))).filter(|(_, d)| *d > 0.5).collect();
        gaining.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (place, delta) in gaining.iter().take(8) {
            out.push(format!("  gaining {delta:+7.1} MB  {place}"));
        }
        *last = places.into_iter().map(|(p, (b, _))| (p, b)).collect();
        out
    });
    for line in lines.unwrap_or_default() {
        log::info!("{line}");
    }
}
