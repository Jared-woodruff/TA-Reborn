//! Finished unit pose for the launcher studio. Like the game's standing portraits, run
//! Create/Activate in a bounded sandbox, suppressing sounds, effects and all world changes.

use glam::{Mat4, Quat, Vec3};
use ta_formats::{cob::CobScript, model3do::Model3do};
use ta_sim::cob::{CobHost, CobInstance, CobProgram, ThreadTag, TICKS_PER_SECOND};

pub fn standing(model: &Model3do, script: Option<CobScript>, active: bool) -> Vec<Mat4> {
    let definition = ta_sim::defs::ModelDef::from_3do(model);
    let mut states = vec![ta_sim::cob::PieceAnim::default(); model.pieces.len()];
    if let Some(script) = script {
        let names: Vec<_> = model.pieces.iter().map(|p| p.name.clone()).collect();
        let mut instance = CobInstance::new(std::sync::Arc::new(CobProgram::new(script, &names)));
        let mut host = Sandbox { now: 0, seed: 0x2545_F491, height: definition.height.to_cob(), active };
        instance.start("Create", &[], ThreadTag::None);
        // The VM bounds instructions, threads and stack as in the game. This is exactly
        // 120 ticks, never an unbounded wait for a third-party script to finish.
        for tick in 0..4 * TICKS_PER_SECOND {
            host.now = tick * 1000 / TICKS_PER_SECOND;
            instance.run(&mut host);
            if tick == 0 {
                let _ = instance.call(&mut host, "SetMaxReloadTime", &[1000]);
                if active { instance.start("Activate", &[], ThreadTag::None); instance.run(&mut host); }
            }
            instance.animate();
        }
        for (cob, piece) in instance.program.piece_map.iter().enumerate() {
            if let Some(piece) = piece && let Some(state) = instance.pieces.get(cob) { states[*piece] = state.clone(); }
        }
    }
    let angle = std::f32::consts::TAU / 65536.0;
    let mut matrices: Vec<Mat4> = Vec::with_capacity(model.pieces.len());
    for (piece, state) in definition.pieces.iter().zip(&states) {
        let [x, y, z] = state.offset;
        let offset = Vec3::new(piece.offset.x.to_f32() - x as f32 / 65536.0,
            piece.offset.y.to_f32() + y as f32 / 65536.0, piece.offset.z.to_f32() + z as f32 / 65536.0);
        let [x, y, z] = state.rotation;
        let rotation = Quat::from_rotation_y(y as f32 * angle) * Quat::from_rotation_x(x as f32 * angle) * Quat::from_rotation_z(-z as f32 * angle);
        let local = Mat4::from_rotation_translation(rotation, offset);
        matrices.push(piece.parent.map_or(local, |parent| matrices[parent] * local));
    }
    // A hidden parent still positions its visible children, matching the game's hierarchy.
    for (matrix, state) in matrices.iter_mut().zip(&states) { if !state.visible { *matrix = Mat4::ZERO; } }
    matrices
}

struct Sandbox { now: i64, seed: u32, height: i32, active: bool }
impl CobHost for Sandbox {
    fn now_ms(&self) -> i64 { self.now }
    fn random(&mut self, lo: i32, hi: i32) -> i32 {
        self.seed ^= self.seed << 13; self.seed ^= self.seed >> 17; self.seed ^= self.seed << 5;
        if hi <= lo { lo } else { (lo as i64 + (self.seed as i64).rem_euclid(hi as i64 - lo as i64 + 1)) as i32 }
    }
    fn get_value(&mut self, _: &CobInstance, id: i32, args: [i32; 4]) -> i32 {
        use ta_sim::host::value::*;
        match id { ACTIVATION => self.active as i32, STANDING_MOVE_ORDERS => 1, STANDING_FIRE_ORDERS => 2,
            HEALTH => 100, UNIT_HEIGHT => self.height,
            HYPOT => (args[0] as f64).hypot(args[1] as f64).min(i32::MAX as f64) as i32,
            ATAN => ta_sim::host::script_angle(((args[0] as f64).atan2(args[1] as f64) * 32768.0 / std::f64::consts::PI) as i32),
            _ => 0 }
    }
    fn set_value(&mut self, _: i32, _: i32) {}
    fn explode(&mut self, _: &CobInstance, _: usize, _: i32) {}
    fn emit_sfx(&mut self, _: &CobInstance, _: usize, _: i32) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_script_keeps_the_hierarchy_and_axis_convention() {
        let piece = |name: &str, parent, offset| ta_formats::model3do::Piece { name: name.into(), parent, offset,
            vertices: Vec::new(), primitives: Vec::new(), selection_primitive: None, children: Vec::new() };
        let model = Model3do { pieces: vec![piece("base", None, [65536, 0, 131072]), piece("child", Some(0), [0, 65536, 65536])] };
        let pose = standing(&model, None, false);
        assert_eq!(pose[0].transform_point3(Vec3::ZERO), Vec3::new(1.0, 0.0, -2.0));
        assert_eq!(pose[1].transform_point3(Vec3::ZERO), Vec3::new(1.0, 1.0, -3.0));
    }
    #[test]
    #[ignore = "reads the locally installed original game"]
    fn installed_unit_create_hides_muzzle_flashes() {
        let mut vfs = ta_vfs::Vfs::new(); vfs.mount_game_dir(std::path::Path::new("C:/GOG Games/Total Annihilation"), "base").unwrap();
        for name in ["ARMSTUMP", "CORRAID"] {
            let model = Model3do::parse(&vfs.read(&format!("objects3d/{name}.3do")).unwrap()).unwrap();
            let script = CobScript::parse(&vfs.read(&format!("scripts/{name}.cob")).unwrap()).unwrap();
            let pose = standing(&model, Some(script), false);
            assert!(pose.iter().any(|m| *m == Mat4::ZERO), "{name} must hide its transient effect pieces");
            assert!(pose.iter().any(|m| *m != Mat4::ZERO), "{name} must remain visible");
        }
    }
}
