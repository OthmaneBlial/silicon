use silicon::{Result, demo};
fn main() -> Result<()> {
    let r = demo::render("showcase", 960, 640, 0.)?;
    println!("{:?}", r.stats);
    r.framebuffer.save_png("assets/screenshots/showcase.png")
}
