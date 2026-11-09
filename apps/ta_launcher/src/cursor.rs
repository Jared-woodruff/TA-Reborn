//! Reborn pointers: brass and gunmetal pictures (`tools/cursors/build.py`, one texture each with
//! mipmaps, so that they stay smooth at any size) put on the pointer's hotspot, and what moves
//! painted over them: the nano-lathe lamp breathing, a light running along the brass, a cog
//! turning while the launcher works. The hotspot never moves; native dialogs and the window's
//! borders keep their own pointers, and the system pointer is an opt-out.
use std::time::Duration;

use egui::{Color32, ColorImage, Context, CursorIcon, Id, LayerId, Mesh, Order, Painter, Pos2, Rect, Shape, Stroke,
    TextureFilter, TextureHandle, TextureId, TextureOptions, TextureWrapMode, Vec2, emath::Rot2, pos2, vec2};
use crate::theme::TEXT_BRIGHT;

/// Pixels a unit in the pictures (`tools/cursors/build.py` PX); a unit is a point at pointer size 1.
const PX: f32 = 6.0;

/// A pointer's picture: where its hotspot is in it (units), and how large it is drawn.
struct Sprite {
    name: &'static str,
    png: &'static [u8],
    hotspot: Vec2,
    scale: f32,
}

macro_rules! sprite {
    ($name:literal, $x:expr, $y:expr, $scale:expr) => {
        Sprite { name: $name, png: include_bytes!(concat!("../cursors/", $name, ".png")), hotspot: vec2($x, $y), scale: $scale }
    };
}

const ARROW: Sprite = sprite!("arrow", 3.0, 3.0, 1.0);
const HAND_POINT: Sprite = sprite!("hand_point", 13.0, 2.4, 0.85);
const HAND_OPEN: Sprite = sprite!("hand_open", 17.6, 20.0, 0.85);
const HAND_CLOSED: Sprite = sprite!("hand_closed", 17.6, 20.0, 0.85);
const IBEAM: Sprite = sprite!("ibeam", 8.0, 15.0, 1.0);
const GEAR: Sprite = sprite!("gear", 13.0, 13.0, 1.0);
const RESIZE: Sprite = sprite!("resize", 17.0, 8.0, 1.0);
const MOVE: Sprite = sprite!("move", 17.0, 17.0, 1.0);
const FORBIDDEN: Sprite = sprite!("forbidden", 14.0, 14.0, 1.0);
const CROSSHAIR: Sprite = sprite!("crosshair", 15.0, 15.0, 1.0);
const ZOOM_IN: Sprite = sprite!("zoom_in", 12.0, 12.0, 1.0);
const ZOOM_OUT: Sprite = sprite!("zoom_out", 12.0, 12.0, 1.0);
const BADGE_COPY: Sprite = sprite!("badge_copy", 8.0, 8.0, 1.0);
const BADGE_ALIAS: Sprite = sprite!("badge_alias", 8.0, 8.0, 1.0);
const BADGE_MENU: Sprite = sprite!("badge_menu", 8.0, 8.0, 1.0);
const BADGE_HELP: Sprite = sprite!("badge_help", 8.0, 8.0, 1.0);

/// The arrow's lamp (units from its hotspot) and the brass a light runs along: up its left
/// edge to the tip and down to its wing, a unit in from the outside.
const ARROW_LAMP: Vec2 = vec2(5.3, 16.6);
const ARROW_GLINT: [Vec2; 3] = [vec2(1.0, 26.4), vec2(1.0, 2.3), vec2(19.4, 19.3)];
/// The pointing hand's fingertip lamp, and the edge of its finger the light runs along.
const HAND_LAMP: Vec2 = vec2(0.0, 2.8);
const HAND_GLINT: [Vec2; 4] = [vec2(-1.6, 18.0), vec2(-1.6, 2.2), vec2(0.0, 0.9), vec2(1.6, 2.2)];
/// Where a badge or a small cog sits beside the arrow (its centre, units from the hotspot).
const BESIDE: Vec2 = vec2(20.0, 24.0);

