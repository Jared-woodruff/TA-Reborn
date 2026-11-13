//! Benchmark page: the game's benchmark run from here (eight scenes on TA Reborn's own worlds
//! and two tests, about five minutes, or its quick run, at the player's settings) and its report:
//! the scores, the settings it recommends (applied with a click), each scene with its picture,
//! the run in time, where the graphics card's time goes, what each effect costs, the frames'
//! times, the settings test, this computer, and the runs before.

use ta_ui::tooltip::TooltipExt as _;
use egui::{Align, Align2, Color32, CornerRadius, Layout, Mesh, Rect, RichText, Sense, Shape, Stroke, Ui, UiBuilder, pos2, vec2};
use ta_config::Preset;
use ta_config::benchmark::{Effect, FRAME_BUCKETS, LEVELS, Mode, Note, Recommendation, Report, Sample, Stats, TARGET_FPS, Tier};
use ta_ui::i18n::{n, t, tf};
use ta_ui::metal;

use super::{caption_width, field_with, page_header, wrapped};
use crate::app::{Action, LauncherApp};
use crate::theme::*;
use crate::widgets::{self, ActionButton, Mark, caption};

/// The series' colours: the graphics card's, the processor's, the memory's, the frame rate's.
const GPU: Color32 = SKY;
const CPU: Color32 = GREEN;
const RAM: Color32 = CREAM;
const FPS: Color32 = AMBER;
/// Thickness of the hero's pipe frame.
const HERO_PIPE: f32 = 4.0;

/// A scene's name (the worlds' are names: not translated) and its moment (translated), as the
/// report's benchmark had them (`first`: the first benchmark's).
fn part_name(id: &str, first: bool) -> (&'static str, &'static str) {
    match ta_ui::benchmark::scene(id, first) {
        Some(scene) => (scene.title, t(scene.subtitle)),
        None => (ta_ui::benchmark::part_title(id), ""),
    }
}

/// A part's name as shown (the tests' translated).
fn part_title(id: &str) -> &'static str {
    ta_ui::benchmark::part_title(id)
}

/// A renderer's pass: its name as shown and its colour (any other: the time between them).
fn pass_style(pass: &str) -> (&'static str, Color32) {
    match pass {
        "shadow" => (n("Shadows"), Color32::from_rgb(120, 132, 150)),
        "ray tracing" => (n("Ray tracing"), Color32::from_rgb(176, 120, 226)),
        "reflection" => (n("Reflections"), Color32::from_rgb(86, 196, 214)),
        "scene" => (n("The scene"), AMBER),
        "particles" => (n("Particles"), Color32::from_rgb(232, 128, 64)),
        "water and effects" => (n("Water and effects"), SKY),
        "bloom" => (n("Bloom"), CREAM),
        "composite" => (n("Finishing"), Color32::from_rgb(140, 170, 120)),
        _ => (n("Between passes"), Color32::from_rgb(70, 68, 62)),
    }
}

/// The passes in the order the renderer draws them.
const PASS_ORDER: [&str; 8] = ["ray tracing", "shadow", "reflection", "scene", "particles", "water and effects", "bloom", "composite"];

fn tier_color(tier: Tier) -> Color32 {
    match tier {
        Tier::Peewee => STEEL_DIM,
        Tier::Hammer => STEEL,
        Tier::Bulldog => GREEN,
        Tier::Goliath => SKY,
        Tier::Commander => AMBER,
        Tier::Krogoth => RED,
    }
}

/// Milliseconds (`text`: the figures) with their unit, as the language writes it.
fn ms(text: String) -> String {
    tf("{ms} ms", &[("ms", &text)])
}

/// Megabytes as gigabytes, as the language writes them.
fn gb(mb: u64) -> String {
    tf("{gb} GB", &[("gb", &format!("{:.0}", mb as f32 / 1024.0))])
}

/// What the computer told, or a dash where it told nothing (outside Windows, some of it).
fn known(text: &str) -> String {
    if text.is_empty() { "—".to_string() } else { text.to_string() }
}

/// Seconds as minutes and seconds ("1:05").
fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0).round() as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A level's name in a sentence ("High").
fn level_name(level: Preset) -> String {
    t(level.label()).chars().enumerate().map(|(i, c)| if i == 0 { c } else { c.to_lowercase().next().unwrap_or(c) }).collect()
}

/// A round step for an axis about `raw` long.
fn nice_step(raw: f32) -> f32 {
    let power = 10f32.powf(raw.max(1e-3).log10().floor());
    let m = raw / power;
    let step = if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 2.5 {
        2.5
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    };
    step * power
}

/// An axis from 0 that holds `max`: its top and its step.
fn axis(max: f32) -> (f32, f32) {
    let step = nice_step(max.max(1e-3) / 4.0);
    ((max / step).ceil().max(1.0) * step, step)
}

/// Every visible work series must fit, including ticks longer than the average
/// frame's CPU work. Untimed GPU samples are not drawn and must not flatten it.
fn work_axis(samples: &[Sample], gpu_timed: bool) -> (f32, f32) {
    let most = samples.iter().map(|s| s.cpu_ms.max(s.tick_ms).max(if gpu_timed { s.gpu_ms } else { 0.0 }))
        .fold(1.0f32, f32::max);
    axis(most * 1.05)
}

