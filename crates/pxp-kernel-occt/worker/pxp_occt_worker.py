#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-2.1-or-later
"""PXP OCCT worker: Open CASCADE Technology behind PXP's kernel protocol 1.

`pxp-kernel-occt` (Rust) starts this program as a separate process and talks
to it over stdin/stdout, one JSON object per line, so a crash inside OCCT
(a C++ library that can abort the process) never takes PXP down: the client
sees the pipe close, reports the failed call, and starts a new worker.

This worker is the **bake-off worker**: it runs OCCT 7.8 through OCP, the
Python bindings CadQuery maintains (`pip install cadquery-ocp`, Apache-2.0
bindings over LGPL-2.1-with-exception OCCT). It exists to measure OCCT on
the same corpus as SolidCraft (ADR-0004). A shipped OCCT backend would speak
the same protocol from a C++ worker; nothing on the Rust side would change.

Protocol 1
----------

On start the worker prints one line::

    {"v": 1, "hello": {"backend": "occt", "occt": "7.8.1", "worker": "python-ocp"}}

then answers each request line::

    {"v": 1, "id": 7, "op": "block", "args": {...}}

with exactly one response line carrying the same id::

    {"v": 1, "id": 7, "ok": {...}}
    {"v": 1, "id": 7, "err": {"kind": "invalid" | "failure" | "empty" |
                              "unsupported" | "unknown_shape", "message": "..."}}

Shapes live in this process under integer ids; `release` frees one. Topology
is reported in the order of OCCT's indexed maps (``TopExp::MapShapes``) of
vertices, edges, faces and shells, which is also the order every history
record and every index in a request refers to.

History (every primitive and boolean returns one, as ``pxp-kernel``
requires; see ``docs/design/kernel-contract.md``):

* faces of a primitive carry the primitive's role (``block.z_max``, ...),
  classified from the face's geometry in the primitive's own frame;
* faces of a boolean are ``modified`` from the input faces OCCT reports
  (``Modified``, or the same face kept unchanged);
* edges and vertices take OCCT's records where it gives them (kept,
  ``Modified``, ``Generated`` from input faces or edges); any other edge is
  ``generated`` from the sources of the faces it bounds, and any other
  vertex from the sources of the faces around it ("derived records", the
  same rule the SolidCraft adapter uses);
* input entities no output names are ``deleted``.

Geometry hash: FNV-1a 64 over a canonical text of the shape (counts; each
vertex's coordinates quantised to the requested resolution; each edge's
curve type, vertex indices and orientation; each face's surface type and
edge indices; volume and area to 9 significant digits). Equal across runs
and, as far as OCCT's arithmetic is, across operating systems.
"""

import json
import os
import re
import sys
import tempfile
import traceback

from OCP.BRep import BRep_Builder, BRep_Tool
from OCP.BRepAdaptor import BRepAdaptor_Curve, BRepAdaptor_Surface
from OCP.BRepAlgoAPI import BRepAlgoAPI_Common, BRepAlgoAPI_Cut, BRepAlgoAPI_Fuse
from OCP.BRepBndLib import BRepBndLib
from OCP.BRepCheck import BRepCheck_Analyzer, BRepCheck_NoError
from OCP.BRepGProp import BRepGProp
from OCP.BRepPrimAPI import (
    BRepPrimAPI_MakeBox,
    BRepPrimAPI_MakeCone,
    BRepPrimAPI_MakeCylinder,
    BRepPrimAPI_MakeSphere,
)
from OCP.BRepTools import BRepTools
from OCP.Bnd import Bnd_Box
from OCP.GProp import GProp_GProps
from OCP.IFSelect import IFSelect_RetDone
from OCP.Interface import Interface_Static
from OCP.STEPControl import (
    STEPControl_AsIs,
    STEPControl_Controller,
    STEPControl_Reader,
    STEPControl_Writer,
)
from OCP.ShapeUpgrade import ShapeUpgrade_UnifySameDomain
from OCP.TopAbs import (
    TopAbs_EDGE,
    TopAbs_FACE,
    TopAbs_REVERSED,
    TopAbs_SHELL,
    TopAbs_SOLID,
    TopAbs_VERTEX,
)
from OCP.TopExp import TopExp, TopExp_Explorer
from OCP.TopTools import (
    TopTools_IndexedDataMapOfShapeListOfShape,
    TopTools_IndexedMapOfShape,
)
from OCP.TopoDS import TopoDS, TopoDS_Compound, TopoDS_Shape
from OCP.gp import gp_Ax2, gp_Dir, gp_Pnt

