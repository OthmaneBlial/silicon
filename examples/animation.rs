use silicon::*;
fn main() -> Result<()> {
    let scene = std::env::args().nth(1).unwrap_or_else(|| "showcase".into());
    let output = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "output/frames".into());
    let mut r = Renderer::new(640, 400)?;
    for i in 0..96 {
        r.render_bands(4, |r| demo::render_into(r, &scene, i as f32 / 24.))?;
        r.framebuffer.save_png(format!("{output}/{i:04}.png"))?;
    }
    println!("96 CPU-rendered {scene} frames written to {output}; encode at 24 fps.");
    Ok(())
}
