//! SILICON owns framebuffer memory and every stage that produces scene pixels.
mod framebuffer;
pub use framebuffer::*;
pub use silicon_math::{Mat3, Mat4, Vec2, Vec3, Vec4};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
