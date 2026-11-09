use super::*;
use std::sync::Arc;

fn assets() -> Assets {
    let vfs = Arc::new(ta_vfs::Vfs::new());
    let data = Arc::new(ta_data::GameData::load(&vfs));
    let types = Arc::new(ta_sim::UnitTypes::compile(&data, &|_| None));
    let compatibility = ta_data::Compatibility::load(&vfs);
    let maps = ["First map", "Second map"].map(|name| crate::setup::MapEntry {
        name: name.into(), title: name.into(), description: String::new(), planet: String::new(), size: String::new(),
        start_positions: 2, info: Default::default(),
    }).to_vec();
    Assets { vfs, data, types, maps: maps.clone(), selectable_maps: maps, campaigns: Vec::new(), data_hash: compatibility.global(), compatibility, mods: Default::default() }
}

fn seat(kind: SlotKind, color: u8) -> Slot {
    Slot { kind, side: 0, color, team: color }
}

fn colours(seats: &[Slot]) -> Vec<u8> {
    seats.iter().map(|s| s.color).collect()
}

/// The player clicked seat 2's colour on to white (seat 3, closed, had it), then opened seat 3
/// from its drop-down: it kept white, and two players of one colour went into the battle.
#[test]
fn a_seat_opened_takes_a_colour_nobody_playing_has() {
    let mut seats = vec![seat(SlotKind::Human, 0), seat(SlotKind::AiNormal, 2), seat(SlotKind::AiNormal, 2), seat(SlotKind::Closed, 3)];
    settle_colours(&mut seats, Some(2));
    assert_eq!(colours(&seats)[..2], [0, 2], "the seats already playing keep their colours");
    assert!(![0, 2].contains(&seats[2].color), "the seat opened has a colour of its own: {:?}", colours(&seats));
    // Opened before the seat that took its colour: it is still the one that moves.
    let mut seats = vec![seat(SlotKind::Human, 0), seat(SlotKind::AiEasy, 2), seat(SlotKind::AiHard, 2)];
    settle_colours(&mut seats, Some(1));
    assert_eq!((seats[0].color, seats[2].color), (0, 2));
    assert!(![0, 2].contains(&seats[1].color));
}

/// Seats sharing a colour otherwise (a map with more starts bringing a seat back into view, a
/// skirmish kept from before): the later one moves; closed seats are no players.
#[test]
fn players_in_view_never_share_a_colour() {
    let mut seats = vec![seat(SlotKind::Human, 0), seat(SlotKind::AiNormal, 2), seat(SlotKind::Closed, 2), seat(SlotKind::AiNormal, 2), seat(SlotKind::AiHard, 5)];
    settle_colours(&mut seats, None);
    assert_eq!((seats[0].color, seats[1].color, seats[2].color, seats[4].color), (0, 2, 2, 5), "the closed seat is no player: left alone");
    assert!(![0, 2, 5].contains(&seats[3].color), "{:?}", colours(&seats));
    // Ten players all of one colour: ten colours.
    let mut seats: Vec<Slot> = (0..10).map(|i| seat(if i == 0 { SlotKind::Human } else { SlotKind::AiNormal }, 7)).collect();
    settle_colours(&mut seats, None);
    let mut all = colours(&seats);
    all.sort_unstable();
    assert_eq!(all, (0..10).collect::<Vec<u8>>());
    // Already apart: nothing moves.
    let mut seats = vec![seat(SlotKind::Human, 4), seat(SlotKind::AiNormal, 1), seat(SlotKind::AiEasy, 9)];
    settle_colours(&mut seats, Some(2));
    assert_eq!(colours(&seats), [4, 1, 9]);
}

