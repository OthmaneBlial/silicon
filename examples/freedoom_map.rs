//! Render Freedoom's E1M1 geometry through SILICON's programmable pipeline.
use minifb::{Key, KeyRepeat, Window, WindowOptions};
use silicon::api::{
    self, Address, Buffer, Color, Device, Filter, MipFilter, Pipeline, Renderer, Sampler,
    ShaderPipeline, Texture, TextureFormat, Vec2, Vec3, Vec4, Vertex,
};
use silicon_math::Mat4;
use std::{
    collections::{BTreeMap, btree_map::Entry},
    fs, io,
    path::Path,
    sync::Arc,
};

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
    floor_flat: [u8; 8],
    ceiling_flat: [u8; 8],
}

#[derive(Clone, Copy)]
struct SideDef {
    x_offset: i16,
    y_offset: i16,
    upper: [u8; 8],
    lower: [u8; 8],
    middle: [u8; 8],
    sector: u16,
}

struct Map {
    vertices: Vec<Vertex2>,
    sectors: Vec<Sector>,
    sides: Vec<SideDef>,
    lines: Vec<[u16; 5]>, // endpoints, flags, side 0, side 1
    segs: Vec<[u16; 5]>,  // endpoints, linedef, side, texture offset
    subsectors: Vec<[u16; 2]>,
    nodes: Vec<Node>,
    things: Vec<(i16, i16, u16, u16, u16)>, // x, y, angle, type, flags
}

#[derive(Clone, Copy)]
struct Node {
    x: i16,
    y: i16,
    dx: i16,
    dy: i16,
    children: [u16; 2],
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

fn u32_at(data: &[u8], offset: usize) -> Result<u32, io::Error> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| invalid("truncated 32-bit value"))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
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
                floor_flat: r[4..12].try_into().unwrap(),
                ceiling_flat: r[12..20].try_into().unwrap(),
            })
        })
        .collect::<Result<_, io::Error>>()?;
    let sides: Vec<SideDef> = records(3, 30)?
        .into_iter()
        .map(|r| {
            Ok(SideDef {
                x_offset: i16_at(r, 0)?,
                y_offset: i16_at(r, 2)?,
                upper: r[4..12].try_into().unwrap(),
                lower: r[12..20].try_into().unwrap(),
                middle: r[20..28].try_into().unwrap(),
                sector: u16_at(r, 28)?,
            })
        })
        .collect::<Result<_, io::Error>>()?;
    let lines: Vec<[u16; 5]> = records(2, 14)?
        .into_iter()
        .map(|r| {
            Ok([
                u16_at(r, 0)?,
                u16_at(r, 2)?,
                u16_at(r, 4)?,
                u16_at(r, 10)?,
                u16_at(r, 12)?,
            ])
        })
        .collect::<Result<_, io::Error>>()?;
    let segs: Vec<[u16; 5]> = records(5, 12)?
        .into_iter()
        .map(|r| {
            Ok([
                u16_at(r, 0)?,
                u16_at(r, 2)?,
                u16_at(r, 6)?,
                u16_at(r, 8)?,
                u16_at(r, 10)?,
            ])
        })
        .collect::<Result<_, io::Error>>()?;
    let subsectors: Vec<[u16; 2]> = records(6, 4)?
        .into_iter()
        .map(|r| Ok([u16_at(r, 0)?, u16_at(r, 2)?]))
        .collect::<Result<_, io::Error>>()?;
    let nodes: Vec<Node> = records(7, 28)?
        .into_iter()
        .map(|r| {
            Ok(Node {
                x: i16_at(r, 0)?,
                y: i16_at(r, 2)?,
                dx: i16_at(r, 4)?,
                dy: i16_at(r, 6)?,
                children: [u16_at(r, 24)?, u16_at(r, 26)?],
            })
        })
        .collect::<Result<_, io::Error>>()?;
    let things: Vec<(i16, i16, u16, u16, u16)> = records(1, 10)?
        .into_iter()
        .map(|r| {
            Ok((
                i16_at(r, 0)?,
                i16_at(r, 2)?,
                u16_at(r, 4)?,
                u16_at(r, 6)?,
                u16_at(r, 8)?,
            ))
        })
        .collect::<Result<_, io::Error>>()?;

    for line in &lines {
        if line[0] as usize >= vertices.len()
            || line[1] as usize >= vertices.len()
            || (line[3] != u16::MAX && line[3] as usize >= sides.len())
            || (line[4] != u16::MAX && line[4] as usize >= sides.len())
        {
            return Err(invalid("LINEDEFS references an invalid vertex or side"));
        }
    }
    for side in &sides {
        if side.sector as usize >= sectors.len() {
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
        let side = lines[seg[2] as usize][3 + seg[3] as usize];
        if side == u16::MAX || side as usize >= sides.len() {
            return Err(invalid("SEGS references a missing sidedef"));
        }
    }
    for leaf in &subsectors {
        if leaf[1] as usize + leaf[0] as usize > segs.len() {
            return Err(invalid("SSECTORS references invalid SEGS"));
        }
    }
    if subsectors.is_empty() || (nodes.is_empty() && subsectors.len() != 1) || nodes.len() > 0x8000
    {
        return Err(invalid("NODES and SSECTORS do not form a supported BSP"));
    }
    for node in &nodes {
        if node.dx == 0 && node.dy == 0 {
            return Err(invalid("NODES contains a zero-length partition"));
        }
        for &child in &node.children {
            let index = (child & 0x7fff) as usize;
            if (child & 0x8000 != 0 && index >= subsectors.len())
                || (child & 0x8000 == 0 && index >= nodes.len())
            {
                return Err(invalid("NODES references an invalid node or subsector"));
            }
        }
    }
    Ok(Map {
        vertices,
        sectors,
        sides,
        lines,
        segs,
        subsectors,
        nodes,
        things,
    })
}

fn lump_bytes<'a>(data: &'a [u8], lumps: &[Lump], name: [u8; 8]) -> Result<&'a [u8], io::Error> {
    let lump = lumps
        .iter()
        .rev()
        .find(|lump| lump.name == name)
        .ok_or_else(|| {
            invalid(format!(
                "WAD lump {} not found",
                String::from_utf8_lossy(&name)
            ))
        })?;
    Ok(&data[lump.offset..lump.offset + lump.size])
}

fn named_lump(data: &[u8], name: [u8; 8]) -> Result<&[u8], io::Error> {
    lump_bytes(data, &wad_lumps(data)?, name)
}

