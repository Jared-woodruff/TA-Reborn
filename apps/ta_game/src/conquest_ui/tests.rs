use super::*;
use crate::setup::MapEntry;

fn entries(n: usize, starts: impl Fn(usize) -> usize) -> Vec<MapEntry> {
    (0..n).map(|i| MapEntry { name: format!("map{i}"), title: format!("Battlefield {i}"),
        description: String::new(), planet: "Green Planet".into(), size: "10 x 10".into(), start_positions: starts(i), info: Default::default() }).collect()
}

fn test_assets(maps: Vec<MapEntry>) -> Assets {
    use std::sync::Arc;
    let vfs = Arc::new(ta_vfs::Vfs::new());
    let data = Arc::new(ta_data::GameData::load(&vfs));
    let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p)));
    Assets { vfs, data, types, maps: maps.clone(), selectable_maps: maps, campaigns: Vec::new(), data_hash: 0, compatibility: Default::default(), mods: Default::default() }
}

#[test]
fn failed_creation_loading_and_abandonment_preserve_the_visible_campaign() {
    let dir = std::env::temp_dir().join(format!("conquest-page-{}", ta_config::profiles::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let maps = entries(8, |_| 3);
    let conquest = Conquest::new(&maps, "ARM", Level::Standard, Size::Small, 14).unwrap();
    let blocker = dir.join("blocked");
    std::fs::write(&blocker, b"unrelated data").unwrap();
    let mut page = ConquestPage::default();
    page.start_at(conquest.clone(), &blocker.join("conquest.toml"));
    assert!(page.state.is_none() && page.error.is_some(), "failed creation cannot allow unpersisted battles");
    assert_eq!(std::fs::read(&blocker).unwrap(), b"unrelated data");

    let path = dir.join("conquest.toml");
    page.start_at(conquest.clone(), &path);
    assert_eq!(page.state, Some(conquest.clone()));
    assert!(page.error.is_none());
    page.set_deployment_at(&conquest, Some(vec![]), &blocker.join("conquest.toml"));
    assert_eq!(page.state, Some(conquest.clone()), "failed roster writes leave the displayed army unchanged");
    assert_eq!(page.pending_deployment, Some(Some(vec![])), "failed selections remain available for retry");
    assert!(page.error.is_some());
    page.set_deployment_at(&conquest, page.pending_deployment.clone().unwrap(), &path);
    assert!(page.error.is_none() && page.pending_deployment.is_none());
    assert_eq!(page.state.as_ref().unwrap().deployment, Some(vec![]));
    let selected = page.state.clone().unwrap();
    page.set_deployment_at(&selected, None, &path);
    assert_eq!(page.state, Some(conquest.clone()));
    page.abandon_at(&blocker.join("conquest.toml"));
    assert_eq!(page.state, Some(conquest.clone()), "failed deletion retains the campaign and offers retry");
    assert!(page.error.is_some());
    std::fs::write(&path, b"broken saved galaxy [").unwrap();
    page.reload_at(&path);
    assert!(page.load_error.is_some(), "load failure blocks a fresh campaign until retry or explicit abandonment");
    assert_eq!(std::fs::read(&path).unwrap(), b"broken saved galaxy [");
    conquest.save_at(&path).unwrap();
    page.abandon_at(&path);
    assert!(page.error.is_some() && page.load_error.is_some(), "a repaired save must be reloaded before it can be abandoned");
    assert_eq!(Conquest::load_at(&path).unwrap(), Some(conquest.clone()));
    page.selected = Some(999);
    page.briefing = Some(999);
    page.reload_at(&path);
    assert!(page.load_error.is_none() && page.error.is_none() && page.selected.is_none() && page.briefing.is_none());
    page.abandon_at(&path);
    assert!(page.state.is_none() && page.error.is_none() && !path.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

/// Every page at sizes from a small window to a laptop's: the text keeps its size, nothing runs
/// off the screen, START CONQUEST is always in sight.
#[test]
fn pages_keep_their_text_size_and_stay_on_screen() {
    let maps = entries(40, |_| 3);
    let assets = test_assets(maps.clone());
    for size in [vec2(480.0, 360.0), vec2(640.0, 480.0), vec2(960.0, 540.0), vec2(1280.0, 720.0), vec2(1920.0, 1080.0)] {
        let ctx = egui::Context::default();
        ta_ui::theme::install(&ctx);
        ta_ui::motion::set_reduced(&ctx, true);
        ta_ui::motion::set_still(&ctx, true);
        let mut page = ConquestPage::default();
        let mut portraits = crate::encyclopaedia::Portraits::default();
        let screen = Rect::from_min_size(Pos2::ZERO, size);
        let run = |page: &mut ConquestPage, portraits: &mut crate::encyclopaedia::Portraits| {
            let mut content = Rect::NOTHING;
            for _ in 0..3 {
                ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ui| {
                    page.draw(ui, &assets, portraits);
                    content = ui.min_rect();
                }).drop_without_applying_deltas();
            }
            content
        };
        let content = run(&mut page, &mut portraits);
        assert!(content.width() <= size.x + 1.0 && content.height() <= size.y + 1.0, "{size:?}: the setup overflowed the viewport: {content:?}");
        let start = ctx.memory(|m| m.area_rect(egui::Id::new("conquest_new_start")));
        assert!(start.is_some_and(|r| screen.expand(1.0).contains_rect(r)), "{size:?}: START CONQUEST out of sight: {start:?}");

        page.load_error = Some("The saved galaxy needs repair. ".repeat(20));
        page.confirm_abandon = true;
        run(&mut page, &mut portraits);
        let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("conquest_load_error"));
        let scale = ctx.layer_transform_to_global(layer).map_or(1.0, |t| t.scaling);
        assert!(scale >= 0.995, "{size:?}: recovery actions shrank to {scale}");
        page.load_error = None;
        page.confirm_abandon = false;

        page.state = Conquest::new(&maps, "ARM", Level::Standard, Size::Large, 14);
        page.select_target();
        let content = run(&mut page, &mut portraits);
        assert!(content.width() <= size.x + 1.0 && content.height() <= size.y + 1.0, "{size:?}: galaxy overflowed the viewport: {content:?}");
        // The dialogs whole on the screen, their buttons (pinned at their foot) with them.
        let on_screen = |id: &str| {
            let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new(id));
            let rect = ctx.memory(|m| m.area_rect(layer.id)).unwrap_or(Rect::NOTHING);
            let to_global = ctx.layer_transform_to_global(layer).unwrap_or_default();
            (to_global.scaling, to_global * rect)
        };
        page.briefing = page.selected;
        run(&mut page, &mut portraits);
        let (scale, rect) = on_screen("conquest_briefing");
        assert!(scale >= 0.5, "{size:?}: the briefing shrank to {scale}");
        assert!(screen.expand(1.0).contains_rect(rect), "{size:?}: the briefing ran off the screen: {rect:?}");
        page.briefing = None;

        page.state = Conquest::demo(&maps);
        page.selected = None;
        page.debrief = true;
        run(&mut page, &mut portraits);
        let (scale, rect) = on_screen("conquest_debrief");
        assert!(scale >= 0.5 && screen.expand(1.0).contains_rect(rect), "{size:?}: the debrief ran off the screen: {rect:?} at {scale}");
        page.debrief = false;
        // (The demo has a counter-attack under way: its defence's briefing.)
        page.brief_defence();
        assert!(page.briefing.is_some(), "the demo's war strikes back");
        run(&mut page, &mut portraits);
        let (scale, rect) = on_screen("conquest_briefing");
        assert!(scale >= 0.5 && screen.expand(1.0).contains_rect(rect), "{size:?}: the defence's briefing ran off the screen: {rect:?} at {scale}");
        page.briefing = None;
        page.selected = None;
        // A promotion's choice of perks.
        if let Some(c) = page.state.as_mut() {
            c.perks.pop();
        }
        page.show_perks();
        run(&mut page, &mut portraits);
        let (scale, rect) = on_screen("conquest_perks");
        assert!(scale >= 0.5 && screen.expand(1.0).contains_rect(rect), "{size:?}: the promotion's choice ran off the screen: {rect:?} at {scale}");
        page.perk_later = true;
        for tab in [EmpireTab::Progress, EmpireTab::Veterans, EmpireTab::Journal] {
            page.empire_tab = tab;
            let content = run(&mut page, &mut portraits);
            assert!(content.width() <= size.x + 1.0 && content.height() <= size.y + 1.0, "{size:?}: campaign tab overflowed the viewport: {content:?}");
            let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("conquest_side"));
            let scale = ctx.layer_transform_to_global(layer).map_or(1.0, |t| t.scaling);
            assert!(scale >= 0.995, "{size:?}: the campaign tab shrank its text to {scale}");
            // (Nothing in the panel may push it wider than it was laid out: a row that cannot
            // wrap would.)
            if !compact_galaxy(size) {
                let (panel, _) = wide_layout(screen, 0.0);
                let side = ctx.memory(|m| m.area_rect(layer.id)).unwrap();
                assert!(side.width() <= panel.width() + 4.0, "{size:?} {tab:?}: the side panel grew to {} wide (laid out {})", side.width(), panel.width());
            }
        }
    }
}

