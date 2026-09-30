use crate::{Color, Result, Vec2};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum TextureFormat {
    Rgba8,
    Rgb8,
    R8,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub enum Filter {
    Nearest,
    #[default]
    Bilinear,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub enum Address {
    Clamp,
    #[default]
    Repeat,
    Mirror,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub enum MipFilter {
    None,
    Nearest,
    #[default]
    Trilinear,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default)]
pub struct Sampler {
    pub filter: Filter,
    pub address: Address,
    pub mip: MipFilter,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pixels: Vec<[u8; 4]>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Texture {
    pub format: TextureFormat,
    pub levels: Vec<MipLevel>,
}
impl Texture {
    pub fn validate(&self) -> Result<()> {
        if self.levels.is_empty() || self.levels.len() > 25 {
            return Err("texture requires 1..25 mip levels".into());
        }
        let mut previous: Option<(u32, u32)> = None;
        for level in &self.levels {
            let count = (level.width as usize)
                .checked_mul(level.height as usize)
                .ok_or("texture size overflow")?;
            if level.width == 0
                || level.height == 0
                || count > 16_777_216
                || count != level.pixels.len()
            {
                return Err("invalid texture mip dimensions/storage".into());
            }
            if previous.is_some_and(|(w, h)| {
                level.width != (w / 2).max(1) || level.height != (h / 2).max(1)
            }) {
                return Err("invalid mip chain dimensions".into());
            }
            previous = Some((level.width, level.height));
        }
        Ok(())
    }

    pub fn new(width: u32, height: u32, format: TextureFormat, bytes: &[u8]) -> Result<Self> {
        let components = match format {
            TextureFormat::Rgba8 => 4,
            TextureFormat::Rgb8 => 3,
            TextureFormat::R8 => 1,
        };
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or("texture size overflow")?;
        if width == 0
            || height == 0
            || count > 16_777_216
            || count.checked_mul(components) != Some(bytes.len())
        {
            return Err("invalid texture dimensions or byte length (maximum 16M texels)".into());
        }
        let pixels = bytes
            .chunks_exact(components)
            .map(|p| match format {
                TextureFormat::Rgba8 => [p[0], p[1], p[2], p[3]],
                TextureFormat::Rgb8 => [p[0], p[1], p[2], 255],
                TextureFormat::R8 => [p[0], p[0], p[0], 255],
            })
            .collect();
        Ok(Self {
            format,
            levels: vec![MipLevel {
                width,
                height,
                pixels,
            }],
        })
    }
    pub fn checker(size: u32) -> Result<Self> {
        if size == 0 || size > 4096 {
            return Err("checker size must be 1..4096".into());
        }
        let mut bytes = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let tile = (size / 8).max(1);
                let c = if ((x / tile) + (y / tile)).is_multiple_of(2) {
                    [238, 226, 188, 255]
                } else {
                    [22, 94, 111, 255]
                };
                bytes.extend(c);
            }
        }
        let mut t = Self::new(size, size, TextureFormat::Rgba8, &bytes)?;
        t.generate_mips();
        Ok(t)
    }
    pub fn generate_mips(&mut self) {
        self.levels.truncate(1);
        while self
            .levels
            .last()
            .is_some_and(|l| l.width > 1 || l.height > 1)
        {
            let src = self.levels.last().unwrap();
            let w = (src.width / 2).max(1);
            let h = (src.height / 2).max(1);
            let mut pixels = Vec::with_capacity((w * h) as usize);
            for y in 0..h {
                for x in 0..w {
                    let mut sum = [0u32; 4];
                    let mut count = 0;
                    for sy in y * src.height / h..(y + 1) * src.height / h {
                        for sx in x * src.width / w..(x + 1) * src.width / w {
                            let c = src.pixels[(sy * src.width + sx) as usize];
                            for i in 0..4 {
                                sum[i] += c[i] as u32;
                            }
                            count += 1;
                        }
                    }
                    pixels.push(sum.map(|s| ((s + count / 2) / count) as u8));
                }
            }
            self.levels.push(MipLevel {
                width: w,
                height: h,
                pixels,
            });
        }
    }
    pub fn lod(&self, dx: Vec2, dy: Vec2) -> f32 {
        let w = self.levels[0].width as f32;
        let h = self.levels[0].height as f32;
        Vec2::new(dx.x * w, dx.y * h)
            .length()
            .max(Vec2::new(dy.x * w, dy.y * h).length())
            .max(1.)
            .log2()
    }
    pub fn sample(&self, uv: Vec2, lod: f32, sampler: Sampler) -> Result<Color> {
        if !uv.is_finite() || !lod.is_finite() {
            return Err("texture coordinates and LOD must be finite".into());
        }
        let level = match sampler.mip {
            MipFilter::None => 0.,
            _ => lod.clamp(0., (self.levels.len() - 1) as f32),
        };
        let c = match sampler.mip {
            MipFilter::Trilinear => {
                let lo = level.floor() as usize;
                let hi = (lo + 1).min(self.levels.len() - 1);
                self.at(uv, lo, sampler)
                    .0
                    .lerp(self.at(uv, hi, sampler).0, level.fract())
            }
            _ => self.at(uv, level.round() as usize, sampler).0,
        };
        Ok(Color(c))
    }
    fn at(&self, uv: Vec2, level: usize, sampler: Sampler) -> Color {
        let mip = &self.levels[level];
        let wrap = |v: f32| match sampler.address {
            Address::Clamp => v.clamp(0., 1.),
            Address::Repeat => v.rem_euclid(1.),
            Address::Mirror => {
                let t = v.rem_euclid(2.);
                if t > 1. { 2. - t } else { t }
            }
        };
        let uv = Vec2::new(wrap(uv.x), wrap(uv.y));
        let address = |i: i64, n: u32| match sampler.address {
            Address::Clamp => i.clamp(0, n as i64 - 1) as u32,
            Address::Repeat => i.rem_euclid(n as i64) as u32,
            Address::Mirror => {
                let j = i.rem_euclid(2 * n as i64);
                if j >= n as i64 {
                    (2 * n as i64 - 1 - j) as u32
                } else {
                    j as u32
                }
            }
        };
        let texel = |x: i64, y: i64| {
            Color::from_rgba8(
                mip.pixels[(address(y, mip.height) * mip.width + address(x, mip.width)) as usize],
            )
            .0
        };
        match sampler.filter {
            Filter::Nearest => Color(texel(
                (uv.x * mip.width as f32).floor() as i64,
                (uv.y * mip.height as f32).floor() as i64,
            )),
            Filter::Bilinear => {
                let x = uv.x * mip.width as f32 - 0.5;
                let y = uv.y * mip.height as f32 - 0.5;
                let ix = x.floor() as i64;
                let iy = y.floor() as i64;
                let fx = x - x.floor();
                let fy = y - y.floor();
                Color(
                    texel(ix, iy)
                        .lerp(texel(ix + 1, iy), fx)
                        .lerp(texel(ix, iy + 1).lerp(texel(ix + 1, iy + 1), fx), fy),
                )
            }
        }
    }
    pub fn mip_bytes(&self, level: usize) -> Option<Vec<u8>> {
        self.levels
            .get(level)
            .map(|l| l.pixels.iter().flatten().copied().collect())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec4;
    #[test]
    fn exact_sampling_addressing_and_mips() {
        let mut t = Texture::new(
            2,
            2,
            TextureFormat::Rgba8,
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        )
        .unwrap();
        let s = Sampler {
            filter: Filter::Nearest,
            address: Address::Clamp,
            mip: MipFilter::None,
        };
        assert_eq!(
            t.sample(Vec2::new(0.25, 0.25), 0., s).unwrap().rgba8(),
            [255, 0, 0, 255]
        );
        assert_eq!(
            t.sample(Vec2::new(1., 1.), 0., s).unwrap().rgba8(),
            [255, 255, 255, 255]
        );
        assert_eq!(
            t.sample(
                Vec2::new(0.5, 0.5),
                0.,
                Sampler {
                    filter: Filter::Bilinear,
                    ..s
                }
            )
            .unwrap()
            .0,
            Vec4::new(0.5, 0.5, 0.5, 1.)
        );
        assert_eq!(
            t.sample(
                Vec2::new(-0.75, 0.25),
                0.,
                Sampler {
                    address: Address::Repeat,
                    ..s
                }
            )
            .unwrap()
            .rgba8(),
            [255, 0, 0, 255]
        );
        t.generate_mips();
        assert_eq!(t.levels.len(), 2);
        assert_eq!(t.mip_bytes(1).unwrap(), [128, 128, 128, 255]);
        assert!(t.sample(Vec2::new(f32::NAN, 0.), 0., s).is_err());
        let mut odd = Texture::new(3, 1, TextureFormat::R8, &[0, 0, 255]).unwrap();
        odd.generate_mips();
        assert_eq!(odd.mip_bytes(1).unwrap(), [85, 85, 85, 255]);
    }
}
