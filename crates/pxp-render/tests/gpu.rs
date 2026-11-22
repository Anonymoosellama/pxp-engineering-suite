// SPDX-License-Identifier: LGPL-2.1-or-later
//! Headless GPU tests of the viewport: they run wherever an adapter exists
//! (a software rasteriser is enough: lavapipe on Linux CI, WARP on Windows)
//! and skip, saying so, where none does.
//!
//! Crash tracing. A driver crash kills the test process, and libtest's
//! captured output dies with it, so the log cannot say what was running.
//! Two aids, both visible with `-- --nocapture` (CI's diagnostic steps
//! pass it; normal runs stay quiet because output is captured):
//!
//! - `trace` prints a `[gpu-test <test name>] <step>` line before each GPU
//!   step (adapter, upload, render, read-back, pick), so the last line in
//!   the log names the step that crashed.
//! - `install_log` sends wgpu's own log records (including the Direct3D 12
//!   debug layer's messages, which wgpu forwards when validation is on) to
//!   stderr. Level from `PXP_GPU_LOG` (`off`, `error`, `warn`, `info`,
//!   `debug`, `trace`); default `warn`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    missing_docs
)]

use pxp_render::{
    Camera, DisplaySettings, GpuContext, Hit, MeshInput, Picked, RenderError, Scene, StandardView,
    Viewport,
};

/// Prints one step marker. The test name comes from the thread name, which
/// libtest sets to the test's name.
fn trace(step: &str) {
    let thread = std::thread::current();
    eprintln!("[gpu-test {}] {step}", thread.name().unwrap_or("?"));
}

/// A minimal `log` backend: every record at or above the level goes to
/// stderr as `[LEVEL target] message`. `eprintln!` is unbuffered, so a
/// record written just before a crash still reaches the log.
struct StderrLog(log::LevelFilter);

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.0
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

/// Installs `StderrLog` once per test process (tests run on several threads
/// and `log` accepts only one logger, so later calls do nothing).
fn install_log() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let level = std::env::var("PXP_GPU_LOG")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(log::LevelFilter::Warn);
        // `log` wants a `&'static` logger; leaking one small struct for the
        // life of the test process is the usual way to get it.
        let logger: &'static StderrLog = Box::leak(Box::new(StderrLog(level)));
        if log::set_logger(logger).is_ok() {
            log::set_max_level(level);
        }
    });
}

fn gpu() -> Option<GpuContext> {
    install_log();
    trace("create instance, adapter and device");
    match GpuContext::headless() {
        Ok(c) => {
            // Backend, driver and device type: which path the crash is on.
            trace(&format!(
                "adapter {:?} backend {:?} type {:?} driver {:?} {:?}",
                c.adapter.name,
                c.adapter.backend,
                c.adapter.device_type,
                c.adapter.driver,
                c.adapter.driver_info
            ));
            Some(c)
        }
        Err(e) => {
            eprintln!("skipping GPU test: {e}");
            None
        }
    }
}

/// A 20 × 30 × 10 mm box at (100, 200, 300), faces in the order
/// −Z, +Z, −Y, +Y, −X, +X, two outward triangles each, plus its 12 edges.
fn box_mesh() -> MeshInput {
    let o = [100.0, 200.0, 300.0];
    let s = [20.0, 30.0, 10.0];
    let corner = |i: usize| -> [f64; 3] {
        [
            o[0] + if i & 1 == 0 { 0.0 } else { s[0] },
            o[1] + if i & 2 == 0 { 0.0 } else { s[1] },
            o[2] + if i & 4 == 0 { 0.0 } else { s[2] },
        ]
    };
    // Each face: four corner indices counter-clockwise from outside, normal.
    let faces: [([usize; 4], [f32; 3]); 6] = [
        ([0, 2, 3, 1], [0.0, 0.0, -1.0]),
        ([4, 5, 7, 6], [0.0, 0.0, 1.0]),
        ([0, 1, 5, 4], [0.0, -1.0, 0.0]),
        ([2, 6, 7, 3], [0.0, 1.0, 0.0]),
        ([0, 4, 6, 2], [-1.0, 0.0, 0.0]),
        ([1, 3, 7, 5], [1.0, 0.0, 0.0]),
    ];
    let mut m = MeshInput::default();
    for (f, (q, n)) in faces.iter().enumerate() {
        let base = u32::try_from(m.positions.len()).unwrap();
        for &c in q {
            m.positions.push(corner(c));
            m.normals.push(*n);
        }
        m.triangles.push([base, base + 1, base + 2]);
        m.triangles.push([base, base + 2, base + 3]);
        let f = u32::try_from(f).unwrap();
        m.tri_face.extend([f, f]);
    }
    for (a, b) in [
        (0, 1),
        (2, 3),
        (4, 5),
        (6, 7),
        (0, 2),
        (1, 3),
        (4, 6),
        (5, 7),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ] {
        m.edges.push(vec![corner(a), corner(b)]);
    }
    m
}