impl LauncherApp {
    pub(crate) fn benchmark_page(&mut self, ui: &mut Ui) {
        page_header(ui, "BENCHMARK", "Eight scenes on TA Reborn's own worlds and two tests, measured: scores for this computer, the settings that suit it, and what each effect costs.");
        self.benchmark_hero(ui);
        ui.add_space(14.0);
        self.benchmark_stopped(ui);
        let Some(report) = self.benchmarks.report().cloned() else {
            self.worlds_strip(ui);
            ui.add_space(14.0);
            about_cards(ui);
            return;
        };
        self.benchmark_report_actions(ui, &report);
        ui.add_space(14.0);
        let scores = report.scores();
        let s = &report.system;
        let gpu_name = if s.vram_mb > 0 { format!("{} · {}", s.gpu, gb(s.vram_mb)) } else { s.gpu.clone() };
        let cpu_name = if s.threads > 0 { tf("{cpu} · {threads} threads", &[("cpu", &known(&short_cpu(&s.cpu))), ("threads", &s.threads)]) } else { known(&short_cpu(&s.cpu)) };
        let ram_name = if s.ram_mb > 0 { gb(s.ram_mb) } else { known("") };
        let big = report.largest();
        let r = &report.reference;
        let drawn = ((report.width as f32 * report.reference_scale).round(), (report.height as f32 * report.reference_scale).round());
        let gpu_lines = [
            if report.gpu_timed {
                tf("{ms} ms a frame at High, {width} × {height} drawn", &[("ms", &format!("{:.2}", r.gpu_ms)), ("width", &drawn.0), ("height", &drawn.1)])
            } else {
                tf("{ms} ms a frame at High (not timed by the card)", &[("ms", &format!("{:.2}", r.frame_ms))])
            },
            tf("{fps} frames a second in the scenes", &[("fps", &format!("{:.0}", report.overall().fps))]),
        ];
        let cpu_lines = [
            tf("{ms} ms a simulation tick for {units} units", &[("ms", &format!("{:.2}", big.tick_ms)), ("units", &big.units)]),
            tf("{ms} ms of other work a frame", &[("ms", &format!("{:.2}", r.prep_ms()))]),
        ];
        let m = &report.memory;
        let ram_lines = [
            tf("Copies at {gbs} GB/s", &[("gbs", &format!("{:.1}", m.bandwidth_gbs))]),
            tf("Reads at random in {ns} ns", &[("ns", &format!("{:.0}", m.latency_ns))]),
        ];
        if ui.available_width() >= 760.0 {
            widgets::columns(ui, 3, |columns| {
                score_card(&mut columns[0], n("GRAPHICS CARD"), scores.gpu, GPU, &gpu_name, &gpu_lines);
                score_card(&mut columns[1], n("PROCESSOR"), scores.cpu, CPU, &cpu_name, &cpu_lines);
                score_card(&mut columns[2], n("MEMORY"), scores.ram, RAM, &ram_name, &ram_lines);
            });
        } else {
            for (title, score, color, name, lines) in [
                (n("GRAPHICS CARD"), scores.gpu, GPU, &gpu_name, &gpu_lines),
                (n("PROCESSOR"), scores.cpu, CPU, &cpu_name, &cpu_lines),
                (n("MEMORY"), scores.ram, RAM, &ram_name, &ram_lines),
            ] {
                score_card(ui, title, score, color, name, lines);
                ui.add_space(space::S);
            }
        }
        ui.add_space(14.0);
        let rec = crate::benchmarks::recommendation(&report);
        if let Some(measured) = &rec {
            self.recommendation_card(ui, measured);
            ui.add_space(14.0);
        }
        if report.first() {
            widgets::card_accent(ui, Some(AMBER), |ui| {
                wrapped(ui, t("A report of the first benchmark (three battles): its scores stand as they were given. Run the benchmark again to see its eight scenes with their pictures, where the graphics card's time goes and what each effect costs."), TEXT);
            });
            ui.add_space(14.0);
        } else {
            self.scenes_gallery(ui, &report);
            ui.add_space(14.0);
        }
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "THE RUN IN TIME");
            timeline(ui, &report);
        });
        ui.add_space(14.0);
        widgets::card(ui, |ui| {
            widgets::card_title(ui, if report.first() { "EACH BATTLE" } else { "EACH SCENE" });
            battles_table(ui, &report);
        });
        ui.add_space(14.0);
        if !report.first() {
            widgets::card(ui, |ui| {
                widgets::card_title(ui, "WHERE THE GRAPHICS CARD'S TIME GOES");
                passes_chart(ui, &report);
            });
            ui.add_space(14.0);
            widgets::card(ui, |ui| {
                widgets::card_title(ui, "WHAT EACH EFFECT COSTS");
                effects_chart(ui, &report);
            });
            ui.add_space(14.0);
            widgets::card(ui, |ui| {
                widgets::card_title(ui, "FRAME TIMES");
                frame_times(ui, &report);
            });
            ui.add_space(14.0);
        }
        let yours = self.config.graphics.preset;
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "SETTINGS TEST");
            settings_test(ui, &report, rec.as_ref(), yours);
        });
        ui.add_space(14.0);
        if ui.available_width() >= 680.0 {
            widgets::columns(ui, 2, |columns| {
                system_card(&mut columns[0], &report);
                self.history_card(&mut columns[1]);
            });
        } else {
            system_card(ui, &report);
            ui.add_space(14.0);
            self.history_card(ui);
        }
    }

    /// The banner: the overall score and its class (or what the benchmark is, or that it is
    /// running), and the button that runs it.
    fn benchmark_hero(&mut self, ui: &mut Ui) {
        let running = self.benchmarks.running.is_some() && self.game.is_running();
        let report = self.benchmarks.report().cloned();
        let stacked = ui.available_width() < 620.0;
        let height = if stacked { if report.is_some() && !running { 332.0 } else { 320.0 } }
            else if report.is_some() && !running { 196.0 } else { 172.0 };
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let painter = ui.painter();
        painter.add(metal::plate(ui.ctx(), rect, PANEL));
        painter.add(metal::pipe_frame(rect, HERO_PIPE));
        let compact = rect.width() < 720.0;
        let inset = metal::pipe_inset(HERO_PIPE) + space::XL;
        let mut content = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(inset, inset - space::S))).layout(if stacked { Layout::top_down(Align::Min) } else { Layout::left_to_right(Align::Center) }));
        let ui = &mut content;
        let installed = self.detection.install().is_some();
        let busy = self.game_running();
        ui.vertical(|ui| {
            ui.set_max_width(if stacked { ui.available_width() } else { (ui.available_width() - if compact { 220.0 } else { 280.0 }).max(1.0) });
            // A lamp and what is going on.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (lamp, _) = ui.allocate_exact_size(vec2(12.0, 16.0), Sense::hover());
                let pulsing = running && !ta_ui::motion::still(ui.ctx());
                let on = if pulsing { (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.3 + 0.7 } else { 1.0 };
                if pulsing {
                    ta_ui::motion::next_frame(ui.ctx(), std::time::Duration::from_millis(33));
                }
                let (status, color) = match (&report, running) {
                    (_, true) => {
                        let what = if self.benchmarks.running_mode == Mode::Quick { t("QUICK RUN") } else { t("BENCHMARK RUNNING") };
                        (tf("{what} · {time}", &[("what", &what), ("time", &crate::battles::length(self.game.running_for().map_or(0, |d| d.as_secs())))]), AMBER)
                    }
                    (Some(r), false) => {
                        let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(r.when);
                        let run = if r.mode == Mode::Quick { t("QUICK RUN") } else { t("FULL RUN") };
                        (tf("SCORED {when} · {run} · v{version} · {width} × {height}", &[("when", &ta_ui::i18n::upper(&crate::battles::age(when))), ("run", &run), ("version", &r.version), ("width", &r.width), ("height", &r.height)]), GREEN)
                    }
                    (None, false) => (t("NOT RUN YET").to_string(), STEEL),
                };
                if running {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
                }
                ui.painter().add(metal::lamp(lamp.center(), 4.0, color, on));
                ui.label(RichText::new(status).font(strong(12.5)).color(color).extra_letter_spacing(ta_ui::i18n::tracking(2.2)));
            });
            match (&report, running) {
                (Some(r), false) => {
                    let overall = r.scores().overall;
                    let tier = Tier::of(overall);
                    // (Against the run before it, at the same version and size: a change of
                    // driver, settings or computer shows.)
                    let before = self.benchmarks.runs.get(self.benchmarks.shown + 1).map(|(_, b)| b).filter(|b| b.version == r.version && b.content == r.content && (b.width, b.height) == (r.width, r.height));
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 14.0;
                        let score_size = if ui.available_width() < 300.0 { 42.0 } else { 58.0 };
                        metal::embossed_label(ui, RichText::new(overall.to_string()).font(display(score_size)), TEXT_BRIGHT).hover_tip(t("The reference computer scores 10,000"));
                        ui.vertical(|ui| {
                            ui.add_space(12.0);
                            ui.label(caption("CLASS"));
                            metal::embossed_label(ui, RichText::new(tier.name()).font(display(26.0)).extra_letter_spacing(ta_ui::i18n::tracking(1.6)), tier_color(tier));
                        });
                        if let Some(b) = before {
                            ui.vertical(|ui| {
                                ui.add_space(12.0);
                                let then = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(b.when);
                                ui.label(caption(&ta_ui::i18n::upper(&tf("SINCE THE RUN {when}", &[("when", &crate::battles::age(then))]))));
                                let change = (overall as f32 / b.scores().overall.max(1) as f32 - 1.0) * 100.0;
                                let (text, color) = if change.abs() < 1.0 { (t("much the same").to_string(), STEEL) }
                                    else if change > 0.0 { (format!("▲ {change:.0}%"), GREEN) } else { (format!("▼ {:.0}%", -change), AMBER) };
                                ui.label(RichText::new(text).font(display(22.0)).color(color)).hover_tip(tf("The run before scored {score}", &[("score", &b.scores().overall)]));
                            });
                        }
                    });
                    tier_ladder(ui, overall);
                }
                (_, true) => {
                    widgets::fitted_title(ui, t("MEASURING"), 40.0, TEXT_BRIGHT);
                    let how = if self.benchmarks.running_mode == Mode::Quick {
                        t("The quick run plays automatically in the game's window, about a minute and a half. Press Esc in the game to cancel.")
                    } else {
                        t("The scenes play automatically in the game's window, about five minutes. Press Esc in the game to cancel.")
                    };
                    wrapped(ui, how, TEXT);
                }
                (None, false) => {
                    widgets::fitted_title(ui, t("HOW FAST IS THIS COMPUTER?"), 36.0, TEXT_BRIGHT);
                    wrapped(ui, t("Eight staged scenes on eight worlds, a test of each graphics level and of each effect, about five minutes at your settings. Close other programs first for a fair score."), TEXT);
                }
            }
        });
        if stacked { ui.add_space(space::S); }
        ui.with_layout(Layout::top_down(if stacked { Align::Min } else { Align::Max }), |ui| {
            // (The two buttons: 52 and 34 high, 4 apart.)
            if !stacked {
                ui.add_space(((ui.available_height() - 90.0) * 0.5).max(0.0));
            }
            let why_not = |button: egui::Response, ok: &str| if !installed {
                button.disabled_tip(t("The original game is needed: see the Play page."))
            } else if busy {
                button.disabled_tip(t("The game is running. Close it to run the benchmark."))
            } else {
                button.hover_tip(t(ok))
            };
            let label = if report.is_some() { n("RUN AGAIN") } else { n("RUN BENCHMARK") };
            // (Alike, and as wide as the longer label needs: a translation never spills out.)
            let need = |ui: &Ui, text: &str, size: f32| widgets::galley(ui, RichText::new(t(text)).font(strong(size)).extra_letter_spacing(ta_ui::i18n::tracking(0.8))).size().x + 30.0;
            let width = (if compact { 180.0f32 } else { 220.0 }).max(need(ui, label, 15.0)).max(need(ui, "QUICK RUN", 13.5));
            let full = why_not(ui.add_enabled(installed && !busy, ActionButton::primary(label).size(width, 52.0)), n("Every scene and both tests: about five minutes; Esc in the game stops it"));
            if full.clicked() {
                ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::BigButton);
                self.push(Action::Benchmark);
            }
            ui.add_space(space::XS);
            let quick = why_not(ui.add_enabled(installed && !busy, ActionButton::secondary("QUICK RUN").size(width, 34.0)), n("The scores alone: about a minute and a half"));
            if quick.clicked() {
                ta_ui::sound::play(ui.ctx(), ta_ui::sound::UiSound::BigButton);
                self.push(Action::QuickBenchmark);
            }
        });
    }

    /// Sharing always uses the selected run, including when an older report is being viewed.
    /// The run's notes (its skins, its accessibility settings) at the left, what can be done with
    /// it at the right (on a narrow page, under them).
    fn benchmark_report_actions(&mut self, ui: &mut Ui, report: &Report) {
        let path = self.benchmarks.report_path().map(std::path::Path::to_path_buf);
        let mut open = None;
        let mut copy = false;
        let buttons = |ui: &mut Ui, open: &mut Option<std::path::PathBuf>, copy: &mut bool| {
            if ui.add(ActionButton::secondary("COPY RESULTS")).hover_tip(t("Copy this run's scores, frame rates and computer details to share")).clicked() {
                *copy = true;
            }
            if let Some(path) = &path
                && ui.add(ActionButton::secondary("OPEN REPORT FILE")).hover_tip(path.display().to_string()).clicked()
            {
                *open = Some(path.clone());
            }
        };
        let notes = |ui: &mut Ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(skin_summary(report)).color(STEEL));
                if report.reduced_motion || report.reduced_flashes {
                    ui.add(egui::Label::new(RichText::new(accessibility_summary(report)).color(AMBER)).wrap());
                }
            });
        };
        if ui.available_width() >= 760.0 {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(vec2(ui.available_width() * 0.55, 0.0), Layout::top_down(Align::Min), notes);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| buttons(ui, &mut open, &mut copy));
            });
        } else {
            notes(ui);
            ui.horizontal_wrapped(|ui| buttons(ui, &mut open, &mut copy));
        }
        if let Some(path) = open {
            self.push(Action::OpenPath(path));
        }
        if copy {
            self.push(Action::Copy(report_summary(report)));
        }
    }

    /// The last run ended without a report: why.
    fn benchmark_stopped(&mut self, ui: &mut Ui) {
        let Some(why) = self.benchmarks.stopped.clone() else { return };
        let mut dismiss = false;
        widgets::card_accent(ui, Some(AMBER), |ui| {
            ui.horizontal(|ui| {
                widgets::status_mark(ui, Mark::Absent);
                ui.label(RichText::new(t("NO REPORT")).font(strong(13.0)).color(AMBER).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(18.0);
                    dismiss = widgets::icon_button(ui, widgets::Glyph::Close, true).hover_tip(t("Dismiss")).clicked();
                });
            });
            wrapped(ui, &why, TEXT);
        });
        ui.add_space(14.0);
        if dismiss {
            self.benchmarks.stopped = None;
        }
    }

    /// The settings the run recommends, against the player's, and the button that applies them.
    fn recommendation_card(&mut self, ui: &mut Ui, rec: &Recommendation) {
        let g = self.config.graphics.clone();
        let limit = self.config.gameplay.unit_limit;
        let applied = rec.applied(&g, limit);
        let mut apply = false;
        widgets::card_accent(ui, Some(GREEN), |ui| {
            ui.horizontal(|ui| {
                widgets::status_mark(ui, Mark::Ok);
                ui.label(RichText::new(t("RECOMMENDED SETTINGS")).font(strong(13.0)).color(GREEN).extra_letter_spacing(ta_ui::i18n::tracking(1.4)));
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                metal::embossed_label(ui, RichText::new(t(rec.level.label())).font(display(30.0)).extra_letter_spacing(ta_ui::i18n::tracking(1.4)), TEXT_BRIGHT);
                let figures = [("fps", format!("{:.0}", rec.expected_fps)), ("target", format!("{:.0}", rec.target_fps))];
                let figures: Vec<(&str, &dyn std::fmt::Display)> = figures.iter().map(|(k, v)| (*k, v as &dyn std::fmt::Display)).collect();
                let held = if rec.notes.contains(&Note::BelowTarget) {
                    tf("Its slowest frames came at {fps} a second in the test, short of {target}: the picture is drawn at a lower resolution", &figures)
                } else if rec.expected_fps.round() >= rec.target_fps.round() {
                    tf("Its slowest frames held {fps} a second in the test, above the {target} it aims for", &figures)
                } else {
                    tf("Its slowest frames held {fps} a second in the test, near enough the {target} it aims for to stay smooth", &figures)
                };
                ui.label(RichText::new(held).color(TEXT));
            });
            ui.add_space(space::S);
            let on_off = |on: bool| if on { t("On") } else { t("Off") };
            let samples = |m: u32| if m <= 1 { t("Off").to_string() } else { format!("{m}×") };
            let rows: [(&str, String, String); 5] = [
                (n("GRAPHICS LEVEL"), level_name(rec.level), level_name(g.preset)),
                (n("ANTIALIASING"), samples(rec.msaa), samples(g.msaa)),
                (n("RENDER SCALE"), format!("{:.0}%", rec.render_scale * 100.0), format!("{:.0}%", g.render_scale * 100.0)),
                (n("VERTICAL SYNC"), on_off(rec.vsync).to_string(), on_off(g.vsync).to_string()),
                (n("UNIT LIMIT"), tf("{n} a player", &[("n", &limit.min(rec.unit_limit))]), tf("{n} a player", &[("n", &limit)])),
            ];
            let width = caption_width(ui, &rows.iter().map(|r| r.0).collect::<Vec<_>>());
            for (label, wanted, now) in rows {
                field_with(ui, label, width, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&wanted).font(strong(13.5)).color(TEXT_BRIGHT));
                        if wanted != now {
                            ui.label(RichText::new(tf("now {value}", &[("value", &now)])).color(STEEL));
                        }
                    });
                });
            }
            wrapped(ui, &tf("The processor keeps up with battles of {n} units a player (two players at it).", &[("n", &rec.unit_limit)]), STEEL);
            for note in &rec.notes {
                let text = match note {
                    Note::GpuBound => n("The graphics card sets the pace: a lower level gains the most frames."),
                    Note::CpuBound => n("The processor sets the pace in the largest battles: a lower unit limit keeps them smooth."),
                    Note::Balanced => n("The graphics card and the processor keep pace with each other."),
                    Note::BelowTarget => n("Even Low falls short of 60 frames a second here: the picture is drawn at a lower resolution."),
                    Note::Memory => n("Memory is short: close other programs while you play."),
                    Note::Vsync => n("The game runs faster than the screen refreshes: vertical sync keeps the picture steady at no cost."),
                    Note::Untimed => n("This graphics card does not time its own work: its score is read from the frames' times."),
                    Note::LeftOut => n("Units of an expansion not installed were left out of the battles: the scores do not compare with other computers'."),
                };
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (dot, _) = ui.allocate_exact_size(vec2(8.0, 18.0), Sense::hover());
                    ui.painter().add(metal::lamp(dot.center(), 2.6, if matches!(note, Note::BelowTarget | Note::Memory | Note::LeftOut) { AMBER } else { GREEN }, 1.0));
                    ui.add(egui::Label::new(RichText::new(t(text)).color(TEXT)).wrap());
                });
            }
            ui.add_space(space::S);
            ui.horizontal(|ui| {
                let label = if applied { n("✔ APPLIED") } else { n("APPLY THESE SETTINGS") };
                let button = ui.add_enabled(!applied, ActionButton::primary(label));
                apply = button.hover_tip(t("The next game starts with them; they stay yours to change in Settings")).clicked();
            });
        });
        if apply {
            self.push(Action::ApplyRecommendation);
        }
    }

    /// Each scene of the run shown: its picture, its world and moment, its frame rate and
    /// slowest frames, what it shows off; the rest of its figures a hover away.
    fn scenes_gallery(&mut self, ui: &mut Ui, report: &Report) {
        let scenes: Vec<ta_config::benchmark::Part> = report.scenes().cloned().collect();
        let pictures: Vec<Option<egui::TextureHandle>> = scenes.iter().map(|p| p.picture.as_deref().and_then(|name| self.benchmarks.picture(ui.ctx(), name))).collect();
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "THE SCENES");
            if report.mode == Mode::Quick {
                wrapped(ui, t("A quick run plays the storm's first seconds and the twenty thousand: a full run plays all eight scenes."), STEEL_DIM);
                ui.add_space(space::S);
            }
            let columns = ((ui.available_width() + 12.0) / 260.0).floor().clamp(1.0, 4.0) as usize;
            let gap = 12.0;
            let width = (ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32;
            for (row, chunk) in scenes.chunks(columns).enumerate() {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (k, part) in chunk.iter().enumerate() {
                        let picture = pictures[row * columns + k].as_ref();
                        ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
                            ui.set_width(width);
                            scene_card(ui, report, part, picture);
                        });
                    }
                });
                ui.add_space(gap);
            }
        });
    }

    /// The runs before (newest first): each a click away.
    fn history_card(&mut self, ui: &mut Ui) {
        let mut pick = None;
        let dir = self.benchmarks.dir.clone();
        widgets::card(ui, |ui| {
            widgets::card_title(ui, "EARLIER RUNS");
            for (i, (_, r)) in self.benchmarks.runs.iter().enumerate().take(10) {
                let shown = i == self.benchmarks.shown;
                let overall = r.scores().overall;
                let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(r.when);
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                let hot = response.hovered();
                let painter = ui.painter();
                if shown || hot {
                    painter.rect_filled(rect, CornerRadius::same(2), if shown { PANEL_HI } else { Color32::from_white_alpha(8) });
                }
                if shown {
                    painter.rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), CornerRadius::ZERO, AMBER);
                }
                // Two lines leave the age and resolution readable in the narrow half-page
                // card. Ellipsize the left text before the measured score/class on the right.
                let (top, bottom) = (rect.min.y + 13.0, rect.min.y + 33.0);
                let tier = Tier::of(overall);
                let score_rect = painter.text(pos2(rect.max.x - 10.0, top), Align2::RIGHT_CENTER, overall.to_string(), strong(15.0), TEXT_BRIGHT);
                let class_rect = painter.text(pos2(rect.max.x - 10.0, bottom), Align2::RIGHT_CENTER, tier.name(), strong(11.0), tier_color(tier));
                let left_text = |text: String, y, font, color, right: f32| {
                    let x = rect.min.x + 12.0;
                    let mut job = egui::text::LayoutJob::simple(text, font, color, (right - x - 12.0).max(0.0));
                    job.wrap.max_rows = 1;
                    job.wrap.break_anywhere = true;
                    let galley = painter.layout_job(job);
                    painter.galley(pos2(x, y - galley.size().y / 2.0), galley, color);
                };
                left_text(crate::battles::age(when), top, body(13.0), if shown { TEXT_BRIGHT } else { TEXT }, score_rect.min.x);
                let run = if r.first() { t("first benchmark") } else if r.mode == Mode::Quick { t("quick run") } else { t("full run") };
                let detail = format!("{run} · {} × {} · v{}", r.width, r.height, r.version);
                left_text(detail.clone(), bottom, body(12.0), STEEL, class_rect.min.x);
                if response.hover_tip(detail).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    pick = Some(i);
                }
            }
            ui.add_space(space::S);
            if ui.link(RichText::new(t("OPEN THE REPORTS' FOLDER")).font(strong(11.5)).extra_letter_spacing(ta_ui::i18n::tracking(1.0))).hover_tip(dir.display().to_string()).clicked() {
                self.push(Action::OpenFolder(dir.clone()));
            }
        });
        if let Some(i) = pick {
            self.benchmarks.shown = i;
        }
    }
}