PROTOCOL = 1
OCCT_VERSION = "7.8.1"
KINDS = (("v", TopAbs_VERTEX), ("e", TopAbs_EDGE), ("f", TopAbs_FACE))


class Refusal(Exception):
    """An error with a protocol kind (invalid, failure, empty, ...)."""

    def __init__(self, kind, message):
        super().__init__(message)
        self.kind = kind


class Entry:
    """A stored shape with its indexed topology maps (built once)."""

    def __init__(self, shape):
        self.shape = shape
        self.maps = {}
        for code, kind in KINDS + (("s", TopAbs_SHELL),):
            m = TopTools_IndexedMapOfShape()
            TopExp.MapShapes_s(shape, kind, m)
            self.maps[code] = m

    def count(self, code):
        return self.maps[code].Extent()


SHAPES = {}
NEXT = [1]


def store(shape):
    sid = NEXT[0]
    NEXT[0] += 1
    SHAPES[sid] = Entry(shape)
    return sid


def get(sid):
    entry = SHAPES.get(sid)
    if entry is None:
        raise Refusal("unknown_shape", f"no shape {sid} in this worker")
    return entry


def vec(v):
    if not isinstance(v, list) or len(v) != 3 or not all(isinstance(x, (int, float)) for x in v):
        raise Refusal("invalid", f"expected [x, y, z], got {v!r}")
    return [float(x) for x in v]


def positive(name, v):
    if not (isinstance(v, (int, float)) and v > 0 and v < float("inf")):
        raise Refusal("invalid", f"{name} must be positive and finite, got {v}")
    return float(v)


def unit(v):
    n = (v[0] ** 2 + v[1] ** 2 + v[2] ** 2) ** 0.5
    if n < 1e-12:
        raise Refusal("invalid", "zero-length direction")
    return [v[0] / n, v[1] / n, v[2] / n]


def cross(a, b):
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def perpendicular(axis):
    """A unit vector perpendicular to `axis` by a fixed rule (the native
    backend's rule: project the coordinate axis least aligned with it)."""
    seed = [1.0, 0.0, 0.0] if abs(axis[0]) < 0.9 else [0.0, 1.0, 0.0]
    d = dot(seed, axis)
    return unit([seed[i] - d * axis[i] for i in range(3)])


def centroid(face):
    p = GProp_GProps()
    BRepGProp.SurfaceProperties_s(face, p)
    c = p.CentreOfMass()
    return [c.X(), c.Y(), c.Z()]


def surface_kind(face):
    return BRepAdaptor_Surface(TopoDS.Face_s(face)).GetType().name


# ---- history ---------------------------------------------------------------


def ancestors(shape, kind, of):
    m = TopTools_IndexedDataMapOfShapeListOfShape()
    TopExp.MapShapesAndAncestors_s(shape, kind, of, m)
    return m


def derive(entry, faces, code, kind, records):
    """Fills missing edge or vertex records from the sources of the faces
    around them (derived records)."""
    adj = ancestors(entry.shape, kind, TopAbs_FACE)
    fmap = entry.maps["f"]
    for i in range(entry.count(code)):
        if records[i] is not None:
            continue
        sub = entry.maps[code].FindKey(i + 1)
        sources = []
        j = adj.FindIndex(sub)
        if j > 0:
            idxs = sorted(fmap.FindIndex(f) - 1 for f in adj.FindFromIndex(j))
            for fi in idxs:
                for s in faces[fi][1]:
                    if s not in sources:
                        sources.append(s)
        records[i] = ["g", sources]


