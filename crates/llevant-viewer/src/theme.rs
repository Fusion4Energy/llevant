//! The viewer's look: fonts, a dark and a light palette (neutral grey with
//! a blue accent; yellow stays for the selection in the views), the colours
//! of overlays, and the house widgets panels and
//! overlays are built from, so that every panel looks alike and a restyle
//! happens here.
//!
//! Fonts (bundled in `assets/`, see the licence files there): IBM Plex Sans
//! for the interface, IBM Plex Mono for numbers and data (coordinates,
//! labels, the status bar), and the Lucide icon font ([`crate::icons`]).

use std::sync::Arc;

use egui::{Color32, FontFamily, FontId, Pos2, Stroke, TextStyle, Theme};

/// The colours of one theme.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Behind everything: text fields, the window's edges.
    pub base: Color32,
    pub panel: Color32,
    /// Buttons and other widgets at rest.
    pub raised: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    /// The blue: what is active or chosen in the interface.
    pub accent: Color32,
    /// Text on the accent.
    pub on_accent: Color32,
    /// Icons of section headers (the accent where it reads well).
    pub icon: Color32,
    /// Links and information.
    pub info: Color32,
    pub warning: Color32,
    pub error: Color32,
    /// Behind the views where nothing is hit (flat colours; realistic ones
    /// have their own sky).
    pub view: [u8; 3],
}

pub const DARK: Palette = Palette {
    base: Color32::from_rgb(24, 25, 27),
    panel: Color32::from_rgb(35, 36, 40),
    raised: Color32::from_rgb(46, 48, 53),
    hover: Color32::from_rgb(57, 59, 65),
    pressed: Color32::from_rgb(68, 71, 78),
    border: Color32::from_rgb(58, 60, 66),
    text: Color32::from_rgb(228, 229, 232),
    text_weak: Color32::from_rgb(154, 156, 163),
    accent: Color32::from_rgb(76, 155, 232),
    on_accent: Color32::from_rgb(8, 18, 30),
    icon: Color32::from_rgb(110, 168, 232),
    info: Color32::from_rgb(110, 168, 255),
    warning: Color32::from_rgb(255, 170, 60),
    error: Color32::from_rgb(255, 105, 105),
    view: [38, 40, 44],
};

pub const LIGHT: Palette = Palette {
    base: Color32::from_rgb(255, 255, 255),
    panel: Color32::from_rgb(244, 244, 246),
    raised: Color32::from_rgb(230, 231, 234),
    hover: Color32::from_rgb(218, 219, 224),
    pressed: Color32::from_rgb(205, 207, 213),
    border: Color32::from_rgb(210, 212, 217),
    text: Color32::from_rgb(29, 30, 33),
    text_weak: Color32::from_rgb(104, 107, 115),
    accent: Color32::from_rgb(47, 123, 214),
    on_accent: Color32::from_rgb(255, 255, 255),
    icon: Color32::from_rgb(47, 123, 214),
    info: Color32::from_rgb(31, 99, 184),
    warning: Color32::from_rgb(190, 100, 0),
    error: Color32::from_rgb(200, 30, 30),
    view: [226, 228, 232],
};

/// The palette of the theme shown.
pub fn palette(ctx: &egui::Context) -> &'static Palette {
    match ctx.theme() {
        Theme::Dark => &DARK,
        Theme::Light => &LIGHT,
    }
}

