use shader::{
    Instruction::*,
    Program,
    spirv::{Module, Stage, link},
};
use silicon::*;
const VERTEX: &[u8] = include_bytes!("../assets/shaders/textured.vert.spv");
const FRAGMENT: &[u8] = include_bytes!("../assets/shaders/textured.frag.spv");
const MATH_FRAGMENT: &[u8] = include_bytes!("../assets/shaders/math.frag.spv");
#[test]
fn pipeline_cache_reuses_linked_programs_and_keys_pipeline_state() {
    let mut cache = PipelineCache::default();
    let first = cache
        .get_or_compile(VERTEX, FRAGMENT, Pipeline::default())
        .unwrap();
    let hit = cache
        .get_or_compile(VERTEX, FRAGMENT, Pipeline::default())
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &hit));

    let changed_state = Pipeline {
        cull: Cull::Back,
        ..Default::default()
    };
    cache
        .get_or_compile(VERTEX, FRAGMENT, changed_state)
        .unwrap();
    let before_eviction = cache.stats();
    assert_eq!(
        (
            before_eviction.hits,
            before_eviction.misses,
            before_eviction.entries
        ),
        (1, 2, 2)
    );
    for reference in 0..=PIPELINE_CACHE_CAPACITY as u8 {
        let state = Pipeline {
            stencil: Some(StencilState {
                compare: silicon::Compare::Always,
                reference,
                read_mask: u8::MAX,
                write_mask: u8::MAX,
                fail: StencilOp::Keep,
                depth_fail: StencilOp::Keep,
                pass: StencilOp::Keep,
            }),
            ..Default::default()
        };
        cache.get_or_compile(VERTEX, FRAGMENT, state).unwrap();
    }
    let stats = cache.stats();
    assert_eq!(stats.entries, PIPELINE_CACHE_CAPACITY);
    assert!(stats.evictions > 0);
    assert!(stats.compile_time > std::time::Duration::ZERO);
    assert!(stats.lookup_time > std::time::Duration::ZERO);
}
#[test]
fn built_in_spirv_scene_reuses_pipeline_across_frames() {
    let first = demo::spirv_cube(16, 16, 0.).unwrap();
    let second = demo::spirv_cube(16, 16, 0.5).unwrap();
    let pipeline = |capture: &FrameCapture| {
        std::sync::Arc::clone(
            capture
                .commands
                .stream()
                .iter()
                .find_map(|command| match command {
                    Command::BindPipeline(pipeline) => Some(pipeline),
                    _ => None,
                })
                .unwrap(),
        )
    };
    assert!(std::sync::Arc::ptr_eq(
        &pipeline(&first),
        &pipeline(&second)
    ));
}
#[test]
fn public_device_api_creates_pipeline_and_submits_a_frame() {
    let device = Device::new();
    let vertex = device.create_shader(VERTEX).unwrap();
    let fragment = device.create_shader(FRAGMENT).unwrap();
    assert_eq!(vertex.stage(), shader::spirv::Stage::Vertex);
    assert_eq!(fragment.stage(), shader::spirv::Stage::Fragment);
    assert!(
        device
            .create_pipeline(&fragment, &vertex, Pipeline::default())
            .is_err()
    );
    let pipeline = device
        .create_pipeline(
            &vertex,
            &fragment,
            Pipeline {
                cull: Cull::Back,
                ..Default::default()
            },
        )
        .unwrap();
    let capture = demo::shader_cube_with_pipeline(32, 24, 0., pipeline).unwrap();
    let mut renderer = Renderer::new(capture.width, capture.height).unwrap();
    let submission = device.submit(&capture.commands, &mut renderer).unwrap();
    assert_eq!(submission.draws, 1);
    assert!(renderer.stats.shaded > 0);
}
fn compiled(bytes: &[u8]) -> shader::spirv::Compiled {
    Module::parse(bytes).unwrap().translate().unwrap()
}
fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
#[test]
fn ordinary_glsl_renders_like_reference_and_replays_in_parallel() {
    let vertex = compiled(VERTEX);
    let fragment = compiled(FRAGMENT);
    assert_eq!(vertex.stage, Stage::Vertex);
    assert_eq!(fragment.stage, Stage::Fragment);
    link(&vertex, &fragment).unwrap();
    let reference_vertex = Program::new(vec![
        Input { dst: 0, slot: 0 },
        shader::Instruction::Mat4 {
            dst: 1,
            src: 0,
            uniform: 0,
        },
        Output { slot: 0, src: 1 },
        Input { dst: 2, slot: 1 },
        Output { slot: 1, src: 2 },
        Input { dst: 3, slot: 2 },
        Output { slot: 2, src: 3 },
    ])
    .unwrap();
    let reference_fragment = Program::new(vec![
        Input { dst: 0, slot: 1 },
        SampleImplicit {
            dst: 1,
            uv: 0,
            texture: 0,
        },
        Input { dst: 2, slot: 0 },
        Mul { dst: 3, a: 1, b: 2 },
        Output { slot: 0, src: 3 },
    ])
    .unwrap();
    let reference =
        demo::shader_cube_with_programs(97, 65, 0.37, reference_vertex, reference_fragment)
            .unwrap()
            .replay()
            .unwrap();
    let capture = demo::spirv_cube(97, 65, 0.37).unwrap();
    let actual = capture.replay().unwrap();
    assert_eq!(actual.framebuffer.bytes(), reference.framebuffer.bytes());
    assert!(actual.stats.shaded > 100);
    let file = std::env::temp_dir().join(format!("silicon-spirv-{}.silicon", std::process::id()));
    capture.save(&file).unwrap();
    let loaded = FrameCapture::load(&file).unwrap();
    std::fs::remove_file(file).unwrap();
    let mut parallel = Renderer::new(97, 65).unwrap();
    parallel.backend = Backend::Simd;
    parallel
        .render_bands(4, |r| Device.submit(&loaded.commands, r).map(|_| ()))
        .unwrap();
    assert_eq!(parallel.framebuffer.bytes(), actual.framebuffer.bytes());
    let mut wrong = fragment.clone();
    wrong.inputs.insert(1, 3);
    assert!(link(&vertex, &wrong).is_err());
    assert!(link(&fragment, &vertex).is_err());
}
#[test]
fn vector_padding_and_implicit_lod_do_not_change_glsl_arithmetic() {
    let fragment = compiled(include_bytes!("../assets/shaders/arithmetic.frag.spv"));
    // The unused Z/W lanes of vec2 division must not become 0/0. LOD is separate from UV math.
    let input = [Vec4::new(0.5, 0.75, 1., 1.), Vec4::new(0.2, 0.4, 19., 23.)];
    let e = fragment
        .program
        .execute_with_lod(
            &input,
            &[],
            &[3.],
            |slot, uv| {
                assert_eq!(slot, 0);
                assert_eq!(uv, Vec4::new(0.2, 0.4, 3., 0.));
                Ok(Vec4::new(1., 0.5, 0.25, 1.))
            },
            true,
        )
        .unwrap();
    let weight = (0.4 + 2. - 0.5) / 4. * 0.5 + (0.2 + 1. - 0.25) / 2. * 0.25;
    let expected = Vec4::new(0.5, 0.375, 0.25, 1.) * weight;
    for (a, b) in e.outputs[0].to_array().into_iter().zip(expected.to_array()) {
        assert!((a - b).abs() < 1e-6);
    }
    assert_eq!(e.samples, 1);
    assert!(
        fragment
            .program
            .execute(&input, &[], |_, _| Ok(Vec4::ZERO), false)
            .unwrap_err()
            .contains("missing implicit LOD")
    );
    assert!(
        fragment
            .program
            .execute_with_lod(&input, &[], &[f32::NAN], |_, _| Ok(Vec4::ZERO), false)
            .is_err()
    );
}
#[test]
fn glsl_std450_floor_fract_sin_cos_run_in_scalar_and_packet_vm() {
    let fragment = compiled(MATH_FRAGMENT);
    let inputs = [
        Vec4::new(-2.5, -0.25, 0.25, 1.2),
        Vec4::new(2.75, 1.25, -0.5, -1.1),
        Vec4::new(0.9, -3.75, 2.0, 0.3),
        Vec4::new(2.5, 4.5, -2.5, 0.7),
    ];
    let scalar: Vec<_> = inputs
        .iter()
        .map(|&input| {
            fragment
                .program
                .execute(&[input], &[], |_, _| Err("no textures".into()), false)
                .unwrap()
                .outputs[0]
        })
        .collect();
    let packet = fragment
        .program
        .execute4(
            std::array::from_fn(|lane| std::slice::from_ref(&inputs[lane])),
            &[],
            [&[], &[], &[], &[]],
            0b1111,
            |_, _, _| Err("no textures".into()),
            [false; 4],
        )
        .unwrap();
    for lane in 0..4 {
        let source = inputs[lane].to_array();
        let positive = source[0] * source[0] + 1.0;
        let expected = [
            source[0].floor()
                + source[0].ceil()
                + source[0].trunc()
                + source[0].round()
                + source[0].round_ties_even(),
            source[1] - source[1].floor() + positive.sqrt() + positive.sqrt().recip(),
            source[2].sin() + source[3].cos(),
            (source[2] * 0.1).exp() + (source[2] * 0.1).exp2() + positive.ln() + positive.log2(),
        ];
        assert_eq!(packet[lane].outputs[0], scalar[lane]);
        for (actual, expected) in packet[lane].outputs[0].to_array().into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-6);
        }
    }
    assert_eq!(scalar[0].x, -12.);
    assert_eq!(scalar[3].x, 12.);
}
#[test]
fn glsl_unary_math_renders_identically_through_worker_bands() {
    let device = Device::new();
    let vertex = device.create_shader(VERTEX).unwrap();
    let fragment = device.create_shader(MATH_FRAGMENT).unwrap();
    let pipeline = device
        .create_pipeline(&vertex, &fragment, Pipeline::default())
        .unwrap();
    let capture = demo::shader_cube_with_pipeline(64, 48, 0.25, pipeline).unwrap();
    let scalar = capture.replay().unwrap().framebuffer.bytes().to_vec();
    let mut packet = Renderer::new(64, 48).unwrap();
    packet.backend = Backend::Simd;
    device.submit(&capture.commands, &mut packet).unwrap();
    assert_eq!(packet.framebuffer.bytes(), scalar);
    let mut bands = Renderer::new(64, 48).unwrap();
    bands.backend = Backend::Simd;
    bands
        .render_bands(4, |renderer| {
            device.submit(&capture.commands, renderer).map(|_| ())
        })
        .unwrap();
    assert_eq!(bands.framebuffer.bytes(), scalar);
}
#[test]
fn float_negate_preserves_sign_bits_in_scalar_and_packet_execution() {
    let module = Module::parse(include_bytes!("../assets/shaders/negate.frag.spv")).unwrap();
    assert!(module.instructions().iter().any(|op| op.opcode == 127));
    let fragment = module.translate().unwrap();
    let source = Vec4::new(2., 0., -0., -4.);
    let expected = [
        (-2.0f32).to_bits(),
        (-0.0f32).to_bits(),
        0.0f32.to_bits(),
        4.0f32.to_bits(),
    ];
    let result = fragment
        .program
        .execute(&[source], &[], |_, _| Err("no textures".into()), false)
        .unwrap()
        .outputs[0]
        .to_array();
    assert_eq!(result.map(f32::to_bits), expected);

    let inputs = [
        [source],
        [Vec4::new(-1., 1., -0., 0.)],
        [Vec4::new(3., -3., 0., -0.)],
        [Vec4::new(0.5, -0.5, -2., 2.)],
    ];
    let empty: &[f32] = &[];
    let packet = fragment
        .program
        .execute4(
            [&inputs[0], &inputs[1], &inputs[2], &inputs[3]],
            &[],
            [empty; 4],
            0b1111,
            |_, _, _| Err("no textures".into()),
            [false; 4],
        )
        .unwrap();
    for (lane, input) in inputs.iter().enumerate() {
        let expected = input[0]
            .to_array()
            .map(|v| f32::from_bits(v.to_bits() ^ 0x8000_0000).to_bits());
        assert_eq!(
            packet[lane].outputs[0].to_array().map(f32::to_bits),
            expected
        );
    }
}
#[test]
fn pbr_spirv_responds_to_roughness_and_normal_maps_and_replays_across_backends() {
    let fragment = compiled(include_bytes!("../assets/shaders/pbr.frag.spv"));
    assert!(fragment.program.instructions().iter().any(|instruction| {
        matches!(
            instruction,
            shader::Instruction::SampleCube { texture: 2, .. }
        )
    }));
    let inputs = [
        Vec4::new(1., 0., 0., 1.),
        Vec4::new(0.4, 0.6, 0., 0.),
        Vec3::new(0., 0., 1.).extend(0.),
        Vec4::new(0.4, 1.2, -0.2, 1.),
    ];
    let shade = |roughness, normal_strength| {
        let mut uniforms = vec![Vec4::ZERO; 36];
        uniforms[12] = Vec4::new(0.8, 0.4, 0.18, 1.);
        uniforms[16] = Vec4::new(0., 0.8, 0., roughness);
        uniforms[20] = Vec3::new(7.5, 5.8, 10.).extend(1.);
        uniforms[32] = Vec4::new(normal_strength, 0., 0., 0.);
        let mut cube_lod = None;
        let color = fragment
            .program
            .execute_with_lod(
                &inputs,
                &uniforms,
                &[0., 0.],
                |slot, coordinate| {
                    Ok(match slot {
                        0 => Vec4::new(1., 1., 1., 1.),
                        1 => Vec4::new(0.7, 0.5, 0.95, 1.),
                        2 => {
                            assert!(coordinate.xyz().is_finite());
                            cube_lod = Some(coordinate.w);
                            Vec4::new(0.25 + coordinate.w * 0.1, 0.5, 0.75, 1.)
                        }
                        _ => unreachable!(),
                    })
                },
                false,
            )
            .unwrap()
            .outputs[0];
        (color, cube_lod.unwrap())
    };
    let (smooth, smooth_lod) = shade(0.12, 0.75);
    let (rough, rough_lod) = shade(0.85, 0.75);
    assert!((smooth_lod - 0.6).abs() < 1e-6);
    assert!((rough_lod - 4.25).abs() < 1e-6);
    assert_ne!(
        smooth.to_array().map(f32::to_bits),
        rough.to_array().map(f32::to_bits)
    );
    let (flat, _) = shade(0.12, 0.);
    assert_ne!(
        smooth.to_array().map(f32::to_bits),
        flat.to_array().map(f32::to_bits)
    );
    assert!(smooth.is_finite() && rough.is_finite() && flat.is_finite());

    let capture = demo::pbr_showcase(240, 160, 0.37).unwrap();
    assert!(
        capture
            .commands
            .stream()
            .iter()
            .any(|command| { matches!(command, Command::BindCubeMap { slot: 2, .. }) })
    );
    let path = std::env::temp_dir().join(format!("silicon-pbr-{}.silicon", std::process::id()));
    capture.save(&path).unwrap();
    let loaded = FrameCapture::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let scalar = loaded.replay().unwrap();
    assert!(scalar.stats.shaded > 1000);
    assert!(scalar.stats.texture_samples > scalar.stats.shaded as u64);
    let mut simd = Renderer::new(240, 160).unwrap();
    simd.backend = Backend::Simd;
    simd.render_bands(4, |band| Device.submit(&loaded.commands, band).map(|_| ()))
        .unwrap();
    assert_eq!(scalar.framebuffer.bytes(), simd.framebuffer.bytes());
}

