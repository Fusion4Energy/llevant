//! The llevant logo (the rosette of the Fusion for Energy emblem, the
//! stars of Europe encircled in yellow): on the start screen, as the
//! window's icon, and as the viewer's spinner, the petals turning about
//! the stars that stay still.
//!
//! `assets/logo.png` is the whole rosette; `logo-stars.png` and
//! `logo-petals.png` are it cut at a radius between the ring of stars and
//! the petals, so that the two can be drawn apart. egui's wgpu renderer
//! makes no mipmaps, so each is uploaded at a few sizes, halved by box
//! filtering, and drawn from the smallest not smaller than the pixels it
//! covers (a 256 px image drawn at 14 px otherwise shimmers as it turns).

use std::f32::consts::TAU;

const LOGO: &[u8] = include_bytes!("../assets/logo.png");
const STARS: &[u8] = include_bytes!("../assets/logo-stars.png");
const PETALS: &[u8] = include_bytes!("../assets/logo-petals.png");

/// Turns per second of the spinner's petals.
const TURNS_PER_SECOND: f32 = 0.25;

/// The window's icon: the rosette.
pub fn icon() -> egui::IconData {
    let (width, height, rgba) = decode(LOGO);
    egui::IconData {
        rgba,
        width,
        height,
    }
}

/// The rosette, `size` points across, still.
pub fn logo(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let t = textures(ui.ctx());
        paint(ui, rect, &t.logo, 0.0);
    }
    r
}

/// The spinner: the rosette, `size` points across, its petals turning.
pub fn spinner(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let t = textures(ui.ctx());
        let time = ui.input(|i| i.time) as f32;
        paint(ui, rect, &t.petals, (time * TURNS_PER_SECOND).fract() * TAU);
        paint(ui, rect, &t.stars, 0.0);
        ui.ctx().request_repaint();
    }
    r
}

/// The spinner at the height of a line of text, where egui's would be.
pub fn small_spinner(ui: &mut egui::Ui) -> egui::Response {
    let size = ui.spacing().interact_size.y;
    spinner(ui, size)
}

/// One image at several sizes, the largest first.
#[derive(Clone)]
struct Levels(Vec<egui::TextureHandle>);

#[derive(Clone)]
struct Textures {
    logo: Levels,
    stars: Levels,
    petals: Levels,
}

fn textures(ctx: &egui::Context) -> Textures {
    let id = egui::Id::new("llevant-logo-textures");
    if let Some(t) = ctx.data(|d| d.get_temp::<Textures>(id)) {
        return t;
    }
    let t = Textures {
        logo: levels(ctx, "logo", LOGO),
        stars: levels(ctx, "logo-stars", STARS),
        petals: levels(ctx, "logo-petals", PETALS),
    };
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

fn paint(ui: &egui::Ui, rect: egui::Rect, levels: &Levels, angle: f32) {
    let px = rect.width() * ui.ctx().pixels_per_point();
    let tex = levels
        .0
        .iter()
        .rev()
        .find(|t| t.size()[0] as f32 >= px)
        .unwrap_or(&levels.0[0]);
    egui::Image::new(egui::load::SizedTexture::new(tex.id(), rect.size()))
        .rotate(angle, egui::Vec2::splat(0.5))
        .paint_at(ui, rect);
}

/// `png` uploaded at its size and at halves of it down to 16 px.
fn levels(ctx: &egui::Context, name: &str, png: &[u8]) -> Levels {
    let (w, h, rgba) = decode(png);
    let mut image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
    let mut out = Vec::new();
    loop {
        let side = image.size[0];
        out.push(ctx.load_texture(
            format!("{name}-{side}"),
            image.clone(),
            egui::TextureOptions::LINEAR,
        ));
        if side < 32 {
            break;
        }
        image = halve(&image);
    }
    Levels(out)
}

/// Half the size, each pixel the mean of four (premultiplied, so that
/// transparent pixels carry no colour into the edges).
fn halve(image: &egui::ColorImage) -> egui::ColorImage {
    let [w, h] = image.size;
    let (hw, hh) = (w / 2, h / 2);
    let mut pixels = Vec::with_capacity(hw * hh);
    for y in 0..hh {
        for x in 0..hw {
            let mut sum = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = image.pixels[(2 * y + dy) * w + 2 * x + dx].to_array();
                for k in 0..4 {
                    sum[k] += u32::from(p[k]);
                }
            }
            let [r, g, b, a] = sum.map(|s| ((s + 2) / 4) as u8);
            pixels.push(egui::Color32::from_rgba_premultiplied(r, g, b, a));
        }
    }
    egui::ColorImage::new([hw, hh], pixels)
}

/// Width, height and RGBA bytes of an 8-bit RGBA PNG.
fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().expect("bundled logo is a PNG");
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).expect("bundled logo decodes");
    buf.truncate(info.buffer_size());
    assert_eq!(
        info.color_type,
        png::ColorType::Rgba,
        "bundled logo is RGBA"
    );
    (info.width, info.height, buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_images_decode_square() {
        for png in [LOGO, STARS, PETALS] {
            let (w, h, rgba) = decode(png);
            assert_eq!(w, h);
            assert_eq!(rgba.len(), (w * h * 4) as usize);
        }
    }
}
