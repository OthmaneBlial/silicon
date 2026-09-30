use crate::*;
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}
impl Mesh {
    pub fn cube() -> Self {
        let mut m = Self::default();
        for (normal, u, v) in [
            (
                Vec3::new(1., 0., 0.),
                Vec3::new(0., 0., -1.),
                Vec3::new(0., 1., 0.),
            ),
            (
                Vec3::new(-1., 0., 0.),
                Vec3::new(0., 0., 1.),
                Vec3::new(0., 1., 0.),
            ),
            (
                Vec3::new(0., 1., 0.),
                Vec3::new(1., 0., 0.),
                Vec3::new(0., 0., -1.),
            ),
            (
                Vec3::new(0., -1., 0.),
                Vec3::new(1., 0., 0.),
                Vec3::new(0., 0., 1.),
            ),
            (
                Vec3::new(0., 0., 1.),
                Vec3::new(1., 0., 0.),
                Vec3::new(0., 1., 0.),
            ),
            (
                Vec3::new(0., 0., -1.),
                Vec3::new(-1., 0., 0.),
                Vec3::new(0., 1., 0.),
            ),
        ] {
            let base = m.vertices.len() as u32;
            for (x, y) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
                m.vertices.push(Vertex {
                    position: normal + u * x + v * y,
                    normal,
                    uv: Vec2::new((x + 1.) * 0.5, (y + 1.) * 0.5),
                    color: Color::WHITE.0,
                });
            }
            m.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        m
    }
    pub fn torus(rings: u32, sides: u32, major: f32, minor: f32) -> Result<Self> {
        if !((3..=512).contains(&rings)
            && (3..=512).contains(&sides)
            && major > 0.
            && minor > 0.
            && major.is_finite()
            && minor.is_finite())
        {
            return Err("invalid torus descriptor".into());
        }
        let mut m = Self::default();
        for r in 0..=rings {
            let u = r as f32 / rings as f32;
            let (su, cu) = (u * std::f32::consts::TAU).sin_cos();
            for s in 0..=sides {
                let v = s as f32 / sides as f32;
                let (sv, cv) = (v * std::f32::consts::TAU).sin_cos();
                let normal = Vec3::new(cu * cv, sv, su * cv);
                m.vertices.push(Vertex {
                    position: Vec3::new(
                        cu * (major + minor * cv),
                        minor * sv,
                        su * (major + minor * cv),
                    ),
                    normal,
                    uv: Vec2::new(u * 8., v * 2.),
                    color: Color::WHITE.0,
                });
            }
        }
        for r in 0..rings {
            for s in 0..sides {
                let a = r * (sides + 1) + s;
                let b = a + sides + 1;
                m.indices.extend([a, a + 1, b, a + 1, b + 1, b]);
            }
        }
        Ok(m)
    }
    /// OBJ v/vt/vn polygon faces, including relative indices; fan triangulation.
    /// Materials are deliberately supplied by the caller, rather than guessed.
    pub fn from_obj(text: &str) -> Result<Self> {
        if text.len() > 32 * 1024 * 1024 {
            return Err("OBJ exceeds 32 MiB limit".into());
        }
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut mesh = Self::default();
        fn index(s: &str, n: usize) -> Result<usize> {
            let i = s.parse::<i64>()?;
            let p = if i > 0 {
                i - 1
            } else if i < 0 {
                n as i64 + i
            } else {
                return Err("OBJ index 0 is invalid".into());
            };
            if p < 0 || p as usize >= n {
                return Err("OBJ face index out of bounds".into());
            }
            Ok(p as usize)
        }
        for (line_number, line) in text.lines().enumerate() {
            let mut words = line.split('#').next().unwrap_or("").split_whitespace();
            let Some(kind) = words.next() else {
                continue;
            };
            let nums = |words: std::str::SplitWhitespace<'_>, count: usize| -> Result<Vec<f32>> {
                let v = words
                    .take(count)
                    .map(|s| s.parse::<f32>().map_err(Into::into))
                    .collect::<Result<Vec<_>>>()?;
                if v.len() != count || v.iter().any(|v| !v.is_finite()) {
                    return Err(format!(
                        "OBJ line {}: expected {count} finite components",
                        line_number + 1
                    )
                    .into());
                }
                Ok(v)
            };
            match kind {
                "v" => {
                    let n = nums(words, 3)?;
                    positions.push(Vec3::new(n[0], n[1], n[2]));
                }
                "vn" => {
                    let n = nums(words, 3)?;
                    normals.push(Vec3::new(n[0], n[1], n[2]).normalize());
                }
                "vt" => {
                    let n = nums(words, 2)?;
                    uvs.push(Vec2::new(n[0], n[1]));
                }
                "f" => {
                    let mut face = Vec::new();
                    for word in words {
                        let pieces: Vec<_> = word.split('/').collect();
                        if pieces.len() > 3 {
                            return Err("OBJ face has too many index components".into());
                        }
                        let p = positions[index(pieces[0], positions.len())?];
                        let uv = if pieces.len() > 1 && !pieces[1].is_empty() {
                            uvs[index(pieces[1], uvs.len())?]
                        } else {
                            Vec2::ZERO
                        };
                        let normal = if pieces.len() > 2 && !pieces[2].is_empty() {
                            normals[index(pieces[2], normals.len())?]
                        } else {
                            Vec3::ZERO
                        };
                        face.push(Vertex {
                            position: p,
                            normal,
                            uv,
                            color: Color::WHITE.0,
                        });
                    }
                    if face.len() < 3 || face.len() > 4096 {
                        return Err("OBJ face requires 3..4096 vertices".into());
                    }
                    for i in 1..face.len() - 1 {
                        let mut tri = [face[0], face[i], face[i + 1]];
                        let n = (tri[1].position - tri[0].position)
                            .cross(tri[2].position - tri[0].position)
                            .normalize();
                        let base = mesh.vertices.len() as u32;
                        for v in &mut tri {
                            if v.normal == Vec3::ZERO {
                                v.normal = n;
                            }
                            mesh.vertices.push(*v);
                        }
                        mesh.indices.extend([base, base + 1, base + 2]);
                    }
                }
                _ => {}
            }
            if mesh.vertices.len() > 1_000_000 || positions.len() > 1_000_000 {
                return Err("OBJ exceeds one million vertices".into());
            }
        }
        if mesh.indices.is_empty() {
            return Err("OBJ has no drawable faces".into());
        }
        Ok(mesh)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn obj_bounds_negative_and_triangulation() {
        let m = Mesh::from_obj("v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf -4 -3 -2 -1\n").unwrap();
        assert_eq!(m.indices.len(), 6);
        assert_eq!(m.vertices[0].normal, Vec3::new(0., 0., 1.));
        assert!(Mesh::from_obj("v 0 0 0\nf 1 2 3").is_err());
        assert!(Mesh::from_obj("v NaN 0 0").is_err());
    }
}