fn setup(ctx: &GpuContext) -> (Scene, Viewport, Camera) {
    let mut scene = Scene::new();
    trace("upload the box mesh");
    scene.add(ctx, &box_mesh(), [0.6, 0.65, 0.7, 1.0]);
    let mut cam = Camera::default();
    cam.set_view(StandardView::Top);
    let (lo, hi) = scene.bounds().unwrap();
    cam.fit(lo, hi, 1.0);
    trace("create the viewport (shaders and pipelines)");
    let vp = Viewport::new(ctx);
    (scene, vp, cam)
}

// The three viewport calls the tests make, each announced by `trace` so a
// crash log names the call that was running.

fn render(
    vp: &mut Viewport,
    ctx: &GpuContext,
    scene: &Scene,
    cam: &Camera,
    s: &DisplaySettings,
    size: [u32; 2],
) {
    trace(&format!("render {}x{}", size[0], size[1]));
    vp.render(ctx, scene, cam, s, size);
}

fn read_pixels(vp: &Viewport, ctx: &GpuContext) -> Result<(Vec<u8>, [u32; 2]), RenderError> {
    trace("read back the colour target");
    vp.read_pixels(ctx)
}

fn pick(
    vp: &mut Viewport,
    ctx: &GpuContext,
    scene: &Scene,
    cam: &Camera,
    s: &DisplaySettings,
    at: [u32; 2],
) -> Result<Option<Hit>, RenderError> {
    trace(&format!("pick at {at:?}"));
    vp.pick(ctx, scene, cam, s, at)
}

fn pixel(px: &[u8], size: [u32; 2], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * size[0] + x) * 4) as usize;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

#[test]
fn a_box_renders_over_the_gradient_and_picks_its_top_face() {
    let Some(ctx) = gpu() else { return };
    let (scene, mut vp, cam) = setup(&ctx);
    let size = [128, 128];
    let s = DisplaySettings::LIGHT;
    render(&mut vp, &ctx, &scene, &cam, &s, size);
    let (px, got) = read_pixels(&vp, &ctx).unwrap();
    assert_eq!(got, size);
    // Background: the top-left corner is the gradient's darker top colour,
    // the bottom-left the lighter one.
    let (top, bottom) = (pixel(&px, size, 0, 0), pixel(&px, size, 0, 127));
    assert!(
        top[2] < bottom[2],
        "gradient runs dark to light: {top:?} {bottom:?}"
    );
    // The centre is the box, lit face-on by the headlight: bright and not
    // the background.
    let centre = pixel(&px, size, 64, 64);
    assert_ne!(centre, pixel(&px, size, 64, 0), "the box covers the centre");
    // Looking down −Z, the face under the centre is +Z (face 1).
    let picked = pick(&mut vp, &ctx, &scene, &cam, &s, [64, 64])
        .unwrap()
        .map(|h| h.picked);
    assert_eq!(picked, Some(Picked { body: 0, face: 1 }));
    assert_eq!(
        pick(&mut vp, &ctx, &scene, &cam, &s, [1, 1])
            .unwrap()
            .map(|h| h.picked),
        None,
        "background picks nothing"
    );
}

#[test]
fn edges_and_silhouettes_outline_the_box() {
    let Some(ctx) = gpu() else { return };
    let (scene, mut vp, cam) = setup(&ctx);
    let size = [128, 128];
    // The edge colour (0.02, 0.02, 0.03) linear is (39, 39, 49) in sRGB bytes.
    let dark = |px: &[u8]| {
        px.chunks(4)
            .filter(|c| c[0] < 70 && c[1] < 70 && c[2] < 70)
            .count()
    };
    let mut plain = DisplaySettings::LIGHT;
    plain.edges = false;
    plain.silhouettes = false;
    render(&mut vp, &ctx, &scene, &cam, &plain, size);
    let none = dark(&read_pixels(&vp, &ctx).unwrap().0);
    render(&mut vp, &ctx, &scene, &cam, &DisplaySettings::LIGHT, size);
    let (px, _) = read_pixels(&vp, &ctx).unwrap();
    let outlined = dark(&px);
    if let Some(dir) = std::env::var_os("PXP_TEST_SHOTS") {
        let img = image::RgbaImage::from_raw(size[0], size[1], px.clone()).unwrap();
        img.save(std::path::Path::new(&dir).join("outline.png"))
            .unwrap();
    }
    assert!(
        outlined > none + 200,
        "edges and silhouettes draw an outline ({none} → {outlined} dark pixels)"
    );
}