fn paletted_rgba(indices: &[u8], palette: &[u8]) -> Result<Vec<u8>, io::Error> {
    if palette.len() < 256 * 3 {
        return Err(invalid("PLAYPAL does not contain a complete palette"));
    }
    let mut rgba = Vec::with_capacity(indices.len() * 4);
    for &index in indices {
        let rgb = &palette[index as usize * 3..][..3];
        rgba.extend_from_slice(rgb);
        rgba.push(255);
    }
    Ok(rgba)
}

fn flat_textures(data: &[u8], map: &Map) -> api::Result<BTreeMap<[u8; 8], Arc<Texture>>> {
    let palette = named_lump(data, *b"PLAYPAL\0")?;
    let mut textures = BTreeMap::new();
    for name in map
        .sectors
        .iter()
        .flat_map(|sector| [sector.floor_flat, sector.ceiling_flat])
        .filter(|&name| name != *b"F_SKY1\0\0")
    {
        if textures.contains_key(&name) {
            continue;
        }
        let flat = named_lump(data, name)?;
        let pixels = flat.get(..64 * 64).ok_or_else(|| {
            invalid(format!(
                "flat {} is shorter than 64x64",
                String::from_utf8_lossy(&name)
            ))
        })?;
        let rgba = paletted_rgba(pixels, palette)?;
        textures.insert(
            name,
            Arc::new(Texture::new(64, 64, TextureFormat::Rgba8, &rgba)?),
        );
    }
    Ok(textures)
}

#[derive(Clone, Copy)]
struct PatchPlacement {
    x: i16,
    y: i16,
    index: u16,
}

struct TextureDef {
    width: u16,
    height: u16,
    patches: Vec<PatchPlacement>,
}

fn texture_definitions(
    bytes: &[u8],
    definitions: &mut BTreeMap<[u8; 8], TextureDef>,
) -> Result<(), io::Error> {
    let count = i32::from_le_bytes(
        bytes
            .get(..4)
            .ok_or_else(|| invalid("truncated TEXTURE directory"))?
            .try_into()
            .unwrap(),
    );
    if !(0..=65_536).contains(&count) {
        return Err(invalid("invalid TEXTURE definition count"));
    }
    let count = count as usize;
    let directory_size = 4usize
        .checked_add(
            count
                .checked_mul(4)
                .ok_or_else(|| invalid("TEXTURE directory overflow"))?,
        )
        .filter(|&size| size <= bytes.len())
        .ok_or_else(|| invalid("TEXTURE directory exceeds lump"))?;
    for index in 0..count {
        let offset = u32_at(bytes, 4 + index * 4)? as usize;
        if offset < directory_size {
            return Err(invalid("TEXTURE definition overlaps its directory"));
        }
        let name: [u8; 8] = bytes
            .get(offset..offset + 8)
            .ok_or_else(|| invalid("truncated TEXTURE definition name"))?
            .try_into()
            .unwrap();
        let width = u16_at(bytes, offset + 12)?;
        let height = u16_at(bytes, offset + 14)?;
        let patch_count = u16_at(bytes, offset + 20)? as usize;
        let end = offset
            .checked_add(22)
            .and_then(|start| start.checked_add(patch_count.checked_mul(10)?))
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| invalid("TEXTURE patches exceed lump"))?;
        if width == 0
            || height == 0
            || width > 4096
            || height > 4096
            || width as usize * height as usize > 16_777_216
        {
            return Err(invalid("TEXTURE dimensions exceed sample limits"));
        }
        let mut patches = Vec::with_capacity(patch_count);
        for patch in (offset + 22..end).step_by(10) {
            patches.push(PatchPlacement {
                x: i16_at(bytes, patch)?,
                y: i16_at(bytes, patch + 2)?,
                index: u16_at(bytes, patch + 4)?,
            });
        }
        definitions.insert(
            name,
            TextureDef {
                width,
                height,
                patches,
            },
        );
    }
    Ok(())
}

fn composite_patch(
    canvas: &mut [u8],
    canvas_width: u16,
    canvas_height: u16,
    patch: &[u8],
    origin_x: i16,
    origin_y: i16,
    palette: &[u8],
) -> Result<(), io::Error> {
    if patch.len() < 8 {
        return Err(invalid("truncated Doom patch header"));
    }
    let width = u16_at(patch, 0)? as usize;
    let height = u16_at(patch, 2)? as usize;
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err(invalid("Doom patch dimensions exceed sample limits"));
    }
    let columns_end = 8usize
        .checked_add(
            width
                .checked_mul(4)
                .ok_or_else(|| invalid("patch column table overflow"))?,
        )
        .filter(|&end| end <= patch.len())
        .ok_or_else(|| invalid("patch column table exceeds lump"))?;
    let canvas_width = canvas_width as i32;
    let canvas_height = canvas_height as i32;
    for column in 0..width {
        let mut cursor = u32_at(patch, 8 + column * 4)? as usize;
        if cursor < columns_end || cursor >= patch.len() {
            return Err(invalid("patch column offset exceeds lump"));
        }
        let mut previous_top: Option<i32> = None;
        let mut terminated = false;
        while cursor < patch.len() {
            let top_delta = patch[cursor];
            if top_delta == 255 {
                terminated = true;
                break;
            }
            let length = *patch
                .get(cursor + 1)
                .ok_or_else(|| invalid("truncated patch post"))? as usize;
            let post_end = cursor
                .checked_add(length + 4)
                .filter(|&end| end <= patch.len())
                .ok_or_else(|| invalid("patch post exceeds lump"))?;
            let top_delta = top_delta as i32;
            let top = previous_top
                .filter(|&previous| top_delta <= previous)
                .map_or(top_delta, |previous| previous + top_delta);
            previous_top = Some(top);
            for row in 0..length {
                let dst_x = origin_x as i32 + column as i32;
                let dst_y = origin_y as i32 + top + row as i32;
                if (0..canvas_width).contains(&dst_x) && (0..canvas_height).contains(&dst_y) {
                    let palette_index = patch[cursor + 3 + row] as usize;
                    let source = &palette[palette_index * 3..][..3];
                    let offset = (dst_y as usize * canvas_width as usize + dst_x as usize) * 4;
                    canvas[offset..offset + 3].copy_from_slice(source);
                    canvas[offset + 3] = 255;
                }
            }
            cursor = post_end;
        }
        if !terminated {
            return Err(invalid("unterminated Doom patch column"));
        }
    }
    Ok(())
}

