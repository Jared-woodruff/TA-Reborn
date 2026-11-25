//! The Supreme Commander import's wizard: the four imports (the maps, `crate::supcom`; the
//! factions, `crate::supcom_factions`; the music, `crate::supcom_music`; the menus' look,
//! `crate::supcom_menus`) made in one go, one after another, in one view. For a player with none
//! of them yet it goes through the game found, what to make, the import and what it came to; the
//! dialog's tabs offer it for the parts not imported yet, and the Mods page and the tabs offer it
//! again for those an older TA Reborn made (UPDATE ALL). It imports nothing of its own: each
//! part's maps or factions are chosen as its tab would choose them, and its tab's own import is
//! started once the part before it has ended (`LauncherApp::wizard_tick`, from the app's update
//! where the imports are polled), as the tabs' imports wait for each other.
//! `pages/supcom_wizard.rs` draws it, and drives it.

use std::sync::mpsc;

use ta_ui::i18n::n;

use crate::supcom::{Tab, Went};

/// The parts in the order they are made: the quick ones first and the maps last (the game's 54
/// take twenty minutes), so that a STOP while the maps are made keeps everything else, and the
/// maps made so far.
pub const ORDER: [Tab; 4] = [Tab::Factions, Tab::Music, Tab::Menus, Tab::Maps];

/// Why the wizard was opened: the parts it offers, and the step it starts at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// Nothing imported yet: every part, from the game's step.
    #[default]
    First,
    /// Some imported: those not imported yet, from the choice's step.
    Rest,
    /// Those an older TA Reborn made, made again (UPDATE ALL): straight to the import.
    Update,
}

/// The wizard's steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    /// The copy of the game found, or what is needed without it.
    #[default]
    Game,
    /// What to make: a row for each part, ticked, with what it takes.
    Choose,
    /// The parts ticked, made one after another.
    Import,
    /// What they came to.
    Done,
}

impl Step {
    /// The steps a wizard of `kind` goes through.
    pub fn of(kind: Kind) -> &'static [Step] {
        match kind {
            Kind::First => &[Step::Game, Step::Choose, Step::Import, Step::Done],
            Kind::Rest => &[Step::Choose, Step::Import, Step::Done],
            Kind::Update => &[Step::Import, Step::Done],
        }
    }

    /// Its name in the row of steps (marked for translation).
    pub fn label(self) -> &'static str {
        match self {
            // (The game's own name: "THE GAME" is a battle's in some languages, the cheats'.)
            Step::Game => n("FORGED ALLIANCE"),
            Step::Choose => n("WHAT TO MAKE"),
            Step::Import => n("IMPORTING"),
            Step::Done => n("DONE"),
        }
    }
}

/// How a part of the import stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Fate {
    /// Its turn not come yet, or held (why: the game being looked for, a mod being installed).
    Waiting(Option<String>),
    /// Asked about before it starts: another mod of its id is installed (REPLACE AND IMPORT, or
    /// KEEP).
    Asking,
    /// Its import under way.
    Making,
    /// Its import ended: how it went, in its own words, and what it could not make (each thing's
    /// name, and why).
    Ended { went: Went, words: String, failed: Vec<(String, String)> },
    /// Not made: it could not start (why).
    Failed(String),
    /// Not made: STOP was pressed before its turn.
    Left,
}

impl Fate {
    /// Its turn is past.
    pub fn over(&self) -> bool {
        matches!(self, Fate::Ended { .. } | Fate::Failed(_) | Fate::Left)
    }
}

/// A part of the import, and how it stands.
#[derive(Clone, Debug)]
pub struct Part {
    pub tab: Tab,
    pub fate: Fate,
}

/// The wizard: its step, the parts it offers and those ticked, and its import, a part at a time.
#[derive(Default)]
pub struct Wizard {
    /// Shown in the dialog in place of the tabs.
    pub shown: bool,
    pub kind: Kind,
    pub step: Step,
    /// The parts offered at the choice's step, in [`ORDER`].
    pub offered: Vec<Tab>,
    /// Those ticked.
    ticked: Vec<Tab>,
    /// The import: each part ticked, in [`ORDER`], and how it stands.
    pub parts: Vec<Part>,
    /// STOP was pressed: the import ends once the part in hand is made (the maps' import once
    /// the map in hand is).
    pub stopping: bool,
    /// The import ended with the dialog closed: its last step is shown when it is next opened.
    pub unseen: bool,
    /// A capture's (`--show supcom-wizard:<step>`): the step to go on to by itself once the game
    /// is looked through, the import begun (all ticked) for its last two.
    pub auto: Option<Step>,
    /// A capture's: the maps the maps' part makes, by their folders' names (a few: all 54 take
    /// twenty minutes).
    pub maps_only: Option<Vec<String>>,
}

