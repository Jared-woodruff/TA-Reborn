//! The factions' mod made whole: every file, in a fixed order, handed to a writer (a folder, an
//! archive, a test), from the player's copy of Forged Alliance. What `ta-tools supcom-units` and
//! the launcher make.

use ta_formats::surface::Material;

use super::{anims, enhance, faction, model, pictures, script, textures, units, work, UNIT_SCALE};
use crate::files::Files;
use crate::scm::{self, Mesh};
use crate::sca;
use crate::unit::{Faction, UnitBlueprint};
use crate::Result;

/// The mod's id and name.
pub const MOD_ID: &str = "supcom-factions";
pub const MOD_NAME: &str = "Supreme Commander factions";
/// The mod's words and tags as its manifest keeps them, in English.
pub const DESCRIPTION: &str = "Supreme Commander: Forged Alliance's four factions, their units and buildings, made from your own copy of the game.";
pub const TAGS: [&str; 3] = ["units", "factions", "supreme commander"];
/// The converter's version, the mod's own (its manifest's `version`): a mod made by an older one
/// is offered to be made again. Raised whenever what is made changes (8: buildings' work and
/// upgrade animations; 9: buildings' floats left out where Forged Alliance hides them, on the
/// ground; 10: the paint as large as Forged Alliance paints it, its 1024s kept rather than halved
/// to 512, smaller textures enlarged keeping their edges sharp, the colour maps BC1, the
/// Seraphim's player's colour glowing; 11: factories' ways out, `[EXITS]` from their roll-off
/// points, their yards solid; weapons that unpack before they fire play their unpack animations in
/// their scripts, from their first frames, those that lock their unit's motion holding it still;
/// anti-air, the towers' and the aircraft's, takes aircraft alone, by order too,
/// `OnlyTargetCategory<n>`;
/// 12: the structures' adjacency, `[ADJACENCY]` and
/// `AdjacencySize`, from the buffs of the player's own copy; the engineering stations working round
/// them by themselves, `DefaultMissionType=AssistPatrol`, by their drones' work and reach; and the
/// units Forged Alliance makes only by upgrading made, so that what upgrades into them says so: the
/// tech 3 Kennel, the Hives of tech 2 and 3, the Cybran shield generator's four upgrades;
/// 13: units of a faction's different tiers sharing a name shown with their tiers, "T2 Engineer"
/// (`units::shown_name`), and every wreck by its unit's name, never its id).
pub const VERSION: u32 = 13;

/// What was made.
#[derive(Clone, Debug, Default)]
pub struct Made {
    pub units: usize,
    pub files: usize,
    pub bytes: u64,
    /// Units left out, and why.
    pub skipped: Vec<(String, String)>,
    /// Stopped before every unit was made (the watch said so): the files written are no whole mod.
    pub stopped: bool,
}

/// The mod's manifest: it needs a TA Reborn as new as the one that made it, `reborn` (models with
/// surfaces of their own, side add-ons).
pub fn manifest(reborn: &str) -> String {
    let tags: Vec<String> = TAGS.iter().map(|t| format!("\"{t}\"")).collect();
    format!(
        "id = \"{MOD_ID}\"\nname = \"{MOD_NAME}\"\nversion = \"{VERSION}\"\nrequires_reborn = \"{reborn}\"\ndescription = \"{DESCRIPTION}\"\ntags = [{}]\n",
        tags.join(", ")
    )
}

/// Makes the mod of `units` (the playable ones of the player's copy), for TA Reborn `reborn` and
/// newer, writing each file through `write` (a path in the mod, its bytes), in a fixed order: the
/// same bytes on every computer. `watch` is told before each unit how many are made and the
/// unit's id: false from it stops the making there ([`Made::stopped`]).
pub fn make(files: &Files, units: &[UnitBlueprint], reborn: &str, write: &mut dyn FnMut(&str, &[u8]) -> Result<()>, watch: &mut dyn FnMut(usize, &str) -> bool) -> Result<Made> {
    let mut made = Made::default();
    let mut put = |path: &str, bytes: &[u8], made: &mut Made| -> Result<()> {
        made.files += 1;
        made.bytes += bytes.len() as u64;
        write(path, bytes)
    };
    put("mod.toml", manifest(reborn).as_bytes(), &mut made)?;
    // What each structure gives the structures beside it, as the player's copy has it.
    let adjacency = crate::adjacency::read(files)?;
    let adjacency_categories = adjacency.categories();
    // (Each unit made, with the footprint its model was given: its wreck lies on as much.)
    let mut done: Vec<std::borrow::Cow<UnitBlueprint>> = Vec::new();
    for (i, u) in units.iter().enumerate() {
        if !watch(i, &u.id) {
            made.stopped = true;
            made.units = done.len();
            return Ok(made);
        }
        // (What it upgrades into in place, and the units its enhancements add: of its faction,
        // among those made. An enhancement's picture where the game has it.)
        let faction: Vec<&UnitBlueprint> = units.iter().filter(|o| o.faction == u.faction).collect();
        let own = units::weapons(u);
        let extra_weapons = enhance::extra_weapons(u, &own);
        let (mut list, _) = enhance::enhancements(u, &faction, &own, &extra_weapons);
        let mut icons: Vec<(String, Vec<u8>)> = Vec::new();
        for e in &mut list {
            let icon = match &e.icon {
                Some(path) => files.read(path)?,
                None => None,
            };
            match icon.and_then(|bytes| pictures::picture(&bytes, None).ok()) {
                Some(picture) => icons.push((format!("unitpics/{}.pcx", enhance::picture_name(u, e)), picture)),
                None => e.icon = None,
            }
        }
        let extra = units::Extra {
            upgrades_to: faction::upgrade_of(u, &faction).map(|t| t.id.as_str()),
            enhancements: enhance::section(u, &list),
            weapons: extra_weapons.clone(),
            exits: Vec::new(),
            adjacency: u.adjacency.as_deref().map_or(&[][..], |table| adjacency.of(table)),
            adjacency_categories: Some(&adjacency_categories),
            name: units::shown_name(u, &faction),
        };
        let shown: Vec<Vec<String>> = list.iter().map(|e| enhance::shown_bones(u, e)).collect();
        match unit(files, u, &extra, &extra_weapons, shown, &mut |path, bytes| put(path, bytes, &mut made))? {
            Ok(footprint) => {
                for (path, picture) in &icons {
                    put(path, picture, &mut made)?;
                }
                done.push(match footprint {
                    None => std::borrow::Cow::Borrowed(u),
                    Some(footprint) => std::borrow::Cow::Owned(UnitBlueprint { footprint: Some(footprint), ..u.clone() }),
                });
            }
            Err(why) => made.skipped.push((u.id.clone(), why)),
        }
    }
    made.units = done.len();
    // The factions' files.
    let factions: Vec<(Faction, Vec<&UnitBlueprint>)> = Faction::PLAYABLE.into_iter().map(|f| {
        let theirs = done.iter().map(|u| u.as_ref()).filter(|u| u.faction == f).collect();
        (f, theirs)
    }).filter(|(_, u): &(Faction, Vec<&UnitBlueprint>)| !u.is_empty()).collect();
    put("gamedata/sidedata_supcom.tdf", faction::side_add_on(&factions).as_bytes(), &mut made)?;
    for (f, theirs) in &factions {
        let name = units::side(f).to_ascii_lowercase();
        put(&format!("download/supcom_{name}.tdf"), faction::menus(theirs).as_bytes(), &mut made)?;
        put(&format!("features/corpses/supcom_{name}.tdf"), faction::wrecks(theirs).as_bytes(), &mut made)?;
        put(&format!("weapons/supcom_{name}.tdf"), faction::weapons(theirs).as_bytes(), &mut made)?;
    }
    Ok(made)
}