fn wall_textures(data: &[u8], map: &Map) -> api::Result<BTreeMap<[u8; 8], Arc<Texture>>> {
    let lumps = wad_lumps(data)?;
    let palette = lump_bytes(data, &lumps, *b"PLAYPAL\0")?;
    if palette.len() < 256 * 3 {
        return Err(invalid("PLAYPAL does not contain a complete palette").into());
    }
    let pnames = lump_bytes(data, &lumps, *b"PNAMES\0\0")?;
    let patch_count = i32::from_le_bytes(
        pnames
            .get(..4)
            .ok_or_else(|| invalid("truncated PNAMES"))?
            .try_into()
            .unwrap(),
    );
    if !(0..=65_536).contains(&patch_count) {
        return Err(invalid("invalid PNAMES entry count").into());
    }
    let patch_count = patch_count as usize;
    let pnames_end = 4usize
        .checked_add(
            patch_count
                .checked_mul(8)
                .ok_or_else(|| invalid("PNAMES overflow"))?,
        )
        .filter(|&end| end <= pnames.len())
        .ok_or_else(|| invalid("PNAMES table exceeds lump"))?;
    let patch_names = pnames[4..pnames_end]
        .chunks_exact(8)
        .map(|name| <[u8; 8]>::try_from(name).unwrap())
        .collect::<Vec<_>>();
    let mut definitions = BTreeMap::new();
    texture_definitions(lump_bytes(data, &lumps, *b"TEXTURE1")?, &mut definitions)?;
    if let Ok(texture2) = lump_bytes(data, &lumps, *b"TEXTURE2") {
        texture_definitions(texture2, &mut definitions)?;
    }

    let names = map
        .sides
        .iter()
        .flat_map(|side| [side.upper, side.lower, side.middle])
        .filter(|name| name[0] != b'-' && name.iter().any(|&byte| byte != 0))
        .collect::<std::collections::BTreeSet<_>>();
    let mut textures = BTreeMap::new();
    let mut total_pixels = 0usize;
    let first_color = &palette[..3];
    for name in names {
        let Some(definition) = definitions.get(&name) else {
            return Err(invalid(format!(
                "wall texture {} has no definition",
                String::from_utf8_lossy(&name)
            ))
            .into());
        };
        total_pixels = total_pixels
            .checked_add(definition.width as usize * definition.height as usize)
            .filter(|&total| total <= 16_777_216)
            .ok_or_else(|| invalid("decoded wall textures exceed the 16M-pixel sample limit"))?;
        let mut rgba =
            Vec::with_capacity(definition.width as usize * definition.height as usize * 4);
        for _ in 0..definition.width as usize * definition.height as usize {
            rgba.extend_from_slice(first_color);
            rgba.push(255);
        }
        for placement in &definition.patches {
            let patch_name = patch_names
                .get(placement.index as usize)
                .ok_or_else(|| invalid("TEXTURE references an invalid PNAMES index"))?;
            let patch = lump_bytes(data, &lumps, *patch_name)?;
            composite_patch(
                &mut rgba,
                definition.width,
                definition.height,
                patch,
                placement.x,
                placement.y,
                palette,
            )?;
        }
        textures.insert(
            name,
            Arc::new(Texture::new(
                definition.width as u32,
                definition.height as u32,
                TextureFormat::Rgba8,
                &rgba,
            )?),
        );
    }
    Ok(textures)
}

fn monster_sprite(kind: u16) -> Option<([u8; 4], i32)> {
    match kind {
        3001 => Some((*b"TROO", 60)),
        3002 => Some((*b"SARG", 150)),
        3004 => Some((*b"POSS", 20)),
        9 => Some((*b"SPOS", 30)),
        _ => None,
    }
}

fn sprite_texture(data: &[u8], prefix: [u8; 4]) -> api::Result<SpriteTexture> {
    let lumps = wad_lumps(data)?;
    let mut name = [0; 8];
    name[..4].copy_from_slice(&prefix);
    name[4..6].copy_from_slice(b"A1");
    let patch = lump_bytes(data, &lumps, name)?;
    let width = u16_at(patch, 0)?;
    let height = u16_at(patch, 2)?;
    let left_offset = i16_at(patch, 4)?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || width as usize * height as usize > 16_777_216
    {
        return Err(invalid("sprite dimensions exceed sample limits").into());
    }
    let palette = lump_bytes(data, &lumps, *b"PLAYPAL\0")?;
    if palette.len() < 256 * 3 {
        return Err(invalid("PLAYPAL does not contain a complete palette").into());
    }
    let mut rgba = vec![0; width as usize * height as usize * 4];
    composite_patch(&mut rgba, width, height, patch, 0, 0, palette)?;
    Ok(SpriteTexture {
        texture: Arc::new(Texture::new(
            width as u32,
            height as u32,
            TextureFormat::Rgba8,
            &rgba,
        )?),
        width: width as f32,
        height: height as f32,
        left_offset: left_offset as f32,
    })
}

fn world(point: Vertex2, height: f32) -> Vec3 {
    Vec3::new(point.x, height, -point.y)
}

fn shaded(rgb: [f32; 3], sector: Sector) -> Vec4 {
    let light = 0.38 + sector.light as f32 / 255.0 * 0.62;
    Vec4::new(rgb[0] * light, rgb[1] * light, rgb[2] * light, 1.0)
}

