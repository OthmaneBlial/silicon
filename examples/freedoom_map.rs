//! Render Freedoom's E1M1 geometry through SILICON's programmable pipeline.
use silicon::api::{self, Color, Device, Pipeline, Renderer, Vec2, Vec3, Vec4, Vertex};
use silicon_math::Mat4;
use std::{fs, io, path::Path};

const MAP_LUMPS: [&str; 11] = [
    "E1M1", "THINGS", "LINEDEFS", "SIDEDEFS", "VERTEXES", "SEGS", "SSECTORS", "NODES", "SECTORS",
    "REJECT", "BLOCKMAP",
];
const MAX_WAD_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Lump {
    name: [u8; 8],
    offset: usize,
    size: usize,
}

impl Lump {
    fn name(&self) -> String {
        String::from_utf8_lossy(&self.name)
            .trim_end_matches('\0')
            .to_owned()
    }
}

#[derive(Clone, Copy)]
struct Vertex2 {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct Sector {
    floor: f32,
    ceiling: f32,
    light: u8,
}

struct Map {
    vertices: Vec<Vertex2>,
    sectors: Vec<Sector>,
    sides: Vec<u16>,
    lines: Vec<[u16; 4]>, // endpoints, side 0, side 1
    segs: Vec<[u16; 4]>,  // endpoints, linedef, side
    subsectors: Vec<[u16; 2]>,
    things: Vec<(i16, i16, u16, u16)>, // x, y, angle, type
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn u16_at(data: &[u8], offset: usize) -> Result<u16, io::Error> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| invalid("truncated 16-bit value"))?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

fn i16_at(data: &[u8], offset: usize) -> Result<i16, io::Error> {
    Ok(u16_at(data, offset)? as i16)
}

fn wad_lumps(data: &[u8]) -> Result<Vec<Lump>, io::Error> {
    if data.len() < 12 || !matches!(&data[..4], b"IWAD" | b"PWAD") {
        return Err(invalid("not a classic IWAD/PWAD"));
    }
    let count = i32::from_le_bytes(data[4..8].try_into().unwrap());
    let directory = i32::from_le_bytes(data[8..12].try_into().unwrap());
    if count < 0 || directory < 0 {
        return Err(invalid("negative WAD directory range"));
    }
    let count = count as usize;
    if count > 1_000_000 {
        return Err(invalid("WAD directory has too many lumps"));
    }
    let directory = directory as usize;
    let end = directory
        .checked_add(
            count
                .checked_mul(16)
                .ok_or_else(|| invalid("directory overflow"))?,
        )
        .filter(|&end| end <= data.len())
        .ok_or_else(|| invalid("WAD directory exceeds file"))?;
    let mut lumps = Vec::with_capacity(count);
    for entry in data[directory..end].chunks_exact(16) {
        let offset = u32::from_le_bytes(entry[0..4].try_into().unwrap()) as usize;
        let size = u32::from_le_bytes(entry[4..8].try_into().unwrap()) as usize;
        if offset
            .checked_add(size)
            .filter(|&end| end <= data.len())
            .is_none()
        {
            return Err(invalid("WAD lump exceeds file"));
        }
        lumps.push(Lump {
            name: entry[8..16].try_into().unwrap(),
            offset,
            size,
        });
    }
    Ok(lumps)
}

fn parse_map(data: &[u8]) -> Result<Map, io::Error> {
    let lumps = wad_lumps(data)?;
    let marker = lumps
        .iter()
        .position(|lump| lump.name() == MAP_LUMPS[0])
        .ok_or_else(|| invalid("E1M1 not found"))?;
    let map_lumps = lumps
        .get(marker..marker + MAP_LUMPS.len())
        .ok_or_else(|| invalid("truncated E1M1 lump sequence"))?;
    for (lump, expected) in map_lumps.iter().zip(MAP_LUMPS) {
        if lump.name() != expected {
            return Err(invalid(format!("expected {expected} after E1M1")));
        }
    }
    let bytes = |index: usize| {
        let lump = map_lumps[index];
        &data[lump.offset..lump.offset + lump.size]
    };
    let records = |index: usize, width: usize| -> Result<Vec<&[u8]>, io::Error> {
        let lump = bytes(index);
        if lump.len() % width != 0 {
            return Err(invalid(format!(
                "{} has a partial record",
                MAP_LUMPS[index]
            )));
        }
        Ok(lump.chunks_exact(width).collect())
    };

    let vertices: Vec<Vertex2> = records(4, 4)?
        .into_iter()
        .map(|r| {
            Ok(Vertex2 {
                x: i16_at(r, 0)? as f32,
                y: i16_at(r, 2)? as f32,
            })
        })
        .collect::<Result<_, io::Error>>()?;
    let sectors: Vec<Sector> = records(8, 26)?
        .into_iter()
        .map(|r| {
            let light = i16_at(r, 20)?.clamp(0, 255) as u8;
            Ok(Sector {
                floor: i16_at(r, 0)? as f32,
                ceiling: i16_at(r, 2)? as f32,
                light,
            })
        })
        .collect::<Result<_, io::Error>>()?;
    let sides: Vec<u16> = records(3, 30)?
        .into_iter()
        .map(|r| u16_at(r, 28))
        .collect::<Result<_, _>>()?;
    let lines: Vec<[u16; 4]> = records(2, 14)?
        .into_iter()
        .map(|r| Ok([u16_at(r, 0)?, u16_at(r, 2)?, u16_at(r, 10)?, u16_at(r, 12)?]))
        .collect::<Result<_, io::Error>>()?;
    let segs: Vec<[u16; 4]> = records(5, 12)?
        .into_iter()
        .map(|r| Ok([u16_at(r, 0)?, u16_at(r, 2)?, u16_at(r, 6)?, u16_at(r, 8)?]))
        .collect::<Result<_, io::Error>>()?;
    let subsectors: Vec<[u16; 2]> = records(6, 4)?
        .into_iter()
        .map(|r| Ok([u16_at(r, 0)?, u16_at(r, 2)?]))
        .collect::<Result<_, io::Error>>()?;
    let things: Vec<(i16, i16, u16, u16)> = records(1, 10)?
        .into_iter()
        .map(|r| Ok((i16_at(r, 0)?, i16_at(r, 2)?, u16_at(r, 4)?, u16_at(r, 6)?)))
        .collect::<Result<_, io::Error>>()?;

    for line in &lines {
        if line[0] as usize >= vertices.len()
            || line[1] as usize >= vertices.len()
            || (line[2] != u16::MAX && line[2] as usize >= sides.len())
            || (line[3] != u16::MAX && line[3] as usize >= sides.len())
        {
            return Err(invalid("LINEDEFS references an invalid vertex or side"));
        }
    }
    for &sector in &sides {
        if sector as usize >= sectors.len() {
            return Err(invalid("SIDEDEFS references an invalid sector"));
        }
    }
    for seg in &segs {
        if seg[0] as usize >= vertices.len()
            || seg[1] as usize >= vertices.len()
            || seg[2] as usize >= lines.len()
            || seg[3] > 1
        {
            return Err(invalid("SEGS references an invalid vertex, line, or side"));
        }
        let side = lines[seg[2] as usize][2 + seg[3] as usize];
        if side == u16::MAX || side as usize >= sides.len() {
            return Err(invalid("SEGS references a missing sidedef"));
        }
    }
    for leaf in &subsectors {
        if leaf[1] as usize + leaf[0] as usize > segs.len() {
            return Err(invalid("SSECTORS references invalid SEGS"));
        }
    }
    Ok(Map {
        vertices,
        sectors,
        sides,
        lines,
        segs,
        subsectors,
        things,
    })
}

fn world(point: Vertex2, height: f32) -> Vec3 {
    Vec3::new(point.x, height, -point.y)
}

fn shaded(rgb: [f32; 3], sector: Sector) -> Vec4 {
    let light = 0.38 + sector.light as f32 / 255.0 * 0.62;
    Vec4::new(rgb[0] * light, rgb[1] * light, rgb[2] * light, 1.0)
}

fn push_triangle(out: &mut Vec<Vertex>, a: Vec3, b: Vec3, c: Vec3, color: Vec4) {
    if (b - a).cross(c - a).length() > 0.01 {
        for position in [a, b, c] {
            out.push(Vertex {
                position,
                normal: Vec3::new(0.0, 1.0, 0.0),
                uv: Vec2::ZERO,
                color,
            });
        }
    }
}

fn push_quad(out: &mut Vec<Vertex>, a: Vertex2, b: Vertex2, low: f32, high: f32, color: Vec4) {
    let a0 = world(a, low);
    let b0 = world(b, low);
    let b1 = world(b, high);
    let a1 = world(a, high);
    push_triangle(out, a0, b0, b1, color);
    push_triangle(out, a0, b1, a1, color);
}

fn convex_hull(mut points: Vec<Vertex2>) -> Vec<Vertex2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup_by(|a, b| a.x == b.x && a.y == b.y);
    if points.len() < 3 {
        return points;
    }
    let cross =
        |a: Vertex2, b: Vertex2, c: Vertex2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let mut hull = Vec::new();
    for &point in &points {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0.0 {
            hull.pop();
        }
        hull.push(point);
    }
    let lower_len = hull.len();
    for &point in points[..points.len() - 1].iter().rev() {
        while hull.len() > lower_len
            && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0.0
        {
            hull.pop();
        }
        hull.push(point);
    }
    hull.pop();
    hull
}

