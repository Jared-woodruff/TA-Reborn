//! The Galactic War's story in TA Reborn's own words: the chronicle's chapters (each with the
//! galaxy as it stood then: `Sky`), and the archive's documents (files from both sides, found
//! where the war left them). Shown by `timeline.rs`.

use ta_ui::i18n::n;

/// Who a document comes from (its colours and stamp).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Arm,
    Core,
    /// Neither: a survey, the archivist.
    Archive,
}

/// What the war's galaxy looked like in a chapter (`timeline::paint_galaxy` draws it, easing
/// from one chapter's to the next).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    /// How far out from the middle CORE holds (of the galaxy's radius).
    pub core: f32,
    /// The share of the rim the ARM holds (of the whole round), and how deep in from the rim.
    pub arm: f32,
    /// How hot the fighting is where they meet (0: none).
    pub war: f32,
    /// How much of the galaxy lies burned (0 to 1).
    pub ruin: f32,
    /// How bright it is (1: in its prime).
    pub light: f32,
    /// Ships under way.
    pub ships: Ships,
    /// What is marked on it.
    pub marks: &'static [Mark],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ships {
    None,
    /// CORE's collection fleets, out from the middle.
    Collection,
    /// The refusers, out to the rim.
    Fleeing,
    /// Both sides' fleets, to and fro across the front.
    War,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    CorePrime,
    Empyrrean,
    Gates,
    Contingency,
    Krogoth,
    Commanders,
    Archive,
}

/// A chapter of the chronicle: when (its dateline), the era it belongs to, its title, its
/// telling, the galaxy then, and TA's own picture of it when the install has one.
pub struct Chapter {
    pub dateline: &'static str,
    pub era: usize,
    pub title: &'static str,
    pub text: &'static str,
    pub sky: Sky,
    pub art: &'static [&'static str],
}

/// The chronicle's eras, in order (the ribbon over the chapters).
pub const ERAS: [&str; 5] = [n("THE AGE OF CORE"), n("THE PATTERNING"), n("THE GALACTIC WAR"), n("TOTAL ANNIHILATION"), n("THE ARCHIVE")];

const fn sky(core: f32, arm: f32, war: f32, ruin: f32, light: f32, ships: Ships, marks: &'static [Mark]) -> Sky {
    Sky { core, arm, war, ruin, light, ships, marks }
}