#[test]
fn skirmish_settings_are_saved_and_restored_for_the_pinned_profile() {
    const CHILD_ROOT: &str = "TA_SKIRMISH_PROFILE_TEST_ROOT";
    let Some(root) = std::env::var_os(CHILD_ROOT).map(std::path::PathBuf::from) else {
        // Profile pinning is process-global. Isolate it from parallel game tests.
        let root = std::env::temp_dir().join(format!("ta-skirmish-profiles-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "menu::tests::skirmish_settings_are_saved_and_restored_for_the_pinned_profile", "--nocapture"])
            .env(CHILD_ROOT, &root).output().unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert!(result.status.success(), "{}\n{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
        return;
    };
    let store = ta_config::profiles::ProfileStore::open(root.clone()).unwrap();
    let first = store.create("First", ta_config::profiles::Allegiance::Arm, None, &Default::default()).unwrap();
    let second = store.create("Second", ta_config::profiles::Allegiance::Core, None, &Default::default()).unwrap();
    let assets = assets();
    ta_config::profiles::pin(root.clone(), &first.id).unwrap();
    let mut menu = Menu::new(&assets);
    menu.rules.start_metal = 5000;
    menu.slots[1].kind = SlotKind::AiHard;
    menu.keep_skirmish(&assets, "First map");
    assert!(store.data_dir(&first.id).unwrap().join(KEPT_SKIRMISH).is_file());
    assert!(!root.join(KEPT_SKIRMISH).exists(), "skirmish preferences must not be shared across profiles");

    ta_config::profiles::pin(root.clone(), &second.id).unwrap();
    let mut menu = Menu::new(&assets);
    menu.restore_skirmish(&assets);
    assert_eq!(menu.rules.start_metal, 1000, "a fresh profile must not inherit another player's setup");
    menu.rules.start_metal = 10000;
    menu.keep_skirmish(&assets, "Second map");

    ta_config::profiles::pin(root, &first.id).unwrap();
    let mut menu = Menu::new(&assets);
    menu.rules.unit_limit = 2500;
    menu.restore_skirmish(&assets);
    assert_eq!((menu.selected_map, menu.rules.start_metal, menu.rules.unit_limit), (0, 5000, 2500));
    assert_eq!(menu.slots[1].kind, SlotKind::AiHard);
}

/// A scenario of the settings folder that can't be played here (a map or unit the game lacks) is
/// listed greyed, and can still be chosen: its page says why, under it START SCENARIO stays dark.
/// (Its row took no clicks: the why was never to be seen.)
#[test]
fn a_scenario_that_cannot_be_played_can_be_chosen_to_see_why() {
    let assets = assets();
    let mut menu = Menu::new(&assets);
    let entry = |id: &str, problem: Option<&str>| {
        let source = format!("id = '{id}'\ntitle = '{id}'\ntagline = 'A line.'\nbriefing = 'Words.'\nmap = 'First map'\nkind = 'battle'\nvictory = [{{ destroy = 'commander' }}]\n[player]\nside = 'ARM'\nstart = 'centre'\n[[enemy]]\nside = 'CORE'\nstart = 'north'\n");
        crate::scenario::Entry { scenario: crate::scenario::Scenario::parse(&source).unwrap(), source, custom: true, problem: problem.map(str::to_string) }
    };
    menu.scenarios = vec![entry("playable", None), entry("unplayable", Some("Needs the map Elsewhere"))];
    menu.page = Page::Scenarios;
    let ctx = egui::Context::default();
    ta_ui::theme::install(&ctx);
    ctx.enable_accesskit();
    let progress = ta_config::CampaignProgress::default();
    let mut time = 0.0;
    let mut frame = |menu: &mut Menu, events: Vec<egui::Event>| {
        time += 1.0 / 30.0;
        let input = egui::RawInput { time: Some(time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            menu.scenarios_page(ui, &assets, &progress);
        });
        let update = out.platform_output.accesskit_update.take();
        out.drop_without_applying_deltas();
        update
    };
    // (The page in place: its entrance over.)
    let mut tree = None;
    for _ in 0..45 {
        tree = frame(&mut menu, Vec::new()).or(tree);
    }
    let row = tree.unwrap().nodes.iter().find(|(_, node)| node.label() == Some("unplayable")).map(|(id, _)| *id).expect("the greyed scenario's row");
    let click = egui::accesskit::ActionRequest { action: egui::accesskit::Action::Click, target_tree: egui::accesskit::TreeId::ROOT, target_node: row, data: None };
    frame(&mut menu, vec![egui::Event::AccessKitActionRequest(click)]);
    frame(&mut menu, Vec::new());
    assert_eq!(menu.scenario_selected, 1, "the scenario that can't be played is chosen, its page shows why");
}
