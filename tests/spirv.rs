use shader::{
    Instruction::*,
    Program,
    spirv::{Module, Stage, link},
};
use silicon::*;
const VERTEX: &[u8] = include_bytes!("../assets/shaders/textured.vert.spv");
const FRAGMENT: &[u8] = include_bytes!("../assets/shaders/textured.frag.spv");
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
