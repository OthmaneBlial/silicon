use silicon::*;
use std::sync::Arc;

const VERTEX_SHADER: &[u8] = include_bytes!("../assets/shaders/mrt.vert.spv");
const FRAGMENT_SHADER: &[u8] = include_bytes!("../assets/shaders/mrt.frag.spv");

#[test]
fn spirv_writes_four_color_targets_and_capture_replays_them() {
    let device = Device::new();
    let vertex_shader = device.create_shader(VERTEX_SHADER).unwrap();
    let fragment_shader = device.create_shader(FRAGMENT_SHADER).unwrap();
    let pipeline = device
        .create_pipeline(&vertex_shader, &fragment_shader, Pipeline::default())
        .unwrap();
    let vertices = device
        .create_vertex_buffer(vec![
            Vertex::new(Vec3::new(-1.0, -1.0, 0.0), Color::WHITE),
            Vertex::new(Vec3::new(3.0, -1.0, 0.0), Color::WHITE),
            Vertex::new(Vec3::new(-1.0, 3.0, 0.0), Color::WHITE),
        ])
        .unwrap();
    let mut commands = device.commands();
    commands.begin_render_pass_with_colors(vec![
        Color::BLACK,
        Color::new(0.1, 0.2, 0.3, 1.0),
        Color::new(0.2, 0.3, 0.4, 1.0),
        Color::new(0.3, 0.4, 0.5, 1.0),
    ]);
    commands.bind_pipeline(Arc::clone(&pipeline));
    commands.bind_vertex_buffer(vertices.clone());
    commands.draw(0, 3);
    commands.end_render_pass();

    let capture = FrameCapture {
        version: 3,
        width: 32,
        height: 32,
        sample_count: SampleCount::Four,
        commands,
    };
    let path = std::env::temp_dir()
        .join(format!("silicon-mrt-{}", std::process::id()))
        .join("capture/frame.silicon");
    capture.save(&path).unwrap();
    let loaded = FrameCapture::load(&path).unwrap();
    std::fs::remove_dir_all(path.ancestors().nth(2).unwrap()).unwrap();
    let renderer = loaded.replay().unwrap();

    let expected = [
        Color::new(0.25, 0.5, 0.75, 1.0),
        Color::new(0.75, 0.5, 0.25, 1.0),
        Color::new(0.0, 1.0, 0.0, 1.0),
        Color::new(1.0, 0.0, 1.0, 1.0),
    ];
    assert_eq!(renderer.framebuffer.color_attachment_count(), 4);
    for (attachment, color) in expected.into_iter().enumerate() {
        assert_eq!(
            renderer
                .framebuffer
                .color_attachment_pixel(attachment, 16, 16)
                .map(Color::rgba8),
            Some(color.rgba8())
        );
    }

    let mut parallel = Renderer::new(32, 32).unwrap();
    parallel.backend = Backend::Simd;
    parallel.set_sample_count(SampleCount::Four).unwrap();
    parallel
        .render_bands(3, |band| device.submit(&loaded.commands, band).map(|_| ()))
        .unwrap();
    for attachment in 0..4 {
        assert_eq!(
            parallel.framebuffer.color_attachment_bytes(attachment),
            renderer.framebuffer.color_attachment_bytes(attachment)
        );
    }

    let mut too_few = device.commands();
    too_few.begin_render_pass_with_colors(vec![Color::BLACK]);
    too_few.bind_pipeline(pipeline);
    too_few.bind_vertex_buffer(vertices);
    too_few.draw(0, 3);
    too_few.end_render_pass();
    let mut target = Renderer::new(32, 32).unwrap();
    target.clear(Color::WHITE);
    let before = target.framebuffer.bytes().to_vec();
    assert!(device.submit(&too_few, &mut target).is_err());
    assert_eq!(target.framebuffer.bytes(), before);

    let mut invalid_version = loaded;
    invalid_version.version = 2;
    assert!(invalid_version.replay().is_err());
    let legacy_path = std::env::temp_dir()
        .join(format!("silicon-mrt-{}", std::process::id()))
        .join("legacy.silicon");
    assert!(invalid_version.save(&legacy_path).is_err());
    assert!(!legacy_path.exists());
}
