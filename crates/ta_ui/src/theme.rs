//! Visual identity of the launcher and the game, after Total Annihilation's own screens:
//! grimy brown-black metal, steel pipes and gunmetal plates, olive-black wells, light
//! stencil-like caps, brass for the main action and focus, TA's green lamps and red for
//! status. The materials are painted by [`crate::metal`]; this holds the palette, the fonts
//! and the egui style.
//!
//! Text uses Bahnschrift, the DIN-style variable font that ships with Windows 10 and later.
//! It is read from the system font folder at runtime (never embedded or redistributed); its
//! weight and width axes provide regular, semi-bold and condensed-bold faces from one file.
//! Where it is missing (Linux, the Steam Deck, macOS) the interface is set in Sofia Sans, a
//! technical sans of the same kind with Latin, Greek and Cyrillic, bundled under the SIL Open
//! Font License (`fonts/`): Semi Condensed for text, Extra Condensed for titles, its lines as tall
//! as Bahnschrift's and its text a little narrower ([`ui_face`]). Chinese, Japanese
//! and Korean, which both lack, come from the system's fonts (see [`cjk_faces`]). Every
//! face's capitals are set in the middle of their line (see [`VMetrics::cap_centring`]), so
//! text centred in a box or beside a lamp, a bullet or an arrow sits level with it.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use egui::epaint::text::VariationCoords;
use egui::style::{HandleShape, Selection, WidgetVisuals, Widgets};
use egui::{
    Color32, CornerRadius, CursorIcon, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Margin,
    Shadow, Stroke, Style, TextStyle, Theme, Visuals, vec2,
};

// ---- Palette ------------------------------------------------------------------------------

/// Deepest background: header, navigation rail and status bar.
pub const BG_DEEP: Color32 = Color32::from_rgb(14, 13, 11);
/// Page background behind the cards.
pub const BG: Color32 = Color32::from_rgb(20, 19, 16);
/// Card surface.
pub const PANEL: Color32 = Color32::from_rgb(31, 30, 26);
/// Raised surface: hovered rows, the selected navigation item.
pub const PANEL_HI: Color32 = Color32::from_rgb(44, 42, 36);
/// Recessed wells (TA's olive-black): text fields, lists, slider rails.
pub const INSET: Color32 = Color32::from_rgb(13, 14, 9);
/// Hairlines and card borders.
pub const LINE: Color32 = Color32::from_rgb(60, 57, 50);
/// Stronger borders: buttons and inputs.
pub const LINE_HI: Color32 = Color32::from_rgb(90, 87, 78);
/// Secondary text (TA's beige-grey labels).
pub const STEEL: Color32 = Color32::from_rgb(158, 153, 138);
/// Tertiary text and inactive glyphs.
pub const STEEL_DIM: Color32 = Color32::from_rgb(124, 119, 106);
/// Body text.
pub const TEXT: Color32 = Color32::from_rgb(216, 211, 196);
/// Headings and emphasised values.
pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(243, 239, 227);
/// Primary accent, brass: the main action, focus and selection. (Named for the amber it
/// was; the names stay so every screen follows the palette.)
pub const AMBER: Color32 = Color32::from_rgb(216, 180, 86);
/// Hovered brass.
pub const AMBER_BRIGHT: Color32 = Color32::from_rgb(242, 210, 124);
/// Pressed brass and brass hairlines.
pub const AMBER_DIM: Color32 = Color32::from_rgb(142, 113, 48);
/// Brass-tinted dark fill for selected and active states.
pub const AMBER_DEEP: Color32 = Color32::from_rgb(56, 46, 21);
/// Text drawn on brass.
pub const ON_AMBER: Color32 = Color32::from_rgb(28, 21, 8);
/// Success, "present", TA's indicator lamps.
pub const GREEN: Color32 = Color32::from_rgb(120, 214, 92);
/// Dark green fill behind success badges.
pub const GREEN_DEEP: Color32 = Color32::from_rgb(26, 44, 20);
/// Errors, "missing".
pub const RED: Color32 = Color32::from_rgb(224, 94, 74);
/// Dark red fill behind error badges.
pub const RED_DEEP: Color32 = Color32::from_rgb(58, 23, 18);
/// Information: replays, radar, allies.
pub const SKY: Color32 = Color32::from_rgb(104, 184, 232);
/// TA's button lettering: pale cream (its in-game buttons' stencil caps).
pub const CREAM: Color32 = Color32::from_rgb(226, 222, 182);

/// The spacing scale, in points: between the parts of one control (`XS`), related items
/// (`S`), groups (`M`), a card's contents and its edge (`L`), cards (`XL`) and a page's
/// regions (`XXL`).
pub mod space {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
    pub const XXL: f32 = 32.0;
}

// ---- Fonts --------------------------------------------------------------------------------

