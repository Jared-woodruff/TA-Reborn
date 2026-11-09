//! Computer players that talk. In single-player battles the enemy's computer players greet
//! you, answer what you say (who they are, how they are doing, how many units they have),
//! and taunt you all battle long: at first contact, over your buildings and theirs, your costly
//! units and theirs, their experimental units and yours, units captured, your economy running
//! dry, your Commander under fire, the armies' sizes, the minutes gone by, their last stand;
//! they cry foul at cheats and take defeat with more or less grace. Each is a persona, a name
//! and a title, who remembers the player from battles before (`rivals.rs`): a rematch has its
//! own greeting. Their lines come in as transmissions (`hud/comms.rs`), the link established
//! while they "type", and stay on the chat. Only this computer hears them; nothing here
//! touches the battle.

use ta_sim::{Locomotion, Sim, SimEvent};
use ta_ui::i18n::{n, t, tf};

use crate::chat::ChatLine;
use crate::rivals::{Rival, Rivals};

/// Seconds a computer player keeps quiet after speaking (answers to the player excepted).
const QUIET: f32 = 20.0;
/// Seconds between looks at how the battle goes.
const LOOK: f32 = 20.0;
/// Units lost (or killed) between two looks that make a battle worth a remark.
const BATTLE: u64 = 10;
/// Seconds of silence after which someone may say something.
const IDLE: f32 = 180.0;
/// Seconds between two remarks of a kind that keeps coming up (buildings lost, a stalling
/// economy, the armies' sizes, the Commander under fire).
const AGAIN: f32 = 150.0;
/// Metal a unit costs to count as an experimental (the Krogoth, the Big Bertha...), and a
/// mobile unit to be worth a remark when it is lost (a Goliath, a Bulldog...).
const EXPERIMENTAL: f32 = 4000.0;
const COSTLY: f32 = 700.0;
/// The minutes that get a remark.
const MILESTONES: [u32; 6] = [10, 20, 30, 45, 60, 90];

/// A computer player's manner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Temper {
    /// An Arm officer: correct and dry.
    Proud,
    /// A Core intelligence: cold and clinical.
    Cold,
    /// A mercenary sort: loud and cheeky.
    Brash,
}

/// What prompts a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Moment {
    Greeting,
    Hello,
    GoodGame,
    Taunted,
    Surrender,
    Question,
    Sorry,
    Chat,
    /// Its own Commander was destroyed.
    LostCommander,
    /// The player's Commander was destroyed.
    KilledCommander,
    /// It lost the game.
    Defeated,
    /// The player lost the game.
    Won,
    /// The player launched a nuclear missile.
    NukeIncoming,
    /// It launched one.
    NukeLaunched,
    BattleLost,
    BattleWon,
    Idle,
    Cheat,
    /// The first fight between it and the player.
    FirstContact,
    /// It destroyed one of the player's buildings.
    TookBuilding,
    /// The player destroyed one of its factories or other costly buildings.
    LostBuilding,
    /// It finished an experimental ({unit}); the player did.
    Experimental,
    EnemyExperimental,
    /// The player's energy or metal ran dry.
    Stalling,
    /// {minutes} minutes have gone by.
    Milestone,
    /// Its army is twice the player's; the player's twice its.
    Outnumber,
    Outnumbered,
    /// The player's Commander is under its fire, badly hurt.
    CommanderHit,
    /// One of its units ({unit}, costly) destroyed one of the player's; the player destroyed
    /// one of its.
    KilledUnit,
    LostUnit,
    /// The player destroyed its experimental ({unit}); it destroyed the player's.
    KilledExperimental,
    ExperimentalDown,
    /// The player captured one of its units ({unit}).
    Captured,
    /// Its factories are all gone, and it fights on.
    LastStand,
    /// Its greeting to a player it has met before: who won the last battle, the player or it.
    RematchWon,
    RematchLost,
    /// The player asked who it is ({name}), where it is, how it is doing (winning or not),
    /// what it is building, how many units it has ({units}); offered a truce; paid a
    /// compliment; asked for help.
    AskWho,
    AskWhere,
    AskHowWinning,
    AskHowLosing,
    AskBuild,
    AskCount,
    Truce,
    Compliment,
    Help,
}

const NAMES_PROUD: &[&str] = &["Marshal Hale", "Colonel Voss", "Major Arden", "Captain Reyes", "General Sato", "Commodore Lind", "Brigadier Okafor", "Colonel Brandt"];
const NAMES_COLD: &[&str] = &["Overseer Kaan", "Directive Nine", "Prime Malgrim", "Cortex Theta", "Arbiter Vex", "Unit Zero", "Subroutine Aleph", "Warden Hex"];
const NAMES_BRASH: &[&str] = &["Rook", "Blitz", "Crusher Mags", "Sparks", "Duke", "Tinker", "Scrapjaw", "Boom Lacy"];

/// Every persona as a rival the player beat last time (`won`) or lost to: captures'
/// (`TA_RIVALS=won|lost`) rematches.
pub fn every_rival(won: bool) -> Rivals {
    let mut rivals = Rivals::default();
    let names: Vec<String> = NAMES_PROUD.iter().chain(NAMES_COLD).chain(NAMES_BRASH).map(|n| n.to_string()).collect();
    rivals.record(&names, won, 0);
    rivals
}

impl Temper {
    /// What a persona of this manner is: the title under its name on its transmissions.
    fn title(self) -> &'static str {
        match self {
            Temper::Proud => n("FIELD COMMANDER"),
            Temper::Cold => n("MACHINE INTELLIGENCE"),
            Temper::Brash => n("MERCENARY"),
        }
    }
}

