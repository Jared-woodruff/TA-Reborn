//! The credits' film in the app (`credits_show.rs` is the film itself): the credits' page opening
//! it, each act's world made under a black chapter card (a new battle, as the encyclopaedia's
//! showroom makes one), its scene played, its camera and time of day, its blasts, its names laid
//! in the world; and the page closing it, the menu's own backdrop and music coming back.
//!
//! The film plays to its song ("The War Reborn", `credits_song.rs`): the menu's music steps aside
//! as the page opens, the song begins as the first act's picture comes in, and the film follows
//! the song's clock as the mixer plays it, not its frames: however long a world takes to make, an
//! act comes in where the song is (its battle moved on to it), and what the film lands on the song
//! lands there. On a capture's fixed clock, or with no sound, the film keeps its own time.

use anyhow::{Result, anyhow};
use glam::Vec2;

use super::*;
use crate::credits_show::{Battle, Blast, CARD_FIRST, FADE_OUT, Phase, Show};

/// The act's battle as the film moves it on: the game and the scene playing on it.
struct FilmBattle<'a> {
    game: &'a mut Game,
    run: &'a mut crate::stage::StageRun,
    /// Where the act is on the song (for the log).
    start: f32,
}

impl Battle for FilmBattle<'_> {
    fn step(&mut self, dt: f32, quiet: bool) -> f32 {
        self.run.before_update(self.game);
        self.game.update(dt);
        self.run.after_update(self.game);
        if quiet {
            // (Long past by the time it is shown: its sounds are not heard.)
            self.game.events_out.clear();
        }
        self.run.seconds(self.game)
    }

    fn blast(&mut self, blast: &Blast, heard: bool) {
        log::debug!("credits: a blast due at {:.3} s of the song goes off at {:.3} s{}", self.start + blast.at, self.start + self.run.seconds(self.game), if heard { "" } else { ", unheard" });
        crate::stage::explode(self.game, blast.weapon, blast.size, blast.x, blast.z, heard);
    }
}

/// How loud the battle is under the credits' music (the film behind the title menu's is
/// [`crate::backdrop::SOUND_LEVEL`]): the credits are for watching, and their song leads. (At
/// the default volumes the battle then sits about as loud as the song on the whole, its blasts
/// over it; at 0.8 it was 3 dB over the song.)
const SOUND_LEVEL: f32 = 0.55;
/// How quickly the menu's music steps aside for the film's, and comes back after it (seconds).
const MUSIC_ASIDE: f32 = 1.0;
const MUSIC_BACK: f32 = 0.8;
/// How quickly the song comes in when the film starts it somewhere in its middle (an act begun
/// later, a skip).
const MUSIC_IN_MIDDLE: f32 = 0.35;

impl State {
    /// The credits' page opened starts the film; closed (or the menu left), it ends it.
    pub(super) fn credits_sync(&mut self) {
        let on = self.screen == Screen::Menu && self.menu.page == crate::menu::Page::Credits && self.showroom.is_none();
        if on && self.credits_show.is_none() {
            let mut sections = crate::credits::sections(&self.assets.vfs);
            self.menu.credits_disclaimer = crate::credits::disclaimer(&mut sections);
            let first = std::env::var("TA_CREDITS_ACT").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
            match Show::new(&sections, first) {
                Some(show) => {
                    log::info!(
                        "credits: {} acts, {} cards, {:.0} s on the song arranged to {:.0} s ({} bars, jumps back {:?})",
                        show.acts.len(),
                        show.cards.len(),
                        show.length(),
                        show.score.length,
                        show.score.count,
                        show.score.jumps
                    );
                    let s = &show.score;
                    log::info!("credits: the song's moments: the band at {:.3} s, drops at {:?}, the loudest bar at {:.3} s, the flash at {:.3} s, the stop at {:.3} s", s.band, s.drops, s.loudest, s.flash, s.stop);
                    for (i, a) in show.acts.iter().enumerate() {
                        log::info!("credits: act {i} ({}) from {:.2} s to {:.2} s, {} cards{}", show.scenes[a.scene].id, a.start, a.end(), a.slots.len(), a.drop.map_or(String::new(), |d| format!(", on a drop at {:.2} s", a.start + d)));
                    }
                    // The menu's music steps aside for the film's.
                    self.audio.hush_music(MUSIC_ASIDE);
                    self.credits_show = Some(show);
                }
                None => {
                    // (No scene to play: the page has nothing to show: back to the title.)
                    log::warn!("credits: no scene could be read");
                    self.menu.page = crate::menu::Page::Title;
                }
            }
        } else if !on && let Some(show) = self.credits_show.take() {
            self.world_text.clear();
            self.renderer.post.settings.dof = None;
            self.camera.view_offset = Vec2::ZERO;
            // The film's music goes; the menu's comes back.
            self.audio.end_score(MUSIC_BACK);
            // (The menu's own battle comes back, unless the film never took its world away.)
            if show.world.is_some() {
                self.pending = Some(Transition::RestoreMenuBackdrop);
            }
        }
        if std::mem::take(&mut self.menu.credits_restart)
            && let Some(show) = self.credits_show.as_mut()
        {
            show.restart();
            self.audio.hush_music(0.2);
        }
        if std::mem::take(&mut self.menu.credits_skip)
            && let Some(show) = self.credits_show.as_mut()
            && show.skip()
        {
            // (The song comes back at the next act's place in it, as its picture comes in.)
            self.audio.hush_music(FADE_OUT);
        }
    }