/// Condensed bold family for titles and the Play button.
pub const DISPLAY_FAMILY: &str = "ta-display";
/// Semi-bold family for buttons, labels and navigation.
pub const STRONG_FAMILY: &str = "ta-strong";

/// Condensed bold display text.
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(DISPLAY_FAMILY.into()))
}

/// Semi-bold text.
pub fn strong(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(STRONG_FAMILY.into()))
}

/// Regular body text.
pub fn body(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Monospaced text (paths, file names).
pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// The weights of the interface's text: [`body`], [`strong`] and [`display`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Strong,
    Display,
}

/// A face of the interface's typeface: a font file, and the variation coordinates that make the
/// face of it.
#[derive(Clone, Copy, Debug)]
pub struct UiFace {
    pub bytes: &'static [u8],
    pub coords: &'static [(&'static [u8; 4], f32)],
}

/// Sofia Sans (Lettersoup: Botio Nikoltchev and Ani Petrova; the SIL Open Font License 1.1, its
/// text in `fonts/OFL.txt`) for where Bahnschrift is missing: Semi Condensed for text, Extra
/// Condensed for titles (as wide as Bahnschrift's condensed bold; Condensed is a sixth wider),
/// each varying by weight. At the same size its lines are as tall as Bahnschrift's (1.2 em), so
/// the layouts made for Bahnschrift hold; its text is 3 to 14% narrower, its capitals 8% shorter.
const SOFIA_SANS_SEMI_CONDENSED: &[u8] = include_bytes!("../fonts/SofiaSansSemiCondensed.ttf");
const SOFIA_SANS_EXTRA_CONDENSED: &[u8] = include_bytes!("../fonts/SofiaSansExtraCondensed.ttf");

/// The interface's face for `weight`: Bahnschrift at that weight (and, for titles, its condensed
/// width) where Windows has it and it can be read, else Sofia Sans's file for it at that weight.
/// `TA_UI_FONT=bundled` takes Sofia Sans anyway, to see the interface as Linux, the Steam Deck and
/// macOS show it.
pub fn ui_face(weight: Weight) -> UiFace {
    face_of(bahnschrift(), weight)
}

/// [`ui_face`] with Bahnschrift's bytes, or without it.
fn face_of(bahnschrift: Option<&'static [u8]>, weight: Weight) -> UiFace {
    let (bytes, coords): (&'static [u8], &'static [(&'static [u8; 4], f32)]) = match (bahnschrift, weight) {
        (Some(bytes), Weight::Regular) => (bytes, &[(b"wght", 400.0)]),
        (Some(bytes), Weight::Strong) => (bytes, &[(b"wght", 600.0)]),
        (Some(bytes), Weight::Display) => (bytes, &[(b"wght", 700.0), (b"wdth", 75.0)]),
        (None, Weight::Regular) => (SOFIA_SANS_SEMI_CONDENSED, &[(b"wght", 400.0)]),
        (None, Weight::Strong) => (SOFIA_SANS_SEMI_CONDENSED, &[(b"wght", 600.0)]),
        (None, Weight::Display) => (SOFIA_SANS_EXTRA_CONDENSED, &[(b"wght", 700.0)]),
    };
    UiFace { bytes, coords }
}

/// Installs the fonts and the dark style. Returns `true` if Bahnschrift, Windows' font, was
/// found (else the interface is in Sofia Sans, bundled).
pub fn install(ctx: &egui::Context) -> bool {
    let system_font = install_fonts(ctx);
    follow_language(ctx);
    // A single, dark identity regardless of the OS theme.
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, apply_style);
    system_font
}

/// Like [`install`], with the dense text and spacing of [`compact`] everywhere (the game:
/// its HUD, tooltips and drop-down lists); [`crate::panel::Panel`]s switch their contents
/// back to [`roomy`].
pub fn install_compact(ctx: &egui::Context) -> bool {
    let system_font = install_fonts(ctx);
    follow_language(ctx);
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        apply_style(style);
        compact(style);
    });
    system_font
}

fn install_fonts(ctx: &egui::Context) -> bool {
    let bahnschrift = bahnschrift();
    ctx.set_fonts(definitions(bahnschrift, &crate::i18n::language()));
    bahnschrift.is_some()
}

