//! Orbit camera of the 3D view: drag to orbit, right or middle drag to pan,
//! scroll to zoom. Gives [`llevant::Camera`]s for any image size.

use llevant::{Aabb, Camera, Cutaway, Vec3};

use crate::state::CameraState;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub state: CameraState,
    /// Radius of the model's bounding sphere (cm): how far rays reach.
    pub radius: f64,
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// `v` turned by `angle` radians about the unit `axis` (Rodrigues).
fn rotate(v: Vec3, axis: Vec3, angle: f64) -> Vec3 {
    let (s, c) = angle.sin_cos();
    let d = v[0] * axis[0] + v[1] * axis[1] + v[2] * axis[2];
    let x = [
        axis[1] * v[2] - axis[2] * v[1],
        axis[2] * v[0] - axis[0] * v[2],
        axis[0] * v[1] - axis[1] * v[0],
    ];
    [0, 1, 2].map(|i| v[i] * c + x[i] * s + axis[i] * d * (1.0 - c))
}

fn scale(a: Vec3, s: f64) -> Vec3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

impl Orbit {
    /// Framing all of `region`, seen from `yaw`, `pitch` (degrees).
    pub fn fit(region: &Aabb, yaw_deg: f64, pitch_deg: f64) -> Orbit {
        let c = region.center();
        let d = [0, 1, 2].map(|i| region.hi[i] - region.lo[i]);
        let radius = (0.5 * (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()).max(1e-3);
        let fov_deg: f64 = 30.0;
        let distance = 1.1 * radius / (0.5 * fov_deg).to_radians().sin();
        Orbit {
            state: CameraState {
                target: c,
                distance,
                yaw_deg,
                pitch_deg,
                fov_deg,
                ortho: false,
            },
            radius,
        }
    }

    /// Unit vector from the eye towards the target.
    pub fn forward(&self) -> Vec3 {
        let (y, p) = (
            self.state.yaw_deg.to_radians(),
            self.state.pitch_deg.to_radians(),
        );
        // The eye sits at yaw/pitch from the target; we look back at it.
        [-p.cos() * y.cos(), -p.cos() * y.sin(), -p.sin()]
    }

    pub fn eye(&self) -> Vec3 {
        add(
            self.state.target,
            scale(self.forward(), -self.state.distance),
        )
    }

    /// Right and up unit vectors of the image.
    fn right_up(&self) -> (Vec3, Vec3) {
        let y = self.state.yaw_deg.to_radians();
        let p = self.state.pitch_deg.to_radians();
        let right = [-y.sin(), y.cos(), 0.0];
        let up = [-p.sin() * y.cos(), -p.sin() * y.sin(), p.cos()];
        (right, up)
    }

    /// The camera for an image of `width × height` pixels, with `cut` cut
    /// away (`None`: nothing).
    pub fn camera(&self, width: usize, height: usize, cut: impl Into<Cutaway>) -> Camera {
        let (_, up) = self.right_up();
        let s = &self.state;
        let ortho = s
            .ortho
            .then(|| 2.0 * s.distance * (0.5 * s.fov_deg).to_radians().tan());
        Camera {
            eye: self.eye(),
            target: self.state.target,
            up,
            fov_deg: self.state.fov_deg,
            width: width.max(1),
            height: height.max(1),
            reach: self.state.distance + 2.0 * self.radius,
            cut: cut.into(),
            ortho,
        }
    }

    /// Turn by a drag of `(dx, dy)` pixels.
    pub fn orbit(&mut self, dx: f64, dy: f64) {
        self.state.yaw_deg = (self.state.yaw_deg - 0.3 * dx).rem_euclid(360.0);
        self.state.pitch_deg = (self.state.pitch_deg + 0.3 * dy).clamp(-89.0, 89.0);
    }

    /// Move the target with a drag of `(dx, dy)` pixels in an image
    /// `height` pixels tall, so that the point under the cursor at the
    /// target's depth follows it.
    pub fn pan(&mut self, dx: f64, dy: f64, height: f64) {
        let (right, up) = self.right_up();
        let per_pixel = 2.0 * self.state.distance * (0.5 * self.state.fov_deg).to_radians().tan()
            / height.max(1.0);
        let shift = add(scale(right, -dx * per_pixel), scale(up, dy * per_pixel));
        self.state.target = add(self.state.target, shift);
    }

    /// Zoom by a scroll of `amount` (positive: closer).
    pub fn zoom(&mut self, amount: f64) {
        let f = (-amount * 0.002).exp();
        self.state.distance = (self.state.distance * f).clamp(1e-3, 1e3 * self.radius.max(1.0));
    }

    /// Move eye and target together by `(right, up, forward)` cm along the
    /// image's axes (forward: the view direction).
    pub fn fly(&mut self, right: f64, up: f64, forward: f64) {
        let (r, u) = self.right_up();
        let f = self.forward();
        let step = add(add(scale(r, right), scale(u, up)), scale(f, forward));
        self.state.target = add(self.state.target, step);
    }

    /// Turn the view about the eye by a drag of `(dx, dy)` pixels: right
    /// turns right, up looks up. The target stays `distance` ahead.
    pub fn look(&mut self, dx: f64, dy: f64) {
        let eye = self.eye();
        self.state.yaw_deg = (self.state.yaw_deg - 0.15 * dx).rem_euclid(360.0);
        self.state.pitch_deg = (self.state.pitch_deg + 0.15 * dy).clamp(-89.0, 89.0);
        self.state.target = add(eye, scale(self.forward(), self.state.distance));
    }

    /// Move the eye towards `p` (a point seen through the view) by the
    /// fraction `1 - exp(-0.002 · amount)` of the way, away for a negative
    /// amount, keeping the view direction. Never reaches `p`. The target
    /// moves to `p`'s depth, so that turning about it afterwards
    /// ([`Orbit::orbit_about`] without a pivot) turns about what was
    /// approached.
    pub fn towards(&mut self, p: Vec3, amount: f64) {
        let eye = self.eye();
        let f = (1.0 - (-0.002 * amount).exp()).min(0.9);
        let to = [0, 1, 2].map(|i| p[i] - eye[i]);
        let eye = add(eye, scale(to, f));
        let fw = self.forward();
        let depth = (0..3).map(|i| (p[i] - eye[i]) * fw[i]).sum::<f64>();
        self.state.distance = depth.max(1e-3);
        self.state.target = add(eye, scale(fw, self.state.distance));
    }

    /// Orbit about `pivot` by a drag of `(dx, dy)` pixels, without
    /// re-centring the view on it: eye and view direction turn together
    /// about the vertical through `pivot` and about the image's horizontal
    /// axis through it.
    pub fn orbit_about(&mut self, pivot: Vec3, dx: f64, dy: f64) {
        let (yaw, pitch) = ((-0.3 * dx).to_radians(), (0.3 * dy).to_radians());
        let new_pitch = self.state.pitch_deg + pitch.to_degrees();
        let pitch = if new_pitch.abs() < 89.0 { pitch } else { 0.0 };
        let (right, _) = self.right_up();
        let turn = |v: Vec3| rotate(rotate(v, right, -pitch), [0.0, 0.0, 1.0], yaw);
        let eye = self.eye();
        let rel = [0, 1, 2].map(|i| eye[i] - pivot[i]);
        let eye = add(pivot, turn(rel));
        let f = turn(self.forward());
        // Back to yaw and pitch: forward = -(cos p cos y, cos p sin y, sin p).
        self.state.pitch_deg = (-f[2]).clamp(-1.0, 1.0).asin().to_degrees();
        self.state.yaw_deg = (-f[1]).atan2(-f[0]).to_degrees().rem_euclid(360.0);
        self.state.target = add(eye, scale(self.forward(), self.state.distance));
    }

    /// Orbit about `p` from now on, with the eye where it is: the view does
    /// not move, but turning and zooming then go about and towards `p`.
    pub fn pivot_at(&mut self, p: Vec3) {
        let eye = self.eye();
        let d = [0, 1, 2].map(|i| eye[i] - p[i]);
        let dist = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if !(dist > 1e-6) {
            return;
        }
        self.state.target = p;
        self.state.distance = dist;
        self.state.pitch_deg = (d[2] / dist).clamp(-1.0, 1.0).asin().to_degrees();
        self.state.yaw_deg = d[1].atan2(d[0]).to_degrees().rem_euclid(360.0);
    }
}

/// Box around what `vis` shows inside `region`, measured: `n × n` parallel
/// rays along each of ±x, ±y, ±z across `region` (CPU, exact first hits),
/// then again across the box found. Models often sit in a huge world cell
/// (a 200 m sphere of air, say) that makes their extent useless for
/// framing; their visible parts do not. Sampled, so a part thinner than
/// the spacing at the very edge can be missed. `None` if nothing is hit.
pub fn visible_extent(
    m: &llevant::Model,
    vis: &llevant::Visibility,
    region: &Aabb,
    n: usize,
) -> Option<Aabb> {
    let first = visible_in(m, vis, region, n)?;
    Some(visible_in(m, vis, &region_pad(&first), n).unwrap_or(first))
}

/// `b` grown by 5 % of its size on each side (a zero-size side by 1 cm).
fn region_pad(b: &Aabb) -> Aabb {
    let mut out = *b;
    for i in 0..3 {
        let d = (0.05 * (b.hi[i] - b.lo[i])).max(1.0);
        out.lo[i] -= d;
        out.hi[i] += d;
    }
    out
}

fn visible_in(
    m: &llevant::Model,
    vis: &llevant::Visibility,
    region: &Aabb,
    n: usize,
) -> Option<Aabb> {
    if !region.is_finite() || region.is_empty() || n == 0 {
        return None;
    }
    let mut rays = Vec::with_capacity(6 * n * n);
    for k in 0..3 {
        let (a, b) = ((k + 1) % 3, (k + 2) % 3);
        let len = region.hi[k] - region.lo[k];
        for i in 0..n {
            for j in 0..n {
                let mut o = [0.0; 3];
                o[a] = region.lo[a] + (region.hi[a] - region.lo[a]) * (i as f64 + 0.5) / n as f64;
                o[b] = region.lo[b] + (region.hi[b] - region.lo[b]) * (j as f64 + 0.5) / n as f64;
                for up in [true, false] {
                    let mut origin = o;
                    let mut dir = [0.0; 3];
                    origin[k] = if up { region.lo[k] } else { region.hi[k] };
                    dir[k] = if up { 1.0 } else { -1.0 };
                    rays.push(llevant::Line {
                        origin,
                        dir,
                        t_min: 0.0,
                        t_max: len,
                    });
                }
            }
        }
    }
    let h = m.first_hits(&rays, vis, None).ok()?;
    let mut out = Aabb::EMPTY;
    for (r, (&id, &t)) in rays.iter().zip(h.ids.iter().zip(&h.t)) {
        if id == llevant::NO_HIT {
            continue;
        }
        let p = [0, 1, 2].map(|i| r.origin[i] + t * r.dir[i]);
        out = out.union(&Aabb { lo: p, hi: p });
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9)
    }

