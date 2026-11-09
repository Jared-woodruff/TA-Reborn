use crate::Msg;
use ta_sim::Command;

pub(crate) fn player_command(command: &Command) -> bool {
    !matches!(command, Command::Abandon { .. } | Command::Cheat { .. })
}

/// Retained bytes of one command, including its inline representation and owned vector
/// capacities. A container must account for its own unused capacity separately.
pub fn command_memory(command: &Command) -> usize {
    let units = match command {
        Command::Order { units, .. } | Command::Formation { units, .. } | Command::SetActive { units, .. }
        | Command::SetCloak { units, .. } | Command::SetStates { units, .. } | Command::SelfDestruct { units, .. }
        | Command::GiveUnits { units, .. } | Command::SetRepeat { factories: units, .. } => units.capacity(),
        Command::Cheat { cheat: ta_sim::cheats::Cheat::Destroy(units) | ta_sim::cheats::Cheat::Heal(units) | ta_sim::cheats::Cheat::Promote(units), .. } => units.capacity(),
        _ => 0,
    };
    let spots = match command {
        Command::Formation { shape: ta_sim::formation::Shape::Spots(spots), .. } => spots.capacity() * std::mem::size_of::<ta_sim::FxVec3>(),
        _ => 0,
    };
    std::mem::size_of::<Command>() + units * std::mem::size_of::<ta_sim::UnitId>() + spots
}

pub(crate) fn history_memory(history: &[(u64, Vec<Command>)]) -> usize {
    history.iter().map(|(_, commands)| history_entry_memory(commands)).sum()
}

pub(crate) fn history_entry_memory(commands: &[Command]) -> usize {
    2 * std::mem::size_of::<(u64, Vec<Command>)>() + commands_memory(commands)
}

fn commands_memory(commands: &[Command]) -> usize {
    // Vec growth can leave spare capacity; charge twice the elements as a conservative bound.
    commands.iter().map(command_memory).sum::<usize>() * 2
}

pub(crate) fn message_memory(message: &Msg) -> usize {
    let nested = match message {
        Msg::Commands(c) | Msg::Tick { commands: c, .. } => commands_memory(c),
        // (Once: what it holds is moved into the history kept, not copied. Charged twice, a
        // part of tiny commands held more than 64 times its bytes, the reader's budget.)
        Msg::History(h) => history_memory(h),
        Msg::Resume { start: s, history: h, .. } => history_memory(h)
            + s.slots.capacity() * std::mem::size_of::<crate::LobbySlot>() + s.spectators.capacity() * std::mem::size_of::<(u8, String)>(),
        Msg::Lobby(l) | Msg::Rematch(l) => l.slots.capacity() * std::mem::size_of::<crate::LobbySlot>() + l.spectators.capacity() * std::mem::size_of::<(u8, String)>()
            + l.map_status.capacity() * std::mem::size_of::<(u8, crate::MapStatus)>(),
        Msg::Maps(maps) => maps.capacity() * std::mem::size_of::<(String, u64)>(),
        Msg::Reject { reason: crate::Refusal::Data { mods } } => mods.capacity() * std::mem::size_of::<crate::NetMod>(),
        Msg::Start(s) => s.slots.capacity() * std::mem::size_of::<crate::LobbySlot>() + s.spectators.capacity() * std::mem::size_of::<(u8, String)>(),
        Msg::Waiting(names) => names.capacity() * std::mem::size_of::<String>(),
        Msg::Pings(p) | Msg::Away(p) => p.capacity() * std::mem::size_of::<(u8, u16)>(),
        Msg::Portrait(png) => png.capacity(),
        Msg::Portraits(p) => p.capacity() * std::mem::size_of::<(u8, Vec<u8>)>() + p.iter().map(|(_, png)| png.capacity()).sum::<usize>(),
        _ => 0,
    };
    std::mem::size_of::<Msg>() + nested + bincode::serialized_size(message).unwrap_or(u64::MAX) as usize
}

pub(crate) fn valid_history(history: &[(u64, Vec<Command>)], upto: u64, mut previous: u64) -> bool {
    history.iter().all(|(tick, _)| {
        let valid = *tick > previous && *tick <= upto;
        previous = *tick;
        valid
    })
}

fn valid_slots(slots: &[crate::LobbySlot], spectators: &[(u8, String)]) -> bool {
    let mut peers = std::collections::BTreeSet::new();
    slots.len() <= 10 && spectators.len() <= 32 && slots.iter().all(|s| {
        s.color < 10 && s.team < 10 && s.side.len() <= 128 && match &s.kind {
            crate::SlotKind::Human { peer, name } => name.len() <= 128 && peers.insert(*peer),
            crate::SlotKind::Ai { difficulty } => *difficulty <= 2,
            crate::SlotKind::Open | crate::SlotKind::Closed => true,
        }
    }) && spectators.iter().all(|(peer, name)| name.len() <= 128 && peers.insert(*peer))
}

