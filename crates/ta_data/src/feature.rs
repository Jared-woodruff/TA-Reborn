//! Map feature definitions (`features/**/*.tdf`): trees, rocks, metal deposits, wreckage.

use ta_formats::tdf::TdfSection;

#[derive(Clone, Debug, Default)]
pub struct FeatureDef {
    /// Section name as written (case preserved); lookups are case-insensitive.
    pub name: String,
    pub world: String,
    pub description: String,
    pub category: String,
    /// 3DO object (`objects3d/<object>.3do`), used by wreckage and some rocks.
    pub object: String,
    /// GAF sprite animation: `anims/<filename>.gaf`, entry `seq_name`.
    pub filename: String,
    pub seq_name: String,
    pub seq_name_shadow: String,
    pub seq_name_burn: String,
    pub seq_name_burn_shadow: String,
    pub seq_name_die: String,
    pub seq_name_reclaim: String,
    pub footprint_x: u32,
    pub footprint_z: u32,
    pub height: f64,
    pub blocking: bool,
    pub hit_density: f64,
    pub damage: f64,
    pub indestructible: bool,
    pub reclaimable: bool,
    pub auto_reclaimable: bool,
    pub metal: f64,
    pub energy: f64,
    pub flamable: bool,
    pub burn_min: f64,
    pub burn_max: f64,
    pub spark_time: f64,
    pub spread_chance: f64,
    pub burn_weapon: String,
    pub geothermal: bool,
    /// Feature this becomes when destroyed / burnt / reclaimed.
    pub feature_dead: String,
    pub feature_burnt: String,
    pub feature_reclamate: String,
    pub animating: bool,
    pub anim_trans: bool,
    pub shad_trans: bool,
    pub no_display_info: bool,
    pub permanent: bool,
    /// TA Reborn's own features (`assets/reborn/features`): the model of TA Reborn's library it
    /// is drawn as up close (`rebornmodel`; its painting was drawn from that model).
    pub reborn_model: String,
    /// The light it casts by night (`rebornlight`: its colour, linear, how far it reaches and how
    /// high over its foot it shines from).
    pub reborn_light: Option<([f32; 3], f32, f32)>,
    /// It sets off fireworks by night (`rebornfireworks`).
    pub reborn_fireworks: bool,
}

impl FeatureDef {
    pub fn from_tdf(s: &TdfSection, world_hint: &str) -> FeatureDef {
        let f = |k: &str, d: f64| s.get_f64_or(k, d);
        let b = |k: &str| s.get_bool_or(k, false);
        let st = |k: &str| s.get(k).unwrap_or("").trim().to_string();
        let world = s.get("world").map(|w| w.trim().to_string()).unwrap_or_else(|| world_hint.to_string());
        FeatureDef {
            name: s.name.trim().to_string(),
            world,
            description: st("description"),
            category: st("category"),
            object: st("object"),
            filename: st("filename"),
            seq_name: st("seqname"),
            seq_name_shadow: st("seqnameshad"),
            seq_name_burn: st("seqnameburn"),
            seq_name_burn_shadow: st("seqnameburnshad"),
            seq_name_die: st("seqnamedie"),
            seq_name_reclaim: st("seqnamereclamate"),
            footprint_x: crate::footprint_side(s, "footprintx"),
            footprint_z: crate::footprint_side(s, "footprintz"),
            height: f("height", 0.0),
            blocking: s.get_bool_or("blocking", true),
            hit_density: f("hitdensity", 0.0),
            damage: f("damage", 1.0),
            indestructible: b("indestructible"),
            reclaimable: b("reclaimable"),
            auto_reclaimable: b("autoreclaimable"),
            metal: f("metal", 0.0),
            energy: f("energy", 0.0),
            flamable: b("flamable"),
            burn_min: f("burnmin", 0.0),
            burn_max: f("burnmax", 0.0),
            spark_time: f("sparktime", 0.0),
            spread_chance: f("spreadchance", 0.0),
            burn_weapon: st("burnweapon"),
            geothermal: b("geothermal"),
            feature_dead: st("featuredead"),
            feature_burnt: st("featureburnt"),
            feature_reclamate: st("featurereclamate"),
            animating: b("animating"),
            anim_trans: b("animtrans"),
            shad_trans: b("shadtrans"),
            no_display_info: b("nodisplayinfo"),
            permanent: b("permanent"),
            reborn_model: st("rebornmodel"),
            reborn_light: s.get("rebornlight").and_then(|v| {
                let n: Vec<f32> = v.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                (n.len() == 5).then(|| ([n[0], n[1], n[2]], n[3], n[4]))
            }),
            reborn_fireworks: b("rebornfireworks"),
        }
    }

    /// Metal deposits: flat, indestructible features carrying metal. Maps label them
    /// inconsistently (`category=metal` on most worlds, `rocks` for green-world
    /// `RockMetal*`), so the properties decide. Extractors are built on top of them.
    pub fn is_metal_deposit(&self) -> bool {
        self.category.eq_ignore_ascii_case("metal") || self.metal > 0.0 && self.indestructible && self.height <= 8.0
    }

    pub fn is_sprite(&self) -> bool {
        self.object.is_empty() && !self.filename.is_empty()
    }
}
