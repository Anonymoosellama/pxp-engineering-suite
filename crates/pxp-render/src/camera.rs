// SPDX-License-Identifier: LGPL-2.1-or-later
//! The view camera: orientation, target, zoom, standard and named views, and
//! the projection matrices the GPU uses. Pure maths, no GPU, fully tested.
//!
//! ## Conventions
//!
//! - Model space is right-handed, Z up, millimetres, as everywhere in PXP.
//! - The camera looks along `-forward` from an eye placed `distance` from
//!   `target`; `orientation` is the rotation taking camera axes (right = +x,
//!   up = +y, toward the viewer = +z) to model axes. The view direction
//!   seen by the user (from eye to target) is therefore `-z` in camera
//!   axes.
//! - The default projection is orthographic, as in NX: `height` is the
//!   model-space height of the viewport. Zoom changes `height`, never the
//!   eye distance, so orthographic zoom has no near-plane surprises.
//!
//! ## Standard views (NX names and keys)
//!
//! | View | Looks along | Screen up | Key |
//! | --- | --- | --- | --- |
//! | Top | −Z | +Y | |
//! | Front | +Y | +Z | |
//! | Right | −X | +Z | |
//! | Back | −Y | +Z | |
//! | Bottom | +Z | −Y | |
//! | Left | +X | +Z | |
//! | Trimetric | from (0.60, 1.00, 0.75) toward the target | +Z | Home |
//! | Isometric | from (1, 1, 1) toward the target | +Z | End |
//!
//! The isometric eye at (1, 1, 1) puts +X down-left, +Y down-right and +Z
//! up, the NX arrangement; the trimetric eye keeps that arrangement with
//! three unequal foreshortenings (our own choice of direction, not NX's).

/// A 3-vector of `f64` (model space).
pub type V3 = [f64; 3];

