use silicon::shader::{Instruction, Program};
use silicon::*;
use std::sync::Arc;

fn shader_pipeline(fragment: Program) -> Arc<ShaderPipeline> {
    let vertex = Program::new(vec![
        Instruction::Input { dst: 0, slot: 0 },
        Instruction::Output { slot: 0, src: 0 },
        Instruction::Input { dst: 1, slot: 2 },
        Instruction::Output { slot: 2, src: 1 },
    ])
    .unwrap();
    Arc::new(ShaderPipeline {
        state: Pipeline::default(),
        vertex,
        fragment,
    })
}

fn record_commands<F: Fn(&mut CommandBuffer)>(
    pipeline: Arc<ShaderPipeline>,
    vertices: Buffer<Vertex>,
    bind_image: &F,
) -> CommandBuffer {
    let mut commands = Device.commands();
    commands.begin_render_pass(Color::BLACK);
    commands.bind_pipeline(pipeline);
    commands.bind_vertex_buffer(vertices);
    bind_image(&mut commands);
    commands.draw(0, 3);
    commands.end_render_pass();
    commands
}

fn render<F: Fn(&mut CommandBuffer)>(
    pipeline: Arc<ShaderPipeline>,
    vertices: Buffer<Vertex>,
    bind_image: &F,
    backend: Backend,
) -> Vec<u8> {
    let commands = record_commands(pipeline, vertices, bind_image);
    let mut renderer = Renderer::new(32, 32).unwrap();
    renderer.backend = backend;
    Device.submit(&commands, &mut renderer).unwrap();
    renderer.framebuffer.bytes().to_vec()
}

fn render_capture<F: Fn(&mut CommandBuffer)>(
    pipeline: Arc<ShaderPipeline>,
    vertices: Buffer<Vertex>,
    bind_image: &F,
    name: &str,
) -> Vec<u8> {
    let capture = FrameCapture {
        version: 2,
        width: 32,
        height: 32,
        sample_count: SampleCount::One,
        commands: record_commands(pipeline, vertices, bind_image),
    };
    let path = std::env::temp_dir().join(format!(
        "silicon-texture-dimensions-{name}-{}.silicon",
        std::process::id()
    ));
    capture.save(&path).unwrap();
    let loaded = FrameCapture::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    loaded.replay().unwrap().framebuffer.bytes().to_vec()
}

fn triangle() -> Buffer<Vertex> {
    Device
        .create_vertex_buffer(vec![
            Vertex::new(Vec3::new(-1.0, -1.0, 0.5), Color::WHITE),
            Vertex::new(Vec3::new(1.0, -1.0, 0.5), Color::WHITE),
            Vertex::new(Vec3::new(0.0, 1.0, 0.5), Color::WHITE),
        ])
        .unwrap()
}

#[test]
fn array_sampling_runs_through_scalar_and_simd_shader_paths() {
    let layers = vec![
        Texture::new(1, 1, TextureFormat::Rgba8, &[255, 0, 0, 255]).unwrap(),
        Texture::new(1, 1, TextureFormat::Rgba8, &[0, 0, 255, 255]).unwrap(),
    ];
    let array = Arc::new(TextureArray::new(layers).unwrap());
    let program = Program::new(vec![
        Instruction::Const {
            dst: 0,
            value: Vec4::new(0.5, 0.5, 1.0, 9.0),
        },
        Instruction::SampleArrayImplicit {
            dst: 1,
            uv: 0,
            texture: 0,
        },
        Instruction::Output { slot: 0, src: 1 },
    ])
    .unwrap();
    let pipeline = shader_pipeline(program);
    let bind = |commands: &mut CommandBuffer| {
        commands.bind_texture_array(
            0,
            Arc::clone(&array),
            Sampler {
                filter: Filter::Nearest,
                mip: MipFilter::None,
                ..Default::default()
            },
        );
    };
    let scalar = render(Arc::clone(&pipeline), triangle(), &bind, Backend::Scalar);
    let simd = render(Arc::clone(&pipeline), triangle(), &bind, Backend::Simd);
    assert_eq!(scalar, simd);
    assert_eq!(&scalar[(16 * 32 + 16) * 4..][..4], &[0, 0, 255, 255]);
    assert_eq!(
        scalar,
        render_capture(Arc::clone(&pipeline), triangle(), &bind, "array")
    );

    let explicit = shader_pipeline(
        Program::new(vec![
            Instruction::Const {
                dst: 0,
                value: Vec4::new(0.5, 0.5, 1.0, 0.0),
            },
            Instruction::SampleArray {
                dst: 1,
                uv: 0,
                texture: 0,
            },
            Instruction::Output { slot: 0, src: 1 },
        ])
        .unwrap(),
    );
    let scalar = render(Arc::clone(&explicit), triangle(), &bind, Backend::Scalar);
    let simd = render(explicit, triangle(), &bind, Backend::Simd);
    assert_eq!(scalar, simd);
    assert_eq!(&scalar[(16 * 32 + 16) * 4..][..4], &[0, 0, 255, 255]);
}

