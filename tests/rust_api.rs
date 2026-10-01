use silicon::api::{
    Color, Device, Pipeline, Renderer, Sampler, Texture, TextureFormat, Vec2, Vec3, Vec4, Vertex,
};
use std::sync::Arc;

#[test]
fn supported_rust_api_renders_a_triangle() {
    let device = Device::new();
    let vertex_shader = device
        .create_shader(include_bytes!("../assets/shaders/textured.vert.spv"))
        .unwrap();
    let fragment_shader = device
        .create_shader(include_bytes!("../assets/shaders/textured.frag.spv"))
        .unwrap();
    let pipeline = device
        .create_pipeline(&vertex_shader, &fragment_shader, Pipeline::default())
        .unwrap();
    let vertices = [
        vertex_at(-0.8, -0.7, 0., 0.),
        vertex_at(0.8, -0.7, 1., 0.),
        vertex_at(0., 0.8, 0.5, 1.),
    ];
    let uniforms = [
        Vec4::new(1., 0., 0., 0.),
        Vec4::new(0., 1., 0., 0.),
        Vec4::new(0., 0., 1., 0.),
        Vec4::new(0., 0., 0., 1.),
    ];
    let texture =
        Arc::new(Texture::new(1, 1, TextureFormat::Rgba8, &[255, 255, 255, 255]).unwrap());
    let mut commands = device.commands();
    commands.begin_render_pass(Color::BLACK);
    commands.bind_pipeline(pipeline);
    commands.bind_vertex_buffer(device.create_vertex_buffer(vertices.to_vec()).unwrap());
    commands.bind_uniform_buffer(device.create_uniform_buffer(uniforms.to_vec()).unwrap());
    commands.bind_texture(0, texture, Sampler::default());
    commands.draw(0, vertices.len() as u32);
    commands.end_render_pass();

    let mut renderer = Renderer::new(32, 32).unwrap();
    let submission = device.submit(&commands, &mut renderer).unwrap();
    assert_eq!(submission.draws, 1);
    assert!(
        renderer
            .framebuffer
            .bytes()
            .chunks_exact(4)
            .any(|pixel| pixel != Color::BLACK.rgba8())
    );
}

fn vertex_at(x: f32, y: f32, u: f32, v: f32) -> Vertex {
    Vertex {
        position: Vec3::new(x, y, 0.),
        normal: Vec3::new(0., 0., 1.),
        uv: Vec2::new(u, v),
        color: Vec4::new(1., 1., 1., 1.),
    }
}