/// A compact, readable report for pasting into a message. Use the selected report's settings
/// and hardware, never today's launcher configuration, so earlier runs remain comparable.
fn report_summary(report: &Report) -> String {
    let scores = report.scores();
    let overall = report.overall();
    let antialiasing = if report.tested_msaa <= 1 { t("Off").to_string() } else { format!("{}×", report.tested_msaa) };
    let frame_rates = |stats: Stats| tf("{fps} FPS average · 1% low {low1} · 0.1% low {low01}", &[("fps", &format!("{:.1}", stats.fps)), ("low1", &format!("{:.1}", stats.low1)), ("low01", &format!("{:.1}", stats.low01))]);
    let run = if report.mode == Mode::Quick { t("quick run") } else { t("full run") };
    let mut lines = vec![
        tf("TA Reborn benchmark · {run} · v{version}", &[("run", &run), ("version", &report.version)]),
        tf("{width} × {height} · {level} · render scale {scale}% · antialiasing {samples}", &[("width", &report.width), ("height", &report.height), ("level", &t(report.graphics.preset.label())), ("scale", &format!("{:.0}", report.graphics.render_scale * 100.0)), ("samples", &antialiasing)]),
        skin_summary(report),
        tf("Overall score: {score} ({class})", &[("score", &scores.overall), ("class", &Tier::of(scores.overall).name())]),
        tf("Scores — graphics card: {gpu} · processor: {cpu} · memory: {ram}", &[("gpu", &scores.gpu), ("cpu", &scores.cpu), ("ram", &scores.ram)]),
        tf("Graphics card: {gpu}", &[("gpu", &known(&report.system.gpu))]),
        tf("Processor: {cpu} · memory: {ram}", &[("cpu", &known(&report.system.cpu)), ("ram", &if report.system.ram_mb > 0 { gb(report.system.ram_mb) } else { known("") })]),
        format!("{}: {}", if report.first() { t("ALL THREE") } else { t("ALL SCENES") }, frame_rates(overall)),
    ];
    if report.reduced_motion || report.reduced_flashes {
        lines.push(accessibility_summary(report));
    }
    for part in report.scenes() {
        lines.push(format!("{}: {}", part_name(&part.id, report.first()).0, frame_rates(part.stats)));
    }
    for (effect, ms, share) in report.effect_costs() {
        lines.push(tf("{effect} costs {ms} ms ({share}%) at High", &[("effect", &t(ta_ui::benchmark::effect_label(effect))), ("ms", &format!("{:.1}", ms.max(0.0))), ("share", &format!("{:.0}", share.max(0.0) * 100.0))]));
    }
    if !report.gpu_timed {
        lines.push(t("This graphics card does not time its own work: its score is read from the frames' times.").to_string());
    }
    if !report.left_out.is_empty() {
        lines.push(tf("Units left out: {units}. Scores do not compare with complete runs.", &[("units", &report.left_out.join(", "))]));
    }
    lines.join("\n")
}