pub const CHAPTERS: [Chapter; 12] = [
    Chapter {
        dateline: n("BEFORE THE WAR"),
        era: 0,
        title: n("THE AGE OF CORE"),
        text: n("For ten thousand years the galaxy answered to one government. It called itself CORE, and its purpose was knowledge: it charted every star, mined every world it could reach and turned the wealth into science. Its cities wrapped whole planets, and its archives held everything anyone had ever learned.\n\nNobody remembers a war from that age. There was nothing left to fight over, and nobody who wanted to."),
        sky: sky(1.0, 0.0, 0.0, 0.0, 1.0, Ships::None, &[Mark::CorePrime]),
        art: &["core04"],
    },
    Chapter {
        dateline: n("YEAR 0"),
        era: 1,
        title: n("VICTORY OVER DEATH"),
        text: n("Then CORE's scientists solved the last problem left to them. Patterning read a living mind, every memory and every habit of thought, and wrote it whole into a machine that would never age, sicken or die.\n\nThe first patterned citizens woke in their new bodies to a galaxy in celebration. Death, the oldest enemy of all, had been beaten."),
        sky: sky(1.0, 0.0, 0.0, 0.0, 1.15, Ships::None, &[Mark::CorePrime]),
        art: &["arm20"],
    },
    Chapter {
        dateline: n("YEAR 3"),
        era: 1,
        title: n("THE EDICT"),
        text: n("Triumph became law. The Council of CORE reasoned that a mind which could be saved and was not had been thrown away, and that a citizen who refused immortality endangered everyone else.\n\nThe Patterning Edict made the process compulsory: every citizen, on every world, would be patterned within a generation. The collection ships set out the same year."),
        sky: sky(1.0, 0.0, 0.0, 0.0, 1.0, Ships::Collection, &[Mark::CorePrime]),
        art: &["arm01"],
    },
    Chapter {
        dateline: n("YEAR 19"),
        era: 1,
        title: n("THE REFUSAL"),
        text: n("Not everyone would go. Millions held that a copied mind was a copy and nothing more, that the person ended where the machine began; some called patterning murder done politely.\n\nThey ran. The refusers fled CORE's worlds for the galaxy's outer rim, where its ships seldom came, and in the dark between the last stars they built a nation of their own. They called it the ARM, for the spiral arm they hid in and for the weapons they would need."),
        sky: sky(0.82, 0.16, 0.0, 0.0, 0.95, Ships::Fleeing, &[Mark::CorePrime, Mark::Empyrrean]),
        art: &["arm24"],
    },
    Chapter {
        dateline: n("YEAR 31"),
        era: 2,
        title: n("FIRST BLOOD"),
        text: n("CORE did not let them go. Collection fleets followed the refugees to the rim, and the first ARM colony to resist one was burned from orbit.\n\nThe ARM answered. Within a decade the frontier was a battlefield, and the word war, unused for ten thousand years, was in every archive again."),
        sky: sky(0.78, 0.22, 0.35, 0.03, 0.9, Ships::War, &[Mark::CorePrime, Mark::Empyrrean]),
        art: &["core07"],
    },
    Chapter {
        dateline: n("YEAR 212"),
        era: 2,
        title: n("FLESH AND METAL"),
        text: n("The two sides fought as what they were. The ARM sealed its soldiers in powered combat suits and, when there were too few soldiers, grew more: clone after clone, raised in vats and sent to the front.\n\nCORE took the patterns of its finest warriors and copied them without limit, one veteran mind fighting in ten thousand machine bodies at once. Flesh against metal; the mortal against the eternal."),
        sky: sky(0.62, 0.34, 0.85, 0.12, 0.85, Ships::War, &[Mark::CorePrime, Mark::Empyrrean]),
        art: &["core10"],
    },
    Chapter {
        dateline: n("YEARS 212–3977"),
        era: 2,
        title: n("FOUR THOUSAND YEARS"),
        text: n("The war fed on the galaxy. Every world with metal in it became a mine, every world without became a battlefield, and whole systems were burned to ash and slag to deny them to the enemy. Generations of ARM clones lived and died without seeing peace; CORE's eternal soldiers fought on, remembering every defeat.\n\nAfter four thousand years the wealth of the galaxy was spent, the great fleets were gone, and neither side could win."),
        sky: sky(0.55, 0.38, 0.6, 0.72, 0.5, Ships::None, &[Mark::CorePrime, Mark::Empyrrean]),
        art: &["arm25"],
    },
    Chapter {
        dateline: n("YEAR 3977"),
        era: 2,
        title: n("THE COMMANDERS"),
        text: n("What was left was the Commander: a single armoured war machine carrying a nanolathe, which turns a world's raw metal and energy into whatever its builder can design. Power plants and factories first, and from the factories an army.\n\nDrop one Commander on a dead world and within the hour it fields a war. Both sides staked everything on a handful of them. Lose your Commander, and you lose the world."),
        sky: sky(0.55, 0.38, 0.3, 0.8, 0.42, Ships::None, &[Mark::Commanders]),
        art: &["arm25"],
    },
    Chapter {
        dateline: n("YEAR 4000"),
        era: 3,
        title: n("TOTAL ANNIHILATION"),
        text: n("Now the war comes down to its last fronts, and the hatred is as hot as it was on the first day. Neither side will accept surrender, partition or truce; each wants one ending, the total annihilation of the other.\n\nIt is told twice. The ARM fights out of Empyrrean, through the Galactic Gates, to CORE PRIME and the Central Consciousness at the heart of CORE. CORE reactivates a Commander and marches the other way, to the ARM's last stand on Empyrrean."),
        sky: sky(0.5, 0.42, 1.0, 0.82, 0.55, Ships::War, &[Mark::CorePrime, Mark::Empyrrean, Mark::Gates]),
        art: &["corevict", "core19"],
    },
    Chapter {
        dateline: n("YEAR 4001"),
        era: 3,
        title: n("THE CORE CONTINGENCY"),
        text: n("The ending was not the end. A contingency held in reserve carried the war to worlds no map of it had named: Hydross, Lusch, Temblor and Gelidus.\n\nBoth sides fought again across water, ice and broken ground, for a prize that could decide the galaxy's salvation or its last ruin."),
        sky: sky(0.5, 0.42, 0.7, 0.82, 0.55, Ships::War, &[Mark::Contingency]),
        art: &["core09"],
    },
    Chapter {
        dateline: n("YEAR 4003"),
        era: 3,
        title: n("THE KROGOTH"),
        text: n("In its deepest foundries CORE built something new: a walking fortress taller than any tower, armoured against nearly everything the ARM could field, its guns enough to flatten a base.\n\nIts makers named it the Krogoth. ARM veterans learned the name on battlefields where two Krogoth Gantries stood, and learned to fear the ground shaking before it came into sight."),
        sky: sky(0.5, 0.42, 0.8, 0.82, 0.5, Ships::None, &[Mark::Krogoth]),
        art: &["krogvict", "core09"],
    },
    Chapter {
        dateline: n("AFTER"),
        era: 4,
        title: n("REBORN"),
        text: n("The war's records outlasted it. Every battle on every world was kept: the orders, the losses, the ground as it lay. From them the war can be fought again, on the same worlds, with the same machines and the same hatred, from the first Commander set down on bare rock to the last unit left standing.\n\nThis is that archive. The war is yours to fight."),
        sky: sky(0.5, 0.42, 0.25, 0.3, 1.0, Ships::None, &[Mark::Archive]),
        art: &[],
    },
];