/// The fonts of the interface in `language`: Bahnschrift's faces (from its bytes, else Sofia
/// Sans's), egui's own behind them for what they lack, then the system's Chinese, Japanese and
/// Korean faces, the language's own first.
fn definitions(bahnschrift: Option<&'static [u8]>, language: &str) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let mut fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    // Hack carries the arrows and geometric shapes (▲ ▼ → ●) that Ubuntu-Light lacks.
    if fonts.font_data.contains_key("Hack") {
        fallbacks.push("Hack".to_owned());
    }
    if let Some(proportional) = fonts.families.get_mut(&FontFamily::Proportional) {
        proportional.clone_from(&fallbacks);
    }

    // (Other scripts' faces are set on the interface font's baseline: egui puts each at its
    // own ascent.)
    let ui_metrics = vmetrics(face_of(bahnschrift, Weight::Regular).bytes, 0);
    // egui's own faces (the fallbacks for the symbols the interface's lacks, and the monospace)
    // with their capitals in the middle of their lines too: then they are level with it.
    for name in ["Hack", "Ubuntu-Light"] {
        if let Some(data) = fonts.font_data.get(name)
            && let Some(m) = vmetrics(&data.font, data.index)
        {
            let mut data = FontData::clone(data);
            data.tweak.y_offset_factor = m.cap_centring();
            fonts.font_data.insert(name.to_owned(), Arc::new(data));
        }
    }
    // The interface's faces, each with its capitals in the middle of its line.
    let face = |weight: Weight| {
        let face = face_of(bahnschrift, weight);
        let centring = vmetrics(face.bytes, 0).map_or(0.0, |m| m.cap_centring());
        Arc::new(FontData::from_static(face.bytes).tweak(FontTweak {
            coords: VariationCoords::new(face.coords.iter().copied()),
            y_offset_factor: centring,
            ..Default::default()
        }))
    };
    fonts.font_data.insert("ui-regular".into(), face(Weight::Regular));
    fonts.font_data.insert("ui-strong".into(), face(Weight::Strong));
    fonts.font_data.insert("ui-display".into(), face(Weight::Display));
    if let Some(proportional) = fonts.families.get_mut(&FontFamily::Proportional) {
        proportional.insert(0, "ui-regular".into());
    }
    let mut strong = vec!["ui-strong".to_owned()];
    let mut display = vec!["ui-display".to_owned()];
    strong.extend(fallbacks.iter().cloned());
    display.extend(fallbacks);
    // Chinese, Japanese and Korean after them: the interface language's faces first.
    let (regular, bold) = cjk_faces(language, ui_metrics);
    for (name, data) in regular.iter().chain(&bold) {
        fonts.font_data.insert(name.clone(), data.clone());
    }
    let names = |faces: &[(String, Arc<FontData>)]| faces.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.extend(names(&regular));
        }
    }
    for family in [&mut strong, &mut display] {
        family.extend(names(&bold));
        family.extend(names(&regular));
    }
    // Named families must always be bound, or egui panics when they are used.
    fonts.families.insert(FontFamily::Name(STRONG_FAMILY.into()), strong);
    fonts.families.insert(FontFamily::Name(DISPLAY_FAMILY.into()), display);
    fonts
}

/// Puts the fonts in again at the start of the frame after the interface's language
/// changes (its own Chinese, Japanese or Korean faces go first).
fn follow_language(ctx: &egui::Context) {
    let id = egui::Id::new("ta-fonts-language");
    let watching = ctx.data_mut(|d| d.get_temp::<u32>(id)).is_some();
    ctx.data_mut(|d| d.insert_temp(id, crate::i18n::generation()));
    if watching {
        return;
    }
    ctx.on_begin_pass(
        "ta-fonts-language",
        Arc::new(move |ui: &mut egui::Ui| {
            let ctx = ui.ctx().clone();
            let now = crate::i18n::generation();
            if ctx.data_mut(|d| d.get_temp::<u32>(id)) != Some(now) {
                ctx.data_mut(|d| d.insert_temp(id, now));
                install_fonts(&ctx);
                ctx.request_repaint();
            }
        }),
    );
}

/// Chinese, Japanese and Korean.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Cjk {
    Simplified,
    Traditional,
    Japanese,
    Korean,
}

impl Cjk {
    /// The script of a language's code ("zh-Hans", "zh-TW", "ja", "ko").
    fn of(language: &str) -> Option<Cjk> {
        let code = language.to_ascii_lowercase();
        let primary = code.split('-').next().unwrap_or("");
        match primary {
            "zh" if code.split('-').skip(1).any(|p| matches!(p, "hant" | "tw" | "hk" | "mo")) => Some(Cjk::Traditional),
            "zh" => Some(Cjk::Simplified),
            "ja" => Some(Cjk::Japanese),
            "ko" => Some(Cjk::Korean),
            _ => None,
        }
    }