    #[test]
    fn basis_is_orthonormal_and_agrees_with_llevant() {
        let region = Aabb {
            lo: [-10.0, -20.0, -5.0],
            hi: [30.0, 20.0, 5.0],
        };
        let mut o = Orbit::fit(&region, 30.0, 20.0);
        o.orbit(17.0, -33.0);
        let cam = o.camera(200, 100, None);
        let [f, r, u] = cam.basis();
        let (right, up) = o.right_up();
        assert!(close(f, o.forward()));
        assert!(close(r, right));
        assert!(close(u, up));
        let eye = o.eye();
        let back = add(eye, scale(f, o.state.distance));
        assert!(close(back, o.state.target));
    }

    #[test]
    fn pivot_keeps_the_eye_and_fly_moves_both() {
        let region = Aabb {
            lo: [-10.0; 3],
            hi: [10.0; 3],
        };
        let mut o = Orbit::fit(&region, 40.0, 15.0);
        let eye = o.eye();
        let p = [3.0, -2.0, 1.0];
        o.pivot_at(p);
        assert!(close(o.eye(), eye));
        assert!(close(o.state.target, p));
        let f = o.forward();
        o.fly(0.0, 0.0, 5.0);
        assert!(close(o.eye(), add(eye, scale(f, 5.0))));
        assert!(close(o.state.target, add(p, scale(f, 5.0))));
    }