/// The selection and the picked point, in the views (and where the
/// interface names them).
pub const SELECTION: Color32 = Color32::from_rgb(255, 199, 44);
/// The point Alt+drag orbits about.
pub const PIVOT: Color32 = Color32::from_rgb(90, 165, 255);
/// Pins and their banners.
pub const PIN: Color32 = Color32::from_rgb(32, 201, 170);
/// Surfaces marked on the slice (a cell's, while its geometry is edited):
/// the blue surface numbers have in cards.
pub const MARKED_SURFACE: Color32 = Color32::from_rgb(96, 160, 240);
/// The cell as edited, drawn on the slice before it is applied.
pub const EDITED_CELL: Color32 = Color32::from_rgb(240, 150, 60);
/// FMESH meshes (in Tally), but the one selected.
pub const MESH: Color32 = Color32::from_rgb(236, 96, 210);
/// Gaps named or marked over the views (their pattern in images is black
/// and grey, `llevant_render::colours::marked`).
pub const GAP_MARK: Color32 = Color32::from_rgb(120, 160, 255);
/// Overlaps and other problems named or marked over the views.
pub const PROBLEM_MARK: Color32 = Color32::from_rgb(255, 80, 200);
/// Under overlay lines, so that they show on cells of any colour.
const HALO: Color32 = Color32::from_rgba_premultiplied(5, 12, 24, 200);
/// Overlays behind the slice plane, or otherwise not quite there.
pub const FADED: f32 = 0.35;

/// The family of the semibold interface font (headings, strong text).
pub fn strong_family() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Text in the data font (coordinates, numbers, labels of cells).
pub fn data(text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).monospace()
}

/// Fonts, text sizes, spacing and both palettes, for the whole window.
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    for (theme, p) in [(Theme::Dark, &DARK), (Theme::Light, &LIGHT)] {
        ctx.style_mut_of(theme, |s| style(s, p, theme == Theme::Dark));
    }
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8], tweak: egui::FontTweak| {
        let mut data = egui::FontData::from_static(bytes);
        data.tweak = tweak;
        fonts.font_data.insert(name.into(), Arc::new(data));
    };
    add(
        "plex-sans",
        include_bytes!("../assets/IBMPlexSans-Regular.ttf"),
        Default::default(),
    );
    add(
        "plex-sans-semibold",
        include_bytes!("../assets/IBMPlexSans-SemiBold.ttf"),
        Default::default(),
    );
    add(
        "plex-mono",
        llevant_render::legend::MONO_FONT,
        Default::default(),
    );
    // Icons a little larger than the text, centred on it.
    add(
        "lucide",
        include_bytes!("../assets/lucide.ttf"),
        egui::FontTweak {
            scale: 1.05,
            y_offset_factor: 0.08,
            ..Default::default()
        },
    );
    // Each family: its font, the icons, then egui's own fonts (symbols,
    // emoji) for anything else.
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    let family = |first: &str| {
        let mut v = vec![first.to_string(), "lucide".to_string()];
        v.extend(fallback.iter().cloned());
        v
    };
    fonts
        .families
        .insert(FontFamily::Proportional, family("plex-sans"));
    fonts
        .families
        .insert(FontFamily::Monospace, family("plex-mono"));
    fonts
        .families
        .insert(strong_family(), family("plex-sans-semibold"));
    ctx.set_fonts(fonts);
}

fn style(s: &mut egui::Style, p: &Palette, dark: bool) {
    s.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.5)),
        (TextStyle::Button, FontId::proportional(13.5)),
        (TextStyle::Heading, FontId::new(17.0, strong_family())),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();
    s.spacing.item_spacing = egui::vec2(6.0, 5.0);
    s.spacing.button_padding = egui::vec2(7.0, 3.0);
    s.spacing.slider_width = 130.0;
    s.spacing.interact_size.y = 22.0;

    let v = &mut s.visuals;
    *v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let r = egui::CornerRadius::same(4);
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.extreme_bg_color = p.base;
    v.faint_bg_color = p.raised;
    v.code_bg_color = p.raised;
    v.hyperlink_color = p.info;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.error;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, p.on_accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.slider_trailing_fill = true;
    v.window_corner_radius = egui::CornerRadius::same(6);
    v.menu_corner_radius = egui::CornerRadius::same(6);

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.weak_bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    for (wv, fill) in [
        (&mut w.inactive, p.raised),
        (&mut w.hovered, p.hover),
        (&mut w.active, p.pressed),
        (&mut w.open, p.raised),
    ] {
        wv.bg_fill = fill;
        wv.weak_bg_fill = fill;
        wv.corner_radius = r;
        wv.fg_stroke = Stroke::new(1.5, p.text);
        wv.bg_stroke = Stroke::NONE;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.6));
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.noninteractive.corner_radius = r;
}

