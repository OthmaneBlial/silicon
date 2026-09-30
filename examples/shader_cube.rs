use silicon::*;
fn main() -> Result<()> {
    let capture = demo::shader_cube(960, 640, 0.)?;
    let mut r = Renderer::new(capture.width, capture.height)?;
    let stats = Device.submit(&capture.commands, &mut r)?;
    println!("{stats:?}\n{:?}", r.stats);
    r.framebuffer
        .save_png("assets/screenshots/shader_cube.png")?;
    std::fs::create_dir_all("output")?;
    capture.save("output/cube.silicon")?;
    if !std::env::args().any(|a| a == "--bless") {
        return Ok(());
    }
    demo::shader_cube(96, 64, 0.)?
        .replay()?
        .framebuffer
        .save_png("tests/golden/shader_cube.png")
}