    /// Its face in Windows' font folder (file, face), regular or bold: Microsoft YaHei UI,
    /// Microsoft JhengHei UI, Yu Gothic UI, Malgun Gothic.
    fn windows(self, bold: bool) -> (&'static str, u32) {
        match (self, bold) {
            (Cjk::Simplified, false) => ("msyh.ttc", 1),
            (Cjk::Simplified, true) => ("msyhbd.ttc", 1),
            (Cjk::Traditional, false) => ("msjh.ttc", 1),
            (Cjk::Traditional, true) => ("msjhbd.ttc", 1),
            (Cjk::Japanese, false) => ("YuGothM.ttc", 1),
            (Cjk::Japanese, true) => ("YuGothB.ttc", 1),
            (Cjk::Korean, false) => ("malgun.ttf", 0),
            (Cjk::Korean, true) => ("malgunbd.ttf", 0),
        }
    }

    /// Its face in Noto Sans CJK's collection (Linux).
    fn noto(self) -> u32 {
        match self {
            Cjk::Japanese => 0,
            Cjk::Korean => 1,
            Cjk::Simplified => 2,
            Cjk::Traditional => 3,
        }
    }

    /// Its face as a font file from the system's fonts, when it has it: the file's bytes and the
    /// face in it.
    fn file(self, bold: bool) -> Option<(&'static [u8], u32)> {
        const NOTO: [&str; 4] = ["/usr/share/fonts/opentype/noto", "/usr/share/fonts/noto-cjk", "/usr/share/fonts/google-noto-cjk", "/usr/share/fonts/OTF"];
        let (file, index) = self.windows(bold);
        let noto_file = if bold { "NotoSansCJK-Bold.ttc" } else { "NotoSansCJK-Regular.ttc" };
        std::iter::once((windows_fonts().join(file), index)).chain(NOTO.iter().map(|dir| (Path::new(dir).join(noto_file), self.noto()))).find_map(|(path, index)| {
            let bytes = font_file(&path)?;
            has_face(bytes, index).then_some((bytes, index))
        })
    }

    /// The face from the system's fonts, when it has it, on a baseline with the interface
    /// font's (whose metrics are `ui`).
    fn face(self, bold: bool, ui: Option<VMetrics>) -> Option<Arc<FontData>> {
        const NOTO: [&str; 4] = ["/usr/share/fonts/opentype/noto", "/usr/share/fonts/noto-cjk", "/usr/share/fonts/google-noto-cjk", "/usr/share/fonts/OTF"];
        let (file, index) = self.windows(bold);
        let noto_file = if bold { "NotoSansCJK-Bold.ttc" } else { "NotoSansCJK-Regular.ttc" };
        std::iter::once((windows_fonts().join(file), index))
            .chain(NOTO.iter().map(|dir| (Path::new(dir).join(noto_file), self.noto())))
            .find_map(|(path, index)| {
                let bytes = font_file(&path)?;
                // (egui puts its baseline its own ascent below the top of its line, and
                // centres its line on the interface font's: moved onto that font's baseline,
                // as its capitals are centred.)
                let lift = ui.zip(vmetrics(bytes, index)).map_or(0.0, |(ui, own)| ui.ascent + ui.cap_centring() - own.ascent - (ui.row() - own.row()) / 2.0);
                has_face(bytes, index).then(|| Arc::new(FontData { font: Cow::Borrowed(bytes), index, tweak: FontTweak { y_offset_factor: lift, ..Default::default() } }))
            })
    }
}

/// Font faces by the names they are put in under.
type Faces = Vec<(String, Arc<FontData>)>;