#[test]
fn sampler_cube_requires_a_cube_image_before_render_pass_clear() {
    let device = Device::new();
    let vertex = device
        .create_shader(include_bytes!("../assets/shaders/pbr.vert.spv"))
        .unwrap();
    let fragment = device
        .create_shader(include_bytes!("../assets/shaders/pbr.frag.spv"))
        .unwrap();
    let pipeline = device
        .create_pipeline(&vertex, &fragment, Pipeline::default())
        .unwrap();
    let texture = std::sync::Arc::new(Texture::checker(4).unwrap());
    let vertices = [
        Vertex::new(Vec3::new(-0.5, -0.5, 0.), Color::WHITE),
        Vertex::new(Vec3::new(0.5, -0.5, 0.), Color::WHITE),
        Vertex::new(Vec3::new(0., 0.5, 0.), Color::WHITE),
    ];
    let mut commands = device.commands();
    commands.begin_render_pass(Color::BLACK);
    commands.bind_pipeline(pipeline);
    commands.bind_vertex_buffer(device.create_vertex_buffer(vertices.to_vec()).unwrap());
    commands.bind_index_buffer(device.create_index_buffer(vec![0, 1, 2]).unwrap());
    commands.bind_uniform_buffer(device.create_uniform_buffer(vec![Vec4::ZERO; 36]).unwrap());
    commands.bind_texture(0, texture.clone(), Sampler::default());
    commands.bind_texture(1, texture.clone(), Sampler::default());
    commands.bind_texture(2, texture, Sampler::default());
    commands.draw_indexed(0, 3);
    commands.end_render_pass();

    let mut renderer = Renderer::new(8, 8).unwrap();
    renderer.clear(Color::WHITE);
    let before = renderer.framebuffer.bytes().to_vec();
    let error = device.submit(&commands, &mut renderer).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("sampler and bound image types do not match")
    );
    assert_eq!(renderer.framebuffer.bytes(), before);
}

