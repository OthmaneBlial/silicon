use silicon::*;
#[test]
fn capture_replay_is_exact_and_owns_resources() {
    let c = demo::shader_cube(96, 64, 0.).unwrap();
    let expected = c.replay().unwrap().framebuffer.bytes().to_vec();
    let path = std::env::temp_dir().join(format!("silicon-test-{}.silicon", std::process::id()));
    c.save(&path).unwrap();
    let loaded = FrameCapture::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(loaded.replay().unwrap().framebuffer.bytes(), expected);
    assert!(expected.chunks_exact(4).any(|p| p[0] > 100));
    let mut r = Renderer::new(96, 64).unwrap();
    let stats = Device.submit(&loaded.commands, &mut r).unwrap();
    assert_eq!(stats.draws, 1);
    assert_eq!(stats.texture_samples, r.stats.shaded);
    assert_eq!(stats.shader_instructions, 24 * 10 + r.stats.shaded * 15);
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
            .any(|t| matches!(t.operation, shader::Instruction::Sample { .. }))
    );
}
