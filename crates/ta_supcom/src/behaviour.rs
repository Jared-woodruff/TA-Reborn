//! What Forged Alliance's scripts say about its units and projectiles that their blueprints do
//! not: a unit's weapons and intel its script switches off as it is made (a Seraphim commander's
//! missile, a sniper's second gun, the Cybran commander's stealth), and what a shell its script
//! splits into (the Salvation's fragments). The scripts are Lua programs; nothing is run. They are
//! read as text for these few statements, each in the function and the block it stands in.
//!
//! - **At birth** ([`Birth`]): a unit is made with every weapon on, then its weapons' `OnCreate`
//!   (`self:SetWeaponEnabled(false)` in a weapon's class), its own `OnCreate` and its
//!   `OnStopBeingBuilt` run, in that order, each statement in turn: `SetWeaponEnabledByLabel
//!   ('<label>', true|false)` and `EnableUnitIntel('<kind>')` / `DisableUnitIntel('<kind>')`.
//!   Statements inside an `if` there (a layer's: a Wagner's guns on land, its torpedo under the
//!   water) depend on where the unit is: what any of them, or the unit's `OnLayerChange`, turns on
//!   is kept ([`Birth::by_layer`]), for TA Reborn's own rules say where each fires from.
//! - **Fragments** ([`fragments`]): a projectile whose `OnImpact` makes child projectiles
//!   (`CreateChildProjectile(ChildProjectileBP)`, once and in a loop of `numProjectiles`), each
//!   with its parent's damage, or a share of it where the script divides it
//!   (`DamageAmount / numProjectiles`).

/// What a unit's script switches off as it is made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Birth {
    /// Weapons (labels as written) off once the unit is made.
    pub weapons_off: Vec<String>,
    /// Weapons switched on or off by where the unit is (in an `if` of its making, or by its
    /// `OnLayerChange`): never counted off.
    pub by_layer: Vec<String>,
    /// Intel (`Radar`, `Sonar`, `RadarStealth`, `SonarStealth`, `Cloak`, `Omni`...) off once made.
    pub intel_off: Vec<String>,
}

impl Birth {
    /// Whether the weapon `label` is off once the unit is made.
    pub fn weapon_off(&self, label: &str) -> bool {
        self.weapons_off.iter().any(|l| l.eq_ignore_ascii_case(label)) && !self.by_layer.iter().any(|l| l.eq_ignore_ascii_case(label))
    }

    /// Whether the intel `kind` is off once the unit is made.
    pub fn intel_off(&self, kind: &str) -> bool {
        self.intel_off.iter().any(|k| k.eq_ignore_ascii_case(kind))
    }
}

/// A script's words: names, strings and single marks, comments and strings' insides left out of
/// the marks (`#` and `--` comments to the line's end, `--[[ ]]` blocks).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Token {
    Name(String),
    Str(String),
    Num(String),
    Mark(char),
}

pub(crate) fn tokens(text: &str) -> Vec<Token> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'#' || (c == b'-' && b.get(i + 1) == Some(&b'-')) {
            if c == b'-' && b.get(i + 2) == Some(&b'[') && b.get(i + 3) == Some(&b'[') {
                i = text[i..].find("]]").map_or(b.len(), |e| i + e + 2);
            } else {
                i = text[i..].find('\n').map_or(b.len(), |e| i + e + 1);
            }
        } else if c == b'\'' || c == b'"' {
            let mut j = i + 1;
            let mut s = Vec::new();
            while j < b.len() && b[j] != c && b[j] != b'\n' {
                if b[j] == b'\\' && j + 1 < b.len() {
                    j += 1;
                }
                s.push(b[j]);
                j += 1;
            }
            out.push(Token::Str(String::from_utf8_lossy(&s).into_owned()));
            i = j + 1;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let j = (i..b.len()).find(|&j| !(b[j].is_ascii_alphanumeric() || b[j] == b'_')).unwrap_or(b.len());
            out.push(Token::Name(text[i..j].to_string()));
            i = j;
        } else if c.is_ascii_digit() {
            let j = (i..b.len()).find(|&j| !(b[j].is_ascii_alphanumeric() || b[j] == b'.')).unwrap_or(b.len());
            out.push(Token::Num(text[i..j].to_string()));
            i = j;
        } else {
            out.push(Token::Mark(c as char));
            i += 1;
        }
    }
    out
}