#[test]
fn volume_sampling_runs_through_scalar_and_simd_shader_paths() {
    let volume = Arc::new(
        Texture3D::new(
            1,
            1,
            2,
            TextureFormat::Rgba8,
            &[255, 0, 0, 255, 0, 255, 0, 255],
        )
        .unwrap(),
    );
    let program = Program::new(vec![
        Instruction::Const {
            dst: 0,
            value: Vec4::new(0.5, 0.5, 0.75, 9.0),
        },
        Instruction::Sample3DImplicit {
            dst: 1,
            coordinate: 0,
            texture: 0,
        },
        Instruction::Output { slot: 0, src: 1 },
    ])
    .unwrap();
    let pipeline = shader_pipeline(program);
    let bind = |commands: &mut CommandBuffer| {
        commands.bind_texture3d(
            0,
            Arc::clone(&volume),
            Sampler {
                filter: Filter::Nearest,
                mip: MipFilter::None,
                ..Default::default()
            },
        );
    };
    let scalar = render(Arc::clone(&pipeline), triangle(), &bind, Backend::Scalar);
    let simd = render(Arc::clone(&pipeline), triangle(), &bind, Backend::Simd);
    assert_eq!(scalar, simd);
    assert_eq!(&scalar[(16 * 32 + 16) * 4..][..4], &[0, 255, 0, 255]);
    assert_eq!(
        scalar,
        render_capture(Arc::clone(&pipeline), triangle(), &bind, "volume")
    );

    let explicit = shader_pipeline(
        Program::new(vec![
            Instruction::Const {
                dst: 0,
                value: Vec4::new(0.5, 0.5, 0.75, 0.0),
            },
            Instruction::Sample3D {
                dst: 1,
                coordinate: 0,
                texture: 0,
            },
            Instruction::Output { slot: 0, src: 1 },
        ])
        .unwrap(),
    );
    let scalar = render(Arc::clone(&explicit), triangle(), &bind, Backend::Scalar);
    let simd = render(explicit, triangle(), &bind, Backend::Simd);
    assert_eq!(scalar, simd);
    assert_eq!(&scalar[(16 * 32 + 16) * 4..][..4], &[0, 255, 0, 255]);
}

#[test]
fn image_dimension_mismatch_is_rejected_before_the_render_pass_clears() {
    let pipeline = shader_pipeline(
        Program::new(vec![
            Instruction::Const {
                dst: 0,
                value: Vec4::new(0.5, 0.5, 0.0, 0.0),
            },
            Instruction::SampleArray {
                dst: 1,
                uv: 0,
                texture: 0,
            },
            Instruction::Output { slot: 0, src: 1 },
        ])
        .unwrap(),
    );
    let texture = Arc::new(Texture::new(1, 1, TextureFormat::Rgba8, &[255, 0, 0, 255]).unwrap());
    let mut commands = Device.commands();
    commands.begin_render_pass(Color::BLACK);
    commands.bind_pipeline(pipeline);
    commands.bind_vertex_buffer(triangle());
    commands.bind_texture(0, texture, Sampler::default());
    commands.draw(0, 3);
    commands.end_render_pass();

    let mut renderer = Renderer::new(32, 32).unwrap();
    renderer.clear(Color::new(0.2, 0.4, 0.6, 1.0));
    let original = renderer.framebuffer.bytes().to_vec();
    assert!(Device.submit(&commands, &mut renderer).is_err());
    assert_eq!(renderer.framebuffer.bytes(), original);
}