pub(crate) fn host_message(message: &Msg) -> bool {
    match message {
        Msg::Lobby(l) | Msg::Rematch(l) => l.map.len() <= 512 && l.title.len() <= 256 && valid_slots(&l.slots, &l.spectators) && l.map_status.len() <= 32 && l.map_title.len() <= 256,
        Msg::Start(s) | Msg::Resume { start: s, .. } => {
            s.map.len() <= 512 && valid_slots(&s.slots, &s.spectators)
                && (!matches!(message, Msg::Resume { player, .. } if *player as usize >= s.slots.iter().filter(|s| s.kind.plays()).count()))
        }
        Msg::Pings(p) => p.len() <= 256,
        Msg::Away(a) => a.len() <= 10 && a.iter().all(|(player, _)| *player < 10),
        Msg::Returned { .. } => true,
        Msg::Portraits(p) => p.len() <= 256 && p.iter().all(|(_, png)| png.len() <= crate::PORTRAIT_BYTES),
        Msg::Ping(_) | Msg::Removed => true,
        Msg::Welcome { .. } | Msg::Reject { .. } | Msg::Chat { .. } | Msg::Tick { .. } | Msg::Desync { .. }
        | Msg::Pace { .. } | Msg::Waiting(_) | Msg::Bye | Msg::History(_) | Msg::ReconnectUnavailable => true,
        _ => false,
    }
}

/// Returns canonical text and whether it is visible only to teammates and spectators.
/// Encoded player ids are untrusted: the host supplies the identity from the connection.
pub(crate) fn authenticate_chat(text: &str, player: Option<u8>) -> Option<(String, bool)> {
    if let Some(rest) = text.strip_prefix(crate::ALLY_PREFIX) {
        let player = player?;
        let (_, line) = rest.split_once(' ')?;
        if line.trim().is_empty() { return None; }
        Some((format!("{}{player} {line}", crate::ALLY_PREFIX), true))
    } else if let Some(rest) = text.strip_prefix(crate::PING_PREFIX) {
        let player = player?;
        let mut words = rest.split_whitespace();
        words.next()?; // Never use the claimed identity.
        let x = words.next()?.parse::<i32>().ok()?;
        let z = words.next()?.parse::<i32>().ok()?;
        // (What it means, if anything: one of six.)
        let kind = match words.next() {
            Some(k) => Some(k.parse::<u8>().ok().filter(|k| *k < 6)?),
            None => None,
        };
        if words.next().is_some() { return None; }
        Some((match kind {
            Some(k) => format!("{}{player} {x} {z} {k}", crate::PING_PREFIX),
            None => format!("{}{player} {x} {z}", crate::PING_PREFIX),
        }, true))
    } else { Some((text.into(), false)) }
}

