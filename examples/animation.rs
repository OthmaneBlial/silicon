use silicon::*;
fn main() -> Result<()> {
    let mut r = Renderer::new(640, 400)?;
    for i in 0..96 {
        r.render_bands(4, |r| demo::render_into(r, "showcase", i as f32 / 24.))?;
        r.framebuffer
            .save_png(format!("output/frames/{i:04}.png"))?;
    }
    println!("96 CPU-rendered frames written to output/frames; encode at 24 fps.");
    Ok(())
}