fn push_triangle_uv(out: &mut Vec<Vertex>, points: [(Vec3, Vec2); 3], color: Vec4) {
    if (points[1].0 - points[0].0)
        .cross(points[2].0 - points[0].0)
        .length()
        > 0.01
    {
        for (position, uv) in points {
            out.push(Vertex {
                position,
                normal: Vec3::new(0.0, 1.0, 0.0),
                uv,
                color,
            });
        }
    }
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

#[derive(Default)]
struct Geometry {
    flats: BTreeMap<[u8; 8], Vec<Vertex>>,
    walls: BTreeMap<[u8; 8], Vec<Vertex>>,
}

#[derive(Clone, Copy)]
struct Player {
    x: f32,
    y: f32,
    angle: f32,
}

#[derive(Clone, Copy)]
struct Controls {
    forward: f32,
    strafe: f32,
    turn: f32,
    speed: f32,
}

struct Draw {
    texture: Arc<Texture>,
    vertices: Buffer<Vertex>,
    count: u32,
}

struct PreparedScene {
    map: Map,
    device: Device,
    pipeline: Arc<ShaderPipeline>,
    sprite_pipeline: Arc<ShaderPipeline>,
    sampler: Sampler,
    draws: Vec<Draw>,
    sprites: BTreeMap<[u8; 4], SpriteTexture>,
    actors: Vec<Actor>,
    start: Player,
    triangles: usize,
}

struct SpriteTexture {
    texture: Arc<Texture>,
    width: f32,
    height: f32,
    left_offset: f32,
}

#[derive(Clone, Copy)]
struct Actor {
    sprite: [u8; 4],
    x: f32,
    y: f32,
    health: i32,
    attack_cooldown: f32,
}

struct WallSection {
    name: [u8; 8],
    side: SideDef,
    seg: [u16; 5],
    endpoints: [Vertex2; 2],
    heights: [f32; 3], // low, high, texture top anchor
    sector: Sector,
}

fn push_wall_quad(out: &mut BTreeMap<[u8; 8], Vec<Vertex>>, texture: &Texture, wall: WallSection) {
    let WallSection {
        name,
        side,
        seg,
        endpoints: [a, b],
        heights: [low, high, top_anchor],
        sector,
    } = wall;
    if name[0] == b'-' || high <= low {
        return;
    }
    let width = texture.levels[0].width as f32;
    let height = texture.levels[0].height as f32;
    let u0 = (seg[4] as f32 + side.x_offset as f32) / width;
    let u1 = u0 + ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt() / width;
    let v_low = (top_anchor - low + side.y_offset as f32) / height;
    let v_high = (top_anchor - high + side.y_offset as f32) / height;
    let a0 = world(a, low);
    let b0 = world(b, low);
    let b1 = world(b, high);
    let a1 = world(a, high);
    let color = shaded([1.0; 3], sector);
    let mesh = out.entry(name).or_default();
    push_triangle_uv(
        mesh,
        [
            (a0, Vec2::new(u0, v_low)),
            (b0, Vec2::new(u1, v_low)),
            (b1, Vec2::new(u1, v_high)),
        ],
        color,
    );
    push_triangle_uv(
        mesh,
        [
            (a0, Vec2::new(u0, v_low)),
            (b1, Vec2::new(u1, v_high)),
            (a1, Vec2::new(u0, v_high)),
        ],
        color,
    );
}

fn actor_vertices(
    actor: Actor,
    sprite: &SpriteTexture,
    camera_angle: f32,
    sector: Sector,
) -> Vec<Vertex> {
    let radians = camera_angle.to_radians();
    let right = Vertex2 {
        x: radians.sin(),
        y: -radians.cos(),
    };
    let left = Vertex2 {
        x: actor.x - right.x * sprite.left_offset,
        y: actor.y - right.y * sprite.left_offset,
    };
    let right = Vertex2 {
        x: left.x + right.x * sprite.width,
        y: left.y + right.y * sprite.width,
    };
    let bottom_left = world(left, sector.floor);
    let bottom_right = world(right, sector.floor);
    let top_left = world(left, sector.floor + sprite.height);
    let top_right = world(right, sector.floor + sprite.height);
    let color = shaded([1.0; 3], sector);
    let mut vertices = Vec::with_capacity(6);
    push_triangle_uv(
        &mut vertices,
        [
            (bottom_left, Vec2::new(0.0, 1.0)),
            (bottom_right, Vec2::new(1.0, 1.0)),
            (top_right, Vec2::new(1.0, 0.0)),
        ],
        color,
    );
    push_triangle_uv(
        &mut vertices,
        [
            (bottom_left, Vec2::new(0.0, 1.0)),
            (top_right, Vec2::new(1.0, 0.0)),
            (top_left, Vec2::new(0.0, 0.0)),
        ],
        color,
    );
    vertices
}

fn geometry(map: &Map, textures: &BTreeMap<[u8; 8], Arc<Texture>>) -> Result<Geometry, io::Error> {
    let mut out = Geometry::default();
    for leaf in &map.subsectors {
        let segs = &map.segs[leaf[1] as usize..leaf[1] as usize + leaf[0] as usize];
        let Some(first) = segs.first() else { continue };
        let front_side = map.lines[first[2] as usize][3 + first[3] as usize];
        let sector_index = map.sides[front_side as usize].sector;
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
        let light = 0.38 + sector.light as f32 / 255.0 * 0.62;
        let floor = Vec4::new(light, light, light, 1.0);
        let ceiling = floor;
        for i in 1..polygon.len() - 1 {
            if sector.floor_flat != *b"F_SKY1\0\0" {
                let mesh = out.flats.entry(sector.floor_flat).or_default();
                push_triangle_uv(
                    mesh,
                    [
                        (root, flat_uv(polygon[0])),
                        (world(polygon[i], sector.floor), flat_uv(polygon[i])),
                        (world(polygon[i + 1], sector.floor), flat_uv(polygon[i + 1])),
                    ],
                    floor,
                );
            }
            if sector.ceiling_flat != *b"F_SKY1\0\0" {
                let mesh = out.flats.entry(sector.ceiling_flat).or_default();
                push_triangle_uv(
                    mesh,
                    [
                        (
                            world(polygon[i + 1], sector.ceiling),
                            flat_uv(polygon[i + 1]),
                        ),
                        (world(polygon[i], sector.ceiling), flat_uv(polygon[i])),
                        (world(polygon[0], sector.ceiling), flat_uv(polygon[0])),
                    ],
                    ceiling,
                );
            }
        }
    }

    for seg in &map.segs {
        let line = map.lines[seg[2] as usize];
        let front_side = line[3 + seg[3] as usize];
        if front_side == u16::MAX {
            return Err(invalid("SEGS selected a missing sidedef"));
        }
        let front_sidedef = map.sides[front_side as usize];
        let front = map.sectors[front_sidedef.sector as usize];
        let a = map.vertices[seg[0] as usize];
        let b = map.vertices[seg[1] as usize];
        let back_side = line[3 + (1 - seg[3]) as usize];
        if back_side == u16::MAX {
            if let Some(texture) = textures.get(&front_sidedef.middle) {
                let bottom_peg = line[2] & 16 != 0;
                let anchor = if bottom_peg {
                    front.floor + texture.levels[0].height as f32
                } else {
                    front.ceiling
                };
                push_wall_quad(
                    &mut out.walls,
                    texture,
                    WallSection {
                        name: front_sidedef.middle,
                        side: front_sidedef,
                        seg: *seg,
                        endpoints: [a, b],
                        heights: [front.floor, front.ceiling, anchor],
                        sector: front,
                    },
                );
            }
            continue;
        }
        let back_sidedef = map.sides[back_side as usize];
        let back = map.sectors[back_sidedef.sector as usize];
        if back.ceiling <= front.floor || back.floor >= front.ceiling {
            if let Some(texture) = textures.get(&front_sidedef.middle) {
                let anchor = if line[2] & 16 != 0 {
                    front.floor + texture.levels[0].height as f32
                } else {
                    front.ceiling
                };
                push_wall_quad(
                    &mut out.walls,
                    texture,
                    WallSection {
                        name: front_sidedef.middle,
                        side: front_sidedef,
                        seg: *seg,
                        endpoints: [a, b],
                        heights: [front.floor, front.ceiling, anchor],
                        sector: front,
                    },
                );
            }
            continue;
        }
        if front.ceiling > back.ceiling
            && let Some(texture) = textures.get(&front_sidedef.upper)
        {
            let anchor = if line[2] & 8 != 0 {
                front.ceiling
            } else {
                back.ceiling + texture.levels[0].height as f32
            };
            push_wall_quad(
                &mut out.walls,
                texture,
                WallSection {
                    name: front_sidedef.upper,
                    side: front_sidedef,
                    seg: *seg,
                    endpoints: [a, b],
                    heights: [back.ceiling.max(front.floor), front.ceiling, anchor],
                    sector: front,
                },
            );
        }
        if back.floor > front.floor
            && let Some(texture) = textures.get(&front_sidedef.lower)
        {
            let anchor = if line[2] & 16 != 0 {
                front.ceiling
            } else {
                back.floor
            };
            push_wall_quad(
                &mut out.walls,
                texture,
                WallSection {
                    name: front_sidedef.lower,
                    side: front_sidedef,
                    seg: *seg,
                    endpoints: [a, b],
                    heights: [front.floor, back.floor.min(front.ceiling), anchor],
                    sector: front,
                },
            );
        }
        // Masked middle textures and their transparency are not implemented in this pass.
    }
    Ok(out)
}

fn flat_uv(point: Vertex2) -> Vec2 {
    Vec2::new(point.x / 64.0, point.y / 64.0)
}

fn point_on_node_side(point: Vertex2, node: Node) -> usize {
    if node.dx == 0 {
        return usize::from((point.x <= node.x as f32) == (node.dy > 0));
    }
    if node.dy == 0 {
        return usize::from((point.y <= node.y as f32) == (node.dx < 0));
    }
    let dx = point.x as f64 - node.x as f64;
    let dy = point.y as f64 - node.y as f64;
    usize::from(node.dx as f64 * dy - node.dy as f64 * dx >= 0.0)
}

fn subsector_at(map: &Map, point: Vertex2) -> Option<usize> {
    if map.nodes.is_empty() {
        return (map.subsectors.len() == 1).then_some(0);
    }
    let mut child = u16::try_from(map.nodes.len().checked_sub(1)?).ok()?;
    for _ in 0..=map.nodes.len() {
        if child & 0x8000 != 0 {
            let index = (child & 0x7fff) as usize;
            return (index < map.subsectors.len()).then_some(index);
        }
        let node = *map.nodes.get(child as usize)?;
        child = node.children[point_on_node_side(point, node)];
    }
    None
}

fn bsp_sector_at(map: &Map, x: f32, y: f32) -> Option<Sector> {
    let leaf = map.subsectors.get(subsector_at(map, Vertex2 { x, y })?)?;
    if leaf[0] == 0 {
        return None;
    }
    let seg = map.segs.get(leaf[1] as usize)?;
    let line = map.lines.get(seg[2] as usize)?;
    let sidedef = map.sides.get(*line.get(3 + seg[3] as usize)? as usize)?;
    map.sectors.get(sidedef.sector as usize).copied()
}

fn distance_to_segment_squared(point: Vertex2, a: Vertex2, b: Vertex2) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length_squared = dx * dx + dy * dy;
    let t = if length_squared == 0.0 {
        0.0
    } else {
        (((point.x - a.x) * dx + (point.y - a.y) * dy) / length_squared).clamp(0.0, 1.0)
    };
    (point.x - (a.x + t * dx)).powi(2) + (point.y - (a.y + t * dy)).powi(2)
}

