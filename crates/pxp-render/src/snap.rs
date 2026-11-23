// SPDX-License-Identifier: LGPL-2.1-or-later
//! Point snapping for dialogs' "Specify Point" steps (the NX snap toolbar).
//!
//! A click gives a GPU hit: the face under the cursor and the 3D point on
//! it. Snapping then looks for a better point near the cursor on screen,
//! within [`SNAP_RADIUS_PX`]:
//!
//! | Snap | Candidates | Stage |
//! | --- | --- | --- |
//! | Existing Point | datum CSYS origins and other points the shell passes | 0 |
//! | End Point | both ends of every displayed edge of the clicked body | 0 |
//! | Mid Point | the arc-length middle of every displayed edge of the clicked body | 0 |
//! | Point on Face | the hit itself | 0 |
//! | Control Point, Intersection, Arc Center, Quadrant | need exact curve geometry | 1 |
//!
//! Edges are the display polylines, so an end point is exact and a curved
//! edge's mid point is within the display chord tolerance; Stage 1
//! evaluates them on the exact B-rep through the kernel.

use crate::camera::{Camera, V3, v3};

/// Pixels within which a snap candidate wins over the plain face hit.
pub const SNAP_RADIUS_PX: f64 = 12.0;

/// Which snaps are on (the snap toolbar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one toggle per toolbar button"
)]
pub struct SnapSet {
    /// Datum origins and other existing points.
    pub existing: bool,
    /// Edge ends.
    pub end: bool,
    /// Edge middles.
    pub mid: bool,
    /// The point on the face under the cursor.
    pub on_face: bool,
}

impl Default for SnapSet {
    fn default() -> Self {
        Self {
            existing: true,
            end: true,
            mid: true,
            on_face: true,
        }
    }
}

/// What a snap chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    /// An existing point.
    Existing,
    /// An edge end.
    End,
    /// An edge middle.
    Mid,
    /// The face hit itself.
    OnFace,
}

impl SnapKind {
    /// Label for the status line.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Existing => "Existing Point",
            Self::End => "End Point",
            Self::Mid => "Mid Point",
            Self::OnFace => "Point on Face",
        }
    }
}

/// Arc-length middle of a polyline.
fn midpoint(line: &[V3]) -> Option<V3> {
    let total: f64 = line
        .windows(2)
        .map(|w| match w {
            [a, b] => v3::len(v3::sub(*b, *a)),
            _ => 0.0,
        })
        .sum();
    let mut left = 0.5 * total;
    for w in line.windows(2) {
        if let [a, b] = w {
            let l = v3::len(v3::sub(*b, *a));
            if l >= left && l > 0.0 {
                return Some(v3::add(*a, v3::scale(v3::sub(*b, *a), left / l)));
            }
            left -= l;
        }
    }
    line.first().copied()
}

/// Snaps a click. `hit` is the face point under the cursor, if the click
/// hit a body; `edges` are that body's display edges; `points` are
/// existing points. Returns the chosen point, or `None` when nothing
/// qualifies (a click on empty space with no snap candidate near it).
#[must_use]
#[allow(
    clippy::too_many_arguments,
    reason = "the view, the click and the candidates are all independent inputs"
)]
pub fn snap(
    camera: &Camera,
    size: [f64; 2],
    scene_radius: f64,
    cursor: [f64; 2],
    hit: Option<V3>,
    edges: &[Vec<V3>],
    points: &[V3],
    set: SnapSet,
) -> Option<(V3, SnapKind)> {
    let mut best: Option<(f64, V3, SnapKind)> = None;
    let mut offer = |p: V3, kind: SnapKind| {
        if let Some(s) = camera.project(p, size, scene_radius) {
            let d = (s[0] - cursor[0]).hypot(s[1] - cursor[1]);
            if d <= SNAP_RADIUS_PX && best.is_none_or(|(bd, _, _)| d < bd) {
                best = Some((d, p, kind));
            }
        }
    };
    if set.existing {
        for p in points {
            offer(*p, SnapKind::Existing);
        }
    }
    if set.end {
        for line in edges {
            if let (Some(a), Some(b)) = (line.first(), line.last()) {
                offer(*a, SnapKind::End);
                offer(*b, SnapKind::End);
            }
        }
    }
    if set.mid {
        for line in edges {
            if let Some(m) = midpoint(line) {
                offer(m, SnapKind::Mid);
            }
        }
    }
    best.map(|(_, p, k)| (p, k))
        .or_else(|| hit.filter(|_| set.on_face).map(|p| (p, SnapKind::OnFace)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::camera::StandardView;

    fn top_camera() -> Camera {
        let mut c = Camera {
            target: [50.0, 50.0, 0.0],
            height: 100.0,
            ..Camera::default()
        };
        c.set_view(StandardView::Top);
        c
    }

    #[test]
    fn nearby_ends_and_middles_beat_the_face_hit() {
        let c = top_camera();
        let size = [100.0, 100.0]; // 1 px per mm
        let edges = vec![vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]]];
        let at = |x: f64, y: f64| c.project([x, y, 0.0], size, 100.0).unwrap();
        let set = SnapSet::default();
        // 5 px from the edge's end: End Point.
        let (p, k) = snap(
            &c,
            size,
            100.0,
            at(3.0, 4.0),
            Some([3.0, 4.0, 0.0]),
            &edges,
            &[],
            set,
        )
        .unwrap();
        assert_eq!((p, k), ([0.0, 0.0, 0.0], SnapKind::End));
        // Near the middle: Mid Point.
        let (p, k) = snap(
            &c,
            size,
            100.0,
            at(52.0, 3.0),
            Some([52.0, 3.0, 0.0]),
            &edges,
            &[],
            set,
        )
        .unwrap();
        assert_eq!((p, k), ([50.0, 0.0, 0.0], SnapKind::Mid));
        // Far from everything: the face point.
        let (p, k) = snap(
            &c,
            size,
            100.0,
            at(30.0, 40.0),
            Some([30.0, 40.0, 0.0]),
            &edges,
            &[],
            set,
        )
        .unwrap();
        assert_eq!((p, k), ([30.0, 40.0, 0.0], SnapKind::OnFace));
        // Existing points (datum origins) win when closest.
        let (_, k) = snap(
            &c,
            size,
            100.0,
            at(1.0, 1.0),
            None,
            &edges,
            &[[0.5, 0.5, 0.0]],
            set,
        )
        .unwrap();
        assert_eq!(k, SnapKind::Existing);
        // Turned off: End Point is not offered, and empty space gives nothing.
        let only_face = SnapSet {
            existing: false,
            end: false,
            mid: false,
            on_face: true,
        };
        assert_eq!(
            snap(&c, size, 100.0, at(3.0, 4.0), None, &edges, &[], only_face),
            None
        );
    }
}