/// What a unit's files are made from, read whole before any is written: a unit whose files do
/// not read is left out with nothing of it written.
struct Read {
    mesh: Mesh,
    paint: [Option<Vec<u8>>; 3],
    far: Option<Mesh>,
    walk: Option<sca::Animation>,
    deaths: Vec<sca::Animation>,
    /// What it plays as it works ([`work::clips`]): those the game has.
    work: Vec<(work::Source, sca::Animation)>,
    /// What its weapons that unpack play (`WeaponUnpackAnimation`), by the game's path: those the
    /// game has.
    unpacks: Vec<(String, sca::Animation)>,
    icon: Option<Vec<u8>>,
    frame: Option<Vec<u8>>,
}

/// Reads what the unit's files are made from (what it plays as it works with the upgrade's where
/// it `upgrades` in place): `Err(why)` when one does not read (a file the archive cannot give, a
/// mesh that is no mesh).
fn read(files: &Files, u: &UnitBlueprint, upgrades: bool) -> std::result::Result<Read, String> {
    let get = |path: &str| files.read(path).map_err(|e| format!("{path}: {e}"));
    let Some(lod0) = u.display.lods.first() else { return Err("no mesh".into()) };
    let Some(bytes) = get(&lod0.mesh)? else { return Err(format!("no {}", lod0.mesh)) };
    let mesh = scm::parse(&bytes).map_err(|e| format!("{}: {e}", lod0.mesh))?;
    let paint = [get(&lod0.albedo)?, get(&lod0.specular)?, get(&lod0.normals)?];
    // (A far mesh that is no mesh is only left out: the near one draws far off too.)
    let far = match u.display.lods.get(1) {
        Some(lod1) => get(&lod1.mesh)?.and_then(|b| scm::parse(&b).ok()),
        None => None,
    };
    let animation = |path: &str| -> std::result::Result<Option<sca::Animation>, String> { Ok(get(path)?.and_then(|b| sca::parse(&b).ok())) };
    let walk = match &u.display.walk {
        Some((path, _)) => animation(path)?,
        None => None,
    };
    let mut deaths = Vec::new();
    for path in &u.display.deaths {
        deaths.extend(animation(path)?);
    }
    // (One the game has not, or that does not read, is no clip: some are named and missing.)
    let mut work = Vec::new();
    for source in work::clips(u, upgrades) {
        if let Some(a) = animation(&source.path)? {
            work.push((source, a));
        }
    }
    let mut unpacks: Vec<(String, sca::Animation)> = Vec::new();
    for path in u.weapons.iter().filter_map(|w| w.unpack.as_ref()?.animation.clone()) {
        if !unpacks.iter().any(|(p, _)| *p == path)
            && let Some(a) = animation(&path)?
        {
            unpacks.push((path, a));
        }
    }
    let icon = get(&pictures::icon_path(u))?;
    let frame = match pictures::frame_path(u) {
        Some(p) => get(&p)?,
        None => None,
    };
    Ok(Read { mesh, paint, far, walk, deaths, work, unpacks, icon, frame })
}

/// One unit's files: its model with its surface and its model for far off, its paint, its build
/// picture, its unit file and its script; the footprint (ogrids) its model was given where it is
/// not its blueprint's (`units::fitted_footprint`). `Err(why)` (inside the Ok) when it is left
/// out: what it is made of does not read, or its model cannot be written (nothing of it written
/// then).
fn unit(files: &Files, u: &UnitBlueprint, extra: &units::Extra, extra_weapons: &[&crate::unit::WeaponBlueprint], shown: Vec<Vec<String>>, put: &mut dyn FnMut(&str, &[u8]) -> Result<()>) -> Result<std::result::Result<Option<[f64; 2]>, String>> {
    let read = match read(files, u, extra.upgrades_to.is_some()) {
        Ok(r) => r,
        Err(why) => return Ok(Err(why)),
    };
    let lod0 = &u.display.lods[0];
    let mesh = &read.mesh;
    // Forged Alliance hides a building's floats (its `Floatation` bones) where it stands on land,
    // and shows them where it floats. TA Reborn stands every building but a floater (on the water
    // alone, `Floater`) on the ground, on land or on the sea bed: its floats are left out of its
    // model, and so of its wreck, its unit's own model at rest (drawn, they were dark slabs under
    // the wrecks of the air factories, the radars, the anti-air and others, 29 buildings), and of
    // what of it stands on the ground.
    let floats = if u.motion == crate::unit::Motion::None && !(u.layers.water && !u.layers.land) { model::floats(mesh) } else { Vec::new() };
    // A building's footprint covers what of its model stands on the ground.
    let footprint = units::fitted_footprint(u, model::ground_extent_leaving_out(mesh, u.display.uniform_scale, &floats));
    let fitted = footprint.map(|f| UnitBlueprint { footprint: Some(f), ..u.clone() });
    let u = fitted.as_ref().unwrap_or(u);
    let name = u.id.to_ascii_lowercase();
    // Its paint: each texture made the surface's (as large as Forged Alliance paints it, 512 to
    // 1024 texels a side), or a plain one where it has none; the colour map as bright as Forged
    // Alliance lights it (by its shader); the Seraphim's team colour moved from their albedo's
    // alpha into the specular map's, glowing as their ramp of light does; the colour map then BC1
    // (its alpha not drawn: half the bytes).
    let mut paint: Vec<(String, Vec<u8>)> = Vec::new();
    for ((kind, plain), source) in [("albedo", textures::PLAIN_ALBEDO), ("specteam", textures::PLAIN_SPECULAR), ("normals", textures::PLAIN_NORMALS)].into_iter().zip(&read.paint) {
        let bytes = match source {
            Some(b) => textures::texture(b).unwrap_or_else(|_| textures::plain(plain)),
            None => textures::plain(plain),
        };
        let bytes = if kind == "albedo" && source.is_some() { textures::brightened(&bytes, textures::paint_gain(&lod0.shader)) } else { bytes };
        paint.push((format!("textures/supcom/{name}_{kind}.dds"), bytes));
    }
    if lod0.shader.eq_ignore_ascii_case("seraphim") {
        match textures::team_from_alpha(&paint[1].1, &paint[0].1, textures::SERAPHIM_GLOW) {
            Ok(both) => paint[1].1 = both,
            Err(e) => return Ok(Err(format!("{}: {e}", lod0.specular))),
        }
    }
    let material = Material { name: name.clone(), albedo: paint[0].0.clone(), specular: paint[1].0.clone(), normals: paint[2].0.clone(), mean: textures::mean(&paint[0].1) };
    paint[0].1 = textures::opaque(&paint[0].1);
    let mut near = match model::convert_leaving_out(mesh, u.display.uniform_scale, material.clone(), &floats) {
        Ok(m) => m,
        Err(why) => return Ok(Err(format!("{}: {why}", lod0.mesh))),
    };
    // (A far model that cannot be written is left out: the near one draws far off too.)
    let far = read.far.as_ref().and_then(|far_mesh| model::convert_onto_leaving_out(far_mesh, mesh, &near, u.display.uniform_scale, material, &floats).ok());
    for (path, bytes) in &paint {
        put(path, bytes)?;
    }
    if let Some(far) = far {
        let far_name = format!("{name}_lod1");
        put(&format!("objects3d/{far_name}.3do"), &far.model.to_bytes())?;
        put(&format!("objects3d/{far_name}.surface"), &far.surface.to_bytes())?;
        near.surface.far = far_name;
    }
    put(&format!("objects3d/{name}.3do"), &near.model.to_bytes())?;
    put(&format!("objects3d/{name}.surface"), &near.surface.to_bytes())?;
    // Its walk, its deaths and what it plays as it works (the far model has the same pieces: they
    // play on it as well); a clip turning what a weapon aims is not made (`work::turns_an_aim`).
    let at_work: Vec<anims::Work> = read.work.iter().map(|(s, a)| anims::Work { name: s.name, animation: a, rate: s.rate, cycle: s.cycle }).collect();
    let mut clips = anims::anims(mesh, &near.bones, u.display.uniform_scale * UNIT_SCALE, read.walk.as_ref(), &read.deaths, &at_work);
    clips.clips.retain(|c| !(matches!(c.name.as_str(), ta_formats::anims::ACTIVE | ta_formats::anims::OPEN) && work::turns_an_aim(u, mesh, &near.bones, c)));
    if !clips.clips.is_empty() {
        put(&format!("objects3d/{name}.anims"), &clips.to_bytes())?;
    }
    // Its build picture.
    if let Some(icon) = &read.icon
        && let Ok(picture) = pictures::picture(icon, read.frame.as_deref())
    {
        put(&format!("unitpics/{name}.pcx"), &picture)?;
    }
    // Its unit file and its script (a factory's ways out from its model: `units::exits`).
    let weapons = units::weapons(u);
    let extra = units::Extra { exits: units::exits(u, mesh, &floats), ..extra.clone() };
    put(&format!("units/{name}.fbi"), units::unit_file(u, &weapons, units::transport(mesh), &extra).as_bytes())?;
    let builds = units::builds(u);
    let pieces: Vec<String> = near.model.pieces.iter().map(|p| p.name.clone()).collect();
    let motions = work::motions(u, mesh, &pieces, u.display.uniform_scale * UNIT_SCALE);
    // (Its own weapons, then those its enhancements switch on, in the slots after them.)
    let slots: Vec<&crate::unit::WeaponBlueprint> = weapons.slots.iter().copied().chain(extra_weapons.iter().copied()).take(units::MAX_WEAPONS).collect();
    // What those that unpack before they fire play: their animations as keys of the model's
    // pieces (`script::Unpacking`), but for the parts its own clips move as it works (they would
    // be moved twice: the Aeon silo's arm, held open by its opening and in its launch sequence).
    let worked: Vec<u32> = clips.clips.iter().filter(|c| matches!(c.name.as_str(), ta_formats::anims::ACTIVE | ta_formats::anims::OPEN)).flat_map(|c| c.pieces.iter().copied()).collect();
    let unpacks = slots
        .iter()
        .map(|w| unpacking(w, &read.unpacks, mesh, &near.bones, u.display.uniform_scale * UNIT_SCALE).and_then(|up| up.leaving_out(&worked)))
        .collect();
    let plan = script::Plan {
        unit: u,
        mesh,
        pieces,
        weapons: slots,
        factory: builds == units::Builds::Factory,
        // (An air staging platform's repair work sprays as a builder's does.)
        builder: builds != units::Builds::No || u.is("AIRSTAGINGPLATFORM"),
        pixels: u.display.uniform_scale * UNIT_SCALE,
        enhancements: shown,
        motions,
        switched: units::activated_when_built(u),
        unpacks,
    };
    put(&format!("scripts/{name}.cob"), &script::script(&plan).to_bytes())?;
    Ok(Ok(footprint))
}

