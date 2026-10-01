# Rust API

`silicon::api` is the supported integration surface for Rust applications.
Its `API_VERSION` is `1`; the crate's root re-exports and lower-level modules
remain available for existing users but may evolve with the implementation.
Incompatible changes to the `silicon::api` surface require an API version
change and release notes.

The API is synchronous and CPU-only. `Device` creates owned buffers, SPIR-V
shader modules and immutable pipelines, records command buffers, and submits
them to a caller-owned `Renderer`. The renderer fixes framebuffer dimensions
and owns the output. Submission structurally validates the command stream
before execution.

The same `Device` also exposes an experimental SIR compute map kernel through
`StorageBuffer`, `StorageLayout`, `ComputePipeline`, `dispatch_compute` and
`dispatch_compute_simd`. The `*_with_layouts` methods apply checked offset/stride
layouts to each input and the output. Dispatch runs one SIR program per 3D
invocation; SIMD uses four-lane SIR execution and a scalar tail. Shader-selected
SIR compute programs also support bounded shader-selected storage loads and
single-writer output stores. Command-buffer integration, shared memory,
barriers, atomics and compute-stage SPIR-V are unsupported. See the
[compute contract](compute.md),
[vector-add example](../examples/compute_vector_add.rs), and
[benchmark example](../examples/compute_bench.rs).

Shader creation accepts at most 1 MiB per SPIR-V 1.0 module and translates only
the documented graphics subset. Pipeline creation requires a compatible
vertex/fragment pair. Vertex buffers are limited to 1M finite vertices, index
buffers to 3M indices, and uniform buffers to 64 finite `Vec4` values. Errors
are returned rather than silently dropping invalid resources or commands.

The runnable [Rust API example](../examples/rust_api.rs) creates shader modules,
links a pipeline, uploads triangle and uniform data, records a draw, submits it,
and saves the resulting framebuffer without opening a window:

```sh
cargo run --release --example rust_api
```

The generated image is `output/rust_api.png`.
