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

Current milestone: vector/matrix math and a CPU-owned RGBA8/BGRA8 framebuffer
with PNG export. See subsequent commits for the developing graphics pipeline.

Apache-2.0. Experimental; no Vulkan, OpenGL or SPIR-V compatibility yet.