def pack(faces, edges, vertices, deleted):
    out = []
    for code, recs in (("v", vertices), ("e", edges), ("f", faces)):
        for i, (rel, sources) in enumerate(recs):
            out.append([code, i, rel, sources])
    return {"records": out, "deleted": deleted}


def primitive_history(entry, roles):
    if len(roles) != entry.count("f"):
        raise Refusal("failure", f"{len(roles)} roles for {entry.count('f')} faces")
    faces = [["g", [["r", r]]] for r in roles]
    edges = [None] * entry.count("e")
    vertices = [None] * entry.count("v")
    derive(entry, faces, "e", TopAbs_EDGE, edges)
    derive(entry, faces, "v", TopAbs_VERTEX, vertices)
    return pack(faces, edges, vertices, [])


def operation_history(algo, inputs, entry):
    recs = {code: [None] * entry.count(code) for code, _ in KINDS}
    named = set()

    def add(code, j, rel, source):
        cur = recs[code][j]
        if cur is None:
            recs[code][j] = [rel, [source]]
        else:
            if source not in cur[1]:
                cur[1].append(source)
            if rel == "m":
                cur[0] = "m"

    for operand, inp in enumerate(inputs):
        for code, kind in KINDS:
            imap = inp.maps[code]
            for i in range(1, imap.Extent() + 1):
                s = imap.FindKey(i)
                src = ["i", operand, code, i - 1]
                if not algo.IsDeleted(s):
                    mod = algo.Modified(s)
                    outs = list(mod) if mod.Size() > 0 else [s]
                    for o in outs:
                        j = entry.maps[code].FindIndex(o)
                        if j > 0:
                            add(code, j - 1, "m", src)
                            named.add((operand, code, i - 1))
                for g in algo.Generated(s):
                    for gcode, gkind in KINDS:
                        if g.ShapeType() == gkind:
                            j = entry.maps[gcode].FindIndex(g)
                            if j > 0:
                                add(gcode, j - 1, "g", src)
                                named.add((operand, code, i - 1))
    faces = recs["f"]
    for i, r in enumerate(faces):
        if r is None:
            raise Refusal("failure", f"OCCT reported no origin for result face {i}")
    derive(entry, faces, "e", TopAbs_EDGE, recs["e"])
    derive(entry, faces, "v", TopAbs_VERTEX, recs["v"])
    deleted = []
    for operand, inp in enumerate(inputs):
        for code, _ in KINDS:
            for i in range(inp.count(code)):
                if (operand, code, i) not in named:
                    deleted.append([operand, code, i])
    return pack(faces, recs["e"], recs["v"], deleted)


# ---- operations ------------------------------------------------------------


def op_block(a):
    csys = a.get("csys") or {}
    origin = vec(csys.get("origin", [0, 0, 0]))
    x = unit(vec(csys.get("x", [1, 0, 0])))
    y0 = vec(csys.get("y", [0, 1, 0]))
    z = unit(cross(x, y0))
    y = cross(z, x)
    size = [positive("block size", s) for s in vec(a.get("size"))]
    shape = BRepPrimAPI_MakeBox(gp_Ax2(gp_Pnt(*origin), gp_Dir(*z), gp_Dir(*x)), *size).Shape()
    sid = store(shape)
    entry = get(sid)
    roles = []
    for i in range(entry.count("f")):
        c = centroid(entry.maps["f"].FindKey(i + 1))
        local = [dot([c[k] - origin[k] for k in range(3)], ax) for ax in (x, y, z)]
        role = None
        for axis, name in enumerate(("x", "y", "z")):
            if abs(local[axis]) < 1e-9 * size[axis]:
                role = f"block.{name}_min"
            elif abs(local[axis] - size[axis]) < 1e-9 * size[axis]:
                role = f"block.{name}_max"
        if role is None:
            raise Refusal("failure", f"cannot classify box face {i}")
        roles.append(role)
    return {"shape": sid, "history": primitive_history(entry, roles)}