fn geometry(map: &Map) -> Result<Vec<Vertex>, io::Error> {
    let mut out = Vec::new();
    for leaf in &map.subsectors {
        let segs = &map.segs[leaf[1] as usize..leaf[1] as usize + leaf[0] as usize];
        let Some(first) = segs.first() else { continue };
        let sector_index = map.sides[map.lines[first[2] as usize][2 + first[3] as usize] as usize];
        let sector = map.sectors[sector_index as usize];
        let polygon = convex_hull(
            segs.iter()
                .flat_map(|seg| [map.vertices[seg[0] as usize], map.vertices[seg[1] as usize]])
                .collect(),
        );
        if polygon.len() < 3 {
            continue;
        }
        let root = world(polygon[0], sector.floor);
        let floor = shaded([0.27, 0.22, 0.14], sector);
        let ceiling = shaded([0.16, 0.19, 0.22], sector);
        for i in 1..polygon.len() - 1 {
            push_triangle(
                &mut out,
                root,
                world(polygon[i], sector.floor),
                world(polygon[i + 1], sector.floor),
                floor,
            );
            push_triangle(
                &mut out,
                world(polygon[i + 1], sector.ceiling),
                world(polygon[i], sector.ceiling),
                world(polygon[0], sector.ceiling),
                ceiling,
            );
        }
    }

    for seg in &map.segs {
        let line = map.lines[seg[2] as usize];
        let front_side = line[2 + seg[3] as usize];
        if front_side == u16::MAX {
            return Err(invalid("SEGS selected a missing sidedef"));
        }
        let front = map.sectors[map.sides[front_side as usize] as usize];
        let a = map.vertices[seg[0] as usize];
        let b = map.vertices[seg[1] as usize];
        let back_side = line[2 + (1 - seg[3]) as usize];
        if back_side == u16::MAX {
            push_quad(
                &mut out,
                a,
                b,
                front.floor,
                front.ceiling,
                shaded([0.61, 0.20, 0.12], front),
            );
            continue;
        }
        let back = map.sectors[map.sides[back_side as usize] as usize];
        if back.ceiling <= front.floor || back.floor >= front.ceiling {
            push_quad(
                &mut out,
                a,
                b,
                front.floor,
                front.ceiling,
                shaded([0.53, 0.22, 0.14], front),
            );
            continue;
        }
        if front.ceiling > back.ceiling {
            push_quad(
                &mut out,
                a,
                b,
                back.ceiling,
                front.ceiling,
                shaded([0.69, 0.34, 0.16], front),
            );
        }
        if back.floor > front.floor {
            push_quad(
                &mut out,
                a,
                b,
                front.floor,
                back.floor,
                shaded([0.36, 0.16, 0.11], front),
            );
        }
        // Masked middle textures and their transparency are not implemented in this pass.
    }
    Ok(out)
}