impl Wizard {
    /// Opens it as `kind`, offering the parts of `parts` (made in [`ORDER`]), all of them ticked;
    /// an update's import begun at once.
    pub fn open(&mut self, kind: Kind, parts: &[Tab]) {
        let offered: Vec<Tab> = ORDER.into_iter().filter(|tab| parts.contains(tab)).collect();
        *self = Wizard { shown: true, kind, step: Step::of(kind)[0], ticked: offered.clone(), offered, ..Default::default() };
        if kind == Kind::Update {
            self.begin(self.ticked.clone());
        }
    }

    pub fn is_ticked(&self, tab: Tab) -> bool {
        self.ticked.contains(&tab)
    }

    /// Ticks the part `tab` (`on`), or not.
    pub fn tick(&mut self, tab: Tab, on: bool) {
        self.ticked.retain(|t| *t != tab);
        if on {
            self.ticked.push(tab);
        }
    }

    /// The import begun, of the parts of `parts` (in [`ORDER`]), each waiting its turn.
    pub fn begin(&mut self, parts: Vec<Tab>) {
        self.parts = ORDER.into_iter().filter(|tab| parts.contains(tab)).map(|tab| Part { tab, fate: Fate::Waiting(None) }).collect();
        self.step = Step::Import;
        self.stopping = false;
        self.unseen = false;
    }

    /// Its import is under way: a part waiting its turn, asked about or being made.
    pub fn running(&self) -> bool {
        self.step == Step::Import && self.parts.iter().any(|p| !p.fate.over())
    }

    /// The part in hand: being made, or asked about.
    pub fn in_hand(&self) -> Option<Tab> {
        self.parts.iter().find(|p| matches!(p.fate, Fate::Making | Fate::Asking)).map(|p| p.tab)
    }

    /// The part to start next, none being in hand.
    pub fn next(&self) -> Option<Tab> {
        if self.in_hand().is_some() {
            return None;
        }
        self.parts.iter().find(|p| matches!(p.fate, Fate::Waiting(_))).map(|p| p.tab)
    }

    /// Whether the part `tab` is the wizard's to tell of: its turn not past (the dialog's tabs
    /// and the launcher's notices leave it to the wizard).
    pub fn holds(&self, tab: Tab) -> bool {
        self.running() && self.parts.iter().any(|p| p.tab == tab && !p.fate.over())
    }

    pub fn fate(&self, tab: Tab) -> Option<&Fate> {
        self.parts.iter().find(|p| p.tab == tab).map(|p| &p.fate)
    }

    pub fn set(&mut self, tab: Tab, fate: Fate) {
        if let Some(part) = self.parts.iter_mut().find(|p| p.tab == tab) {
            part.fate = fate;
        }
    }

    /// STOP: the import ends once the part in hand is made.
    pub fn stop(&mut self) {
        if self.running() {
            self.stopping = true;
        }
    }

    /// The parts whose turn has not come are left out (STOP).
    pub fn leave_rest(&mut self) {
        for part in &mut self.parts {
            if matches!(part.fate, Fate::Waiting(_)) {
                part.fate = Fate::Left;
            }
        }
    }

    /// Every part's turn is past: its last step, to be seen when the dialog is next opened where
    /// it is closed (`seen`: it is shown).
    pub fn finish(&mut self, seen: bool) {
        self.step = Step::Done;
        self.unseen = !seen;
    }

    /// How the import went, all its parts told: well where each made all it had to, badly where
    /// none made anything.
    pub fn went(&self) -> Went {
        let wents: Vec<Went> = self.parts.iter().filter_map(|p| match &p.fate {
            Fate::Ended { went, .. } => Some(*went),
            Fate::Failed(_) => Some(Went::Badly),
            Fate::Left | Fate::Waiting(_) | Fate::Asking | Fate::Making => None,
        }).collect();
        if wents.is_empty() || wents.iter().all(|w| *w == Went::Nothing) {
            Went::Nothing
        } else if wents.iter().all(|w| *w == Went::Well) {
            Went::Well
        } else if wents.iter().all(|w| matches!(w, Went::Badly | Went::Nothing)) {
            Went::Badly
        } else {
            Went::Partly
        }
    }

    /// The dialog closed: a wizard whose import is not under way is put away (its last step seen,
    /// or never begun); one under way goes on, told in the footer.
    pub fn close(&mut self) {
        if !self.running() {
            *self = Wizard::default();
        }
    }

    /// Whether a capture may take its picture: its step reached, its import ended (or, with
    /// `TA_SUPCOM_SHOT_IMPORTING=1`, while it is under way).
    pub fn capture_ready(&self) -> bool {
        let during = std::env::var_os("TA_SUPCOM_SHOT_IMPORTING").is_some_and(|v| v == "1");
        self.auto.is_none() && (!self.running() || (during && self.in_hand().is_some()))
    }
}

/// The Mods page's offer of the wizard: once Forged Alliance is found on the computer and none of
/// its imports is made, until the player takes it up or puts it away (kept in the launcher's
/// settings, `supcom.toml`). Never a dialog opened by itself.
#[derive(Default)]
pub struct Offer {
    /// Taken up or put away, this launch or before.
    pub done: bool,
    /// The game looked for, on a worker, once a launch.
    seek: Option<mpsc::Receiver<bool>>,
    /// Whether the game was found.
    pub found: Option<bool>,
}