pub(crate) fn is_name(t: &Token, name: &str) -> bool {
    matches!(t, Token::Name(n) if n == name)
}

/// The end of the block that the opener at `start` opens (`function`, `if`, `do`, `repeat`: the
/// index just past its `end` or `until`).
pub(crate) fn block_end(t: &[Token], start: usize) -> usize {
    let mut depth = 0i32;
    for (k, tok) in t.iter().enumerate().skip(start) {
        if let Token::Name(n) = tok {
            match n.as_str() {
                "function" | "if" | "do" | "repeat" => depth += 1,
                "end" | "until" => {
                    depth -= 1;
                    if depth == 0 {
                        return k + 1;
                    }
                }
                _ => {}
            }
        }
    }
    t.len()
}

/// The bodies of the functions assigned to `name` (`name = function(...) ... end`), as token
/// ranges (the body after the parameters, its `end` left out).
pub(crate) fn functions(t: &[Token], name: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut k = from;
    while k + 2 < to {
        if is_name(&t[k], name) && t[k + 1] == Token::Mark('=') && is_name(&t[k + 2], "function") {
            let end = block_end(t, k + 2).min(to);
            // (The body begins after the parameters' closing bracket.)
            let body = (k + 3..end).find(|&j| t[j] == Token::Mark(')')).map_or(k + 3, |j| j + 1);
            out.push((body, end.saturating_sub(1)));
            k = end;
        } else {
            k += 1;
        }
    }
    out
}

/// The calls `method('<first>', <second>)` of any of `methods` in `t[from..to]`, in the order
/// they stand in: which of the methods it calls (its index), the first argument (a string) and the
/// second (`true`, `false` or none), and whether the call stands inside an `if` or loop of that
/// range.
pub(crate) fn calls(t: &[Token], methods: &[&str], from: usize, to: usize) -> Vec<(usize, String, Option<bool>, bool)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut k = from;
    while k < to {
        if let Token::Name(n) = &t[k] {
            match n.as_str() {
                "if" | "do" | "repeat" | "function" => depth += 1,
                "end" | "until" => depth -= 1,
                _ if t.get(k + 1) == Some(&Token::Mark('(')) => {
                    if let Some(which) = methods.iter().position(|m| n == m)
                        && let Some(Token::Str(first)) = t.get(k + 2)
                    {
                        let second = match (t.get(k + 3), t.get(k + 4)) {
                            (Some(Token::Mark(',')), Some(Token::Name(v))) if v == "true" => Some(true),
                            (Some(Token::Mark(',')), Some(Token::Name(v))) if v == "false" => Some(false),
                            _ => None,
                        };
                        out.push((which, first.clone(), second, depth > 0));
                    }
                }
                _ => {}
            }
        }
        k += 1;
    }
    out
}

