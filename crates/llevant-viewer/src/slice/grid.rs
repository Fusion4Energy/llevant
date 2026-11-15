//! A grid over the slice: lines of the plane's coordinates at round
//! steps (1, 2 or 5 times a power of ten cm, at least [`MIN_GAP`] points
//! apart), fainter ones between them where they fit, the zero lines
//! stronger, and the coordinates written along the bottom and left edges.
//! It tells where the slice is and how large what it shows is.

use crate::slice::PlaneView;

/// Smallest gap between labelled lines (points).
pub const MIN_GAP: f32 = 90.0;
/// Smallest gap between the faint lines between them (points).
const MIN_MINOR_GAP: f32 = 14.0;

/// The labelled step (cm) for a view `cm_per_point` across: the smallest
/// 1, 2 or 5 × 10ⁿ cm at least [`MIN_GAP`] points long; and the step of
/// the faint lines between (a fifth or a half of it), if they are far
/// enough apart.
pub fn steps(cm_per_point: f64) -> (f64, Option<f64>) {
    let want = MIN_GAP as f64 * cm_per_point;
    let p = 10f64.powf(want.log10().floor());
    let major = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|k| k * p)
        .find(|&s| s >= want * (1.0 - 1e-12))
        .unwrap_or(10.0 * p);
    // 2 × 10ⁿ splits in two, the others in five.
    let lead = (major / 10f64.powf(major.log10().floor())).round();
    let minor = if lead == 2.0 {
        major / 2.0
    } else {
        major / 5.0
    };
    let minor = (minor / cm_per_point >= MIN_MINOR_GAP as f64).then_some(minor);
    (major, minor)
}

/// `x` written for a grid of step `step`: as many decimals as the step
/// needs, no `-0`.
pub fn label(x: f64, step: f64) -> String {
    let decimals = (-step.log10().floor()).max(0.0) as usize;
    let s = format!("{x:.decimals$}");
    if s.trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        format!("{:.decimals$}", 0.0)
    } else {
        s
    }
}

/// Multiples of `step` from `lo` to `hi` (at most 400).
fn ticks(lo: f64, hi: f64, step: f64) -> impl Iterator<Item = f64> {
    let first = (lo / step).ceil() as i64;
    let last = (hi / step).floor() as i64;
    (first..=last.min(first + 400)).map(move |k| k as f64 * step)
}

/// Draw the grid of `plane` over `rect`, its coordinates named `names`.
pub fn draw(painter: &egui::Painter, rect: egui::Rect, plane: &PlaneView, names: [&str; 2]) {
    let painter = painter.with_clip_rect(rect);
    let cm_per_point = plane.width / rect.width() as f64;
    let (major, minor) = steps(cm_per_point);
    let (u0, u1) = (
        plane.centre[0] - 0.5 * plane.width,
        plane.centre[0] + 0.5 * plane.width,
    );
    let (v0, v1) = (
        plane.centre[1] - 0.5 * plane.height,
        plane.centre[1] + 0.5 * plane.height,
    );
    let x = |u: f64| rect.min.x + ((u - u0) / plane.width) as f32 * rect.width();
    let y = |v: f64| rect.min.y + ((v1 - v) / plane.height) as f32 * rect.height();
    // Dark lines with a light halo read over any colour.
    let dark = |a: u8, w: f32| egui::Stroke::new(w, egui::Color32::from_black_alpha(a));
    let halo = |a: u8, w: f32| egui::Stroke::new(w, egui::Color32::from_white_alpha(a));
    if let Some(m) = minor {
        for u in ticks(u0, u1, m) {
            painter.vline(x(u), rect.y_range(), dark(28, 1.0));
        }
        for v in ticks(v0, v1, m) {
            painter.hline(rect.x_range(), y(v), dark(28, 1.0));
        }
    }
    for (zero, (w, a)) in [(false, (1.0, 80)), (true, (1.5, 150))] {
        let is_zero = |t: f64| t.abs() < 0.5 * major;
        for u in ticks(u0, u1, major).filter(|&u| is_zero(u) == zero) {
            painter.vline(x(u), rect.y_range(), halo(50, w + 2.0));
            painter.vline(x(u), rect.y_range(), dark(a, w));
        }
        for v in ticks(v0, v1, major).filter(|&v| is_zero(v) == zero) {
            painter.hline(rect.x_range(), y(v), halo(50, w + 2.0));
            painter.hline(rect.x_range(), y(v), dark(a, w));
        }
    }
    // The coordinates, on pills along the bottom and left edges.
    let font = egui::FontId::monospace(10.5);
    let pill = |pos: egui::Pos2, align: egui::Align2, text: String| -> egui::Rect {
        let galley = painter.layout_no_wrap(text, font.clone(), egui::Color32::WHITE);
        let r = align
            .anchor_size(pos, galley.size())
            .expand2(egui::vec2(3.0, 1.0));
        painter.rect_filled(r, 3.0, egui::Color32::from_black_alpha(150));
        painter.galley(r.min + egui::vec2(3.0, 1.0), galley, egui::Color32::WHITE);
        r
    };
    let bottom = rect.max.y - 4.0;
    // Which coordinate is which, and the step, in the bottom-left corner;
    // the numbers along the bottom start clear of it.
    let corner = pill(
        egui::pos2(rect.min.x + 4.0, bottom),
        egui::Align2::LEFT_BOTTOM,
        format!("{} → {} ↑ · {} cm", names[0], names[1], label(major, major)),
    );
    for u in ticks(u0, u1, major) {
        let px = x(u);
        if px > corner.max.x + 24.0 && px < rect.max.x - 20.0 {
            pill(
                egui::pos2(px, bottom),
                egui::Align2::CENTER_BOTTOM,
                label(u, major),
            );
        }
    }
    for v in ticks(v0, v1, major) {
        let py = y(v);
        if py > rect.min.y + 12.0 && py < rect.max.y - 30.0 {
            pill(
                egui::pos2(rect.min.x + 4.0, py),
                egui::Align2::LEFT_CENTER,
                label(v, major),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_steps() {
        // 0.1 cm a point: 90 points are 9 cm, so 10 cm, in fifths.
        assert_eq!(steps(0.1), (10.0, Some(2.0)));
        // 1 cm a point: 90 cm, so 100 cm; fifths of 20 points.
        assert_eq!(steps(1.0), (100.0, Some(20.0)));
        // 0.15: 13.5 cm, so 20 cm in halves.
        assert_eq!(steps(0.15), (20.0, Some(10.0)));
        // 0.4: 36 cm, so 50 cm; fifths are 25 points.
        assert_eq!(steps(0.4), (50.0, Some(10.0)));
        // Faint lines only where they are far enough apart.
        let (major, minor) = steps(0.5);
        assert_eq!(major, 50.0);
        assert_eq!(minor, Some(10.0));
        let (_, minor) = steps(0.9);
        assert_eq!(minor, Some(20.0));
        assert_eq!(steps(1e-4).0, 0.01);
    }

    #[test]
    fn labels_fit_the_step() {
        assert_eq!(label(20.0, 10.0), "20");
        assert_eq!(label(-0.0000001, 10.0), "0");
        assert_eq!(label(0.2, 0.1), "0.2");
        assert_eq!(label(0.05, 0.01), "0.05");
        assert_eq!(label(-150.0, 50.0), "-150");
    }
}
