use silicon::*;
#[test]
fn capture_replay_is_exact_and_owns_resources() {
    let c = demo::shader_cube(96, 64, 0.).unwrap();
    let expected = c.replay().unwrap().framebuffer.bytes().to_vec();
    let directory =
        std::env::temp_dir().join(format!("silicon-test-capture-{}", std::process::id()));
    let path = directory.join("nested/frame.silicon");
    assert!(!path.exists());
    c.save(&path).unwrap();
    let loaded = FrameCapture::load(&path).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(loaded.replay().unwrap().framebuffer.bytes(), expected);
    assert!(expected.chunks_exact(4).any(|p| p[0] > 100));
    let mut r = Renderer::new(96, 64).unwrap();
    let stats = Device.submit(&loaded.commands, &mut r).unwrap();
    assert_eq!(stats.draws, 1);
    assert_eq!(stats.texture_samples, r.stats.shaded);
    assert_eq!(stats.shader_instructions, 24 * 10 + r.stats.shaded * 15);
}

#[test]
fn capture_preserves_multisample_state_and_reads_legacy_captures() {
    let mut capture = demo::shader_cube(64, 48, 0.).unwrap();
    capture.sample_count = SampleCount::Four;
    let expected = capture.replay().unwrap();
    assert_eq!(expected.sample_count(), SampleCount::Four);

    let path = std::env::temp_dir().join(format!(
        "silicon-multisample-capture-{}.silicon",
        std::process::id()
    ));
    capture.save(&path).unwrap();
    let loaded = FrameCapture::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let actual = loaded.replay().unwrap();
    assert_eq!(actual.sample_count(), SampleCount::Four);
    assert_eq!(actual.framebuffer.bytes(), expected.framebuffer.bytes());

    let legacy = FrameCapture::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/seeds/capture/seed.json"
    ))
    .unwrap();
    assert_eq!(legacy.sample_count, SampleCount::One);
}
#[test]
fn invalid_command_stream_never_changes_target() {
    let mut r = Renderer::new(8, 8).unwrap();
    r.clear(Color::WHITE);
    let before = r.framebuffer.bytes().to_vec();
    let mut cmd = Device.commands();
    cmd.begin_render_pass(Color::BLACK);
    cmd.draw(0, 3);
    cmd.end_render_pass();
    assert!(Device.submit(&cmd, &mut r).is_err());
    assert_eq!(r.framebuffer.bytes(), before);
}

#[test]
fn compute_storage_instructions_are_rejected_before_graphics_clear() {
    use shader::Instruction::*;
    use std::sync::Arc;

    let vertex =
        shader::Program::new(vec![Input { dst: 0, slot: 0 }, Output { slot: 0, src: 0 }]).unwrap();
    let fragment = shader::Program::new(vec![
        Const {
            dst: 0,
            value: Vec4::ZERO,
        },
        Input { dst: 1, slot: 0 },
        StorageStore { index: 0, src: 1 },
        Output { slot: 0, src: 1 },
    ])
    .unwrap();
    let pipeline = Arc::new(ShaderPipeline {
        state: Pipeline::default(),
        vertex,
        fragment,
    });
    let vertices = Device
        .create_vertex_buffer(vec![
            Vertex::new(math::Vec3::new(-1.0, -1.0, 0.5), Color::WHITE),
            Vertex::new(math::Vec3::new(1.0, -1.0, 0.5), Color::WHITE),
            Vertex::new(math::Vec3::new(-1.0, 1.0, 0.5), Color::WHITE),
        ])
        .unwrap();
    let mut commands = Device.commands();
    commands.begin_render_pass(Color::BLACK);
    commands.bind_pipeline(pipeline);
    commands.bind_vertex_buffer(vertices);
    commands.draw(0, 3);
    commands.end_render_pass();

    let mut renderer = Renderer::new(8, 8).unwrap();
    renderer.clear(Color::WHITE);
    let before = renderer.framebuffer.bytes().to_vec();
    assert!(commands.validate().is_err());
    assert!(Device.submit(&commands, &mut renderer).is_err());
    assert_eq!(renderer.framebuffer.bytes(), before);
}

