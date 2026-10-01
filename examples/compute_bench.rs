use silicon::api::{Device, Vec4};
use silicon::shader::{Instruction as I, Program};
use std::{hint::black_box, time::Instant};

const ELEMENTS: usize = 65_536;
const SAMPLES: usize = 11;
const SIR_REPEATS: usize = 5;
const CPU_REPEATS: usize = 64;

#[derive(Debug)]
struct Timing {
    median_ms: f64,
    min_ms: f64,
    max_ms: f64,
}

fn measure(mut run: impl FnMut(), repeats: usize) -> Timing {
    run();
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        for _ in 0..repeats {
            run();
        }
        samples.push(start.elapsed().as_secs_f64() * 1000.0 / repeats as f64);
    }
    samples.sort_by(f64::total_cmp);
    Timing {
        median_ms: samples[SAMPLES / 2],
        min_ms: samples[0],
        max_ms: samples[SAMPLES - 1],
    }
}

fn vector_add_program() -> Program {
    Program::new(vec![
        I::Input { dst: 0, slot: 4 },
        I::Input { dst: 1, slot: 5 },
        I::Add { dst: 2, a: 0, b: 1 },
        I::Output { slot: 0, src: 2 },
    ])
    .unwrap()
}

fn matrix_vector_program() -> Program {
    Program::new(vec![
        I::Input { dst: 0, slot: 4 },
        I::Input { dst: 1, slot: 5 },
        I::Input { dst: 2, slot: 6 },
        I::Input { dst: 3, slot: 7 },
        I::Input { dst: 4, slot: 8 },
        I::Dot4 { dst: 5, a: 0, b: 4 },
        I::Dot4 { dst: 6, a: 1, b: 4 },
        I::Dot4 { dst: 7, a: 2, b: 4 },
        I::Dot4 { dst: 8, a: 3, b: 4 },
        I::Compose {
            dst: 9,
            sources: [5, 6, 7, 8],
            lanes: [0; 4],
        },
        I::Output { slot: 0, src: 9 },
    ])
    .unwrap()
}

fn print_timing(name: &str, timing: Timing) {
    println!(
        "{name:<24} median {:>8.3} ms  range {:>8.3}–{:>8.3} ms",
        timing.median_ms, timing.min_ms, timing.max_ms
    );
}