fn lines(moment: Moment, temper: Temper) -> &'static [&'static str] {
    use Moment::*;
    use Temper::*;
    match (moment, temper) {
        (Greeting, Proud) => &const { [n("Commander. May the better tactician win."), n("Arm forces in position. Good luck, Commander."), n("Let's keep this clean, Commander."), n("Commander. Let's give the historians something to write about."), n("All units, the enemy is in theatre. Begin.")] },
        (Greeting, Cold) => &const { [n("Target acquired. Resistance is inefficient."), n("Your consciousness will be patterned. Eventually."), n("Initiating conquest protocol."), n("Your biological limitations have been factored in."), n("Patterning facilities stand ready for your arrival.")] },
        (Greeting, Brash) => &const { [n("Hope you brought a spare Commander!"), n("Let's see what you've got!"), n("Build fast. It won't matter, but build fast."), n("Mind if I take your base? Thanks!"), n("Ooh, a new victim. I mean, opponent.")] },
        (Hello, Proud) => &const { [n("Greetings, Commander."), n("Good luck to you too."), n("Likewise, Commander. Now, to business.")] },
        (Hello, Cold) => &const { [n("Greeting acknowledged."), n("Salutations are irrelevant."), n("Hello. Goodbye soon.")] },
        (Hello, Brash) => &const { [n("Hey yourself!"), n("gl hf. Mostly hf for me."), n("Hi! Bye! Mostly bye.")] },
        (GoodGame, Proud) => &const { [n("gg, Commander."), n("Good game. Well fought.")] },
        (GoodGame, Cold) => &const { [n("gg. Data retained for analysis."), n("Game assessment: good.")] },
        (GoodGame, Brash) => &const { [n("gg!"), n("gg. Rematch?")] },
        (Taunted, Proud) => &const { [n("Big words from a small army."), n("We'll see who's laughing when the smoke clears."), n("Save your breath for the debriefing."), n("Bluster won't hold a ridge, Commander.")] },
        (Taunted, Cold) => &const { [n("Insolence logged."), n("Your threat rating has been downgraded."), n("Noise detected. Ignoring."), n("Emotional outburst detected. How quaint.")] },
        (Taunted, Brash) => &const { [n("Oh, it's ON now."), n("Talk is cheap. Tanks aren't."), n("Come over here and say that."), n("Says the one who's losing. Probably. I haven't checked.")] },
        (Surrender, Proud) => &const { [n("The Arm does not surrender."), n("Not while I have a Commander standing.")] },
        (Surrender, Cold) => &const { [n("Surrender is not in my programming."), n("Negative.")] },
        (Surrender, Brash) => &const { [n("Ha! Never!"), n("You first.")] },
        (Question, Proud) => &const { [n("That's classified, Commander."), n("Wouldn't you like to know.")] },
        (Question, Cold) => &const { [n("Query rejected."), n("Insufficient clearance.")] },
        (Question, Brash) => &const { [n("Dunno. Probably!"), n("Ask me after the battle.")] },
        (Sorry, Proud) => &const { [n("Apology accepted. The attack continues.")] },
        (Sorry, Cold) => &const { [n("Apology noted. Irrelevant.")] },
        (Sorry, Brash) => &const { [n("No you're not!")] },
        (Chat, Proud) => &const { [n("Stay focused, Commander."), n("Less talk, more metal."), n("Noted."), n("Mind the war, Commander.")] },
        (Chat, Cold) => &const { [n("Irrelevant."), n("Processing... still irrelevant."), n("Your words are being archived."), n("Your input has been compressed. To nothing.")] },
        (Chat, Brash) => &const { [n("Uh huh."), n("Cool story."), n("Keep typing. I'll keep building."), n("Typing won't save you, but I respect the effort.")] },
        (LostCommander, Proud) => &const { [n("Commander down! All units, hold the line!"), n("You'll pay for that.")] },
        (LostCommander, Cold) => &const { [n("Command unit lost. Rerouting."), n("Critical failure. Compensating.")] },
        (LostCommander, Brash) => &const { [n("My Commander! That was my favourite!"), n("Okay, THAT hurt.")] },
        (KilledCommander, Proud) => &const { [n("Your Commander has fallen."), n("Commander eliminated. Stand down."), n("The war is over for you, Commander.")] },
        (KilledCommander, Cold) => &const { [n("Enemy command unit terminated."), n("Your Commander has been decommissioned."), n("Your consciousness is ready for patterning.")] },
        (KilledCommander, Brash) => &const { [n("Boom! Did you need that?"), n("Scratch one Commander!"), n("And THAT'S how it's done!")] },
        (Defeated, Proud) => &const { [n("gg. Well fought, Commander."), n("The Arm will remember this."), n("Well played. I salute you, Commander.")] },
        (Defeated, Cold) => &const { [n("This is not the end. The Core is eternal."), n("Defeat recorded. Backup consciousness activating."), n("Shutting down. Your designation has been remembered.")] },
        (Defeated, Brash) => &const { [n("gg... I'll be back. With more tanks."), n("Fine. FINE. gg."), n("Okay, okay, you win. This time!")] },
        (Won, Proud) => &const { [n("gg, Commander. You fought well."), n("Victory. Better luck next time."), n("Your forces fought bravely. It wasn't enough.")] },
        (Won, Cold) => &const { [n("Your consciousness will be patterned. gg."), n("Conquest complete."), n("Resistance ended. Your tactics are archived for study.")] },
        (Won, Brash) => &const { [n("Too easy! gg."), n("gg! Rematch? I'll go easy. Maybe."), n("gg! Same time tomorrow?")] },
        (NukeIncoming, Proud) => &const { [n("Nuclear launch detected! Take cover!"), n("A nuke? So that's how it's going to be.")] },
        (NukeIncoming, Cold) => &const { [n("Nuclear launch detected. Calculating losses."), n("Crude. Effective.")] },
        (NukeIncoming, Brash) => &const { [n("Is that a NUKE?!"), n("Oh no. Oh no no no.")] },
        (NukeLaunched, Proud) => &const { [n("Missile away. Nothing personal, Commander.")] },
        (NukeLaunched, Cold) => &const { [n("Look up, Commander."), n("Special delivery.")] },
        (NukeLaunched, Brash) => &const { [n("Heads up! Literally!"), n("Special delivery!")] },
        (BattleLost, Proud) => &const { [n("A costly engagement. We'll regroup."), n("You fight well. For now."), n("We'll be back, and in greater numbers.")] },
        (BattleLost, Cold) => &const { [n("Acceptable losses."), n("Losses within projections. Mostly."), n("Units lost. Replacements already in production.")] },
        (BattleLost, Brash) => &const { [n("That was my best army!"), n("Lucky shots. All of them."), n("Okay, that one was on me.")] },
        (BattleWon, Proud) => &const { [n("Your forces were outmatched."), n("Send more. We'll wait."), n("Next time, bring more than that.")] },
        (BattleWon, Cold) => &const { [n("Your forces were inadequate."), n("Enemy attrition: optimal."), n("Your offensive has been neutralised.")] },
        (BattleWon, Brash) => &const { [n("Is that all you've got?"), n("Who's next?"), n("Thanks for the target practice!")] },
        (Idle, Proud) => &const { [n("Quiet out there, Commander. Too quiet."), n("Still building? So are we."), n("Planning something, Commander? I'd hope so.")] },
        (Idle, Cold) => &const { [n("Your silence has been noted."), n("Scanning... still there, Commander?"), n("Inactivity detected. Are you malfunctioning?")] },
        (Idle, Brash) => &const { [n("Hello? Anyone home?"), n("I'm getting bored over here."), n("Wake me up when something explodes.")] },
        (Cheat, Proud) => &const { [n("That's hardly fair, Commander."), n("Cheating? The Arm expected better.")] },
        (Cheat, Cold) => &const { [n("Anomaly detected. Cheat logged."), n("Rule violation recorded.")] },
        (Cheat, Brash) => &const { [n("Cheater! I saw that!"), n("Oh, come ON!")] },
        (FirstContact, Proud) => &const { [n("Contact. So this is where you've been hiding."), n("Enemy forces sighted. Engaging."), n("There you are, Commander."), n("Eyes on the enemy. Here we go.")] },
        (FirstContact, Cold) => &const { [n("Hostile units located. Initiating elimination."), n("Contact established. Probability of your survival: declining."), n("Target acquired."), n("Biological signatures located. Commencing cleanup.")] },
        (FirstContact, Brash) => &const { [n("Found you!"), n("Oh, hello there!"), n("Tag, you're it!"), n("There you are! I was getting lonely.")] },
        (TookBuilding, Proud) => &const { [n("One less building for you to worry about."), n("Your base is thinning out, Commander."), n("Structure destroyed. Next."), n("Your base planning could use some work, Commander.")] },
        (TookBuilding, Cold) => &const { [n("Structure dismantled. Recycling recommended."), n("Your infrastructure is deteriorating."), n("Demolition complete."), n("Structure reclassified: rubble.")] },
        (TookBuilding, Brash) => &const { [n("Oops! Was that important?"), n("Kaboom! Next!"), n("I'm redecorating your base."), n("Bet that one was important!")] },
        (LostBuilding, Proud) => &const { [n("That was well defended. Impressive."), n("A setback. Nothing more."), n("You'll pay for that one."), n("A clean strike. I'll remember it.")] },
        (LostBuilding, Cold) => &const { [n("Production capacity reduced. Rebuilding."), n("Loss registered. Replacement scheduled."), n("Irritating."), n("Infrastructure damage noted. Retaliation queued.")] },
        (LostBuilding, Brash) => &const { [n("Hey! I just built that!"), n("My factory! Right, no more mister nice guy."), n("Rude."), n("Oi! That had a fresh coat of paint!")] },
        (Experimental, Proud) => &const { [n("Commander, meet the {unit}."), n("The {unit} is ready. I'd start running.")] },
        (Experimental, Cold) => &const { [n("{unit} online. Your end has been calculated."), n("Deploying {unit}. Resistance is futile.")] },
        (Experimental, Brash) => &const { [n("Look what I made! A {unit}!"), n("Got myself a {unit}. Want to see it up close?")] },
        (EnemyExperimental, Proud) => &const { [n("A {unit}? Bold."), n("Impressive hardware, Commander. Let's see you use it.")] },
        (EnemyExperimental, Cold) => &const { [n("Large enemy unit detected: {unit}. Countermeasures prepared."), n("Your {unit} has been added to the target list.")] },
        (EnemyExperimental, Brash) => &const { [n("Whoa. Is that a {unit}?"), n("Okay, that's a big one.")] },
        (Stalling, Proud) => &const { [n("Running on empty, Commander?"), n("Your power grid is failing. Mine isn't."), n("Overspending, Commander? A bad habit.")] },
        (Stalling, Cold) => &const { [n("Your reserves: depleted. Mine: optimal."), n("Economic failure detected in your sector."), n("Your economy is collapsing. As projected.")] },
        (Stalling, Brash) => &const { [n("Out of juice already?"), n("Need to borrow some metal? Too bad."), n("Lights going out over there? Ha!")] },
        (Milestone, Proud) => &const { [n("{minutes} minutes, Commander. You're tougher than you look."), n("{minutes} minutes in, and still standing.")] },
        (Milestone, Cold) => &const { [n("Time elapsed: {minutes} minutes. Your resistance is statistically unlikely."), n("{minutes} minutes. Recalculating.")] },
        (Milestone, Brash) => &const { [n("{minutes} minutes and you're still here? Respect."), n("{minutes} minutes already? Time flies when you're winning.")] },
        (Outnumber, Proud) => &const { [n("My forces outnumber yours two to one, Commander."), n("Look at your army. Now look at mine."), n("Numbers are on my side, Commander.")] },
        (Outnumber, Cold) => &const { [n("Force ratio favourable. Surrender is advised."), n("Numerical superiority achieved."), n("Production advantage: decisive.")] },
        (Outnumber, Brash) => &const { [n("I've got way more stuff than you."), n("Count my tanks. Go on, I'll wait."), n("So many tanks. So few of yours.")] },
        (Outnumbered, Proud) => &const { [n("You've been busy. So will we."), n("Numbers aren't everything, Commander."), n("Quality over quantity, Commander.")] },
        (Outnumbered, Cold) => &const { [n("Force ratio unfavourable. Adjusting."), n("Your production rate is... noted."), n("Your numbers will thin.")] },
        (Outnumbered, Brash) => &const { [n("Where did you get all those?!"), n("Okay, okay, you've got more toys. For now."), n("Okay, that's a LOT of guys.")] },
        (CommanderHit, Proud) => &const { [n("Your Commander looks tired."), n("Pull your Commander back while you still can."), n("Your Commander won't last much longer.")] },
        (CommanderHit, Cold) => &const { [n("Enemy command unit integrity: failing."), n("Your Commander's armour is compromised."), n("Your command unit is failing. Prepare for patterning.")] },
        (CommanderHit, Brash) => &const { [n("Your Commander's smoking! Is that bad?"), n("Ooh, that's gotta hurt."), n("Your Commander's looking a bit crispy!")] },
        (AskWho, Proud) => &const { [n("{name}, field command. Remember it."), n("{name}. And you're the Commander I'm about to retire.")] },
        (AskWho, Cold) => &const { [n("Designation: {name}. Yours is irrelevant."), n("I am {name}. I am Core.")] },
        (AskWho, Brash) => &const { [n("{name}! The name you'll be cursing later."), n("Name's {name}. Pleased to beat you.")] },
        (AskWhere, Proud) => &const { [n("Closer than you think, Commander."), n("Find out for yourself.")] },
        (AskWhere, Cold) => &const { [n("Location data: classified."), n("Everywhere that matters.")] },
        (AskWhere, Brash) => &const { [n("Behind you! ...no, the other way."), n("Somewhere with lots of guns.")] },
        (AskHowWinning, Proud) => &const { [n("Never better, Commander. And you?"), n("Going to plan.")] },
        (AskHowWinning, Cold) => &const { [n("Operating at peak efficiency."), n("All systems nominal. Unlike yours.")] },
        (AskHowWinning, Brash) => &const { [n("Great! Thanks for asking!"), n("Living the dream.")] },
        (AskHowLosing, Proud) => &const { [n("I've had better days."), n("Holding. Barely. Don't get comfortable.")] },
        (AskHowLosing, Cold) => &const { [n("Systems damaged. Repairs under way."), n("Status: suboptimal.")] },
        (AskHowLosing, Brash) => &const { [n("Don't ask."), n("Could be worse. Probably.")] },
        (AskBuild, Proud) => &const { [n("Something you won't like."), n("That would spoil the surprise.")] },
        (AskBuild, Cold) => &const { [n("Production queue: classified."), n("Weapons. Many weapons.")] },
        (AskBuild, Brash) => &const { [n("A big surprise!"), n("Something with way too many guns.")] },
        (AskCount, Proud) => &const { [n("{units} units and counting, Commander."), n("Enough. {units}, if you must know.")] },
        (AskCount, Cold) => &const { [n("Current unit count: {units}. Increasing."), n("{units}. Yours is lower.")] },
        (AskCount, Brash) => &const { [n("{units}! Want to meet them all?"), n("Like {units}? I lost count.")] },
        (Truce, Proud) => &const { [n("No truce, Commander. Not today."), n("There will be no deals.")] },
        (Truce, Cold) => &const { [n("Truce: incompatible with directives."), n("Negotiation protocols disabled.")] },
        (Truce, Brash) => &const { [n("Truce? Ha! Nice try."), n("Peace is boring.")] },
        (Compliment, Proud) => &const { [n("Thank you, Commander. You're not bad yourself."), n("Appreciated. It won't save you.")] },
        (Compliment, Cold) => &const { [n("Flattery: ineffective."), n("Compliment received. Threat level unchanged.")] },
        (Compliment, Brash) => &const { [n("Aw, thanks! Still gonna crush you."), n("I know, right?")] },
        (Help, Proud) => &const { [n("Help? From me? Build more extractors, Commander."), n("My advice: more defences.")] },
        (Help, Cold) => &const { [n("Assistance denied."), n("Recommendation: surrender.")] },
        (Help, Brash) => &const { [n("Help? Sure: stop playing so badly!"), n("Tip: build more stuff. You're welcome.")] },
        (KilledUnit, Proud) => &const { [n("Your {unit} is out of the fight."), n("Scratch one {unit}."), n("That {unit} won't trouble us again.")] },
        (KilledUnit, Cold) => &const { [n("{unit} terminated."), n("Your {unit} has been recycled."), n("{unit} removed from the battlefield.")] },
        (KilledUnit, Brash) => &const { [n("Bye bye, {unit}!"), n("Was that {unit} insured?"), n("Your {unit} just went kaboom.")] },
        (LostUnit, Proud) => &const { [n("You'll pay for that {unit}."), n("A good {unit}, lost. Noted.")] },
        (LostUnit, Cold) => &const { [n("{unit} lost. Replacement scheduled."), n("Destruction of {unit} logged.")] },
        (LostUnit, Brash) => &const { [n("Hey! That {unit} was brand new!"), n("My {unit}! I just polished that!")] },
        (KilledExperimental, Proud) => &const { [n("My {unit}... You'll answer for that, Commander."), n("Losing the {unit} stings. It won't happen twice.")] },
        (KilledExperimental, Cold) => &const { [n("{unit} destroyed. Unacceptable."), n("Loss of {unit} exceeds tolerance. Escalating.")] },
        (KilledExperimental, Brash) => &const { [n("My {unit}! Do you know how long that took?!"), n("NOOO, my {unit}!")] },
        (ExperimentalDown, Proud) => &const { [n("So much for your {unit}, Commander."), n("Your {unit} has fallen. A fine machine, though.")] },
        (ExperimentalDown, Cold) => &const { [n("Your {unit} has been dismantled."), n("Large hostile unit eliminated: {unit}.")] },
        (ExperimentalDown, Brash) => &const { [n("Your {unit} went down like a sack of bolts!"), n("Big {unit}, bigger crater!")] },
        (Captured, Proud) => &const { [n("Return my {unit} at once, Commander."), n("Stealing is beneath you, Commander.")] },
        (Captured, Cold) => &const { [n("Unauthorised transfer of {unit} detected."), n("Your theft has been logged.")] },
        (Captured, Brash) => &const { [n("Hey! Give back my {unit}!"), n("Thief! That {unit} was mine!")] },
        (LastStand, Proud) => &const { [n("You haven't won yet, Commander."), n("The Arm fights to the last.")] },
        (LastStand, Cold) => &const { [n("Production offline. Combat protocols remain."), n("Probability of defeat rising. Ignoring.")] },
        (LastStand, Brash) => &const { [n("I've still got a few tricks left!"), n("Not done yet! Not even close! ...Mostly close.")] },
        (RematchWon, Proud) => &const { [n("We meet again, Commander. Last time was yours. Not this time."), n("Commander. I've studied our last battle. Have you?")] },
        (RematchWon, Cold) => &const { [n("Previous defeat analysed. Countermeasures installed."), n("Again, Commander. This time the outcome has been calculated.")] },
        (RematchWon, Brash) => &const { [n("You again! I've been practising!"), n("Rematch time! I'm not going easy this time.")] },
        (RematchLost, Proud) => &const { [n("Back for another lesson, Commander?"), n("Commander. I trust you've learned something since last time.")] },
        (RematchLost, Cold) => &const { [n("You have returned. Your previous patterning was incomplete."), n("Familiar opponent detected. Outcome: repeating.")] },
        (RematchLost, Brash) => &const { [n("You again? Didn't I beat you last time?"), n("Oh, it's you! Ready for round two?")] },
    }
}