/// What weapon `w` plays to unpack before it fires, where it unpacks and the game has its
/// animation (`animations`, by path) and that moves a piece of the model (`pieces`: each piece's
/// bone; mesh units `pixels` pixels): its animation as keys of the pieces, at its rate.
pub fn unpacking(w: &crate::unit::WeaponBlueprint, animations: &[(String, sca::Animation)], mesh: &Mesh, pieces: &[usize], pixels: f64) -> Option<script::Unpacking> {
    let u = w.unpack.as_ref()?;
    let path = u.animation.as_ref()?;
    let (_, animation) = animations.iter().find(|(p, _)| p == path)?;
    let clips = anims::anims(mesh, pieces, pixels, None, &[], &[anims::Work { name: "unpack", animation, rate: u.rate, cycle: false }]);
    let clip = clips.clips.into_iter().find(|c| c.name == "unpack")?;
    Some(script::Unpacking { clip, repack_ms: (u.repack * 1000.0).round() as i32, locks_motion: u.locks_motion, after_launch: w.counted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// A blueprint of the test faction's: `cats` its categories, the rest its sections (its own
    /// `General` and `Display`, where it gives them: a table holds a key once, the first read).
    fn blueprint(cats: &str, rest: &str) -> String {
        let general = if rest.contains("General =") { "" } else { "General = { FactionName = 'UEF' }, " };
        let display = if rest.contains("Display =") { "" } else { "Display = { UniformScale = 0.05 }, " };
        format!("UnitBlueprint {{ Categories = {{ 'PRODUCTSC1', 'UEF', {cats} }}, {general}{display}{rest} }}")
    }

    /// A gun on the test tank's turret, firing `projectile`.
    fn gun(label: &str, extra: &str, projectile: &str) -> String {
        format!(
            "{{ Label = '{label}', WeaponCategory = 'Direct Fire', Damage = 100, MaxRadius = 22, RateOfFire = 1.5, MuzzleVelocity = 30, MuzzleSalvoDelay = 0, MuzzleSalvoSize = 1, \
             RackBones = {{ {{ RackBone = 'Turret_Barrel', MuzzleBones = {{ 'Turret_Barrel', 'Exhaust' }} }} }}, Turreted = true, TurretBoneYaw = 'Turret', TurretBonePitch = 'Turret_Barrel', \
             TurretYawSpeed = 90, TurretPitchSpeed = 45, ProjectileId = '{projectile}', FireTargetLayerCapsTable = {{ Land = 'Land|Water' }}, {extra} }}"
        )
    }

    /// A small faction in an archive of the game's own: a commander (its missile switched off as
    /// it is made), a factory and the one its upgrade makes, an engineer, a tank whose shells split,
    /// a silo storing strategic missiles and an anti-nuke, each drawn with the test tank's mesh,
    /// with the projectiles they fire and the scripts saying what they switch off and split into.
    fn archive() -> Vec<u8> {
        let mesh = crate::scm::tests::tank();
        let units: Vec<(&str, String)> = vec![
            ("UEL0001", blueprint("'COMMAND', 'MOBILE', 'LAND'", &format!(
                "Defense = {{ MaxHealth = 12000, RegenRate = 10 }}, Economy = {{ BuildCostMass = 18000, BuildCostEnergy = 5000000, BuildTime = 6000000, BuildRate = 10, BuildableCategory = {{ 'BUILTBYCOMMANDER UEF' }} }}, \
                 General = {{ FactionName = 'UEF', BuildBones = {{ YawBone = 'Turret', BuildEffectBones = {{ 'Turret_Barrel' }} }} }}, Intel = {{ VisionRadius = 26, SonarRadius = 60 }}, \
                 Physics = {{ MotionType = 'RULEUMT_Amphibious', MaxSpeed = 1.7, TurnRate = 90 }}, \
                 Weapon = {{ {}, {{ Label = 'Missile', WeaponCategory = 'Missile', Damage = 3000, MaxRadius = 60, RateOfFire = 1, CountedProjectile = true, ProjectileId = '/projectiles/Tactical/Tactical_proj.bp' }} }}, \
                 Enhancements = {{ Slots = {{ Back = {{ name = 'Back' }} }}, \
                   AdvancedEngineering = {{ BuildCostMass = 720, BuildCostEnergy = 18000, BuildTime = 900, BuildableCategoryAdds = 'BUILTBYTIER2COMMANDER UEF', Name = '<LOC e_1>Tech 2 Engineering Suite', NewBuildRate = 30, NewHealth = 3000, NewRegenRate = 20, Slot = 'LCH', Icon = 'aes' }}, \
                   AdvancedEngineeringRemove = {{ BuildCostMass = 1, Slot = 'LCH', Prerequisite = 'AdvancedEngineering' }}, \
                   HeavyAntiMatterCannon = {{ BuildCostMass = 800, BuildCostEnergy = 24000, BuildTime = 800, Name = 'Zephyr Amplifier', NewMaxRadius = 30, ZephyrDamageMod = 100, Slot = 'RCH', ShowBones = {{ 'Turret_Barrel' }} }}, \
                   Missile = {{ BuildCostMass = 1000, BuildCostEnergy = 50000, BuildTime = 1000, Name = 'Tactical Missile Launcher', Slot = 'Back' }}, \
                   Shield = {{ BuildCostMass = 1500, BuildCostEnergy = 112500, BuildTime = 1500, Name = 'Personal Shield Generator', ShieldMaxHealth = 24000, Slot = 'Back' }} }}",
                gun("RightZephyr", "", "/projectiles/Shell/Shell_proj.bp")
            ))),
            ("UEB0101", blueprint("'STRUCTURE', 'FACTORY', 'LAND', 'TECH1', 'SIZE16', 'BUILTBYCOMMANDER', 'BUILTBYTIER1ENGINEER'",
                "Economy = { BuildCostMass = 240, BuildCostEnergy = 2100, BuildTime = 300, BuildRate = 20, BuildableCategory = { 'BUILTBYTIER1FACTORY UEF MOBILE', 'BUILTBYTIER1FACTORY UEF STRUCTURE LAND' } }, \
                 General = { FactionName = 'UEF', UpgradesTo = 'ueb0201' }, Footprint = { SizeX = 4, SizeZ = 4 }, Display = { UniformScale = 0.05, BuildAttachBone = 'Turret' }, \
                 Physics = { MotionType = 'RULEUMT_None', RollOffPoints = { { UnitSpin = 90, X = 3, Y = 0, Z = 0 }, { UnitSpin = -90, X = -3, Y = 0, Z = 0 } } }")),
            ("UEB0201", blueprint("'STRUCTURE', 'FACTORY', 'LAND', 'TECH2', 'BUILTBYTIER1FACTORY'",
                "Economy = { BuildCostMass = 1040, BuildCostEnergy = 12000, BuildTime = 2400, BuildRate = 40, BuildableCategory = { 'BUILTBYTIER2FACTORY UEF MOBILE' } }, \
                 General = { FactionName = 'UEF', UpgradesFrom = 'ueb0101' }, Footprint = { SizeX = 4, SizeZ = 4 }")),
            ("UEB0104", blueprint("'STRUCTURE', 'TECH2', 'BUILTBYTIER2COMMANDER'", "Economy = { BuildCostMass = 300, BuildCostEnergy = 3000, BuildTime = 600 }")),
            ("UEL0105", blueprint("'MOBILE', 'LAND', 'TECH1', 'ENGINEER', 'BUILTBYTIER1FACTORY', 'BUILTBYTIER2FACTORY'",
                "Economy = { BuildCostMass = 52, BuildCostEnergy = 260, BuildTime = 260, BuildRate = 5, BuildableCategory = { 'BUILTBYTIER1ENGINEER UEF' } }, \
                 Physics = { MotionType = 'RULEUMT_Land', MaxSpeed = 1.9, TurnRate = 120 }, Display = { UniformScale = 0.05, Mesh = { LODs = { { ShaderName = 'Seraphim' } } } }")),
            ("UEL0201", blueprint("'MOBILE', 'LAND', 'TECH1', 'DIRECTFIRE', 'BUILTBYTIER1FACTORY', 'BUILTBYTIER2FACTORY'", &format!(
                "Defense = {{ MaxHealth = 300 }}, Economy = {{ BuildCostMass = 56, BuildCostEnergy = 266, BuildTime = 280 }}, Physics = {{ MotionType = 'RULEUMT_Land', MaxSpeed = 3.4, TurnRate = 90 }}, \
                 Weapon = {{ {} }}",
                gun("MainGun", "DoTPulses = 3, DoTTime = 1", "/projectiles/Frag/Frag_proj.bp")
            ))),
            ("UEB2305", blueprint("'STRUCTURE', 'TECH1', 'BUILTBYTIER1ENGINEER'",
                "Economy = { BuildCostMass = 15000, BuildCostEnergy = 210000, BuildTime = 9000, BuildRate = 1080 }, \
                 Weapon = { { Label = 'NukeMissiles', WeaponCategory = 'Missile', Damage = 0, MaxRadius = 20000, RateOfFire = 1, CountedProjectile = true, ManualFire = 1, \
                   NukeInnerRingDamage = 70000, NukeInnerRingRadius = 30, NukeOuterRingDamage = 500, NukeOuterRingRadius = 40, ProjectileId = '/projectiles/Nuke/Nuke_proj.bp' } }")),
            ("UEB4302", blueprint("'STRUCTURE', 'TECH1', 'BUILTBYTIER1ENGINEER'",
                "Economy = { BuildCostMass = 1500, BuildCostEnergy = 100000, BuildTime = 9000, BuildRate = 1080 }, \
                 Weapon = { { Label = 'AntiNuke', WeaponCategory = 'Defense', Damage = 30, MaxRadius = 90, RateOfFire = 0.6, CountedProjectile = true, \
                   TargetRestrictOnlyAllow = 'STRATEGIC MISSILE', ProjectileId = '/projectiles/AntiNuke/AntiNuke_proj.bp' } }")),
            // An extractor pumping as it works and turning into the next as it upgrades, that next,
            // a generator whose turret spins once it is built, and a radar whose turret spins
            // while it watches.
            ("UEB1103", blueprint("'STRUCTURE', 'MASSEXTRACTION', 'TECH1', 'SIZE4', 'BUILTBYTIER1ENGINEER'",
                "Economy = { BuildCostMass = 50, BuildCostEnergy = 360, BuildTime = 60, BuildRate = 10, MaintenanceConsumptionPerSecondEnergy = 2, ProductionPerSecondMass = 2 }, \
                 General = { FactionName = 'UEF', UpgradesTo = 'ueb1202' }, Display = { UniformScale = 0.05, AnimationOpen = '/units/UEB1103/UEB1103_pump.sca', AnimationUpgrade = '/units/UEB1103/UEB1103_up.sca' }")),
            ("UEB1202", blueprint("'STRUCTURE', 'MASSEXTRACTION', 'TECH2', 'BUILTBYTIER2ENGINEER'",
                "Economy = { BuildCostMass = 900, BuildCostEnergy = 5400, BuildTime = 900, MaintenanceConsumptionPerSecondEnergy = 9, ProductionPerSecondMass = 6 }, \
                 General = { FactionName = 'UEF', UpgradesFrom = 'ueb1103' }")),
            ("UEB1101", blueprint("'STRUCTURE', 'ENERGYPRODUCTION', 'TECH1', 'SIZE4', 'BUILTBYTIER1ENGINEER'",
                "Adjacency = 'GeneratorBuffs', Economy = { BuildCostMass = 75, BuildCostEnergy = 750, BuildTime = 125, ProductionPerSecondEnergy = 20 }")),
            ("UEB3101", blueprint("'STRUCTURE', 'RADAR', 'INTELLIGENCE', 'TECH1', 'SIZE4', 'BUILTBYTIER1ENGINEER'",
                "Economy = { BuildCostMass = 60, BuildCostEnergy = 600, BuildTime = 120, MaintenanceConsumptionPerSecondEnergy = 20 }, Intel = { RadarRadius = 115, VisionRadius = 20 }")),
            ("UEL0111", blueprint("'MOBILE', 'LAND', 'TECH2', 'INDIRECTFIRE', 'BUILTBYTIER1FACTORY'",
                "Economy = { BuildCostMass = 200, BuildCostEnergy = 1500, BuildTime = 1000 }, Physics = { MotionType = 'RULEUMT_Land', MaxSpeed = 2.8, TurnRate = 60 }, \
                 Weapon = { { Label = 'MissileWeapon', WeaponCategory = 'Missile', Damage = 300, MaxRadius = 60, RateOfFire = 0.1, MuzzleVelocity = 3, Turreted = false, \
                   RackBones = { { RackBone = 'Turret_Barrel', MuzzleBones = { 'Turret_Barrel' } } }, ProjectileId = '/projectiles/Tactical/Tactical_proj.bp', \
                   FireTargetLayerCapsTable = { Land = 'Land|Water' }, WeaponUnpacks = true, WeaponUnpackAnimation = '/units/UEL0111/UEL0111_aopen.sca', \
                   WeaponUnpackAnimationRate = 1, WeaponRepackTimeout = 1, WeaponUnpackLocksMotion = true } }")),
            ("UEB2104", blueprint("'STRUCTURE', 'ANTIAIR', 'TECH1', 'BUILTBYTIER1ENGINEER'",
                "Economy = { BuildCostMass = 70, BuildCostEnergy = 400, BuildTime = 100 }, \
                 Weapon = { { Label = 'AAGun', WeaponCategory = 'Anti Air', Damage = 20, MaxRadius = 44, RateOfFire = 3, MuzzleVelocity = 45, Turreted = true, \
                   TurretBoneYaw = 'Turret', TurretBonePitch = 'Turret_Barrel', TurretYawSpeed = 180, TurretPitchSpeed = 90, \
                   RackBones = { { RackBone = 'Turret_Barrel', MuzzleBones = { 'Turret_Barrel' } } }, ProjectileId = '/projectiles/Shell/Shell_proj.bp', \
                   FireTargetLayerCapsTable = { Land = 'Air', Water = 'Air' } } }")),
            // An engineering station the engineer builds, working through its drone, and the next
            // tier of it, which nothing builds: made by upgrading alone, with two drones. (Made-up
            // figures.)
            ("XEB0104", blueprint("'STRUCTURE', 'TECH2', 'ENGINEERSTATION', 'BUILTBYTIER1ENGINEER'",
                "Economy = { BuildCostMass = 400, BuildCostEnergy = 2000, BuildTime = 1000, BuildRate = 18, BuildableCategory = { 'xeb0204' }, EngineeringPods = { { PodUnitID = 'XEA3204', PodName = 'PodOne' } } }, \
                 General = { FactionName = 'UEF', UpgradesTo = 'xeb0204', CommandCaps = { RULEUCC_Guard = false, RULEUCC_Stop = true } }")),
            ("XEB0204", blueprint("'STRUCTURE', 'TECH3', 'ENGINEERSTATION'",
                "Economy = { BuildCostMass = 600, BuildCostEnergy = 3000, BuildTime = 1500, BuildRate = 18, EngineeringPods = { { PodUnitID = 'XEA3204', PodName = 'PodOne' }, { PodUnitID = 'XEA3204', PodName = 'PodTwo' } } }, \
                 General = { FactionName = 'UEF', UpgradesFrom = 'xeb0104', CommandCaps = { RULEUCC_Guard = true } }")),
            ("XEA3204", blueprint("'UEF', 'MOBILE', 'AIR', 'POD'", "AI = { GuardScanRadius = 12 }, Economy = { BuildRate = 12 }, Physics = { MotionType = 'RULEUMT_Air' }")),
        ];
        let projectile = |cats: &str, rest: &str| format!("ProjectileBlueprint {{ Categories = {{ 'PROJECTILE', {cats} }}, {rest} }}");
        // (What the structures give those beside them.)
        let mut files: Vec<(String, Vec<u8>)> = vec![("lua/sim/AdjacencyBuffs.lua".into(), crate::adjacency::tests::BUFFS.as_bytes().to_vec())];
        for (id, bp) in &units {
            files.push((format!("units/{id}/{id}_unit.bp"), bp.clone().into_bytes()));
            files.push((format!("units/{id}/{id}_lod0.scm"), mesh.clone()));
        }
        files.push(("units/UEL0001/UEL0001_script.lua".into(), b"Unit = Class(Base) { Weapons = { Missile = Class(Launcher) { OnCreate = function(self) self:SetWeaponEnabled(false) end, }, }, \
            OnStopBeingBuilt = function(self, builder, layer) self:DisableUnitIntel('Sonar') end, \
            CreateEnhancement = function(self, enh) if enh == 'Missile' then self:SetWeaponEnabledByLabel('Missile', true) elseif enh == 'MissileRemove' then self:SetWeaponEnabledByLabel('Missile', false) end end, }".to_vec()));
        files.push(("units/UEB1103/UEB1103_script.lua".into(), b"Extractor = Class(Base) { PlayActiveAnimation = function(self) Base.PlayActiveAnimation(self) \
            self.Pump = CreateAnimator(self) self.Pump:PlayAnim(self:GetBlueprint().Display.AnimationOpen, true) end, \
            OnProductionPaused = function(self) self.Pump:SetRate(0) end, }".to_vec()));
        files.push(("units/UEB1101/UEB1101_script.lua".into(), b"Generator = Class(Base) { OnStopBeingBuilt = function(self, builder, layer) \
            self.Trash:Add(CreateRotator(self, 'Turret', 'y', nil, 0, 30, 90)) end, }".to_vec()));
        files.push(("units/UEB3101/UEB3101_script.lua".into(), b"Radar = Class(Base) { OnIntelEnabled = function(self) self.Dish = CreateRotator(self, 'Turret', 'y') \
            self.Dish:SetTargetSpeed(-45) self.Dish:SetAccel(15) end, OnIntelDisabled = function(self) self.Dish:SetTargetSpeed(0) end, }".to_vec()));
        // The pump lifts the turret and lowers it again; the upgrade turns the barrel up.
        let still = [1.0f32, 0.0, 0.0, 0.0];
        let key = |y: f32| ([0.0, y, -3.0], still);
        files.push(("units/UEB1103/UEB1103_pump.sca".into(), crate::sca::tests::write(&[("Turret", -1)], &[(0.0, vec![key(4.0)]), (0.5, vec![key(5.0)]), (1.0, vec![key(4.0)])], 1.0)));
        let s = std::f32::consts::FRAC_1_SQRT_2;
        files.push(("units/UEB1103/UEB1103_up.sca".into(), crate::sca::tests::write(&[("Turret_Barrel", -1)], &[(0.0, vec![([0.0, -0.5, 3.0], still)]), (2.0, vec![([0.0, -0.5, 3.0], [s, -s, 0.0, 0.0])])], 2.0)));
        let (c, n) = ((std::f32::consts::PI / 8.0).cos(), (std::f32::consts::PI / 8.0).sin());
        files.push(("units/UEL0111/UEL0111_aopen.sca".into(), crate::sca::tests::write(&[("Turret", -1)], &[(0.0, vec![([0.0, 4.0, -3.0], still)]), (0.5, vec![([0.0, 4.5, -3.0], [c, -n, 0.0, 0.0])]), (1.0, vec![([0.0, 5.0, -3.0], [s, -s, 0.0, 0.0])])], 1.0)));
        // Paint: the factory's colour map of 1024 texels (DXT1, every level, a quarter of its blocks
        // of three colours, as Forged Alliance's are) and its specular map of 512 (DXT5); the
        // tank's and the engineer's small and uncompressed (enlarged to 512), the engineer drawn as
        // the Seraphim are (its player's colour in its colour map's alpha).
        let dxt = |side: u32, fourcc: &[u8; 4], seed: u32| -> Vec<u8> {
            let levels = side.ilog2() + 1;
            let mut file = ta_formats::dds::header(side, side, levels, Some(fourcc));
            let grey = |v: u32| (((v >> 3) << 11) | ((v >> 2) << 5) | (v >> 3)) as u16;
            for l in 0..levels {
                for i in 0..((side >> l).max(4) / 4).pow(2) {
                    let k = i.wrapping_mul(2_654_435_761).wrapping_add(seed + l);
                    if fourcc == b"DXT5" {
                        file.extend_from_slice(&[(k >> 24) as u8, (k >> 16) as u8, (k >> 5) as u8, (k >> 13) as u8, (k >> 3) as u8, (k >> 9) as u8, (k >> 1) as u8, (k >> 19) as u8]);
                    }
                    let (c0, c1, bits) = if fourcc == b"DXT1" && i % 4 == 0 {
                        // (Three colours: the ends and halfway, never the see-through black.)
                        let bits = (0..16).fold(0u32, |b, t| b | (((k >> t) % 3) << (2 * t)));
                        (grey(40 + i % 50), grey(110 + i % 60), bits)
                    } else {
                        (((k >> 16) as u16) | 0x8000, (k as u16) & 0x7FFF, k.rotate_left(7))
                    };
                    file.extend_from_slice(&c0.to_le_bytes());
                    file.extend_from_slice(&c1.to_le_bytes());
                    file.extend_from_slice(&bits.to_le_bytes());
                }
            }
            file
        };
        let small = |seed: u32| -> Vec<u8> {
            let mut file = ta_formats::dds::header(64, 64, 1, None);
            for i in 0..64 * 64u32 {
                let (x, y) = (i % 64, i / 64);
                file.extend_from_slice(&[(x * 4 + seed) as u8, (y * 3) as u8, ((x ^ y) * 5) as u8, if (x / 8 + y / 8) % 3 == 0 { 255 } else { 0 }]);
            }
            file
        };
        files.push(("units/UEB0101/UEB0101_albedo.dds".into(), dxt(1024, b"DXT1", 1)));
        files.push(("units/UEB0101/UEB0101_specteam.dds".into(), dxt(512, b"DXT5", 2)));
        for (id, seed) in [("UEL0201", 3), ("UEL0105", 4)] {
            files.push((format!("units/{id}/{id}_albedo.dds"), small(seed)));
            files.push((format!("units/{id}/{id}_specteam.dds"), small(seed + 40)));
        }
        // The engineering suite's button.
        let mut icon = ta_formats::dds::header(8, 8, 1, None);
        icon.extend(std::iter::repeat_n([40u8, 80, 160, 255], 64).flatten());
        files.push(("textures/ui/common/game/uef-enhancements/aes_btn_up.dds".into(), icon));
        files.push(("projectiles/Shell/Shell_proj.bp".into(), projectile("'DIRECTFIRE'", "Physics = { InitialSpeed = 30 }").into_bytes()));
        files.push(("projectiles/Frag/Frag_proj.bp".into(), projectile("'INDIRECTFIRE'", "Physics = { InitialSpeed = 30 }").into_bytes()));
        files.push(("projectiles/Frag/Frag_script.lua".into(), b"Frag = Class(Base) { OnImpact = function(self, t, e) local ChildProjectileBP = '/projectiles/Shell/Shell_proj.bp' \
            self:CreateChildProjectile(ChildProjectileBP) local numProjectiles = 4 for i = 0, (numProjectiles -1) do self:CreateChildProjectile(ChildProjectileBP) end end, }".to_vec()));
        files.push(("projectiles/Tactical/Tactical_proj.bp".into(), projectile("'TACTICAL', 'MISSILE'", "Economy = { BuildCostMass = 180, BuildCostEnergy = 3600, BuildTime = 300 }, Physics = { MaxSpeed = 12, TrackTarget = true }").into_bytes()));
        files.push(("projectiles/Nuke/Nuke_proj.bp".into(), projectile("'STRATEGIC', 'MISSILE'", "Economy = { BuildCostMass = 12000, BuildCostEnergy = 1350000, BuildTime = 324000 }, Physics = { MaxSpeed = 30, Acceleration = 3 }").into_bytes()));
        files.push(("projectiles/AntiNuke/AntiNuke_proj.bp".into(), projectile("'ANTIMISSILE'", "Economy = { BuildCostMass = 3600, BuildCostEnergy = 360000, BuildTime = 259200 }, Physics = { MaxSpeed = 90, Acceleration = 30, TrackTarget = true, TurnRate = 180 }").into_bytes()));
        let entries: Vec<(&str, &[u8], bool)> = files.iter().map(|(p, b)| (p.as_str(), b.as_slice(), true)).collect();
        crate::scd::tests::zip(&entries)
    }

    /// The test faction made into the mod: every unit made, and the mod's every file (its path
    /// and bytes) the same on every computer: the same bytes twice here, and the hash the
    /// converter gives on every platform (raised whenever what it makes changes: the bytes must
    /// never depend on a platform's powers, roots or trigonometry, or on an order not fixed).
    #[test]
    fn the_mod_is_the_same_bytes_on_every_computer() {
        let dir = crate::tests::Temp::new("make");
        std::fs::write(dir.0.join("units.scd"), archive()).unwrap();
        let files = Files::of(Vec::new(), vec![crate::scd::Scd::open(&dir.0.join("units.scd")).unwrap()]);
        let units: Vec<UnitBlueprint> = crate::unit::catalogue(&files).unwrap().into_iter().filter(|u| u.playable()).collect();
        assert_eq!(units.len(), 16, "the station's next tier, made by upgrading alone, among them; not its drone");
        let run = || {
            let mut hash = Sha256::new();
            let mut paths: Vec<String> = Vec::new();
            let made = make(&files, &units, "test", &mut |path, bytes| {
                hash.update(path.as_bytes());
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
                paths.push(path.to_string());
                Ok(())
            }, &mut |_, _| true).unwrap();
            let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
            (made, paths, hex)
        };
        let (made, paths, first) = run();
        assert_eq!((made.units, made.skipped.len()), (16, 0), "{:?}", made.skipped);
        // The tech 1 factory upgrades into the tech 2 one, which no builder builds.
        let mut texts: std::collections::BTreeMap<String, String> = Default::default();
        make(&files, &units, "test", &mut |path, bytes| {
            if path.ends_with(".fbi") || path.ends_with(".tdf") {
                texts.insert(path.to_string(), String::from_utf8_lossy(bytes).into_owned());
            }
            Ok(())
        }, &mut |_, _| true).unwrap();
        assert!(texts["units/ueb0101.fbi"].contains("	UpgradesTo=UEB0201;"), "{}", texts["units/ueb0101.fbi"]);
        assert!(!texts["units/ueb0201.fbi"].contains("UpgradesTo"));
        assert!(!texts["download/supcom_uef.tdf"].contains("UNITNAME=UEB0201;"), "on no menu");
        // The engineering station upgrades into its next tier, which nothing builds; each works
        // round it by itself, by its drones' reach (12 ogrids) and its own work or theirs, the more.
        let station = |id: &str| ta_formats::tdf::parse(&texts[&format!("units/{id}.fbi")]).sections.remove(0);
        let (t2, t3) = (station("xeb0104"), station("xeb0204"));
        assert_eq!((t2.get("UpgradesTo"), t2.get("DefaultMissionType"), t2.get("WorkerTime"), t2.get("BuildDistance"), t2.get("canguard")), (Some("XEB0204"), Some("AssistPatrol"), Some("180"), Some("384"), None));
        assert_eq!((t3.get("UpgradesTo"), t3.get("DefaultMissionType"), t3.get("WorkerTime"), t3.get("canguard")), (None, Some("AssistPatrol"), Some("240"), Some("1")), "two drones' work; told to help, as FA lets it");
        assert!(!texts["download/supcom_uef.tdf"].contains("UNITNAME=XEB0204;") && texts["download/supcom_uef.tdf"].contains("UNITNAME=XEB0104;"), "the next tier on no menu");
        assert!(!texts.contains_key("units/xea3204.fbi"), "the drone is not made");
        // The generator gives the structures beside it its buffs; the factory, the extractor and
        // the radar say their size classes.
        let generator = ta_formats::tdf::parse(&texts["units/ueb1101.fbi"]).sections.remove(0);
        let buffs = generator.sections.iter().find(|s| s.name == "ADJACENCY").expect("its adjacency");
        let buff = &buffs.sections[0];
        assert_eq!((buff.name.as_str(), buff.get("Affects"), buff.get("Add"), buff.get("Size")), ("GenBuildSize4", Some("BuildEnergy"), Some("-0.05"), Some("4")));
        assert_eq!(buffs.sections.len(), 4);
        assert_eq!(buffs.sections[3].get("Category"), Some("ARTILLERY"));
        for (id, size) in [("ueb0101", "16"), ("ueb1103", "4"), ("ueb3101", "4"), ("ueb1101", "4")] {
            assert_eq!(ta_formats::tdf::parse(&texts[&format!("units/{id}.fbi")]).sections[0].get("AdjacencySize"), Some(size), "{id}");
        }
        assert!(station("xeb0104").get("AdjacencySize").is_none(), "a station has no size class: it takes nothing");
        // The commander's enhancements, each as the engine plays it; the shield left out.
        let acu = ta_formats::tdf::parse(&texts["units/uel0001.fbi"]);
        let list = acu.section("UNITINFO").and_then(|u| u.sections.iter().find(|s| s.name == "ENHANCEMENTS")).expect("its enhancements");
        let names: Vec<&str> = list.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["AdvancedEngineering", "HeavyAntiMatterCannon", "Missile"], "the shield and the removal left out");
        let suite = &list.sections[0];
        assert_eq!((suite.get("Name"), suite.get("Slot"), suite.get("BuildTime"), suite.get("WorkerTime")), (Some("Tech 2 Engineering Suite"), Some("LCH"), Some("9000"), Some("200")));
        assert_eq!((suite.get("MaxDamage"), suite.get("HealRate"), suite.get("BuildOptions"), suite.get("Picture")), (Some("3000"), Some("20"), Some("UEB0104"), Some("uel0001_advancedengineering")));
        assert!(paths.iter().any(|p| p == "unitpics/uel0001_advancedengineering.pcx"), "its picture");
        let gun = &list.sections[1];
        assert_eq!(gun.get("Weapon1"), Some("UEL0001_RIGHTZEPHYR_HEAVYANTIMATTERCANNON"));
        assert_eq!(list.sections[2].get("Weapon2"), Some("UEL0001_MISSILE"), "the missile in the slot after the gun's");
        let fbi = acu.section("UNITINFO").unwrap();
        assert_eq!((fbi.get("Weapon1"), fbi.get("Weapon2")), (Some("UEL0001_RIGHTZEPHYR"), None), "a fresh commander without the missile");
        let weapons = ta_formats::tdf::parse(&texts["weapons/supcom_uef.tdf"]);
        let amplified = weapons.section("UEL0001_RIGHTZEPHYR_HEAVYANTIMATTERCANNON").expect("the gun as amplified");
        assert_eq!(amplified.get("range"), Some("960"), "30 ogrids");
        assert!(weapons.section("UEL0001_MISSILE").is_some());
        // Its script shows the gun's barrel once amplified.
        // The factory's ways out to either side; the truck unpacks before it fires, holding still;
        // the tower shoots aircraft alone.
        let factory = ta_formats::tdf::parse(&texts["units/ueb0101.fbi"]);
        let ways = factory.section("UNITINFO").and_then(|u| u.sections.iter().find(|s| s.name == "EXITS")).expect("its ways out");
        assert_eq!(ways.sections.iter().map(|s| s.get("Heading").unwrap_or("")).collect::<Vec<_>>(), ["90", "-90"]);
        assert!(texts["units/ueb2104.fbi"].contains("OnlyTargetCategory1=VTOL;") && !texts["units/uel0111.fbi"].contains("OnlyTargetCategory"));
        let menus = &texts["download/supcom_uef.tdf"];
        assert!(!menus.contains("UNITNAME=UEB0104;"), "the station not on its menu: its suite's");
        assert!(paths.iter().any(|p| p == "units/ueb0201.fbi") && paths.iter().any(|p| p == "weapons/supcom_uef.tdf"), "{paths:?}");
        assert_eq!(run().2, first, "made twice, the same bytes");
        assert_eq!(first, EXPECTED, "the converter's bytes changed: if it was meant to, put the new hash here");
    }

    /// The test faction's buildings at work: the extractor's clips (its pump, its upgrade), its
    /// script switched on and off; the generator spinning once its build is done, the radar while
    /// it is on.
    #[test]
    fn buildings_play_and_spin_as_they_work() {
        let dir = crate::tests::Temp::new("make-work");
        std::fs::write(dir.0.join("units.scd"), archive()).unwrap();
        let files = Files::of(Vec::new(), vec![crate::scd::Scd::open(&dir.0.join("units.scd")).unwrap()]);
        let units: Vec<UnitBlueprint> = crate::unit::catalogue(&files).unwrap().into_iter().filter(|u| u.playable()).collect();
        let mut written: std::collections::BTreeMap<String, Vec<u8>> = Default::default();
        make(&files, &units, "test", &mut |path, bytes| {
            written.insert(path.to_string(), bytes.to_vec());
            Ok(())
        }, &mut |_, _| true).unwrap();
        let clips = ta_formats::anims::Anims::parse(&written["objects3d/ueb1103.anims"]).unwrap();
        let names: Vec<&str> = clips.clips.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["active", "upgrade"]);
        let pump = clips.clip("active").unwrap();
        assert_eq!((pump.seconds, pump.pieces.as_slice()), (1.0, &[1u32][..]), "the turret (piece 1) up and down");
        assert_eq!(pump.frame(1)[0].moved[1], (1.0 * 0.05 * 32.0 * 65536.0f64).round() as i32);
        assert!(!written.contains_key("objects3d/ueb1202.anims"), "the next has nothing to play");
        let script = |id: &str| ta_formats::cob::CobScript::parse(&written[&format!("scripts/{id}.cob")]).unwrap();
        let extractor = script("ueb1103");
        assert!(extractor.function("Activate").is_none(), "nothing spins: its pump is its clip");
        let (generator, radar) = (script("ueb1101"), script("ueb3101"));
        for f in ["StartWorking", "StopWorking", "WorkWhenBuilt"] {
            assert!(generator.function(f).is_some(), "{f}");
        }
        assert!(generator.function("Activate").is_none(), "no switch: it starts once built");
        let listing = ta_formats::cob::disassemble(&generator);
        assert!(listing.contains("spin             1 1"), "the turret about y: {listing}");
        for f in ["Activate", "Deactivate", "StartWorking", "StopWorking"] {
            assert!(radar.function(f).is_some(), "{f}");
        }
        assert!(radar.function("WorkWhenBuilt").is_none());
        let listing = ta_formats::cob::disassemble(&radar);
        assert!(listing.contains("push-const       -8192") && listing.contains("stop-spin        1 1"), "45 degrees a second the other way, stopped: {listing}");
        let fbi = |id: &str| String::from_utf8(written[&format!("units/{id}.fbi")].clone()).unwrap();
        assert!(fbi("ueb3101").contains("ActivateWhenBuilt=1;") && !fbi("ueb1101").contains("ActivateWhenBuilt"));
        // The missile truck raises its launcher before it fires (an aim of its own, though it has no
        // turret), lowers it again its target gone, and holds still meanwhile.
        let truck = script("uel0111");
        for f in ["AimPrimary", "UnpackWeapon1", "PackWeapon1", "TargetCleared", "RestoreAfterDelay", "StartMoving", "StopMoving"] {
            assert!(truck.function(f).is_some(), "{f}");
        }
        let listing = ta_formats::cob::disassemble(&truck);
        assert!(listing.contains("turn             1 0") && listing.contains("move             1 1"), "its turret turned up and lifted: {listing}");
        assert!(script("uel0201").function("UnpackWeapon1").is_none(), "a tank fires at once");
    }

    /// The test faction's paint: the factory's colour map of 1024 kept at 1024 (BC1, its own
    /// colours), its specular map of 512 at 512; the tank's small maps enlarged to 512; the
    /// engineer, drawn as the Seraphim, with its player's colour moved from its colour map's alpha
    /// into its specular map's and glowing there; the stand-in for a texture a unit lacks at 512.
    #[test]
    fn the_paint_keeps_its_own_size() {
        use ta_formats::dds;
        let dir = crate::tests::Temp::new("make-paint");
        std::fs::write(dir.0.join("units.scd"), archive()).unwrap();
        let files = Files::of(Vec::new(), vec![crate::scd::Scd::open(&dir.0.join("units.scd")).unwrap()]);
        let units: Vec<UnitBlueprint> = crate::unit::catalogue(&files).unwrap().into_iter().filter(|u| u.playable()).collect();
        let mut written: std::collections::BTreeMap<String, Vec<u8>> = Default::default();
        make(&files, &units, "test", &mut |path, bytes| {
            if path.starts_with("textures/") {
                written.insert(path.to_string(), bytes.to_vec());
            }
            Ok(())
        }, &mut |_, _| true).unwrap();
        let header = |path: &str| {
            let bytes = &written[path];
            let h = dds::Header::parse(bytes).unwrap();
            assert_eq!(h.levels_held(bytes), h.levels, "{path}: every level");
            (h.width, h.format)
        };
        assert_eq!(header("textures/supcom/ueb0101_albedo.dds"), (1024, dds::Format::Dxt1));
        assert_eq!(header("textures/supcom/ueb0101_specteam.dds"), (512, dds::Format::Dxt5));
        assert_eq!(header("textures/supcom/ueb0101_normals.dds"), (512, dds::Format::Dxt5), "a stand-in");
        assert_eq!(header("textures/supcom/uel0201_albedo.dds"), (512, dds::Format::Dxt1));
        // (The factory's colours, its source's, its blocks of three colours to a step or two: no
        // gain for a unit naming no shader.)
        let source = files.read("units/ueb0101/ueb0101_albedo.dds").unwrap().unwrap();
        let (was, now) = (dds::decode(&source, 0).unwrap(), dds::decode(&written["textures/supcom/ueb0101_albedo.dds"], 0).unwrap());
        let mut off = 0u64;
        for (a, b) in was.rgba.iter().zip(&now.rgba) {
            assert!((0..3).all(|k| a[k].abs_diff(b[k]) <= 12), "{a:?} became {b:?}");
            off += (0..3).map(|k| a[k].abs_diff(b[k]) as u64).sum::<u64>();
        }
        assert!(off * 2 < was.rgba.len() as u64 * 3, "{off} steps off over {} texels", was.rgba.len());
        // (The engineer's player's colour where its colour map's alpha was, glowing.)
        let spec = dds::decode(&written["textures/supcom/uel0105_specteam.dds"], 0).unwrap();
        let marked = spec.rgba.iter().filter(|t| t[3] > 200).count();
        assert!(marked > 512 * 512 / 5, "{marked}");
        // (Glowing where marked, to the colour blocks' encoding where marked texels meet others.)
        let glowing = spec.rgba.iter().filter(|t| t[3] > 200 && t[2] as u32 + 12 >= textures::SERAPHIM_GLOW).count();
        assert!(glowing * 100 >= marked * 95, "{glowing} of {marked} marked texels glowing");
    }

    /// The hash of the test faction's mod (see the test above).
    const EXPECTED: &str = "767037adbecaa137b1d628c47f1cf69d01d1930189019d781e078a5fa69ea16e";

    /// A unit whose mesh does not read is left out, with its reason; the rest are made.
    #[test]
    fn a_unit_that_does_not_read_is_left_out_with_its_reason() {
        let dir = crate::tests::Temp::new("make-broken");
        let good = blueprint("'MOBILE', 'LAND', 'TECH1', 'BUILTBYTIER1FACTORY'", "Physics = { MotionType = 'RULEUMT_Land', MaxSpeed = 3, TurnRate = 90 }");
        let mesh = crate::scm::tests::tank();
        let zip = crate::scd::tests::zip(&[
            ("units/UEL0201/UEL0201_unit.bp", good.as_bytes(), false),
            ("units/UEL0201/UEL0201_lod0.scm", &mesh, false),
            ("units/UEL0202/UEL0202_unit.bp", good.as_bytes(), false),
            ("units/UEL0202/UEL0202_lod0.scm", b"MODL not a mesh at all, cut short", false),
        ]);
        std::fs::write(dir.0.join("units.scd"), zip).unwrap();
        let files = Files::of(Vec::new(), vec![crate::scd::Scd::open(&dir.0.join("units.scd")).unwrap()]);
        let units: Vec<UnitBlueprint> = crate::unit::catalogue(&files).unwrap();
        let mut written: Vec<String> = Vec::new();
        let made = make(&files, &units, "test", &mut |path, _| {
            written.push(path.to_string());
            Ok(())
        }, &mut |_, _| true).unwrap();
        assert_eq!(made.units, 1);
        assert_eq!(made.skipped.len(), 1);
        assert!(made.skipped[0].0 == "UEL0202" && made.skipped[0].1.contains("uel0202_lod0.scm"), "{:?}", made.skipped);
        assert!(!written.iter().any(|p| p.contains("uel0202")), "nothing of it written: {written:?}");
    }
}
