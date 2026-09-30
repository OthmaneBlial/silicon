use silicon::*;
fn main() -> Result<()> {
    let mut renderer = Renderer::new(960, 640)?;
    demo::render_into(&mut renderer, "stencil", 0.)?;
    renderer
        .framebuffer
        .save_png("assets/screenshots/stencil.png")
}