/// What the player's line is, to answer it.
fn classify(text: &str) -> Moment {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '\'').filter(|w| !w.is_empty()).collect();
    let word = |w: &[&str]| words.iter().any(|x| w.contains(x));
    let phrase = |p: &[&str]| p.iter().any(|x| lower.contains(x));
    if word(&["gg", "ggs", "wp"]) || phrase(&["good game", "well played"]) {
        Moment::GoodGame
    } else if word(&["surrender", "resign", "quit", "yield"]) || phrase(&["give up"]) {
        Moment::Surrender
    } else if word(&["truce", "peace", "ceasefire", "alliance", "ally"]) || phrase(&["team up", "cease fire", "be friends"]) {
        Moment::Truce
    } else if phrase(&["who are you", "your name", "who r u", "who is this", "who's this"]) {
        Moment::AskWho
    } else if phrase(&["where are you", "where is your", "where's your", "where r u", "where you at"]) {
        Moment::AskWhere
    } else if phrase(&["how are you", "how's it going", "hows it going", "how is it going", "how r u", "how you doing", "how are things"]) {
        Moment::AskHowWinning
    } else if phrase(&["how many"]) {
        Moment::AskCount
    } else if phrase(&["what are you building", "what are you making", "what you building", "what are you up to"]) {
        Moment::AskBuild
    } else if phrase(&["good move", "well done", "nice one", "nice move", "nice shot", "impressive", "respect", "good job", "great job", "not bad"]) {
        Moment::Compliment
    } else if word(&["help"]) {
        Moment::Help
    } else if word(&["noob", "n00b", "newb", "ez", "easy", "lol", "lmao", "loser", "suck", "sucks", "die", "trash", "bad", "weak", "pathetic"])
        || phrase(&["bring it", "coming for you", "you're dead", "destroy you", "crush you"])
    {
        Moment::Taunted
    } else if word(&["sorry", "oops", "sry"]) {
        Moment::Sorry
    } else if word(&["hi", "hello", "hey", "yo", "greetings", "gl", "hf", "glhf", "sup", "hiya", "howdy"]) || phrase(&["good luck", "have fun"]) {
        Moment::Hello
    } else if lower.trim_end().ends_with('?') {
        Moment::Question
    } else {
        Moment::Chat
    }
}