def axial_roles(entry, base, axis, side_kinds, prefix, has_top=True):
    roles = []
    for i in range(entry.count("f")):
        f = entry.maps["f"].FindKey(i + 1)
        kind = surface_kind(f)
        if kind in side_kinds:
            roles.append(f"{prefix}.side" if prefix != "sphere" else "sphere.surface")
        elif kind == "GeomAbs_Plane":
            t = dot([centroid(f)[k] - base[k] for k in range(3)], axis)
            roles.append(f"{prefix}.top" if (t > 1e-9 and has_top) else f"{prefix}.bottom")
        else:
            raise Refusal("failure", f"unexpected {kind} face in a {prefix}")
    return roles


def op_cylinder(a):
    base = vec(a.get("base"))
    axis = unit(vec(a.get("axis")))
    r = positive("cylinder diameter", a.get("diameter")) / 2.0
    h = positive("cylinder height", a.get("height"))
    ax2 = gp_Ax2(gp_Pnt(*base), gp_Dir(*axis), gp_Dir(*perpendicular(axis)))
    shape = BRepPrimAPI_MakeCylinder(ax2, r, h).Shape()
    sid = store(shape)
    entry = get(sid)
    roles = axial_roles(entry, base, axis, ("GeomAbs_Cylinder",), "cylinder")
    return {"shape": sid, "history": primitive_history(entry, roles)}


def op_cone(a):
    base = vec(a.get("base"))
    axis = unit(vec(a.get("axis")))
    r1 = positive("cone base diameter", a.get("base_diameter")) / 2.0
    top = a.get("top_diameter")
    if not (isinstance(top, (int, float)) and top >= 0):
        raise Refusal("invalid", f"cone top diameter must be non-negative, got {top}")
    r2 = float(top) / 2.0
    h = positive("cone height", a.get("height"))
    if abs(r1 - r2) < 1e-12:
        raise Refusal("invalid", "cone diameters are equal (that is a cylinder)")
    ax2 = gp_Ax2(gp_Pnt(*base), gp_Dir(*axis), gp_Dir(*perpendicular(axis)))
    shape = BRepPrimAPI_MakeCone(ax2, r1, r2, h).Shape()
    sid = store(shape)
    entry = get(sid)
    roles = axial_roles(entry, base, axis, ("GeomAbs_Cone",), "cone", has_top=r2 > 0)
    return {"shape": sid, "history": primitive_history(entry, roles)}


def op_sphere(a):
    c = vec(a.get("center"))
    r = positive("sphere diameter", a.get("diameter")) / 2.0
    shape = BRepPrimAPI_MakeSphere(gp_Pnt(*c), r).Shape()
    sid = store(shape)
    entry = get(sid)
    roles = ["sphere.surface"] * entry.count("f")
    return {"shape": sid, "history": primitive_history(entry, roles)}


def has_material(shape):
    e = TopExp_Explorer(shape, TopAbs_SOLID)
    if e.More():
        return True
    return TopExp_Explorer(shape, TopAbs_FACE).More()


def op_boolean(a):
    target, tool = get(a.get("target")), get(a.get("tool"))
    algo = {"unite": BRepAlgoAPI_Fuse, "subtract": BRepAlgoAPI_Cut, "intersect": BRepAlgoAPI_Common}.get(a.get("op"))
    if algo is None:
        raise Refusal("invalid", f"unknown boolean {a.get('op')!r}")
    b = algo(target.shape, tool.shape)
    if not b.IsDone():
        raise Refusal("failure", "OCCT boolean did not complete")
    result = b.Shape()
    if result.IsNull() or not has_material(result):
        raise Refusal("empty", "boolean left no material")
    sid = store(result)
    entry = get(sid)
    if entry.count("f") == 0:
        del SHAPES[sid]
        raise Refusal("empty", "boolean left no faces")
    return {"shape": sid, "history": operation_history(b, [target, tool], entry)}


