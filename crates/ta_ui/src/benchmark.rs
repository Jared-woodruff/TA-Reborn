//! The benchmark's scenes as the game's captions and the launcher's report name them: the world
//! (a name: not translated), the moment, and three things each shows off.

use crate::i18n::n;

pub struct SceneInfo {
    pub id: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub features: [&'static str; 3],
}

/// The scenes, in the order a full run plays them (`ta_config::benchmark::FULL_PARTS`, the tests
/// aside).
pub const SCENES: [SceneInfo; 8] = [
    SceneInfo { id: "sapphire", title: "SAPPHIRE REACH", subtitle: n("Dawn on the sea"), features: [n("Water reflections"), n("Battle fleets"), n("Air war")] },
    SceneInfo { id: "cockpit", title: "GREENMANTLE", subtitle: n("Riding in a Stumpy"), features: [n("Cockpit view"), n("3D forest"), n("River crossing")] },
    SceneInfo { id: "highland", title: "HIGHLAND RIFT", subtitle: n("A thunderstorm"), features: [n("Rain and lightning"), n("Burning woods"), n("Tower assault")] },
    SceneInfo { id: "space", title: "LAGRANGE STATION", subtitle: n("In the void"), features: [n("Space world"), n("Gantries"), n("Dogfights")] },
    SceneInfo { id: "build", title: "REDSTONE MESAS", subtitle: n("A base rising"), features: [n("Nano-lathes"), n("Desert sunset"), n("Construction")] },
    SceneInfo { id: "swarm", title: "REACTOR GRID", subtitle: n("Twenty thousand"), features: [n("20,000 units"), n("Metal world"), n("Strategic zoom")] },
    SceneInfo { id: "cinder", title: "CINDER BASIN", subtitle: n("Nightfall"), features: [n("Lava world"), n("Mass battle"), n("Nuclear strike")] },
    SceneInfo { id: "winter", title: "MIDNIGHT SQUARE", subtitle: n("Midnight"), features: [n("Fireworks"), n("Snowfall"), n("Night lights")] },
];

/// The first benchmark's battles (its reports' parts) where they differ.
const FIRST: [SceneInfo; 2] = [
    SceneInfo { id: "sapphire", title: "SAPPHIRE REACH", subtitle: n("Dawn on the sea"), features: [n("Water reflections"), n("Battle fleets"), n("Air war")] },
    SceneInfo { id: "highland", title: "HIGHLAND RIFT", subtitle: n("Rain on the river"), features: [n("Rain and lightning"), n("Burning woods"), n("Tower assault")] },
];

/// A scene by its id (`first`: as the first benchmark had it).
pub fn scene(id: &str, first: bool) -> Option<&'static SceneInfo> {
    let older = if first { FIRST.iter().find(|s| s.id == id) } else { None };
    older.or_else(|| SCENES.iter().find(|s| s.id == id))
}

/// What the effects test turns off, by name (to be translated).
pub fn effect_label(effect: ta_config::benchmark::Effect) -> &'static str {
    use ta_config::benchmark::Effect;
    match effect {
        Effect::High => n("High"),
        Effect::RayTracing => n("Ray tracing"),
        Effect::Reflections => n("Water reflections"),
        Effect::Shadows => n("Soft shadows"),
        Effect::AmbientOcclusion => n("Ambient occlusion"),
        Effect::Trees => n("3D trees and rocks"),
        Effect::HdSurfaces => n("HD surfaces"),
        Effect::Weather => n("Weather"),
        Effect::HeatHaze => n("Heat haze"),
        Effect::RenderScale => n("Full resolution"),
    }
}

/// A part's name as shown: a scene's world, or a test's name (translated).
pub fn part_title(id: &str) -> &'static str {
    match id {
        "settings" => crate::i18n::t("SETTINGS TEST"),
        "effects" => crate::i18n::t("EFFECTS TEST"),
        id => scene(id, false).map_or("", |s| s.title),
    }
}