    /// Whether the film keeps its own time: a capture's fixed clock (the film is in step with its
    /// frames, its sound written down on that clock), or no sound to follow.
    fn credits_free(&self) -> bool {
        self.capture_dt().is_some() || self.audio.is_silent()
    }

    /// What the film asks of the frame's update: its acts begun, played and cut, on the song's clock.
    pub(super) fn credits_update(&mut self, dt: f32) {
        let Some(mut show) = self.credits_show.take() else { return };
        show.frames += 1;
        show.calm = self.settings.gameplay.reduced_flashes;
        let free = self.credits_free();
        let music = self.audio.score_position();
        show.follow(music, dt, free);
        if show.frames % 120 == 0 && show.scored {
            log::debug!("credits: the film at {:.3} s of its song, the music at {}", show.song, music.map_or("-".to_string(), |m| format!("{m:.3} s")));
        }
        match show.phase {
            Phase::Chapter => {
                show.since += dt;
                // (The card is on the screen before the world is made: it takes a few seconds, the
                // screen standing still. The card's time starts again after it.)
                if !show.ready && show.since >= CARD_FIRST && show.frames >= 3 {
                    match self.credits_begin_act(&mut show) {
                        Ok(()) => {
                            show.ready = true;
                            show.since = 0.0;
                        }
                        Err(e) => {
                            log::warn!("credits: act {}: {e:#}", show.act);
                            // (Skipped: the next, or the end.)
                            show.next_act();
                            self.credits_show = Some(show);
                            return;
                        }
                    }
                }
                if show.picture_due() {
                    if !show.scored {
                        // The film's music begins with the picture: the song from the act's place
                        // in it (its first note for the first act).
                        let from = show.plan().start as f64;
                        let fade = if from > 0.0 { MUSIC_IN_MIDDLE } else { 0.0 };
                        let arrangement = show.score.arrangement.clone();
                        let path = show.score.path.clone();
                        show.music_begins(free);
                        if !self.audio.play_score(&path, arrangement, from, fade) {
                            // (No song: the menu's music again, the film on its own time.)
                            log::warn!("credits: the song {path} cannot be played");
                            self.audio.end_score(0.0);
                            show.waiting = None;
                        }
                        log::info!("credits: the song from {from:.2} s with act {} (frame {})", show.act, self.frames);
                    } else {
                        log::info!("credits: act {} comes in at {:.3} s of the song (due at {:.3} s; frame {})", show.act, show.song, show.plan().start, self.frames);
                    }
                    show.phase = Phase::Play;
                    show.since = 0.0;
                    show.shown = 0.0;
                }
            }
            Phase::Play | Phase::Out => {
                // The battle moved on to where the song is (all at once after a world was made
                // late, a quarter of a second at a time), its blasts going off on their own
                // times among the steps.
                if let Some(mut run) = show.run.take() {
                    show.clock = run.seconds(&self.game);
                    let to = if show.waiting.is_some() { show.clock } else { show.act_clock() };
                    let start = show.plan().start;
                    show.catch_up(to, &mut FilmBattle { game: &mut self.game, run: &mut run, start });
                    show.run = Some(run);
                }
                show.shown += dt;
                if show.phase == Phase::Play && show.clock >= show.plan().length {
                    if show.scene().finale() {
                        // (The song stops dead: the picture is cut to black with it.)
                        show.next_act();
                    } else {
                        show.phase = Phase::Out;
                        show.since = 0.0;
                    }
                } else if show.phase == Phase::Out {
                    show.since += dt;
                    if show.since >= FADE_OUT {
                        show.next_act();
                    }
                }
            }
            Phase::End => show.since += dt,
        }
        self.credits_show = Some(show);
    }