fn main() -> silicon::api::Result<()> {
    let device = Device::new();
    let a: Vec<_> = (0..ELEMENTS)
        .map(|i| {
            let x = (i & 255) as f32 * 0.125;
            Vec4::new(x, x + 1.0, x - 2.0, 1.0)
        })
        .collect();
    let b: Vec<_> = (0..ELEMENTS)
        .map(|i| {
            let x = (i & 127) as f32 * 0.25;
            Vec4::new(x + 3.0, x - 1.0, x, 2.0)
        })
        .collect();
    let rows = [
        Vec4::new(1.25, 0.5, -0.75, 2.0),
        Vec4::new(-0.25, 1.5, 0.125, -1.0),
        Vec4::new(2.0, -0.5, 0.75, 0.25),
        Vec4::new(0.1, 0.2, 0.3, 1.0),
    ];

    let a_buffer = device.create_storage_buffer(a.clone())?;
    let b_buffer = device.create_storage_buffer(b.clone())?;
    let vector_inputs = [&a_buffer, &b_buffer];
    let mut vector_output = device.create_storage_buffer(vec![Vec4::ZERO; ELEMENTS])?;
    let mut vector_simd_output = device.create_storage_buffer(vec![Vec4::ZERO; ELEMENTS])?;
    let vector_pipeline = device.create_compute_pipeline(vector_add_program(), [64, 1, 1])?;
    let groups = [ELEMENTS as u32 / 64, 1, 1];

    let matrix_inputs: Vec<_> = rows
        .iter()
        .map(|row| device.create_storage_buffer(vec![*row; ELEMENTS]))
        .collect::<silicon::api::Result<_>>()?;
    let matrix_buffer_refs: Vec<_> = matrix_inputs.iter().chain([&a_buffer]).collect();
    let matrix_pipeline = device.create_compute_pipeline(matrix_vector_program(), [64, 1, 1])?;
    let mut matrix_output = device.create_storage_buffer(vec![Vec4::ZERO; ELEMENTS])?;
    let mut matrix_simd_output = device.create_storage_buffer(vec![Vec4::ZERO; ELEMENTS])?;

    let mut cpu_vector_output = vec![Vec4::ZERO; ELEMENTS];
    for i in 0..ELEMENTS {
        cpu_vector_output[i] = a[i] + b[i];
    }
    device.dispatch_compute(&vector_pipeline, groups, &vector_inputs, &mut vector_output)?;
    assert_eq!(vector_output.as_slice(), cpu_vector_output);
    device.dispatch_compute_simd(
        &vector_pipeline,
        groups,
        &vector_inputs,
        &mut vector_simd_output,
    )?;
    assert_eq!(vector_simd_output.as_slice(), cpu_vector_output);

    let mut cpu_matrix_output = vec![Vec4::ZERO; ELEMENTS];
    for i in 0..ELEMENTS {
        let v = a[i];
        cpu_matrix_output[i] = Vec4::new(
            rows[0].dot(v),
            rows[1].dot(v),
            rows[2].dot(v),
            rows[3].dot(v),
        );
    }
    device.dispatch_compute(
        &matrix_pipeline,
        groups,
        &matrix_buffer_refs,
        &mut matrix_output,
    )?;
    assert_eq!(matrix_output.as_slice(), cpu_matrix_output);
    device.dispatch_compute_simd(
        &matrix_pipeline,
        groups,
        &matrix_buffer_refs,
        &mut matrix_simd_output,
    )?;
    assert_eq!(matrix_simd_output.as_slice(), cpu_matrix_output);

    println!(
        "SILICON compute benchmark: {ELEMENTS} vec4 elements, {} samples; release mode recommended",
        SAMPLES
    );
    println!("Times are per completed operation; no host GPU is involved.");
    println!("\nVector addition (Rust slice loop vs SIR map):");
    print_timing(
        "Rust CPU reference",
        measure(
            || {
                for i in 0..ELEMENTS {
                    cpu_vector_output[i] = a[i] + b[i];
                }
                black_box(&cpu_vector_output);
            },
            CPU_REPEATS,
        ),
    );
    print_timing(
        "SIR scalar dispatch",
        measure(
            || {
                device
                    .dispatch_compute(&vector_pipeline, groups, &vector_inputs, &mut vector_output)
                    .expect("validated vector-add dispatch");
                black_box(vector_output.as_slice());
            },
            SIR_REPEATS,
        ),
    );
    print_timing(
        "SIR SIMD4 dispatch",
        measure(
            || {
                device
                    .dispatch_compute_simd(
                        &vector_pipeline,
                        groups,
                        &vector_inputs,
                        &mut vector_simd_output,
                    )
                    .expect("validated vector-add SIMD dispatch");
                black_box(vector_simd_output.as_slice());
            },
            SIR_REPEATS,
        ),
    );

    println!("\n4×4 matrix-vector transform (Rust reference vs SIR map):");
    print_timing(
        "Rust CPU reference",
        measure(
            || {
                for i in 0..ELEMENTS {
                    let v = a[i];
                    cpu_matrix_output[i] = Vec4::new(
                        rows[0].dot(v),
                        rows[1].dot(v),
                        rows[2].dot(v),
                        rows[3].dot(v),
                    );
                }
                black_box(&cpu_matrix_output);
            },
            CPU_REPEATS,
        ),
    );
    print_timing(
        "SIR scalar dispatch",
        measure(
            || {
                device
                    .dispatch_compute(
                        &matrix_pipeline,
                        groups,
                        &matrix_buffer_refs,
                        &mut matrix_output,
                    )
                    .expect("validated matrix-vector dispatch");
                black_box(matrix_output.as_slice());
            },
            SIR_REPEATS,
        ),
    );
    print_timing(
        "SIR SIMD4 dispatch",
        measure(
            || {
                device
                    .dispatch_compute_simd(
                        &matrix_pipeline,
                        groups,
                        &matrix_buffer_refs,
                        &mut matrix_simd_output,
                    )
                    .expect("validated matrix-vector SIMD dispatch");
                black_box(matrix_simd_output.as_slice());
            },
            SIR_REPEATS,
        ),
    );
    Ok(())
}