/// The nano-lathe's green.
const NANO: Color32 = Color32::from_rgb(120, 236, 98);

#[derive(Clone, Copy, Default)]
struct Motion { previous: f64, elapsed: f32 }

fn animation_time(ctx: &Context, moving: bool) -> f32 {
    let now = ctx.input(|i| i.time);
    let id = Id::new("reborn_pointer_clock").with(ctx.viewport_id());
    ctx.data_mut(|data| {
        let mut clock = data.get_temp::<Motion>(id).unwrap_or(Motion { previous: now, elapsed: 0.0 });
        // Every movement below repeats within this period (a whole number of cog turns, lamp
        // breaths and glints), so wrapping a long session cannot make one jump.
        if moving { clock.elapsed = (clock.elapsed + (now - clock.previous).clamp(0.0, 0.1) as f32) % PERIOD; }
        clock.previous = now;
        data.insert_temp(id, clock);
        clock.elapsed
    })
}

/// The pointers' common period (seconds): 4 cog turns, 4 lamp breaths, 6 turns of the cog's
/// green and 2 glints.
const PERIOD: f32 = 9.0;

fn scale(ctx: &Context, config: &ta_config::Config) -> f32 {
    let size = config.gameplay.cursor_scale;
    let size = if size.is_finite() { size.clamp(1.0, 3.0) } else { 1.0 };
    // egui points already include interface zoom. Pointer accessibility size must
    // remain independent of UI zoom, while retaining the display's native DPI scale.
    size / ctx.zoom_factor()
}

/// Space below the hotspot occupied by the largest pointer/badge, plus a small gap.
pub(crate) fn tooltip_clearance(ctx: &Context, config: &ta_config::Config) -> f32 {
    if config.launcher.themed_cursor { 36.0 * scale(ctx, config) + 4.0 } else { 16.0 }
}

fn animated(icon: CursorIcon) -> bool {
    matches!(icon, CursorIcon::Default | CursorIcon::PointingHand | CursorIcon::Progress | CursorIcon::Wait
        | CursorIcon::Copy | CursorIcon::Alias | CursorIcon::ContextMenu | CursorIcon::Help)
}

pub fn show(ctx: &Context, config: &ta_config::Config) {
    // Explicit visual-QA contact sheet, absent from ordinary launcher sessions.
    static PREVIEW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let previewing = *PREVIEW.get_or_init(|| std::env::var_os("TA_CURSOR_PREVIEW").is_some());
    let (focused, at) = ctx.input(|i| {
        let focused = i.focused && i.viewport().focused.unwrap_or(true) && !i.viewport().minimized.unwrap_or(false);
        (focused, i.pointer.hover_pos())
    });
    let at = at.filter(|at| focused && ctx.content_rect().contains(*at));
    let icon = ctx.output(|output| output.cursor_icon);
    let visible = config.launcher.themed_cursor && at.is_some() && icon != CursorIcon::None;
    let moving = !config.gameplay.reduced_motion && !ta_ui::motion::still(ctx)
        && focused && (previewing || visible && animated(icon));
    let time = animation_time(ctx, moving);
    if previewing { preview(ctx, time); }
    if !visible { return; }
    // Debug is egui's top, noninteractive paint layer: tooltips and modal windows
    // must not cover a software pointer or intercept its original hit-test location.
    let painter = ctx.layer_painter(LayerId::new(Order::Debug, Id::new("reborn_pointer")));
    paint(&painter, at.unwrap(), icon, scale(ctx, config), config.gameplay.high_contrast_cursor, time);
    ctx.set_cursor_icon(CursorIcon::None);
    if moving { ctx.request_repaint_after(Duration::from_millis(33)); }
}

