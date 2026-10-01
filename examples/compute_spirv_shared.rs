use silicon::api::{Device, Vec4};

fn main() -> silicon::api::Result<()> {
    const ELEMENTS: usize = 4096;
    const LOCAL_SIZE: usize = 64;
    let device = Device::new();
    let pipeline = device.create_compute_pipeline_from_spirv(include_bytes!(
        "../assets/shaders/compute_shared.comp.spv"
    ))?;
    assert_eq!(pipeline.local_size(), [LOCAL_SIZE as u32, 1, 1]);
    assert_eq!(pipeline.shared_memory_vec4s(), LOCAL_SIZE);

    let input = device.create_storage_buffer(
        (0..ELEMENTS)
            .map(|i| Vec4::new(i as f32, i as f32 * 0.5, 1.0 - i as f32, 1.0))
            .collect(),
    )?;
    let mut output = device.create_storage_buffer(vec![Vec4::ZERO; ELEMENTS])?;
    let stats = device.dispatch_compute(
        &pipeline,
        [(ELEMENTS / LOCAL_SIZE) as u32, 1, 1],
        &[&input],
        &mut output,
    )?;

    for (index, &actual) in output.as_slice().iter().enumerate() {
        let first_in_group = index / LOCAL_SIZE * LOCAL_SIZE;
        assert_eq!(actual, input.as_slice()[first_in_group]);
    }
    println!(
        "SILICON SPIR-V compute: {} invocations, {} workgroups, {} instructions; workgroup broadcast verified",
        stats.invocations, stats.workgroups, stats.instructions
    );
    Ok(())
}