/// Font files read so far, by path (none: unreadable).
type FontFiles = Mutex<Vec<(PathBuf, Option<&'static [u8]>)>>;

/// The faces for the scripts Bahnschrift lacks, from the system's fonts (never shipped):
/// (regular, bold). The interface language's first, regular and bold, so the characters
/// Chinese and Japanese share take its shapes; then Chinese and Korean, for names and
/// chat in them (and the languages' own names in the list of languages).
fn cjk_faces(language: &str, ui: Option<VMetrics>) -> (Faces, Faces) {
    let own = Cjk::of(language);
    let mut regular = Vec::new();
    let mut bold = Vec::new();
    let name = |script: Cjk, bold: bool| {
        let (file, index) = script.windows(bold);
        format!("cjk-{file}-{index}")
    };
    if let Some(script) = own
        && let Some(face) = script.face(true, ui)
    {
        bold.push((name(script, true), face));
    }
    for script in own.into_iter().chain([Cjk::Simplified, Cjk::Korean]) {
        if !regular.iter().any(|(n, _)| *n == name(script, false))
            && let Some(face) = script.face(false, ui)
        {
            regular.push((name(script, false), face));
        }
    }
    (regular, bold)
}

/// A face's vertical metrics, in ems.
#[derive(Clone, Copy, Debug)]
struct VMetrics {
    ascent: f32,
    /// (Below the baseline: positive.)
    descent: f32,
    gap: f32,
    /// The height of its capitals.
    cap: f32,
}

impl VMetrics {
    /// The height of a line of it.
    fn row(&self) -> f32 {
        self.ascent + self.descent + self.gap
    }


    /// How far down to move its glyphs (in ems) so its capitals sit in the middle of their
    /// line: egui puts the baseline an ascent below the line's top and the whole line gap
    /// under the text, which leaves capitals high (Bahnschrift's by 0.16 em), while
    /// everything centres text by its line (buttons, fields, rows beside lamps and arrows).
    fn cap_centring(&self) -> f32 {
        self.row() / 2.0 - (self.ascent - self.cap / 2.0)
    }
}

/// A face's vertical metrics as egui reads them: OS/2's typographic ones when the font asks
/// for them, else hhea's (`skrifa`'s rule); its capitals' height from OS/2 (else an
/// estimate).
fn vmetrics(bytes: &[u8], index: u32) -> Option<VMetrics> {
    let be16 = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let be32 = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let offset = if bytes.starts_with(b"ttcf") { be32(12 + 4 * index as usize)? as usize } else { 0 };
    let tables = be16(offset + 4)? as usize;
    let table = |tag: &[u8; 4]| {
        (0..tables).find_map(|i| {
            let record = offset + 12 + 16 * i;
            if bytes.get(record..record + 4)? == tag { be32(record + 8).map(|o| o as usize) } else { None }
        })
    };
    let units = be16(table(b"head")? + 18)? as f32;
    let hhea = table(b"hhea")?;
    let hhea = (be16(hhea + 4)? as i16, be16(hhea + 6)? as i16, be16(hhea + 8)? as i16);
    let os2 = table(b"OS/2");
    let typo = os2.and_then(|os2| Some((be16(os2 + 62)?, (be16(os2 + 68)? as i16, be16(os2 + 70)? as i16, be16(os2 + 72)? as i16))));
    let (ascent, descent, gap) = match typo {
        Some((selection, typo)) if selection & 0x80 != 0 => typo,
        _ if hhea.0 != 0 || hhea.1 != 0 => hhea,
        Some((_, typo)) => typo,
        None => return None,
    };
    // (sCapHeight is in OS/2 from its version 2.)
    let cap = os2.filter(|&os2| be16(os2).is_some_and(|version| version >= 2)).and_then(|os2| be16(os2 + 88)).map(|c| c as i16).filter(|&c| c > 0);
    let em = |v: i16| v as f32 / units;
    (units > 0.0).then(|| VMetrics { ascent: em(ascent), descent: em(descent).abs(), gap: em(gap).max(0.0), cap: cap.map_or(em(ascent) * 0.7, em) })
}

/// The system's font folder (Windows').
fn windows_fonts() -> PathBuf {
    std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join("Fonts")
}

/// A font file's bytes, read once and kept: every interface (and every change of language)
/// shares them.
fn font_file(path: &Path) -> Option<&'static [u8]> {
    static FILES: FontFiles = Mutex::new(Vec::new());
    let mut files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, bytes)) = files.iter().find(|(p, _)| p == path) {
        return *bytes;
    }
    let bytes = std::fs::read(path).ok().map(|b| &*Box::leak(b.into_boxed_slice()));
    files.push((path.to_path_buf(), bytes));
    bytes
}

/// Whether the font file has a face `index` with a table directory (egui panics on font
/// data it cannot parse).
fn has_face(bytes: &[u8], index: u32) -> bool {
    let be32 = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let be16 = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let offset = if bytes.starts_with(b"ttcf") {
        match be32(8) {
            Some(count) if index < count => be32(12 + 4 * index as usize).map_or(usize::MAX, |o| o as usize),
            _ => return false,
        }
    } else if index == 0 {
        0
    } else {
        return false;
    };
    // TrueType, CFF ("OTTO") or Apple's "true", and the directory's records in the file.
    matches!(be32(offset), Some(0x0001_0000 | 0x4F54_544F | 0x7472_7565)) && be16(offset + 4).is_some_and(|tables| bytes.len() >= offset + 12 + 16 * tables as usize)
}

