//! A mod's interface theme in a battle (its manifest's `[theme]`, `ta_vfs::mods::Theme`): the
//! battle's palette (`ta_ui::theme::set_battle_palette`) and its titles' font
//! (`ta_ui::theme::set_battle_font`), which the HUD is drawn in (`ta_ui::theme::hud`: the classic
//! and the modern HUD, egui's own widgets in them, the companion screen's HUD), and the modern
//! HUD's panel and bar pictures (`hud_skin`). The menus, the battle's dialogs and pages, the
//! tutorial's windows, the loading screen and the cockpit keep TA Reborn's own look. The theme is
//! the last mod's in the load order that has one; the side it is drawn for is that of the player
//! whose interface is shown (a spectator's, as they follow another, theirs). Off the battlefield,
//! or with the player's MODS' INTERFACE THEMES off, TA Reborn's own; under the menus' theme's
//! look (MENU THEME, `menu_theme::battle_in_look`), which a battle with the modern HUD takes,
//! TA Reborn's own too, graded into that look.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

use crate::hud_skin::ThemePictures;
use ta_ui::theme::{Palette, parse_colour};
use ta_vfs::mods::{ModInfo, ModKind, Theme, media_path};

/// Whether mods' themes may be drawn at all: captures turn them off with `TA_MOD_THEME=0` (read
/// once).
pub fn allowed_here() -> bool {
    static ALLOWED: OnceLock<bool> = OnceLock::new();
    *ALLOWED.get_or_init(|| std::env::var("TA_MOD_THEME").map_or(true, |v| v != "0"))
}

/// A mod's interface theme, with the folder it names its pictures and font from.
#[derive(Clone, Debug)]
pub struct ModTheme {
    pub id: String,
    pub dir: PathBuf,
    pub theme: Theme,
}

/// The theme of the mods in play: the last in the load order that has one (a folder's).
pub fn of(mods: &[ModInfo]) -> Option<ModTheme> {
    mods.iter().rev().find(|m| m.kind == ModKind::Directory && !m.manifest.theme.is_empty())
        .map(|m| ModTheme { id: m.manifest.id.clone(), dir: m.path.clone(), theme: m.manifest.theme.clone() })
}

impl ModTheme {
    /// The battle's palette for `side`: TA Reborn's with the theme's colours (a colour written
    /// otherwise than `#rrggbb` is left as TA Reborn's).
    pub fn palette(&self, side: &str) -> Palette {
        Palette::REBORN.themed(&|key| self.theme.colour(side, key).and_then(parse_colour))
    }

    /// The picture tiled behind the plates (`panel`) or the top bar (`bar`) for `side`, where the
    /// theme names one that is there.
    pub fn texture(&self, side: &str, which: &str) -> Option<PathBuf> {
        self.theme.texture(side, which).and_then(|named| media_path(&self.dir, named))
    }

    /// The theme's font file, where it names one that is there.
    pub fn font_path(&self) -> Option<PathBuf> {
        media_path(&self.dir, &self.theme.font)
    }

    /// The theme's font, where it names one that is there and reads.
    pub fn font(&self) -> Option<Arc<Vec<u8>>> {
        let path = self.font_path()?;
        // (As the mod check reads it: a font it passes is one drawn here.)
        match ta_data::presentation::read_theme_font(&path) {
            Ok(bytes) => Some(Arc::new(bytes)),
            Err(error) => {
                log::warn!("mod {}: the theme's font {}: {error}: TA Reborn's own is used", self.id, path.display());
                None
            }
        }
    }
}

/// A theme's font as read, with its file and that file's stamp (when it was last written, its
/// length): read again only when another file is named, or it changed on disk.
type ReadFont = (PathBuf, Option<(SystemTime, u64)>, Option<Arc<Vec<u8>>>);

/// What a battle's interface is drawn in now, so that it changes only when the theme or the side
/// does: the theme's mod and the side, the font read for it and the pictures.
#[derive(Default)]
pub struct Applied {
    /// The theme's mod and the side (lower case) it is drawn for, while it is.
    key: Option<(String, String)>,
    /// The theme's font as last read (kept from one battle to the next while its file is the
    /// same).
    font: Option<ReadFont>,
    /// The pictures behind the plates and the top bar (none: the side's own metal), as the HUDs
    /// are given them: a new one only when they change.
    pub pictures: Arc<ThemePictures>,
}

