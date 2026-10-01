use silicon::api::{Device, Vec4};

fn main() -> silicon::api::Result<()> {
    const ELEMENTS: usize = 4100;
    let device = Device::new();
    let pipeline = device.create_compute_pipeline_from_spirv(include_bytes!(
        "../assets/shaders/compute_invert.comp.spv"
    ))?;
    let input = device.create_storage_buffer(
        (0..ELEMENTS)
            .map(|i| {
                let value = i as f32 / ELEMENTS as f32;
                Vec4::new(value, value * 0.5, 1.0 - value, 1.0)
            })
            .collect(),
    )?;
    let mut output = device.create_storage_buffer(vec![Vec4::ZERO; ELEMENTS])?;

    let stats = device.dispatch_compute(&pipeline, [65, 1, 1], &[&input], &mut output)?;
    assert_eq!(stats.invocations, 4160);
    assert_eq!(stats.workgroups, 65);
    for (&actual, &source) in output.as_slice().iter().zip(input.as_slice()) {
        assert_eq!(actual, Vec4::new(1.0, 1.0, 1.0, 1.0) - source);
    }
    println!(
        "SILICON SPIR-V compute: {} invocations, {} workgroups, {} instructions; vec4 inversion verified",
        stats.invocations, stats.workgroups, stats.instructions
    );
    Ok(())
}