fn preview(ctx: &Context, time: f32) {
    use CursorIcon::*;
    egui::Window::new("Reborn pointer artwork").fixed_pos(pos2(180., 100.)).resizable(false).show(ctx, |ui| {
        for icons in [
            [Default, PointingHand, Text, VerticalText, Wait, Progress],
            [Grab, Grabbing, Move, Copy, Alias, ContextMenu],
            [Help, NotAllowed, ZoomIn, ZoomOut, ResizeHorizontal, ResizeNeSw],
        ] {
            ui.horizontal(|ui| {
                for icon in icons {
                    let (rect, _) = ui.allocate_exact_size(vec2(100., 115.), egui::Sense::hover());
                    paint(ui.painter(), rect.center() - vec2(4., 24.), icon, 1.5, false, time);
                    ui.painter().text(rect.center_bottom() - vec2(0., 8.), egui::Align2::CENTER_BOTTOM,
                        format!("{icon:?}"), egui::FontId::proportional(10.), TEXT_BRIGHT);
                }
            });
        }
    });
}

/// A picture's texture, made once a context: mipmapped, so that it is smooth at any size.
fn texture(ctx: &Context, sprite: &Sprite) -> Option<(TextureId, Vec2)> {
    let id = Id::new(("reborn_pointer_picture", sprite.name));
    let handle = match ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        Some(handle) => handle,
        None => {
            let picture = image::load_from_memory(sprite.png).ok()?.to_rgba8();
            let size = [picture.width() as usize, picture.height() as usize];
            let image = ColorImage::from_rgba_unmultiplied(size, picture.as_raw());
            let options = TextureOptions { magnification: TextureFilter::Linear, minification: TextureFilter::Linear,
                wrap_mode: TextureWrapMode::ClampToEdge, mipmap_mode: Some(TextureFilter::Linear) };
            let handle = ctx.load_texture(format!("pointer-{}", sprite.name), image, options);
            ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
            handle
        }
    };
    Some((handle.id(), handle.size_vec2() / PX))
}

