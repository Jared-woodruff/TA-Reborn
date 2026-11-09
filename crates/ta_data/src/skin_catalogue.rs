//! Authoring reference generated from the player's own resolved unit definitions.
//! Only names are listed: original textures, geometry and scripts are never exported.

use std::fmt::Write;
use ta_formats::model3do::Model3do;
use ta_vfs::Vfs;

/// A TOML guide for skin authors, including modded units in the mounted VFS.
/// This is reference metadata, not a skin manifest or simulation input.
pub fn skin_catalogue(vfs: &Vfs) -> String {
    let data = crate::GameData::load(vfs);
    let mut units: Vec<_> = data.units.iter().collect();
    units.sort_by_key(|u| u.name.to_ascii_lowercase());
    let mut out = String::from("# Unit texture reference, generated from your installed game and enabled mods.\n\
        # Copy a texture identifier into [units.<id>.textures] in skin.toml.\n\
        # '*' covers eligible armour; '_untextured' targets plain-colour armour faces.\n\
        # Team colours, animated lights, tracks and functional markings are protected.\n\
        # This guide contains names only, not original art; it is not exported in .taskin packs.\n\n");
    for unit in units {
        let object = unit.object.to_ascii_lowercase();
        let model = vfs.read_opt(&format!("objects3d/{object}.3do"))
            .and_then(|bytes| Model3do::parse(&bytes).ok());
        let _ = writeln!(out, "[[unit]]\nid = {}\nname = {}\nobject = {}\nmodel_available = {}",
            quote(&unit.name.to_ascii_lowercase()), quote(&unit.display_name), quote(&object), model.is_some());
        let names = model.as_ref().map(Model3do::texture_names).unwrap_or_default();
        let _ = writeln!(out, "textures = [{}]\n", names.iter().map(|s| quote(&s.to_ascii_lowercase())).collect::<Vec<_>>().join(", "));
    }
    out
}

fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => { let _ = write!(out, "\\u{:04X}", c as u32); }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