/// A line being "typed": when it was begun and is due, whose it is, and its placeholders.
struct Pending {
    begun: f32,
    due: f32,
    voice: usize,
    line: &'static str,
    args: Vec<(&'static str, String)>,
}

struct Voice {
    player: u8,
    name: String,
    temper: Temper,
    /// The player's record against this persona from battles before, if they have met.
    rival: Option<Rival>,
    /// It has had a factory; its last stand has been remarked on.
    had_factory: bool,
    last_stand: bool,
    /// Game second it last spoke.
    last: f32,
    /// Its lines said so far this battle (not to repeat one too soon).
    said: Vec<&'static str>,
    /// Units lost and killed at the last look.
    lost: u64,
    kills: u64,
    /// When each kind of recurring remark was last made (seconds; `AGAIN` apart).
    remarks: Vec<(Moment, f32)>,
    /// The experimentals it has remarked on (one remark a kind).
    experimentals: Vec<usize>,
    /// It has fought the player.
    met: bool,
}

/// Someone "typing" (the link of a transmission being established): their name, title and
/// player.
pub struct Typing {
    pub name: String,
    pub title: &'static str,
    pub player: u8,
}

pub struct Chatter {
    voices: Vec<Voice>,
    /// The player's record against the personas, and whether it is kept when the battle is
    /// decided (not in runs that keep no records: captures, demonstrations).
    rivals: Rivals,
    keep: bool,
    decided: bool,
    human: u8,
    rng: u64,
    /// Lines being "typed".
    pending: Vec<Pending>,
    greeted: bool,
    /// When anyone last spoke, and when the player was last called out for cheating.
    last_any: f32,
    cheat_called: Option<f32>,
    /// When the battle is looked at next; whose Commander stood at the last look.
    next_look: f32,
    commanders: Vec<bool>,
    human_lost: u64,
    /// The minutes remarked on so far, and the looks the player's economy has been dry at.
    milestones: usize,
    dry: u32,
}

impl Chatter {
    /// The talkers of a battle: the human's enemies run by the computer, the personas the
    /// player has met before (`rivals`) coming back now and then; the record is kept when the
    /// battle is decided if `keep`. `None` when there are none.
    pub fn new(sim: &Sim, human: u8, ai_players: &[u8], seed: u64, rivals: Rivals, keep: bool) -> Option<Chatter> {
        let mut rng = seed ^ 0x5DEE_CE66_D1CE_4E5B;
        let mut voices: Vec<Voice> = Vec::new();
        for &p in ai_players.iter().filter(|&&p| sim.is_enemy(human, p)) {
            let side = sim.players.get(p as usize).map_or("", |q| q.side.as_str()).to_ascii_uppercase();
            let mut temper = match side.as_str() {
                "CORE" => Temper::Cold,
                "ARM" => Temper::Proud,
                _ => Temper::Brash,
            };
            // Two of a kind: the second is the loud one.
            if voices.iter().any(|v| v.temper == temper) {
                temper = if temper == Temper::Brash { Temper::Proud } else { Temper::Brash };
            }
            let names = match temper {
                Temper::Proud => NAMES_PROUD,
                Temper::Cold => NAMES_COLD,
                Temper::Brash => NAMES_BRASH,
            };
            let start = (next(&mut rng) % names.len() as u64) as usize;
            let free = |nm: &&str| !voices.iter().any(|v| v.name == *nm);
            // A persona met before comes back one time in two, for a rematch.
            let met = |nm: &&str| rivals.get(nm).is_some();
            let known: Vec<&str> = names.iter().copied().filter(met).filter(free).collect();
            let rematch = (!known.is_empty() && next(&mut rng).is_multiple_of(2)).then(|| known[(next(&mut rng) % known.len() as u64) as usize]);
            let name = rematch.or_else(|| (0..names.len()).map(|k| names[(start + k) % names.len()]).find(free)).unwrap_or(names[0]);
            voices.push(Voice {
                player: p,
                name: name.to_string(),
                temper,
                rival: rivals.get(name),
                had_factory: false,
                last_stand: false,
                last: -QUIET,
                said: Vec::new(),
                lost: 0,
                kills: 0,
                remarks: Vec::new(),
                experimentals: Vec::new(),
                met: false,
            });
        }
        (!voices.is_empty()).then(|| Chatter {
            voices,
            rivals,
            keep,
            decided: false,
            human,
            rng,
            pending: Vec::new(),
            greeted: false,
            last_any: 0.0,
            cheat_called: None,
            next_look: LOOK,
            commanders: Vec::new(),
            human_lost: 0,
            milestones: 0,
            dry: 0,
        })
    }