fn skin_summary(report: &Report) -> String {
    let name = |id: &str| super::skins::builtin_name(id).unwrap_or_else(|| id.to_string());
    let friendly = name(report.unit_skin());
    let enemy = name(report.enemy_unit_skin());
    let skin = if report.unit_skin() == report.enemy_unit_skin() {
        tf("Unit skin: {skin}", &[("skin", &friendly)])
    } else {
        tf("Unit skins: yours and allies {friendly} · enemies {enemy}", &[("friendly", &friendly), ("enemy", &enemy)])
    };
    if report.unit_skin() == "reborn" && report.enemy_unit_skin() == "reborn" {
        skin
    } else {
        format!("{skin}. {}", t("Compare scores with the same unit skin."))
    }
}

fn accessibility_summary(report: &Report) -> String {
    let settings = [(n("REDUCED MOTION"), report.reduced_motion), (n("REDUCED FLASHES"), report.reduced_flashes)]
        .into_iter().filter(|(_, enabled)| *enabled).map(|(label, _)| t(label).to_string()).collect::<Vec<_>>().join(" · ");
    format!("{settings}. {}", t("Accessibility effects change rendering work. Compare scores with the same accessibility settings."))
}

impl LauncherApp {
    /// Before the first run: the eight worlds its scenes are staged on, each drawn from the
    /// install's own art (TA Reborn's maps, in its palette), with its name and its moment; eight
    /// across, or four in a narrow window. Nothing without an install.
    fn worlds_strip(&mut self, ui: &mut Ui) {
        let root = self.detection.install().map(|install| install.root.clone());
        let Some(root) = root else { return };
        let maps = ta_ui::benchmark::SCENES.map(|scene| scene.title);
        self.worlds.want(Some(&root), &maps, crate::library::cache_dir(&self.store.dir(), "banner"));
        let width = ui.available_width();
        let (columns, gap) = (if width >= 880.0 { 8 } else { 4 }, 10.0);
        let w = ((width - gap * (columns - 1) as f32) / columns as f32).floor();
        let well = (w * 0.62).round();
        for row in ta_ui::benchmark::SCENES.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for scene in row {
                    let (rect, _) = ui.allocate_exact_size(vec2(w, well + 46.0), Sense::hover());
                    if !ui.is_rect_visible(rect) { continue; }
                    ui.painter().add(metal::plate(ui.ctx(), rect, PANEL));
                    let frame = Rect::from_min_size(rect.min + vec2(5.0, 5.0), vec2(w - 10.0, well));
                    let texture = self.worlds.pictures.iter().find(|(map, _)| map.eq_ignore_ascii_case(scene.title)).map(|(_, texture)| texture.clone());
                    crate::library::picture(ui, frame, texture.as_ref(), !self.worlds.loading() && texture.is_none());
                    // (Smaller rather than cut short, where a name is long for its tile.)
                    let room = (w - 12.0).max(1.0);
                    let title = widgets::fitted_galley(ui, |size| RichText::new(scene.title).font(strong(size)).color(CREAM).extra_letter_spacing(ta_ui::i18n::tracking(0.5)), 11.5, room, 0.78);
                    let moment = widgets::fitted_galley(ui, |size| RichText::new(t(scene.subtitle)).font(body(size)).color(STEEL), 11.5, room, 0.8);
                    let top = frame.bottom() + 6.0;
                    ui.painter().galley(pos2(rect.left() + 7.0, top), title.clone(), CREAM);
                    ui.painter().galley(pos2(rect.left() + 7.0, top + title.size().y + 1.0), moment, STEEL);
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
    }
}

/// Before the first run: what the benchmark does.
fn about_cards(ui: &mut Ui) {
    let cards = [
        (n("EIGHT SCENES"), n("A sea battle at dawn, a ride in a tank's cockpit through a forest, a thunderstorm, a station in the void, a base rising under the nano-lathes, twenty thousand units on a metal world, a lava world's nightfall and a town's fireworks at midnight.")),
        (n("TWO TESTS"), n("The same seconds of the storm at Low, Medium, High and Ultra, for the best level that holds 60 frames a second; then at High with each effect off in turn, for what each one costs.")),
        (n("THE SCORES"), n("The graphics card, the processor and the memory against the reference computer (an RTX 4080 with a Ryzen 9 5950X), which scores 10,000 in each. A quick run measures them alone, in a minute and a half.")),
    ];
    let card = |ui: &mut Ui, (title, text): (&str, &str)| {
        widgets::card(ui, |ui| {
            ui.set_min_height(120.0);
            ui.add(egui::Label::new(RichText::new(t(title)).font(strong(14.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.4))).wrap());
            ui.add(egui::Label::new(RichText::new(t(text)).color(STEEL)).wrap());
        });
    };
    if ui.available_width() >= 760.0 {
        widgets::columns(ui, 3, |columns| {
            for (ui, content) in columns.iter_mut().zip(cards) { card(ui, content); }
        });
    } else {
        for content in cards { card(ui, content); ui.add_space(space::S); }
    }
}

/// The classes in a row, the score's lit, a needle where it falls in it.
fn tier_ladder(ui: &mut Ui, overall: u32) {
    const TIERS: [Tier; 6] = [Tier::Peewee, Tier::Hammer, Tier::Bulldog, Tier::Goliath, Tier::Commander, Tier::Krogoth];
    let width = ui.available_width().min(520.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
    let painter = ui.painter();
    let here = Tier::of(overall);
    let each = rect.width() / TIERS.len() as f32;
    for (i, tier) in TIERS.iter().enumerate() {
        let from = match i {
            0 => 0,
            _ => TIERS[i - 1].next_at().unwrap_or(0),
        };
        let to = tier.next_at().unwrap_or(15_000);
        let seg = Rect::from_min_size(pos2(rect.min.x + i as f32 * each, rect.min.y), vec2(each - 3.0, 8.0));
        let lit = *tier <= here;
        painter.rect_filled(seg, CornerRadius::same(1), if lit { tier_color(*tier).gamma_multiply(if *tier == here { 1.0 } else { 0.45 }) } else { INSET });
        // The six names need more room than the bars at enlarged UI scales.
        if each >= 65.0 || *tier == here {
            let label = painter.layout_no_wrap(tier.name().into(), strong(9.5), if *tier == here { tier_color(*tier) } else { STEEL_DIM });
            let x = seg.min.x.min(rect.max.x - label.size().x).max(rect.min.x);
            painter.galley(pos2(x, seg.max.y + 4.0), label, STEEL_DIM);
        }
        if *tier == here {
            let f = ((overall.saturating_sub(from)) as f32 / (to - from).max(1) as f32).clamp(0.0, 1.0);
            let x = seg.min.x + f * seg.width();
            painter.add(Shape::convex_polygon(vec![pos2(x, seg.min.y - 1.0), pos2(x - 5.0, seg.min.y - 8.0), pos2(x + 5.0, seg.min.y - 8.0)], TEXT_BRIGHT, Stroke::NONE));
        }
    }
}

/// A processor's name as its maker gives it, less what says nothing (`(R)`, `(TM)`, `CPU`,
/// `Processor`, its count of cores and its clock): "AMD Ryzen 9 5950X 16-Core Processor" is an
/// "AMD Ryzen 9 5950X". (The whole name is in the computer's card.)
fn short_cpu(name: &str) -> String {
    let name = name.replace("(R)", "").replace("(r)", "").replace("(TM)", "").replace("(tm)", "");
    let name = name.split(" @ ").next().unwrap_or(&name).to_string();
    name.split_whitespace()
        .filter(|w| !w.eq_ignore_ascii_case("CPU") && !w.eq_ignore_ascii_case("Processor"))
        .filter(|w| !(w.to_ascii_lowercase().ends_with("-core") && w[..w.len() - 5].chars().all(|c| c.is_ascii_digit())))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A sub-score: its dial, the part measured and what was measured of it. Each line keeps to its
/// room (the name two lines, each figure one, all of it a hover away), so the three cards stand
/// the same height side by side.
fn score_card(ui: &mut Ui, title: &str, score: u32, color: Color32, name: &str, lines: &[String]) {
    widgets::card(ui, |ui| {
        widgets::card_title(ui, title);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 92.0), Sense::hover());
        let painter = ui.painter();
        let c = pos2(rect.center().x, rect.max.y - 10.0);
        let radius = 72.0f32.min(rect.width() / 2.0 - 6.0);
        // The dial: a half circle from 500 to 30,000 (as it scales: 10,000 near three quarters
        // round), the score's arc lit.
        let at = |v: f32| ((v.max(1.0) / 500.0).ln() / 60f32.ln()).clamp(0.0, 1.0);
        let arc = |from: f32, to: f32| -> Vec<egui::Pos2> {
            let steps = 40;
            (0..=steps).map(|k| {
                let f = from + (to - from) * k as f32 / steps as f32;
                let a = std::f32::consts::PI * (1.0 + f);
                c + vec2(a.cos(), a.sin()) * radius
            })
            .collect()
        };
        painter.add(Shape::line(arc(0.0, 1.0), Stroke::new(9.0, INSET)));
        painter.add(Shape::line(arc(0.0, at(score as f32)), Stroke::new(9.0, color.gamma_multiply(0.85))));
        // The reference's mark.
        let r = std::f32::consts::PI * (1.0 + at(10_000.0));
        let dir = vec2(r.cos(), r.sin());
        painter.line_segment([c + dir * (radius - 9.0), c + dir * (radius + 9.0)], Stroke::new(2.0, TEXT_BRIGHT));
        painter.text(c + vec2(0.0, -6.0), Align2::CENTER_BOTTOM, score.to_string(), display(30.0), TEXT_BRIGHT);
        ui.add_space(space::XS);
        let kept = |ui: &mut Ui, text: &str, font: egui::FontId, color: Color32, rows: usize| {
            let width = ui.available_width().max(1.0);
            let mut job = egui::text::LayoutJob::simple(text.to_string(), font.clone(), color, width);
            job.wrap.max_rows = rows;
            job.wrap.break_anywhere = false;
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let height = ui.fonts_mut(|f| f.row_height(&font)) * rows as f32;
            let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
            let elided = galley.elided;
            ui.painter().galley(rect.min, galley, color);
            if elided {
                response.on_hover_text(text);
            }
        };
        kept(ui, name, strong(13.0), TEXT_BRIGHT, 2);
        for line in lines {
            kept(ui, line, body(12.5), STEEL, 1);
        }
    });
}

/// The run in time: the frame rate (its slowest frame's band below it, the target's line) over
/// the processor's and the graphics card's work on a frame, each part in its band; the values
/// under the pointer.
fn timeline(ui: &mut Ui, r: &Report) {
    let samples = &r.timeline;
    if samples.len() < 2 {
        wrapped(ui, t("No frames were measured."), STEEL);
        return;
    }
    // The legend.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        for (color, label) in [(FPS, n("Frames a second")), (FPS.gamma_multiply(0.5), n("Slowest frame")), (CPU, n("Processor (ms)")), (GPU, n("Graphics card (ms)")), (CPU.gamma_multiply(0.5), n("Simulation tick (ms)"))] {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (swatch, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                ui.painter().rect_filled(Rect::from_center_size(swatch.center(), vec2(14.0, 4.0)), CornerRadius::same(1), color);
                ui.label(RichText::new(t(label)).font(body(12.0)).color(STEEL));
            });
        }
    });
    ui.add_space(space::XS);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 344.0), Sense::hover());
    let painter = ui.painter_at(rect.expand(2.0));
    let left = 40.0;
    let fps_area = Rect::from_min_max(pos2(rect.min.x + left, rect.min.y + 36.0), pos2(rect.max.x - 6.0, rect.min.y + 210.0));
    let work_area = Rect::from_min_max(pos2(fps_area.min.x, fps_area.max.y + 22.0), pos2(fps_area.max.x, rect.max.y - 22.0));
    painter.add(metal::well(fps_area.expand(2.0), INSET));
    painter.add(metal::well(work_area.expand(2.0), INSET));
    let t0 = samples[0].t;
    let t1 = samples.last().map_or(t0 + 1.0, |s| s.t + 0.25).max(t0 + 1.0);
    let x_of = |time: f32| fps_area.min.x + (time - t0) / (t1 - t0) * fps_area.width();
    // Each part's band; named at the top, on two rows by turns (each name with room as far as the
    // next band named on its row).
    let mut bands: Vec<(u8, f32, f32)> = Vec::new();
    let mut start = 0;
    while start < samples.len() {
        let part = samples[start].part;
        let end = samples[start..].iter().position(|s| s.part != part).map_or(samples.len(), |k| start + k);
        bands.push((part, x_of(samples[start].t), x_of(samples[end - 1].t + 0.25)));
        start = end;
    }
    // (Where each row's last name ends.)
    let mut row_end = [f32::NEG_INFINITY; 2];
    for (i, &(part, x0, x1)) in bands.iter().enumerate() {
        if part % 2 == 1 {
            for area in [fps_area, work_area] {
                painter.rect_filled(Rect::from_x_y_ranges(x0..=x1, area.y_range()), CornerRadius::ZERO, Color32::from_white_alpha(6));
            }
        }
        let id = r.parts.get(part as usize).map_or("", |p| p.id.as_str());
        let name = widgets::galley(ui, RichText::new(part_title(id)).font(strong(10.5)).color(STEEL).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
        let row = i % 2;
        let end = bands.get(i + 2).map_or(fps_area.max.x, |b| b.1) - 4.0;
        // From the band's start; else (the last bands: narrow) ending where its room ends, clear
        // of the name before it on the row.
        let x = if x0 + 4.0 + name.size().x <= end { Some(x0 + 4.0) } else { Some(end - name.size().x).filter(|&x| x >= row_end[row] + 8.0) };
        if let Some(x) = x {
            let y = rect.min.y + 3.0 + row as f32 * 15.0;
            row_end[row] = x + name.size().x;
            painter.galley(pos2(x, y), name, STEEL);
            painter.line_segment([pos2(x0, y), pos2(x0, fps_area.min.y)], Stroke::new(1.0, LINE.gamma_multiply(0.6)));
        }
        painter.line_segment([pos2(x0, fps_area.min.y), pos2(x0, work_area.max.y)], Stroke::new(1.0, LINE));
    }
    // The frame rate's scale, and the target's line.
    let (top, step) = axis(samples.iter().map(|s| s.fps).fold(TARGET_FPS, f32::max) * 1.05);
    let y_fps = |v: f32| fps_area.max.y - (v / top).clamp(0.0, 1.0) * fps_area.height();
    let mut v = 0.0;
    while v <= top + 0.1 {
        let y = y_fps(v);
        painter.hline(fps_area.x_range(), y, Stroke::new(1.0, Color32::from_white_alpha(10)));
        painter.text(pos2(fps_area.min.x - 6.0, y), Align2::RIGHT_CENTER, format!("{v:.0}"), body(11.0), STEEL_DIM);
        v += step;
    }
    let target = y_fps(TARGET_FPS);
    painter.extend(Shape::dashed_line(&[pos2(fps_area.min.x, target), pos2(fps_area.max.x, target)], Stroke::new(1.0, GREEN.gamma_multiply(0.6)), 6.0, 4.0));
    // The slowest frames' band under the rate, then the rate.
    let mut band = Mesh::default();
    for s in samples {
        let x = x_of(s.t + 0.125);
        let i = band.vertices.len() as u32;
        band.colored_vertex(pos2(x, y_fps(s.fps)), FPS.gamma_multiply(0.3));
        band.colored_vertex(pos2(x, y_fps(s.low.min(s.fps))), FPS.gamma_multiply(0.3));
        if i >= 2 {
            band.add_triangle(i - 2, i - 1, i);
            band.add_triangle(i - 1, i, i + 1);
        }
    }
    painter.add(Shape::mesh(band));
    painter.add(Shape::line(samples.iter().map(|s| pos2(x_of(s.t + 0.125), y_fps(s.low.min(s.fps)))).collect(), Stroke::new(1.0, FPS.gamma_multiply(0.5))));
    painter.add(Shape::line(samples.iter().map(|s| pos2(x_of(s.t + 0.125), y_fps(s.fps))).collect(), Stroke::new(1.6, FPS)));
    // The work on a frame.
    let (ms_top, ms_step) = work_axis(samples, r.gpu_timed);
    let y_ms = |v: f32| work_area.max.y - (v / ms_top).clamp(0.0, 1.0) * work_area.height();
    let mut v = 0.0;
    while v <= ms_top + 0.01 {
        let y = y_ms(v);
        painter.hline(work_area.x_range(), y, Stroke::new(1.0, Color32::from_white_alpha(10)));
        painter.text(pos2(work_area.min.x - 6.0, y), Align2::RIGHT_CENTER, format!("{v:.0}"), body(11.0), STEEL_DIM);
        v += ms_step;
    }
    let series = |f: fn(&Sample) -> f32| samples.iter().map(|s| pos2(x_of(s.t + 0.125), y_ms(f(s)))).collect::<Vec<_>>();
    painter.add(Shape::line(series(|s| s.tick_ms), Stroke::new(1.0, CPU.gamma_multiply(0.5))));
    painter.add(Shape::line(series(|s| s.cpu_ms), Stroke::new(1.4, CPU)));
    if r.gpu_timed {
        painter.add(Shape::line(series(|s| s.gpu_ms), Stroke::new(1.4, GPU)));
    }
    // The time under it all.
    let (_, t_step) = axis((t1 - t0) / 1.6);
    let mut time = 0.0;
    while time <= t1 - t0 + 0.1 {
        let x = x_of(t0 + time);
        painter.text(pos2(x, work_area.max.y + 6.0), Align2::CENTER_TOP, clock(time), body(11.0), STEEL_DIM);
        time += t_step.max(5.0);
    }
    // The values under the pointer.
    if let Some(p) = response.hover_pos().filter(|p| fps_area.x_range().contains(p.x)) {
        let at = t0 + (p.x - fps_area.min.x) / fps_area.width() * (t1 - t0);
        if let Some(s) = samples.iter().min_by(|a, b| (a.t + 0.125 - at).abs().total_cmp(&(b.t + 0.125 - at).abs())) {
            let x = x_of(s.t + 0.125);
            painter.vline(x, fps_area.min.y..=work_area.max.y, Stroke::new(1.0, TEXT_BRIGHT.gamma_multiply(0.5)));
            painter.circle_filled(pos2(x, y_fps(s.fps)), 3.5, FPS);
            let id = r.parts.get(s.part as usize).map_or("", |p| p.id.as_str());
            let s = *s;
            response.hover_tip_ui(|ui| {
                ui.label(RichText::new(format!("{} · {}", part_title(id), clock(s.t))).font(strong(12.0)).color(TEXT_BRIGHT));
                ui.label(RichText::new(tf("{fps} frames a second (the slowest at {low})", &[("fps", &format!("{:.0}", s.fps)), ("low", &format!("{:.0}", s.low))])).color(FPS));
                ui.label(RichText::new(tf("Processor {cpu} ms · graphics card {gpu} ms", &[("cpu", &format!("{:.1}", s.cpu_ms)), ("gpu", &format!("{:.1}", s.gpu_ms))])).color(TEXT));
                ui.label(RichText::new(tf("Simulation {ms} ms a tick · {units} units", &[("ms", &format!("{:.2}", s.tick_ms)), ("units", &s.units)])).color(STEEL));
            });
        }
    }
}

