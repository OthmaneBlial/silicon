# SILICON

**A GPU built entirely in software.**

An experimental graphics processor in Rust. This project is being built in
working, tested milestones. The rendering pipeline belongs to SILICON; the host
GPU is never used to produce scene pixels.

## Build

```sh
cargo test --workspace
cargo run --example pixel
```

Current milestone: programmable vertex/fragment stages, six-plane homogeneous
clipping, fixed-point tiled triangle coverage, perspective-correct varyings,
depth, stencil, blending, filtered mipmapped textures and a lit OBJ scene.

```sh
cargo run --release --example showcase
```

![CPU-rendered sculpture scene](assets/screenshots/showcase.png)

Apache-2.0. Experimental; no Vulkan, OpenGL or SPIR-V compatibility yet.
