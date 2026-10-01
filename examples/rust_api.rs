use silicon::{Cull, Device, Pipeline, Renderer, Result, demo};

fn main() -> Result<()> {
    let device = Device::new();
    let vertex = device.create_shader(include_bytes!("../assets/shaders/textured.vert.spv"))?;
    let fragment = device.create_shader(include_bytes!("../assets/shaders/textured.frag.spv"))?;
    let pipeline = device.create_pipeline(
        &vertex,
        &fragment,
        Pipeline {
            cull: Cull::Back,
            ..Default::default()
        },
    )?;

    let capture = demo::shader_cube_with_pipeline(320, 240, 0., pipeline)?;
    let mut renderer = Renderer::new(capture.width, capture.height)?;
    let submitted = device.submit(&capture.commands, &mut renderer)?;
    std::fs::create_dir_all("output")?;
    renderer.framebuffer.save_png("output/rust_api.png")?;
    println!("Submitted {} draw(s)", submitted.draws);
    Ok(())
}