pub(crate) fn chat_visible(start: Option<&crate::StartInfo>, source: Option<u8>, target: Option<u8>, private: bool) -> bool {
    if !private { return true; }
    let Some(source) = source else { return false };
    let Some(target) = target else { return true }; // Spectators see every team.
    if source == target { return true; }
    let Some(start) = start else { return false };
    let team = |player: u8| start.slots.iter().filter(|s| s.kind.plays()).nth(player as usize).map(|s| s.team);
    match (team(source), team(target)) { (Some(a), Some(b)) => a == b, _ => false }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ALLY_PREFIX, LobbySlot, PING_PREFIX, SlotKind, StartInfo};

    fn slot(kind: SlotKind, team: u8) -> LobbySlot {
        LobbySlot { kind, side: "ARM".into(), color: team, ready: true, team }
    }
    fn human(peer: u8, team: u8) -> LobbySlot {
        slot(SlotKind::Human { peer, name: format!("P{peer}") }, team)
    }

    #[test]
    fn chat_goes_out_in_the_senders_name_and_malformed_signals_are_refused() {
        // The player a line claims to be from is replaced by the connection's.
        assert_eq!(authenticate_chat(&format!("{ALLY_PREFIX}7 flank left"), Some(2)), Some((format!("{ALLY_PREFIX}2 flank left"), true)));
        // An allies' line needs a sender and words.
        assert_eq!(authenticate_chat(&format!("{ALLY_PREFIX}7 flank"), None), None);
        assert_eq!(authenticate_chat(&format!("{ALLY_PREFIX}7   "), Some(2)), None);
        assert_eq!(authenticate_chat(&format!("{ALLY_PREFIX}alone"), Some(2)), None);
        // A ping: two whole numbers, then at most one of the six meanings.
        assert_eq!(authenticate_chat(&format!("{PING_PREFIX}9 100 -40"), Some(1)), Some((format!("{PING_PREFIX}1 100 -40"), true)));
        assert_eq!(authenticate_chat(&format!("{PING_PREFIX}9 100 -40 5"), Some(1)), Some((format!("{PING_PREFIX}1 100 -40 5"), true)));
        for bad in ["9 100", "9 100 x", "9 1.5 2", "9 1 2 6", "9 1 2 -1", "9 1 2 3 4", ""] {
            assert_eq!(authenticate_chat(&format!("{PING_PREFIX}{bad}"), Some(1)), None, "{bad:?}");
        }
        assert_eq!(authenticate_chat(&format!("{PING_PREFIX}9 1 2"), None), None, "nobody's ping");
        // Anything else is said as it is, to everyone.
        assert_eq!(authenticate_chat("gg wp", None), Some(("gg wp".into(), false)));
    }

    #[test]
    fn private_lines_reach_the_senders_team_and_spectators_alone() {
        let start = StartInfo {
            map: "Test".into(),
            seed: 1,
            // Players 0 and 2 on team 0, player 1 on team 1 (the open seat is no player).
            slots: vec![human(0, 0), slot(SlotKind::Open, 5), human(1, 1), slot(SlotKind::Ai { difficulty: 1 }, 0)],
            rules: Default::default(),
            spectators: vec![(9, "Watcher".into())],
        };
        assert!(chat_visible(Some(&start), Some(1), Some(0), false), "a line to all");
        assert!(chat_visible(Some(&start), Some(0), Some(2), true), "an ally");
        assert!(!chat_visible(Some(&start), Some(0), Some(1), true), "an enemy");
        assert!(chat_visible(Some(&start), Some(1), Some(1), true), "oneself");
        assert!(chat_visible(Some(&start), Some(1), None, true), "a spectator sees every team's");
        assert!(!chat_visible(Some(&start), None, Some(0), true), "a spectator's private line reaches no player");
        assert!(!chat_visible(Some(&start), Some(0), Some(7), true), "no such player");
        assert!(!chat_visible(None, Some(0), Some(1), true), "before the battle, no teams to tell");
    }

    #[test]
    fn a_hosts_lobby_must_be_within_bounds_and_name_each_peer_once() {
        let lobby = |slots: Vec<LobbySlot>, spectators: Vec<(u8, String)>| Msg::Start(StartInfo { map: "Test".into(), seed: 1, slots, rules: Default::default(), spectators });
        assert!(host_message(&lobby(vec![human(0, 0), human(1, 1)], vec![(2, "W".into())])));
        assert!(!host_message(&lobby(vec![human(0, 0), human(0, 1)], vec![])), "a peer in two seats");
        assert!(!host_message(&lobby(vec![human(0, 0)], vec![(0, "W".into())])), "a player watching too");
        assert!(!host_message(&lobby((0..11).map(|p| human(p, 0)).collect(), vec![])), "eleven seats");
        assert!(!host_message(&lobby(vec![slot(SlotKind::Ai { difficulty: 3 }, 0)], vec![])), "no fourth difficulty");
        assert!(!host_message(&lobby(vec![LobbySlot { color: 10, ..human(0, 0) }], vec![])), "colour out of range");
        assert!(!host_message(&lobby(vec![LobbySlot { side: "X".repeat(129), ..human(0, 0) }], vec![])), "side's name too long");
        assert!(!host_message(&lobby(vec![human(0, 0)], (0..33).map(|p| (p + 1, String::new())).collect())), "thirty-three watching");
        // A player to resume as must be one of those playing.
        let start = StartInfo { map: "Test".into(), seed: 1, slots: vec![human(0, 0), human(1, 1)], rules: Default::default(), spectators: vec![] };
        assert!(host_message(&Msg::Resume { start: start.clone(), player: 1, upto: 0, history: Vec::new(), paused: false, speed: 100 }));
        assert!(!host_message(&Msg::Resume { start, player: 2, upto: 0, history: Vec::new(), paused: false, speed: 100 }));
    }

    #[test]
    fn history_must_run_forward_and_not_past_its_end() {
        let at = |ticks: &[u64]| ticks.iter().map(|&t| (t, Vec::new())).collect::<Vec<(u64, Vec<Command>)>>();
        assert!(valid_history(&at(&[3, 5, 9]), 9, 2));
        assert!(valid_history(&at(&[]), 0, 0));
        assert!(!valid_history(&at(&[3, 3]), 9, 0), "a tick twice");
        assert!(!valid_history(&at(&[5, 4]), 9, 0), "backwards");
        assert!(!valid_history(&at(&[3]), 9, 3), "not after what came before");
        assert!(!valid_history(&at(&[10]), 9, 0), "past its end");
    }

    #[test]
    fn a_commands_memory_counts_what_it_holds() {
        let small = Command::Order { player: 0, units: Vec::new(), order: ta_sim::Order::Stop, queue: false };
        let big = Command::Order { player: 0, units: vec![ta_sim::UnitId { slot: 1, generation: 0 }; 1000], order: ta_sim::Order::Stop, queue: false };
        assert_eq!(command_memory(&small), std::mem::size_of::<Command>());
        assert!(command_memory(&big) >= std::mem::size_of::<Command>() + 1000 * std::mem::size_of::<ta_sim::UnitId>());
        assert_eq!(history_memory(&[(1, vec![small.clone()]), (2, vec![small])]), 2 * history_entry_memory(&[Command::Order { player: 0, units: Vec::new(), order: ta_sim::Order::Stop, queue: false }]));
        assert!(!player_command(&Command::Abandon { player: 0 }));
    }
}
