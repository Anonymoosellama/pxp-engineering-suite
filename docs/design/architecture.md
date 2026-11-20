# Design note: workspace architecture and layering

The master prompt's layering, as built in session 1. `tools/layers.toml` is
the machine-readable version and `tools/check_repo.py` enforces it in CI.

```mermaid
flowchart BT
    subgraph L0[Foundation]
        units[pxp-units ✔]
        math[pxp-math ✔]
        diag[pxp-diag ✔]
        compute[pxp-compute ✔]
        platform[pxp-platform]
    end
    subgraph L1[Document core]
        doc[pxp-doc ✔]
    end
    subgraph L2[File format and geometry]
        format[pxp-format]
        kernel[pxp-kernel ✔ trait]
        native[pxp-kernel-native]
        occt[pxp-kernel-occt]
        sketch[pxp-sketch]
        tess[pxp-tess]
    end
    subgraph L3[Shared services]
        appkit[pxp-appkit ✔]
        render[pxp-render]
        io[pxp-io + pxp-io-*]
        domain[pxp-cam, pxp-additive, pxp-machine, pxp-post,<br/>pxp-vnc, pxp-fea, pxp-topopt, pxp-scan, pxp-pointcloud]
    end
    subgraph L4[Applications]
        apps[pxp-app-modeling, -drafting, -manufacturing,<br/>-postgen, -machine-kit, -cae ✔ registered]
    end
    subgraph L5[Shell]
        ui[pxp-ui]
        app[pxp-app]
        cli[pxp-cli ✔]
    end
    L1 --> L0
    L2 --> L1
    L3 --> L2
    L4 --> L3
    L5 --> L4
    native -. implements .-> kernel
    occt -. implements .-> kernel
```

✔ = implemented in session 1 (see `docs/STATUS.md` for scope).

## Rules (enforced)

1. A crate depends only on its own layer or lower layers.
2. Applications never depend on each other; they share domain libraries.
3. Only shell crates name a concrete kernel backend; everything else uses the
   `pxp-kernel` trait through `KernelHost`.
4. Every crate is assigned to exactly one layer.
5. Every crate root has `#![forbid(unsafe_code)]`; future FFI crates are
   listed explicitly in `[unsafe].allow` with an ADR.

## Why the extra crate `pxp-appkit`

The master prompt's `Application` trait mentions `CommandSpec`, `ChangeSet`
and `UiContext`. The document core must not know about UI, and applications
must not depend on each other, so the trait and the registry live in their
own shared-services crate. `UiContext` is a toolkit-neutral description
(tabs, navigators, active application) that `pxp-ui` renders.
