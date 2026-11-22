// SPDX-License-Identifier: LGPL-2.1-or-later
// PXP viewport shaders (WGSL). One uniform block for every pass.
//
// Depth is reversed (near = 1, far = 0, cleared to 0, test Greater), so the
// background has depth 0 and "nearer" means "larger".

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,          // model → camera (for view-space normals)
    section: vec4<f32>,         // plane n·p + d = 0; the side n·p + d > 0 is cut away
    flags: vec4<f32>,           // x: section on, y: silhouettes on, z: edge depth bias, w: unused
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
    edge_color: vec4<f32>,
    cap_color: vec4<f32>,       // back faces seen through the section cut
    viewport: vec4<f32>,        // x: width px, y: height px
    highlight: vec4<u32>,       // pick ids [x, y) drawn in the selection colour
};

@group(0) @binding(0) var<uniform> frame: Frame;

// ---------- background: a vertical gradient on one oversized triangle ----------

struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) t: f32,
};

@vertex
fn bg_vs(@builtin(vertex_index) i: u32) -> BgOut {
    // (−1,−1), (3,−1), (−1,3): covers the screen with one triangle.
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    var o: BgOut;
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    o.t = y * 0.5 + 0.5;
    return o;
}

@fragment
fn bg_fs(in: BgOut) -> @location(0) vec4<f32> {
    return mix(frame.bg_bottom, frame.bg_top, clamp(in.t, 0.0, 1.0));
}

// ---------- shaded faces and pick ids ----------

struct MeshIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) id: u32,
    @location(3) color: vec4<f32>,
};

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal_view: vec3<f32>,
    @location(2) @interpolate(flat) id: u32,
    @location(3) color: vec4<f32>,
};

@vertex
fn mesh_vs(v: MeshIn) -> MeshOut {
    var o: MeshOut;
    o.clip = frame.view_proj * vec4<f32>(v.pos, 1.0);
    o.world = v.pos;
    // The view matrix is a rotation plus translation: its upper 3×3 maps
    // normals correctly.
    o.normal_view = (frame.view * vec4<f32>(v.normal, 0.0)).xyz;
    o.id = v.id;
    o.color = v.color;
    return o;
}

// True where the section plane removes the fragment.
fn cut(world: vec3<f32>) -> bool {
    return frame.flags.x > 0.5 && dot(frame.section.xyz, world) + frame.section.w > 0.0;
}

@fragment
fn mesh_fs(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if (cut(in.world)) {
        discard;
    }
    if (!front) {
        // Inside of a solid: only visible through the section cut, where it
        // reads as the cut face.
        return frame.cap_color;
    }
    // Headlight: light comes from the viewer, plus a little from above.
    let n = normalize(in.normal_view);
    let key = max(n.z, 0.0);
    let fill = max(dot(n, normalize(vec3<f32>(0.3, 0.8, 0.5))), 0.0);
    let spec = pow(key, 40.0) * 0.25;
    let shade = 0.28 + 0.62 * key + 0.15 * fill;
    if (in.id >= frame.highlight.x && in.id < frame.highlight.y) {
        // Selection colour: orange, as in NX; no highlight glare, so the
        // colour stays readable face-on.
        return vec4<f32>(vec3<f32>(1.0, 0.45, 0.05) * min(shade, 1.0), 1.0);
    }
    return vec4<f32>(in.color.rgb * shade + vec3<f32>(spec), in.color.a);
}

struct PickOut {
    @location(0) id: u32,
    // The fragment's depth, in a colour target: depth textures can only be
    // copied whole, colour textures one texel at a time.
    @location(1) depth: f32,
};

@fragment
fn pick_fs(in: MeshOut) -> PickOut {
    if (cut(in.world)) {
        discard;
    }
    var o: PickOut;
    o.id = in.id;
    o.depth = in.clip.z;
    return o;
}

// ---------- B-rep edges, drawn slightly toward the viewer ----------

struct EdgeIn {
    @location(0) pos: vec3<f32>,
};

struct EdgeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn edge_vs(v: EdgeIn) -> EdgeOut {
    var o: EdgeOut;
    var c = frame.view_proj * vec4<f32>(v.pos, 1.0);
    // Reversed depth: nearer is larger. Pull edges forward by a small,
    // perspective-correct amount so they win against their own faces.
    c.z = c.z + frame.flags.z * c.w;
    o.clip = c;
    o.world = v.pos;
    return o;
}

@fragment
fn edge_fs(in: EdgeOut) -> @location(0) vec4<f32> {
    if (cut(in.world)) {
        discard;
    }
    return frame.edge_color;
}

// ---------- silhouettes: depth discontinuities, as a full-screen pass ----------

@group(0) @binding(1) var depth_tex: texture_depth_2d;

@vertex
fn sil_vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn depth_at(p: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(depth_tex));
    let q = clamp(p, vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return textureLoad(depth_tex, q, 0);
}

@fragment
fn sil_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let c = depth_at(p);
    let l = depth_at(p + vec2<i32>(-1, 0));
    let r = depth_at(p + vec2<i32>(1, 0));
    let u = depth_at(p + vec2<i32>(0, -1));
    let d = depth_at(p + vec2<i32>(0, 1));
    // The outline belongs to the nearer side of a jump; background is 0.
    let near_side = c >= max(max(l, r), max(u, d)) - 1e-7;
    let jump = max(max(abs(c - l), abs(c - r)), max(abs(c - u), abs(c - d)));
    // A smooth surface changes depth gradually: compare the jump with the
    // local slope (second difference) so tilted faces do not outline.
    let curve = max(abs(l + r - 2.0 * c), abs(u + d - 2.0 * c));
    let outline = c > 0.0 && near_side && jump > 0.004 && curve > 0.004;
    // Discard inside the branch and end the function with a return. Naga
    // copies a trailing `discard;` into HLSL as the last statement, and the
    // Direct3D compiler (FXC) then refuses the shader with X3507 "not all
    // control paths return a value", which broke every viewport on Windows.
    // Every fragment shader in this file keeps that shape: discard in an
    // `if`, return last.
    if (!outline) {
        discard;
    }
    return frame.edge_color;
}