/// A titled group of settings with its icon, open at first; its state is
/// kept by egui.
/// Give a widget (a text field, say) a name for screen readers and tests.
pub fn name(r: &egui::Response, name: &str) {
    let enabled = r.enabled();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, enabled, name));
}

pub fn section<R>(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<R> {
    let accent = palette(ui.ctx()).icon;
    let mut job = egui::text::LayoutJob::default();
    let font = FontId::new(13.5, strong_family());
    job.append(
        &format!("{icon}  "),
        0.0,
        egui::TextFormat::simple(font.clone(), accent),
    );
    job.append(
        title,
        0.0,
        egui::TextFormat::simple(font, ui.visuals().text_color()),
    );
    egui::CollapsingHeader::new(job)
        .id_salt(title)
        .default_open(true)
        .show(ui, add_contents)
        .body_returned
}

/// A button opening a popup of settings, which stays open while they are
/// changed in it (a menu closes on any click) until a click outside it or
/// Esc.
pub fn settings_button(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    content: impl FnOnce(&mut egui::Ui),
) -> egui::Response {
    let config = egui::containers::menu::MenuConfig::new()
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    egui::containers::menu::MenuButton::new(text.into())
        .config(config)
        .ui(ui, content)
        .0
}

/// A row of choices, one of them `current`; true if it changed.
pub fn choice<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    current: &mut T,
    options: &[(T, &str)],
) -> bool {
    let mut changed = false;
    // Wrapped, so that a narrow panel shows every choice.
    ui.horizontal_wrapped(|ui| {
        for &(v, name) in options {
            changed |= ui.selectable_value(current, v, name).changed();
        }
    });
    changed
}

/// A marker at `pos`: a ring with a dot, or a cross-hair ring, over a dark
/// halo.
pub fn marker(painter: &egui::Painter, pos: Pos2, colour: Color32, cross: bool) {
    let halo = Stroke::new(4.0, HALO.gamma_multiply(colour.a() as f32 / 255.0));
    let stroke = Stroke::new(1.8, colour);
    for s in [halo, stroke] {
        painter.circle_stroke(pos, 6.0, s);
        if cross {
            painter.hline(pos.x - 11.0..=pos.x - 6.0, pos.y, s);
            painter.hline(pos.x + 6.0..=pos.x + 11.0, pos.y, s);
            painter.vline(pos.x, pos.y - 11.0..=pos.y - 6.0, s);
            painter.vline(pos.x, pos.y + 6.0..=pos.y + 11.0, s);
        }
    }
    if !cross {
        painter.circle_filled(pos, 2.5, colour);
    }
}