    /// The film's camera this frame, if an act plays: its lens's focus.
    pub(super) fn credits_camera(&mut self) -> Option<f32> {
        let show = self.credits_show.as_mut()?;
        if !matches!(show.phase, Phase::Play | Phase::Out) {
            return None;
        }
        let mut clock = show.run.as_ref()?.seconds(&self.game).max(0.0);
        if self.settings.gameplay.reduced_motion {
            clock = show.held_clock(clock);
        }
        let view = show.run.as_mut()?.free_view(&self.game, clock)?;
        let camera = &mut self.camera;
        camera.view_offset = Vec2::ZERO;
        camera.fov_y = view.fov;
        camera.roll = view.roll;
        camera.near = Some(view.near);
        camera.place(view.eye, view.yaw, view.pitch, view.distance);
        Some(view.distance)
    }

    /// The time of day the act has now, and where its moon is.
    pub(super) fn credits_hours(&self) -> Option<f32> {
        let run = self.credits_show.as_ref()?.run.as_ref()?;
        run.hours(run.seconds(&self.game).max(0.0))
    }

    /// Whether a film is playing (the battle behind is the film's).
    pub(super) fn credits_playing(&self) -> bool {
        self.credits_show.as_ref().is_some_and(|s| s.world.is_some())
    }

    pub(super) fn credits_sound_level(&self) -> Option<f32> {
        self.credits_show.is_some().then_some(SOUND_LEVEL)
    }

    /// Makes the world of the act (a new battle: its scene's map, nobody's eyes on it), puts its
    /// scene on it and its cards in it, from the first second.
    fn credits_begin_act(&mut self, show: &mut Show) -> Result<()> {
        let act = show.plan().clone();
        let scene = show.scene().clone();
        let ending = show.ending();
        let stage = crate::credits_show::build_stage(&scene, &act, ending)?;
        let world = scene.stage.map.clone().ok_or_else(|| anyhow!("the scene names no map"))?;
        let mut setup = stage.setup(Some(&world), &self.assets)?;
        setup.local_player = None;
        let started = Instant::now();
        // The menu's battle (its film) goes; so does the last act's world.
        self.menu_film = None;
        self.audio.reset_scene();
        self.renderer.clear_particles();
        self.game = Game::new(&self.assets, &mut self.renderer, &setup)?;
        self.game.tally.enabled = false;
        let run = crate::stage::StageRun::begin(&stage, &mut self.game);
        let start = crate::scene::game_seconds(self.game.sim.tick, 0.0);
        self.game.effects.scene_weather(stage.rain.unwrap_or(false), start as f32);
        self.renderer.lighting.moon_dir = stage.moon_dir();
        // The cards, laid out in the world to come at their times.
        self.world_text.clear();
        let viewport = self.camera.viewport;
        let aspect = (viewport.x / viewport.y.max(1.0)).clamp(1.2, 2.4);
        let beat = (show.score.bar / 4.0) as f32;
        show.blasts = crate::credits_show::place_texts(&mut self.world_text, &mut self.renderer, &show.cards, &scene, &act, start as f32, aspect, beat, ending);
        show.start = start as f32;
        show.run = Some(run);
        show.stage = Some(stage);
        show.world = Some(world.clone());
        show.clock = 0.0;
        log::info!(
            "credits: act {} ({}) on {world} made in {:.1} s: {} cards over {:.0} s, {} blasts; the song at {:.2} s, the act's picture due at {:.2} s",
            show.act,
            scene.act.title,
            started.elapsed().as_secs_f32(),
            act.slots.len(),
            act.length,
            show.blasts.len(),
            show.song,
            act.start
        );
        Ok(())
    }
}
