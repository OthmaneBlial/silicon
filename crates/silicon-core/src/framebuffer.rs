use crate::{Result, Vec4};
use std::{fs::File, io::BufWriter, path::Path};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub Vec4);
impl Color {
    pub const BLACK: Self = Self(Vec4::new(0., 0., 0., 1.));
    pub const WHITE: Self = Self(Vec4::new(1., 1., 1., 1.));
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self(Vec4::new(r, g, b, a))
    }
    pub fn rgba8(self) -> [u8; 4] {
        self.0
            .to_array()
            .map(|v| (v.clamp(0., 1.) * 255.).round() as u8)
    }
    pub fn from_rgba8(v: [u8; 4]) -> Self {
        Self(Vec4::from_array(v.map(|x| x as f32 / 255.)))
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub min_depth: f32,
    pub max_depth: f32,
}
/// Tightly packed by default; stride is measured in bytes. Maximum 16M pixels.
#[derive(Clone)]
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
    pub(crate) pixels: Vec<u8>,
    pub(crate) depth: Vec<f32>,
    pub(crate) stencil: Vec<u8>,
}
impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Self::with_format(width, height, PixelFormat::Rgba8)
    }
    pub fn with_format(width: u32, height: u32, format: PixelFormat) -> Result<Self> {
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or("framebuffer size overflow")?;
        if width == 0 || height == 0 || count > 16_777_216 {
            return Err("framebuffer requires 1..16777216 pixels".into());
        }
        Ok(Self {
            width,
            height,
            stride: width as usize * 4,
            format,
            pixels: vec![0; count * 4],
            depth: vec![1.; count],
            stencil: vec![0; count],
        })
    }
    pub fn clear(&mut self, color: Color) {
        let mut c = color.rgba8();
        if self.format == PixelFormat::Bgra8 {
            c.swap(0, 2);
        }
        for p in self.pixels.chunks_exact_mut(4) {
            p.copy_from_slice(&c);
        }
    }
    pub fn clear_depth(&mut self, z: f32) {
        self.depth.fill(z);
    }
    pub fn clear_stencil(&mut self, value: u8) {
        self.stencil.fill(value);
    }
    pub fn bytes(&self) -> &[u8] {
        &self.pixels
    }
    pub fn set_pixel(&mut self, x: u32, y: u32, c: Color) -> Result<()> {
        if x >= self.width || y >= self.height {
            return Err("pixel outside framebuffer".into());
        }
        self.write((y * self.width + x) as usize, c);
        Ok(())
    }
    pub fn pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.read((y * self.width + x) as usize))
    }
    pub fn depth_at(&self, x: u32, y: u32) -> Option<f32> {
        if x >= self.width || y >= self.height {
            None
        } else {
            Some(self.depth[(y * self.width + x) as usize])
        }
    }
    pub(crate) fn read(&self, index: usize) -> Color {
        let mut c: [u8; 4] = self.pixels[index * 4..index * 4 + 4].try_into().unwrap();
        if self.format == PixelFormat::Bgra8 {
            c.swap(0, 2);
        }
        Color::from_rgba8(c)
    }
    pub(crate) fn write(&mut self, index: usize, c: Color) {
        let mut c = c.rgba8();
        if self.format == PixelFormat::Bgra8 {
            c.swap(0, 2);
        }
        self.pixels[index * 4..index * 4 + 4].copy_from_slice(&c);
    }
    pub fn present_buffer(&self) -> Vec<u32> {
        self.pixels
            .chunks_exact(4)
            .map(|p| {
                let (r, b) = if self.format == PixelFormat::Rgba8 {
                    (p[0], p[2])
                } else {
                    (p[2], p[0])
                };
                ((r as u32) << 16) | ((p[1] as u32) << 8) | b as u32
            })
            .collect()
    }
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut encoder =
            png::Encoder::new(BufWriter::new(File::create(path)?), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        if self.format == PixelFormat::Rgba8 {
            writer.write_image_data(&self.pixels)?;
        } else {
            let mut bytes = self.pixels.clone();
            for p in bytes.chunks_exact_mut(4) {
                p.swap(0, 2);
            }
            writer.write_image_data(&bytes)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_pixels_and_bounds() {
        for format in [PixelFormat::Rgba8, PixelFormat::Bgra8] {
            let mut fb = Framebuffer::with_format(3, 2, format).unwrap();
            let c = Color::from_rgba8([4, 80, 221, 255]);
            fb.clear(c);
            assert_eq!(fb.pixel(2, 1), Some(c));
            assert!(fb.set_pixel(3, 0, c).is_err());
            assert_eq!(fb.stride, 12);
            assert_eq!(fb.bytes().len(), 24);
            assert_eq!(fb.present_buffer()[0], 0x0450dd);
        }
        assert!(Framebuffer::new(u32::MAX, 2).is_err());
    }
}