#[test]
fn a_section_plane_cuts_the_box_away() {
    let Some(ctx) = gpu() else { return };
    let (scene, mut vp, cam) = setup(&ctx);
    let size = [64, 64];
    let mut s = DisplaySettings::LIGHT;
    // Keep x < 110 (the box spans 100..120): the right half disappears.
    s.section = Some(([1.0, 0.0, 0.0], -110.0));
    // 0.65 mm per pixel; the centre pixel (32) is x = 110.
    let right_of_centre = [40, 32]; // x ≈ 115
    let left_of_centre = [24, 32]; // x ≈ 105
    assert_eq!(
        pick(&mut vp, &ctx, &scene, &cam, &s, right_of_centre).unwrap_or(None),
        None
    );
    render(&mut vp, &ctx, &scene, &cam, &s, size);
    assert_eq!(
        pick(&mut vp, &ctx, &scene, &cam, &s, right_of_centre)
            .unwrap()
            .map(|h| h.picked),
        None,
        "cut side is gone"
    );
    // Looking down, the cut leaves the far (−Z) face's inside visible
    // through the opening only where nothing else is: the kept half still
    // picks its top face.
    assert_eq!(
        pick(&mut vp, &ctx, &scene, &cam, &s, left_of_centre)
            .unwrap()
            .map(|h| h.picked),
        Some(Picked { body: 0, face: 1 })
    );
}

#[test]
fn the_orbit_benchmark_runs_and_reports() {
    let Some(ctx) = gpu() else { return };
    trace("orbit benchmark: 20k triangles, 3 frames at 160x120");
    let r = pxp_render::bench::orbit(&ctx, 20_000, 3, [160, 120]).unwrap();
    assert!(
        r.triangles >= 15_000 && r.frames == 3 && r.fps() > 0.0,
        "{r}"
    );
}

#[test]
fn the_selected_face_is_drawn_in_the_selection_colour() {
    let Some(ctx) = gpu() else { return };
    let (scene, mut vp, cam) = setup(&ctx);
    let size = [64, 64];
    let mut s = DisplaySettings::LIGHT;
    s.highlight = Some((0, Some(1)));
    render(&mut vp, &ctx, &scene, &cam, &s, size);
    let (px, _) = read_pixels(&vp, &ctx).unwrap();
    let c = pixel(&px, size, 32, 32);
    assert!(c[0] > 200 && c[2] < 120, "orange at the centre: {c:?}");
    s.highlight = Some((0, Some(0))); // the hidden bottom face
    render(&mut vp, &ctx, &scene, &cam, &s, size);
    let (px, _) = read_pixels(&vp, &ctx).unwrap();
    let c = pixel(&px, size, 32, 32);
    assert!(
        c[0] < 230 || c[2] > 150,
        "the top face is not highlighted: {c:?}"
    );
}

#[test]
fn a_pick_returns_the_point_on_the_face() {
    let Some(ctx) = gpu() else { return };
    let (scene, mut vp, cam) = setup(&ctx);
    let size = [128, 128];
    render(&mut vp, &ctx, &scene, &cam, &DisplaySettings::LIGHT, size);
    let hit = pick(
        &mut vp,
        &ctx,
        &scene,
        &cam,
        &DisplaySettings::LIGHT,
        [64, 64],
    )
    .unwrap()
    .unwrap();
    // Looking down on the box's top face (z = 310) at its centre (110, 215).
    let [x, y, z] = hit.point;
    assert!((z - 310.0).abs() < 0.05, "on the top face: {:?}", hit.point);
    assert!(
        (x - 110.0).abs() < 0.5 && (y - 215.0).abs() < 0.5,
        "under the cursor: {:?}",
        hit.point
    );
    assert_eq!(scene.edges(0).len(), 12, "edges kept for snapping");
}
