use crate::{Color, Result, Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};
pub const MAX_ANISOTROPY: u8 = 16;
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub enum TextureFormat {
    Rgba8,
    Rgb8,
    R8,
    Depth32Float,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    depth: Option<Vec<f32>>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Texture {
    pub format: TextureFormat,
    pub levels: Vec<MipLevel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CubeFace {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}
impl CubeFace {
    pub const ALL: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];
}

/// Six square color textures in +X, -X, +Y, -Y, +Z, -Z order.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CubeMap {
    faces: [Texture; 6],
}
impl CubeMap {
    pub fn new(faces: [Texture; 6]) -> Result<Self> {
        let map = Self { faces };
        map.validate()?;
        Ok(map)
    }

    pub fn validate(&self) -> Result<()> {
        let first = &self.faces[0];
        first.validate()?;
        let base = &first.levels[0];
        if base.width != base.height || matches!(first.format, TextureFormat::Depth32Float) {
            return Err("cube faces must be square color textures".into());
        }
        let mut texels = 0usize;
        for face in &self.faces {
            face.validate()?;
            if matches!(face.format, TextureFormat::Depth32Float)
                || face.levels.len() != first.levels.len()
                || face
                    .levels
                    .iter()
                    .zip(&first.levels)
                    .any(|(a, b)| a.width != b.width || a.height != b.height)
            {
                return Err("cube faces require matching color mip dimensions".into());
            }
            for level in &face.levels {
                texels = texels
                    .checked_add(level.width as usize * level.height as usize)
                    .ok_or("cube map size overflow")?;
            }
        }
        if texels > 16_777_216 {
            return Err("cube map exceeds 16M total mip texels".into());
        }
        Ok(())
    }

    /// Direction represented by a point on one face, with UV in [0, 1].
    pub fn face_direction(face: CubeFace, u: f32, v: f32) -> Vec3 {
        let s = u * 2. - 1.;
        let t = v * 2. - 1.;
        match face {
            CubeFace::PositiveX => Vec3::new(1., -t, -s),
            CubeFace::NegativeX => Vec3::new(-1., -t, s),
            CubeFace::PositiveY => Vec3::new(s, 1., t),
            CubeFace::NegativeY => Vec3::new(s, -1., -t),
            CubeFace::PositiveZ => Vec3::new(s, -t, 1.),
            CubeFace::NegativeZ => Vec3::new(-s, -t, -1.),
        }
        .normalize()
    }

    fn face_for(direction: Vec3) -> Option<CubeFace> {
        let (x, y, z) = (direction.x, direction.y, direction.z);
        let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
        if !direction.is_finite() || ax.max(ay).max(az) == 0. {
            return None;
        }
        Some(if ax >= ay && ax >= az {
            if x >= 0. {
                CubeFace::PositiveX
            } else {
                CubeFace::NegativeX
            }
        } else if ay >= az {
            if y >= 0. {
                CubeFace::PositiveY
            } else {
                CubeFace::NegativeY
            }
        } else if z >= 0. {
            CubeFace::PositiveZ
        } else {
            CubeFace::NegativeZ
        })
    }

    fn face_index(face: CubeFace) -> usize {
        match face {
            CubeFace::PositiveX => 0,
            CubeFace::NegativeX => 1,
            CubeFace::PositiveY => 2,
            CubeFace::NegativeY => 3,
            CubeFace::PositiveZ => 4,
            CubeFace::NegativeZ => 5,
        }
    }

    fn project_to_face(face: CubeFace, direction: Vec3) -> Option<Vec2> {
        let (s, t, major) = match face {
            CubeFace::PositiveX => (-direction.z, -direction.y, direction.x),
            CubeFace::NegativeX => (direction.z, -direction.y, -direction.x),
            CubeFace::PositiveY => (direction.x, direction.z, direction.y),
            CubeFace::NegativeY => (direction.x, -direction.z, -direction.y),
            CubeFace::PositiveZ => (direction.x, -direction.y, direction.z),
            CubeFace::NegativeZ => (-direction.x, -direction.y, -direction.z),
        };
        if !direction.is_finite() || major == 0. || !major.is_finite() {
            return None;
        }
        let uv = Vec2::new((s / major + 1.) * 0.5, (t / major + 1.) * 0.5);
        uv.is_finite().then_some(uv)
    }

    pub fn mip_levels(&self) -> usize {
        self.faces[0].levels.len()
    }