/// Basic vector helpers on `[f64; 3]`, kept local so the crate needs no
/// maths dependency.
pub mod v3 {
    use super::V3;
    /// a + b
    #[must_use]
    pub fn add(a: V3, b: V3) -> V3 {
        [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
    }
    /// a − b
    #[must_use]
    pub fn sub(a: V3, b: V3) -> V3 {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    /// s · a
    #[must_use]
    pub fn scale(a: V3, s: f64) -> V3 {
        [a[0] * s, a[1] * s, a[2] * s]
    }
    /// a · b
    #[must_use]
    pub fn dot(a: V3, b: V3) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    /// a × b
    #[must_use]
    pub fn cross(a: V3, b: V3) -> V3 {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
    /// |a|
    #[must_use]
    pub fn len(a: V3) -> f64 {
        dot(a, a).sqrt()
    }
    /// a / |a|, or `fallback` for a (near) zero vector.
    #[must_use]
    pub fn unit_or(a: V3, fallback: V3) -> V3 {
        let l = len(a);
        if l > 1e-300 {
            scale(a, 1.0 / l)
        } else {
            fallback
        }
    }
}

/// A unit quaternion `w + xi + yj + zk` used for the view orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    /// Scalar part.
    pub w: f64,
    /// Vector part.
    pub v: V3,
}

impl Quat {
    /// No rotation.
    pub const IDENTITY: Self = Self {
        w: 1.0,
        v: [0.0; 3],
    };

    /// Rotation by `angle` radians about the unit `axis`.
    #[must_use]
    pub fn axis_angle(axis: V3, angle: f64) -> Self {
        let (s, c) = (angle * 0.5).sin_cos();
        let a = v3::unit_or(axis, [0.0, 0.0, 1.0]);
        Self {
            w: c,
            v: v3::scale(a, s),
        }
    }

    /// The rotation whose columns are the given orthonormal axes, i.e. the
    /// one taking camera (x, y, z) to (`right`, `up`, `back`).
    /// Shepperd's method: pick the largest diagonal term for stability.
    #[must_use]
    pub fn from_axes(right: V3, up: V3, back: V3) -> Self {
        let (m00, m11, m22) = (right[0], up[1], back[2]);
        let trace = m00 + m11 + m22;
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            Self {
                w: 0.25 * s,
                v: [
                    (up[2] - back[1]) / s,
                    (back[0] - right[2]) / s,
                    (right[1] - up[0]) / s,
                ],
            }
        } else if m00 > m11 && m00 > m22 {
            let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
            Self {
                w: (up[2] - back[1]) / s,
                v: [0.25 * s, (up[0] + right[1]) / s, (back[0] + right[2]) / s],
            }
        } else if m11 > m22 {
            let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
            Self {
                w: (back[0] - right[2]) / s,
                v: [(up[0] + right[1]) / s, 0.25 * s, (back[1] + up[2]) / s],
            }
        } else {
            let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
            Self {
                w: (right[1] - up[0]) / s,
                v: [(back[0] + right[2]) / s, (back[1] + up[2]) / s, 0.25 * s],
            }
        };
        q.normalized()
    }

    /// `self ∘ other`: apply `other` first, then `self`.
    #[must_use]
    pub fn then_after(self, o: Self) -> Self {
        Self {
            w: self.w * o.w - v3::dot(self.v, o.v),
            v: v3::add(
                v3::add(v3::scale(o.v, self.w), v3::scale(self.v, o.w)),
                v3::cross(self.v, o.v),
            ),
        }
    }

    /// Rotates a vector: q v q*.
    #[must_use]
    pub fn rotate(self, p: V3) -> V3 {
        // v' = p + 2w(v×p) + 2 v×(v×p)
        let t = v3::scale(v3::cross(self.v, p), 2.0);
        v3::add(v3::add(p, v3::scale(t, self.w)), v3::cross(self.v, t))
    }

    /// Re-normalised (keeps accumulated orbit rotations unit length).
    #[must_use]
    pub fn normalized(self) -> Self {
        let n = (self.w * self.w + v3::dot(self.v, self.v)).sqrt();
        if n > 0.0 {
            Self {
                w: self.w / n,
                v: v3::scale(self.v, 1.0 / n),
            }
        } else {
            Self::IDENTITY
        }
    }
}

/// The standard views (see the module table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StandardView {
    /// Looking down −Z.
    Top,
    /// Looking along +Y.
    Front,
    /// Looking along −X.
    Right,
    /// Looking along −Y.
    Back,
    /// Looking along +Z.
    Bottom,
    /// Looking along +X.
    Left,
    /// Home key.
    Trimetric,
    /// End key.
    Isometric,
}

impl StandardView {
    /// Every standard view, in menu order.
    pub const ALL: [Self; 8] = [
        Self::Trimetric,
        Self::Isometric,
        Self::Top,
        Self::Front,
        Self::Right,
        Self::Back,
        Self::Bottom,
        Self::Left,
    ];

    /// Menu label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Top => "Top",
            Self::Front => "Front",
            Self::Right => "Right",
            Self::Back => "Back",
            Self::Bottom => "Bottom",
            Self::Left => "Left",
            Self::Trimetric => "Trimetric",
            Self::Isometric => "Isometric",
        }
    }

    /// (direction from target toward the eye, screen up), both in model axes.
    #[must_use]
    pub fn eye_and_up(self) -> (V3, V3) {
        match self {
            Self::Top => ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            Self::Front => ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Right => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Back => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Bottom => ([0.0, 0.0, -1.0], [0.0, -1.0, 0.0]),
            Self::Left => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            Self::Trimetric => (
                v3::unit_or([0.60, 1.00, 0.75], [0.0, 0.0, 1.0]),
                [0.0, 0.0, 1.0],
            ),
            Self::Isometric => (
                v3::unit_or([1.0, 1.0, 1.0], [0.0, 0.0, 1.0]),
                [0.0, 0.0, 1.0],
            ),
        }
    }

    /// The orientation that shows this view.
    #[must_use]
    pub fn orientation(self) -> Quat {
        let (back, up_hint) = self.eye_and_up();
        let right = v3::unit_or(v3::cross(up_hint, back), [1.0, 0.0, 0.0]);
        let up = v3::cross(back, right);
        Quat::from_axes(right, up, back)
    }
}

