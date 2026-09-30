use crate::{Color, Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub color: Vec4,
}
impl Vertex {
    pub fn new(position: Vec3, color: Color) -> Self {
        Self {
            position,
            normal: Vec3::new(0., 0., 1.),
            uv: Vec2::ZERO,
            color: color.0,
        }
    }
}
/// Slots: color, UV, normal, world position. Each is a perspective-correct vec4.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct VertexOutput {
    pub position: Vec4,
    pub varyings: [Vec4; 4],
}
impl VertexOutput {
    pub fn lerp(self, b: Self, t: f32) -> Self {
        Self {
            position: self.position.lerp(b.position, t),
            varyings: std::array::from_fn(|i| self.varyings[i].lerp(b.varyings[i], t)),
        }
    }
    pub fn is_finite(&self) -> bool {
        self.position.is_finite() && self.varyings.iter().all(|v| v.is_finite())
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Fragment {
    pub x: u32,
    pub y: u32,
    pub primitive: u32,
    pub depth: f32,
    pub barycentric: Vec3,
    pub varyings: [Vec4; 4],
    pub uv_dx: Vec2,
    pub uv_dy: Vec2,
}
impl Fragment {
    pub fn color(&self) -> Color {
        Color(self.varyings[0])
    }
    pub fn uv(&self) -> Vec2 {
        Vec2::new(self.varyings[1].x, self.varyings[1].y)
    }
    pub fn normal(&self) -> Vec3 {
        self.varyings[2].xyz().normalize()
    }
    pub fn world(&self) -> Vec3 {
        self.varyings[3].xyz()
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum Compare {
    Never,
    #[default]
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
    Always,
}
impl Compare {
    pub fn test<T: PartialOrd>(self, a: T, b: T) -> bool {
        match self {
            Self::Never => false,
            Self::Less => a < b,
            Self::LessEqual => a <= b,
            Self::Greater => a > b,
            Self::GreaterEqual => a >= b,
            Self::Equal => a == b,
            Self::NotEqual => a != b,
            Self::Always => true,
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum Cull {
    #[default]
    None,
    Front,
    Back,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum FrontFace {
    #[default]
    Ccw,
    Cw,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum Blend {
    #[default]
    Replace,
    Alpha,
    Add,
    Multiply,
}
impl Blend {
    pub fn apply(self, src: Color, dst: Color) -> Color {
        let s = src.0;
        let d = dst.0;
        match self {
            Self::Replace => src,
            Self::Alpha => Color(Vec4::new(
                s.x * s.w + d.x * (1. - s.w),
                s.y * s.w + d.y * (1. - s.w),
                s.z * s.w + d.z * (1. - s.w),
                s.w + d.w * (1. - s.w),
            )),
            Self::Add => Color(s + d),
            Self::Multiply => Color(s.component_mul(d)),
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum StencilOp {
    #[default]
    Keep,
    Zero,
    Replace,
    IncrementClamp,
    DecrementClamp,
    Invert,
}
impl StencilOp {
    pub fn apply(self, old: u8, reference: u8) -> u8 {
        match self {
            Self::Keep => old,
            Self::Zero => 0,
            Self::Replace => reference,
            Self::IncrementClamp => old.saturating_add(1),
            Self::DecrementClamp => old.saturating_sub(1),
            Self::Invert => !old,
        }
    }
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct StencilState {
    pub compare: Compare,
    pub reference: u8,
    pub read_mask: u8,
    pub write_mask: u8,
    pub fail: StencilOp,
    pub depth_fail: StencilOp,
    pub pass: StencilOp,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Pipeline {
    pub color_write: bool,
    pub depth_compare: Compare,
    pub depth_write: bool,
    pub cull: Cull,
    pub front_face: FrontFace,
    pub blend: Blend,
    pub stencil: Option<StencilState>,
}
impl Default for Pipeline {
    fn default() -> Self {
        Self {
            color_write: true,
            depth_compare: Compare::Less,
            depth_write: true,
            cull: Cull::None,
            front_face: FrontFace::Ccw,
            blend: Blend::Replace,
            stencil: None,
        }
    }
}
/// Sutherland–Hodgman clipping in homogeneous coordinates, with 0 <= z <= w.
pub fn clip_triangle(triangle: [VertexOutput; 3]) -> Vec<VertexOutput> {
    if !triangle.iter().all(VertexOutput::is_finite) {
        return Vec::new();
    }
    let mut poly = triangle.to_vec();
    for plane in 0..6 {
        let distance = |v: Vec4| match plane {
            0 => v.w + v.x,
            1 => v.w - v.x,
            2 => v.w + v.y,
            3 => v.w - v.y,
            4 => v.z,
            _ => v.w - v.z,
        };
        let mut out = Vec::with_capacity(poly.len() + 1);
        if let Some(&last) = poly.last() {
            let mut a = last;
            let mut da = distance(a.position);
            for &b in &poly {
                let db = distance(b.position);
                if (da >= 0.) != (db >= 0.) {
                    out.push(a.lerp(b, da / (da - db)));
                }
                if db >= 0. {
                    out.push(b);
                }
                a = b;
                da = db;
            }
        }
        poly = out;
    }
    poly
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clips_each_plane_and_interpolates() {
        let v = |p| VertexOutput {
            position: p,
            varyings: [p; 4],
        };
        for p in [
            Vec4::new(-2., 0., 0.5, 1.),
            Vec4::new(2., 0., 0.5, 1.),
            Vec4::new(0., -2., 0.5, 1.),
            Vec4::new(0., 2., 0.5, 1.),
            Vec4::new(0., 0., -1., 1.),
            Vec4::new(0., 0., 2., 1.),
        ] {
            let out = clip_triangle([
                v(p),
                v(Vec4::new(-0.3, -0.3, 0.5, 1.)),
                v(Vec4::new(0.3, 0.3, 0.5, 1.)),
            ]);
            assert!(out.len() >= 3);
            for v in out {
                let p = v.position;
                assert!(
                    p.x.abs() <= p.w + 1e-6
                        && p.y.abs() <= p.w + 1e-6
                        && p.z >= -1e-6
                        && p.z <= p.w + 1e-6
                );
                assert_eq!(p, v.varyings[0]);
            }
        }
    }
    #[test]
    fn depth_blend_stencil() {
        assert!(Compare::Less.test(0.2, 0.3));
        assert!(!Compare::Never.test(0., 1.));
        assert!(Compare::NotEqual.test(1, 2));
        let c = Blend::Alpha.apply(Color::new(1., 0., 0., 0.5), Color::BLACK);
        assert_eq!(c, Color::new(0.5, 0., 0., 1.));
        assert_eq!(StencilOp::IncrementClamp.apply(255, 0), 255);
    }
}