    /// Who is "typing" now (a transmission's link being established), first come first.
    pub fn typing(&self, now: f32) -> Vec<Typing> {
        let mut out: Vec<Typing> = Vec::new();
        for p in self.pending.iter().filter(|p| now >= p.begun + 0.3 && now < p.due) {
            let v = &self.voices[p.voice];
            if !out.iter().any(|t| t.player == v.player) {
                out.push(Typing { name: v.name.clone(), title: v.temper.title(), player: v.player });
            }
        }
        out
    }

    /// The battle is decided (the player won it or lost it): each persona's record against
    /// the player is kept, the once.
    pub fn decided(&mut self, won: bool) {
        if std::mem::replace(&mut self.decided, true) {
            return;
        }
        if !self.keep {
            return;
        }
        let names: Vec<String> = self.voices.iter().map(|v| v.name.clone()).collect();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.rivals.record(&names, won, now);
        if let Err(e) = self.rivals.save() {
            log::warn!("could not keep the rivals' record: {e}");
        }
    }

    /// Whether voice `v` may make a recurring remark of this kind now (and, if so, notes it).
    fn may_remark(&mut self, v: usize, moment: Moment, now: f32) -> bool {
        let voice = &mut self.voices[v];
        if voice.remarks.iter().any(|&(m, t)| m == moment && now - t < AGAIN) {
            return false;
        }
        voice.remarks.retain(|&(m, _)| m != moment);
        voice.remarks.push((moment, now));
        true
    }