/// Orthographic or perspective projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// Parallel projection (the NX default).
    Orthographic,
    /// Perspective with a 30° vertical field of view.
    Perspective,
}

/// The view camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Point the view rotates about and centres on (model space, mm).
    pub target: V3,
    /// Camera axes → model axes.
    pub orientation: Quat,
    /// Model-space height of the viewport (orthographic), mm.
    pub height: f64,
    /// Projection.
    pub projection: Projection,
}

/// Vertical field of view used by [`Projection::Perspective`].
pub const PERSPECTIVE_FOV: f64 = 30.0 * std::f64::consts::PI / 180.0;

impl Default for Camera {
    fn default() -> Self {
        Self {
            target: [0.0; 3],
            orientation: StandardView::Trimetric.orientation(),
            height: 200.0,
            projection: Projection::Orthographic,
        }
    }
}

impl Camera {
    /// Screen right, up and toward-the-viewer axes in model space.
    #[must_use]
    pub fn axes(&self) -> (V3, V3, V3) {
        let q = self.orientation;
        (
            q.rotate([1.0, 0.0, 0.0]),
            q.rotate([0.0, 1.0, 0.0]),
            q.rotate([0.0, 0.0, 1.0]),
        )
    }

    /// Eye distance from the target: far enough that the whole scene of
    /// "radius" `scene_radius` is in front of the eye, and, in perspective,
    /// the distance at which `height` fills the view.
    #[must_use]
    pub fn eye_distance(&self, scene_radius: f64) -> f64 {
        match self.projection {
            Projection::Orthographic => 4.0 * scene_radius.max(self.height),
            Projection::Perspective => 0.5 * self.height / (0.5 * PERSPECTIVE_FOV).tan(),
        }
    }

    /// Shows a standard view, keeping target and zoom.
    pub fn set_view(&mut self, view: StandardView) {
        self.orientation = view.orientation();
    }

    /// F8 "snap view": the standard view (of the six orthographic ones and
    /// the two pictorial ones) whose eye direction is nearest the current
    /// one.
    #[must_use]
    pub fn nearest_standard_view(&self) -> StandardView {
        let (_, _, back) = self.axes();
        let mut best = (StandardView::Top, f64::NEG_INFINITY);
        for v in StandardView::ALL {
            let d = v3::dot(v.eye_and_up().0, back);
            if d > best.1 {
                best = (v, d);
            }
        }
        best.0
    }

    /// Orbit (MB2 drag): `dx`, `dy` in pixels for a viewport `height_px`
    /// tall. A drag across the viewport height turns 180°. Horizontal motion
    /// turns about the screen's up axis, vertical about its right axis,
    /// both through `pivot` (the target, or the point under the cursor when
    /// the drag began).
    pub fn orbit(&mut self, dx: f64, dy: f64, height_px: f64, pivot: V3) {
        let k = std::f64::consts::PI / height_px.max(1.0);
        let (right, up, _) = self.axes();
        // Turning the camera one way makes the model appear to turn the
        // other: a rightward drag turns the model to the right, so the
        // camera turns by −dx about up.
        let q = Quat::axis_angle(up, -dx * k).then_after(Quat::axis_angle(right, -dy * k));
        self.orientation = q.then_after(self.orientation).normalized();
        // Keep the pivot fixed on screen: rotate the target about it.
        let offset = v3::sub(self.target, pivot);
        self.target = v3::add(pivot, q.rotate(offset));
    }