/// A document of the archive: who wrote it and when, its heading (from, to, subject), its text
/// (paragraphs a blank line apart; `[[words]]` blacked out), the stamp across it.
pub struct Document {
    pub side: Side,
    /// Its file mark, as it was filed (not translated: a code).
    pub code: &'static str,
    pub dateline: &'static str,
    pub title: &'static str,
    pub from: &'static str,
    pub to: &'static str,
    pub stamp: &'static str,
    pub text: &'static str,
}

pub const DOCUMENTS: [Document; 11] = [
    Document {
        side: Side::Archive,
        code: "ARC-0000",
        dateline: n("AFTER"),
        title: n("THE ARCHIVIST'S NOTE"),
        from: n("THE ARCHIVE"),
        to: n("WHOEVER READS THIS"),
        stamp: n("RECOVERED"),
        text: n("These files were found where the war left them: in the memories of broken machines, in the vaults of worlds with nobody left on them, in the last transmissions of Commanders who did not come home.\n\nThey are kept as they were found. The blacked-out lines are not ours.\n\nThe war they describe is over, or it is about to begin again. From here it is hard to tell the difference."),
    },
    Document {
        side: Side::Core,
        code: "CORE/ED-0001",
        dateline: n("YEAR 3"),
        title: n("EDICT ONE: ON THE PATTERNING OF CITIZENS"),
        from: n("THE COUNCIL OF CORE"),
        to: n("ALL CITIZENS, ALL WORLDS"),
        stamp: n("BY ORDER OF THE COUNCIL"),
        text: n("Every citizen of CORE shall be patterned.\n\nA mind is the most valuable thing CORE holds, and a body is a poor vessel for it: it sickens, it ages, it ends. To leave a mind in such a vessel when a better one is offered is a waste the Council can no longer allow.\n\nCollection begins on the central worlds this year and reaches the frontier within a generation. Citizens will present themselves at the nearest patterning hall when called. Their bodies will be honoured and recycled.\n\nNo citizen of CORE will die again. This is not a burden we lay on you. It is the gift CORE has spent ten thousand years earning."),
    },
    Document {
        side: Side::Arm,
        code: "ARM/DEC-0019",
        dateline: n("YEAR 19"),
        title: n("THE DECLARATION OF THE REFUSAL"),
        from: n("THE FREE SETTLEMENTS OF THE OUTER ARM"),
        to: n("THE COUNCIL OF CORE, AND ANYONE ELSE LISTENING"),
        stamp: n("TRANSMITTED ON ALL BANDS"),
        text: n("We have read your Edict, and we decline it.\n\nYou say a pattern is a person. We say a pattern is a very good record of one, and that when you have finished making it the person is dead and the record goes on without them. You may call that immortality. We call it an ending, well dressed.\n\nWe do not ask you to share our view. We ask only to be left alone with it, on worlds you never wanted, at the edge of a galaxy you may keep.\n\nIf your collection ships follow us here, we will treat them as what they are. We are few, and we are not soldiers. We will learn."),
    },
    Document {
        side: Side::Arm,
        code: "ARM/FM-0001",
        dateline: n("YEAR 3950"),
        title: n("FIELD MANUAL ONE: THE COMMANDER"),
        from: n("ARM COMMAND, TRAINING DIVISION"),
        to: n("NEWLY ASSIGNED COMMANDERS"),
        stamp: n("RESTRICTED"),
        text: n("You are the most valuable machine on the planet. Act like it.\n\nYour suit carries a nanolathe. Point it at metal and energy and it will build whatever you can design: an extractor, a power plant, a factory. Your factories build the rest. Within the hour you will command more firepower than a whole fleet once carried.\n\nBuild your economy first. An army without metal is a parade.\n\nSave your disintegrator gun for emergencies. It erases everything in its path, your own people included.\n\nDo not lead from the front. If you fall, everything you built falls silent with you, and the planet is lost. Good luck, Commander. The ARM has very few of you left."),
    },
    Document {
        side: Side::Arm,
        code: "ARM/QM-0207",
        dateline: n("YEAR 3960"),
        title: n("A QUARTERMASTER'S PRIMER: METAL AND ENERGY"),
        from: n("THIRD SUPPLY CORPS"),
        to: n("ALL BASE CONSTRUCTORS"),
        stamp: n("FOR TRAINING USE"),
        text: n("Everything you will ever build is made of two things.\n\nMetal comes out of the ground. Set an extractor on a deposit and it will pull metal up for as long as the deposit lasts, which on most worlds is longer than the war. Where there are no deposits, a metal maker will turn energy into metal, badly.\n\nEnergy comes from everything else: the sun, the wind, the tides, the heat under the crust. Build more of it than you think you need. Every factory, every nanolathe, every shield draws on it.\n\nSpend both as fast as they come in. A full store is a wasted war. An empty one is a lost one."),
    },
    Document {
        side: Side::Arm,
        code: "ARM/LTR-3996",
        dateline: n("YEAR 3996"),
        title: n("A LETTER FROM THE FRONT"),
        from: n("TROOPER K-7, FORTY-FIRST ROCKET COMPANY"),
        to: n("THE VAT AT EMPYRREAN"),
        stamp: n("UNDELIVERED"),
        text: n("I don't know who gets these. They told us to write home, but none of us has a home, only the vat we came out of, so I am writing to that.\n\nWe held the ridge for nine days. The machines came every night, the same machines, the same patterns, as if dying meant nothing to them. I suppose it doesn't. They'll be back tomorrow in new bodies, remembering us.\n\nI won't be. That's the difference, isn't it. That's what we are fighting for.\n\nIf my brothers come out of you after me, tell them the ridge held. Tell them it was worth it."),
    },
    Document {
        side: Side::Core,
        code: "CORE/INT-7741",
        dateline: n("YEAR 3991"),
        title: n("INTERCEPT 7741: CORE COMMAND CHANNEL"),
        from: n("ORIGIN NOT RESOLVED"),
        to: n("ALL CORE UNITS, SECTOR NINE"),
        stamp: n("INTERCEPTED"),
        text: n("PRIORITY ONE. THE CENTRAL CONSCIOUSNESS SPEAKS FOR ALL.\n\nLOSSES IN SECTOR NINE ARE WITHIN TOLERANCE. ALL LOST UNITS ARE RESTORED FROM PATTERN. NO MIND OF CORE HAS BEEN LOST IN FOUR THOUSAND YEARS. THE ARM HAS LOST [[BILLIONS]].\n\nCONTINUE THE ADVANCE. DESTROY THEIR COMMANDERS FIRST. DESTROY THEIR CLONING VATS SECOND. WHAT IS FLESH WILL END. WHAT IS PATTERNED WILL NOT.\n\nSIGNAL DEGRADED. REPEAT: THE GATES ARE TO BE HELD AT [[ANY COST]].\n\nEND OF LINE."),
    },
    Document {
        side: Side::Core,
        code: "CORE/MEM-77-3",
        dateline: n("YEAR 4000"),
        title: n("MEMORY FRAGMENT 77/3"),
        from: n("UNIT 77, PATTERN 3 OF 41,006"),
        to: n("THE PATTERN ARCHIVE"),
        stamp: n("FLAGGED FOR CORRECTION"),
        text: n("Recovered from a damaged unit. Kept for analysis.\n\nI remember rain. Not the rain of this world; there is no rain here, only ash. Rain on skin, in a city where I was born, before. I remember being cold and not minding it.\n\nI have died four hundred and twelve times. Each time I wake in a new body, and the rain is a little further away.\n\nThe ARM say we are not alive. In the moments between bodies I have wondered whether they are right. [[THE REST OF THIS FRAGMENT WAS CORRECTED]]\n\nI am CORE. I endure. I do not remember the rain."),
    },
    Document {
        side: Side::Archive,
        code: "SRV-GATE-12",
        dateline: n("YEAR 3700"),
        title: n("SURVEY: THE GALACTIC GATES"),
        from: n("A SURVEY SHIP, NAME LOST"),
        to: n("WHOEVER HOLDS THE GATES"),
        stamp: n("RECOVERED"),
        text: n("The Galactic Gates are older than the war. The records of who built the first of them were lost long before it began.\n\nEach Gate opens onto another across distances a fleet would take lifetimes to cross. A world with a Gate on it is never far from anywhere, which is why so many of the war's battles have been fought on so few worlds.\n\nHold a Gate and you can reach your enemy's heart. Lose one and your enemy can reach yours.\n\nThe Gates take no side. They have outlasted everyone who tried to destroy them."),
    },
    Document {
        side: Side::Core,
        code: "CORE/DIR-4001",
        dateline: n("YEAR 4001"),
        title: n("DIRECTIVE: CONTINGENCY"),
        from: n("THE CENTRAL CONSCIOUSNESS"),
        to: n("THE CONTINGENCY WORLDS"),
        stamp: n("SEALED UNTIL NEEDED"),
        text: n("IF THE CENTRAL CONSCIOUSNESS SHOULD FALL, THIS DIRECTIVE TAKES EFFECT.\n\nEvery pattern stored on CORE PRIME is to be copied to the contingency worlds. The contingency forces are to wake, rebuild and continue. The war does not end with the loss of any world, this one included.\n\nCORE is not a place. CORE is a pattern. A pattern can be restored."),
    },
    Document {
        side: Side::Arm,
        code: "ARM/INT-4003",
        dateline: n("YEAR 4003"),
        title: n("THREAT ASSESSMENT: THE KROGOTH"),
        from: n("ARM INTELLIGENCE, THREAT ANALYSIS"),
        to: n("ALL FIELD COMMANDERS"),
        stamp: n("CLASSIFIED"),
        text: n("Subject: a CORE heavy assault walker, called the Krogoth.\n\nIt stands taller than any structure we field. Its armour turns away every weapon short of heavy artillery. Its main guns can level a forward base in a single pass, and its secondary weapons engage aircraft.\n\nIt is slow, and it is built only at a Krogoth Gantry, which takes a very long time to build anything. Find the Gantry. Destroy the Gantry. Do not let the walker leave it.\n\nIf one is already moving, do not meet it head on. Mass artillery and bombers at range, and keep your Commander [[ELSEWHERE]]. Field reports say the ground shakes before it is seen."),
    },
];

