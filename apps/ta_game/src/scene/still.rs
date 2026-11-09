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