/// Puts `sprite`'s hotspot at `at`, turned by `angle` about it.
fn picture(p: &Painter, at: Pos2, scale: f32, sprite: &Sprite, angle: f32) {
    let Some((texture, size)) = texture(p.ctx(), sprite) else { return };
    let s = scale * sprite.scale;
    let min = at - sprite.hotspot * s;
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    if angle == 0.0 {
        // Whole pixels: the picture as drawn, not resampled half a pixel over.
        let ppp = p.ctx().pixels_per_point();
        let min = pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp);
        p.image(texture, Rect::from_min_size(min, size * s), uv, Color32::WHITE);
        return;
    }
    let turn = Rot2::from_angle(angle);
    let mut mesh = Mesh::with_texture(texture);
    for (corner, uv) in [(vec2(0.0, 0.0), uv.left_top()), (vec2(size.x, 0.0), uv.right_top()), (size, uv.right_bottom()), (vec2(0.0, size.y), uv.left_bottom())] {
        mesh.vertices.push(egui::epaint::Vertex { pos: at + turn * ((min + corner * s) - at), uv, color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(Shape::mesh(mesh));
}

/// A nano-lathe lamp breathing: its glow swells and settles, never flashing.
fn lamp(p: &Painter, centre: Pos2, scale: f32, time: f32) {
    let breath = 0.5 - 0.5 * (time * std::f32::consts::TAU / (PERIOD / 4.0)).cos();
    let glow = |radius: f32, alpha: f32| p.circle_filled(centre, radius * scale, NANO.gamma_multiply(alpha));
    glow(3.4 + 0.6 * breath, 0.10 + 0.10 * breath);
    glow(2.2, 0.20 + 0.16 * breath);
    p.circle_filled(centre, 0.62 * scale, Color32::from_rgb(214, 255, 196).gamma_multiply(0.55 + 0.45 * breath));
}

/// A light running along the brass (`path`, units from the hotspot): every 4.5 seconds it sweeps
/// from the path's start to its end in 0.8 of one, brightest in its middle.
fn glint(p: &Painter, at: Pos2, scale: f32, path: &[Vec2], time: f32) {
    let phase = time % 4.5;
    if phase > 0.8 { return; }
    let lengths: Vec<f32> = path.windows(2).map(|w| (w[1] - w[0]).length()).collect();
    let total: f32 = lengths.iter().sum();
    let at_length = |mut d: f32| -> Vec2 {
        for (i, length) in lengths.iter().enumerate() {
            if d <= *length || i + 1 == lengths.len() { return path[i] + (path[i + 1] - path[i]) * (d / length).clamp(0.0, 1.0); }
            d -= length;
        }
        path[0]
    };
    // An eased sweep, its streak 5 units long, fading at both ends of the brass.
    let s = phase / 0.8;
    let head = (s * s * (3.0 - 2.0 * s)) * (total + 5.0);
    let steps = 10;
    for i in 0..steps {
        let (d0, d1) = (head - 5.0 + 5.0 * i as f32 / steps as f32, head - 5.0 + 5.0 * (i + 1) as f32 / steps as f32);
        if d1 < 0.0 || d0 > total { continue; }
        let middle = (i as f32 + 0.5) / steps as f32;
        let strength = (1.0 - (middle * 2.0 - 1.0).abs()).powf(0.7) * (s * 4.0).min(1.0) * ((1.0 - s) * 4.0).min(1.0);
        let (a, b) = (at + at_length(d0.max(0.0)) * scale, at + at_length(d1.min(total)) * scale);
        p.line_segment([a, b], Stroke::new(1.1 * scale, Color32::from_rgba_unmultiplied(255, 246, 214, (200.0 * strength) as u8)));
    }
}

/// The working cog, turning, and the nano-lathe's green running round its hub the other way.
fn busy(p: &Painter, centre: Pos2, scale: f32, time: f32) {
    picture(p, centre, scale, &GEAR, time * std::f32::consts::TAU / (PERIOD / 4.0));
    let head = -time * std::f32::consts::TAU / (PERIOD / 6.0);
    for i in 0..10 {
        let a0 = head + i as f32 * 0.16;
        let a1 = a0 + 0.17;
        let alpha = (1.0 - i as f32 / 10.0) * 0.9;
        p.line_segment([centre + Vec2::angled(a0) * 6.4 * scale, centre + Vec2::angled(a1) * 6.4 * scale],
            Stroke::new(1.5 * scale, NANO.gamma_multiply(alpha)));
    }
}

/// The arrow, its lamp breathing and a light running along its brass.
fn arrow(p: &Painter, at: Pos2, scale: f32, time: f32) {
    picture(p, at, scale, &ARROW, 0.0);
    lamp(p, at + ARROW_LAMP * scale, scale, time);
    glint(p, at, scale, &ARROW_GLINT, time);
}

fn paint(p: &Painter, at: Pos2, icon: CursorIcon, scale: f32, contrast: bool, time: f32) {
    use CursorIcon::*;
    if icon == None { return; }
    if contrast {
        p.circle_stroke(at, 17.0 * scale, Stroke::new(4.0 * scale, Color32::BLACK));
        p.circle_stroke(at, 17.0 * scale, Stroke::new(2.0 * scale, Color32::WHITE));
    }
    let quarter = std::f32::consts::FRAC_PI_4;
    match icon {
        Text => picture(p, at, scale, &IBEAM, 0.0),
        VerticalText => picture(p, at, scale, &IBEAM, 2.0 * quarter),
        ResizeHorizontal | ResizeEast | ResizeWest | ResizeColumn => picture(p, at, scale, &RESIZE, 0.0),
        ResizeVertical | ResizeNorth | ResizeSouth | ResizeRow => picture(p, at, scale, &RESIZE, 2.0 * quarter),
        ResizeNeSw | ResizeNorthEast | ResizeSouthWest => picture(p, at, scale, &RESIZE, -quarter),
        ResizeNwSe | ResizeNorthWest | ResizeSouthEast => picture(p, at, scale, &RESIZE, quarter),
        Move | AllScroll => picture(p, at, scale, &MOVE, 0.0),
        PointingHand => {
            picture(p, at, scale, &HAND_POINT, 0.0);
            let s = scale * HAND_POINT.scale;
            lamp(p, at + HAND_LAMP * s, s, time);
            glint(p, at, s, &HAND_GLINT, time);
        }
        Grab => picture(p, at, scale, &HAND_OPEN, 0.0),
        Grabbing => picture(p, at, scale, &HAND_CLOSED, 0.0),
        NotAllowed | NoDrop => picture(p, at, scale, &FORBIDDEN, 0.0),
        Crosshair | Cell => picture(p, at, scale, &CROSSHAIR, 0.0),
        Wait => busy(p, at, scale, time),
        Progress => {
            arrow(p, at, scale, time);
            busy(p, at + BESIDE * scale, scale * 0.62, time);
        }
        ZoomIn => picture(p, at, scale, &ZOOM_IN, 0.0),
        ZoomOut => picture(p, at, scale, &ZOOM_OUT, 0.0),
        Copy | Alias | ContextMenu | Help => {
            arrow(p, at, scale, time);
            let badge = match icon { Copy => &BADGE_COPY, Alias => &BADGE_ALIAS, ContextMenu => &BADGE_MENU, _ => &BADGE_HELP };
            picture(p, at + BESIDE * scale, scale, badge, 0.0);
        }
        Default => arrow(p, at, scale, time),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(time: f64, position: Option<Pos2>, focused: bool, minimized: bool) -> egui::RawInput {
        let mut input = egui::RawInput { time: Some(time), focused, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500., 300.))),
            events: position.map(|at| vec![egui::Event::PointerMoved(at)]).unwrap_or_else(|| vec![egui::Event::PointerGone]), ..Default::default() };
        input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().minimized = Some(minimized);
        input
    }

    fn frame(ctx: &Context, config: &ta_config::Config, time: f64, icon: CursorIcon) -> egui::FullOutput {
        let mut raw = input(time, Some(pos2(60., 60.)), true, false);
        if ctx.input(|i| i.pointer.hover_pos()) == Some(pos2(60., 60.)) { raw.events.clear(); }
        let mut output = ctx.run_ui(raw, |ui| { ui.ctx().set_cursor_icon(icon); show(ui.ctx(), config); });
        output.textures_delta.clear();
        output
    }

    #[test]
    fn custom_cursor_only_replaces_the_native_pointer_inside_the_focused_window() {
        let ctx = Context::default();
        let mut config = ta_config::Config::default();
        for (enabled, focused, minimized, position, replaced) in [
            (true, true, false, Some(pos2(30., 30.)), true), (false, true, false, Some(pos2(30., 30.)), false),
            (true, false, false, Some(pos2(30., 30.)), false), (true, true, true, Some(pos2(30., 30.)), false),
            (true, true, false, Some(pos2(700., 30.)), false), (true, true, false, Option::None, false),
        ] {
            config.launcher.themed_cursor = enabled;
            let mut result = ctx.run_ui(input(0., position, focused, minimized), |ui| { ui.ctx().set_cursor_icon(CursorIcon::Text); show(ui.ctx(), &config); });
            result.textures_delta.clear();
            assert_eq!(result.platform_output.cursor_icon == CursorIcon::None, replaced);
        }
        let mut raw = input(0., Some(pos2(30., 30.)), true, false);
        raw.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().focused = Some(false);
        let mut result = ctx.run_ui(raw, |ui| { ui.ctx().set_cursor_icon(CursorIcon::Text); show(ui.ctx(), &config); });
        result.textures_delta.clear();
        assert_eq!(result.platform_output.cursor_icon, CursorIcon::Text);
        assert_eq!(frame(&ctx, &config, 0., CursorIcon::None).platform_output.cursor_icon, CursorIcon::None);
    }

    #[test]
    fn animated_pointer_freezes_without_scheduling_frames_when_motion_is_reduced() {
        let ctx = Context::default();
        let mut config = ta_config::Config::default();
        let shapes = |out: egui::FullOutput| out.shapes.into_iter().map(|s| s.shape).filter(|s| *s != Shape::Noop).collect::<Vec<_>>();
        for n in 0..4 { frame(&ctx, &config, n as f64 * 0.05, CursorIcon::Wait); }
        let first = frame(&ctx, &config, 0.25, CursorIcon::Wait);
        let later = frame(&ctx, &config, 0.30, CursorIcon::Wait);
        assert!(later.viewport_output[&egui::ViewportId::ROOT].repaint_delay <= Duration::from_millis(33));
        assert_ne!(shapes(first), shapes(later));
        config.gameplay.reduced_motion = true;
        let still = frame(&ctx, &config, 0.35, CursorIcon::Wait);
        let later = frame(&ctx, &config, 9.0, CursorIcon::Wait);
        assert!(later.viewport_output[&egui::ViewportId::ROOT].repaint_delay > Duration::from_secs(1));
        assert_eq!(shapes(still), shapes(later), "Reduced motion must hold the indicator at its current angle");
        config.gameplay.reduced_motion = false;
        let clock_id = Id::new("reborn_pointer_clock").with(ctx.viewport_id());
        let old_time = ctx.data(|d| d.get_temp::<Motion>(clock_id).unwrap().elapsed);
        frame(&ctx, &config, 90., CursorIcon::Wait);
        let new_time = ctx.data(|d| d.get_temp::<Motion>(clock_id).unwrap().elapsed);
        assert!((new_time - old_time) <= 0.10001, "Refocusing after a long pause must not jump the animation");
    }

    #[test]
    fn cursor_and_tooltip_clearance_keep_their_display_size_when_ui_zoom_changes() {
        let config = ta_config::Config::default();
        let ctx = Context::default();
        let original = scale(&ctx, &config);
        for zoom in [1.0, 1.5, 2.0] {
            ctx.set_zoom_factor(zoom);
            frame(&ctx, &config, 0., CursorIcon::Default);
            assert!((scale(&ctx, &config) * ctx.zoom_factor() - original).abs() < 0.001);
            assert!(((tooltip_clearance(&ctx, &config) - 4.) * ctx.zoom_factor() - 36.).abs() < 0.001);
        }
    }

    #[test]
    fn every_context_has_bounded_geometry_and_distinct_action_symbols() {
        let ctx = Context::default();
        let config = ta_config::Config::default();
        let render = |icon| frame(&ctx, &config, 0., icon).shapes.into_iter().map(|s| s.shape).filter(|s| *s != Shape::Noop).collect::<Vec<_>>();
        for icon in CursorIcon::ALL.into_iter().filter(|icon| *icon != CursorIcon::None) {
            let shapes = render(icon);
            assert!(!shapes.is_empty(), "{icon:?}");
            let bounds = shapes.iter().fold(Rect::NOTHING, |bounds, shape| bounds.union(shape.visual_bounding_rect()));
            assert!(bounds.is_finite() && Rect::from_min_max(pos2(25., 25.), pos2(96., 96.)).contains_rect(bounds), "{icon:?}: {bounds:?}");
        }
        for pair in [[CursorIcon::Default, CursorIcon::PointingHand], [CursorIcon::Grab, CursorIcon::Grabbing], [CursorIcon::Copy, CursorIcon::Alias], [CursorIcon::Help, CursorIcon::ContextMenu], [CursorIcon::Wait, CursorIcon::Progress]] {
            assert_ne!(render(pair[0]), render(pair[1]), "{pair:?} must explain different actions");
        }
    }

    /// Each picture is the size its sprite says (units at PX pixels), its hotspot inside it.
    #[test]
    fn every_picture_decodes_with_its_hotspot_inside() {
        for sprite in [&ARROW, &HAND_POINT, &HAND_OPEN, &HAND_CLOSED, &IBEAM, &GEAR, &RESIZE, &MOVE, &FORBIDDEN, &CROSSHAIR,
            &ZOOM_IN, &ZOOM_OUT, &BADGE_COPY, &BADGE_ALIAS, &BADGE_MENU, &BADGE_HELP] {
            let picture = image::load_from_memory(sprite.png).unwrap_or_else(|e| panic!("{}: {e}", sprite.name));
            let size = vec2(picture.width() as f32, picture.height() as f32) / PX;
            assert!(size.x.fract() == 0.0 && size.y.fract() == 0.0, "{}: {size:?} units", sprite.name);
            assert!(Rect::from_min_size(Pos2::ZERO, size).contains(sprite.hotspot.to_pos2()), "{}: hotspot {:?} outside {size:?}", sprite.name, sprite.hotspot);
        }
    }
}
