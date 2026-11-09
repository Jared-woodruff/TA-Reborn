//! Idle alert bookkeeping. Work stays linear in the idle population; only the lamps
//! that can be drawn are sorted. The scratch vector is reused between frames.

use std::collections::HashMap;
use ta_sim::UnitId;
use super::{IDLE_ALERT, IDLE_ALERT_END};

const VISIBLE: usize = 24;

#[derive(Default)]
pub(super) struct IdleAlerts {
    since: HashMap<UnitId, (f32, bool)>,
    due: Vec<(UnitId, f32)>,
}

impl IdleAlerts {
    pub(super) fn update(&mut self, idle: impl IntoIterator<Item = UnitId>, now: f32) -> [Option<(UnitId, f32)>; VISIBLE] {
        for (_, present) in self.since.values_mut() { *present = false; }
        for id in idle {
            self.since.entry(id).or_insert((now, false)).1 = true;
        }
        self.since.retain(|_, (_, present)| *present);
        self.due.clear();
        self.due.extend(self.since.iter().filter_map(|(&id, &(since, _))| {
            let age = now - since;
            (IDLE_ALERT..IDLE_ALERT_END).contains(&age).then_some((id, age))
        }));
        let newest = |a: &(UnitId, f32), b: &(UnitId, f32)| a.1.total_cmp(&b.1).then(a.0.slot.cmp(&b.0.slot));
        if self.due.len() > VISIBLE {
            self.due.select_nth_unstable_by(VISIBLE, newest);
            self.due.truncate(VISIBLE);
        }
        self.due.sort_by(newest);
        std::array::from_fn(|i| self.due.get(i).copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baseline(since: &mut HashMap<UnitId, f32>, idle: &[UnitId], now: f32) -> Vec<(UnitId, f32)> {
        since.retain(|id, _| idle.contains(id));
        for &id in idle { since.entry(id).or_insert(now); }
        let mut due: Vec<_> = since.iter().map(|(&id, &t)| (id, now - t)).filter(|&(_, age)| (IDLE_ALERT..IDLE_ALERT_END).contains(&age)).collect();
        due.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.slot.cmp(&b.0.slot)));
        due.truncate(VISIBLE);
        due
    }

    #[test]
    fn alerts_match_existing_order_and_timing_through_idle_transitions() {
        let mut old = HashMap::new();
        let mut new = IdleAlerts::default();
        for frame in 0..90 {
            let idle: Vec<_> = (0..180).filter(|slot| (slot + frame / 5) % 7 != 0).map(|slot| UnitId { slot, generation: if frame < 40 { 1 } else { 2 } }).collect();
            let now = frame as f32 * 0.5;
            let expected = baseline(&mut old, &idle, now);
            let got: Vec<_> = new.update(idle, now).into_iter().flatten().collect();
            assert_eq!(got, expected, "frame {frame}");
        }
        assert!(new.update([], 100.0).into_iter().all(|x| x.is_none()));
        assert!(new.since.is_empty());
    }

    #[test]
    fn expired_alerts_stay_expired_until_the_unit_becomes_busy() {
        let id = UnitId { slot: 3, generation: 7 };
        let mut alerts = IdleAlerts::default();
        let (first, last) = (IDLE_ALERT, IDLE_ALERT_END);
        assert_eq!(alerts.update([id, id], 1.0)[0], None);
        assert_eq!(alerts.update([id], 1.0 + first - 1.0)[0], None, "a builder that has only just finished is not forgotten yet");
        assert_eq!(alerts.update([id], 1.0 + first)[0], Some((id, first)));
        assert_eq!(alerts.update([id], 1.0 + last)[0], None);
        assert_eq!(alerts.update([id], 2.0 + last)[0], None);
        alerts.update([], 3.0 + last);
        alerts.update([id], 4.0 + last);
        assert_eq!(alerts.update([id], 4.0 + last + first)[0], Some((id, first)));
    }

    /// `cargo test --release -p ta_game hud::idle::tests::idle_alert_benchmark -- --ignored --nocapture`
    #[test]
    #[ignore = "manual performance measurement"]
    fn idle_alert_benchmark() {
        use std::{hint::black_box, time::Instant};
        for count in [100, 1_000, 10_000] {
            let ids: Vec<_> = (0..count).map(|slot| UnitId { slot, generation: 1 }).collect();
            let mut old = HashMap::new();
            let mut new = IdleAlerts::default();
            baseline(&mut old, &ids, 0.0);
            new.update(ids.iter().copied(), 0.0);
            let mut old_times = Vec::new();
            let mut new_times = Vec::new();
            for round in 0..11 {
                let mut measure_old = || { let start = Instant::now(); for _ in 0..20 { black_box(baseline(&mut old, black_box(&ids), 5.0)); } start.elapsed().as_secs_f64() * 1e6 / 20.0 };
                let mut measure_new = || { let start = Instant::now(); for _ in 0..20 { black_box(new.update(black_box(&ids).iter().copied(), 5.0)); } start.elapsed().as_secs_f64() * 1e6 / 20.0 };
                let (before, after) = if round % 2 == 0 { (measure_old(), measure_new()) } else { let after = measure_new(); (measure_old(), after) };
                old_times.push(before); new_times.push(after);
            }
            old_times.sort_by(f64::total_cmp); new_times.sort_by(f64::total_cmp);
            println!("idle alerts: {count} units, baseline {:.2} us/frame, optimised {:.2} us/frame", old_times[5], new_times[5]);
        }
    }
}
