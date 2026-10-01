use silicon::api::{Device, Vec4};

fn main() -> silicon::api::Result<()> {
    const ELEMENTS: usize = 4096;
    let device = Device::new();
    let pipeline = device.create_compute_pipeline_from_spirv(include_bytes!(
        "../assets/shaders/compute_vector_add.comp.spv"
    ))?;
    let a = device.create_storage_buffer(
        (0..ELEMENTS)
            .map(|i| Vec4::new(i as f32, 2.0, 3.0, 4.0))
            .collect(),
    )?;
    let b = device.create_storage_buffer(
        (0..ELEMENTS)
            .map(|i| Vec4::new(5.0, i as f32, 7.0, 8.0))
            .collect(),
    )?;
    let mut output = device.create_storage_buffer(vec![Vec4::ZERO; ELEMENTS])?;

    let stats = device.dispatch_compute(&pipeline, [64, 1, 1], &[&a, &b], &mut output)?;
    for i in 0..ELEMENTS {
        assert_eq!(output.as_slice()[i], a.as_slice()[i] + b.as_slice()[i]);
    }
    println!(
        "SILICON SPIR-V compute: {} invocations, {} workgroups, {} instructions; vector-add verified",
        stats.invocations, stats.workgroups, stats.instructions
    );
    Ok(())
}