impl Applied {
    /// Draws the interface in `theme` for `side` (None for either: TA Reborn's own). The
    /// palette, the font and the pictures change only when the theme or the side does: every
    /// frame, it compares them as they are and makes nothing.
    pub fn apply(&mut self, ctx: &egui::Context, theme: Option<&ModTheme>, side: Option<&str>) {
        let now = theme.zip(side);
        let same = match (&self.key, now) {
            (Some((id, shown)), Some((theme, side))) => *id == theme.id && shown.eq_ignore_ascii_case(side),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        match now {
            Some((theme, side)) => {
                ta_ui::theme::set_battle_palette(theme.palette(side));
                let font = self.font_of(theme);
                ta_ui::theme::set_battle_font(ctx, font);
                let pictures = ThemePictures { panel: theme.texture(side, "panel"), bar: theme.texture(side, "bar") };
                if *self.pictures != pictures {
                    self.pictures = Arc::new(pictures);
                }
                self.key = Some((theme.id.clone(), side.to_ascii_lowercase()));
                log::info!("interface theme of mod {} for {side}", theme.id);
            }
            None => {
                ta_ui::theme::set_battle_palette(Palette::REBORN);
                ta_ui::theme::set_battle_font(ctx, None);
                if self.pictures.panel.is_some() || self.pictures.bar.is_some() {
                    self.pictures = Arc::default();
                }
                crate::hud_skin::forget_theme_pictures();
                self.key = None;
            }
        }
    }

    /// The font of `theme`: as read before, where it names the same file and that has not
    /// changed (a battle after another with the same mod), else read now.
    fn font_of(&mut self, theme: &ModTheme) -> Option<Arc<Vec<u8>>> {
        let path = theme.font_path()?;
        let stamp = std::fs::metadata(&path).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
        if let Some((read, read_stamp, font)) = &self.font
            && *read == path
            && *read_stamp == stamp
        {
            return font.clone();
        }
        let font = theme.font();
        self.font = Some((path, stamp, font.clone()));
        font
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ta_vfs::mods::ModManifest;

    fn module(id: &str, theme: &str) -> ModInfo {
        let manifest: ModManifest = toml::from_str(&format!("id = \"{id}\"\nname = \"{id}\"\n{theme}")).unwrap();
        ModInfo { manifest, path: PathBuf::from(id), kind: ModKind::Directory }
    }

    /// The theme is the last mod's in the load order that has one; a mod without one, after it,
    /// leaves it.
    #[test]
    fn the_theme_is_the_last_themed_mods_in_the_load_order() {
        let mods = [module("a", "[theme]\naccent = \"#ff0000\""), module("b", "[theme]\naccent = \"#00ff00\""), module("c", "")];
        assert_eq!(of(&mods).map(|t| t.id), Some("b".to_string()));
        assert!(of(&mods[2..]).is_none());
    }

    /// A theme's palette is TA Reborn's with its colours, a side's own over the theme's, and a
    /// colour that is no colour left as TA Reborn's.
    #[test]
    fn a_themes_palette_takes_its_colours_and_a_sides_own() {
        let theme = of(&[module("t", "[theme]\naccent = \"#5ab4ff\"\ntext = \"oops\"\nmetal = \"#88aacc\"\n[theme.sides.core]\naccent = \"#ce4834\"")]).unwrap();
        let arm = theme.palette("ARM");
        assert_eq!(arm.accent, egui::Color32::from_rgb(0x5a, 0xb4, 0xff));
        assert_ne!(arm.accent_bright, Palette::REBORN.accent_bright, "its shades made from it");
        assert_eq!(arm.text, Palette::REBORN.text, "no colour: TA Reborn's");
        assert_eq!(arm.metal_bar, egui::Color32::from_rgb(0x88, 0xaa, 0xcc));
        assert_eq!(arm.panel, Palette::REBORN.panel, "left out: TA Reborn's");
        assert_eq!(theme.palette("Core").accent, egui::Color32::from_rgb(0xce, 0x48, 0x34));
    }

    /// TA Reborn's own colours given as a theme make (nearly) TA Reborn's palette: the shades a
    /// theme's colours are made into are those of TA Reborn's own.
    #[test]
    fn reborns_colours_as_a_theme_make_reborns_palette() {
        let hex = |c: egui::Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
        let p = Palette::REBORN;
        let theme = of(&[module("t", &format!("[theme]\naccent = \"{}\"\ntext = \"{}\"\ndim = \"{}\"\npanel = \"{}\"\ngood = \"{}\"\nbad = \"{}\"", hex(p.accent), hex(p.text), hex(p.steel), hex(p.panel), hex(p.good), hex(p.bad)))]).unwrap();
        let made = theme.palette("arm");
        let near = |a: egui::Color32, b: egui::Color32, name: &str| {
            let d = (a.r() as i32 - b.r() as i32).abs().max((a.g() as i32 - b.g() as i32).abs()).max((a.b() as i32 - b.b() as i32).abs());
            assert!(d <= 12, "{name}: {a:?} against {b:?}");
        };
        for (name, a, b) in [("accent_bright", made.accent_bright, p.accent_bright), ("accent_dim", made.accent_dim, p.accent_dim), ("accent_deep", made.accent_deep, p.accent_deep),
            ("on_accent", made.on_accent, p.on_accent), ("selection", made.selection, p.selection), ("text_bright", made.text_bright, p.text_bright), ("steel_dim", made.steel_dim, p.steel_dim),
            ("bg", made.bg, p.bg), ("bg_deep", made.bg_deep, p.bg_deep), ("panel_hi", made.panel_hi, p.panel_hi), ("faint_bg", made.faint_bg, p.faint_bg), ("widget", made.widget, p.widget),
            ("widget_hover", made.widget_hover, p.widget_hover), ("line", made.line, p.line), ("line_hi", made.line_hi, p.line_hi), ("good_deep", made.good_deep, p.good_deep), ("bad_deep", made.bad_deep, p.bad_deep)] {
            near(a, b, name);
        }
    }

    /// A themed mod in a folder of the test's own: a font (a copy of the interface's own, which
    /// reads), a panel picture for every side and a bar for Core's, and Core's accent.
    fn themed_mod(name: &str) -> ModTheme {
        let dir = std::env::temp_dir().join(format!("ta_mod_theme_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("media")).unwrap();
        let font = ta_ui::theme::ui_face(ta_ui::theme::Weight::Display).bytes;
        std::fs::write(dir.join("media/Font.ttf"), font).unwrap();
        for picture in ["panel.png", "core_bar.png"] {
            image::RgbaImage::from_pixel(8, 8, image::Rgba([30, 40, 50, 255])).save(dir.join("media").join(picture)).unwrap();
        }
        let manifest = "id = \"t\"\nname = \"t\"\n[theme]\naccent = \"#5ab4ff\"\nfont = \"media/Font.ttf\"\npanel_texture = \"media/panel.png\"\n[theme.sides.core]\naccent = \"#ce4834\"\nbar_texture = \"media/core_bar.png\"";
        let manifest: ModManifest = toml::from_str(manifest).unwrap();
        of(&[ModInfo { manifest, path: dir, kind: ModKind::Directory }]).unwrap()
    }

    /// The interface takes a theme only when the theme or the side changes: frame after frame, the
    /// same theme and side (any case) make nothing new; another side changes the palette and the
    /// pictures, never reads the font again; put away (off the battlefield, before a loading
    /// screen), TA Reborn's own, and the next battle with the same mod's theme finds its font read.
    #[test]
    fn a_theme_is_taken_only_when_it_or_the_side_changes() {
        let theme = themed_mod("applied");
        let ctx = egui::Context::default();
        crate::theme::install_compact(&ctx);
        let mut applied = Applied::default();
        applied.apply(&ctx, Some(&theme), Some("ARM"));
        assert_eq!(ta_ui::theme::battle_palette().accent, egui::Color32::from_rgb(0x5a, 0xb4, 0xff));
        assert!(ta_ui::theme::has_battle_font(&ctx));
        let font = applied.font.as_ref().and_then(|(_, _, font)| font.clone()).expect("its font read");
        let pictures = applied.pictures.clone();
        assert!(pictures.panel.is_some() && pictures.bar.is_none());
        for side in ["ARM", "arm", "Arm"] {
            applied.apply(&ctx, Some(&theme), Some(side));
            assert!(Arc::ptr_eq(&pictures, &applied.pictures), "{side}: nothing made again");
        }
        applied.apply(&ctx, Some(&theme), Some("CORE"));
        assert_eq!(ta_ui::theme::battle_palette().accent, egui::Color32::from_rgb(0xce, 0x48, 0x34), "Core's own");
        assert!(applied.pictures.bar.is_some() && applied.pictures.panel == pictures.panel);
        let again = applied.font.as_ref().and_then(|(_, _, font)| font.clone()).unwrap();
        assert!(Arc::ptr_eq(&font, &again), "its font not read again for another side");
        applied.apply(&ctx, None, None);
        assert_eq!(ta_ui::theme::battle_palette(), Palette::REBORN);
        assert!(!ta_ui::theme::has_battle_font(&ctx));
        assert_eq!(*applied.pictures, ThemePictures::default());
        applied.apply(&ctx, Some(&theme), Some("arm"));
        let next = applied.font.as_ref().and_then(|(_, _, font)| font.clone()).unwrap();
        assert!(Arc::ptr_eq(&font, &next), "the next battle with the mod: its font as read");
        assert!(ta_ui::theme::has_battle_font(&ctx));
        applied.apply(&ctx, None, None);
        let _ = std::fs::remove_dir_all(&theme.dir);
    }
}
