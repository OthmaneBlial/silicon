use silicon::*;
fn main() -> Result<()> {
    let mut r = Renderer::new(800, 600)?;
    r.clear(Color::new(0.025, 0.035, 0.055, 1.));
    let vertices = [
        Vertex::new(Vec3::new(-0.8, -0.7, 0.5), Color::new(1., 0.15, 0.1, 1.)),
        Vertex::new(Vec3::new(0.8, -0.7, 0.5), Color::new(0.1, 1., 0.35, 1.)),
        Vertex::new(Vec3::new(0., 0.8, 0.5), Color::new(0.15, 0.3, 1., 1.)),
    ];
    r.draw(
        &vertices,
        None,
        Pipeline::default(),
        |v| VertexOutput {
            position: v.position.extend(1.),
            varyings: [v.color, Vec4::ZERO, Vec4::ZERO, Vec4::ZERO],
        },
        |f| Some(f.color()),
    )?;
    println!("{} covered fragments", r.stats.fragments);
    r.framebuffer.save_png("assets/screenshots/triangle.png")
}