/// Each scene's numbers, and all of them together.
fn battles_table(ui: &mut Ui, r: &Report) {
    let all = if r.first() { t("ALL THREE") } else { t("ALL SCENES") };
    let rows: Vec<(String, String, Stats)> = r
        .scenes()
        .map(|p| {
            let (name, what) = part_name(&p.id, r.first());
            (name.to_string(), what.to_string(), p.stats)
        })
        .chain(std::iter::once((all.to_string(), String::new(), r.overall())))
        .collect();
    let width = ui.available_width();
    // (Narrow: the work on a frame left out.)
    let wide = width >= 760.0;
    egui::ScrollArea::horizontal().id_salt("bench_battles").show(ui, |ui| {
        egui::Grid::new("bench_battles").striped(true).spacing(vec2(18.0, 8.0)).min_col_width(40.0).show(ui, |ui| {
            let first = if r.first() { n("BATTLE") } else { n("SCENE") };
            let heads: Vec<&str> = if wide {
                vec![first, n("FPS"), n("1% LOW"), n("0.1% LOW"), n("FRAME"), n("STUTTERS"), n("PROCESSOR"), n("GRAPHICS CARD"), n("SIMULATION"), n("UNITS")]
            } else {
                vec![first, n("FPS"), n("1% LOW"), n("0.1% LOW"), n("FRAME"), n("UNITS")]
            };
            for head in &heads {
                ui.label(caption(head));
            }
            ui.end_row();
            for (name, what, s) in &rows {
                ui.vertical(|ui| {
                    ui.set_min_width(150.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add(egui::Label::new(RichText::new(name).font(strong(13.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(0.8))).extend());
                    if !what.is_empty() {
                        ui.add(egui::Label::new(RichText::new(what).font(body(11.5)).color(STEEL_DIM)).extend());
                    }
                });
                let figure = |ui: &mut Ui, text: String, color: Color32| {
                    ui.label(RichText::new(text).font(strong(14.0)).color(color));
                };
                figure(ui, format!("{:.0}", s.fps), FPS);
                figure(ui, format!("{:.0}", s.low1), if s.low1 < TARGET_FPS * 0.9 { RED } else { TEXT_BRIGHT });
                figure(ui, format!("{:.0}", s.low01), TEXT);
                figure(ui, ms(format!("{:.1} / {:.1}", s.frame_ms, s.frame_p99)), TEXT);
                if wide {
                    figure(ui, s.hitches.to_string(), if s.hitches > 0 { AMBER } else { TEXT });
                    figure(ui, ms(format!("{:.1}", s.cpu_ms)), CPU);
                    figure(ui, if r.gpu_timed { ms(format!("{:.1}", s.gpu_ms)) } else { "—".into() }, GPU);
                    figure(ui, ms(format!("{:.2}", s.tick_ms)), CPU.gamma_multiply(0.7));
                }
                figure(ui, s.units.to_string(), TEXT);
                ui.end_row();
            }
        });
    });
    ui.add_space(space::XS);
    wrapped(ui, t("FPS: frames a second on average; the lows: the slowest 1% and 0.1% of frames; frame: its time on average and for 99 frames in 100; stutters: frames of 50 ms or more, three times the usual or longer."), STEEL_DIM);
}

/// The levels side by side: their frame rates against the target, the one recommended and the
/// player's marked.
fn settings_test(ui: &mut Ui, r: &Report, rec: Option<&Recommendation>, yours: Preset) {
    if r.levels.is_empty() {
        wrapped(ui, t("The settings test did not run."), STEEL);
        return;
    }
    let most = r.levels.iter().map(|l| l.stats.fps).fold(TARGET_FPS, f32::max) * 1.08;
    // Each level's figures, and the column they need (the bars take the rest).
    let rows: Vec<(Preset, Stats, std::sync::Arc<egui::Galley>)> = LEVELS
        .iter()
        .filter_map(|&level| r.level(level).map(|run| (level, run.stats)))
        .map(|(level, stats)| {
            let mut text = tf("{fps} FPS · 1% low {low}", &[("fps", &format!("{:.0}", stats.fps)), ("low", &format!("{:.0}", stats.low1))]);
            if rec.is_some_and(|rec| level == rec.level) {
                text = format!("{text} · {}", t("RECOMMENDED"));
            } else if level == yours {
                text = format!("{text} · {}", t("YOUR LEVEL"));
            }
            let color = if rec.is_some_and(|rec| level == rec.level) { GREEN } else { TEXT };
            (level, stats, widgets::galley(ui, RichText::new(text).font(body(12.5)).color(color)))
        })
        .collect();
    let label_width = LEVELS.iter().map(|l| widgets::galley(ui, RichText::new(t(l.label())).font(strong(14.0))).size().x).fold(90.0, f32::max) + 20.0;
    let value_width = rows.iter().map(|(_, _, g)| g.size().x).fold(0.0, f32::max) + 16.0;
    if ui.available_width() < label_width + value_width + 100.0 {
        // At narrow widths (including enlarged text), keep the figures above their bar.
        // Reserving an 80px bar beside long values used to paint outside the viewport.
        for (level, stats, text) in &rows {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t(level.label())).font(strong(14.0)).color(if rec.is_some_and(|rec| *level == rec.level) { GREEN } else { TEXT_BRIGHT }));
                ui.add(egui::Label::new(RichText::new(&text.job.text).font(body(12.5)).color(TEXT)).wrap());
            });
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
            let painter = ui.painter_at(row);
            painter.add(metal::well(row, INSET));
            let x_of = |fps: f32| row.min.x + (fps / most).clamp(0.0, 1.0) * row.width();
            let picked = rec.is_some_and(|rec| *level == rec.level);
            let fill = Rect::from_min_max(row.min, pos2(x_of(stats.fps), row.max.y)).shrink(2.0);
            if fill.is_positive() {
                painter.add(metal::hgrad(fill, if picked { GREEN.gamma_multiply(0.55) } else { AMBER_DIM.gamma_multiply(0.7) }, if picked { GREEN } else { AMBER }));
            }
            painter.vline(x_of(stats.low1), row.y_range(), Stroke::new(2.0, TEXT_BRIGHT));
            painter.vline(x_of(TARGET_FPS), row.y_range(), Stroke::new(1.0, GREEN));
            ui.add_space(space::S);
        }
    } else {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), rows.len() as f32 * 38.0 + 22.0), Sense::hover());
        let painter = ui.painter();
        let bars = Rect::from_min_max(pos2(rect.min.x + label_width, rect.min.y + 18.0), pos2((rect.max.x - value_width).max(rect.min.x + label_width + 80.0), rect.max.y));
        let x_of = |fps: f32| bars.min.x + (fps / most).clamp(0.0, 1.0) * bars.width();
        for (i, (level, stats, text)) in rows.into_iter().enumerate() {
            let y = bars.min.y + i as f32 * 38.0;
            let row = Rect::from_min_max(pos2(bars.min.x, y + 6.0), pos2(bars.max.x, y + 28.0));
            let picked = rec.is_some_and(|rec| level == rec.level);
            painter.text(pos2(rect.min.x, row.center().y), Align2::LEFT_CENTER, t(level.label()), strong(14.0), if picked { GREEN } else { TEXT_BRIGHT });
            painter.add(metal::well(row, INSET));
            let fill = Rect::from_min_max(row.min, pos2(x_of(stats.fps), row.max.y)).shrink(2.0);
            if fill.is_positive() {
                painter.add(metal::hgrad(fill, if picked { GREEN.gamma_multiply(0.55) } else { AMBER_DIM.gamma_multiply(0.7) }, if picked { GREEN } else { AMBER }));
            }
            // The slowest 1%'s mark.
            let low = x_of(stats.low1);
            painter.vline(low, row.y_range(), Stroke::new(2.0, TEXT_BRIGHT));
            let height = text.size().y;
            painter.galley(pos2(bars.max.x + 12.0, row.center().y - height / 2.0), text, TEXT);
        }
        let x = x_of(TARGET_FPS);
        painter.extend(Shape::dashed_line(&[pos2(x, bars.min.y), pos2(x, bars.max.y)], Stroke::new(1.0, GREEN), 5.0, 4.0));
        painter.text(pos2(x, rect.min.y + 2.0), Align2::CENTER_TOP, tf("{fps} FPS", &[("fps", &format!("{TARGET_FPS:.0}"))]), strong(10.5), GREEN);
    }
    ui.add_space(space::S);
    let samples = if r.tested_msaa <= 1 { t("no antialiasing").to_string() } else { tf("{n}× antialiasing", &[("n", &r.tested_msaa)]) };
    let what = if r.first() { t("The same seconds of Highland Rift at each level") } else { t("The same seconds of the storm on Highland Rift at each level") };
    wrapped(ui, &tf("{what}, with {samples} throughout (it changes only as the game starts). The white mark is the slowest 1% of frames.", &[("what", &what), ("samples", &samples)]), STEEL_DIM);
}

