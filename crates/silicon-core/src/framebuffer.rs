use crate::{Result, Vec4};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::BufWriter, path::Path};
pub const MAX_COLOR_ATTACHMENTS: usize = 4;
const MAX_ATTACHMENT_BYTES: usize = 512 * 1024 * 1024;
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
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    additional_pixels: Vec<Vec<u8>>,
    pub(crate) depth: Vec<f32>,
    pub(crate) stencil: Vec<u8>,
    samples: Option<MultisampleAttachments>,
}
#[derive(Clone)]
struct MultisampleAttachments {
    count: usize,
    colors: Vec<Vec<[u8; 4]>>,
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
        Self::check_attachment_memory(count, 1, SampleCount::One)?;
        Ok(Self {
            width,
            height,
            stride: width as usize * 4,
            format,
            pixels: vec![0; count * 4],
            additional_pixels: Vec::new(),
            depth: vec![1.; count],
            stencil: vec![0; count],
            samples: None,
        })
    }
    fn check_attachment_memory(
        pixels: usize,
        color_attachments: usize,
        samples: SampleCount,
    ) -> Result<()> {
        let bytes_per_pixel = color_attachments
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_add(5))
            .ok_or("framebuffer attachment size overflow")?;
        let base = pixels
            .checked_mul(bytes_per_pixel)
            .ok_or("framebuffer attachment size overflow")?;
        let sample_bytes = if samples == SampleCount::One {
            0
        } else {
            pixels
                .checked_mul(samples.get())
                .and_then(|count| count.checked_mul(bytes_per_pixel))
                .ok_or("framebuffer attachment size overflow")?
        };
        let bytes = base
            .checked_add(sample_bytes)
            .ok_or("framebuffer attachment size overflow")?;
        if bytes > MAX_ATTACHMENT_BYTES {
            return Err("framebuffer attachments exceed 512 MiB".into());
        }
        Ok(())
    }
    pub fn color_attachment_count(&self) -> usize {
        self.additional_pixels.len() + 1
    }
    pub(crate) fn set_color_attachment_count(&mut self, count: usize) -> Result<()> {
        if !(1..=MAX_COLOR_ATTACHMENTS).contains(&count) {
            return Err(
                format!("color attachment count must be 1..={MAX_COLOR_ATTACHMENTS}").into(),
            );
        }
        Self::check_attachment_memory(self.depth.len(), count, self.sample_count())?;
        let additional = count - 1;
        if additional < self.additional_pixels.len() {
            self.additional_pixels.truncate(additional);
            if let Some(samples) = &mut self.samples {
                samples.colors.truncate(count);
            }
            return Ok(());
        }
        let pixel_bytes = self.depth.len() * 4;
        let pixel_count = self.depth.len();
        let mut new_pixels = Vec::new();
        new_pixels
            .try_reserve_exact(additional - self.additional_pixels.len())
            .map_err(|_| "could not allocate color attachments")?;
        for _ in self.additional_pixels.len()..additional {
            let mut pixels = Vec::new();
            pixels
                .try_reserve_exact(pixel_bytes)
                .map_err(|_| "could not allocate color attachment")?;
            pixels.resize(pixel_bytes, 0);
            new_pixels.push(pixels);
        }
        let new_samples = if let Some(samples) = &self.samples {
            let sample_len = pixel_count
                .checked_mul(samples.count)
                .ok_or("multisample attachment size overflow")?;
            let mut colors = Vec::new();
            colors
                .try_reserve_exact(count - samples.colors.len())
                .map_err(|_| "could not allocate multisample color attachments")?;
            for _ in samples.colors.len()..count {
                let mut color = Vec::new();
                color
                    .try_reserve_exact(sample_len)
                    .map_err(|_| "could not allocate multisample color attachment")?;
                color.resize(sample_len, [0; 4]);
                colors.push(color);
            }
            colors
        } else {
            Vec::new()
        };
        self.additional_pixels.extend(new_pixels);
        if let Some(samples) = &mut self.samples {
            samples.colors.extend(new_samples);
        }
        Ok(())
    }
    pub(crate) fn set_sample_count(&mut self, count: SampleCount) -> Result<()> {
        if self.sample_count() == count {
            return Ok(());
        }
        Self::check_attachment_memory(self.depth.len(), self.color_attachment_count(), count)?;
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
        let mut colors = Vec::new();
        colors
            .try_reserve_exact(self.color_attachment_count())
            .map_err(|_| "could not allocate multisample color attachments")?;
        for attachment in 0..self.color_attachment_count() {
            let mut attachment_colors = Vec::new();
            attachment_colors
                .try_reserve_exact(sample_len)
                .map_err(|_| "could not allocate multisample color attachment")?;
            for index in 0..self.depth.len() {
                let color = self.read_attachment(attachment, index).rgba8();
                for _ in 0..sample_count {
                    attachment_colors.push(color);
                }
            }
            colors.push(attachment_colors);
        }
        let mut depth = Vec::new();
        let mut stencil = Vec::new();
        depth
            .try_reserve_exact(sample_len)
            .map_err(|_| "could not allocate multisample depth attachment")?;
        stencil
            .try_reserve_exact(sample_len)
            .map_err(|_| "could not allocate multisample stencil attachment")?;
        for index in 0..self.depth.len() {
            for _ in 0..sample_count {
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
        for attachment in 0..self.color_attachment_count() {
            self.clear_attachment(attachment, color);
        }
    }
    pub(crate) fn clear_attachments(&mut self, colors: &[Color]) -> Result<()> {
        if colors.len() != self.color_attachment_count() {
            return Err("clear color count does not match framebuffer attachments".into());
        }
        for (attachment, &color) in colors.iter().enumerate() {
            self.clear_attachment(attachment, color);
        }
        Ok(())
    }
    fn clear_attachment(&mut self, attachment: usize, color: Color) {
        let mut bytes = color.rgba8();
        if self.format == PixelFormat::Bgra8 {
            bytes.swap(0, 2);
        }
        let pixels = if attachment == 0 {
            &mut self.pixels
        } else {
            &mut self.additional_pixels[attachment - 1]
        };
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&bytes);
        }
        if let Some(samples) = &mut self.samples {
            samples.colors[attachment].fill(color.rgba8());
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
    /// Returns the packed bytes for a color attachment in the framebuffer's pixel format.
    pub fn color_attachment_bytes(&self, attachment: usize) -> Option<&[u8]> {
        if attachment == 0 {
            Some(&self.pixels)
        } else {
            self.additional_pixels
                .get(attachment - 1)
                .map(Vec::as_slice)
        }
    }
    pub fn color_attachment_pixel(&self, attachment: usize, x: u32, y: u32) -> Option<Color> {
        if x >= self.width || y >= self.height || attachment >= self.color_attachment_count() {
            return None;
        }
        Some(self.read_attachment(attachment, (y * self.width + x) as usize))
    }
    pub fn set_pixel(&mut self, x: u32, y: u32, c: Color) -> Result<()> {
        if x >= self.width || y >= self.height {
            return Err("pixel outside framebuffer".into());
        }
        let index = (y * self.width + x) as usize;
        self.write(index, c);
        if let Some(samples) = &mut self.samples {
            let start = index * samples.count;
            samples.colors[0][start..start + samples.count].fill(c.rgba8());
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
        self.read_attachment(0, index)
    }
    fn read_attachment(&self, attachment: usize, index: usize) -> Color {
        let pixels = if attachment == 0 {
            &self.pixels
        } else {
            &self.additional_pixels[attachment - 1]
        };
        let mut c: [u8; 4] = pixels[index * 4..index * 4 + 4].try_into().unwrap();
        if self.format == PixelFormat::Bgra8 {
            c.swap(0, 2);
        }
        Color::from_rgba8(c)
    }
    pub(crate) fn write(&mut self, index: usize, c: Color) {
        self.write_attachment(0, index, c);
    }
    fn write_attachment(&mut self, attachment: usize, index: usize, c: Color) {
        let mut c = c.rgba8();
        if self.format == PixelFormat::Bgra8 {
            c.swap(0, 2);
        }
        let pixels = if attachment == 0 {
            &mut self.pixels
        } else {
            &mut self.additional_pixels[attachment - 1]
        };
        pixels[index * 4..index * 4 + 4].copy_from_slice(&c);
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
    pub(crate) fn read_sample_color(
        &self,
        attachment: usize,
        index: usize,
        sample: usize,
    ) -> Color {
        self.samples.as_ref().map_or_else(
            || self.read_attachment(attachment, index),
            |samples| Color::from_rgba8(samples.colors[attachment][index * samples.count + sample]),
        )
    }
    pub(crate) fn write_sample_color(
        &mut self,
        attachment: usize,
        index: usize,
        sample: usize,
        color: Color,
    ) {
        if let Some(samples) = &mut self.samples {
            let sample_index = index * samples.count + sample;
            samples.colors[attachment][sample_index] = color.rgba8();
        } else {
            self.write_attachment(attachment, index, color);
        }
    }
    pub(crate) fn resolve_samples(&mut self) {
        let Self {
            pixels,
            additional_pixels,
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
        for (pixels, attachment_samples) in std::iter::once(&mut *pixels)
            .chain(additional_pixels.iter_mut())
            .zip(&samples.colors)
        {
            for index in 0..depth.len() {
                let start = index * count;
                let mut color = [0u32; 4];
                for sample in &attachment_samples[start..start + count] {
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
            }
        }
        for index in 0..depth.len() {
            let start = index * count;
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
            additional_pixels: self
                .additional_pixels
                .iter()
                .map(|pixels| pixels[start * 4..end * 4].to_vec())
                .collect(),
            depth: self.depth[start..end].to_vec(),
            stencil: self.stencil[start..end].to_vec(),
            samples: self.samples.as_ref().map(|samples| MultisampleAttachments {
                count: samples.count,
                colors: samples
                    .colors
                    .iter()
                    .map(|colors| colors[sample_range(samples.count)].to_vec())
                    .collect(),
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
            || self.color_attachment_count() != band.color_attachment_count()
        {
            return Err("incompatible framebuffer band".into());
        }
        let start = (y * self.width) as usize;
        let end = start + (band.height * self.width) as usize;
        self.pixels[start * 4..end * 4].copy_from_slice(&band.pixels);
        for (pixels, band_pixels) in self
            .additional_pixels
            .iter_mut()
            .zip(&band.additional_pixels)
        {
            pixels[start * 4..end * 4].copy_from_slice(band_pixels);
        }
        self.depth[start..end].copy_from_slice(&band.depth);
        self.stencil[start..end].copy_from_slice(&band.stencil);
        if let (Some(dst), Some(src)) = (&mut self.samples, &band.samples) {
            let sample_start = start * dst.count;
            let sample_end = end * dst.count;
            for (dst, src) in dst.colors.iter_mut().zip(&src.colors) {
                dst[sample_start..sample_end].copy_from_slice(src);
            }
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
    #[test]
    fn color_targets_keep_independent_clear_values_and_formats() {
        let mut fb = Framebuffer::with_format(2, 1, PixelFormat::Bgra8).unwrap();
        fb.set_color_attachment_count(2).unwrap();
        fb.clear_attachments(&[Color::new(1., 0., 0., 1.), Color::new(0., 0., 1., 1.)])
            .unwrap();
        assert_eq!(fb.color_attachment_count(), 2);
        assert_eq!(
            fb.color_attachment_pixel(0, 1, 0),
            Some(Color::new(1., 0., 0., 1.))
        );
        assert_eq!(
            fb.color_attachment_pixel(1, 1, 0),
            Some(Color::new(0., 0., 1., 1.))
        );
        assert_eq!(fb.bytes(), &[0, 0, 255, 255, 0, 0, 255, 255]);
        assert_eq!(
            fb.color_attachment_bytes(1).unwrap(),
            &[255, 0, 0, 255, 255, 0, 0, 255]
        );
        assert!(
            fb.set_color_attachment_count(MAX_COLOR_ATTACHMENTS + 1)
                .is_err()
        );
        assert!(fb.clear_attachments(&[Color::BLACK]).is_err());
        assert_eq!(fb.color_attachment_count(), 2);
    }
}
