// SPDX-License-Identifier: LGPL-2.1-or-later
//! The orbit benchmark behind the Stage 0 gate item "orbiting the
//! 5-million-triangle benchmark model holds the agreed frame rate on the
//! reference GPU".
//!
//! The model is generated, so every machine renders the same thing: a
//! torus knot-like tube, a smooth closed surface tessellated into an
//! `n × m` grid of quads (two triangles each), split into 64 "faces" so
//! picking ids and per-face vertex duplication behave as for a real part.
//! Each frame turns the camera by 3° and waits for the GPU to finish, so
//! the frame time is honest (no queued-but-unrendered frames).

use crate::camera::{Camera, StandardView};
use crate::gpu::{DisplaySettings, GpuContext, MeshInput, RenderError, Scene, Viewport};
use std::time::{Duration, Instant};

/// The outcome of one benchmark run.
#[derive(Debug, Clone)]
pub struct OrbitReport {
    /// Triangles drawn per frame.
    pub triangles: usize,
    /// Frames measured (after one warm-up frame).
    pub frames: u32,
    /// Viewport size.
    pub size: [u32; 2],
    /// Wall time of the measured frames.
    pub elapsed: Duration,
    /// Slowest single frame.
    pub worst: Duration,
    /// Adapter description.
    pub adapter: String,
    /// The adapter is a software rasteriser.
    pub software: bool,
}

impl OrbitReport {
    /// Mean frames per second.
    #[must_use]
    pub fn fps(&self) -> f64 {
        f64::from(self.frames) / self.elapsed.as_secs_f64().max(1e-9)
    }
}

impl std::fmt::Display for OrbitReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} triangles, {}×{} px, {} frames: {:.2} fps mean, worst frame {:.1} ms on {}{}",
            self.triangles,
            self.size[0],
            self.size[1],
            self.frames,
            self.fps(),
            self.worst.as_secs_f64() * 1e3,
            self.adapter,
            if self.software {
                " (software rasteriser)"
            } else {
                ""
            }
        )
    }
}

/// A closed tube around a (3, 2) torus knot with about `triangles`
/// triangles: smooth, self-occluding, and nothing like a box, so depth,
/// silhouettes and lighting all do real work.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names // grid indices i, j and quad corners a..d
)]
pub fn benchmark_mesh(triangles: usize) -> MeshInput {
    use std::f64::consts::TAU;
    // n around the knot, m around the tube, n = 8 m, 2 n m triangles.
    let m = ((triangles as f64 / 16.0).sqrt().round() as usize).max(3);
    let n = 8 * m;
    let knot = |t: f64| {
        let (p, q) = (2.0, 3.0);
        let r = 40.0 + 15.0 * (q * t).cos();
        [r * (p * t).cos(), r * (p * t).sin(), 15.0 * (q * t).sin()]
    };
    let sub = crate::camera::v3::sub;
    let mut positions = Vec::with_capacity(n * m);
    let mut normals = Vec::with_capacity(n * m);
    for i in 0..n {
        let t = TAU * i as f64 / n as f64;
        let c = knot(t);
        let tangent =
            crate::camera::v3::unit_or(sub(knot(t + 1e-4), knot(t - 1e-4)), [1.0, 0.0, 0.0]);
        let side = crate::camera::v3::unit_or(
            crate::camera::v3::cross(tangent, [0.0, 0.0, 1.0]),
            [0.0, 1.0, 0.0],
        );
        let up = crate::camera::v3::cross(side, tangent);
        for j in 0..m {
            let a = TAU * j as f64 / m as f64;
            let dir = crate::camera::v3::add(
                crate::camera::v3::scale(side, a.cos()),
                crate::camera::v3::scale(up, a.sin()),
            );
            positions.push(crate::camera::v3::add(
                c,
                crate::camera::v3::scale(dir, 6.0),
            ));
            normals.push(dir.map(|x| x as f32));
        }
    }
    let idx = |i: usize, j: usize| u32::try_from((i % n) * m + (j % m)).unwrap_or(0);
    let mut tris = Vec::with_capacity(2 * n * m);
    let mut tri_face = Vec::with_capacity(2 * n * m);
    for i in 0..n {
        for j in 0..m {
            let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
            let face = u32::try_from(i * 64 / n).unwrap_or(0);
            tris.push([a, b, c]);
            tris.push([a, c, d]);
            tri_face.push(face);
            tri_face.push(face);
        }
    }
    MeshInput {
        positions,
        normals,
        triangles: tris,
        tri_face,
        edges: Vec::new(),
    }
}

/// Renders `frames` frames of an orbit around the benchmark model.
///
/// # Errors
/// Read-back errors from the GPU.
pub fn orbit(
    ctx: &GpuContext,
    triangles: usize,
    frames: u32,
    size: [u32; 2],
) -> Result<OrbitReport, RenderError> {
    let mesh = benchmark_mesh(triangles);
    let drawn = mesh.triangles.len();
    let mut scene = Scene::new();
    scene.add(ctx, &mesh, [0.55, 0.62, 0.72, 1.0]);
    let mut viewport = Viewport::new(ctx);
    let mut camera = Camera::default();
    camera.set_view(StandardView::Trimetric);
    if let Some((lo, hi)) = scene.bounds() {
        camera.fit(lo, hi, f64::from(size[0]) / f64::from(size[1].max(1)));
    }
    let settings = DisplaySettings::LIGHT;
    let wait = |ctx: &GpuContext| {
        ctx.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map(|_| ())
            .map_err(|e| RenderError::Readback(e.to_string()))
    };
    // Warm-up: pipelines compiled, buffers resident.
    viewport.render(ctx, &scene, &camera, &settings, size);
    wait(ctx)?;
    let start = Instant::now();
    let mut worst = Duration::ZERO;
    for _ in 0..frames {
        let t = Instant::now();
        camera.orbit(
            f64::from(size[1]) / 60.0,
            0.0,
            f64::from(size[1]),
            camera.target,
        );
        viewport.render(ctx, &scene, &camera, &settings, size);
        wait(ctx)?;
        worst = worst.max(t.elapsed());
    }
    Ok(OrbitReport {
        triangles: drawn,
        frames,
        size,
        elapsed: start.elapsed(),
        worst,
        adapter: format!(
            "{} ({:?}, {:?})",
            ctx.adapter.name, ctx.adapter.backend, ctx.adapter.device_type
        ),
        software: ctx.software_warning().is_some(),
    })
}