/// A scene's card: its picture (or its name where it has none), its world and moment, its frame
/// rate, slowest frames and stutters, what it shows off; the rest a hover away.
fn scene_card(ui: &mut Ui, r: &Report, part: &ta_config::benchmark::Part, picture: Option<&egui::TextureHandle>) {
    let width = ui.available_width();
    let (frame, response) = ui.allocate_exact_size(vec2(width, width * 9.0 / 16.0), Sense::hover());
    let painter = ui.painter();
    painter.add(metal::well(frame, INSET));
    let inner = frame.shrink(2.0);
    match picture {
        Some(texture) => {
            painter.image(texture.id(), inner, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.text(inner.center(), Align2::CENTER_CENTER, t("No picture"), body(12.0), STEEL_DIM);
        }
    }
    let info = ta_ui::benchmark::scene(&part.id, r.first());
    let s = part.stats;
    // The frame rate over the picture's foot, on a shade.
    let shade = Rect::from_min_max(pos2(inner.min.x, inner.max.y - 34.0), inner.max);
    painter.rect_filled(shade, CornerRadius::ZERO, Color32::from_black_alpha(150));
    let fps = painter.text(pos2(shade.min.x + 10.0, shade.center().y), Align2::LEFT_CENTER, format!("{:.0}", s.fps), display(22.0), TEXT_BRIGHT);
    painter.text(pos2(fps.max.x + 5.0, shade.center().y + 3.0), Align2::LEFT_CENTER, t("FPS"), strong(10.5), AMBER_BRIGHT);
    let low = tf("1% low {low}", &[("low", &format!("{:.0}", s.low1))]);
    painter.text(pos2(shade.max.x - 10.0, shade.center().y), Align2::RIGHT_CENTER, low, strong(12.0), if s.low1 < TARGET_FPS * 0.9 { RED } else { TEXT });
    ui.add_space(space::XS);
    let (title, subtitle) = part_name(&part.id, r.first());
    ui.label(RichText::new(title).font(strong(14.0)).color(TEXT_BRIGHT).extra_letter_spacing(ta_ui::i18n::tracking(1.0)));
    ui.label(RichText::new(subtitle).font(body(12.5)).color(STEEL));
    if let Some(info) = info {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(5.0, 4.0);
            for feature in info.features {
                chip(ui, t(feature));
            }
        });
    }
    response.hover_tip_ui(|ui| {
        ui.label(RichText::new(format!("{title} · {subtitle}")).font(strong(12.0)).color(TEXT_BRIGHT));
        ui.label(RichText::new(tf("{fps} frames a second · 1% low {low1} · 0.1% low {low01}", &[("fps", &format!("{:.0}", s.fps)), ("low1", &format!("{:.0}", s.low1)), ("low01", &format!("{:.0}", s.low01))])).color(FPS));
        ui.label(RichText::new(tf("Processor {cpu} ms · graphics card {gpu} ms a frame", &[("cpu", &format!("{:.1}", s.cpu_ms)), ("gpu", &format!("{:.1}", s.gpu_ms))])).color(TEXT));
        ui.label(RichText::new(tf("Simulation {ms} ms a tick · {units} units at most", &[("ms", &format!("{:.2}", s.tick_ms)), ("units", &s.units)])).color(STEEL));
        if s.hitches > 0 {
            ui.label(RichText::new(tf("{n} stutters", &[("n", &s.hitches)])).color(AMBER));
        }
    });
}

/// What a scene shows off, as a chip: allocated whole, so that a row with no room for it puts it
/// on the next.
fn chip(ui: &mut Ui, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), strong(10.5), AMBER_BRIGHT);
    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(12.0, 5.0), Sense::hover());
    let painter = ui.painter();
    painter.rect(rect, CornerRadius::same(2), Color32::from_black_alpha(90), Stroke::new(1.0, AMBER.gamma_multiply(0.45)), egui::StrokeKind::Inside);
    painter.galley(rect.center() - galley.size() / 2.0, galley, AMBER_BRIGHT);
}

/// A scene's row of the passes' chart: its id, its passes' times (the time between them
/// unnamed), its frame.
type PassRow<'a> = (&'a str, Vec<(String, f32)>, f32);