    /// Pan (MB2+MB3 or Shift+MB2): the model follows the cursor.
    pub fn pan(&mut self, dx: f64, dy: f64, height_px: f64) {
        let mm_per_px = self.height / height_px.max(1.0);
        let (right, up, _) = self.axes();
        let shift = v3::add(
            v3::scale(right, -dx * mm_per_px),
            v3::scale(up, dy * mm_per_px),
        );
        self.target = v3::add(self.target, shift);
    }

    /// Zoom by `factor` (> 1 zooms in) keeping the model point under the
    /// cursor fixed. `at_px` is the cursor relative to the viewport centre
    /// (pixels, +y down, as GUI toolkits report it).
    pub fn zoom_at(&mut self, factor: f64, at_px: [f64; 2], height_px: f64) {
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let mm_per_px = self.height / height_px.max(1.0);
        let (right, up, _) = self.axes();
        // The model point under the cursor, in the target's view plane.
        let under = v3::add(
            self.target,
            v3::add(
                v3::scale(right, at_px[0] * mm_per_px),
                v3::scale(up, -at_px[1] * mm_per_px),
            ),
        );
        let new_height = (self.height / factor).clamp(1e-6, 1e9);
        let s = new_height / self.height;
        // Scale the target's offset from that point by the same ratio.
        self.target = v3::add(under, v3::scale(v3::sub(self.target, under), s));
        self.height = new_height;
    }

    /// Fit (Ctrl+F): centre on the box and zoom so it fills ~90 % of the
    /// viewport whatever the orientation (the bounding sphere is used).
    pub fn fit(&mut self, min: V3, max: V3, aspect: f64) {
        let center = v3::scale(v3::add(min, max), 0.5);
        let radius = (0.5 * v3::len(v3::sub(max, min))).max(1e-6);
        self.target = center;
        // The sphere must fit both the height and, for narrow viewports,
        // the width.
        let fit_height = 2.0 * radius / 0.9;
        self.height = if aspect < 1.0 {
            fit_height / aspect.max(1e-3)
        } else {
            fit_height
        };
    }

    /// View matrix (model → camera), column-major `f32` for WGSL, with the
    /// model translated by `-origin` first. Rendering relative to a nearby
    /// origin keeps `f32` precision for parts far from the model origin.
    #[must_use]
    pub fn view_matrix(&self, scene_radius: f64, origin: V3) -> [[f32; 4]; 4] {
        let (r, u, b) = self.axes();
        let eye = v3::add(
            v3::sub(self.target, origin),
            v3::scale(b, self.eye_distance(scene_radius)),
        );
        // Rows of the rotation are the camera axes; translation is −R·eye.
        let t = [-v3::dot(r, eye), -v3::dot(u, eye), -v3::dot(b, eye)];
        cols([
            [r[0], u[0], b[0], 0.0],
            [r[1], u[1], b[1], 0.0],
            [r[2], u[2], b[2], 0.0],
            [t[0], t[1], t[2], 1.0],
        ])
    }

    /// Projection matrix (camera → clip), column-major, WebGPU depth range
    /// 0..1, reversed so that near = 1 and far = 0 (better depth precision;
    /// the depth test is `Greater`).
    #[must_use]
    pub fn projection_matrix(&self, aspect: f64, scene_radius: f64) -> [[f32; 4]; 4] {
        let dist = self.eye_distance(scene_radius);
        // Depth range: everything within the scene sphere around the
        // target, with margin.
        let near = (dist - 2.0 * scene_radius.max(self.height)).max(dist * 1e-4);
        let far = dist + 2.0 * scene_radius.max(self.height);
        match self.projection {
            Projection::Orthographic => {
                let h = self.height * 0.5;
                let w = h * aspect;
                // z_clip = (z_cam + far) / (far − near) maps −far→0, −near→1.
                let a = 1.0 / (far - near);
                cols([
                    [1.0 / w, 0.0, 0.0, 0.0],
                    [0.0, 1.0 / h, 0.0, 0.0],
                    [0.0, 0.0, a, 0.0],
                    [0.0, 0.0, far * a, 1.0],
                ])
            }
            Projection::Perspective => {
                let f = 1.0 / (0.5 * PERSPECTIVE_FOV).tan();
                // Reversed-Z perspective: z_ndc = near/(−z_cam) mapped so
                // −near → 1 and −far → 0.
                let a = near / (far - near);
                let b2 = far * near / (far - near);
                cols([
                    [f / aspect, 0.0, 0.0, 0.0],
                    [0.0, f, 0.0, 0.0],
                    [0.0, 0.0, a, -1.0],
                    [0.0, 0.0, b2, 0.0],
                ])
            }
        }
    }