fn can_occupy(map: &Map, player: Player, from: Sector) -> bool {
    let Some(sector) = bsp_sector_at(map, player.x, player.y) else {
        return false;
    };
    if sector.floor > from.floor + 24.0 || sector.ceiling < sector.floor + 56.0 {
        return false;
    }
    let point = Vertex2 {
        x: player.x,
        y: player.y,
    };
    for line in &map.lines {
        if line[2] & 1 == 0 && line[4] != u16::MAX {
            continue;
        }
        let a = map.vertices[line[0] as usize];
        let b = map.vertices[line[1] as usize];
        if distance_to_segment_squared(point, a, b) < 16.0 * 16.0 {
            return false;
        }
    }
    true
}

fn move_player(map: &Map, player: &mut Player, controls: Controls, delta: f32) {
    player.angle = (player.angle + controls.turn * delta).rem_euclid(360.0);
    let angle = player.angle.to_radians();
    let dx =
        (controls.forward * angle.cos() - controls.strafe * angle.sin()) * controls.speed * delta;
    let dy =
        (controls.forward * angle.sin() + controls.strafe * angle.cos()) * controls.speed * delta;
    for (x_axis, amount) in [(true, dx), (false, dy)] {
        if amount == 0.0 {
            continue;
        }
        let Some(from) = bsp_sector_at(map, player.x, player.y) else {
            break;
        };
        let mut candidate = *player;
        if x_axis {
            candidate.x += amount;
        } else {
            candidate.y += amount;
        }
        if can_occupy(map, candidate, from) {
            *player = candidate;
        }
    }
}