/// Each scene's frame on the graphics card, pass by pass: a bar each, as long as its frame, its
/// passes in the renderer's order (and the time between them).
fn passes_chart(ui: &mut Ui, r: &Report) {
    let rows: Vec<PassRow> = r
        .scenes()
        .filter(|p| !p.passes.is_empty())
        .map(|p| {
            let mut passes: Vec<(String, f32)> = PASS_ORDER.iter().filter_map(|&name| p.passes.iter().find(|q| q.pass == name).map(|q| (q.pass.clone(), q.ms))).collect();
            let named: f32 = passes.iter().map(|(_, ms)| ms).sum();
            let frame = p.stats.gpu_ms.max(named);
            if frame - named > 0.05 {
                passes.push((String::new(), frame - named));
            }
            (p.id.as_str(), passes, frame)
        })
        .collect();
    if rows.is_empty() || !r.gpu_timed {
        wrapped(ui, t("This graphics card does not time its own work: its passes were not measured."), STEEL);
        return;
    }
    // The legend: the passes there are.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let mut shown: Vec<&str> = PASS_ORDER.iter().copied().filter(|name| rows.iter().any(|(_, passes, _)| passes.iter().any(|(p, _)| p == name))).collect();
        if rows.iter().any(|(_, passes, _)| passes.iter().any(|(p, _)| p.is_empty())) {
            shown.push("");
        }
        for name in shown {
            let (label, color) = pass_style(name);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (swatch, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                ui.painter().rect_filled(swatch.shrink(1.0), CornerRadius::same(2), color);
                ui.label(RichText::new(t(label)).font(body(12.0)).color(STEEL));
            });
        }
    });
    ui.add_space(space::S);
    let most = rows.iter().map(|(_, _, frame)| *frame).fold(1.0f32, f32::max);
    let label_width = 150.0f32.min(ui.available_width() * 0.35);
    let value_width = 70.0;
    for (id, passes, frame) in &rows {
        let (row, response) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
        let painter = ui.painter();
        painter.text(pos2(row.min.x, row.center().y), Align2::LEFT_CENTER, part_title(id), strong(11.5), TEXT_BRIGHT);
        let bar = Rect::from_min_max(pos2(row.min.x + label_width, row.min.y + 4.0), pos2(row.max.x - value_width, row.max.y - 4.0));
        painter.add(metal::well(bar, INSET));
        let mut x = bar.min.x + 1.0;
        for (name, ms) in passes {
            let w = (bar.width() - 2.0) * ms / most;
            if w > 0.5 {
                painter.rect_filled(Rect::from_min_max(pos2(x, bar.min.y + 1.0), pos2(x + w, bar.max.y - 1.0)), CornerRadius::ZERO, pass_style(name).1);
            }
            x += w;
        }
        painter.text(pos2(row.max.x, row.center().y), Align2::RIGHT_CENTER, ms(format!("{frame:.1}")), strong(12.5), TEXT_BRIGHT);
        response.hover_tip_ui(|ui| {
            ui.label(RichText::new(part_title(id)).font(strong(12.0)).color(TEXT_BRIGHT));
            for (name, ms) in passes {
                let (label, color) = pass_style(name);
                ui.label(RichText::new(format!("{} · {:.2} ms", t(label), ms)).color(color));
            }
        });
    }
    ui.add_space(space::XS);
    wrapped(ui, t("The graphics card's time on a frame of each scene, as the renderer's passes took it at your settings."), STEEL_DIM);
}

/// The effects test: what each effect costs the graphics card at High, from the dearest; the
/// ones too cheap to measure together at the foot.
fn effects_chart(ui: &mut Ui, r: &Report) {
    let mut costs = r.effect_costs();
    if costs.is_empty() {
        wrapped(ui, t("A full run measures what each effect costs: the storm's same seconds at High, with one effect turned off at a time."), STEEL);
        return;
    }
    costs.sort_by(|a, b| b.1.total_cmp(&a.1));
    let high = r.effects.iter().find(|e| e.effect == Effect::High).map_or(0.0, |e| if r.gpu_timed && e.stats.gpu_ms > 0.0 { e.stats.gpu_ms } else { e.stats.frame_ms });
    wrapped(ui, &tf("At High the graphics card took {ms} ms a frame of the storm. Turning each of these off saved:", &[("ms", &format!("{high:.1}"))]), TEXT);
    ui.add_space(space::S);
    // (Under a fifth of a millisecond is within the measurement's own noise.)
    const NOISE: f32 = 0.2;
    let most = costs.iter().map(|c| c.1).fold(NOISE, f32::max);
    let label_width = costs.iter().map(|(e, _, _)| widgets::galley(ui, RichText::new(t(ta_ui::benchmark::effect_label(*e))).font(strong(12.5))).size().x).fold(120.0, f32::max) + 16.0;
    let value_width = 150.0;
    let mut cheap = Vec::new();
    for (effect, cost, share) in &costs {
        if *cost < NOISE {
            cheap.push(t(ta_ui::benchmark::effect_label(*effect)));
            continue;
        }
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
        let painter = ui.painter();
        painter.text(pos2(row.min.x, row.center().y), Align2::LEFT_CENTER, t(ta_ui::benchmark::effect_label(*effect)), strong(12.5), TEXT_BRIGHT);
        let bar = Rect::from_min_max(pos2(row.min.x + label_width, row.min.y + 5.0), pos2((row.max.x - value_width).max(row.min.x + label_width + 40.0), row.max.y - 5.0));
        painter.add(metal::well(bar, INSET));
        let fill = Rect::from_min_size(bar.min + vec2(1.0, 1.0), vec2((bar.width() - 2.0) * cost / most, bar.height() - 2.0));
        if fill.is_positive() {
            painter.add(metal::hgrad(fill, GPU.gamma_multiply(0.5), GPU));
        }
        painter.text(pos2(bar.max.x + 10.0, row.center().y), Align2::LEFT_CENTER, tf("{ms} ms · {share}%", &[("ms", &format!("{cost:.1}")), ("share", &format!("{:.0}", share * 100.0))]), strong(12.0), TEXT);
    }
    if !cheap.is_empty() {
        ui.add_space(space::XS);
        wrapped(ui, &tf("Too little to measure here: {effects}.", &[("effects", &cheap.join(", "))]), STEEL);
    }
    ui.add_space(space::XS);
    wrapped(ui, t("Each from the same seconds at High, the rest as they were; full resolution against three quarters of it. What an effect costs depends on what is in view: water, trees and fire are, here."), STEEL_DIM);
}

/// The scenes' frames by their time: how many came at each rate, the middle one's time and the
/// slowest, and the stutters.
fn frame_times(ui: &mut Ui, r: &Report) {
    let counts = &r.histogram;
    let total: u32 = counts.iter().sum();
    if total == 0 {
        wrapped(ui, t("No frames were measured."), STEEL);
        return;
    }
    let labels: Vec<String> = (0..=FRAME_BUCKETS.len())
        .map(|i| {
            let fps = |ms: f32| (1000.0 / ms).round();
            match i {
                0 => tf("{fps}+", &[("fps", &fps(FRAME_BUCKETS[0]))]),
                i if i == FRAME_BUCKETS.len() => tf("under {fps}", &[("fps", &fps(FRAME_BUCKETS[i - 1]))]),
                i => format!("{}–{}", fps(FRAME_BUCKETS[i]), fps(FRAME_BUCKETS[i - 1])),
            }
        })
        .collect();
    ui.label(RichText::new(t("Frames a second, frame by frame")).font(body(11.5)).color(STEEL_DIM));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::hover());
    let painter = ui.painter();
    let area = Rect::from_min_max(pos2(rect.min.x, rect.min.y + 18.0), pos2(rect.max.x, rect.max.y - 22.0));
    let each = area.width() / labels.len() as f32;
    let most = counts.iter().copied().max().unwrap_or(1).max(1) as f32;
    for (i, (count, label)) in counts.iter().zip(&labels).enumerate() {
        let x0 = area.min.x + i as f32 * each;
        let column = Rect::from_min_max(pos2(x0 + 6.0, area.min.y), pos2(x0 + each - 6.0, area.max.y));
        painter.add(metal::well(column, INSET));
        let h = (column.height() - 2.0) * *count as f32 / most;
        // Colour by how smooth: from green (60 and more) to red (under 30).
        let color = match i {
            0..=2 => GREEN,
            3 => AMBER,
            4 => AMBER_DIM,
            _ => RED,
        };
        if h > 0.5 {
            painter.rect_filled(Rect::from_min_max(pos2(column.min.x + 1.0, column.max.y - 1.0 - h), pos2(column.max.x - 1.0, column.max.y - 1.0)), CornerRadius::same(1), color.gamma_multiply(0.85));
        }
        let share = *count as f32 / total as f32 * 100.0;
        painter.text(pos2(column.center().x, column.max.y - 1.0 - h - 3.0), Align2::CENTER_BOTTOM, format!("{share:.0}%"), strong(11.0), TEXT);
        painter.text(pos2(column.center().x, area.max.y + 5.0), Align2::CENTER_TOP, label, body(11.0), STEEL);
    }
    ui.add_space(space::S);
    let o = r.overall();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 24.0;
        for (label, value, color) in [
            (n("MIDDLE FRAME"), ms(format!("{:.1}", o.frame_p50)), TEXT_BRIGHT),
            (n("1 IN 10 LONGER THAN"), ms(format!("{:.1}", o.frame_p90)), TEXT_BRIGHT),
            (n("1 IN 100 LONGER THAN"), ms(format!("{:.1}", o.frame_p99)), TEXT_BRIGHT),
            (n("STUTTERS"), o.hitches.to_string(), if o.hitches > 0 { AMBER } else { GREEN }),
        ] {
            ui.vertical(|ui| {
                ui.label(caption(label));
                ui.label(RichText::new(value).font(display(20.0)).color(color));
            });
        }
    });
}