fn player_sector(map: &Map, x: i16, y: i16) -> Result<Sector, io::Error> {
    let point = Vertex2 {
        x: x as f32,
        y: y as f32,
    };
    let mut nearest: Option<(f32, Sector)> = None;
    for leaf in &map.subsectors {
        let segs = &map.segs[leaf[1] as usize..leaf[1] as usize + leaf[0] as usize];
        if segs.is_empty() {
            continue;
        }
        let polygon = convex_hull(
            segs.iter()
                .flat_map(|seg| [map.vertices[seg[0] as usize], map.vertices[seg[1] as usize]])
                .collect(),
        );
        if polygon.is_empty() {
            continue;
        }
        let center = polygon
            .iter()
            .fold(Vertex2 { x: 0.0, y: 0.0 }, |sum, point| Vertex2 {
                x: sum.x + point.x,
                y: sum.y + point.y,
            });
        let center = Vertex2 {
            x: center.x / polygon.len() as f32,
            y: center.y / polygon.len() as f32,
        };
        let distance = (center.x - point.x).powi(2) + (center.y - point.y).powi(2);
        let seg = segs[0];
        let line = map.lines[seg[2] as usize];
        let side = line[2 + seg[3] as usize];
        let sector = map.sectors[map.sides[side as usize] as usize];
        // ponytail: nearest leaf centroid can select a neighbor at borders; use NODES traversal if needed.
        if nearest.is_none_or(|(best, _)| distance < best) {
            nearest = Some((distance, sector));
        }
    }
    nearest
        .map(|(_, sector)| sector)
        .ok_or_else(|| invalid("E1M1 has no nonempty BSP leaves"))
}