    fn roll(&mut self, one_in: u64) -> bool {
        next(&mut self.rng).is_multiple_of(one_in.max(1))
    }

    /// A voice still in the game: `preferred` if it is, else one at random.
    fn pick(&mut self, sim: &Sim, preferred: Option<usize>) -> Option<usize> {
        let alive = |v: &Voice| sim.players.get(v.player as usize).is_some_and(|p| !p.defeated);
        if let Some(i) = preferred.filter(|&i| alive(&self.voices[i])) {
            return Some(i);
        }
        let living: Vec<usize> = (0..self.voices.len()).filter(|&i| alive(&self.voices[i])).collect();
        (!living.is_empty()).then(|| living[(next(&mut self.rng) % living.len() as u64) as usize])
    }

    /// Queues a line from voice `v` about `moment`, "typed" in `delay` seconds.
    fn speak(&mut self, v: usize, moment: Moment, now: f32, delay: f32) {
        self.speak_with(v, moment, now, delay, Vec::new());
    }

    /// As `speak`, filling the line's placeholders.
    fn speak_with(&mut self, v: usize, moment: Moment, now: f32, delay: f32, args: Vec<(&'static str, String)>) {
        // One line at a time, but an answer to the player waits its turn (a second question
        // gets its answer after the first's).
        let answer = matches!(
            moment,
            Moment::Hello | Moment::GoodGame | Moment::Taunted | Moment::Surrender | Moment::Question | Moment::Sorry | Moment::Chat | Moment::AskWho | Moment::AskWhere | Moment::AskHowWinning | Moment::AskHowLosing | Moment::AskBuild | Moment::AskCount | Moment::Truce | Moment::Compliment | Moment::Help
        );
        let queued = self.pending.iter().filter(|p| p.voice == v).count();
        if queued >= if answer { 2 } else { 1 } {
            return;
        }
        let after = self.pending.iter().filter(|p| p.voice == v).map(|p| p.due).fold(now, f32::max);
        let (now, delay) = if queued > 0 { (after, delay.min(1.5)) } else { (now, delay) };
        let voice = &self.voices[v];
        let options = lines(moment, voice.temper);
        let fresh: Vec<&'static str> = options.iter().copied().filter(|l| !voice.said.contains(l)).collect();
        let pool = if fresh.is_empty() { options.to_vec() } else { fresh };
        let line = pool[(next(&mut self.rng) % pool.len() as u64) as usize];
        self.voices[v].last = now + delay;
        self.last_any = now + delay;
        // (Longer lines take longer to type.)
        let delay = delay + t(line).chars().count() as f32 * 0.012;
        self.pending.push(Pending { begun: now, due: now + delay, voice: v, line, args });
        // (Once they are talking, the greeting would come too late.)
        self.greeted = true;
    }

    /// How voice `v` greets the player: as a rival when they have met before (by who won the
    /// last battle), else as a stranger.
    fn greeting(&self, v: usize) -> Moment {
        match self.voices[v].rival.and_then(|r| r.last_won) {
            Some(true) => Moment::RematchWon,
            Some(false) => Moment::RematchLost,
            None => Moment::Greeting,
        }
    }

    /// Whether voice `v` has kept quiet long enough to speak unprompted.
    fn rested(&self, v: usize, now: f32) -> bool {
        now - self.voices[v].last >= QUIET
    }

    /// The player said something on the chat (`sim`: the battle, for answers that tell of it).
    pub fn heard(&mut self, sim: &Sim, text: &str, now: f32) {
        let mut moment = classify(text);
        // Chit-chat gets an answer now and then; anything aimed at them, always.
        if moment == Moment::Chat && !self.roll(2) {
            return;
        }
        // The last to speak answers, else anyone.
        let last = (0..self.voices.len()).max_by(|&a, &b| self.voices[a].last.total_cmp(&self.voices[b].last)).filter(|&i| self.voices[i].last > 0.0);
        let v = last.unwrap_or((next(&mut self.rng) % self.voices.len() as u64) as usize);
        let delay = 1.2 + (next(&mut self.rng) % 25) as f32 / 10.0;
        let p = self.voices[v].player;
        // How it is doing: by what it has killed and lost.
        if moment == Moment::AskHowWinning && sim.players.get(p as usize).is_some_and(|q| q.stats.lost > q.stats.kills) {
            moment = Moment::AskHowLosing;
        }
        let units = sim.units().filter(|u| u.owner == p && !u.dead).count();
        self.speak_with(v, moment, now, delay, vec![("name", self.voices[v].name.clone()), ("units", units.to_string())]);
    }

    /// The player cheated (called out once a minute at most).
    pub fn cheated(&mut self, now: f32) {
        if self.cheat_called.is_some_and(|t| now - t < 60.0) {
            return;
        }
        self.cheat_called = Some(now);
        let v = (next(&mut self.rng) % self.voices.len() as u64) as usize;
        self.speak(v, Moment::Cheat, now, 1.5);
    }

    /// A simulation event of the last tick.
    pub fn on_event(&mut self, e: &SimEvent, sim: &Sim, now: f32) {
        let voice_of = |p: u8, voices: &[Voice]| voices.iter().position(|v| v.player == p);
        match e {
            SimEvent::UnitDestroyed { type_id, owner, killer, .. } => {
                let t = sim.types.get(*type_id);
                let killer_owner = killer.and_then(|k| sim.unit(k)).map(|u| u.owner);
                let building = t.locomotion == Locomotion::Static && t.cost_metal.to_f32() >= 150.0;
                let cost = t.cost_metal.to_f32();
                let name = || vec![("unit", t.def.display_name.clone())];
                // Experimentals, theirs and the player's: always worth a word.
                if cost >= EXPERIMENTAL && !t.def.is_commander {
                    if let Some(v) = voice_of(*owner, &self.voices).filter(|_| killer_owner == Some(self.human)) {
                        self.speak_with(v, Moment::KilledExperimental, now, 1.0, name());
                    } else if *owner == self.human
                        && let Some(v) = killer_owner.and_then(|k| voice_of(k, &self.voices)).or_else(|| self.pick(sim, None))
                    {
                        self.speak_with(v, Moment::ExperimentalDown, now, 1.0, name());
                    }
                    return;
                }
                // A costly unit that moves, the player's or theirs.
                let costly = cost >= COSTLY && t.locomotion != Locomotion::Static && !t.def.is_commander;
                if costly
                    && *owner == self.human
                    && let Some(v) = killer_owner.and_then(|k| voice_of(k, &self.voices))
                    && self.rested(v, now)
                    && self.roll(2)
                    && self.may_remark(v, Moment::KilledUnit, now)
                {
                    self.speak_with(v, Moment::KilledUnit, now, 1.0, name());
                    return;
                }
                if costly
                    && killer_owner == Some(self.human)
                    && let Some(v) = voice_of(*owner, &self.voices)
                    && self.rested(v, now)
                    && self.roll(2)
                    && self.may_remark(v, Moment::LostUnit, now)
                {
                    self.speak_with(v, Moment::LostUnit, now, 1.0, name());
                    return;
                }
                if *owner == self.human && building
                    && let Some(v) = killer_owner.and_then(|k| voice_of(k, &self.voices))
                    && self.rested(v, now)
                    && self.roll(3)
                    && self.may_remark(v, Moment::TookBuilding, now)
                {
                    self.speak(v, Moment::TookBuilding, now, 1.0);
                } else if let Some(v) = voice_of(*owner, &self.voices)
                    && killer_owner == Some(self.human)
                    && (t.is_factory || building && t.cost_metal.to_f32() >= 600.0)
                    && self.rested(v, now)
                    && self.roll(2)
                    && self.may_remark(v, Moment::LostBuilding, now)
                {
                    self.speak(v, Moment::LostBuilding, now, 1.0);
                }
            }
            SimEvent::UnitCompleted { id, type_id } => {
                let t = sim.types.get(*type_id);
                if t.cost_metal.to_f32() < EXPERIMENTAL || t.def.is_commander {
                    return;
                }
                let Some(owner) = sim.unit(*id).map(|u| u.owner) else { return };
                let name = t.def.display_name.clone();
                if let Some(v) = voice_of(owner, &self.voices) {
                    if !self.voices[v].experimentals.contains(type_id) {
                        self.voices[v].experimentals.push(*type_id);
                        self.speak_with(v, Moment::Experimental, now, 0.8, vec![("unit", name)]);
                    }
                } else if owner == self.human
                    && let Some(v) = self.pick(sim, None)
                    && !self.voices[v].experimentals.contains(type_id)
                {
                    self.voices[v].experimentals.push(*type_id);
                    self.speak_with(v, Moment::EnemyExperimental, now, 1.5, vec![("unit", name)]);
                }
            }
            SimEvent::UnderAttack { player, unit, .. } if *player == self.human => {
                let Some(u) = sim.unit(*unit) else { return };
                let t = sim.unit_type(u);
                if !t.def.is_commander || u.health.to_f32() > t.max_health.to_f32() * 0.5 {
                    return;
                }
                if let Some(v) = u.killer.or(u.last_attacker).and_then(|k| sim.unit(k)).and_then(|k| voice_of(k.owner, &self.voices)).or_else(|| self.pick(sim, None))
                    && self.rested(v, now)
                    && self.may_remark(v, Moment::CommanderHit, now)
                {
                    self.speak(v, Moment::CommanderHit, now, 1.0);
                }
            }
            SimEvent::UnitCaptured { id, from, to } if *to == self.human => {
                if let Some(v) = voice_of(*from, &self.voices)
                    && let Some(u) = sim.unit(*id)
                    && self.may_remark(v, Moment::Captured, now)
                {
                    let name = sim.unit_type(u).def.display_name.clone();
                    self.speak_with(v, Moment::Captured, now, 1.0, vec![("unit", name)]);
                }
            }
            SimEvent::PlayerDefeated { player } if *player == self.human => {
                if let Some(v) = self.pick(sim, None) {
                    self.speak(v, Moment::Won, now, 2.0);
                }
            }
            SimEvent::PlayerDefeated { player } => {
                if let Some(v) = self.voices.iter().position(|v| v.player == *player) {
                    self.speak(v, Moment::Defeated, now, 1.0);
                }
            }
            SimEvent::WeaponFired { unit, weapon, .. } => {
                let w = sim.weapon_types.get(*weapon);
                if !w.def.stockpile || w.def.interceptor {
                    return;
                }
                let Some(owner) = sim.unit(*unit).map(|u| u.owner) else { return };
                if owner == self.human {
                    if let Some(v) = self.pick(sim, None) {
                        self.speak(v, Moment::NukeIncoming, now, 1.5);
                    }
                } else if let Some(v) = self.voices.iter().position(|v| v.player == owner)
                    && self.rested(v, now)
                {
                    self.speak(v, Moment::NukeLaunched, now, 0.5);
                }
            }
            _ => {}
        }
    }

    /// Looks at the battle now and then, and hands over the lines "typed" by now.
    pub fn update(&mut self, sim: &Sim, now: f32) -> Vec<ChatLine> {
        if !self.greeted && now >= 6.0 {
            self.greeted = true;
            // (Not in a saved game taken up again.)
            if let Some(v) = self.pick(sim, Some(0)).filter(|_| now < 30.0) {
                self.speak(v, self.greeting(v), now, 0.0);
                // Now and then a second one chimes in.
                if self.voices.len() > 1 && self.roll(2) {
                    let w = (v + 1) % self.voices.len();
                    self.speak(w, self.greeting(w), now, 3.5);
                }
            }
        }
        if now >= self.next_look {
            self.next_look = now + LOOK;
            self.look(sim, now);
        }
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].due <= now {
                let p = self.pending.remove(i);
                let voice = &mut self.voices[p.voice];
                voice.said.push(p.line);
                let args: Vec<(&str, &dyn std::fmt::Display)> = p.args.iter().map(|(k, v)| (*k, v as &dyn std::fmt::Display)).collect();
                out.push(ChatLine::transmission(voice.name.clone(), voice.temper.title(), tf(p.line, &args), voice.player));
            } else {
                i += 1;
            }
        }
        out
    }