/// The system's faces for the scripts Bahnschrift lacks (Chinese, Japanese and Korean), as font
/// files and the face in each, the interface language's own first: for text drawn outside the
/// interface, in the world (the credits' letters, `apps/ta_game/src/world_text.rs`). None when
/// the system has none.
pub fn fallback_faces() -> Vec<(&'static [u8], u32)> {
    let own = Cjk::of(&crate::i18n::language());
    let mut faces: Vec<(&'static [u8], u32)> = Vec::new();
    for script in own.into_iter().chain([Cjk::Simplified, Cjk::Japanese, Cjk::Korean, Cjk::Traditional]) {
        if let Some(face) = script.file(false)
            && !faces.iter().any(|f| std::ptr::eq(f.0.as_ptr(), face.0.as_ptr()) && f.1 == face.1)
        {
            faces.push(face);
        }
    }
    faces
}

/// Bahnschrift from the Windows font folder, if present and parseable (and not passed by:
/// `TA_UI_FONT=bundled`), read once.
fn bahnschrift() -> Option<&'static [u8]> {
    static FOUND: OnceLock<Option<&'static [u8]>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        if std::env::var_os("TA_UI_FONT").is_some_and(|v| v.eq_ignore_ascii_case("bundled")) {
            return None;
        }
        let bytes = font_file(&windows_fonts().join("bahnschrift.ttf"))?;
        // egui panics on font data it cannot parse, so only accept a font whose variation axes
        // can be read — which also proves the weight axis we rely on exists.
        let has_weight_axis = has_face(bytes, 0) && FontData::from_static(bytes).variation_axes().iter().any(|axis| axis.tag.to_be_bytes() == *b"wght");
        if !has_weight_axis {
            log::info!("Bahnschrift unusable; the interface is in Sofia Sans");
            return None;
        }
        Some(bytes)
    })
}

// ---- Style --------------------------------------------------------------------------------

fn apply_style(style: &mut Style) {
    style.visuals = visuals();
    roomy(style);
    style.animation_time = 0.12;
}

/// Text sizes and spacing of the launcher and the game's pages and dialogs.
pub fn roomy(style: &mut Style) {
    style.text_styles = [
        (TextStyle::Small, body(12.0)),
        (TextStyle::Body, body(14.5)),
        (TextStyle::Button, strong(14.0)),
        (TextStyle::Heading, display(26.0)),
        (TextStyle::Monospace, mono(12.5)),
    ]
    .into();

    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(8.0, 7.0);
    spacing.button_padding = vec2(12.0, 5.0);
    spacing.interact_size = vec2(40.0, 26.0);
    spacing.slider_width = 230.0;
    spacing.combo_width = 220.0;
    spacing.text_edit_width = 230.0;
    spacing.icon_width = 16.0;
    spacing.icon_width_inner = 9.0;
    spacing.icon_spacing = 8.0;
    spacing.menu_margin = Margin::same(6);
    spacing.window_margin = Margin::same(12);
    // A scroll bar in sight before the pointer comes near (see `crate::scroll`).
    spacing.scroll = crate::scroll::style();
}

/// Denser text and spacing for the in-game HUD, with the same colours.
pub fn compact(style: &mut Style) {
    style.text_styles = [
        (TextStyle::Small, body(11.0)),
        (TextStyle::Body, body(13.0)),
        (TextStyle::Button, strong(12.5)),
        (TextStyle::Heading, display(22.0)),
        (TextStyle::Monospace, mono(11.5)),
    ]
    .into();
    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(8.0, 3.0);
    spacing.button_padding = vec2(4.0, 1.0);
    spacing.interact_size = vec2(40.0, 18.0);
    spacing.slider_width = 100.0;
    spacing.combo_width = 100.0;
    spacing.text_edit_width = 280.0;
    spacing.icon_width = 14.0;
    spacing.icon_width_inner = 8.0;
    spacing.icon_spacing = 4.0;
    spacing.menu_margin = Margin::same(6);
    spacing.window_margin = Margin::same(8);
    // egui's own scroll bars in the HUD (its build menu's grid keeps its width).
    spacing.scroll = egui::style::ScrollStyle::default();
}

/// Custom dark visuals: brown-black metal, square-ish corners, brass focus.
fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.weak_text_color = Some(STEEL);
    v.hyperlink_color = AMBER_BRIGHT;
    v.faint_bg_color = Color32::from_rgb(26, 25, 21);
    v.extreme_bg_color = INSET;
    v.text_edit_bg_color = Some(INSET);
    v.code_bg_color = PANEL_HI;
    v.warn_fg_color = AMBER;
    v.error_fg_color = RED;

    v.window_corner_radius = CornerRadius::same(3);
    v.window_fill = PANEL;
    v.window_stroke = Stroke::new(1.0, LINE_HI);
    v.window_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(150) };
    v.menu_corner_radius = CornerRadius::same(2);
    v.popup_shadow = Shadow { offset: [0, 6], blur: 16, spread: 0, color: Color32::from_black_alpha(150) };
    v.panel_fill = BG;

    v.selection = Selection { bg_fill: Color32::from_rgb(96, 78, 32), stroke: Stroke::new(1.0, AMBER) };
    v.widgets = widgets();
    v.text_cursor.stroke = Stroke::new(2.0, AMBER);
    v.slider_trailing_fill = true;
    v.handle_shape = HandleShape::Rect { aspect_ratio: 0.55 };
    v.interact_cursor = Some(CursorIcon::PointingHand);
    v
}