#[test]
fn malformed_headers_ids_types_blocks_and_decorations_are_rejected() {
    let module = Module::parse(VERTEX).unwrap();
    let original = words(VERTEX);
    let reject = |w: Vec<u32>| {
        assert!(
            Module::parse(&bytes(&w))
                .and_then(|m| m.translate())
                .is_err()
        )
    };
    for (i, v) in [(0, 0), (1, 0x00010600), (3, 0), (3, 65537), (4, 1), (5, 0)] {
        let mut w = original.clone();
        w[i] = v;
        reject(w);
    }
    let mutate = |opcode: u16, operand: usize, value: u32| {
        let op = module
            .instructions()
            .iter()
            .find(|op| op.opcode == opcode)
            .unwrap();
        let mut w = original.clone();
        w[op.word + 1 + operand] = value;
        reject(w);
    };
    mutate(22, 1, 64);
    mutate(61, 2, 0);
    mutate(65, 3, 24); // wrong width, bad ID, float index
    mutate(248, 0, 4); // duplicate result ID
    mutate(59, 2, 12); // unsupported/mismatched storage class
    mutate(54, 2, 1); // unsupported function control
    let stride = module
        .instructions()
        .iter()
        .find(|op| op.opcode == 72 && op.operands[2] == 7)
        .unwrap();
    let mut w = original.clone();
    w[stride.word + 4] = 0;
    reject(w);
    let ret = module
        .instructions()
        .iter()
        .find(|op| op.opcode == 253)
        .unwrap();
    let mut w = original.clone();
    w[ret.word] = (1 << 16) | 249;
    let err = Module::parse(&bytes(&w)).unwrap_err();
    assert!(err.contains("OpBranch") && err.contains("word"));
    let mut big = VERTEX.to_vec();
    for chunk in big.chunks_exact_mut(4) {
        chunk.reverse();
    }
    assert_eq!(
        Module::parse(&big)
            .unwrap()
            .translate()
            .unwrap()
            .program
            .instructions()
            .len(),
        compiled(VERTEX).program.instructions().len()
    );
    for n in 0..VERTEX.len() {
        assert!(
            Module::parse(&VERTEX[..n])
                .and_then(|m| m.translate())
                .is_err()
        );
    }
}
#[test]
fn deterministic_binary_mutations_cannot_panic_the_parser_or_translator() {
    let sources = [
        words(VERTEX),
        words(include_bytes!("../assets/shaders/lit.frag.spv")),
        words(include_bytes!("../assets/shaders/locals.frag.spv")),
    ];
    let mut seed = 0x31ab8492u32;
    for i in 0..1500 {
        let source = &sources[i % sources.len()];
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let index = seed as usize % source.len();
        let mut w = source.clone();
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        w[index] ^= seed;
        let input = bytes(&w);
        assert!(
            std::panic::catch_unwind(|| Module::parse(&input).and_then(|m| m.translate())).is_ok(),
            "word {index}"
        );
    }
}