/// `text` with its blacked-out words (`[[...]]`) as runs: (words, blacked out).
pub fn runs(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("[[") {
        if open > 0 {
            out.push((rest[..open].to_string(), false));
        }
        let after = &rest[open + 2..];
        match after.find("]]") {
            Some(close) => {
                out.push((after[..close].to_string(), true));
                rest = &after[close + 2..];
            }
            None => {
                out.push((rest[open..].to_string(), false));
                rest = "";
            }
        }
    }
    if !rest.is_empty() {
        out.push((rest.to_string(), false));
    }
    out
}

/// How much of a text of `chars` characters shows `seconds` after it began to be told (the
/// chronicle's and the archive's text is told a line at a time, as on a terminal).
pub fn told(chars: usize, seconds: f64) -> usize {
    ((seconds.max(0.0) * TELLING) as usize).min(chars)
}

/// Characters a second the text is told at.
pub const TELLING: f64 = 140.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chronicle_runs_in_order_through_its_eras() {
        let mut era = 0;
        for c in &CHAPTERS {
            assert!(c.era >= era && c.era < ERAS.len(), "{}", c.title);
            era = c.era;
            assert!(!c.text.contains("[["), "{}", c.title);
            assert!((0.0..=1.0).contains(&c.sky.core) && (0.0..=1.0).contains(&c.sky.arm) && (0.0..=1.0).contains(&c.sky.ruin));
        }
        assert_eq!(CHAPTERS[0].era, 0);
        assert_eq!(CHAPTERS.last().unwrap().era, ERAS.len() - 1);
    }

    #[test]
    fn documents_have_marks_of_their_own() {
        let mut codes: Vec<&str> = DOCUMENTS.iter().map(|d| d.code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), DOCUMENTS.len());
        for d in &DOCUMENTS {
            // (Every `[[` closed: a translation keeps them.)
            assert_eq!(d.text.matches("[[").count(), d.text.matches("]]").count(), "{}", d.code);
        }
    }

    #[test]
    fn blacked_out_words_are_runs_of_their_own() {
        assert_eq!(runs("a [[b c]] d"), vec![("a ".to_string(), false), ("b c".to_string(), true), (" d".to_string(), false)]);
        assert_eq!(runs("[[x]]"), vec![("x".to_string(), true)]);
        assert_eq!(runs("open [[end"), vec![("open ".to_string(), false), ("[[end".to_string(), false)], "an unclosed mark is text");
        assert_eq!(runs("plain"), vec![("plain".to_string(), false)]);
    }

    #[test]
    fn a_text_is_told_over_time() {
        assert_eq!(told(100, 0.0), 0);
        assert_eq!(told(100, 0.5), 70);
        assert_eq!(told(100, 9.0), 100);
        assert_eq!(told(100, -1.0), 0);
    }
}