fn widgets() -> Widgets {
    let radius = CornerRadius::same(2);
    let visuals = |fill: Color32, border: Color32, text: Color32, text_width: f32| WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, border),
        corner_radius: radius,
        fg_stroke: Stroke::new(text_width, text),
        expansion: 0.0,
    };
    Widgets {
        // Labels, separators and frames.
        noninteractive: visuals(PANEL, LINE, TEXT, 1.0),
        inactive: visuals(Color32::from_rgb(40, 39, 34), LINE_HI, TEXT, 1.0),
        hovered: visuals(Color32::from_rgb(54, 52, 45), AMBER_DIM, TEXT_BRIGHT, 1.5),
        active: visuals(AMBER_DEEP, AMBER, AMBER_BRIGHT, 2.0),
        open: visuals(PANEL_HI, AMBER_DIM, TEXT_BRIGHT, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every face's capitals sit in the middle of their line: Bahnschrift's (where Windows has
    /// it) and Sofia Sans's.
    #[test]
    fn capitals_sit_in_the_middle_of_their_line() {
        for bahnschrift in [bahnschrift(), None] {
            let ctx = egui::Context::default();
            install(&ctx);
            ctx.set_fonts(definitions(bahnschrift, "en"));
            ctx.run_ui(Default::default(), |ui| {
                for font in [display(30.0), strong(12.0), body(14.5)] {
                    let galley = ui.painter().layout_no_wrap("H".to_owned(), font.clone(), Color32::WHITE);
                    let glyph = &galley.rows[0].row.glyphs[0];
                    let ink = galley.rows[0].pos.y + glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y / 2.0;
                    assert!((ink - galley.size().y / 2.0).abs() < 0.75, "{font:?} in {bahnschrift:?}: capitals centred at {ink}, the line's middle {}", galley.size().y / 2.0, bahnschrift = bahnschrift.map(|_| "Bahnschrift"));
                }
            })
            .drop_without_applying_deltas();
        }
    }

    /// Without Bahnschrift the interface is in Sofia Sans: its text and titles in their own
    /// files, its lines as tall as Bahnschrift's 1.2 em and its text no wider (where Windows has
    /// it to compare, laid out as the interface lays them out), so the layouts made for
    /// Bahnschrift hold.
    #[test]
    fn without_bahnschrift_the_interface_is_in_sofia_sans() {
        // (By their bytes: a debug build may keep two copies of one embedded file, at two places.)
        assert!(face_of(None, Weight::Regular).bytes == SOFIA_SANS_SEMI_CONDENSED);
        assert!(face_of(None, Weight::Strong).bytes == SOFIA_SANS_SEMI_CONDENSED);
        assert!(face_of(None, Weight::Display).bytes == SOFIA_SANS_EXTRA_CONDENSED);
        for bytes in [SOFIA_SANS_SEMI_CONDENSED, SOFIA_SANS_EXTRA_CONDENSED] {
            assert!(has_face(bytes, 0));
            let axes = FontData::from_static(bytes).variation_axes();
            assert!(axes.iter().any(|axis| axis.tag.to_be_bytes() == *b"wght"), "a weight axis");
            let line = vmetrics(bytes, 0).unwrap().row();
            assert!((line - 1.2).abs() < 0.002, "a line {line} em high");
        }
        let Some(bahnschrift) = bahnschrift() else { return };
        // As tall and as wide, laid out as the interface lays them out.
        let size = |bahnschrift: Option<&'static [u8]>, text: &str, font: FontId| {
            let ctx = egui::Context::default();
            install(&ctx);
            ctx.set_fonts(definitions(bahnschrift, "en"));
            let mut size = egui::Vec2::ZERO;
            ctx.run_ui(Default::default(), |ui| size = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE).size()).drop_without_applying_deltas();
            size
        };
        for (text, font) in [
            ("Set up for this computer. They are yours to change in Settings.", body(14.5)),
            ("Настройки игры, звук и управление", body(14.5)),
            ("RESTORE DEFAULTS", strong(14.0)),
            ("SINGLE PLAYER", display(26.0)),
            ("НАСТРОЙКИ", display(26.0)),
        ] {
            let (ours, sofia) = (size(Some(bahnschrift), text, font.clone()), size(None, text, font.clone()));
            let wide = sofia.x / ours.x;
            assert!((0.84..=1.0).contains(&wide), "{text:?}: Sofia Sans {:.1} points wide, Bahnschrift {:.1} ({wide:.2})", sofia.x, ours.x);
            assert!((sofia.y - ours.y).abs() < 0.3, "{text:?}: a line of Sofia Sans {:.2} points high, of Bahnschrift {:.2}", sofia.y, ours.y);
        }
    }

    /// Sofia Sans draws Cyrillic as Russian and Ukrainian readers know it: the Bulgarian forms
    /// it draws by default (в as tall as b, т as wide as m) are mapped away in the bundled copy
    /// (`tools/fonts/sofia_sans.py`), since egui's shaping never asks for its Russian ones.
    #[test]
    fn sofia_sans_draws_cyrillic_in_the_forms_russian_and_ukrainian_use() {
        let ctx = egui::Context::default();
        install(&ctx);
        ctx.set_fonts(definitions(None, "ru"));
        ctx.run_ui(Default::default(), |ui| {
            for font in [body(40.0), display(40.0)] {
                // The ink of a letter: its width and height.
                let ink = |letter: &str| {
                    let galley = ui.painter().layout_no_wrap(letter.to_owned(), font.clone(), Color32::WHITE);
                    galley.rows[0].row.glyphs[0].uv_rect.size
                };
                let (ve, b, x) = (ink("в"), ink("b"), ink("x"));
                assert!(ve.y < b.y * 0.85 && (ve.y - x.y).abs() < x.y * 0.1, "{font:?}: в {:.1} high, b {:.1}, x {:.1}: in its Bulgarian form", ve.y, b.y, x.y);
                let (te, m) = (ink("т"), ink("m"));
                assert!(te.x < m.x * 0.8, "{font:?}: т {:.1} wide, m {:.1}: in its Bulgarian form", te.x, m.x);
            }
        })
        .drop_without_applying_deltas();
    }

    /// Where a font's table `tag` starts.
    fn table(font: &[u8], tag: &[u8; 4]) -> usize {
        let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
        let record = (0..tables).map(|i| 12 + 16 * i).find(|&r| &font[r..r + 4] == tag).unwrap_or_else(|| panic!("no {} table", String::from_utf8_lossy(tag)));
        u32::from_be_bytes(font[record + 8..record + 12].try_into().unwrap()) as usize
    }

    /// Whether a font's character map (its Unicode table for the Basic Multilingual Plane,
    /// format 4) has `c`.
    fn maps(font: &[u8], c: char) -> bool {
        let be16 = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]) as usize;
        let be32 = |at: usize| u32::from_be_bytes(font[at..at + 4].try_into().unwrap()) as usize;
        let cmap = table(font, b"cmap");
        let table = (0..be16(cmap + 2)).map(|i| cmap + 4 + 8 * i).find(|&r| (be16(r), be16(r + 2)) == (3, 1)).map(|r| cmap + be32(r + 4)).expect("Windows' Unicode table");
        assert_eq!(be16(table), 4, "format 4");
        let segments = be16(table + 6) / 2;
        let (ends, starts, deltas, offsets) = (table + 14, table + 16 + 2 * segments, table + 16 + 4 * segments, table + 16 + 6 * segments);
        let c = c as usize;
        if c > 0xFFFF {
            return false;
        }
        let Some(i) = (0..segments).find(|&i| be16(ends + 2 * i) >= c) else { return false };
        let start = be16(starts + 2 * i);
        if c < start {
            return false;
        }
        let offset = be16(offsets + 2 * i);
        let glyph = if offset == 0 {
            (c + be16(deltas + 2 * i)) & 0xFFFF
        } else {
            let at = offsets + 2 * i + offset + 2 * (c - start);
            match be16(at) {
                0 => 0,
                g => (g + be16(deltas + 2 * i)) & 0xFFFF,
            }
        };
        glyph != 0
    }

    /// Sofia Sans has every letter of every translation but Chinese, Japanese and Korean (which
    /// come from the system's fonts): Russian and Ukrainian above all (Barlow, first thought of,
    /// has no Cyrillic), Greek too.
    #[test]
    fn sofia_sans_has_every_letter_of_the_translations() {
        let cjk = |c: char| matches!(c as u32, 0x1100..=0x11FF | 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF);
        let lang = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lang");
        let mut letters = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(&lang).expect("the translations") {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "toml") {
                letters.extend(std::fs::read_to_string(&path).unwrap().chars().filter(|&c| c.is_alphanumeric() && !cjk(c)));
            }
        }
        assert!(['д', 'ї', 'ґ', 'ł', 'ğ', 'ã', 'ß'].iter().all(|c| letters.contains(c)), "the translations' letters read");
        // (And the letters of every language's own name, and Greek's, for names and chat.)
        letters.extend("ΑαΒβΓγΔδΕεΖζΗηΘθΙιΚκΛλΜμΝνΞξΟοΠπΡρΣσςΤτΥυΦφΧχΨψΩωάέήίόύώ".chars());
        for (name, font) in [("Sofia Sans Semi Condensed", SOFIA_SANS_SEMI_CONDENSED), ("Sofia Sans Extra Condensed", SOFIA_SANS_EXTRA_CONDENSED)] {
            let missing: String = letters.iter().filter(|&&c| !maps(font, c)).collect();
            assert!(missing.is_empty(), "{name} lacks {missing:?}");
        }
    }
}
