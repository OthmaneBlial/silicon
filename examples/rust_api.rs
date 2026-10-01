use silicon::api::{Color, Device, Pipeline, Renderer, Result, Sampler, Texture, TextureFormat};
use silicon::api::{Vec2, Vec3, Vec4, Vertex};
use std::sync::Arc;

fn main() -> Result<()> {
    let device = Device::new();
    let vertex = device.create_shader(include_bytes!("../assets/shaders/textured.vert.spv"))?;
    let fragment = device.create_shader(include_bytes!("../assets/shaders/textured.frag.spv"))?;
    let pipeline = device.create_pipeline(&vertex, &fragment, Pipeline::default())?;
    let vertices = [
        vertex_at(-0.8, -0.7, 0., 0., 1.),
        vertex_at(0.8, -0.7, 1., 0., 1.),
        vertex_at(0., 0.8, 0.5, 1., 1.),
    ];
    let uniforms = [
        Vec4::new(1., 0., 0., 0.),
        Vec4::new(0., 1., 0., 0.),
        Vec4::new(0., 0., 1., 0.),
        Vec4::new(0., 0., 0., 1.),
    ];
    let texture = Arc::new(Texture::new(
        1,
        1,
        TextureFormat::Rgba8,
        &[255, 255, 255, 255],
    )?);
    let mut commands = device.commands();
    commands.begin_render_pass(Color::new(0.025, 0.035, 0.055, 1.));
    commands.bind_pipeline(pipeline);
    commands.bind_vertex_buffer(device.create_vertex_buffer(vertices.to_vec())?);
    commands.bind_uniform_buffer(device.create_uniform_buffer(uniforms.to_vec())?);
    commands.bind_texture(0, texture, Sampler::default());
    commands.draw(0, vertices.len() as u32);
    commands.end_render_pass();

    let mut renderer = Renderer::new(320, 240)?;
    let submitted = device.submit(&commands, &mut renderer)?;
    std::fs::create_dir_all("output")?;
    renderer.framebuffer.save_png("output/rust_api.png")?;
    println!("Submitted {} draw(s)", submitted.draws);
    Ok(())
}

fn vertex_at(x: f32, y: f32, u: f32, v: f32, red: f32) -> Vertex {
    Vertex {
        position: Vec3::new(x, y, 0.),
        normal: Vec3::new(0., 0., 1.),
        uv: Vec2::new(u, v),
        color: Vec4::new(red, 1., 1., 1.),
    }
}
