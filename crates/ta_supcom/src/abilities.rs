//! What Forged Alliance's unit scripts say of the abilities TA Reborn plays for mods (docs/MODDING.md,
//! "Abilities"), read as text as [`crate::behaviour`] reads them; nothing is run:
//!
//! - **A weapon toggle** ([`WeaponToggle`]): the weapons its `OnScriptBitSet` switches on and off
//!   for its weapon toggle's bit (`if bit == 1 then ... SetWeaponEnabledByLabel('<label>', true|false)`):
//!   the sniper bot's long gun, the Cybran anti-air's and cruisers' ground guns.
//! - **Stuns switched off at birth** ([`Scripted::stun_off`]): a weapon's class whose `OnCreate`
//!   disables its stun buff (`self:DisableBuff('STUN')`), and the enhancements that switch it back on
//!   ([`Scripted::stun_on`]: `GetWeaponByLabel('<label>')` and `ReEnableBuff('STUN')` in its branch of
//!   `CreateEnhancement`): the Cybran support commander's EMP Burst.
//! - **Remote viewing switched off** ([`Scripted::remote_view_off`]): `DisableRemoteViewingButtons()`
//!   as the unit is made (`OnCreate`, `OnStopBeingBuilt`): the Seraphim commander's, whose remote
//!   viewing enhancement the game leaves out.

use crate::behaviour::{Token, block_end, calls, functions, is_name, table_end, tokens};

/// A unit's weapon toggle as its script switches it: the weapons (labels as written) on and off
/// while it is on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeaponToggle {
    pub on: Vec<String>,
    pub off: Vec<String>,
}

/// What a unit's script says of TA Reborn's abilities.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scripted {
    pub toggle: Option<WeaponToggle>,
    /// Weapons (labels) whose stun is off once the unit is made.
    pub stun_off: Vec<String>,
    /// The enhancements that switch a weapon's stun on: (the enhancement, the weapon's label).
    pub stun_on: Vec<(String, String)>,
    pub remote_view_off: bool,
}

impl Scripted {
    /// Whether the stun of weapon `label` is off once the unit is made.
    pub fn stun_off(&self, label: &str) -> bool {
        self.stun_off.iter().any(|l| l.eq_ignore_ascii_case(label))
    }

    /// The weapon whose stun enhancement `name` switches on, if it does.
    pub fn stun_on(&self, name: &str) -> Option<&str> {
        self.stun_on.iter().find(|(e, _)| e.eq_ignore_ascii_case(name)).map(|(_, w)| w.as_str())
    }
}

/// What the unit script `text` says of the abilities.
pub fn scripted(text: &str) -> Scripted {
    let t = tokens(text);
    Scripted { toggle: toggle(&t), stun_off: stun_off(&t), stun_on: stun_on(&t), remote_view_off: remote_view_off(&t) }
}

/// The weapons the last `OnScriptBitSet` switches for its weapon toggle's bit (1).
fn toggle(t: &[Token]) -> Option<WeaponToggle> {
    let (from, to) = functions(t, "OnScriptBitSet", 0, t.len()).into_iter().last()?;
    let mut out = WeaponToggle::default();
    let mut k = from;
    while k + 5 < to {
        // (`if bit == 1 then`.)
        let bit_one = is_name(&t[k], "if") && is_name(&t[k + 1], "bit") && t[k + 2] == Token::Mark('=') && t[k + 3] == Token::Mark('=') && t[k + 4] == Token::Num("1".into()) && is_name(&t[k + 5], "then");
        if bit_one {
            let end = block_end(t, k).min(to);
            for (_, label, on, _) in calls(t, &["SetWeaponEnabledByLabel"], k + 6, end) {
                match on {
                    Some(true) => out.on.push(label),
                    Some(false) => out.off.push(label),
                    None => {}
                }
            }
            k = end;
        } else {
            k += 1;
        }
    }
    (!out.on.is_empty()).then_some(out)
}