/// What the unit script `text` switches off as the unit is made.
pub fn birth(text: &str) -> Birth {
    let t = tokens(text);
    let mut off: Vec<String> = Vec::new();
    let mut by_layer: Vec<String> = Vec::new();
    let mut intel_off: Vec<String> = Vec::new();
    let set = |list: &mut Vec<String>, name: &str, on: bool| {
        list.retain(|l| !l.eq_ignore_ascii_case(name));
        if !on {
            list.push(name.to_string());
        }
    };
    // The weapons' classes (`Weapons = { <Label> = Class(...) { ... }, ... }`): those whose own
    // OnCreate switches them off.
    if let Some(at) = (0..t.len().saturating_sub(2)).find(|&k| is_name(&t[k], "Weapons") && t[k + 1] == Token::Mark('=') && t[k + 2] == Token::Mark('{')) {
        let close = table_end(&t, at + 2);
        let mut k = at + 3;
        while k + 2 < close {
            if let Token::Name(label) = &t[k]
                && t[k + 1] == Token::Mark('=')
                && is_name(&t[k + 2], "Class")
            {
                // Its class body: the next `{` and its match.
                let Some(open) = (k + 3..close).find(|&j| t[j] == Token::Mark('{')) else { break };
                let end = table_end(&t, open);
                for (from, to) in functions(&t, "OnCreate", open, end) {
                    let disables = (from..to).any(|j| {
                        is_name(&t[j], "SetWeaponEnabled") && t.get(j + 1) == Some(&Token::Mark('(')) && t.get(j + 2).is_some_and(|v| is_name(v, "false"))
                    });
                    if disables {
                        set(&mut off, label, false);
                    }
                }
                k = end;
            } else {
                k += 1;
            }
        }
    }
    // The unit's own OnCreate, then OnStopBeingBuilt (the functions outside the weapons' table:
    // the last of each name, a script's own overriding).
    let weapons_table = (0..t.len().saturating_sub(2)).find(|&k| is_name(&t[k], "Weapons") && t[k + 1] == Token::Mark('=') && t[k + 2] == Token::Mark('{')).map(|at| (at, table_end(&t, at + 2)));
    let outside = |range: &(usize, usize)| weapons_table.is_none_or(|(a, b)| range.0 > b || range.1 < a);
    for name in ["OnCreate", "OnStopBeingBuilt"] {
        let Some(body) = functions(&t, name, 0, t.len()).into_iter().rfind(outside) else { continue };
        for (_, label, on, inside) in calls(&t, &["SetWeaponEnabledByLabel"], body.0, body.1) {
            match (on, inside) {
                (Some(on), false) => set(&mut off, &label, on),
                (Some(true), true) if !by_layer.iter().any(|l| l.eq_ignore_ascii_case(&label)) => by_layer.push(label),
                _ => {}
            }
        }
        // (Switched on and off in the order the script does it: the last call of a kind's
        // standing, an intel switched off and on again on.)
        for (which, kind, _, inside) in calls(&t, &["EnableUnitIntel", "DisableUnitIntel"], body.0, body.1) {
            if !inside {
                set(&mut intel_off, &kind, which == 0);
            }
        }
    }
    for body in functions(&t, "OnLayerChange", 0, t.len()) {
        for (_, label, on, _) in calls(&t, &["SetWeaponEnabledByLabel"], body.0, body.1) {
            if on == Some(true) && !by_layer.iter().any(|l| l.eq_ignore_ascii_case(&label)) {
                by_layer.push(label);
            }
        }
    }
    Birth { weapons_off: off, by_layer, intel_off }
}

/// The index just past the `}` closing the table opened at `open` (`t[open]` is `{`).
pub(crate) fn table_end(t: &[Token], open: usize) -> usize {
    let mut depth = 0i32;
    for (k, tok) in t.iter().enumerate().skip(open) {
        match tok {
            Token::Mark('{') => depth += 1,
            Token::Mark('}') => {
                depth -= 1;
                if depth == 0 {
                    return k + 1;
                }
            }
            _ => {}
        }
    }
    t.len()
}

/// What a unit script's `CreateEnhancement` does for one enhancement (its branch: `if enh ==
/// '<name>' then`, or `elseif`, several names to a branch with `or`): the weapons it switches on
/// and off (`SetWeaponEnabledByLabel`) and the intel (`EnableUnitIntel`, `DisableUnitIntel`), in
/// the branch and the blocks within it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnhancementScript {
    pub name: String,
    pub weapons_on: Vec<String>,
    pub weapons_off: Vec<String>,
    pub intel_on: Vec<String>,
    pub intel_off: Vec<String>,
    /// The orders it gives the unit (`AddCommandCap('RULEUCC_Teleport')`: the Personal Teleporter).
    pub command_caps: Vec<String>,
    /// The units it makes (`CreateUnitHPR('UEA0001', ...)`: the UEF commanders' pods), by id.
    pub units_made: Vec<String>,
}