def op_topology(a):
    e = get(a.get("shape"))
    return {code: e.count(code) for code in ("v", "e", "f", "s")}


def op_body_kind(a):
    s = get(a.get("shape")).shape
    if TopExp_Explorer(s, TopAbs_SOLID).More():
        return {"kind": "solid"}
    if TopExp_Explorer(s, TopAbs_FACE).More():
        return {"kind": "sheet"}
    return {"kind": "wire"}


def mass(shape):
    vp, sp = GProp_GProps(), GProp_GProps()
    BRepGProp.SurfaceProperties_s(shape, sp)
    volume = 0.0
    c = sp.CentreOfMass()
    if TopExp_Explorer(shape, TopAbs_SOLID).More():
        BRepGProp.VolumeProperties_s(shape, vp)
        volume = vp.Mass()
        c = vp.CentreOfMass()
    return volume, sp.Mass(), [c.X(), c.Y(), c.Z()]


def op_mass(a):
    v, ar, c = mass(get(a.get("shape")).shape)
    return {"volume": v, "area": ar, "centroid": c}


def op_bbox(a):
    b = Bnd_Box()
    BRepBndLib.AddOptimal_s(get(a.get("shape")).shape, b, False, False)
    if b.IsVoid():
        raise Refusal("failure", "empty bounding box")
    x0, y0, z0, x1, y1, z1 = b.Get()
    return {"min": [x0, y0, z0], "max": [x1, y1, z1]}