#[test]
fn local_snapshots_component_stores_and_vector_uniforms_execute() {
    let fragment = compiled(include_bytes!("../assets/shaders/locals.frag.spv"));
    let mut uniforms = vec![Vec4::ZERO; 33];
    // Padding in host resources must never participate in vec2/vec3 arithmetic.
    uniforms[24] = Vec4::new(0.2, 0.4, 19., 23.);
    uniforms[28] = Vec4::new(2., 41., 43., 47.);
    uniforms[32] = Vec4::new(1., 2., 0.3, 53.);
    let input = Vec4::new(3., 4., 0., 0.);
    let e = fragment
        .program
        .execute(&[input], &uniforms, |_, _| Err("no textures".into()), true)
        .unwrap();
    let length = 13f32.sqrt();
    let expected = Vec4::new(
        3. + 0.2 + 3. / length,
        4. + 0.4 + 2. / length,
        length + 2.,
        0.3,
    );
    for (a, b) in e.outputs[0].to_array().into_iter().zip(expected.to_array()) {
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }
    let source = include_bytes!("../assets/shaders/locals.frag.spv");
    let module = Module::parse(source).unwrap();
    let mut w = words(source);
    let first_store = module
        .instructions()
        .iter()
        .find(|op| op.opcode == 62)
        .unwrap();
    w.drain(first_store.word..first_store.word + 3);
    assert!(
        Module::parse(&bytes(&w))
            .unwrap()
            .translate()
            .unwrap_err()
            .contains("before initialization")
    );
}
#[test]
fn lit_glsl_matches_native_scene_and_replays_exactly() {
    let capture = demo::spirv_showcase(97, 65, 0.37).unwrap();
    let actual = capture.replay().unwrap();
    let reference = demo::render("showcase", 97, 65, 0.37).unwrap();
    assert_eq!(actual.stats.triangles, 12588);
    assert_eq!(actual.stats.shaded, reference.stats.shaded);
    // External constant folding and CPU powf may differ by one quantized color unit.
    for (a, b) in actual
        .framebuffer
        .bytes()
        .iter()
        .zip(reference.framebuffer.bytes())
    {
        assert!(a.abs_diff(*b) <= 1, "GLSL {a}, native {b}");
    }
    let file = std::env::temp_dir().join(format!("silicon-lit-{}.silicon", std::process::id()));
    capture.save(&file).unwrap();
    let loaded = FrameCapture::load(&file).unwrap();
    std::fs::remove_file(file).unwrap();
    let mut parallel = Renderer::new(97, 65).unwrap();
    parallel.backend = Backend::Simd;
    parallel
        .render_bands(4, |r| Device.submit(&loaded.commands, r).map(|_| ()))
        .unwrap();
    assert_eq!(parallel.framebuffer.bytes(), actual.framebuffer.bytes());
    for y in 0..65 {
        for x in 0..97 {
            assert_eq!(
                actual.framebuffer.depth_at(x, y),
                reference.framebuffer.depth_at(x, y)
            );
            assert_eq!(
                parallel.framebuffer.depth_at(x, y),
                actual.framebuffer.depth_at(x, y)
            );
        }
    }
    let vertex = compiled(include_bytes!("../assets/shaders/lit.vert.spv"));
    let fragment = compiled(include_bytes!("../assets/shaders/lit.frag.spv"));
    link(&vertex, &fragment).unwrap();
    // This actual shader lowers to more than 64 instructions producing temporaries.
    assert!(fragment.program.instructions().len() > 100);
    let mut trace = Renderer::new(97, 65).unwrap();
    trace.debug_pixel = Some((48, 32));
    let stats = Device.submit(&capture.commands, &mut trace).unwrap();
    assert!(
        stats
            .shader_traces
            .iter()
            .any(|(_, trace)| trace.len() > 100)
    );
}