/// The computer the run was on.
fn system_card(ui: &mut Ui, r: &Report) {
    let s = &r.system;
    widgets::card(ui, |ui| {
        widgets::card_title(ui, "THIS COMPUTER");
        let width = caption_width(ui, &[n("GRAPHICS CARD"), n("DRIVER"), n("PROCESSOR"), n("MEMORY"), n("SCREEN"), n("THE GAME USED"), n("LEFT OUT")]);
        field_with(ui, n("GRAPHICS CARD"), width, |ui| {
            wrapped(ui, &if s.vram_mb > 0 { format!("{} · {}", s.gpu, gb(s.vram_mb)) } else { s.gpu.clone() }, TEXT_BRIGHT);
        });
        field_with(ui, n("DRIVER"), width, |ui| {
            wrapped(ui, &format!("{} · {}", s.backend, s.driver), TEXT);
        });
        field_with(ui, n("PROCESSOR"), width, |ui| {
            wrapped(ui, &tf("{cpu} · {threads} threads", &[("cpu", &known(&s.cpu)), ("threads", &s.threads)]), TEXT_BRIGHT);
        });
        field_with(ui, n("MEMORY"), width, |ui| {
            wrapped(ui, &if s.ram_mb > 0 { gb(s.ram_mb) } else { known("") }, TEXT_BRIGHT);
        });
        field_with(ui, n("SCREEN"), width, |ui| {
            let text = if r.refresh_hz > 0.0 { tf("{width} × {height} at {hz} Hz", &[("width", &r.width), ("height", &r.height), ("hz", &format!("{:.0}", r.refresh_hz))]) } else { format!("{} × {}", r.width, r.height) };
            wrapped(ui, &text, TEXT);
        });
        // (Measured on Windows only.)
        if r.memory.peak_mb > 0 {
            field_with(ui, n("THE GAME USED"), width, |ui| {
                wrapped(ui, &tf("{mb} MB of memory at most", &[("mb", &r.memory.peak_mb)]), TEXT);
            });
        }
        if !r.left_out.is_empty() {
            field_with(ui, n("LEFT OUT"), width, |ui| {
                wrapped(ui, &r.left_out.join(", "), AMBER);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_scale_includes_simulation_work_without_hidden_gpu_outliers() {
        let samples = [Sample { cpu_ms: 4.0, gpu_ms: 7.0, tick_ms: 85.0, ..Default::default() },
            Sample { cpu_ms: 5.0, gpu_ms: 900.0, tick_ms: 120.0, ..Default::default() }];
        let (top, step) = work_axis(&samples, false);
        assert!((120.0..900.0).contains(&top), "all shown tick timings fit without an unmeasured GPU value flattening the chart");
        assert!(step > 0.0);
        assert!(work_axis(&samples, true).0 >= 900.0, "a measured GPU series fits too");
    }

    #[test]
    fn processors_names_lose_what_says_nothing() {
        assert_eq!(short_cpu("AMD Ryzen 9 5950X 16-Core Processor"), "AMD Ryzen 9 5950X");
        assert_eq!(short_cpu("Intel(R) Core(TM) i7-9700K CPU @ 3.60GHz"), "Intel Core i7-9700K");
        assert_eq!(short_cpu("12th Gen Intel(R) Core(TM) i5-12400F"), "12th Gen Intel Core i5-12400F");
        assert_eq!(short_cpu("Apple M2"), "Apple M2");
    }

    #[test]
    fn axes_take_round_steps() {
        assert_eq!(axis(100.0), (100.0, 25.0));
        assert_eq!(axis(212.0), (300.0, 100.0));
        assert_eq!(axis(9.0), (10.0, 2.5));
        assert_eq!(clock(65.4), "1:05");
    }

    /// Before a first run, with an install: the eight worlds in a row (or two rows) inside the
    /// page however narrow, each tile laid out whether or not its picture could be drawn.
    #[test]
    fn the_worlds_before_a_first_run_keep_inside_the_page() {
        use clap::Parser;
        let root = std::env::temp_dir().join(format!("ta_launcher_bench_worlds_{}", std::process::id()));
        let game = root.join("game");
        std::fs::create_dir_all(&game).unwrap();
        for file in ta_vfs::install::REQUIRED_FILES {
            std::fs::write(game.join(file), b"").unwrap();
        }
        let config = root.join("config.toml");
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]);
        let ctx = egui::Context::default();
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        app.detection = crate::detect::detect(Some(&game), true);
        assert!(app.detection.install().is_some());
        for width in [320.0, 470.0, 880.0, 1400.0] {
            for _ in 0..3 {
                let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 5000.0))), ..Default::default() };
                let mut bounds = Rect::NOTHING;
                ctx.run_ui(input, |ui| { app.benchmark_page(ui); bounds = ui.min_rect(); }).drop_without_applying_deltas();
                assert!(bounds.max.x <= width + 1.0, "the worlds exceeded {width}: {bounds:?}");
            }
        }
        // (The pictures were asked for, from that install.)
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.worlds.loading() && std::time::Instant::now() < until {
            app.worlds.poll(&ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.worlds.loading(), "an install whose archives are empty draws nothing, at once");
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn benchmark_page_stacks_cards_inside_a_narrow_viewport() {
        use clap::Parser;
        use ta_config::benchmark::{LevelRun, Part};
        let root = std::env::temp_dir().join(format!("ta_launcher_bench_layout_{}", std::process::id()));
        let config = root.join("config.toml");
        let args = crate::Args::parse_from(["ta-launcher", "--offline", "--pretend-missing", "--config", config.to_str().unwrap()]);
        let ctx = egui::Context::default();
        let mut app = LauncherApp::new(&eframe::CreationContext::_new_kittest(ctx.clone()), args);
        app.skins.shutdown();
        let stats = Stats { frames: 1000, fps: 100.0, low1: 80.0, frame_ms: 10.0, ..Default::default() };
        for measured in [false, true] {
            if measured { app.benchmarks.runs.push((root.join("sample.json"), Report {
                format: 1, when: 1_790_000_000, parts: vec![Part { id: "sapphire".into(), stats, ..Default::default() }],
                levels: LEVELS.iter().map(|&level| LevelRun { level, stats }).collect(), ..Default::default()
            })); }
            for width in [320.0, 400.0, 470.0, 680.0, 1120.0] {
                for _ in 0..3 {
                    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 5000.0))), ..Default::default() };
                    let mut bounds = Rect::NOTHING;
                    ctx.run_ui(input, |ui| { app.benchmark_page(ui); bounds = ui.min_rect(); }).drop_without_applying_deltas();
                    assert!(bounds.max.x <= width + 1.0, "benchmark measured={measured} exceeded width {width}: {bounds:?}");
                }
            }
        }
        drop(app);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn settings_chart_fits_narrow_viewports_with_wrapped_figures() {
        use ta_config::benchmark::LevelRun;
        let report = Report { levels: LEVELS.iter().map(|&level| LevelRun { level,
            stats: Stats { frames: 300, fps: 1234.0, low1: 1200.0, ..Default::default() } }).collect(), ..Default::default() };
        let rec = report.recommend();
        for width in [240.0, 320.0, 470.0, 720.0] {
            let ctx = egui::Context::default();
            crate::theme::install(&ctx);
            let mut bounds = Rect::NOTHING;
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 800.0))), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| { settings_test(ui, &report, Some(&rec), Preset::High); bounds = ui.min_rect(); });
            output.textures_delta.clear();
            assert!(bounds.max.x <= width + 1.0, "chart layout exceeded {width}: {bounds:?}");
            for clipped in output.shapes {
                if let Shape::Text(text) = clipped.shape {
                    assert!(text.pos.x + text.galley.size().x <= width + 1.0, "chart text overflow at {width}: {}", text.galley.job.text);
                }
            }
        }
    }

    #[test]
    fn shared_results_use_report_settings_and_exclude_the_settings_test() {
        use ta_config::benchmark::{Part, System};
        let report = Report {
            version: "2.1.0".into(), width: 2560, height: 1440, tested_msaa: 4,
            graphics: ta_config::Graphics { preset: Preset::High, render_scale: 0.75, ..Default::default() },
            system: System { gpu: "Test GPU".into(), cpu: "Test CPU".into(), ram_mb: 32768, ..Default::default() },
            parts: vec![
                Part { id: "sapphire".into(), stats: Stats { frames: 600, frame_ms: 10.0, fps: 100.0, low1: 80.0, low01: 60.0, ..Default::default() }, ..Default::default() },
                Part { id: "settings".into(), stats: Stats { frames: 6000, fps: 999.0, ..Default::default() }, ..Default::default() },
                Part { id: "highland".into(), stats: Stats { frames: 300, frame_ms: 20.0, fps: 50.0, low1: 40.0, low01: 30.0, ..Default::default() }, ..Default::default() },
            ],
            left_out: vec!["CORKROG".into()],
            ..Default::default()
        };
        let summary = report_summary(&report);
        assert!(summary.contains("v2.1.0"));
        assert!(summary.contains("2560 × 1440 · HIGH · render scale 75% · antialiasing 4×"));
        assert!(summary.contains("Unit skin: Reborn"));
        assert!(!summary.contains("Compare scores with the same unit skin"));
        assert!(summary.contains("Test GPU") && summary.contains("Test CPU") && summary.contains("32 GB"));
        assert!(summary.contains("ALL THREE: 75.0 FPS average · 1% low 40.0 · 0.1% low 30.0"));
        assert!(summary.contains("SAPPHIRE REACH: 100.0 FPS") && summary.contains("HIGHLAND RIFT: 50.0 FPS"));
        assert!(!summary.contains("999.0 FPS") && !summary.contains("SETTINGS TEST"));
        assert!(summary.contains("does not time its own work") && summary.contains("Units left out: CORKROG"));
        assert!(!summary.contains("Accessibility effects"));
        let reduced = Report { reduced_motion: true, reduced_flashes: true, ..report };
        let summary = report_summary(&reduced);
        assert!(summary.contains("REDUCED MOTION · REDUCED FLASHES"));
        assert!(summary.contains("Compare scores with the same accessibility settings"));
    }

    #[test]
    fn shared_results_identify_the_recorded_skin_and_comparison_requirement() {
        for (id, name) in [("christmas", "Christmas"), ("builtin:tiger", "Tiger"), ("my-custom-armour", "my-custom-armour")] {
            let report = Report { rendered_unit_skin: Some(id.into()), ..Default::default() };
            let summary = report_summary(&report);
            assert!(summary.contains(&format!("Unit skin: {name}")), "{summary}");
            assert!(summary.contains("Compare scores with the same unit skin."));
        }
        let split = Report { rendered_unit_skin: Some("winter".into()), rendered_enemy_unit_skin: Some("desert".into()), ..Default::default() };
        let summary = report_summary(&split);
        assert!(summary.contains("yours and allies winter · enemies desert"));
        assert!(summary.contains("Compare scores with the same unit skin."));
        let camo = Report { rendered_unit_skin: Some("builtin:winter".into()), rendered_enemy_unit_skin: Some("builtin:desert".into()), ..Default::default() };
        let summary = report_summary(&camo);
        assert!(summary.contains("yours and allies Winter · enemies Desert"));
        assert!(!summary.contains("builtin:"));
        let heritage = Report { rendered_unit_skin: Some("builtin:heritage".into()), ..Default::default() };
        let summary = report_summary(&heritage);
        assert!(summary.contains("Unit skin: Heritage"));
        assert!(!summary.contains("builtin:"));
        // Older reports saved the selected pack even though the benchmark rendered Reborn.
        let legacy = Report { graphics: ta_config::Graphics { unit_skin: "christmas".into(), ..Default::default() }, ..Default::default() };
        let summary = report_summary(&legacy);
        assert!(summary.contains("Unit skin: Reborn"));
        assert!(!summary.contains("Compare scores with the same unit skin."));
    }
}
