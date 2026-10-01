use silicon::api::{Device, Vec4};

fn main() -> silicon::api::Result<()> {
    const INVOCATIONS: usize = 64;
    let device = Device::new();
    let pipeline = device.create_compute_pipeline_from_spirv(include_bytes!(
        "../assets/shaders/compute_atomic_uint.comp.spv"
    ))?;
    let mut counters = device.create_storage_buffer(vec![
        Vec4::new(30.0, 2.0, 3.0, 4.0),
        Vec4::new(5.0, 6.0, 7.0, 8.0),
        Vec4::new(7.0, 8.0, 9.0, 10.0),
    ])?;
    let mut output = device.create_storage_buffer(vec![Vec4::ZERO; INVOCATIONS])?;
    let mut atomic_buffers = [&mut counters];
    let stats = device.dispatch_compute_with_atomics(
        &pipeline,
        [INVOCATIONS as u32, 1, 1],
        &[],
        &mut atomic_buffers,
        &mut output,
    )?;

    assert_eq!(counters.as_slice()[0].x, 94.0);
    assert_eq!(counters.as_slice()[1].x, 7.0);
    assert_eq!(counters.as_slice()[2].x, 9.0);
    let mut old_adds: Vec<_> = output.as_slice().iter().map(|value| value.x).collect();
    old_adds.sort_by(f32::total_cmp);
    assert_eq!(
        old_adds,
        (30..94).map(|value| value as f32).collect::<Vec<_>>()
    );
    println!(
        "SILICON SPIR-V compute: {} invocations, {} workgroups; uint add, exchange and compare-exchange verified",
        stats.invocations, stats.workgroups
    );
    Ok(())
}
