# Design note: the geometry kernel contract

Code: `crates/pxp-kernel`. Plan: ADR-0004.

## The contract

```mermaid
flowchart LR
    A[Application recompute] -->|KernelHost::op| H[KernelHost]
    H -->|catch_unwind| K[GeometryKernel backend]
    K --> R[OpResult]
    R --> S[shape: ShapeHandle]
    R --> Y[history: TopoHistory]
    R --> W[warnings]
    H -->|topology of result| K
    H -->|check_complete| Y
    Y -- incomplete --> X[reject: IncompleteHistory, release shape]
    Y -- complete --> P[persistent naming in pxp-doc]
    W --> N[Part Navigator badges]
```

- **Every** topological operation returns `OpResult { shape, history,
  warnings }`.
- `TopoHistory` has one record per output vertex, edge and face: the
  relation (generated or modified) and its sources, which are input entities
  (`Input { operand, entity }`), profile curves (`Profile { curve }`), or
  semantic roles for entities with no input (`Role("block.z_max")`).
  Primitive face roles are part of the naming contract and never change
  after release.
- `KernelHost::op` checks history completeness **before** anyone can store
  the result. Backends that cannot provide history fail loudly instead of
  silently breaking associativity in Drafting, CAM and CAE.
- Backends implement `capabilities()`; the UI greys out what is missing.
  Unsupported operations return `KernelError::Unsupported`, never invented
  geometry.
- Methods take `&self` and the trait is `Send + Sync`, because recompute
  calls the kernel from several threads at once.
- An operation that succeeds but leaves no material (a subtract whose tool
  swallows the target) returns `KernelError::EmptyResult`, never an empty
  body.
- `export_native` / `import_native` move a shape's exact B-rep in and out
  of the backend in its own versioned format. Documents store it wrapped in
  the **stored-shape envelope** (`encode_stored_shape`): `"PXPB"`, envelope
  version, backend id, length-prefixed native bytes. The backend id tells a
  reopened file which kernel wrote the geometry.
- `geometry_hash(shape, resolution)` is platform-independent: coordinates
  are quantised to `resolution` (documents use
  `DEFAULT_HASH_RESOLUTION` = 1 nm) so last-bit differences between
  platform maths libraries do not change it.

## Derived records

`check_complete` requires a record for every vertex, edge and face. A
kernel may report provenance for faces only, or leave new edges without an
origin. A backend may then **derive** the missing records from exact
adjacency, never from geometric guesses, and must document the rules in its
crate docs. The SolidCraft backend (`pxp-kernel-native`, module `history`)
does this:

```mermaid
flowchart TD
    F[SolidCraft FaceProv per face] --> FR[face record: SolidCraft's own origins]
    E[SolidCraft EdgeProv per edge] --> EO{has origins?}
    EO -- input edges --> EM[Modified from those input edges]
    EO -- faces meeting --> EG1[Generated from both sides' input faces]
    EO -- none --> EG2[Generated from the sources of the faces it separates]
    V[B-rep vertex] --> VM{incident edges' input edges share exactly one endpoint at this position?}
    VM -- yes --> VI[Modified from that input vertex]
    VM -- no --> VG[Generated from the sources of the faces around it]
```

A block's top-front edge is therefore "generated from `block.z_max` and
`block.y_min`", and a block corner kept by a boolean is "modified from
vertex 3 of operand 0". `deleted` lists every input face, edge and vertex no
output names. If SolidCraft ever returns a body without history the adapter
returns an empty history and `KernelHost::op` rejects it.

## Backends

| Backend | Crate | Hosting | Status |
| --- | --- | --- | --- |
| SolidCraft kernel (on SolidCore) | `pxp-kernel-native` | in-process (safe Rust) | primitives, booleans, history, hash, native B-rep I/O (session 2) |
| OCCT | `pxp-kernel-occt` | worker process | session 3 |

## Bake-off harness

`pxp_kernel::bakeoff` runs a corpus of cases through any `KernelHost` and
grades each against an answer that never comes from a kernel. Run it with
`pxp-cli bakeoff`; ADR-0004 records the results.

## Hosting

In-process hosting with panic containment exists now, and is what the
safe-Rust SolidCraft backend uses. Native backends (OCCT) can crash the
process in ways `catch_unwind` cannot stop; the OCCT half of sub-stage 0F
(session 3) adds out-of-process hosting: a worker per backend, a versioned
request/response protocol, automatic restart, and replay or rollback of the
in-flight document transaction.

## How persistent naming will use the history (planned)

A persistent name is a chain of steps, one per feature that touched the
entity: `(feature ObjectId, relation, source)`. On recompute, each stored name
is resolved by walking the new history maps from the feature's inputs. If no
entity matches, or several do and no rule disambiguates, the reference
becomes **broken** (shown in the navigator with a repair action), never
re-bound to a guess. Inputs to the design: FreeCAD 1.0's toponaming work
(LGPL-2.0-or-later, study only) and the OCCT `BRepTools_History`
generated/modified/deleted model.