    /// Where model point `p` lands in a viewport of `size` pixels (from the
    /// top-left, +y down), or `None` behind a perspective eye. Used for
    /// overlays (WCS, datum axes, labels) painted by the shell.
    #[must_use]
    pub fn project(&self, point: V3, size: [f64; 2], scene_radius: f64) -> Option<[f64; 2]> {
        let (right, up, back) = self.axes();
        let rel = v3::sub(point, self.target);
        let (x, y) = (v3::dot(rel, right), v3::dot(rel, up));
        let px_per_mm = size[1] / self.height.max(1e-12);
        let (sx, sy) = match self.projection {
            Projection::Orthographic => (x * px_per_mm, y * px_per_mm),
            Projection::Perspective => {
                let d = self.eye_distance(scene_radius);
                // Depth in front of the eye; the target plane is at d.
                let depth = d - v3::dot(rel, back);
                if depth <= d * 1e-6 {
                    return None;
                }
                let k = d / depth;
                (x * k * px_per_mm, y * k * px_per_mm)
            }
        };
        Some([0.5 * size[0] + sx, 0.5 * size[1] - sy])
    }

    /// The model point at viewport pixel `px` (from the top-left, +y down)
    /// whose depth-buffer value is `depth` (reversed: 1 at the near plane,
    /// 0 at the far plane), the inverse of the projection the GPU used for
    /// a viewport of `size` pixels. GPU picking uses it to turn the depth
    /// under the cursor into a 3D point.
    #[must_use]
    pub fn unproject(&self, px: [f64; 2], size: [f64; 2], depth: f64, scene_radius: f64) -> V3 {
        let (right, up, back) = self.axes();
        let dist = self.eye_distance(scene_radius);
        let near = (dist - 2.0 * scene_radius.max(self.height)).max(dist * 1e-4);
        let far = dist + 2.0 * scene_radius.max(self.height);
        let mm_per_px = self.height / size[1].max(1.0);
        // Offsets from the viewport centre in the target's plane (mm).
        let sx = (px[0] - 0.5 * size[0]) * mm_per_px;
        let sy = (0.5 * size[1] - px[1]) * mm_per_px;
        let eye = v3::add(self.target, v3::scale(back, dist));
        // Camera-space depth (negative in front of the eye), and the factor
        // that scales target-plane offsets to that depth.
        let (z, k) = match self.projection {
            // z_ndc = (z + far) / (far − near)
            Projection::Orthographic => (depth * (far - near) - far, 1.0),
            Projection::Perspective => {
                // z_ndc = (a z + b) / (−z), a = n/(f−n), b = f n/(f−n)
                let a = near / (far - near);
                let b = far * near / (far - near);
                let z = -b / (depth + a);
                (z, -z / dist)
            }
        };
        v3::add(
            eye,
            v3::add(
                v3::add(v3::scale(right, sx * k), v3::scale(up, sy * k)),
                v3::scale(back, z),
            ),
        )
    }

    /// Where model direction `d` points on screen, as a unit 2-vector
    /// (+x right, +y up) scaled by how much of it faces the screen. Used to
    /// draw the view triad.
    #[must_use]
    pub fn screen_direction(&self, d: V3) -> [f64; 2] {
        let (r, u, _) = self.axes();
        [v3::dot(d, r), v3::dot(d, u)]
    }
}

