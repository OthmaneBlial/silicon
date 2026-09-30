use silicon::{Color, Framebuffer, Result};
fn main() -> Result<()> {
    let mut fb = Framebuffer::new(512, 320)?;
    for y in 0..fb.height {
        for x in 0..fb.width {
            fb.set_pixel(x, y, Color::new(x as f32 / 511., y as f32 / 319., 0.35, 1.))?;
        }
    }
    fb.save_png("assets/screenshots/pixel.png")
}