/// What the unit script `text`'s `CreateEnhancement` (its last) does for each enhancement it names,
/// in the order they first come.
pub fn enhancements(text: &str) -> Vec<EnhancementScript> {
    let t = tokens(text);
    let Some((from, to)) = functions(&t, "CreateEnhancement", 0, t.len()).into_iter().last() else { return Vec::new() };
    let mut out: Vec<EnhancementScript> = Vec::new();
    let mut depth = 0i32;
    // The branch's names and the depth of its `if`.
    let mut current: Vec<String> = Vec::new();
    let mut branch_depth = i32::MAX;
    let mut k = from;
    while k < to {
        match &t[k] {
            Token::Name(n) if n == "if" || n == "elseif" => {
                if n == "if" {
                    depth += 1;
                }
                let then = (k + 1..to).find(|&j| is_name(&t[j], "then")).unwrap_or(to);
                let names: Vec<String> = (k + 1..then)
                    .filter_map(|j| match (&t[j], t.get(j + 1), t.get(j + 2), t.get(j + 3)) {
                        (Token::Name(e), Some(Token::Mark('=')), Some(Token::Mark('=')), Some(Token::Str(name))) if e == "enh" => Some(name.clone()),
                        _ => None,
                    })
                    .collect();
                if !names.is_empty() {
                    current = names;
                    branch_depth = depth;
                } else if depth == branch_depth {
                    // (An `elseif` of the chain naming none of them: no enhancement's branch.)
                    current.clear();
                }
                k = then;
            }
            Token::Name(n) if n == "else" => {
                if depth == branch_depth {
                    current.clear();
                }
            }
            Token::Name(n) if n == "do" || n == "function" || n == "repeat" => depth += 1,
            Token::Name(n) if n == "end" || n == "until" => {
                if depth == branch_depth {
                    current.clear();
                    branch_depth = i32::MAX;
                }
                depth -= 1;
            }
            Token::Name(n) if t.get(k + 1) == Some(&Token::Mark('(')) && !current.is_empty() => {
                let first = match t.get(k + 2) {
                    Some(Token::Str(s)) => Some(s.clone()),
                    _ => None,
                };
                let second = match (t.get(k + 3), t.get(k + 4)) {
                    (Some(Token::Mark(',')), Some(Token::Name(v))) if v == "true" => Some(true),
                    (Some(Token::Mark(',')), Some(Token::Name(v))) if v == "false" => Some(false),
                    _ => None,
                };
                if let Some(first) = first {
                    for name in &current {
                        let entry = match out.iter().position(|e| e.name.eq_ignore_ascii_case(name)) {
                            Some(i) => &mut out[i],
                            None => {
                                out.push(EnhancementScript { name: name.clone(), ..Default::default() });
                                out.last_mut().expect("just pushed")
                            }
                        };
                        let put = |list: &mut Vec<String>, v: &str| {
                            if !list.iter().any(|l| l.eq_ignore_ascii_case(v)) {
                                list.push(v.to_string());
                            }
                        };
                        match (n.as_str(), second) {
                            ("SetWeaponEnabledByLabel", Some(true)) => put(&mut entry.weapons_on, &first),
                            ("SetWeaponEnabledByLabel", Some(false)) => put(&mut entry.weapons_off, &first),
                            ("EnableUnitIntel", _) => put(&mut entry.intel_on, &first),
                            ("DisableUnitIntel", _) => put(&mut entry.intel_off, &first),
                            ("AddCommandCap", _) => put(&mut entry.command_caps, &first),
                            ("CreateUnitHPR", _) => entry.units_made.push(first.to_ascii_uppercase()),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
        k += 1;
    }
    out
}

/// The bone a unit's script builds its units at (`BuildAttachBone = 'Attachpoint'` in its class:
/// Forged Alliance's factories that move name theirs there), if it names one.
pub fn build_attach_bone(text: &str) -> Option<String> {
    let t = tokens(text);
    (0..t.len().saturating_sub(2)).find_map(|k| match (&t[k], &t[k + 1], &t[k + 2]) {
        (Token::Name(n), Token::Mark('='), Token::Str(bone)) if n == "BuildAttachBone" && !bone.trim().is_empty() => Some(bone.trim().to_string()),
        _ => None,
    })
}

/// What a projectile splits into as it lands: the child's blueprint (as written), how many of
/// them, and the share of the parent's damage each carries (1: all of it).
#[derive(Clone, Debug, PartialEq)]
pub struct Fragments {
    pub child: String,
    pub count: u32,
    pub share: f64,
}

/// What the projectile script `text` splits its projectile into, if it does.
pub fn fragments(text: &str) -> Option<Fragments> {
    let t = tokens(text);
    let impact = functions(&t, "OnImpact", 0, t.len()).into_iter().next()?;
    let (from, to) = impact;
    // The child's blueprint: `ChildProjectileBP = '<path>'`.
    let child = (from..to.saturating_sub(2)).find_map(|k| match (&t[k], &t[k + 1], &t[k + 2]) {
        (Token::Name(n), Token::Mark('='), Token::Str(path)) if n == "ChildProjectileBP" => Some(path.clone()),
        _ => None,
    })?;
    // `numProjectiles = N`.
    let many = (from..to.saturating_sub(2)).find_map(|k| match (&t[k], &t[k + 1], &t[k + 2]) {
        (Token::Name(n), Token::Mark('='), Token::Num(v)) if n == "numProjectiles" => v.parse::<u32>().ok(),
        _ => None,
    });
    // Each `CreateChildProjectile(` makes one, a loop's `numProjectiles` of them.
    let mut count = 0u32;
    let mut k = from;
    let mut loops: Vec<usize> = Vec::new();
    while k < to {
        match &t[k] {
            Token::Name(n) if n == "for" || n == "while" => {
                let end = block_end(&t, (k..to).find(|&j| is_name(&t[j], "do")).unwrap_or(k));
                loops.push(end);
            }
            Token::Name(n) if n == "CreateChildProjectile" && t.get(k + 1) == Some(&Token::Mark('(')) => {
                loops.retain(|&end| end > k);
                count += if loops.is_empty() { 1 } else { many.unwrap_or(1) };
            }
            _ => {}
        }
        k += 1;
    }
    if count <= 1 {
        return None;
    }
    // A script dividing its damage among them (`DamageAmount / numProjectiles`).
    let divided = (from..to.saturating_sub(2)).any(|k| is_name(&t[k], "DamageAmount") && t[k + 1] == Token::Mark('/') && is_name(&t[k + 2], "numProjectiles"));
    let share = match (divided, many) {
        (true, Some(n)) if n > 0 => 1.0 / n as f64,
        _ => 1.0,
    };
    Some(Fragments { child, count, share })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A commander's `CreateEnhancement`: branches by name (two to one with `or`), what each
    /// switches on and off, in blocks within it as well, and a branch for none of them.
    #[test]
    fn a_scripts_enhancements_are_read_branch_by_branch() {
        let script = r#"
Unit = Class(Base) {
    CreateEnhancement = function(self, enh)
        Base.CreateEnhancement(self, enh)
        local bp = self:GetBlueprint().Enhancements[enh]
        if enh == 'Missile' then
            self:SetWeaponEnabledByLabel('Missile', true)
        elseif enh =='Billy' then
            if not self.Done then
                self:SetWeaponEnabledByLabel('Missile', false)
            end
            self:SetWeaponEnabledByLabel('Nuke', true)
        elseif enh == 'Stealth' then
            self:EnableUnitIntel('RadarStealth')
            self:EnableUnitIntel('SonarStealth')
        elseif enh == 'StealthRemove' or enh == 'CloakRemove' then
            self:DisableUnitIntel('RadarStealth')
        elseif enh == 'Teleporter' then
            self:AddCommandCap('RULEUCC_Teleport')
        elseif enh == 'LeftPod' then
            local pod = CreateUnitHPR('uea0001', self:GetArmy(), 0, 0, 0, 0, 0, 0)
        elseif self.Other then
            self:SetWeaponEnabledByLabel('Gun', true)
        end
        self:SetWeaponEnabledByLabel('After', true)
    end,
}
"#;
        let e = enhancements(script);
        let find = |n: &str| e.iter().find(|x| x.name == n).unwrap_or_else(|| panic!("{n}: {e:?}"));
        assert_eq!(find("Missile").weapons_on, ["Missile"]);
        assert_eq!((find("Billy").weapons_on.clone(), find("Billy").weapons_off.clone()), (vec!["Nuke".to_string()], vec!["Missile".to_string()]), "within a block of its branch too");
        assert_eq!(find("Stealth").intel_on, ["RadarStealth", "SonarStealth"]);
        assert_eq!(find("CloakRemove").intel_off, ["RadarStealth"], "two to one branch");
        assert_eq!(find("Teleporter").command_caps, ["RULEUCC_Teleport"], "the orders it gives");
        assert_eq!(find("LeftPod").units_made, ["UEA0001"], "the units it makes");
        assert_eq!(build_attach_bone("Fatboy = Class(Factory) {\n    BuildAttachBone = 'Build_Attachpoint',\n}"), Some("Build_Attachpoint".to_string()));
        assert_eq!(build_attach_bone("Unit = Class(Base) {}"), None);
        assert!(!e.iter().any(|x| x.weapons_on.iter().any(|w| w == "Gun" || w == "After")), "a branch for none, and what comes after the chain: {e:?}");
        assert!(enhancements("Unit = Class(Base) { OnCreate = function(self) end, }").is_empty());
    }

    /// A unit made with a missile its class switches off, a toggled gun its OnCreate switches off,
    /// a gun its OnStopBeingBuilt switches back on, weapons switched by the layer it stands on,
    /// and stealth and sonar switched off once built (the Cybran commander's).
    const UNIT: &str = r#"
local Base = import('/lua/x.lua').Base
Thing = Class(Base) {
    Weapons = {
        MainGun = Class(Gun) {},
        Missile = Class(Launcher) {
            OnCreate = function(self)
                Launcher.OnCreate(self)
                self:SetWeaponEnabled(false)
            end,
        },
        OverCharge = Class(Cannon) {
            OnEnableWeapon = function(self)
                self.unit:SetWeaponEnabledByLabel('MainGun', false)
            end,
        },
        Bolter = Class(Gun) {},
        Torpedo = Class(Tube) {},
    },
    OnCreate = function(self)
        Base.OnCreate(self)
        self:SetWeaponEnabledByLabel('SniperGun', false)  # toggled on later
        self:SetWeaponEnabledByLabel('Rifle', false)
    end,
    OnStopBeingBuilt = function(self, builder, layer)
        Base.OnStopBeingBuilt(self, builder, layer)
        self:SetWeaponEnabledByLabel('Rifle', true)
        -- self:SetWeaponEnabledByLabel('MainGun', false)
        if layer == 'Land' then
            self:SetWeaponEnabledByLabel('Bolter', true)
            self:SetWeaponEnabledByLabel('Torpedo', false)
        elseif layer == 'Seabed' then
            self:SetWeaponEnabledByLabel('Bolter', false)
            self:SetWeaponEnabledByLabel('Torpedo', true)
        end
        self:DisableUnitIntel('RadarStealth')
        self:DisableUnitIntel('Sonar')
    end,
    OnLayerChange = function(self, new, old)
        if new == 'Seabed' then self:SetWeaponEnabledByLabel('Torpedo', true) end
    end,
}
"#;

    #[test]
    fn what_a_unit_switches_off_as_it_is_made() {
        let b = birth(UNIT);
        assert!(b.weapon_off("Missile"), "its class's OnCreate: {b:?}");
        assert!(b.weapon_off("snipergun"), "its OnCreate: {b:?}");
        assert!(!b.weapon_off("Rifle"), "switched back on once built: {b:?}");
        assert!(!b.weapon_off("MainGun"), "off only while the overcharge is: {b:?}");
        assert!(!b.weapon_off("Bolter") && !b.weapon_off("Torpedo"), "by the layer: {b:?}");
        assert!(b.intel_off("radarstealth") && b.intel_off("Sonar") && !b.intel_off("Radar"), "{b:?}");
        assert_eq!(birth("Unit = Class(Base) { OnCreate = function(self) end, }"), Birth::default());
    }

    /// An intel switched off and then on again as the unit is made stays on, and one switched on
    /// and then off stays off: the script's last word on it stands, whichever way it switches.
    #[test]
    fn intel_switched_off_and_on_again_is_on() {
        let script = |first: &str, then: &str| format!("Unit = Class(Base) {{\n    OnStopBeingBuilt = function(self, builder, layer)\n        self:{first}UnitIntel('Sonar')\n        self:{then}UnitIntel('Sonar')\n    end,\n}}");
        assert!(!birth(&script("Disable", "Enable")).intel_off("Sonar"));
        assert!(birth(&script("Enable", "Disable")).intel_off("Sonar"));
        // (Across the two: OnCreate's switch off undone by OnStopBeingBuilt's switch on.)
        let both = "Unit = Class(Base) {\n    OnCreate = function(self) self:DisableUnitIntel('Cloak') end,\n    OnStopBeingBuilt = function(self, builder, layer) self:EnableUnitIntel('Cloak') end,\n}";
        assert!(!birth(both).intel_off("Cloak"));
    }

    #[test]
    fn what_a_shell_splits_into() {
        // One along its path and five spread round (each with all of its parent's damage), as
        // the Salvation's shells do; a script dividing the damage among eight.
        let shell = r#"
Shell = Class(Base) {
    OnImpact = function(self, TargetType, TargetEntity)
        local ChildProjectileBP = '/projectiles/Frag02/Frag02_proj.bp'
        self:CreateChildProjectile(ChildProjectileBP):SetVelocity(vx, vy, vz):PassDamageData(self.DamageData)
        local numProjectiles = 5
        for i = 0, (numProjectiles -1) do
            local proj = self:CreateChildProjectile(ChildProjectileBP)
            proj:PassDamageData(self.DamageData)
        end
        self:Destroy()
    end,
}
"#;
        assert_eq!(fragments(shell), Some(Fragments { child: "/projectiles/Frag02/Frag02_proj.bp".into(), count: 6, share: 1.0 }));
        let divided = r#"
Missile = Class(Base) {
    OnImpact = function(self, TargetType, TargetEntity)
        local numProjectiles = 8
        local ChildProjectileBP = '/projectiles/M02/M02_proj.bp'
        self.DamageData.DamageAmount = self.DamageData.DamageAmount / numProjectiles
        for i = 0, (numProjectiles -1) do
            local proj = self:CreateChildProjectile(ChildProjectileBP)
        end
    end,
}
"#;
        assert_eq!(fragments(divided), Some(Fragments { child: "/projectiles/M02/M02_proj.bp".into(), count: 8, share: 0.125 }));
        // A commented-out child and none at all split nothing.
        assert_eq!(fragments("P = Class(B) { OnImpact = function(self) #self:CreateChildProjectile(X)\n end, }"), None);
        assert_eq!(fragments("P = Class(B) { }"), None);
    }
}