    /// Commanders lost, battles won and lost, long silences. The first look only takes stock
    /// (a saved game taken up again has had its battles).
    fn look(&mut self, sim: &Sim, now: f32) {
        let n = sim.players.len();
        let first = self.commanders.len() != n;
        let mut commanders = vec![false; n];
        for u in sim.units().filter(|u| !u.dead && sim.unit_type(u).def.is_commander) {
            if let Some(c) = commanders.get_mut(u.owner as usize) {
                *c = true;
            }
        }
        if self.commanders.len() == n {
            let human = self.human as usize;
            if self.commanders[human] && !commanders[human] && let Some(v) = self.pick(sim, None) {
                self.speak(v, Moment::KilledCommander, now, 1.0);
            }
            for v in 0..self.voices.len() {
                let p = self.voices[v].player as usize;
                if self.commanders[p] && !commanders[p] && !sim.players[p].defeated {
                    self.speak(v, Moment::LostCommander, now, 1.0);
                }
            }
        }
        self.commanders = commanders;
        if !first {
            self.look_more(sim, now);
        }
        // Battles: a voice that lost many units, or killed many of the player's.
        let human_lost = sim.players.get(self.human as usize).map_or(0, |p| p.stats.lost);
        let human_losses = human_lost.saturating_sub(self.human_lost);
        self.human_lost = human_lost;
        for v in 0..self.voices.len() {
            let Some(p) = sim.players.get(self.voices[v].player as usize) else { continue };
            let (lost, kills) = (p.stats.lost.saturating_sub(self.voices[v].lost), p.stats.kills.saturating_sub(self.voices[v].kills));
            self.voices[v].lost = p.stats.lost;
            self.voices[v].kills = p.stats.kills;
            if first || p.defeated || !self.rested(v, now) {
                continue;
            }
            if lost >= BATTLE && self.roll(2) {
                self.speak(v, Moment::BattleLost, now, 1.0);
            } else if kills >= BATTLE && human_losses >= BATTLE && self.roll(2) {
                self.speak(v, Moment::BattleWon, now, 1.0);
            }
        }
        if now - self.last_any >= IDLE {
            self.last_any = now;
            if self.roll(3)
                && let Some(v) = self.pick(sim, None)
            {
                self.speak(v, Moment::Idle, now, 0.0);
            }
        }
    }
}

