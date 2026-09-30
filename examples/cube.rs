use silicon::{Result, demo};
fn main() -> Result<()> {
    let r = demo::render("cube", 960, 640, 0.)?;
    println!("{:?}", r.stats);
    r.framebuffer.save_png("assets/screenshots/cube.png")
}