/// Narrows a `f64` matrix to the `f32` the GPU takes.
#[allow(clippy::cast_possible_truncation)]
fn cols(m: [[f64; 4]; 4]) -> [[f32; 4]; 4] {
    m.map(|c| c.map(|x| x as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool {
        v3::len(v3::sub(a, b)) < 1e-9
    }

    #[test]
    fn standard_views_look_where_the_table_says() {
        // Front: looking along +Y, so the eye is at −Y, and +X is to the right.
        let mut c = Camera::default();
        c.set_view(StandardView::Front);
        let (r, u, b) = c.axes();
        assert!(
            close(r, [1.0, 0.0, 0.0]) && close(u, [0.0, 0.0, 1.0]) && close(b, [0.0, -1.0, 0.0])
        );
        // Isometric: +X down-left, +Y down-right, +Z up.
        c.set_view(StandardView::Isometric);
        let [xr, xu] = c.screen_direction([1.0, 0.0, 0.0]);
        let [yr, yu] = c.screen_direction([0.0, 1.0, 0.0]);
        let [_, zu] = c.screen_direction([0.0, 0.0, 1.0]);
        assert!(xr < 0.0 && xu < 0.0 && yr > 0.0 && yu < 0.0 && zu > 0.0);
        for v in StandardView::ALL {
            c.set_view(v);
            assert_eq!(c.nearest_standard_view(), v, "{v:?} snaps to itself");
        }
    }

    #[test]
    fn orbit_keeps_the_pivot_and_snap_returns_to_a_standard_view() {
        let mut c = Camera {
            target: [10.0, 0.0, 0.0],
            ..Camera::default()
        };
        c.set_view(StandardView::Top);
        c.orbit(30.0, 10.0, 600.0, [10.0, 0.0, 0.0]);
        assert!(
            close(c.target, [10.0, 0.0, 0.0]),
            "pivot at the target stays put"
        );
        assert_eq!(
            c.nearest_standard_view(),
            StandardView::Top,
            "a small orbit snaps back (F8)"
        );
        let (r, u, b) = c.axes();
        assert!(
            (v3::dot(r, u)).abs() < 1e-12 && (v3::dot(u, b)).abs() < 1e-12,
            "still orthonormal"
        );
        // A drag of the full height is a half turn: top becomes bottom.
        let mut c = Camera::default();
        c.set_view(StandardView::Top);
        c.orbit(0.0, 600.0, 600.0, c.target);
        assert_eq!(c.nearest_standard_view(), StandardView::Bottom);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor() {
        let mut c = Camera::default();
        c.set_view(StandardView::Top);
        let at = [100.0, -50.0];
        let h_px = 500.0;
        let (r, u, _) = c.axes();
        let under = |c: &Camera| {
            v3::add(
                c.target,
                v3::add(
                    v3::scale(r, at[0] * c.height / h_px),
                    v3::scale(u, -at[1] * c.height / h_px),
                ),
            )
        };
        let before = under(&c);
        c.zoom_at(2.0, at, h_px);
        assert!((c.height - 100.0).abs() < 1e-9);
        assert!(close(under(&c), before));
        c.zoom_at(f64::NAN, at, h_px);
        assert!(
            (c.height - 100.0).abs() < 1e-9,
            "a bad factor changes nothing"
        );
    }

    #[test]
    fn pan_moves_the_model_with_the_cursor_and_fit_frames_the_box() {
        let mut c = Camera::default();
        c.set_view(StandardView::Front);
        c.height = 100.0;
        c.pan(50.0, 0.0, 100.0);
        // Dragging right by half the viewport moves the target left by 50 mm,
        // so the model appears to move right.
        assert!(close(c.target, [-50.0, 0.0, 0.0]));
        c.fit([0.0, 0.0, 0.0], [20.0, 20.0, 10.0], 1.6);
        assert!(close(c.target, [10.0, 10.0, 5.0]));
        assert!((c.height - 2.0 * 15.0 / 0.9).abs() < 1e-9);
    }

    #[test]
    fn projection_matches_the_view() {
        let mut c = Camera::default();
        c.set_view(StandardView::Front);
        c.height = 100.0;
        // 10 mm right of and 25 mm above the target, in a 200 × 100 px view.
        let p = c.project([10.0, 0.0, 25.0], [200.0, 100.0], 50.0).unwrap();
        assert!(
            (p[0] - 110.0).abs() < 1e-9 && (p[1] - 25.0).abs() < 1e-9,
            "{p:?}"
        );
        c.projection = Projection::Perspective;
        let centre = c.project(c.target, [200.0, 100.0], 50.0).unwrap();
        assert!((centre[0] - 100.0).abs() < 1e-9 && (centre[1] - 50.0).abs() < 1e-9);
        // Far behind the eye: not drawn.
        assert!(c.project([0.0, -1e6, 0.0], [200.0, 100.0], 50.0).is_none());
    }

    #[test]
    #[allow(
        clippy::many_single_char_names,
        reason = "c, p, v, m: camera, point and matrices, as in the maths"
    )]
    fn unproject_inverts_the_gpu_projection() {
        for projection in [Projection::Orthographic, Projection::Perspective] {
            let c = Camera {
                target: [5.0, 6.0, 7.0],
                projection,
                ..Camera::default()
            };
            let (radius, size) = (50.0, [300.0, 200.0]);
            let p = [12.0, -3.0, 20.0];
            // Depth exactly as the GPU computes it from the matrices.
            let v = c.view_matrix(radius, [0.0; 3]);
            let m = c.projection_matrix(size[0] / size[1], radius);
            let apply = |m: [[f32; 4]; 4], x: [f64; 4]| -> [f64; 4] {
                let mut o = [0.0; 4];
                for (col, &xi) in m.iter().zip(x.iter()) {
                    for (oi, &mij) in o.iter_mut().zip(col.iter()) {
                        *oi += f64::from(mij) * xi;
                    }
                }
                o
            };
            let clip = apply(m, apply(v, [p[0], p[1], p[2], 1.0]));
            let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
            let px = [
                (ndc[0] + 1.0) * 0.5 * size[0],
                (1.0 - ndc[1]) * 0.5 * size[1],
            ];
            let back = c.unproject(px, size, ndc[2], radius);
            assert!(
                v3::len(v3::sub(back, p)) < 1e-3,
                "{projection:?}: {back:?} vs {p:?}"
            );
        }
    }

    #[test]
    fn matrices_map_the_target_to_the_centre_and_depth_into_range() {
        for projection in [Projection::Orthographic, Projection::Perspective] {
            let c = Camera {
                target: [5.0, 6.0, 7.0],
                projection,
                ..Camera::default()
            };
            let v = c.view_matrix(50.0, [0.0; 3]);
            let p = c.projection_matrix(1.5, 50.0);
            let apply = |m: [[f32; 4]; 4], x: [f32; 4]| -> [f32; 4] {
                let mut o = [0.0f32; 4];
                for (col, &xi) in m.iter().zip(x.iter()) {
                    for (oi, &mij) in o.iter_mut().zip(col.iter()) {
                        *oi += mij * xi;
                    }
                }
                o
            };
            let clip = apply(p, apply(v, [5.0, 6.0, 7.0, 1.0]));
            let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
            assert!(
                ndc[0].abs() < 1e-4 && ndc[1].abs() < 1e-4,
                "{projection:?}: target at the centre {ndc:?}"
            );
            assert!(
                ndc[2] > 0.0 && ndc[2] < 1.0,
                "{projection:?}: depth in range {ndc:?}"
            );
        }
    }
}