#[test]
fn compact_maps_keep_positive_scrollable_geometry() {
    assert!(!compact_galaxy(vec2(1280.0, 720.0)));
    for screen in [vec2(480.0, 360.0), vec2(640.0, 480.0), vec2(960.0, 540.0), vec2(1280.0, 600.0)] {
        assert!(compact_galaxy(screen));
        let canvas = Rect::from_min_size(pos2(-350.0, -200.0), vec2((screen.x - 32.0).max(840.0), 520.0));
        let (area, bounds) = compact_map_rects(canvas);
        assert!(canvas.contains_rect(bounds) && bounds.contains_rect(area));
        assert!(area.width() >= 740.0 && area.height() >= 440.0, "scrolling retains room for large galaxy labels");
        assert!(page_width(screen.x, 760.0) + panel_chrome() <= screen.x - 20.0);
    }
}

/// A galaxy laid out: its planets, its names' sizes, its lanes, where names may go, its scale.
type Layout = (Vec<(Pos2, f32, f32)>, Vec<Vec2>, Vec<(Pos2, Pos2)>, Rect, f32);

/// Galaxies laid out as the page does at `screen`, names as long as TA's longest.
fn layouts(size: Size, screen: Vec2) -> Vec<Layout> {
    let titles = ["Core Prime Industrial Area", "Sail Away", "Checker Ponds", "King of the Hill", "The Desert Triad", "Icy Bergs", "Plains and Passes", "Pillopeens"];
    let maps: Vec<MapEntry> = (0..40)
        .map(|i| MapEntry {
            name: format!("map{i}"),
            title: titles[i % titles.len()].into(),
            description: String::new(),
            planet: "Green Planet".into(),
            size: "10 x 10".into(),
            start_positions: 2 + i % 3,
            info: Default::default(),
        })
        .collect();
    let (_, view) = wide_layout(Rect::from_min_size(Pos2::ZERO, screen), 0.0);
    (0..100u64)
        .filter_map(|seed| Conquest::new(&maps, "ARM", Level::Standard, size, seed))
        .map(|c| {
            let planets = (0..c.systems.len()).map(|i| (view.at(&c, i), view.radius(&c, i), if i == c.capital { 6.0 * view.k } else { 0.0 })).collect();
            let sizes = c.systems.iter().map(|s| vec2(s.name.len() as f32 * 6.2 * view.k.sqrt(), 14.0 * view.k.sqrt())).collect();
            let lanes = c.lanes.iter().map(|&(a, b)| (view.at(&c, a), view.at(&c, b))).collect();
            (planets, sizes, lanes, view.bounds, view.k)
        })
        .collect()
}