    #[test]
    fn first_person_moves() {
        let region = Aabb {
            lo: [-10.0; 3],
            hi: [10.0; 3],
        };
        // Looking keeps the eye.
        let mut o = Orbit::fit(&region, 40.0, 15.0);
        let eye = o.eye();
        o.look(30.0, -12.0);
        assert!(close(o.eye(), eye));
        // Dragging right turns right: the view direction moves towards the
        // old right vector.
        let mut o = Orbit::fit(&region, 40.0, 15.0);
        let (right, _) = o.right_up();
        let f0 = o.forward();
        o.look(20.0, 0.0);
        let f1 = o.forward();
        assert!((0..3).map(|i| (f1[i] - f0[i]) * right[i]).sum::<f64>() > 0.0);
        // Moving towards a point keeps the view direction and gets closer.
        let mut o = Orbit::fit(&region, 40.0, 15.0);
        let p = [1.0, 2.0, 0.5];
        let d0 = (0..3).map(|i| (o.eye()[i] - p[i]).powi(2)).sum::<f64>();
        o.towards(p, 300.0);
        assert!(close(o.forward(), f0));
        let d1 = (0..3).map(|i| (o.eye()[i] - p[i]).powi(2)).sum::<f64>();
        assert!(d1 < d0);
        // Orbiting about the target is the orbit camera's turn.
        let mut a = Orbit::fit(&region, 40.0, 15.0);
        let mut b = a;
        a.orbit(17.0, -9.0);
        b.orbit_about(b.state.target, 17.0, -9.0);
        assert!(close(a.eye(), b.eye()));
        assert!(close(a.forward(), b.forward()));
    }