    /// Neighbor-center isotropic LOD, projected through the center direction's face.
    pub fn lod(&self, direction: Vec3, dx: Vec3, dy: Vec3) -> f32 {
        let max_lod = (self.mip_levels() - 1) as f32;
        let Some(face) = Self::face_for(direction) else {
            return max_lod;
        };
        let Some(center) = Self::project_to_face(face, direction) else {
            return max_lod;
        };
        let (Some(uv_x), Some(uv_y)) = (
            Self::project_to_face(face, direction + dx),
            Self::project_to_face(face, direction + dy),
        ) else {
            return max_lod;
        };
        let width = self.faces[0].levels[0].width as f32;
        let height = self.faces[0].levels[0].height as f32;
        let rho = Vec2::new((uv_x.x - center.x) * width, (uv_x.y - center.y) * height)
            .length()
            .max(Vec2::new((uv_y.x - center.x) * width, (uv_y.y - center.y) * height).length());
        if rho.is_finite() {
            rho.max(1.).log2()
        } else {
            max_lod
        }
    }

    fn sample_face_level(&self, face: CubeFace, uv: Vec2, level: usize) -> Vec4 {
        let width = self.faces[0].levels[level].width;
        let height = self.faces[0].levels[level].height;
        let sample = |face, uv: Vec2| {
            let x = (uv.x * width as f32).floor().clamp(0., width as f32 - 1.) as u32;
            let y = (uv.y * height as f32).floor().clamp(0., height as f32 - 1.) as u32;
            let image = &self.faces[Self::face_index(face)].levels[level];
            Color::from_rgba8(image.pixels[(y * width + x) as usize]).0
        };
        let texel = |x: i64, y: i64| {
            let outside_x = x < 0 || x >= width as i64;
            let outside_y = y < 0 || y >= height as i64;
            if !outside_x && !outside_y {
                return sample(
                    face,
                    Vec2::new(
                        (x as f32 + 0.5) / width as f32,
                        (y as f32 + 0.5) / height as f32,
                    ),
                );
            }
            let direction = Self::face_direction(
                face,
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
            );
            if outside_x && outside_y {
                let corner = Vec3::new(
                    direction.x.signum(),
                    direction.y.signum(),
                    direction.z.signum(),
                );
                let faces = [
                    if corner.x > 0. {
                        CubeFace::PositiveX
                    } else {
                        CubeFace::NegativeX
                    },
                    if corner.y > 0. {
                        CubeFace::PositiveY
                    } else {
                        CubeFace::NegativeY
                    },
                    if corner.z > 0. {
                        CubeFace::PositiveZ
                    } else {
                        CubeFace::NegativeZ
                    },
                ];
                return faces
                    .into_iter()
                    .map(|face| sample(face, Self::project_to_face(face, corner).unwrap()))
                    .fold(Vec4::ZERO, |sum, color| sum + color)
                    / 3.;
            }
            {
                let adjacent = Self::face_for(direction).unwrap_or(face);
                let adjacent_uv = Self::project_to_face(adjacent, direction).unwrap_or(uv);
                sample(adjacent, adjacent_uv)
            }
        };
        let x = uv.x.clamp(0., 1.) * width as f32 - 0.5;
        let y = uv.y.clamp(0., 1.) * height as f32 - 0.5;
        let ix = x.floor() as i64;
        let iy = y.floor() as i64;
        let fx = x - x.floor();
        let fy = y - y.floor();
        texel(ix, iy)
            .lerp(texel(ix + 1, iy), fx)
            .lerp(texel(ix, iy + 1).lerp(texel(ix + 1, iy + 1), fx), fy)
    }