fn render(path: &Path, output: &Path) -> api::Result<()> {
    if fs::metadata(path)?.len() > MAX_WAD_BYTES {
        return Err(invalid("WAD exceeds the 128 MiB sample limit").into());
    }
    let data = fs::read(path)?;
    let map = parse_map(&data)?;
    let (x, y, angle, _) = map
        .things
        .iter()
        .copied()
        .find(|thing| thing.3 == 1)
        .ok_or_else(|| invalid("E1M1 has no player-1 start"))?;
    let vertices = geometry(&map)?;
    if vertices.is_empty() {
        return Err(invalid("E1M1 produced no renderable geometry").into());
    }
    let sector = player_sector(&map, x, y)?;
    let radians = (angle as f32).to_radians();
    let eye = Vec3::new(x as f32, sector.floor + 41.0, -(y as f32));
    let forward = Vec3::new(radians.cos(), 0.0, -radians.sin());
    let view = Mat4::look_at(eye, eye + forward, Vec3::new(0.0, 1.0, 0.0));
    let projection = Mat4::perspective(1.22, 4.0 / 3.0, 1.0, 8192.0);
    let transform = projection * view;

    let device = Device::new();
    let vertex_shader =
        device.create_shader(include_bytes!("../assets/shaders/freedoom_map.vert.spv"))?;
    let fragment_shader =
        device.create_shader(include_bytes!("../assets/shaders/freedoom_map.frag.spv"))?;
    let pipeline = device.create_pipeline(&vertex_shader, &fragment_shader, Pipeline::default())?;
    let vertex_count = u32::try_from(vertices.len())
        .map_err(|_| invalid("E1M1 vertex count exceeds SILICON draw range"))?;
    let triangle_count = vertices.len() / 3;
    let vertex_buffer = device.create_vertex_buffer(vertices)?;
    let uniforms = transform.0.into_iter().map(Vec4::from_array).collect();
    let uniform_buffer = device.create_uniform_buffer(uniforms)?;
    let mut commands = device.commands();
    commands.begin_render_pass(Color::new(0.018, 0.024, 0.032, 1.0));
    commands.bind_pipeline(pipeline);
    commands.bind_vertex_buffer(vertex_buffer);
    commands.bind_uniform_buffer(uniform_buffer);
    commands.draw(0, vertex_count);
    commands.end_render_pass();
    let mut renderer = Renderer::new(960, 720)?;
    let submission = device.submit(&commands, &mut renderer)?;
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    renderer.framebuffer.save_png(output)?;
    println!(
        "E1M1: {triangle_count} triangles, {} SILICON draw(s), player start ({x}, {y}, {angle}°)",
        submission.draws
    );
    Ok(())
}

fn main() -> api::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let wad = args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: cargo run --release --example freedoom_map -- <freedoom1.wad> [output.png]",
        )
    })?;
    let output = args
        .next()
        .unwrap_or_else(|| "output/freedoom_map.png".into());
    if args.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected at most a WAD path and an output path",
        )
        .into());
    }
    render(Path::new(&wad), Path::new(&output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_truncated_and_out_of_range_wads() {
        assert!(parse_map(b"IWAD").is_err());
        let mut wad = b"IWAD\x01\x00\x00\x00\x0c\x00\x00\x00".to_vec();
        wad.extend_from_slice(&100u32.to_le_bytes());
        wad.extend_from_slice(&4u32.to_le_bytes());
        wad.extend_from_slice(b"E1M1\0\0\0");
        assert!(parse_map(&wad).is_err());
    }

    #[test]
    fn convex_hull_discards_interior_bsp_vertices() {
        let hull = convex_hull(vec![
            Vertex2 { x: 0.0, y: 0.0 },
            Vertex2 { x: 2.0, y: 0.0 },
            Vertex2 { x: 2.0, y: 2.0 },
            Vertex2 { x: 0.0, y: 2.0 },
            Vertex2 { x: 1.0, y: 1.0 },
        ]);
        assert_eq!(hull.len(), 4);
    }
}
