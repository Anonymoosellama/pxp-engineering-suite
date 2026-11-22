// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-render: the 3D viewport
//!
//! wgpu rendering of B-rep display meshes with NX-style viewing:
//!
//! | Module | What it does | GPU? |
//! | --- | --- | --- |
//! | [`camera`] | orientation, orbit, pan, zoom about the cursor, fit, standard views (Home = trimetric, End = isometric, F8 = snap), projection matrices | no |
//! | [`mouse`] | the NX mouse grammar (MB2 rotate, MB2+MB3 or Shift+MB2 pan, MB1+MB2 or Ctrl+MB2 zoom, wheel zoom, MB2 click = OK) | no |
//! | [`gpu`] | offscreen viewport: shaded faces, B-rep edges, silhouettes, section plane, GPU id-buffer picking, read-back | yes |
//! | [`snap`] | point snapping for "Specify Point" steps (End, Mid, Existing, Point on Face) | no |
//! | [`bench`](mod@bench) | the generated 5-million-triangle orbit benchmark | yes |
//!
//! The viewport renders into its own textures; the desktop shell shows the
//! colour texture, and tests, screenshots and the benchmark read it back.
//! Without a hardware GPU, [`gpu::GpuContext::headless`] falls back to the
//! OS software rasteriser and [`gpu::GpuContext::software_warning`] says so.
//!
//! ## Must never
//!
//! - Treat a display mesh as geometry: picking reports a body and face,
//!   which the shell maps back to the exact B-rep.
//! - Block the UI thread on a long GPU job other than a single pick.
#![forbid(unsafe_code)]

pub mod bench;
pub mod camera;
pub mod gpu;
pub mod mouse;
pub mod snap;

pub use camera::{Camera, Projection, StandardView};
pub use gpu::{
    COLOR_FORMAT, DisplaySettings, GpuContext, Hit, MeshInput, Picked, RenderError, Scene, Viewport,
};
pub use mouse::{Button, Modifiers, MouseGrammar, ViewAction};

/// Re-export of the wgpu this crate is built on, so the shell shares one
/// version.
pub use wgpu;