    #[test]
    fn pan_and_zoom() {
        let region = Aabb {
            lo: [-1.0; 3],
            hi: [1.0; 3],
        };
        let mut o = Orbit::fit(&region, 0.0, 0.0);
        let d = o.state.distance;
        o.zoom(500.0);
        assert!(o.state.distance < d);
        let t = o.state.target;
        o.pan(10.0, 0.0, 100.0);
        assert!(o.state.target != t);
        // Looking along -x from +x: right is +y, so dragging right moves the
        // target to -y.
        assert!(o.state.target[1] < t[1]);
    }
}

/// How long the camera must rest for where it is to count as a place
/// looked at.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(600);
/// Places kept to go back to.
const PLACES: usize = 50;

/// The places the 3D camera rested at, to go back and forward through as
/// in a browser (Alt+← and Alt+→). Moving the camera is not an edit (undo
/// leaves it alone); this is its own history.
#[derive(Clone, Debug, Default)]
pub struct CameraHistory {
    /// Where the camera rested last.
    settled: Option<CameraState>,
    /// Where it is while moving (or just stopped), and since when.
    moving: Option<(CameraState, std::time::Instant)>,
    back: Vec<CameraState>,
    forward: Vec<CameraState>,
}

impl CameraHistory {
    /// The camera is at `now` at time `at` (each frame).
    pub fn observe(&mut self, now: CameraState, at: std::time::Instant) {
        let Some(settled) = self.settled else {
            self.settled = Some(now);
            return;
        };
        if settled == now {
            self.moving = None;
            return;
        }
        match self.moving {
            Some((c, since)) if c == now => {
                if at.duration_since(since) >= SETTLE {
                    self.settle(now);
                }
            }
            _ => self.moving = Some((now, at)),
        }
    }

    /// `now` is a place looked at: the one before goes back.
    fn settle(&mut self, now: CameraState) {
        if let Some(s) = self.settled {
            self.back.push(s);
            if self.back.len() > PLACES {
                self.back.remove(0);
            }
        }
        self.settled = Some(now);
        self.moving = None;
        self.forward.clear();
    }

    /// Where the camera may still be on its way to counts as reached.
    fn reach(&mut self) {
        if let Some((c, _)) = self.moving {
            if Some(c) != self.settled {
                self.settle(c);
            }
        }
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty() || self.moving.is_some_and(|(c, _)| Some(c) != self.settled)
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// The camera moved and has not rested long enough yet: a frame later
    /// is needed to record where it stops.
    pub fn settling(&self) -> bool {
        self.moving.is_some()
    }

    /// The place before, if any (the camera goes there).
    pub fn back(&mut self) -> Option<CameraState> {
        self.reach();
        let prev = self.back.pop()?;
        self.forward.extend(self.settled);
        self.settled = Some(prev);
        Some(prev)
    }

    /// The place after one gone back from, if any.
    pub fn forward(&mut self) -> Option<CameraState> {
        let next = self.forward.pop()?;
        self.back.extend(self.settled);
        self.settled = Some(next);
        self.moving = None;
        Some(next)
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn at(yaw: f64) -> CameraState {
        CameraState {
            target: [0.0; 3],
            distance: 10.0,
            yaw_deg: yaw,
            pitch_deg: 0.0,
            fov_deg: 40.0,
            ortho: false,
        }
    }

    #[test]
    fn places_rested_at_go_back_and_forward() {
        let t0 = Instant::now();
        let s = |k: u64| t0 + Duration::from_millis(k);
        let mut h = CameraHistory::default();
        h.observe(at(0.0), s(0));
        // Passing through 10° without resting: not a place.
        h.observe(at(10.0), s(100));
        h.observe(at(20.0), s(200));
        h.observe(at(20.0), s(900));
        h.observe(at(30.0), s(1000));
        h.observe(at(30.0), s(1700));
        assert_eq!(h.back(), Some(at(20.0)));
        assert_eq!(h.back(), Some(at(0.0)));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some(at(20.0)));
        assert_eq!(h.forward(), Some(at(30.0)));
        assert!(!h.can_go_forward());
    }

    #[test]
    fn going_back_right_after_moving_keeps_the_place_reached() {
        let t0 = Instant::now();
        let mut h = CameraHistory::default();
        h.observe(at(0.0), t0);
        h.observe(at(45.0), t0 + Duration::from_millis(50));
        assert!(h.can_go_back());
        assert_eq!(h.back(), Some(at(0.0)));
        assert_eq!(h.forward(), Some(at(45.0)));
    }
}
