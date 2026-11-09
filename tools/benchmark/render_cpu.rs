// Invoked by render_cpu.ps1, which supplies the baseline/current source module paths.
use std::{alloc::{GlobalAlloc, Layout, System}, hint::black_box, sync::atomic::{AtomicBool, AtomicUsize, Ordering}, time::Instant};
use glam::Vec2;

struct Allocator;
static TRACK: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
#[global_allocator] static ALLOCATOR: Allocator = Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) { ALLOCS.fetch_add(1, Ordering::Relaxed); BYTES.fetch_add(layout.size(), Ordering::Relaxed); }
        unsafe { System.alloc(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) { ALLOCS.fetch_add(1, Ordering::Relaxed); BYTES.fetch_add(size, Ordering::Relaxed); }
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) { unsafe { System.dealloc(ptr, layout) } }
}

fn allocations(mut f: impl FnMut()) -> (usize, usize) {
    ALLOCS.store(0, Ordering::Relaxed); BYTES.store(0, Ordering::Relaxed); TRACK.store(true, Ordering::Relaxed);
    f(); TRACK.store(false, Ordering::Relaxed);
    (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed))
}
fn measure(mut old: impl FnMut(), mut new: impl FnMut(), rounds: usize) -> (f64, f64) {
    for _ in 0..20 { old(); new(); }
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for i in 0..11 {
        let timed = |f: &mut dyn FnMut(), times: &mut Vec<f64>| {
            let start = Instant::now(); for _ in 0..rounds { f(); }
            times.push(start.elapsed().as_secs_f64() * 1e6 / rounds as f64);
        };
        if i % 2 == 0 { timed(&mut old, &mut a); timed(&mut new, &mut b); }
        else { timed(&mut new, &mut b); timed(&mut old, &mut a); }
    }
    a.sort_by(f64::total_cmp); b.sort_by(f64::total_cmp); (a[5], b[5])
}
fn old_groups(models: usize, keys: &[Option<usize>]) -> Vec<Vec<u32>> {
    let mut groups = vec![Vec::new(); models];
    for (k, &m) in keys.iter().enumerate() {
        if let Some(group) = m.and_then(|m| groups.get_mut(m)) { group.push(k as u32); }
    }
    groups
}
fn main() {
    let quick = std::env::args().any(|a| a == "--quick");
    if quick { println!("Quick validation run: fewer iterations; use the full run for performance comparisons."); }
    println!("Optimized rustc CPU microbench; median of 11 alternating runs; microseconds/call. Whole LightGrid build and model grouping only, not frame time.");
    for (models, count) in [(512, 100), (512, 3500), (1024, 20_000)] {
        let keys: Vec<_> = (0..count).map(|i| (i % 9 != 0).then_some(i * 7919 % models)).collect();
        let mut groups = batching::InstanceGroups::default();
        groups.build(models, keys.iter().copied());
        assert_eq!(groups.groups().collect::<Vec<_>>(), old_groups(models, &keys).iter().map(Vec::as_slice).collect::<Vec<_>>());
        let alloc_old = allocations(|| { black_box(old_groups(black_box(models), black_box(&keys))); });
        let alloc_new = allocations(|| { groups.build(black_box(models), black_box(&keys).iter().copied()); black_box(&groups.order); });
        let (a,b) = measure(|| { black_box(old_groups(black_box(models), black_box(&keys))); }, || { groups.build(black_box(models), black_box(&keys).iter().copied()); black_box(&groups.order); }, if quick { 100 } else { 1000 });
        println!("groups models={models} instances={count} old_us={a:.3} new_us={b:.3} speedup={:.2} old_allocs={} new_allocs={} old_bytes={} new_bytes={}", a/b, alloc_old.0, alloc_new.0, alloc_old.1, alloc_new.1);
    }
    for count in [128, 1000, 5000, 20_000] {
        for uniform in [false, true] {
            let old: Vec<_> = (0..count).map(|i| {
                let pos = [(i * 7919 % 8000) as f32, 20.0, (i * 3557 % 8000) as f32];
                let color = if uniform { [2.0, 1.0, 0.5] } else { [(i * 31 % 71) as f32 * 0.1, (i * 23 % 51) as f32 * 0.1, (i * 7 % 41) as f32 * 0.1] };
                old_lights::PointLight::new(pos, color, if uniform { 90.0 } else { 40.0 + (i * 17 % 760) as f32 })
            }).collect();
            let new: Vec<_> = old.iter().map(|l| new_lights::PointLight::new(l.position, l.color, l.radius)).collect();
            let a = old_lights::LightGrid::build(&old, Vec2::ZERO, Vec2::splat(8000.0));
            let b = new_lights::LightGrid::build(&new, Vec2::ZERO, Vec2::splat(8000.0));
            assert_eq!((a.origin,a.cell,a.dims,&a.words), (b.origin,b.cell,b.dims,&b.words));
            assert_eq!(a.lights.iter().map(|l| (l.pos_radius,l.color)).collect::<Vec<_>>(), b.lights.iter().map(|l| (l.pos_radius,l.color)).collect::<Vec<_>>());
            let (a,b) = measure(|| { black_box(old_lights::LightGrid::build(black_box(&old), Vec2::ZERO, Vec2::splat(8000.0))); }, || { black_box(new_lights::LightGrid::build(black_box(&new), Vec2::ZERO, Vec2::splat(8000.0))); }, if quick { 30 } else { 300 });
            println!("lights={count} uniform={uniform} old_us={a:.3} new_us={b:.3} speedup={:.2}", a/b);
        }
    }
}
