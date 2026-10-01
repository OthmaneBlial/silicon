use crate::{Color, Result, Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};
pub const MAX_ANISOTROPY: u8 = 16;
const MAX_TEXTURE_TEXELS: usize = 16_777_216;
const MAX_TEXTURE_ARRAY_LAYERS: usize = 2048;
const MAX_TEXTURE_3D_MIP_TEXELS: usize = 33_554_432;

fn wrap_coordinate(value: f32, address: Address) -> f32 {
    match address {
        Address::Clamp => value.clamp(0., 1.),
        Address::Repeat => value.rem_euclid(1.),
        Address::Mirror => {
            let value = value.rem_euclid(2.);
            if value > 1. { 2. - value } else { value }
        }
    }
}

fn address_texel(index: i64, size: u32, address: Address) -> u32 {
    match address {
        Address::Clamp => index.clamp(0, size as i64 - 1) as u32,
        Address::Repeat => index.rem_euclid(size as i64) as u32,
        Address::Mirror => {
            let index = index.rem_euclid(2 * size as i64);
            if index >= size as i64 {
                (2 * size as i64 - 1 - index) as u32
            } else {
                index as u32
            }
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
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

/// Same-sized 2D textures sampled from an integer layer coordinate.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TextureArray {
    layers: Vec<Texture>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Texture3DLevel {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pixels: Vec<[u8; 4]>,
}

/// Color volume stored as a bounded chain of 3D mip levels.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Texture3D {
    pub format: TextureFormat,
    levels: Vec<Texture3DLevel>,
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

impl TextureArray {
    pub fn new(layers: Vec<Texture>) -> Result<Self> {
        let array = Self { layers };
        array.validate()?;
        Ok(array)
    }

    pub fn validate(&self) -> Result<()> {
        let Some(first) = self.layers.first() else {
            return Err("texture array requires at least one layer".into());
        };
        if self.layers.len() > MAX_TEXTURE_ARRAY_LAYERS {
            return Err(format!("texture array exceeds {MAX_TEXTURE_ARRAY_LAYERS} layers").into());
        }
        first.validate()?;
        let mut total_texels = 0usize;
        for layer in &self.layers {
            layer.validate()?;
            if layer.format != first.format || layer.levels.len() != first.levels.len() {
                return Err("texture array layers require matching formats and mip counts".into());
            }
            for (level, first_level) in layer.levels.iter().zip(&first.levels) {
                if level.width != first_level.width || level.height != first_level.height {
                    return Err("texture array layers require matching mip dimensions".into());
                }
                total_texels = total_texels
                    .checked_add(
                        (level.width as usize)
                            .checked_mul(level.height as usize)
                            .ok_or("texture array size overflow")?,
                    )
                    .ok_or("texture array size overflow")?;
                if total_texels > MAX_TEXTURE_TEXELS {
                    return Err("texture array exceeds 16M total mip texels".into());
                }
            }
        }
        Ok(())
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    pub fn lod(&self, dx: Vec2, dy: Vec2) -> f32 {
        self.layers[0].lod(dx, dy)
    }

    pub fn sample(&self, uv: Vec2, layer: f32, lod: f32, sampler: Sampler) -> Result<Color> {
        if !layer.is_finite() || layer < 0.0 || layer.fract() != 0.0 {
            return Err("texture-array layer must be a finite non-negative integer".into());
        }
        let layer = layer as usize;
        let texture = self
            .layers
            .get(layer)
            .ok_or_else(|| format!("texture-array layer {layer} is out of bounds"))?;
        texture.sample(uv, lod, sampler)
    }
}

impl Texture3D {
    pub fn new(
        width: u32,
        height: u32,
        depth: u32,
        format: TextureFormat,
        bytes: &[u8],
    ) -> Result<Self> {
        let components = match format {
            TextureFormat::Rgba8 => 4,
            TextureFormat::Rgb8 => 3,
            TextureFormat::R8 => 1,
            TextureFormat::Depth32Float => {
                return Err("3D textures require an R8, RGB8 or RGBA8 format".into());
            }
        };
        let count = (width as usize)
            .checked_mul(height as usize)
            .and_then(|count| count.checked_mul(depth as usize))
            .ok_or("3D texture size overflow")?;
        if width == 0
            || height == 0
            || depth == 0
            || count > MAX_TEXTURE_TEXELS
            || count.checked_mul(components) != Some(bytes.len())
        {
            return Err("invalid 3D texture dimensions or byte length (maximum 16M texels)".into());
        }
        let pixels = bytes
            .chunks_exact(components)
            .map(|pixel| match format {
                TextureFormat::Rgba8 => [pixel[0], pixel[1], pixel[2], pixel[3]],
                TextureFormat::Rgb8 => [pixel[0], pixel[1], pixel[2], 255],
                TextureFormat::R8 => [pixel[0], pixel[0], pixel[0], 255],
                TextureFormat::Depth32Float => unreachable!(),
            })
            .collect();
        let texture = Self {
            format,
            levels: vec![Texture3DLevel {
                width,
                height,
                depth,
                pixels,
            }],
        };
        texture.validate()?;
        Ok(texture)
    }

    pub fn validate(&self) -> Result<()> {
        if self.levels.is_empty() || self.levels.len() > 25 {
            return Err("3D texture requires 1..25 mip levels".into());
        }
        if matches!(self.format, TextureFormat::Depth32Float) {
            return Err("3D textures do not support depth format".into());
        }
        let mut previous: Option<(u32, u32, u32)> = None;
        let mut total_texels = 0usize;
        for level in &self.levels {
            let count = (level.width as usize)
                .checked_mul(level.height as usize)
                .and_then(|count| count.checked_mul(level.depth as usize))
                .ok_or("3D texture size overflow")?;
            if level.width == 0
                || level.height == 0
                || level.depth == 0
                || count > MAX_TEXTURE_TEXELS
                || level.pixels.len() != count
            {
                return Err("invalid 3D texture mip dimensions/storage".into());
            }
            if previous.is_some_and(|(width, height, depth)| {
                level.width != (width / 2).max(1)
                    || level.height != (height / 2).max(1)
                    || level.depth != (depth / 2).max(1)
            }) {
                return Err("invalid 3D texture mip chain dimensions".into());
            }
            total_texels = total_texels
                .checked_add(count)
                .ok_or("3D texture mip size overflow")?;
            if total_texels > MAX_TEXTURE_3D_MIP_TEXELS {
                return Err("3D texture exceeds 32M total mip texels".into());
            }
            previous = Some((level.width, level.height, level.depth));
        }
        Ok(())
    }

    pub fn mip_levels(&self) -> usize {
        self.levels.len()
    }

    pub fn generate_mips(&mut self) {
        self.levels.truncate(1);
        while self
            .levels
            .last()
            .is_some_and(|level| level.width > 1 || level.height > 1 || level.depth > 1)
        {
            let source = self.levels.last().unwrap();
            let (width, height, depth) = (
                (source.width / 2).max(1),
                (source.height / 2).max(1),
                (source.depth / 2).max(1),
            );
            let mut pixels = Vec::with_capacity((width * height * depth) as usize);
            for z in 0..depth {
                for y in 0..height {
                    for x in 0..width {
                        let mut sum = [0u32; 4];
                        let mut count = 0;
                        for source_z in z * source.depth / depth..(z + 1) * source.depth / depth {
                            for source_y in
                                y * source.height / height..(y + 1) * source.height / height
                            {
                                for source_x in
                                    x * source.width / width..(x + 1) * source.width / width
                                {
                                    let pixel = source.pixels[((source_z * source.height
                                        + source_y)
                                        * source.width
                                        + source_x)
                                        as usize];
                                    for component in 0..4 {
                                        sum[component] += pixel[component] as u32;
                                    }
                                    count += 1;
                                }
                            }
                        }
                        pixels.push(sum.map(|value| ((value + count / 2) / count) as u8));
                    }
                }
            }
            self.levels.push(Texture3DLevel {
                width,
                height,
                depth,
                pixels,
            });
        }
    }

    pub fn lod(&self, dx: Vec3, dy: Vec3) -> f32 {
        let level = &self.levels[0];
        let scale = |derivative: Vec3| {
            Vec3::new(
                derivative.x * level.width as f32,
                derivative.y * level.height as f32,
                derivative.z * level.depth as f32,
            )
            .length()
        };
        scale(dx).max(scale(dy)).max(1.).log2()
    }

    pub fn sample(&self, coordinate: Vec3, lod: f32, sampler: Sampler) -> Result<Color> {
        if !coordinate.is_finite() || !lod.is_finite() {
            return Err("3D texture coordinates and LOD must be finite".into());
        }
        let level = match sampler.mip {
            MipFilter::None => 0.,
            _ => lod.clamp(0., (self.levels.len() - 1) as f32),
        };
        let sample_level = |index| self.at(coordinate, index, sampler);
        let color = if matches!(sampler.mip, MipFilter::Trilinear) {
            let low = level.floor() as usize;
            let high = (low + 1).min(self.levels.len() - 1);
            sample_level(low).lerp(sample_level(high), level.fract())
        } else {
            sample_level(level.round() as usize)
        };
        Ok(Color(color))
    }

    fn at(&self, coordinate: Vec3, level: usize, sampler: Sampler) -> Vec4 {
        let mip = &self.levels[level];
        let coordinate = Vec3::new(
            wrap_coordinate(coordinate.x, sampler.address),
            wrap_coordinate(coordinate.y, sampler.address),
            wrap_coordinate(coordinate.z, sampler.address),
        );
        let texel = |x: i64, y: i64, z: i64| {
            let x = address_texel(x, mip.width, sampler.address);
            let y = address_texel(y, mip.height, sampler.address);
            let z = address_texel(z, mip.depth, sampler.address);
            Color::from_rgba8(mip.pixels[((z * mip.height + y) * mip.width + x) as usize]).0
        };
        if matches!(sampler.filter, Filter::Nearest) {
            return texel(
                (coordinate.x * mip.width as f32).floor() as i64,
                (coordinate.y * mip.height as f32).floor() as i64,
                (coordinate.z * mip.depth as f32).floor() as i64,
            );
        }
        let position = Vec3::new(
            coordinate.x * mip.width as f32 - 0.5,
            coordinate.y * mip.height as f32 - 0.5,
            coordinate.z * mip.depth as f32 - 0.5,
        );
        let (x, y, z) = (
            position.x.floor() as i64,
            position.y.floor() as i64,
            position.z.floor() as i64,
        );
        let (fx, fy, fz) = (
            position.x - position.x.floor(),
            position.y - position.y.floor(),
            position.z - position.z.floor(),
        );
        let plane = |z| {
            texel(x, y, z)
                .lerp(texel(x + 1, y, z), fx)
                .lerp(texel(x, y + 1, z).lerp(texel(x + 1, y + 1, z), fx), fy)
        };
        plane(z).lerp(plane(z + 1), fz)
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
        let uv = Vec2::new(
            wrap_coordinate(uv.x, sampler.address),
            wrap_coordinate(uv.y, sampler.address),
        );
        let texel = |x: i64, y: i64| {
            let i = (address_texel(y, mip.height, sampler.address) * mip.width
                + address_texel(x, mip.width, sampler.address)) as usize;
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
    fn cube_edge_taps_follow_direction_across_adjacent_faces() {
        let encode = |direction: Vec3| {
            [direction.x, direction.y, direction.z]
                .map(|component| ((component * 0.5 + 0.5) * 255.).round() as u8)
        };
        let faces = CubeFace::ALL.map(|face| {
            let mut bytes = Vec::new();
            for y in 0..16 {
                for x in 0..16 {
                    let color = encode(CubeMap::face_direction(
                        face,
                        (x as f32 + 0.5) / 16.,
                        (y as f32 + 0.5) / 16.,
                    ));
                    bytes.extend([color[0], color[1], color[2], 255]);
                }
            }
            let mut texture = Texture::new(16, 16, TextureFormat::Rgba8, &bytes).unwrap();
            texture.generate_mips();
            texture
        });
        let cube = CubeMap::new(faces).unwrap();
        let sampler = Sampler {
            filter: Filter::Bilinear,
            mip: MipFilter::Trilinear,
            ..Default::default()
        };
        let check = |direction: Vec3| {
            let expected = encode(direction.normalize());
            let actual = cube.sample(direction, 1.25, sampler).unwrap().rgba8();
            for channel in 0..3 {
                assert!(
                    actual[channel].abs_diff(expected[channel]) <= 24,
                    "direction {direction:?}: {actual:?} vs {expected:?}"
                );
            }
        };
        let epsilon = 0.0001;
        for sign_a in [-1., 1.] {
            for sign_b in [-1., 1.] {
                for tangent in [-0.75, -0.25, 0., 0.25, 0.75] {
                    check(Vec3::new(sign_a * (1. + epsilon), sign_b, tangent));
                    check(Vec3::new(sign_a, sign_b * (1. + epsilon), tangent));
                    check(Vec3::new(sign_a * (1. + epsilon), tangent, sign_b));
                    check(Vec3::new(sign_a, tangent, sign_b * (1. + epsilon)));
                    check(Vec3::new(tangent, sign_a * (1. + epsilon), sign_b));
                    check(Vec3::new(tangent, sign_a, sign_b * (1. + epsilon)));
                }
            }
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

    #[test]
    fn texture_array_selects_checked_integer_layers() {
        let red = Texture::new(1, 1, TextureFormat::Rgba8, &[255, 0, 0, 255]).unwrap();
        let blue = Texture::new(1, 1, TextureFormat::Rgba8, &[0, 0, 255, 255]).unwrap();
        let array = TextureArray::new(vec![red, blue]).unwrap();
        assert_eq!(array.layer_count(), 2);
        assert_eq!(
            array
                .sample(Vec2::new(0.5, 0.5), 1.0, 0.0, Sampler::default())
                .unwrap()
                .rgba8(),
            [0, 0, 255, 255]
        );
        assert!(
            array
                .sample(Vec2::ZERO, 0.5, 0.0, Sampler::default())
                .is_err()
        );
        assert!(
            array
                .sample(Vec2::ZERO, 2.0, 0.0, Sampler::default())
                .is_err()
        );

        let mismatched = vec![
            Texture::new(2, 1, TextureFormat::Rgba8, &[0; 8]).unwrap(),
            Texture::new(1, 2, TextureFormat::Rgba8, &[0; 8]).unwrap(),
        ];
        assert!(TextureArray::new(mismatched).is_err());
    }

    #[test]
    fn volume_texture_filters_xyz_and_builds_mips() {
        let mut bytes = vec![0; 2 * 2 * 2 * 4];
        for pixel in bytes.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        bytes[..4].copy_from_slice(&[255, 255, 255, 255]);
        let mut volume = Texture3D::new(2, 2, 2, TextureFormat::Rgba8, &bytes).unwrap();
        let filtered = volume
            .sample(
                Vec3::new(0.5, 0.5, 0.5),
                0.0,
                Sampler {
                    filter: Filter::Bilinear,
                    mip: MipFilter::None,
                    ..Default::default()
                },
            )
            .unwrap()
            .rgba8();
        assert_eq!(filtered, [32, 32, 32, 255]);

        volume.generate_mips();
        assert_eq!(volume.mip_levels(), 2);
        assert_eq!(
            volume
                .sample(
                    Vec3::new(0.5, 0.5, 0.5),
                    1.0,
                    Sampler {
                        filter: Filter::Nearest,
                        mip: MipFilter::Nearest,
                        ..Default::default()
                    },
                )
                .unwrap()
                .rgba8(),
            [32, 32, 32, 255]
        );
        assert!(Texture3D::new(1, 1, 1, TextureFormat::Depth32Float, &[0; 4]).is_err());
        assert!(Texture3D::new(0, 1, 1, TextureFormat::R8, &[]).is_err());
    }
}
