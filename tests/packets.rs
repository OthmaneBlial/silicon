use shader::{Instruction::*, Program, spirv::Module};
use silicon::*;
fn bits(v: Vec4) -> [u32; 4] {
    v.to_array().map(f32::to_bits)
}
#[test]
fn masked_packets_match_scalar_values_traces_and_samples() {
    let programs = [
        Module::parse(include_bytes!("../assets/shaders/lit.frag.spv"))
            .unwrap()
            .translate()
            .unwrap()
            .program,
        Module::parse(include_bytes!("../assets/shaders/locals.frag.spv"))
            .unwrap()
            .translate()
            .unwrap()
            .program,
        Program::new(vec![
            Input { dst: 0, slot: 0 },
            Input { dst: 1, slot: 1 },
            Const {
                dst: 2,
                value: Vec4::new(1., 1., 1., 1.),
            },
            Add { dst: 3, a: 0, b: 2 },
            Sub { dst: 4, a: 3, b: 2 },
            Mul { dst: 5, a: 4, b: 2 },
            Div { dst: 6, a: 5, b: 2 },
            Normalize3 { dst: 7, src: 6 },
            Saturate { dst: 8, src: 7 },
            Dot4 { dst: 9, a: 7, b: 8 },
            shader::Instruction::Mat4 {
                dst: 10,
                src: 6,
                uniform: 0,
            },
            Swizzle {
                dst: 11,
                src: 10,
                lanes: [2, 0, 3, 1],
            },
            Sample {
                dst: 12,
                uv: 1,
                texture: 0,
            },
            Compose {
                dst: 13,
                sources: [11, 9, 12, 8],
                lanes: [0, 1, 2, 3],
            },
            Normalize {
                dst: 14,
                src: 0,
                components: 1,
            },
            Length {
                dst: 15,
                src: 0,
                components: 1,
            },
            Output { slot: 1, src: 14 },
            Output { slot: 2, src: 15 },
            Output { slot: 0, src: 13 },
        ])
        .unwrap(),
    ];
    let inputs: [[Vec4; 4]; 4] = std::array::from_fn(|i| {
        [
            Vec4::new(-0.2 + i as f32 * 0.1, 0.4, 0.8, 1.),
            Vec4::new(0.1, 0.3 + i as f32 * 0.1, 2., 0.),
            if i == 0 {
                Vec4::ZERO
            } else {
                Vec4::new(0.1, 0.8, 0.4, 99.)
            },
            Vec4::new(i as f32, 1., -0.5, 23.),
        ]
    });
    let mut uniforms = vec![Vec4::ZERO; 33];
    for i in 0..4 {
        let mut row = [0.; 4];
        row[i] = 1.;
        uniforms[i] = Vec4::from_array(row);
    }
    uniforms[12] = Vec4::new(0.7, 0.8, 0.9, 1.);
    uniforms[16] = Vec4::new(1., 0.4, 0.2, 0.);
    uniforms[20] = Vec4::new(4., 3., 5., 1.);
    uniforms[24] = Vec4::new(0.1, 0.3, 91., 93.);
    uniforms[28] = Vec4::new(2., 95., 97., 99.);
    uniforms[32] = Vec4::new(0.2, 0.4, 0.6, 101.);
    let lods: [[f32; 1]; 4] = std::array::from_fn(|i| [i as f32 * 0.5]);
    for p in programs {
        for mask in 0..16 {
            let mut calls = Vec::new();
            let e = p
                .execute4(
                    std::array::from_fn(|i| {
                        if mask & (1 << i) != 0 {
                            inputs[i].as_slice()
                        } else {
                            &[]
                        }
                    }),
                    &uniforms,
                    std::array::from_fn(|i| {
                        if mask & (1 << i) != 0 {
                            lods[i].as_slice()
                        } else {
                            &[]
                        }
                    }),
                    mask,
                    |i, slot, uv| {
                        calls.push((i, slot, bits(uv)));
                        Ok(Vec4::new(uv.x, uv.y, 0.5, 1.))
                    },
                    [true; 4],
                )
                .unwrap();
            for i in 0..4 {
                if mask & (1 << i) == 0 {
                    assert_eq!(e[i].instructions, 0);
                    assert!(e[i].trace.is_empty());
                    continue;
                }
                let mut scalar_calls = Vec::new();
                let scalar = p
                    .execute_with_lod(
                        &inputs[i],
                        &uniforms,
                        &lods[i],
                        |slot, uv| {
                            scalar_calls.push((i, slot, bits(uv)));
                            Ok(Vec4::new(uv.x, uv.y, 0.5, 1.))
                        },
                        true,
                    )
                    .unwrap();
                assert_eq!(e[i].outputs.map(bits), scalar.outputs.map(bits));
                assert_eq!(e[i].samples, scalar.samples);
                assert_eq!(e[i].instructions, scalar.instructions);
                assert_eq!(e[i].trace.len(), scalar.trace.len());
                for (actual, expected) in e[i].trace.iter().zip(&scalar.trace) {
                    assert_eq!(actual.instruction, expected.instruction);
                    assert_eq!(
                        bits(actual.value),
                        bits(expected.value),
                        "PC {} lane {i}",
                        actual.instruction
                    );
                }
                assert_eq!(
                    calls
                        .iter()
                        .filter(|(lane, _, _)| *lane == i)
                        .cloned()
                        .collect::<Vec<_>>(),
                    scalar_calls
                );
            }
        }
    }
}
#[test]
fn packet_masks_and_resources_fail_without_reading_inactive_lanes() {
    let copy = Program::new(vec![Input { dst: 0, slot: 0 }, Output { slot: 0, src: 0 }]).unwrap();
    let mut values = [[Vec4::new(-0., f32::from_bits(1), f32::MAX, -f32::MAX)]; 4];
    copy.execute4(
        values.each_ref().map(|v| v.as_slice()),
        &[],
        [&[]; 4],
        15,
        |_, _, _| unreachable!(),
        [false; 4],
    )
    .unwrap();
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for i in 0..4 {
            for c in 0..4 {
                let saved = values[i][0];
                let mut v = saved.to_array();
                v[c] = invalid;
                values[i][0] = Vec4::from_array(v);
                let inputs = values.each_ref().map(|v| v.as_slice());
                assert!(
                    copy.execute4(
                        inputs,
                        &[],
                        [&[]; 4],
                        15,
                        |_, _, _| unreachable!(),
                        [false; 4]
                    )
                    .unwrap_err()
                    .contains(&format!("lane {i}"))
                );
                copy.execute4(
                    inputs,
                    &[],
                    [&[]; 4],
                    15 & !(1 << i),
                    |_, _, _| unreachable!(),
                    [false; 4],
                )
                .unwrap();
                values[i][0] = saved;
            }
        }
    }
    let p = Program::new(vec![
        Input { dst: 0, slot: 0 },
        SampleImplicit {
            dst: 1,
            uv: 0,
            texture: 0,
        },
        Output { slot: 0, src: 1 },
    ])
    .unwrap();
    let input = [Vec4::new(0.2, 0.4, 0., 0.)];
    let empty: [&[Vec4]; 4] = [&[]; 4];
    let empty_lods: [&[f32]; 4] = [&[]; 4];
    assert!(
        p.execute4(
            empty,
            &[],
            empty_lods,
            16,
            |_, _, _| unreachable!(),
            [false; 4]
        )
        .is_err()
    );
    p.execute4(
        empty,
        &[],
        empty_lods,
        0,
        |_, _, _| unreachable!(),
        [true; 4],
    )
    .unwrap();
    assert!(
        p.execute4(
            [&input, &[], &[], &[]],
            &[],
            empty_lods,
            1,
            |_, _, _| unreachable!(),
            [false; 4]
        )
        .unwrap_err()
        .contains("missing implicit LOD")
    );
    let lod = [0.];
    let result = p
        .execute4(
            [&input, &[], &[], &[]],
            &[],
            [&lod, &[], &[], &[]],
            1,
            |i, _, _| {
                assert_eq!(i, 0);
                Ok(Vec4::new(1., 1., 1., 1.))
            },
            [false; 4],
        )
        .unwrap();
    assert_eq!(result[0].samples, 1);
    let nan = [f32::NAN];
    assert!(
        p.execute4(
            [&input, &[], &[], &[]],
            &[],
            [&nan, &[], &[], &[]],
            1,
            |_, _, _| unreachable!(),
            [false; 4]
        )
        .is_err()
    );
    assert!(
        p.execute4(
            [&input, &[], &[], &[]],
            &[],
            [&lod, &[], &[], &[]],
            1,
            |_, _, _| Ok(Vec4::new(f32::NAN, 0., 0., 1.)),
            [false; 4]
        )
        .is_err()
    );
}
#[test]
fn recorded_shader_packets_preserve_attachments_and_pixel_debugging() {
    for scene in ["shader_cube", "spirv_cube", "spirv_showcase"] {
        let capture = match scene {
            "shader_cube" => demo::shader_cube(97, 65, 0.37),
            "spirv_cube" => demo::spirv_cube(97, 65, 0.37),
            _ => demo::spirv_showcase(97, 65, 0.37),
        }
        .unwrap();
        let mut scalar = Renderer::new(97, 65).unwrap();
        scalar.debug_pixel = Some((48, 32));
        let reference = Device.submit(&capture.commands, &mut scalar).unwrap();
        assert!(!reference.shader_traces.is_empty());
        for workers in [1, 4] {
            let mut actual = Renderer::new(97, 65).unwrap();
            actual.backend = Backend::Simd;
            actual.debug_pixel = Some((48, 32));
            if workers == 1 {
                let submitted = Device.submit(&capture.commands, &mut actual).unwrap();
                let traces = |s: &Submission| {
                    s.shader_traces
                        .iter()
                        .map(|(primitive, trace)| {
                            (
                                *primitive,
                                trace
                                    .iter()
                                    .map(|t| (t.instruction, bits(t.value)))
                                    .collect::<Vec<_>>(),
                            )
                        })
                        .collect::<Vec<_>>()
                };
                assert_eq!(traces(&submitted), traces(&reference));
            } else {
                actual
                    .render_bands(workers, |r| Device.submit(&capture.commands, r).map(|_| ()))
                    .unwrap();
            }
            assert_eq!(actual.framebuffer.bytes(), scalar.framebuffer.bytes());
            assert!(actual.stats.shader_packets > 0);
            assert!(actual.stats.shaded <= actual.stats.shader_packets * 4);
            for y in 0..65 {
                for x in 0..97 {
                    assert_eq!(
                        actual.framebuffer.depth_at(x, y),
                        scalar.framebuffer.depth_at(x, y)
                    );
                    assert_eq!(
                        actual.framebuffer.stencil_at(x, y),
                        scalar.framebuffer.stencil_at(x, y)
                    );
                }
            }
            assert!(
                actual
                    .traces
                    .iter()
                    .any(|t| t.depth_pass && t.output.is_some())
            );
        }
    }
}