/// A floating label for the point at `anchor`: a box up and to the right
/// (left near the right edge of `clip`, below near its top), joined to the
/// point by a leader line, with a `title` in the interface font over `lines`
/// in the data font; `colour` marks its border and left edge. The rectangle
/// it covers.
pub fn banner(
    painter: &egui::Painter,
    clip: egui::Rect,
    anchor: Pos2,
    colour: Color32,
    title: &str,
    lines: &[String],
) -> egui::Rect {
    let p = palette(painter.ctx());
    let title = painter.layout_no_wrap(
        title.to_string(),
        FontId::new(12.5, strong_family()),
        p.text,
    );
    let body: Vec<_> = lines
        .iter()
        .map(|l| painter.layout_no_wrap(l.clone(), FontId::monospace(11.5), p.text))
        .collect();
    let pad = egui::vec2(7.0, 4.0);
    let width = body
        .iter()
        .map(|g| g.size().x)
        .fold(title.size().x, f32::max);
    let height = title.size().y + body.iter().map(|g| g.size().y).sum::<f32>();
    // Room for the strip on the left.
    let size = egui::vec2(width + 2.0, height) + 2.0 * pad;
    let mut min = anchor + egui::vec2(16.0, -16.0 - size.y);
    if min.x + size.x > clip.max.x {
        min.x = anchor.x - 16.0 - size.x;
    }
    if min.y < clip.min.y {
        min.y = anchor.y + 16.0;
    }
    let rect = egui::Rect::from_min_size(min, size);
    let corner = Pos2::new(
        if rect.min.x > anchor.x {
            rect.min.x
        } else {
            rect.max.x
        },
        if rect.max.y < anchor.y {
            rect.max.y
        } else {
            rect.min.y
        },
    );
    painter.line_segment([anchor, corner], Stroke::new(3.0, HALO));
    painter.line_segment([anchor, corner], Stroke::new(1.2, colour));
    painter.rect(
        rect,
        egui::CornerRadius::same(4),
        p.panel.gamma_multiply(0.94),
        Stroke::new(1.0, colour),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
        egui::CornerRadius {
            nw: 4,
            sw: 4,
            ne: 0,
            se: 0,
        },
        colour,
    );
    let mut at = rect.min + pad + egui::vec2(2.0, 0.0);
    let th = title.size().y;
    painter.galley(at, title, p.text);
    at.y += th;
    for g in body {
        let h = g.size().y;
        painter.galley(at, g, p.text);
        at.y += h;
    }
    rect
}

/// A progress bar `width` across, `fraction` of it filled with the accent,
/// `text` centred on it. The text is drawn twice, clipped at the end of
/// the fill: on the accent in its own colour, elsewhere in the text's, so
/// that it reads at any fraction (egui's bar draws it in the accent's text
/// colour throughout, unreadable on the empty part).
pub fn progress_bar(ui: &mut egui::Ui, fraction: f32, width: f32, text: &str) -> egui::Response {
    let p = palette(ui.ctx());
    let h = ui.spacing().interact_size.y.max(18.0);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(width, h), egui::Sense::hover());
    let fraction = fraction.clamp(0.0, 1.0);
    r.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::ProgressIndicator, ui.is_enabled(), text)
    });
    if !ui.is_rect_visible(rect) {
        return r;
    }
    let painter = ui.painter_at(rect);
    let radius = h / 2.0;
    painter.rect_filled(rect, radius, p.base);
    let split = rect.left() + rect.width() * fraction;
    if fraction > 0.0 {
        let fill = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(split.max(rect.left() + h), rect.bottom()),
        );
        painter.rect_filled(fill, radius, p.accent);
    }
    let galley = painter.layout_no_wrap(
        text.to_string(),
        TextStyle::Body.resolve(ui.style()),
        Color32::PLACEHOLDER,
    );
    let at = rect.center() - galley.size() / 2.0;
    let (done, rest) = (
        egui::Rect::from_min_max(rect.min, egui::pos2(split, rect.bottom())),
        egui::Rect::from_min_max(egui::pos2(split, rect.top()), rect.max),
    );
    painter
        .with_clip_rect(rest)
        .galley(at, galley.clone(), p.text);
    painter.with_clip_rect(done).galley(at, galley, p.on_accent);
    r
}

/// A small tag of `text` at `pos` (its bottom left): `colour` on a dark
/// plate.
pub fn tag(painter: &egui::Painter, pos: Pos2, colour: Color32, text: &str) {
    let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::monospace(11.0), colour);
    let r = egui::Rect::from_min_size(pos - egui::vec2(0.0, galley.size().y), galley.size())
        .expand2(egui::vec2(3.0, 1.0));
    painter.rect_filled(r, 3.0, Color32::from_black_alpha(170));
    painter.galley(r.min + egui::vec2(3.0, 1.0), galley, colour);
}