impl Offer {
    pub fn new(done: bool) -> Self {
        Offer { done, ..Default::default() }
    }

    /// Looks for the game once (on a worker), while the offer may still be made: never in the
    /// launcher's tests, which never look for this computer's copy.
    pub fn seek(&mut self) {
        if self.done || self.found.is_some() || self.seek.is_some() || cfg!(test) {
            return;
        }
        let (send, receive) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("supcom seek".into()).spawn(move || {
            let _ = send.send(!crate::supcom::copies_found().is_empty());
        });
        match spawned {
            Ok(_) => self.seek = Some(receive),
            Err(_) => self.found = Some(false),
        }
    }

    /// The game is being looked for (a capture waits for it).
    pub fn seeking(&self) -> bool {
        self.seek.is_some()
    }

    /// Takes in what the look found.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(seek) = &self.seek else { return };
        match seek.try_recv() {
            Ok(found) => {
                self.found = Some(found);
                self.seek = None;
                ctx.request_repaint();
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.found = Some(false);
                self.seek = None;
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
        }
    }

    /// The game found, or not, as a look would have it: for tests.
    #[cfg(test)]
    pub fn pretend(&mut self, found: bool) {
        self.found = Some(found);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parts are made in their order whatever order they are offered in, all ticked at first;
    /// an update begins at once; the part in hand is the one being made or asked about, and the
    /// next waits for it; STOP leaves those whose turn has not come.
    #[test]
    fn the_parts_go_in_their_order_one_at_a_time() {
        let mut wizard = Wizard::default();
        wizard.open(Kind::First, &[Tab::Maps, Tab::Menus, Tab::Factions, Tab::Music]);
        assert_eq!((wizard.step, wizard.offered.clone()), (Step::Game, ORDER.to_vec()));
        assert!(ORDER.iter().all(|tab| wizard.is_ticked(*tab)) && !wizard.running());
        wizard.tick(Tab::Music, false);
        wizard.begin(vec![Tab::Maps, Tab::Factions, Tab::Menus]);
        assert!(wizard.running());
        assert_eq!(wizard.parts.iter().map(|p| p.tab).collect::<Vec<_>>(), [Tab::Factions, Tab::Menus, Tab::Maps]);
        assert_eq!(wizard.next(), Some(Tab::Factions));
        wizard.set(Tab::Factions, Fate::Making);
        assert_eq!((wizard.in_hand(), wizard.next()), (Some(Tab::Factions), None));
        assert!(wizard.holds(Tab::Factions) && wizard.holds(Tab::Maps) && !wizard.holds(Tab::Music));
        wizard.stop();
        wizard.set(Tab::Factions, Fate::Ended { went: Went::Well, words: "made".into(), failed: Vec::new() });
        wizard.leave_rest();
        assert!(!wizard.running() && wizard.fate(Tab::Maps) == Some(&Fate::Left));
        assert_eq!(wizard.went(), Went::Well);
        wizard.finish(false);
        assert!(wizard.unseen && wizard.step == Step::Done);
        wizard.close();
        assert!(!wizard.shown && wizard.parts.is_empty(), "put away once over");
        // An update: straight to the import, of the parts offered.
        wizard.open(Kind::Update, &[Tab::Music, Tab::Factions]);
        assert_eq!(wizard.step, Step::Import);
        assert_eq!(wizard.parts.iter().map(|p| p.tab).collect::<Vec<_>>(), [Tab::Factions, Tab::Music]);
        wizard.close();
        assert!(wizard.shown && wizard.running(), "under way, it goes on with the dialog closed");
    }

    /// How the import went, all told: well, partly (some parts failed or made only some), badly
    /// (nothing made), or nothing (stopped before anything was made).
    #[test]
    fn the_import_is_told_as_it_went() {
        let ended = |went| Fate::Ended { went, words: String::new(), failed: Vec::new() };
        let went = |fates: Vec<Fate>| {
            let mut wizard = Wizard::default();
            wizard.begin(ORDER.to_vec());
            for (tab, fate) in ORDER.into_iter().zip(fates) {
                wizard.set(tab, fate);
            }
            wizard.went()
        };
        assert_eq!(went(vec![ended(Went::Well), ended(Went::Well), ended(Went::Well), ended(Went::Well)]), Went::Well);
        assert_eq!(went(vec![ended(Went::Well), Fate::Failed("why".into()), ended(Went::Well), Fate::Left]), Went::Partly);
        assert_eq!(went(vec![ended(Went::Well), ended(Went::Partly), Fate::Left, Fate::Left]), Went::Partly);
        assert_eq!(went(vec![Fate::Failed("why".into()), ended(Went::Badly), Fate::Left, Fate::Left]), Went::Badly);
        assert_eq!(went(vec![ended(Went::Nothing), Fate::Left, Fate::Left, Fate::Left]), Went::Nothing);
    }
}