fn cross2(a: Vertex2, b: Vertex2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn ray_segment_distance(
    origin: Vertex2,
    direction: Vertex2,
    a: Vertex2,
    b: Vertex2,
) -> Option<f32> {
    let segment = Vertex2 {
        x: b.x - a.x,
        y: b.y - a.y,
    };
    let relative = Vertex2 {
        x: a.x - origin.x,
        y: a.y - origin.y,
    };
    let denominator = cross2(direction, segment);
    if denominator.abs() < 0.0001 {
        return None;
    }
    let distance = cross2(relative, segment) / denominator;
    let along = cross2(relative, direction) / denominator;
    (distance >= 0.0 && (0.0..=1.0).contains(&along)).then_some(distance)
}

fn fire_weapon(map: &Map, actors: &mut [Actor], player: Player) -> bool {
    let origin = Vertex2 {
        x: player.x,
        y: player.y,
    };
    let radians = player.angle.to_radians();
    let direction = Vertex2 {
        x: radians.cos(),
        y: radians.sin(),
    };
    let nearest_wall = map
        .lines
        .iter()
        .filter(|line| line[2] & 1 != 0 || line[4] == u16::MAX)
        .filter_map(|line| {
            ray_segment_distance(
                origin,
                direction,
                map.vertices[line[0] as usize],
                map.vertices[line[1] as usize],
            )
        })
        .fold(f32::INFINITY, f32::min);
    let target = actors
        .iter()
        .enumerate()
        .filter(|(_, actor)| actor.health > 0)
        .filter_map(|(index, actor)| {
            let dx = actor.x - player.x;
            let dy = actor.y - player.y;
            let distance = (dx * dx + dy * dy).sqrt();
            let aim_error =
                (dy.atan2(dx).to_degrees() - player.angle + 180.0).rem_euclid(360.0) - 180.0;
            let aim_width = 1.0 + (18.0 / distance.max(1.0)).atan().to_degrees();
            let along_ray = dx * direction.x + dy * direction.y;
            (distance <= 1024.0
                && aim_error.abs() <= aim_width
                && along_ray <= nearest_wall
                && along_ray > 0.0)
                .then_some((index, along_ray))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index);
    if let Some(index) = target {
        actors[index].health -= 20;
        actors[index].health <= 0
    } else {
        false
    }
}

fn update_actors(map: &Map, actors: &mut [Actor], player: Player, health: &mut i32, delta: f32) {
    for actor in actors.iter_mut().filter(|actor| actor.health > 0) {
        actor.attack_cooldown = (actor.attack_cooldown - delta).max(0.0);
        let dx = player.x - actor.x;
        let dy = player.y - actor.y;
        let distance = (dx * dx + dy * dy).sqrt();
        if distance < 48.0 {
            if actor.attack_cooldown == 0.0 {
                *health -= 8;
                actor.attack_cooldown = 0.85;
            }
        } else if distance < 640.0 {
            let mut enemy = Player {
                x: actor.x,
                y: actor.y,
                angle: dy.atan2(dx).to_degrees(),
            };
            move_player(
                map,
                &mut enemy,
                Controls {
                    forward: 1.0,
                    strafe: 0.0,
                    turn: 0.0,
                    speed: 36.0,
                },
                delta,
            );
            actor.x = enemy.x;
            actor.y = enemy.y;
        }
    }
}

impl PreparedScene {
    fn load(path: &Path) -> api::Result<Self> {
        if fs::metadata(path)?.len() > MAX_WAD_BYTES {
            return Err(invalid("WAD exceeds the 128 MiB sample limit").into());
        }
        let data = fs::read(path)?;
        let map = parse_map(&data)?;
        let (x, y, angle, _, _) = map
            .things
            .iter()
            .copied()
            .find(|thing| thing.3 == 1)
            .ok_or_else(|| invalid("E1M1 has no player-1 start"))?;
        let wall_textures = wall_textures(&data, &map)?;
        let flat_textures = flat_textures(&data, &map)?;
        let geometry = geometry(&map, &wall_textures)?;
        if geometry.walls.is_empty() && geometry.flats.is_empty() {
            return Err(invalid("E1M1 produced no renderable geometry").into());
        }
        let triangles = (geometry.walls.values().map(Vec::len).sum::<usize>()
            + geometry.flats.values().map(Vec::len).sum::<usize>())
            / 3;
        let device = Device::new();
        let vertex_shader =
            device.create_shader(include_bytes!("../assets/shaders/textured.vert.spv"))?;
        let fragment_shader =
            device.create_shader(include_bytes!("../assets/shaders/textured.frag.spv"))?;
        let pipeline =
            device.create_pipeline(&vertex_shader, &fragment_shader, Pipeline::default())?;
        let sprite_fragment =
            device.create_shader(include_bytes!("../assets/shaders/freedoom_sprite.frag.spv"))?;
        let sprite_pipeline =
            device.create_pipeline(&vertex_shader, &sprite_fragment, Pipeline::default())?;
        let sampler = Sampler {
            filter: Filter::Nearest,
            address: Address::Repeat,
            mip: MipFilter::None,
        };
        let mut sprites = BTreeMap::new();
        let mut actors = Vec::new();
        for &(x, y, _, kind, flags) in &map.things {
            let Some((prefix, health)) = monster_sprite(kind) else {
                continue;
            };
            if flags & 2 == 0 || flags & 16 != 0 {
                continue;
            }
            if bsp_sector_at(&map, x as f32, y as f32).is_none() {
                continue;
            }
            if let Entry::Vacant(entry) = sprites.entry(prefix) {
                entry.insert(sprite_texture(&data, prefix)?);
            }
            actors.push(Actor {
                sprite: prefix,
                x: x as f32,
                y: y as f32,
                health,
                attack_cooldown: 0.0,
            });
        }
        let mut draws = Vec::new();
        for (name, vertices) in geometry.flats {
            let texture = flat_textures
                .get(&name)
                .ok_or_else(|| {
                    invalid(format!(
                        "flat {} was not decoded",
                        String::from_utf8_lossy(&name)
                    ))
                })?
                .clone();
            let count = u32::try_from(vertices.len())
                .map_err(|_| invalid("E1M1 flat vertex count exceeds SILICON draw range"))?;
            draws.push(Draw {
                texture,
                vertices: device.create_vertex_buffer(vertices)?,
                count,
            });
        }
        for (name, vertices) in geometry.walls {
            let texture = wall_textures
                .get(&name)
                .ok_or_else(|| {
                    invalid(format!(
                        "wall texture {} was not decoded",
                        String::from_utf8_lossy(&name)
                    ))
                })?
                .clone();
            let count = u32::try_from(vertices.len())
                .map_err(|_| invalid("E1M1 wall vertex count exceeds SILICON draw range"))?;
            draws.push(Draw {
                texture,
                vertices: device.create_vertex_buffer(vertices)?,
                count,
            });
        }
        Ok(Self {
            map,
            device,
            pipeline,
            sprite_pipeline,
            sampler,
            draws,
            sprites,
            actors,
            start: Player {
                x: x as f32,
                y: y as f32,
                angle: angle as f32,
            },
            triangles,
        })
    }

    fn draw(
        &self,
        player: Player,
        actors: &[Actor],
        renderer: &mut Renderer,
    ) -> api::Result<api::Submission> {
        let sector = bsp_sector_at(&self.map, player.x, player.y)
            .ok_or_else(|| invalid("player is outside every E1M1 BSP leaf"))?;
        let eye = Vec3::new(player.x, sector.floor + 41.0, -player.y);
        let radians = player.angle.to_radians();
        let forward = Vec3::new(radians.cos(), 0.0, -radians.sin());
        let view = Mat4::look_at(eye, eye + forward, Vec3::new(0.0, 1.0, 0.0));
        let projection = Mat4::perspective(1.22, 4.0 / 3.0, 1.0, 8192.0);
        let uniforms = (projection * view)
            .0
            .into_iter()
            .map(Vec4::from_array)
            .collect();
        let uniform_buffer = self.device.create_uniform_buffer(uniforms)?;
        let mut commands = self.device.commands();
        commands.begin_render_pass(Color::new(0.12, 0.22, 0.36, 1.0));
        commands.bind_pipeline(self.pipeline.clone());
        commands.bind_uniform_buffer(uniform_buffer.clone());
        for draw in &self.draws {
            commands.bind_texture(0, draw.texture.clone(), self.sampler);
            commands.bind_vertex_buffer(draw.vertices.clone());
            commands.draw(0, draw.count);
        }
        commands.bind_pipeline(self.sprite_pipeline.clone());
        commands.bind_uniform_buffer(uniform_buffer);
        for actor in actors.iter().filter(|actor| actor.health > 0) {
            let Some(sector) = bsp_sector_at(&self.map, actor.x, actor.y) else {
                continue;
            };
            let Some(sprite) = self.sprites.get(&actor.sprite) else {
                continue;
            };
            let vertices = actor_vertices(*actor, sprite, player.angle, sector);
            if vertices.is_empty() {
                continue;
            }
            let count = u32::try_from(vertices.len())
                .map_err(|_| invalid("E1M1 sprite vertex count exceeds SILICON draw range"))?;
            commands.bind_texture(0, sprite.texture.clone(), self.sampler);
            commands.bind_vertex_buffer(self.device.create_vertex_buffer(vertices)?);
            commands.draw(0, count);
        }
        commands.end_render_pass();
        self.device.submit(&commands, renderer)
    }
}

fn save_frame(renderer: &Renderer, output: &Path) -> api::Result<()> {
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    renderer.framebuffer.save_png(output)
}

fn frame_triangles(scene: &PreparedScene, draws: u64) -> api::Result<usize> {
    let draws =
        usize::try_from(draws).map_err(|_| invalid("SILICON draw count exceeds host range"))?;
    Ok(scene.triangles + draws.saturating_sub(scene.draws.len()) * 2)
}

fn render(path: &Path, output: &Path) -> api::Result<()> {
    let scene = PreparedScene::load(path)?;
    let mut renderer = Renderer::new(960, 720)?;
    let submission = scene.draw(scene.start, &scene.actors, &mut renderer)?;
    save_frame(&renderer, output)?;
    let triangles = frame_triangles(&scene, submission.draws)?;
    println!(
        "E1M1: {} triangles, {} SILICON draw(s), player start ({}, {}, {}°)",
        triangles, submission.draws, scene.start.x, scene.start.y, scene.start.angle
    );
    Ok(())
}

fn run_interactive(path: &Path, output: &Path) -> api::Result<()> {
    let scene = PreparedScene::load(path)?;
    let mut renderer = Renderer::new(960, 720)?;
    let mut window = Window::new(
        "SILICON | Freedoom E1M1",
        960,
        720,
        WindowOptions::default(),
    )?;
    window.set_target_fps(60);
    let mut pixels = vec![0; 960 * 720];
    let mut player = scene.start;
    let mut actors = scene.actors.clone();
    let mut health = 100;
    let mut ammo = 200;
    let mut kills = 0;
    let mut shot_cooldown = 0.0f32;
    let mut last = std::time::Instant::now();
    let mut frames = 0u64;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        let now = std::time::Instant::now();
        let delta = now.duration_since(last).as_secs_f32().min(0.05);
        last = now;
        if health > 0 {
            let axis = |positive, negative| {
                (window.is_key_down(positive) as i8 - window.is_key_down(negative) as i8) as f32
            };
            move_player(
                &scene.map,
                &mut player,
                Controls {
                    forward: axis(Key::W, Key::S),
                    strafe: axis(Key::D, Key::A),
                    turn: axis(Key::Right, Key::Left) * 100.0,
                    speed: if window.is_key_down(Key::LeftShift)
                        || window.is_key_down(Key::RightShift)
                    {
                        240.0
                    } else {
                        160.0
                    },
                },
                delta,
            );
            shot_cooldown = (shot_cooldown - delta).max(0.0);
            if window.is_key_pressed(Key::Space, KeyRepeat::No) && shot_cooldown == 0.0 && ammo > 0
            {
                ammo -= 1;
                shot_cooldown = 0.35;
                kills += usize::from(fire_weapon(&scene.map, &mut actors, player));
            }
            update_actors(&scene.map, &mut actors, player, &mut health, delta);
            health = health.max(0);
        }
        let submission = scene.draw(player, &actors, &mut renderer)?;
        renderer.framebuffer.present_into(&mut pixels)?;
        let remaining = actors.iter().filter(|actor| actor.health > 0).count();
        let state = if health == 0 {
            "DEAD"
        } else if remaining == 0 {
            "CLEAR"
        } else {
            "PLAYING"
        };
        let triangles = frame_triangles(&scene, submission.draws)?;
        window.set_title(&format!(
            "SILICON | E1M1 {state} | WASD move, arrows turn, Shift run, Space fire | HP {health} | ammo {ammo} | kills {kills}/{} | {} triangles, {} draws",
            scene.actors.len(),
            triangles,
            submission.draws
        ));
        window.update_with_buffer(&pixels, 960, 720)?;
        frames += 1;
    }
    save_frame(&renderer, output)?;
    println!(
        "E1M1 session: {frames} SILICON-rendered frames, {kills}/{} kills, health {health}; saved {}",
        scene.actors.len(),
        output.display()
    );
    Ok(())
}

fn main() -> api::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let wad = args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: cargo run --release --example freedoom_map -- <freedoom1.wad> [output.png | --interactive]",
        )
    })?;
    let output_or_mode = args.next();
    let interactive = output_or_mode.as_deref() == Some(std::ffi::OsStr::new("--interactive"));
    let output = if interactive {
        std::ffi::OsString::from("output/freedoom_map.png")
    } else {
        output_or_mode.unwrap_or_else(|| "output/freedoom_map.png".into())
    };
    if args.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a WAD path and either an output path or --interactive",
        )
        .into());
    }
    if interactive {
        run_interactive(Path::new(&wad), Path::new(&output))
    } else {
        render(Path::new(&wad), Path::new(&output))
    }
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

    #[test]
    fn flat_pixels_use_the_wad_palette() {
        let mut palette = vec![0; 256 * 3];
        palette[3..6].copy_from_slice(&[19, 87, 203]);
        assert_eq!(paletted_rgba(&[1], &palette).unwrap(), [19, 87, 203, 255]);
    }

    #[test]
    fn composes_a_classic_patch_column() {
        let mut palette = vec![0; 256 * 3];
        palette[3..6].copy_from_slice(&[19, 87, 203]);
        let patch = [
            1, 0, 1, 0, 0, 0, 0, 0, // 1x1 patch header
            12, 0, 0, 0, // first column offset
            0, 1, 0, 1, 0, 255, // one post and column terminator
        ];
        let mut canvas = vec![0; 8];
        composite_patch(&mut canvas, 2, 1, &patch, 1, 0, &palette).unwrap();
        assert_eq!(canvas, [0, 0, 0, 0, 19, 87, 203, 255]);
    }

    #[test]
    fn reads_patch_placements_from_a_texture_definition() {
        let mut bytes = vec![0; 40];
        bytes[0..4].copy_from_slice(&1i32.to_le_bytes());
        bytes[4..8].copy_from_slice(&8u32.to_le_bytes());
        bytes[8..16].copy_from_slice(b"TEST\0\0\0\0");
        bytes[20..22].copy_from_slice(&2u16.to_le_bytes());
        bytes[22..24].copy_from_slice(&3u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&1u16.to_le_bytes());
        bytes[30..32].copy_from_slice(&4i16.to_le_bytes());
        bytes[32..34].copy_from_slice(&(-2i16).to_le_bytes());
        bytes[34..36].copy_from_slice(&7u16.to_le_bytes());

        let mut definitions = BTreeMap::new();
        texture_definitions(&bytes, &mut definitions).unwrap();
        let texture = &definitions[b"TEST\0\0\0\0"];
        assert_eq!((texture.width, texture.height), (2, 3));
        assert_eq!(texture.patches.len(), 1);
        assert_eq!(texture.patches[0].x, 4);
        assert_eq!(texture.patches[0].y, -2);
        assert_eq!(texture.patches[0].index, 7);
    }

    #[test]
    fn bsp_selects_the_sector_and_rejects_solid_wall_overlap() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            light: 255,
            floor_flat: *b"FLOOR0_1",
            ceiling_flat: *b"CEIL1_1\0",
        };
        let raised = Sector {
            floor: 32.0,
            ..sector
        };
        let side = |sector| SideDef {
            x_offset: 0,
            y_offset: 0,
            upper: [0; 8],
            lower: [0; 8],
            middle: [0; 8],
            sector,
        };
        let map = Map {
            vertices: vec![
                Vertex2 { x: 96.0, y: 0.0 },
                Vertex2 { x: 96.0, y: 128.0 },
                Vertex2 { x: 160.0, y: 0.0 },
                Vertex2 { x: 160.0, y: 128.0 },
            ],
            sectors: vec![sector, raised],
            sides: vec![side(0), side(1)],
            lines: vec![[0, 1, 1, 0, u16::MAX], [2, 3, 1, 1, u16::MAX]],
            segs: vec![[0, 1, 0, 0, 0], [2, 3, 1, 0, 0]],
            subsectors: vec![[1, 0], [1, 1]],
            nodes: vec![Node {
                x: 128,
                y: 0,
                dx: 0,
                dy: 1,
                children: [0x8001, 0x8000],
            }],
            things: vec![],
        };
        assert_eq!(bsp_sector_at(&map, 64.0, 64.0).unwrap().floor, 0.0);
        assert_eq!(bsp_sector_at(&map, 200.0, 64.0).unwrap().floor, 32.0);
        let diagonal = Node {
            x: 0,
            y: 0,
            dx: 1,
            dy: 1,
            children: [0, 0],
        };
        assert_eq!(point_on_node_side(Vertex2 { x: 2.0, y: 0.0 }, diagonal), 0);
        assert_eq!(point_on_node_side(Vertex2 { x: 0.0, y: 2.0 }, diagonal), 1);
        assert!(!can_occupy(
            &map,
            Player {
                x: 104.0,
                y: 64.0,
                angle: 0.0,
            },
            sector,
        ));
        assert!(can_occupy(
            &map,
            Player {
                x: 64.0,
                y: 64.0,
                angle: 0.0,
            },
            sector,
        ));
        let mut player = Player {
            x: 80.0,
            y: 64.0,
            angle: 0.0,
        };
        move_player(
            &map,
            &mut player,
            Controls {
                forward: 1.0,
                strafe: 0.0,
                turn: 10.0,
                speed: 160.0,
            },
            0.05,
        );
        assert_eq!(player.x, 80.0);
        assert_eq!(player.angle, 0.5);

        let mut cyclic = map;
        cyclic.nodes[0].children = [0, 0];
        assert_eq!(subsector_at(&cyclic, Vertex2 { x: 0.0, y: 0.0 }), None);
    }

    #[test]
    fn hitscans_damage_an_exposed_enemy_but_stop_at_a_blocking_line() {
        let mut actors = [Actor {
            sprite: *b"TROO",
            x: 100.0,
            y: 0.0,
            health: 20,
            attack_cooldown: 0.0,
        }];
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        let map = Map {
            vertices: vec![],
            sectors: vec![],
            sides: vec![],
            lines: vec![],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        assert!(fire_weapon(&map, &mut actors, player));
        assert_eq!(actors[0].health, 0);

        actors[0].health = 20;
        let map = Map {
            vertices: vec![Vertex2 { x: 50.0, y: -32.0 }, Vertex2 { x: 50.0, y: 32.0 }],
            lines: vec![[0, 1, 1, u16::MAX, u16::MAX]],
            ..map
        };
        assert!(!fire_weapon(&map, &mut actors, player));
        assert_eq!(actors[0].health, 20);
    }

    #[test]
    fn enemies_chase_and_melee_on_a_cooldown() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
        };
        let map = Map {
            vertices: vec![
                Vertex2 {
                    x: -200.0,
                    y: -200.0,
                },
                Vertex2 {
                    x: -200.0,
                    y: 200.0,
                },
            ],
            sectors: vec![sector],
            sides: vec![SideDef {
                x_offset: 0,
                y_offset: 0,
                upper: [0; 8],
                lower: [0; 8],
                middle: [0; 8],
                sector: 0,
            }],
            lines: vec![[0, 1, 1, 0, u16::MAX]],
            segs: vec![[0, 1, 0, 0, 0]],
            subsectors: vec![[1, 0]],
            nodes: vec![],
            things: vec![],
        };
        let mut actors = [Actor {
            sprite: *b"TROO",
            x: 100.0,
            y: 0.0,
            health: 60,
            attack_cooldown: 0.0,
        }];
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        let mut health = 100;
        update_actors(&map, &mut actors, player, &mut health, 1.0);
        assert_eq!(actors[0].x, 64.0);
        assert_eq!(health, 100);

        actors[0].x = 40.0;
        update_actors(&map, &mut actors, player, &mut health, 0.05);
        update_actors(&map, &mut actors, player, &mut health, 0.84);
        assert_eq!(health, 92);
        update_actors(&map, &mut actors, player, &mut health, 0.02);
        assert_eq!(health, 84);
    }
}