#[test]
fn names_keep_clear_of_each_other_and_of_the_rings() {
    // (The largest galaxies on the smallest wide screen are crowded: a name or two in a hundred
    // galaxies may have to sit over a planet.)
    for screen in [vec2(1280.0, 720.0), vec2(1920.0, 1080.0)] {
        for (size, crowded) in [(Size::Small, 0), (Size::Medium, 0), (Size::Large, 2)] {
            let (mut worst, mut rings, mut astray) = (0, 0, 0);
            for (planets, sizes, lanes, bounds, k) in layouts(size, screen) {
                let labels = galaxy::place_labels(&planets, &sizes, &lanes, bounds, k);
                let overlaps = (0..labels.len()).flat_map(|i| (i + 1..labels.len()).map(move |j| (i, j))).filter(|&(i, j)| labels[i].intersects(labels[j])).count();
                worst = worst.max(overlaps);
                // (A name never under its own planet's selection ring, nor another's.)
                rings += labels.iter().enumerate().filter(|&(i, r)| planets.iter().enumerate().any(|(j, &(p, rad, _))| r.distance_to_pos(p) < rad + galaxy::SELECT_RING * k + 1.0 && (i == j || r.distance_to_pos(p) < rad))).count();
                // (Nor nearer another planet than its own, where it would read as that one's.)
                astray += labels.iter().enumerate().filter(|&(i, r)| planets.iter().enumerate().any(|(j, &(p, _, _))| j != i && r.distance_to_pos(p) + 2.0 < r.distance_to_pos(planets[i].0))).count();
            }
            println!("{screen:?} {size:?}: at most {worst} overlapping, {rings} under a ring, {astray} nearer another planet in 100 galaxies");
            assert_eq!(worst, 0, "{size:?}: names overlapping");
            assert!(rings <= crowded, "{size:?}: {rings} names under a ring");
            assert!(astray <= 3 + crowded * 5, "{size:?}: {astray} names nearer another planet than their own");
        }
    }
}