/// The weapons (labels) of the script's `Weapons` table whose class's `OnCreate` disables its stun.
fn stun_off(t: &[Token]) -> Vec<String> {
    let mut out = Vec::new();
    let Some(at) = (0..t.len().saturating_sub(2)).find(|&k| is_name(&t[k], "Weapons") && t[k + 1] == Token::Mark('=') && t[k + 2] == Token::Mark('{')) else { return out };
    let close = table_end(t, at + 2);
    let mut k = at + 3;
    while k + 2 < close {
        if let Token::Name(label) = &t[k]
            && t[k + 1] == Token::Mark('=')
            && is_name(&t[k + 2], "Class")
        {
            let Some(open) = (k + 3..close).find(|&j| t[j] == Token::Mark('{')) else { break };
            let end = table_end(t, open);
            for (from, to) in functions(t, "OnCreate", open, end) {
                if calls(t, &["DisableBuff"], from, to).iter().any(|(_, buff, _, _)| buff.eq_ignore_ascii_case("STUN")) {
                    out.push(label.clone());
                }
            }
            k = end;
        } else {
            k += 1;
        }
    }
    out
}

/// The enhancements whose branch of the last `CreateEnhancement` re-enables a weapon's stun: the
/// enhancement its branch names last before (`enh == '<name>'`), the weapon it took by its label.
fn stun_on(t: &[Token]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some((from, to)) = functions(t, "CreateEnhancement", 0, t.len()).into_iter().last() else { return out };
    let (mut enhancement, mut weapon): (Option<String>, Option<String>) = (None, None);
    for k in from..to {
        match (&t[k], t.get(k + 1), t.get(k + 2), t.get(k + 3)) {
            (Token::Name(e), Some(Token::Mark('=')), Some(Token::Mark('=')), Some(Token::Str(name))) if e == "enh" => {
                enhancement = Some(name.clone());
                weapon = None;
            }
            (Token::Name(n), Some(Token::Mark('(')), Some(Token::Str(label)), _) if n == "GetWeaponByLabel" => weapon = Some(label.clone()),
            (Token::Name(n), Some(Token::Mark('(')), Some(Token::Str(buff)), _) if n == "ReEnableBuff" && buff.eq_ignore_ascii_case("STUN") => {
                if let (Some(e), Some(w)) = (&enhancement, &weapon) {
                    out.push((e.clone(), w.clone()));
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether the script switches its unit's remote viewing off as it is made.
fn remote_view_off(t: &[Token]) -> bool {
    ["OnCreate", "OnStopBeingBuilt"].iter().any(|name| {
        functions(t, name, 0, t.len()).into_iter().any(|(from, to)| (from..to).any(|k| is_name(&t[k], "DisableRemoteViewingButtons") && t.get(k + 1) == Some(&Token::Mark('('))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sniper's toggle in its weapon toggle's bit, another bit's switches left out; a stun its
    /// weapon's class switches off at birth and an enhancement switches back on; remote viewing
    /// switched off once built.
    #[test]
    fn what_a_script_says_of_the_abilities() {
        let script = r#"
Unit = Class(Base) {
    Weapons = {
        Gun = Class(Rifle) {
            OnCreate = function(self)
                Rifle.OnCreate(self)
                self:DisableBuff('STUN')
            end,
        },
        Long = Class(Rifle) {},
    },
    OnStopBeingBuilt = function(self, builder, layer)
        Base.OnStopBeingBuilt(self, builder, layer)
        self:DisableRemoteViewingButtons()
    end,
    OnScriptBitSet = function(self, bit)
        if bit == 8 then
            self:SetWeaponEnabledByLabel('Cloak', false)
        end
        if bit == 1 then
            self:SetWeaponEnabledByLabel('Long', true)
            self:SetWeaponEnabledByLabel('Gun', false)
        end
    end,
    CreateEnhancement = function(self, enh)
        if enh == 'Emp' then
            local wep = self:GetWeaponByLabel('Gun')
            wep:ReEnableBuff('STUN')
        elseif enh == 'EmpRemove' then
            local wep = self:GetWeaponByLabel('Gun')
            wep:DisableBuff('STUN')
        end
    end,
}
"#;
        let s = scripted(script);
        assert_eq!(s.toggle, Some(WeaponToggle { on: vec!["Long".into()], off: vec!["Gun".into()] }));
        assert!(s.stun_off("gun") && !s.stun_off("Long"));
        assert_eq!(s.stun_on("emp"), Some("Gun"));
        assert_eq!(s.stun_on("EmpRemove"), None);
        assert!(s.remote_view_off);
        assert_eq!(scripted("Unit = Class(Base) { OnCreate = function(self) end, }"), Scripted::default());
    }
}