#[test]
fn shadow_glsl_samples_a_serialized_depth_pass_and_replays_exactly() {
    let vertex_bytes = include_bytes!("../assets/shaders/lit.vert.spv");
    let fragment_bytes = include_bytes!("../assets/shaders/shadow.frag.spv");
    let vertex = compiled(vertex_bytes);
    let fragment = compiled(fragment_bytes);
    link(&vertex, &fragment).unwrap();
    let module = Module::parse(fragment_bytes).unwrap();
    let sample = module
        .instructions()
        .iter()
        .find(|op| op.opcode == 88)
        .unwrap();
    let mut invalid_mask = words(fragment_bytes);
    invalid_mask[sample.word + 5] = 4;
    assert!(
        Module::parse(&bytes(&invalid_mask))
            .and_then(|m| m.translate())
            .is_err()
    );
    let mut invalid_lod = words(fragment_bytes);
    invalid_lod[sample.word + 6] = sample.operands[3];
    assert!(
        Module::parse(&bytes(&invalid_lod))
            .and_then(|m| m.translate())
            .is_err()
    );

    let (capture, depth_stats) = demo::shadow_showcase(97, 65, 0.37).unwrap();
    assert_eq!(depth_stats.triangles, 12_588);
    assert!(capture.commands.stream().iter().any(|command| matches!(
        command,
        Command::BindTexture { slot: 1, texture, .. }
            if matches!(texture.format, TextureFormat::Depth32Float)
                && texture.levels[0].width == 512
                && texture.levels[0].height == 512
    )));

    let shadowed = capture.replay().unwrap();
    let baseline = demo::spirv_showcase(97, 65, 0.37)
        .unwrap()
        .replay()
        .unwrap();
    let changed_pixels = shadowed
        .framebuffer
        .bytes()
        .chunks_exact(4)
        .zip(baseline.framebuffer.bytes().chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed_pixels > 10,
        "shadow map changed {changed_pixels} pixels"
    );

    let path = std::env::temp_dir().join(format!("silicon-shadow-{}.silicon", std::process::id()));
    capture.save(&path).unwrap();
    let loaded = FrameCapture::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let mut parallel = Renderer::new(97, 65).unwrap();
    parallel.backend = Backend::Simd;
    parallel
        .render_bands(4, |r| Device.submit(&loaded.commands, r).map(|_| ()))
        .unwrap();
    assert_eq!(parallel.framebuffer.bytes(), shadowed.framebuffer.bytes());
    for y in 0..65 {
        for x in 0..97 {
            assert_eq!(
                parallel.framebuffer.depth_at(x, y),
                shadowed.framebuffer.depth_at(x, y)
            );
            assert_eq!(
                parallel.framebuffer.stencil_at(x, y),
                shadowed.framebuffer.stencil_at(x, y)
            );
        }
    }
}

