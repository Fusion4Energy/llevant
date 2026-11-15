//! Rigid transforms, in the form the tracer uses. They are built by migjorn
//! from `TRn` / `*TRn` cards, `TRCL` and `FILL` transforms.

use crate::vec3::{dot, Vec3};

/// A rigid map from a parent frame to a local frame: `local = rot · p + disp`.
///
/// Surfaces with a transform number and filled universes are both defined in a
/// local (MCNP "auxiliary") frame; this maps parent coordinates into it.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Affine {
    /// Rows are the local axes expressed in parent coordinates.
    pub rot: [Vec3; 3],
    pub disp: Vec3,
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        rot: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        disp: [0.0; 3],
    };

    pub fn translation(t: Vec3) -> Affine {
        Affine {
            disp: t,
            ..Affine::IDENTITY
        }
    }

    /// Map a point from the parent frame to the local frame.
    #[inline]
    pub fn apply(&self, p: Vec3) -> Vec3 {
        [
            dot(self.rot[0], p) + self.disp[0],
            dot(self.rot[1], p) + self.disp[1],
            dot(self.rot[2], p) + self.disp[2],
        ]
    }

    /// Map a direction (no translation).
    #[inline]
    pub fn apply_dir(&self, v: Vec3) -> Vec3 {
        [
            dot(self.rot[0], v),
            dot(self.rot[1], v),
            dot(self.rot[2], v),
        ]
    }

    /// `rotᵀ · v`: a local direction back in the parent frame.
    #[inline]
    pub fn apply_dir_inv(&self, v: Vec3) -> Vec3 {
        let r = &self.rot;
        [
            r[0][0] * v[0] + r[1][0] * v[1] + r[2][0] * v[2],
            r[0][1] * v[0] + r[1][1] * v[1] + r[2][1] * v[2],
            r[0][2] * v[0] + r[1][2] * v[1] + r[2][2] * v[2],
        ]
    }

    /// The inverse map, local → parent.
    pub fn inverse(&self) -> Affine {
        let r = &self.rot;
        let rt = [
            [r[0][0], r[1][0], r[2][0]],
            [r[0][1], r[1][1], r[2][1]],
            [r[0][2], r[1][2], r[2][2]],
        ];
        let d = self.apply_dir_inv(self.disp);
        Affine {
            rot: rt,
            disp: [-d[0], -d[1], -d[2]],
        }
    }

    /// `self ∘ inner`: first `inner`, then `self`.
    pub fn compose(&self, inner: &Affine) -> Affine {
        let mut rot = [[0.0; 3]; 3];
        for (i, row) in rot.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| self.rot[i][k] * inner.rot[k][j]).sum();
            }
        }
        let d = self.apply(inner.disp);
        Affine { rot, disp: d }
    }

    pub fn is_identity_rotation(&self) -> bool {
        self.rot == Affine::IDENTITY.rot
    }
}

/// migjorn builds transforms the way MCNP does and maps the auxiliary frame
/// to the main one; llevant's map goes the other way, main to local.
impl From<migjorn::Affine> for Affine {
    fn from(a: migjorn::Affine) -> Affine {
        let inv = a.inverse();
        Affine {
            rot: inv.rotation,
            disp: inv.translation,
        }
    }
}

/// The transform of `TRn` card values, for tests.
#[cfg(test)]
pub(crate) fn from_mcnp(values: &[f64], degrees: bool) -> Result<Affine, String> {
    migjorn::Affine::from_mcnp(values, degrees).map(Affine::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-12)
    }

    #[test]
    fn inverse_of_migjorn() {
        let m = migjorn::Affine::from_mcnp(
            &[
                1.0, -2.0, 3.0, 30.0, 60.0, 90.0, 120.0, 30.0, 90.0, 90.0, 90.0, 0.0,
            ],
            true,
        )
        .unwrap();
        let p = [0.3, 4.0, -7.5];
        assert!(close(Affine::from(m).apply(m.apply(p)), p));
        assert!(close(Affine::from(m).inverse().apply(p), m.apply(p)));
    }

    #[test]
    fn displacement_only() {
        let t = from_mcnp(&[1.0, 2.0, 3.0], false).unwrap();
        assert!(close(t.apply([1.0, 2.0, 3.0]), [0.0; 3]));
    }

    #[test]
    fn rotation_about_z_in_degrees() {
        // Auxiliary x' is main y: a point on main +y is on local +x.
        let t = from_mcnp(
            &[
                0.0, 0.0, 0.0, 90.0, 0.0, 90.0, 180.0, 90.0, 90.0, 90.0, 90.0, 0.0,
            ],
            true,
        )
        .unwrap();
        assert!(close(t.apply([0.0, 1.0, 0.0]), [1.0, 0.0, 0.0]));
        assert!(close(
            t.inverse().apply(t.apply([3.0, -2.0, 7.0])),
            [3.0, -2.0, 7.0]
        ));
    }

    #[test]
    fn origin_convention_m() {
        let r = [0.0, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let mut v = vec![5.0, 0.0, 0.0];
        v.extend_from_slice(&r);
        let t = from_mcnp(&v, false).unwrap();
        // Auxiliary origin at main (5,0,0).
        assert!(close(t.apply([5.0, 0.0, 0.0]), [0.0; 3]));
        v.push(-1.0);
        let t = from_mcnp(&v, false).unwrap();
        // Main origin at auxiliary (5,0,0).
        assert!(close(t.apply([0.0; 3]), [5.0, 0.0, 0.0]));
    }

    #[test]
    #[allow(clippy::approx_constant)] // MCNP inputs carry 4-digit cosines
    fn nearly_orthogonal_is_cleaned() {
        let t = from_mcnp(
            &[
                0.0, 0.0, 0.0, 0.7071, 0.7071, 0.0, -0.7071, 0.7071, 0.0, 0.0, 0.0, 1.0,
            ],
            false,
        )
        .unwrap();
        for i in 0..3 {
            assert!((dot(t.rot[i], t.rot[i]) - 1.0).abs() < 1e-14);
        }
    }
}
