//! Small row-major math toolkit. Right-handed world, +Y up, clip depth 0..w.
use serde::{Deserialize, Serialize};
use std::ops::{Add, Div, Mul, Neg, Sub};

macro_rules! vector {
    ($name:ident, $($field:ident),+) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
        pub struct $name { $(pub $field: f32),+ }
        impl $name {
            pub const ZERO: Self = Self { $($field: 0.0),+ };
            pub const fn new($($field: f32),+) -> Self { Self { $($field),+ } }
            pub fn dot(self, rhs: Self) -> f32 { 0.0 $(+ self.$field * rhs.$field)+ }
            pub fn length(self) -> f32 { self.dot(self).sqrt() }
            pub fn normalize(self) -> Self { let n = self.length(); if n > 0.0 { self / n } else { Self::ZERO } }
            pub fn lerp(self, rhs: Self, t: f32) -> Self { self * (1.0-t) + rhs * t }
            pub fn is_finite(self) -> bool { true $(&& self.$field.is_finite())+ }
        }
        impl Add for $name { type Output = Self; fn add(self, rhs: Self) -> Self { Self { $($field: self.$field + rhs.$field),+ } } }
        impl Sub for $name { type Output = Self; fn sub(self, rhs: Self) -> Self { Self { $($field: self.$field - rhs.$field),+ } } }
        impl Mul<f32> for $name { type Output = Self; fn mul(self, rhs: f32) -> Self { Self { $($field: self.$field * rhs),+ } } }
        impl Div<f32> for $name { type Output = Self; fn div(self, rhs: f32) -> Self { Self { $($field: self.$field / rhs),+ } } }
        impl Neg for $name { type Output = Self; fn neg(self) -> Self { self * -1.0 } }
    }
}
vector!(Vec2, x, y);
vector!(Vec3, x, y, z);
vector!(Vec4, x, y, z, w);
impl Vec3 {
    pub fn cross(self, b: Self) -> Self {
        Self::new(
            self.y * b.z - self.z * b.y,
            self.z * b.x - self.x * b.z,
            self.x * b.y - self.y * b.x,
        )
    }
    pub fn extend(self, w: f32) -> Vec4 {
        Vec4::new(self.x, self.y, self.z, w)
    }
}
impl Vec4 {
    pub fn xyz(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }
    pub fn component_mul(self, b: Self) -> Self {
        Self::new(self.x * b.x, self.y * b.y, self.z * b.z, self.w * b.w)
    }
    pub fn to_array(self) -> [f32; 4] {
        [self.x, self.y, self.z, self.w]
    }
    pub fn from_array(v: [f32; 4]) -> Self {
        Self::new(v[0], v[1], v[2], v[3])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mat3(pub [[f32; 3]; 3]);
impl Mat3 {
    pub const IDENTITY: Self = Self([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]);
    pub fn transform(self, v: Vec3) -> Vec3 {
        let a = [v.x, v.y, v.z];
        let r = self
            .0
            .map(|row| row.iter().zip(a).map(|(x, y)| x * y).sum());
        Vec3::new(r[0], r[1], r[2])
    }
    /// Inverse transpose of the affine linear component (normal matrix).
    pub fn normal_matrix(m: Mat4) -> Option<Self> {
        let a = Vec3::new(m.0[0][0], m.0[1][0], m.0[2][0]);
        let b = Vec3::new(m.0[0][1], m.0[1][1], m.0[2][1]);
        let c = Vec3::new(m.0[0][2], m.0[1][2], m.0[2][2]);
        let det = a.dot(b.cross(c));
        if !det.is_finite() || det.abs() < 1e-10 {
            return None;
        }
        let x = b.cross(c) / det;
        let y = c.cross(a) / det;
        let z = a.cross(b) / det;
        Some(Self([[x.x, y.x, z.x], [x.y, y.y, z.y], [x.z, y.z, z.z]]))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mat4(pub [[f32; 4]; 4]);
impl Mat4 {
    pub const IDENTITY: Self = Self([
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ]);
    pub fn translation(v: Vec3) -> Self {
        let mut m = Self::IDENTITY;
        m.0[0][3] = v.x;
        m.0[1][3] = v.y;
        m.0[2][3] = v.z;
        m
    }
    pub fn scale(v: Vec3) -> Self {
        Self([
            [v.x, 0., 0., 0.],
            [0., v.y, 0., 0.],
            [0., 0., v.z, 0.],
            [0., 0., 0., 1.],
        ])
    }
    pub fn rotation_y(a: f32) -> Self {
        let (s, c) = a.sin_cos();
        Self([
            [c, 0., s, 0.],
            [0., 1., 0., 0.],
            [-s, 0., c, 0.],
            [0., 0., 0., 1.],
        ])
    }
    pub fn rotation_x(a: f32) -> Self {
        let (s, c) = a.sin_cos();
        Self([
            [1., 0., 0., 0.],
            [0., c, -s, 0.],
            [0., s, c, 0.],
            [0., 0., 0., 1.],
        ])
    }
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        assert!(
            fov_y > 0.0 && fov_y < std::f32::consts::PI && aspect > 0.0 && near > 0.0 && far > near
        );
        let f = 1.0 / (fov_y * 0.5).tan();
        let z = far / (near - far);
        Self([
            [f / aspect, 0., 0., 0.],
            [0., f, 0., 0.],
            [0., 0., z, z * near],
            [0., 0., -1., 0.],
        ])
    }
    pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        assert!(right > left && top > bottom && far > near);
        Self([
            [
                2. / (right - left),
                0.,
                0.,
                -(right + left) / (right - left),
            ],
            [
                0.,
                2. / (top - bottom),
                0.,
                -(top + bottom) / (top - bottom),
            ],
            [0., 0., 1. / (near - far), near / (near - far)],
            [0., 0., 0., 1.],
        ])
    }
    pub fn look_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let z = (eye - target).normalize();
        let x = up.cross(z).normalize();
        let y = z.cross(x);
        Self([
            [x.x, x.y, x.z, -x.dot(eye)],
            [y.x, y.y, y.z, -y.dot(eye)],
            [z.x, z.y, z.z, -z.dot(eye)],
            [0., 0., 0., 1.],
        ])
    }
    pub fn transform(self, v: Vec4) -> Vec4 {
        let a = v.to_array();
        Vec4::from_array(
            self.0
                .map(|row| row.iter().zip(a).map(|(x, y)| x * y).sum()),
        )
    }
}
impl Mul for Mat4 {
    type Output = Self;
    fn mul(self, b: Self) -> Self {
        let mut out = [[0.; 4]; 4];
        for (i, row) in out.iter_mut().enumerate() {
            for (j, x) in row.iter_mut().enumerate() {
                *x = (0..4).map(|k| self.0[i][k] * b.0[k][j]).sum();
            }
        }
        Self(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transforms_and_depth() {
        let p = Mat4::perspective(1., 1., 0.1, 10.);
        let n = p.transform(Vec3::new(0., 0., -0.1).extend(1.));
        let f = p.transform(Vec3::new(0., 0., -10.).extend(1.));
        assert!(n.z.abs() < 1e-6);
        assert!((f.z / f.w - 1.).abs() < 1e-6);
        assert_eq!(
            (Mat4::translation(Vec3::new(1., 2., 3.)) * Mat4::IDENTITY)
                .transform(Vec4::new(0., 0., 0., 1.)),
            Vec4::new(1., 2., 3., 1.)
        );
        assert_eq!(
            Vec3::new(1., 0., 0.).cross(Vec3::new(0., 1., 0.)),
            Vec3::new(0., 0., 1.)
        );
        let normal = Mat3::normal_matrix(Mat4::scale(Vec3::new(2., 3., 4.)))
            .unwrap()
            .transform(Vec3::new(1., 1., 1.));
        assert_eq!(normal, Vec3::new(0.5, 1. / 3., 0.25));
    }
}