#[test]
fn workgroup_barriers_are_rejected_before_graphics_clear() {
    use shader::Instruction::*;
    use std::sync::Arc;

    let vertex =
        shader::Program::new(vec![Input { dst: 0, slot: 0 }, Output { slot: 0, src: 0 }]).unwrap();
    let fragment = shader::Program::new(vec![
        WorkgroupBarrier,
        Const {
            dst: 0,
            value: Vec4::ZERO,
        },
        Output { slot: 0, src: 0 },
    ])
    .unwrap();
    let pipeline = Arc::new(ShaderPipeline {
        state: Pipeline::default(),
        vertex,
        fragment,
    });
    let mut commands = Device.commands();
    commands.begin_render_pass(Color::BLACK);
    commands.bind_pipeline(pipeline);
    commands.end_render_pass();

    let mut renderer = Renderer::new(8, 8).unwrap();
    renderer.clear(Color::WHITE);
    let before = renderer.framebuffer.bytes().to_vec();
    assert!(commands.validate().is_err());
    assert!(Device.submit(&commands, &mut renderer).is_err());
    assert_eq!(renderer.framebuffer.bytes(), before);
}

#[test]
fn compute_atomics_are_rejected_before_graphics_clear() {
    use shader::Instruction::*;
    use std::sync::Arc;

    let vertex =
        shader::Program::new(vec![Input { dst: 0, slot: 0 }, Output { slot: 0, src: 0 }]).unwrap();
    let fragment = shader::Program::new(vec![
        Const {
            dst: 0,
            value: Vec4::ZERO,
        },
        Const {
            dst: 1,
            value: Vec4::new(1.0, 0.0, 0.0, 0.0),
        },
        AtomicAdd {
            dst: 2,
            buffer: 0,
            index: 0,
            value: 1,
        },
        Output { slot: 0, src: 2 },
    ])
    .unwrap();
    let pipeline = Arc::new(ShaderPipeline {
        state: Pipeline::default(),
        vertex,
        fragment,
    });
    let mut commands = Device.commands();
    commands.begin_render_pass(Color::BLACK);
    commands.bind_pipeline(pipeline);
    commands.end_render_pass();

    let mut renderer = Renderer::new(8, 8).unwrap();
    renderer.clear(Color::WHITE);
    let before = renderer.framebuffer.bytes().to_vec();
    assert!(commands.validate().is_err());
    assert!(Device.submit(&commands, &mut renderer).is_err());
    assert_eq!(renderer.framebuffer.bytes(), before);
}

#[test]
fn selected_pixel_traces_executed_sir() {
    let c = demo::shader_cube(96, 64, 0.).unwrap();
    let mut r = Renderer::new(96, 64).unwrap();
    r.debug_pixel = Some((48, 32));
    let s = Device.submit(&c.commands, &mut r).unwrap();
    assert!(!s.shader_traces.is_empty());
    assert_eq!(s.shader_traces[0].1.len(), 15);
    assert!(
        s.shader_traces[0]
            .1
            .iter()
            .any(|t| matches!(t.operation, shader::Instruction::SampleImplicit { .. }))
    );
}

#[test]
fn profile_collects_command_and_raster_stage_times() {
    let capture = demo::shader_cube(96, 64, 0.).unwrap();
    let mut renderer = Renderer::new(96, 64).unwrap();
    renderer.profile_shaders = true;
    Device.submit(&capture.commands, &mut renderer).unwrap();
    let stats = renderer.stats;
    assert!(stats.command_processing_time > std::time::Duration::ZERO);
    assert!(stats.vertex_time > std::time::Duration::ZERO);
    assert!(stats.primitive_setup_time > std::time::Duration::ZERO);
    assert!(stats.rasterization_time > std::time::Duration::ZERO);
    assert!(stats.shader_time > std::time::Duration::ZERO);
    assert!(stats.blend_write_time > std::time::Duration::ZERO);
    assert!(stats.triangles > 0 && stats.fragments > 0 && stats.shaded > 0);
}
