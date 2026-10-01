use crate::{Result, Vec4};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::BufWriter, path::Path};
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
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
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
}
/// The number of coverage/color/depth/stencil samples stored per pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SampleCount {
    #[default]
    One,
    Two,
    Four,
}
impl SampleCount {
    pub const fn get(self) -> usize {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Four => 4,
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
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
    samples: Option<MultisampleAttachments>,
}
#[derive(Clone)]
struct MultisampleAttachments {
    count: usize,
    colors: Vec<[u8; 4]>,
    depth: Vec<f32>,
    stencil: Vec<u8>,
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
            samples: None,
        })
    }
    pub(crate) fn set_sample_count(&mut self, count: SampleCount) -> Result<()> {
        if self.sample_count() == count {
            return Ok(());
        }
        if count == SampleCount::One {
            self.samples = None;
            return Ok(());
        }
        let sample_count = count.get();
        let sample_len = self
            .depth
            .len()
            .checked_mul(sample_count)
            .ok_or("multisample attachment size overflow")?;
        if sample_len
            .checked_mul(9)
            .is_none_or(|bytes| bytes > 512 * 1024 * 1024)
        {
            return Err("multisample attachments exceed 512 MiB".into());
        }
        let mut colors = Vec::new();
        let mut depth = Vec::new();
        let mut stencil = Vec::new();
        colors
            .try_reserve_exact(sample_len)
            .map_err(|_| "could not allocate multisample color attachment")?;
        depth
            .try_reserve_exact(sample_len)
            .map_err(|_| "could not allocate multisample depth attachment")?;
        stencil
            .try_reserve_exact(sample_len)
            .map_err(|_| "could not allocate multisample stencil attachment")?;
        for index in 0..self.depth.len() {
            let color = self.read(index).rgba8();
            for _ in 0..sample_count {
                colors.push(color);
                depth.push(self.depth[index]);
                stencil.push(self.stencil[index]);
            }
        }
        self.samples = Some(MultisampleAttachments {
            count: sample_count,
            colors,
            depth,
            stencil,
        });
        Ok(())
    }
    pub(crate) fn sample_count(&self) -> SampleCount {
        match self.samples.as_ref().map(|samples| samples.count) {
            Some(2) => SampleCount::Two,
            Some(4) => SampleCount::Four,
            _ => SampleCount::One,
        }
    }
    pub fn clear(&mut self, color: Color) {
        let mut c = color.rgba8();
        if self.format == PixelFormat::Bgra8 {
            c.swap(0, 2);
        }
        for p in self.pixels.chunks_exact_mut(4) {
            p.copy_from_slice(&c);
        }
        if let Some(samples) = &mut self.samples {
            samples.colors.fill(color.rgba8());
        }
    }
    pub fn clear_depth(&mut self, z: f32) {
        self.depth.fill(z);
        if let Some(samples) = &mut self.samples {
            samples.depth.fill(z);
        }
    }
    pub fn clear_stencil(&mut self, value: u8) {
        self.stencil.fill(value);
        if let Some(samples) = &mut self.samples {
            samples.stencil.fill(value);
        }
    }
    pub fn bytes(&self) -> &[u8] {
        &self.pixels
    }
    pub fn set_pixel(&mut self, x: u32, y: u32, c: Color) -> Result<()> {
        if x >= self.width || y >= self.height {
            return Err("pixel outside framebuffer".into());
        }
        let index = (y * self.width + x) as usize;
        self.write(index, c);
        if let Some(samples) = &mut self.samples {
            let start = index * samples.count;
            samples.colors[start..start + samples.count].fill(c.rgba8());
        }
        Ok(())
    }
    pub fn pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.read((y * self.width + x) as usize))
    }
    pub fn stencil_at(&self, x: u32, y: u32) -> Option<u8> {
        if x >= self.width || y >= self.height {
            None
        } else {
            Some(self.stencil[(y * self.width + x) as usize])
        }
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
    pub(crate) fn sample_depth(&self, index: usize, sample: usize) -> f32 {
        self.samples.as_ref().map_or(self.depth[index], |samples| {
            samples.depth[index * samples.count + sample]
        })
    }
    pub(crate) fn sample_stencil(&self, index: usize, sample: usize) -> u8 {
        self.samples
            .as_ref()
            .map_or(self.stencil[index], |samples| {
                samples.stencil[index * samples.count + sample]
            })
    }
    pub(crate) fn write_sample_depth(&mut self, index: usize, sample: usize, z: f32) {
        if let Some(samples) = &mut self.samples {
            let sample_index = index * samples.count + sample;
            samples.depth[sample_index] = z;
        } else {
            self.depth[index] = z;
        }
    }
    pub(crate) fn write_sample_stencil(&mut self, index: usize, sample: usize, value: u8) {
        if let Some(samples) = &mut self.samples {
            let sample_index = index * samples.count + sample;
            samples.stencil[sample_index] = value;
        } else {
            self.stencil[index] = value;
        }
    }
    pub(crate) fn read_sample_color(&self, index: usize, sample: usize) -> Color {
        self.samples.as_ref().map_or_else(
            || self.read(index),
            |samples| Color::from_rgba8(samples.colors[index * samples.count + sample]),
        )
    }
    pub(crate) fn write_sample_color(&mut self, index: usize, sample: usize, color: Color) {
        if let Some(samples) = &mut self.samples {
            let sample_index = index * samples.count + sample;
            samples.colors[sample_index] = color.rgba8();
        } else {
            self.write(index, color);
        }
    }
    pub(crate) fn resolve_samples(&mut self) {
        let Self {
            pixels,
            depth,
            stencil,
            format,
            samples,
            ..
        } = self;
        let Some(samples) = samples.as_ref() else {
            return;
        };
        let count = samples.count;
        for index in 0..depth.len() {
            let start = index * count;
            let mut color = [0u32; 4];
            for sample in &samples.colors[start..start + count] {
                for channel in 0..4 {
                    color[channel] += sample[channel] as u32;
                }
            }
            let mut color =
                color.map(|channel| ((channel + count as u32 / 2) / count as u32) as u8);
            if *format == PixelFormat::Bgra8 {
                color.swap(0, 2);
            }
            pixels[index * 4..index * 4 + 4].copy_from_slice(&color);
            depth[index] = samples.depth[start..start + count]
                .iter()
                .copied()
                .fold(f32::INFINITY, f32::min);
            stencil[index] = samples.stencil[start];
        }
    }
    pub(crate) fn band(&self, y: u32, height: u32) -> Result<Self> {
        if height == 0 || y + height > self.height {
            return Err("framebuffer band outside surface".into());
        }
        let start = (y * self.width) as usize;
        let end = ((y + height) * self.width) as usize;
        let sample_range = |count: usize| start * count..end * count;
        Ok(Self {
            width: self.width,
            height,
            stride: self.stride,
            format: self.format,
            pixels: self.pixels[start * 4..end * 4].to_vec(),
            depth: self.depth[start..end].to_vec(),
            stencil: self.stencil[start..end].to_vec(),
            samples: self.samples.as_ref().map(|samples| MultisampleAttachments {
                count: samples.count,
                colors: samples.colors[sample_range(samples.count)].to_vec(),
                depth: samples.depth[sample_range(samples.count)].to_vec(),
                stencil: samples.stencil[sample_range(samples.count)].to_vec(),
            }),
        })
    }
    pub(crate) fn copy_band_from(&mut self, y: u32, band: &Self) -> Result<()> {
        if band.width != self.width
            || band.height == 0
            || y + band.height > self.height
            || self.sample_count() != band.sample_count()
        {
            return Err("incompatible framebuffer band".into());
        }
        let start = (y * self.width) as usize;
        let end = start + (band.height * self.width) as usize;
        self.pixels[start * 4..end * 4].copy_from_slice(&band.pixels);
        self.depth[start..end].copy_from_slice(&band.depth);
        self.stencil[start..end].copy_from_slice(&band.stencil);
        if let (Some(dst), Some(src)) = (&mut self.samples, &band.samples) {
            let sample_start = start * dst.count;
            let sample_end = end * dst.count;
            dst.colors[sample_start..sample_end].copy_from_slice(&src.colors);
            dst.depth[sample_start..sample_end].copy_from_slice(&src.depth);
            dst.stencil[sample_start..sample_end].copy_from_slice(&src.stencil);
        }
        Ok(())
    }
    pub fn present_into(&self, output: &mut [u32]) -> Result<()> {
        if output.len() != self.pixels.len() / 4 {
            return Err("presentation buffer size mismatch".into());
        }
        for (out, p) in output.iter_mut().zip(self.pixels.chunks_exact(4)) {
            let (r, b) = if self.format == PixelFormat::Rgba8 {
                (p[0], p[2])
            } else {
                (p[2], p[0])
            };
            *out = ((r as u32) << 16) | ((p[1] as u32) << 8) | b as u32;
        }
        Ok(())
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