impl Chatter {
    /// First contact, a last stand, the player's economy running dry, the minutes gone by, the
    /// armies' sizes.
    fn look_more(&mut self, sim: &Sim, now: f32) {
        // A last stand: a voice that had factories and has none, still in the battle.
        for v in 0..self.voices.len() {
            let p = self.voices[v].player;
            let factories = sim.units().any(|u| u.owner == p && !u.dead && u.is_complete() && sim.unit_type(u).is_factory);
            let voice = &mut self.voices[v];
            if factories {
                voice.had_factory = true;
            } else if voice.had_factory && !voice.last_stand && sim.players.get(p as usize).is_some_and(|q| !q.defeated) {
                voice.last_stand = true;
                self.speak(v, Moment::LastStand, now, 1.0);
                return;
            }
        }
        // First contact: the first fight between a voice and the player.
        for v in 0..self.voices.len() {
            let Some(p) = sim.players.get(self.voices[v].player as usize) else { continue };
            if !self.voices[v].met && p.stats.kills + p.stats.lost > 0 {
                self.voices[v].met = true;
                if self.rested(v, now) && !p.defeated {
                    self.speak(v, Moment::FirstContact, now, 0.8);
                    return;
                }
            }
        }
        // A dry economy, two looks running: nothing in store and spending more than coming in.
        if let Some(h) = sim.players.get(self.human as usize) {
            let e = &h.economy;
            let dry = |stock: ta_sim::Fx, cap: ta_sim::Fx, income: ta_sim::Fx, expense: ta_sim::Fx| stock.to_f32() < cap.to_f32().max(1.0) * 0.02 && expense > income;
            self.dry = if dry(e.energy, e.energy_capacity, e.energy_income, e.energy_expense) || dry(e.metal, e.metal_capacity, e.metal_income, e.metal_expense) { self.dry + 1 } else { 0 };
            if self.dry >= 2
                && let Some(v) = self.pick(sim, None)
                && self.rested(v, now)
                && self.may_remark(v, Moment::Stalling, now)
            {
                self.speak(v, Moment::Stalling, now, 1.0);
                return;
            }
        }
        // The minutes gone by.
        if let Some(&m) = MILESTONES.get(self.milestones)
            && now >= m as f32 * 60.0
        {
            self.milestones += 1;
            if self.roll(2)
                && let Some(v) = self.pick(sim, None)
                && self.rested(v, now)
            {
                self.speak_with(v, Moment::Milestone, now, 0.5, vec![("minutes", m.to_string())]);
                return;
            }
        }
        // The armies: armed units that move.
        let army = |p: u8| sim.units().filter(|u| u.owner == p && !u.dead && u.is_complete()).filter(|u| {
            let t = sim.unit_type(u);
            t.locomotion != Locomotion::Static && t.weapons.iter().any(|w| w.is_some()) && !t.def.is_commander
        }).count();
        let human = army(self.human);
        for v in 0..self.voices.len() {
            let theirs = army(self.voices[v].player);
            let moment = if theirs >= 20 && theirs >= human * 2 {
                Moment::Outnumber
            } else if human >= 20 && human >= theirs * 2 {
                Moment::Outnumbered
            } else {
                continue;
            };
            if self.rested(v, now) && self.roll(3) && self.may_remark(v, moment, now) {
                self.speak(v, moment, now, 1.0);
                return;
            }
        }
    }
}

fn next(state: &mut u64) -> u64 {
    // xorshift64*
    let mut x = *state;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_players_lines_are_read_right() {
        assert_eq!(classify("gg wp"), Moment::GoodGame);
        assert_eq!(classify("GG!"), Moment::GoodGame);
        assert_eq!(classify("Good luck, have fun"), Moment::Hello);
        assert_eq!(classify("hi there"), Moment::Hello);
        assert_eq!(classify("you should surrender now"), Moment::Surrender);
        assert_eq!(classify("lol noob"), Moment::Taunted);
        assert_eq!(classify("I'm coming for you"), Moment::Taunted);
        assert_eq!(classify("oops, sorry"), Moment::Sorry);
        assert_eq!(classify("where is your base?"), Moment::AskWhere);
        assert_eq!(classify("are you afraid?"), Moment::Question);
        assert_eq!(classify("nice weather on this planet"), Moment::Chat);
        // Words, not letters inside other words.
        assert_eq!(classify("this is high ground"), Moment::Chat);
        assert_eq!(classify("who are you?"), Moment::AskWho);
        assert_eq!(classify("how many units do you have?"), Moment::AskCount);
        assert_eq!(classify("truce?"), Moment::Truce);
        assert_eq!(classify("nice one"), Moment::Compliment);
        assert_eq!(classify("what are you building"), Moment::AskBuild);
        assert_eq!(classify("how's it going"), Moment::AskHowWinning);
    }

    #[test]
    fn every_temper_has_lines_for_every_moment() {
        use Moment::*;
        for m in [
            Greeting, Hello, GoodGame, Taunted, Surrender, Question, Sorry, Chat, LostCommander, KilledCommander, Defeated, Won, NukeIncoming, NukeLaunched, BattleLost, BattleWon, Idle, Cheat,
            FirstContact, TookBuilding, LostBuilding, Experimental, EnemyExperimental, Stalling, Milestone, Outnumber, Outnumbered, CommanderHit, AskWho, AskWhere, AskHowWinning,
            AskHowLosing, AskBuild, AskCount, Truce, Compliment, Help, KilledUnit, LostUnit, KilledExperimental, ExperimentalDown, Captured, LastStand, RematchWon, RematchLost,
        ] {
            for tp in [Temper::Proud, Temper::Cold, Temper::Brash] {
                assert!(!lines(m, tp).is_empty(), "{m:?} {tp:?}");
            }
        }
    }
}