#[test]
fn extended_math_and_local_pointer_validation_reject_malformed_inputs() {
    let source = include_bytes!("../assets/shaders/lit.frag.spv");
    let module = Module::parse(source).unwrap();
    let original = words(source);
    let reject = |w: Vec<u32>| {
        assert!(
            Module::parse(&bytes(&w))
                .and_then(|m| m.translate())
                .is_err()
        )
    };
    let ext = module
        .instructions()
        .iter()
        .find(|op| op.opcode == 12)
        .unwrap();
    for (operand, value) in [(2, ext.operands[0]), (3, 999), (0, 2)] {
        let mut w = original.clone();
        w[ext.word + 1 + operand] = value;
        reject(w);
    }
    let component = module
        .instructions()
        .iter()
        .find(|op| op.opcode == 65 && op.operands.len() == 5)
        .unwrap();
    let out_of_range = module
        .instructions()
        .iter()
        .find(|op| op.opcode == 43 && op.operands[2] == 1115684864)
        .unwrap()
        .operands[1];
    let mut w = original.clone();
    w[component.word + 5] = out_of_range;
    reject(w); // float index
    let local = module
        .instructions()
        .iter()
        .find(|op| op.opcode == 59 && op.operands[2] == 7)
        .unwrap();
    let mut w = original.clone();
    w[local.word + 3] = 2;
    reject(w);
}