    pub fn sample(&self, direction: Vec3, lod: f32, sampler: Sampler) -> Result<Color> {
        if !direction.is_finite() || !lod.is_finite() {
            return Err("cube direction and LOD must be finite".into());
        }
        let face = Self::face_for(direction).ok_or("cube direction must be nonzero")?;
        let uv = Self::project_to_face(face, direction).ok_or("invalid cube direction")?;
        if matches!(sampler.filter, Filter::Nearest) {
            return self.faces[Self::face_index(face)].sample(
                uv,
                lod,
                Sampler {
                    address: Address::Clamp,
                    ..sampler
                },
            );
        }
        let level = match sampler.mip {
            MipFilter::None => 0.,
            _ => lod.clamp(0., (self.mip_levels() - 1) as f32),
        };
        let color = if matches!(sampler.mip, MipFilter::Trilinear) {
            let low = level.floor() as usize;
            let high = (low + 1).min(self.mip_levels() - 1);
            self.sample_face_level(face, uv, low)
                .lerp(self.sample_face_level(face, uv, high), level.fract())
        } else {
            self.sample_face_level(face, uv, level.round() as usize)
        };
        Ok(Color(color))
    }
}
impl Texture {
    pub fn validate(&self) -> Result<()> {
        if self.levels.is_empty()
            || self.levels.len() > 25
            || (matches!(self.format, TextureFormat::Depth32Float) && self.levels.len() != 1)
        {
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
                || match self.format {
                    TextureFormat::Depth32Float => {
                        !level.pixels.is_empty()
                            || level.depth.as_ref().is_none_or(|values| {
                                values.len() != count
                                    || values
                                        .iter()
                                        .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
                            })
                    }
                    _ => level.pixels.len() != count || level.depth.is_some(),
                }
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
            TextureFormat::Depth32Float => {
                return Err("use Texture::depth32 for floating-point depth data".into());
            }
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
                TextureFormat::Depth32Float => unreachable!(),
            })
            .collect();
        Ok(Self {
            format,
            levels: vec![MipLevel {
                width,
                height,
                pixels,
                depth: None,
            }],
        })
    }
    /// A single-level 32-bit depth texture, ready to sample from a CPU depth pass.
    pub fn depth32(width: u32, height: u32, values: &[f32]) -> Result<Self> {
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or("texture size overflow")?;
        if width == 0
            || height == 0
            || count > 16_777_216
            || values.len() != count
            || values
                .iter()
                .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
        {
            return Err("invalid depth texture dimensions or values".into());
        }
        Ok(Self {
            format: TextureFormat::Depth32Float,
            levels: vec![MipLevel {
                width,
                height,
                pixels: Vec::new(),
                depth: Some(values.to_vec()),
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
        if matches!(self.format, TextureFormat::Depth32Float) {
            return;
        }
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
                depth: None,
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
    /// Samples along the larger UV derivative and chooses mip detail from the smaller one.
    pub fn sample_anisotropic(
        &self,
        uv: Vec2,
        dx: Vec2,
        dy: Vec2,
        sampler: Sampler,
        max_anisotropy: u8,
    ) -> Result<Color> {
        if !uv.is_finite() || !dx.is_finite() || !dy.is_finite() {
            return Err("texture coordinates and derivatives must be finite".into());
        }
        if !(1..=MAX_ANISOTROPY).contains(&max_anisotropy) {
            return Err(format!("maximum anisotropy must be 1..={MAX_ANISOTROPY}").into());
        }
        let base = &self.levels[0];
        let dx_texels = Vec2::new(dx.x * base.width as f32, dx.y * base.height as f32);
        let dy_texels = Vec2::new(dy.x * base.width as f32, dy.y * base.height as f32);
        let dx_length = dx_texels.length();
        let dy_length = dy_texels.length();
        let (major, major_length, minor_length) = if dx_length >= dy_length {
            (dx, dx_length, dy_length)
        } else {
            (dy, dy_length, dx_length)
        };
        let lod = minor_length
            .max(major_length / max_anisotropy as f32)
            .max(1.)
            .log2();
        let footprint = if matches!(sampler.mip, MipFilter::None) {
            1.
        } else {
            lod.exp2().max(1.)
        };
        let taps = ((major_length / footprint).ceil() as usize).clamp(1, max_anisotropy as usize);
        if taps == 1 {
            return self.sample(uv, lod, sampler);
        }
        let mut color = Vec4::ZERO;
        for tap in 0..taps {
            let offset = (tap as f32 + 0.5) / taps as f32 - 0.5;
            color = color + self.sample(uv + major * offset, lod, sampler)?.0;
        }
        Ok(Color(color / taps as f32))
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
            let i = (address(y, mip.height) * mip.width + address(x, mip.width)) as usize;
            if let Some(depth) = &mip.depth {
                Vec4::new(depth[i], 0., 0., 1.)
            } else {
                Color::from_rgba8(mip.pixels[i]).0
            }
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
        self.levels.get(level).and_then(|l| {
            l.depth
                .is_none()
                .then(|| l.pixels.iter().flatten().copied().collect())
        })
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
    #[test]
    fn anisotropic_sampling_preserves_minor_axis_detail() {
        let mut pixels = Vec::new();
        for y in 0..32 {
            for _ in 0..32 {
                let value = if y % 2 == 0 { 255 } else { 0 };
                pixels.extend([value, value, value, 255]);
            }
        }
        let mut texture = Texture::new(32, 32, TextureFormat::Rgba8, &pixels).unwrap();
        texture.generate_mips();
        let uv = |row: f32| Vec2::new(0.5, row / 32.);
        let dx = Vec2::new(0.25, 0.);
        let dy = Vec2::new(0., 0.001);
        let sampler = Sampler::default();
        let lod = texture.lod(dx, dy);
        let filtered = texture.sample(uv(8.5), lod, sampler).unwrap().0.x;
        let bright = texture
            .sample_anisotropic(uv(8.5), dx, dy, sampler, 16)
            .unwrap()
            .0
            .x;
        let dark = texture
            .sample_anisotropic(uv(9.5), dx, dy, sampler, 16)
            .unwrap()
            .0
            .x;
        assert!((filtered - 0.5).abs() < 0.01);
        assert!(bright > 0.99 && dark < 0.01);
        assert!(
            texture
                .sample_anisotropic(uv(8.5), dx, dy, sampler, MAX_ANISOTROPY + 1)
                .is_err()
        );
    }

    #[test]
    fn depth32_sampling_and_old_texture_serialization() {
        let sampler = Sampler {
            filter: Filter::Nearest,
            address: Address::Clamp,
            mip: MipFilter::None,
        };
        let mut depth = Texture::depth32(2, 2, &[0.125, 0.375, 0.625, 0.875]).unwrap();
        depth.validate().unwrap();
        assert_eq!(depth.mip_bytes(0), None);
        assert_eq!(
            depth.sample(Vec2::new(0.25, 0.25), 0., sampler).unwrap().0,
            Vec4::new(0.125, 0., 0., 1.)
        );
        assert_eq!(
            depth
                .sample(
                    Vec2::new(0.5, 0.5),
                    0.,
                    Sampler {
                        filter: Filter::Bilinear,
                        ..sampler
                    }
                )
                .unwrap()
                .0,
            Vec4::new(0.5, 0., 0., 1.)
        );
        depth.generate_mips();
        assert_eq!(depth.levels.len(), 1);
        let restored: Texture =
            serde_json::from_slice(&serde_json::to_vec(&depth).unwrap()).unwrap();
        assert_eq!(
            restored
                .sample(Vec2::new(0.75, 0.75), 0., sampler)
                .unwrap()
                .0
                .x,
            0.875
        );

        let old: Texture = serde_json::from_slice(
            br#"{"format":"Rgba8","levels":[{"width":1,"height":1,"pixels":[[1,2,3,4]]}]}"#,
        )
        .unwrap();
        old.validate().unwrap();
        assert_eq!(
            old.sample(Vec2::new(0.5, 0.5), 0., sampler)
                .unwrap()
                .rgba8(),
            [1, 2, 3, 4]
        );

        assert!(Texture::depth32(0, 2, &[]).is_err());
        assert!(Texture::depth32(1, 1, &[f32::NAN]).is_err());
        assert!(Texture::depth32(1, 1, &[1.01]).is_err());
        assert!(Texture::new(1, 1, TextureFormat::Depth32Float, &[0]).is_err());
        depth.levels[0].depth.as_mut().unwrap()[0] = f32::INFINITY;
        assert!(depth.validate().is_err());
    }

    #[test]
    fn cube_map_selects_faces_and_rejects_invalid_directions() {
        let colors = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 0, 255],
            [255, 0, 255, 255],
            [0, 255, 255, 255],
        ];
        let faces = colors.map(|color| Texture::new(1, 1, TextureFormat::Rgba8, &color).unwrap());
        let cube = CubeMap::new(faces).unwrap();
        let sampler = Sampler {
            filter: Filter::Nearest,
            mip: MipFilter::None,
            ..Default::default()
        };
        for (face, color) in CubeFace::ALL.into_iter().zip(colors) {
            let direction = CubeMap::face_direction(face, 0.5, 0.5);
            assert_eq!(
                cube.sample(direction * 7., 0., sampler).unwrap().rgba8(),
                color
            );
        }
        assert_eq!(
            cube.sample(Vec3::new(1., 1., 0.), 0., sampler)
                .unwrap()
                .rgba8(),
            colors[0]
        );
        assert!(cube.sample(Vec3::ZERO, 0., sampler).is_err());
        assert!(
            cube.sample(Vec3::new(f32::NAN, 0., 1.), 0., sampler)
                .is_err()
        );
        assert!(
            cube.sample(Vec3::new(0., 0., 1.), f32::INFINITY, sampler)
                .is_err()
        );
    }

    #[test]
    fn cube_bilinear_filter_is_continuous_across_edges_and_corners() {
        let colors = [
            [255, 0, 0, 255],
            [0, 0, 255, 255],
            [0, 255, 0, 255],
            [255, 255, 0, 255],
            [255, 0, 255, 255],
            [0, 255, 255, 255],
        ];
        let faces = colors.map(|color| {
            let mut face = Texture::new(4, 4, TextureFormat::Rgba8, &color.repeat(16)).unwrap();
            face.generate_mips();
            face
        });
        let cube = CubeMap::new(faces).unwrap();
        let epsilon = 0.0001;
        for mip in [MipFilter::None, MipFilter::Nearest, MipFilter::Trilinear] {
            let sampler = Sampler {
                filter: Filter::Bilinear,
                mip,
                ..Default::default()
            };
            let continuous = |from, to| {
                let from = cube.sample(from, 0.5, sampler).unwrap().rgba8();
                let to = cube.sample(to, 0.5, sampler).unwrap().rgba8();
                for channel in 0..4 {
                    assert!(
                        from[channel].abs_diff(to[channel]) <= 2,
                        "{from:?} vs {to:?}"
                    );
                }
                from
            };
            for sign_a in [-1., 1.] {
                for sign_b in [-1., 1.] {
                    for tangent in [-1., -0.5, 0., 0.5, 1.] {
                        continuous(
                            Vec3::new(sign_a * (1. + epsilon), sign_b, tangent),
                            Vec3::new(sign_a, sign_b * (1. + epsilon), tangent),
                        );
                        continuous(
                            Vec3::new(sign_a * (1. + epsilon), tangent, sign_b),
                            Vec3::new(sign_a, tangent, sign_b * (1. + epsilon)),
                        );
                        continuous(
                            Vec3::new(tangent, sign_a * (1. + epsilon), sign_b),
                            Vec3::new(tangent, sign_a, sign_b * (1. + epsilon)),
                        );
                    }
                }
            }
            let center = cube
                .sample(Vec3::new(1., 0.9999, 0.), 0.5, sampler)
                .unwrap()
                .rgba8();
            assert!(center[0] > 120 && center[1] > 120);
        }
    }

    #[test]
    fn cube_lod_projects_neighbors_through_the_center_face() {
        let face = || {
            let mut texture =
                Texture::new(16, 16, TextureFormat::Rgba8, &vec![255; 16 * 16 * 4]).unwrap();
            texture.generate_mips();
            texture
        };
        let cube = CubeMap::new(std::array::from_fn(|_| face())).unwrap();
        let lod = cube.lod(Vec3::new(1., 0.9, 0.), Vec3::new(0., 0.2, 0.), Vec3::ZERO);
        assert!((lod - 1.6f32.log2()).abs() < 1e-6);
        assert_eq!(cube.lod(Vec3::ZERO, Vec3::ZERO, Vec3::ZERO), 4.);
    }

    #[test]
    fn cube_map_requires_matching_square_color_faces() {
        let rgba = |w, h| {
            Texture::new(
                w,
                h,
                TextureFormat::Rgba8,
                &vec![0; w as usize * h as usize * 4],
            )
            .unwrap()
        };
        let mut faces = std::array::from_fn(|_| rgba(2, 2));
        faces[5] = rgba(2, 1);
        assert!(CubeMap::new(faces).is_err());
        let mut faces = std::array::from_fn(|_| rgba(2, 2));
        faces[5].generate_mips();
        assert!(CubeMap::new(faces).is_err());
        let depth = Texture::depth32(2, 2, &[0.5; 4]).unwrap();
        let faces = [
            depth,
            rgba(2, 2),
            rgba(2, 2),
            rgba(2, 2),
            rgba(2, 2),
            rgba(2, 2),
        ];
        assert!(CubeMap::new(faces).is_err());
    }
}