def fnv(text):
    h = 0xCBF29CE484222325
    for byte in text.encode("utf-8"):
        h ^= byte
        h = (h * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return h


def sig(x):
    return f"{x:.9g}"


def op_hash(a):
    e = get(a.get("shape"))
    res = positive("hash resolution", a.get("resolution"))
    q = lambda x: str(round(x / res))  # noqa: E731 - quantise to the resolution
    parts = [f"{e.count('v')} {e.count('e')} {e.count('f')} {e.count('s')}"]
    vmap = e.maps["v"]
    for i in range(1, vmap.Extent() + 1):
        p = BRep_Tool.Pnt_s(TopoDS.Vertex_s(vmap.FindKey(i)))
        parts.append("v " + " ".join(q(c) for c in (p.X(), p.Y(), p.Z())))
    emap = e.maps["e"]
    for i in range(1, emap.Extent() + 1):
        edge = TopoDS.Edge_s(emap.FindKey(i))
        first, last = TopExp.FirstVertex_s(edge), TopExp.LastVertex_s(edge)
        kind = "degenerate" if BRep_Tool.Degenerated_s(edge) else BRepAdaptor_Curve(edge).GetType().name
        parts.append(f"e {kind} {vmap.FindIndex(first)} {vmap.FindIndex(last)}")
    fmap = e.maps["f"]
    for i in range(1, fmap.Extent() + 1):
        face = fmap.FindKey(i)
        edges = []
        ex = TopExp_Explorer(face, TopAbs_EDGE)
        while ex.More():
            edges.append(str(emap.FindIndex(ex.Current())) + ("r" if ex.Current().Orientation() == TopAbs_REVERSED else ""))
            ex.Next()
        parts.append(f"f {surface_kind(face)} {'r' if face.Orientation() == TopAbs_REVERSED else 'f'} " + ",".join(edges))
    v, ar, _ = mass(e.shape)
    parts.append(f"m {sig(v)} {sig(ar)}")
    return {"hash": f"{fnv(chr(10).join(parts)):016x}"}


def op_merged(a):
    s = get(a.get("shape")).shape
    u = ShapeUpgrade_UnifySameDomain(s, True, True, False)
    u.Build()
    merged = Entry(u.Shape())
    return {"faces": merged.count("f"), "edges": merged.count("e"), "vertices": merged.count("v")}


def op_export_native(a):
    s = get(a.get("shape")).shape
    fd, path = tempfile.mkstemp(suffix=".brep")
    os.close(fd)
    try:
        if not BRepTools.Write_s(s, path):
            raise Refusal("failure", "BRepTools::Write failed")
        with open(path, encoding="ascii") as f:
            return {"brep": f.read()}
    finally:
        os.unlink(path)


def op_import_native(a):
    text = a.get("brep")
    if not isinstance(text, str) or not text.startswith("DBRep_DrawableShape") and "CASCADE Topology" not in text[:200]:
        raise Refusal("invalid", "not an OCCT BRep text")
    fd, path = tempfile.mkstemp(suffix=".brep")
    try:
        with os.fdopen(fd, "w", encoding="ascii") as f:
            f.write(text)
        s = TopoDS_Shape()
        if not BRepTools.Read_s(s, path, BRep_Builder()) or s.IsNull():
            raise Refusal("invalid", "BRepTools::Read could not read the bytes")
        return {"shape": store(s)}
    finally:
        os.unlink(path)


def closed_solid(shape):
    if shape.ShapeType() != TopAbs_SOLID:
        return False
    ex = TopExp_Explorer(shape, TopAbs_SHELL)
    any_shell = False
    while ex.More():
        any_shell = True
        if not BRep_Tool.IsClosed_s(ex.Current()):
            return False
        ex.Next()
    return any_shell


def is_mesh(shape):
    ex = TopExp_Explorer(shape, TopAbs_FACE)
    while ex.More():
        if BRep_Tool.Surface_s(TopoDS.Face_s(ex.Current())) is not None:
            return False
        ex.Next()
    return True


def op_import_step(a):
    text = a.get("text")
    if not isinstance(text, str):
        raise Refusal("invalid", "STEP text expected")
    fd, path = tempfile.mkstemp(suffix=".stp")
    try:
        with os.fdopen(fd, "w", encoding="latin-1", errors="replace") as f:
            f.write(text)
        r = STEPControl_Reader()
        if r.ReadFile(path) != IFSelect_RetDone:
            raise Refusal("invalid", "OCCT could not read the STEP file")
        r.TransferRoots()
        whole = r.OneShape()
    finally:
        os.unlink(path)
    if whole.IsNull():
        raise Refusal("failure", "STEP import: no shape")
    m = re.search(r"FILE_SCHEMA\s*\(\s*\(\s*'([^']*)'", text)
    shapes, warnings = [], []
    ex = TopExp_Explorer(whole, TopAbs_SOLID)
    while ex.More():
        s = ex.Current()
        shapes.append({"shape": store(s), "name": "", "closed": closed_solid(s), "mesh": is_mesh(s)})
        ex.Next()
    ex = TopExp_Explorer(whole, TopAbs_SHELL, TopAbs_SOLID)
    while ex.More():
        s = ex.Current()
        shapes.append({"shape": store(s), "name": "", "closed": False, "mesh": is_mesh(s)})
        ex.Next()
    if not shapes:
        raise Refusal("failure", "STEP import: no solids or shells")
    solids = sum(1 for s in shapes if s["closed"])
    return {"shapes": shapes, "parts": len(shapes), "closed_parts": solids,
            "schema": m.group(1) if m else "", "warnings": warnings}


def op_export_step(a):
    ids = a.get("shapes")
    if not isinstance(ids, list) or not ids:
        raise Refusal("invalid", "shapes to export expected")
    # The schema switch exists only once the STEP controller is registered.
    STEPControl_Controller.Init_s()
    Interface_Static.SetCVal_s("write.step.schema", "AP242DIS")
    w = STEPControl_Writer()
    for sid in ids:
        w.Transfer(get(sid).shape, STEPControl_AsIs)
    fd, path = tempfile.mkstemp(suffix=".stp")
    os.close(fd)
    try:
        if w.Write(path) != IFSelect_RetDone:
            raise Refusal("failure", "OCCT could not write STEP")
        with open(path, encoding="latin-1") as f:
            text = f.read()
    finally:
        os.unlink(path)
    # No time stamp: identical shapes give identical text.
    text = re.sub(r"(FILE_NAME\s*\(\s*'[^']*'\s*,\s*)'[^']*'", r"\1''", text, count=1)
    # OCCT names products after a counter that grows through the process's
    # life ("Open CASCADE STEP translator 7.8 3"); name them body-1, body-2,
    # ... in order of appearance, as the native writer does.
    names = []
    for m in re.finditer(r"Open CASCADE STEP translator [0-9.]+ [0-9]+", text):
        if m.group(0) not in names:
            names.append(m.group(0))
    for i, n in enumerate(names, 1):
        text = text.replace("'" + n + "'", f"'body-{i}'")
    return {"text": text}


def op_check(a):
    s = get(a.get("shape")).shape
    an = BRepCheck_Analyzer(s)
    if an.IsValid():
        return {"failed": []}
    counts = {}
    for code, kind in KINDS + (("s", TopAbs_SHELL),):
        m = get(a.get("shape")).maps[code]
        for i in range(1, m.Extent() + 1):
            res = an.Result(m.FindKey(i))
            if res is None:
                continue
            for st in res.Status():
                if st != BRepCheck_NoError:
                    counts[st.name] = counts.get(st.name, 0) + 1
    failed = [f"{name} (×{n})" for name, n in sorted(counts.items())] or ["BRepCheck_Analyzer: invalid"]
    return {"failed": failed}


def op_test_abort(a):
    """Ends the process at once, as an OCCT abort would: lets the client's
    tests prove that a dead worker costs one call and nothing else."""
    os._exit(70)


def op_release(a):
    SHAPES.pop(a.get("shape"), None)
    return {}


OPS = {
    "block": op_block,
    "cylinder": op_cylinder,
    "cone": op_cone,
    "sphere": op_sphere,
    "boolean": op_boolean,
    "topology": op_topology,
    "body_kind": op_body_kind,
    "mass_properties": op_mass,
    "bounding_box": op_bbox,
    "geometry_hash": op_hash,
    "merged_counts": op_merged,
    "export_native": op_export_native,
    "import_native": op_import_native,
    "import_step": op_import_step,
    "export_step": op_export_step,
    "check_validity": op_check,
    "release": op_release,
    "test_abort": op_test_abort,
}


# The protocol owns the process's original stdout. OCCT's C++ code prints
# progress and statistics to file descriptor 1 (the STEP writer does), which
# would corrupt the stream, so descriptor 1 is pointed at stderr and replies
# go to a private duplicate of the original stdout.
PROTOCOL_OUT = None


def reply(msg):
    PROTOCOL_OUT.write(json.dumps(msg, separators=(",", ":")) + "\n")
    PROTOCOL_OUT.flush()


def main():
    global PROTOCOL_OUT
    sys.stdout.flush()
    PROTOCOL_OUT = os.fdopen(os.dup(1), "w", encoding="utf-8")
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    reply({"v": PROTOCOL, "hello": {"backend": "occt", "occt": OCCT_VERSION, "worker": "python-ocp", "ops": sorted(OPS)}})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        rid = None
        try:
            req = json.loads(line)
            rid = req.get("id")
            if req.get("v") != PROTOCOL:
                raise Refusal("invalid", f"protocol {req.get('v')} not supported (this worker speaks {PROTOCOL})")
            fn = OPS.get(req.get("op"))
            if fn is None:
                raise Refusal("unsupported", f"unknown op {req.get('op')!r}")
            reply({"v": PROTOCOL, "id": rid, "ok": fn(req.get("args") or {})})
        except Refusal as r:
            reply({"v": PROTOCOL, "id": rid, "err": {"kind": r.kind, "message": str(r)}})
        except Exception as ex:  # an OCCT exception is a failure of this call, not of the worker
            reply({"v": PROTOCOL, "id": rid, "err": {"kind": "failure", "message": f"{type(ex).__name__}: {ex}",
                                                       "trace": traceback.format_exc(limit=3)}})


if __name__ == "__main__":
    main()
