use silicon::{Result, demo};
fn main() -> Result<()> {
    let r = demo::render("triangle_3d", 960, 640, 0.)?;
    println!("{:?}", r.stats);
    r.framebuffer.save_png("assets/screenshots/triangle_3d.png")
}