#[test]
fn lanes_stop_short_of_names() {
    let hole = Rect::from_min_max(pos2(40.0, -5.0), pos2(60.0, 5.0));
    let pieces = galaxy::lane_pieces(pos2(0.0, 0.0), pos2(100.0, 0.0), &[hole]);
    assert_eq!(pieces, vec![(0.0, 0.4), (0.6, 1.0)]);
    assert_eq!(galaxy::lane_pieces(pos2(0.0, 20.0), pos2(100.0, 20.0), &[hole]), vec![(0.0, 1.0)], "a lane clear of the name runs whole");
}

#[test]
fn territories_meet_half_way_and_keep_their_own_system() {
    let points = [pos2(0.0, 0.0), pos2(100.0, 0.0), pos2(50.0, 80.0), pos2(900.0, 900.0)];
    let cells = galaxy::territories(&points, &[90.0; 4]);
    for (i, cell) in cells.iter().enumerate() {
        assert!(cell.len() >= 3, "{i}: a territory");
        // Its own system inside it (a convex polygon: on the same side of every edge).
        let inside = (0..cell.len()).all(|v| {
            let (a, b) = (cell[v].0, cell[(v + 1) % cell.len()].0);
            (b - a).x * (points[i] - a).y - (b - a).y * (points[i] - a).x >= -0.01
        }) || (0..cell.len()).all(|v| {
            let (a, b) = (cell[v].0, cell[(v + 1) % cell.len()].0);
            (b - a).x * (points[i] - a).y - (b - a).y * (points[i] - a).x <= 0.01
        });
        assert!(inside, "{i}: its system inside its territory");
        // Every corner no further than its reach, and no nearer another system than its own.
        for &(p, _) in cell {
            assert!(p.distance(points[i]) <= 90.01);
            assert!(points.iter().enumerate().all(|(j, &q)| j == i || p.distance(points[i]) <= p.distance(q) + 0.01), "{i}: a corner nearer another system");
        }
    }
    // The first two meet on the line half way between them, each edge naming the other.
    assert!(cells[0].iter().any(|&(p, across)| across == Some(1) && (p.x - 50.0).abs() < 0.01));
    assert!(cells[1].iter().any(|&(p, across)| across == Some(0) && (p.x - 50.0).abs() < 0.01));
    // The far one meets nobody: its rim only.
    assert!(cells[3].iter().all(|&(_, across)| across.is_none()));
}

#[test]
fn the_report_opens_once_the_battle_is_in() {
    let maps = entries(16, |_| 3);
    let mut c = Conquest::new(&maps, "ARM", Level::Standard, Size::Small, 5).unwrap();
    let mut page = ConquestPage { state: Some(c.clone()), debrief_after: Some(c.battles), ..Default::default() };
    let assets = test_assets(maps.clone());
    let ctx = egui::Context::default();
    ta_ui::theme::install(&ctx);
    let mut portraits = crate::encyclopaedia::Portraits::default();
    let mut frame = |page: &mut ConquestPage| {
        ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0))), ..Default::default() }, |ui| {
            page.draw(ui, &assets, &mut portraits);
        }).drop_without_applying_deltas();
    };
    frame(&mut page);
    assert!(!page.debrief, "no report before the result is in");
    let target = (0..c.systems.len()).find(|&i| c.attackable(i)).unwrap();
    assert!(c.record(crate::conquest::BattleResult { seed: c.seed, battle: c.battles, system: target, won: true, survivors: vec![], selected: vec![], undeployed: vec![], commander_xp: None, duration_ticks: 900 }));
    page.state = Some(c);
    frame(&mut page);
    assert!(page.debrief && page.debrief_after.is_none(), "the report opens when the result is in, once");
}
