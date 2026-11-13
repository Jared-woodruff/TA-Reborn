//! A unit as it stands finished (for its portrait, `hud.rs`): its script's `Create` run in a
//! sandbox, and `Activate` for a unit switched on when built, until its pieces come to rest:
//! a solar collector opened, a radar dish up, its muzzle flares hidden. Nothing here touches
//! the simulation.

use glam::Mat4;
use ta_sim::cob::{CobInstance, TICKS_PER_SECOND, ThreadTag};
use ta_sim::{FxVec3, Unit, UnitId, UnitType};

use super::walk::Sandbox;

/// How long the script is given to settle (ticks).
const SETTLE_TICKS: i64 = 4 * TICKS_PER_SECOND;

/// The pieces' matrices of a `t` standing finished, its origin at the world's (pieces the script
/// hides zeroed, as the scene draws them).
pub fn standing_pose(t: &UnitType) -> Vec<Mat4> {
    let mut unit = Unit::new(UnitId { slot: u32::MAX, generation: 0 }, t.id, 0, FxVec3::ZERO);
    if let Some(program) = t.script.clone() {
        let mut script = CobInstance::new(program);
        let active = t.def.activate_when_built;
        let mut host = Sandbox { now: 0, seed: 0x2545_F491, height: t.model.height.to_cob(), active };
        script.start("Create", &[], ThreadTag::None);
        for tick in 0..SETTLE_TICKS {
            host.now = tick * 1000 / TICKS_PER_SECOND;
            script.run(&mut host);
            if tick == 0 {
                // (As the game tells a new unit: its reload, its being switched on.)
                let _ = script.call(&mut host, "SetMaxReloadTime", &[1000]);
                if active {
                    script.start("Activate", &[], ThreadTag::None);
                    script.run(&mut host);
                }
            }
            script.animate();
        }
        unit.cob = Some(script);
    }
    let mut out = Vec::new();
    super::piece_matrices(&unit, t, Mat4::IDENTITY, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pieces each unit's portrait shows and hides, as `standing_pose` stands it (the
    /// install's units, and the mod's `TA_TEST_MOD=<mod folder>` mounted over it; `TA_UNITS=a,b`
    /// for some): `cargo test --release -p ta_game --bin ta-reborn still::tests::standing_poses --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "reads the install"]
    fn standing_poses() {
        let Ok(install) = ta_vfs::install::discover(None) else { return };
        let module = std::env::var("TA_TEST_MOD").ok().map(|dir| ta_vfs::mods::read_mod(std::path::Path::new(&dir)).unwrap().expect("a mod's folder"));
        let mounted = ta_vfs::mods::mount(Some(&install.root), ta_vfs::builtin_dir().as_deref(), &module.iter().collect::<Vec<_>>(), &|m| m.manifest.id.clone());
        let vfs = mounted.vfs;
        let only: Option<Vec<String>> = std::env::var("TA_UNITS").ok().map(|v| v.split(',').map(|s| s.trim().to_ascii_uppercase()).collect());
        let data = ta_data::GameData::load(&vfs);
        let types = ta_sim::UnitTypes::compile(&data, &|p| vfs.read_opt(p));
        for t in types.types.iter().filter(|t| only.as_ref().is_none_or(|o| o.contains(&t.name))) {
            let pose = standing_pose(t);
            let (shown, hidden): (Vec<_>, Vec<_>) = t.model.pieces.iter().zip(&pose).partition(|(_, m)| **m != Mat4::ZERO);
            let names = |v: Vec<(&ta_sim::defs::PieceDef, &Mat4)>| v.iter().map(|(p, _)| p.name.clone()).collect::<Vec<_>>().join(" ");
            println!("{}: shown {}\n{:width$}  hidden {}", t.name, names(shown), "", names(hidden), width = t.name.len());
        }
    }
}
