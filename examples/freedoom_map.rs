//! Render Freedoom maps through SILICON's programmable pipeline.
use minifb::{Key, KeyRepeat, Window, WindowOptions};
use silicon::api::{
    self, Address, Color, Device, Filter, MipFilter, Pipeline, Renderer, Sampler, ShaderPipeline,
    Texture, TextureFormat, Vec2, Vec3, Vec4, Vertex,
};
use silicon_math::Mat4;
use std::{
    collections::{BTreeMap, btree_map::Entry},
    fs, io,
    path::Path,
    sync::Arc,
};

const MAP_LUMPS_AFTER_MARKER: [&str; 10] = [
    "THINGS", "LINEDEFS", "SIDEDEFS", "VERTEXES", "SEGS", "SSECTORS", "NODES", "SECTORS", "REJECT",
    "BLOCKMAP",
];
const MAX_WAD_BYTES: u64 = 128 * 1024 * 1024;
const DEPTH_BUCKET_SIZE: f32 = 2048.0;
const ACTOR_HEIGHT: f32 = 56.0;
const ACTOR_STEP_HEIGHT: f32 = 24.0;
const ACTOR_RADIUS: f32 = 16.0;
const ACTOR_WAKE_RANGE: f32 = 640.0;
const ACTOR_TARGET_THRESHOLD: f32 = 100.0 / 35.0;
const ACTOR_IDLE_CYCLE: f32 = 20.0 / 35.0;
const LINE_TWO_SIDED: u16 = 4;
const LINE_SOUND_BLOCK: u16 = 64;
const LINE_DOOR_RAISE: u16 = 1;
const LINE_BLUE_LOCKED_DOOR: u16 = 26;
const LINE_BLAZING_DOOR_RAISE: u16 = 117;
const LINE_WALK_OPEN_DOOR: u16 = 2;
const LINE_USE_DOWN_WAIT_UP_PLATFORM: u16 = 62;
const LINE_USE_LOWER_FLOOR_TO_LOWEST: u16 = 23;
const LINE_PLAT_DOWN_WAIT_UP: u16 = 88;
const LINE_EXIT_USE: u16 = 11;
const USE_RANGE: f32 = 64.0;
const DOOR_SPEED: f32 = 70.0;
const BLAZING_DOOR_SPEED: f32 = DOOR_SPEED * 4.0;
const DOOR_WAIT: f32 = 150.0 / 35.0;
const PLATFORM_SPEED: f32 = 140.0;
const PLATFORM_WAIT: f32 = 3.0;
const FLOOR_SPEED: f32 = 35.0;
const DOOM_TICS_PER_SECOND: f32 = 35.0;
const SECTOR_SECRET: u16 = 9;
const SECTOR_NUKAGE_DAMAGE: u16 = 7;
const SECTOR_LIGHT_FLASH: u16 = 1;
const SECTOR_LIGHT_STROBE_SLOW: u16 = 12;
const STROBE_BRIGHT_TICS: f32 = 5.0;
const STROBE_SLOW_DARK_TICS: f32 = 35.0;
const NUKAGE_DAMAGE_TICS: f32 = 32.0;
const NUKAGE_DAMAGE: i32 = 5;
const DOOM_SKY_MID: f32 = 100.0;
const DOOM_SKY_BASE_WIDTH: f32 = 320.0;
const DOOM_SKY_ANGLE_COLUMNS: f32 = 1024.0;
const SKY_MESH_SEGMENTS: usize = 32;

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
    special: u16,
    light: u8,
    tag: u16,
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
    lines: Vec<[u16; 7]>, // endpoints, flags, side 0, side 1, special, tag
    segs: Vec<[u16; 5]>,  // endpoints, linedef, side, texture offset
    subsectors: Vec<[u16; 2]>,
    nodes: Vec<Node>,
    things: Vec<(i16, i16, u16, u16, u16)>, // x, y, angle, type, flags
}

#[derive(Clone, Copy, Default)]
struct Bounds2 {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
}

#[derive(Clone, Copy)]
struct Bounds3 {
    min: Vec3,
    max: Vec3,
}

#[derive(Clone, Copy)]
struct Node {
    x: i16,
    y: i16,
    dx: i16,
    dy: i16,
    child_bounds: [Bounds2; 2],
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

fn parse_map(data: &[u8], map_name: &str) -> Result<Map, io::Error> {
    let lumps = wad_lumps(data)?;
    let marker = lumps
        .iter()
        .position(|lump| lump.name() == map_name)
        .ok_or_else(|| invalid(format!("{map_name} not found")))?;
    let map_lumps = lumps
        .get(marker..marker + 1 + MAP_LUMPS_AFTER_MARKER.len())
        .ok_or_else(|| invalid(format!("truncated {map_name} lump sequence")))?;
    for (lump, expected) in map_lumps
        .iter()
        .zip(std::iter::once(map_name).chain(MAP_LUMPS_AFTER_MARKER))
    {
        if lump.name() != expected {
            return Err(invalid(format!("expected {expected} after {map_name}")));
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
                map_lumps[index].name()
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
                special: u16_at(r, 22)?,
                light,
                tag: u16_at(r, 24)?,
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
    let lines: Vec<[u16; 7]> = records(2, 14)?
        .into_iter()
        .map(|r| {
            Ok([
                u16_at(r, 0)?,
                u16_at(r, 2)?,
                u16_at(r, 4)?,
                u16_at(r, 10)?,
                u16_at(r, 12)?,
                u16_at(r, 6)?,
                u16_at(r, 8)?,
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
                child_bounds: [
                    Bounds2 {
                        min_x: i16_at(r, 12)? as f32,
                        min_y: i16_at(r, 10)? as f32,
                        max_x: i16_at(r, 14)? as f32,
                        max_y: i16_at(r, 8)? as f32,
                    },
                    Bounds2 {
                        min_x: i16_at(r, 20)? as f32,
                        min_y: i16_at(r, 18)? as f32,
                        max_x: i16_at(r, 22)? as f32,
                        max_y: i16_at(r, 16)? as f32,
                    },
                ],
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
        if node
            .child_bounds
            .iter()
            .any(|b| b.min_x > b.max_x || b.min_y > b.max_y)
        {
            return Err(invalid("NODES contains an inverted child bounding box"));
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

fn wall_textures(
    data: &[u8],
    map: &Map,
    sky_name: Option<[u8; 8]>,
) -> api::Result<BTreeMap<[u8; 8], Arc<Texture>>> {
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

    let mut names = map
        .sides
        .iter()
        .flat_map(|side| [side.upper, side.lower, side.middle])
        .filter(|name| name[0] != b'-' && name.iter().any(|&byte| byte != 0))
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(sky_name) = sky_name {
        names.insert(sky_name);
    }
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
            rgba.push(0);
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

fn sprite_actor_textures(data: &[u8], prefix: [u8; 4]) -> api::Result<Vec<Vec<SpriteTexture>>> {
    let lumps = wad_lumps(data)?;
    let last_frame = match &prefix {
        b"TROO" => b'M',
        b"SARG" => b'N',
        b"POSS" | b"SPOS" => b'L',
        _ => b'G',
    };
    (b'A'..=last_frame)
        .map(|frame| {
            let mut name = [0; 8];
            name[..4].copy_from_slice(&prefix);
            name[4] = frame;
            name[5] = b'0';
            if lumps.iter().any(|lump| lump.name == name) {
                return Ok(vec![sprite_patch_texture(data, name)?; 8]);
            }
            (1..=8)
                .map(|rotation| {
                    let (mut name, mut flip) = sprite_view_name(prefix, frame, rotation);
                    if !lumps.iter().any(|lump| lump.name == name) {
                        name = [0; 8];
                        name[..4].copy_from_slice(&prefix);
                        name[4] = frame;
                        name[5] = b'0' + rotation;
                        flip = false;
                    }
                    let mut texture = sprite_patch_texture(data, name)?;
                    texture.horizontal_flip = flip;
                    Ok(texture)
                })
                .collect()
        })
        .collect()
}

fn sprite_view_name(prefix: [u8; 4], frame: u8, rotation: u8) -> ([u8; 8], bool) {
    let (first, second, flip): (u8, Option<u8>, bool) = match rotation {
        1 => (b'1', None, false),
        2 | 8 => (b'2', Some(b'8'), rotation == 8),
        3 | 7 => (b'3', Some(b'7'), rotation == 7),
        4 | 6 => (b'4', Some(b'6'), rotation == 6),
        5 => (b'5', None, false),
        _ => unreachable!("sprite rotation must be between one and eight"),
    };
    let mut name = [0; 8];
    name[..4].copy_from_slice(&prefix);
    name[4] = frame;
    name[5] = first;
    if let Some(second) = second {
        name[6] = frame;
        name[7] = second;
    }
    (name, flip)
}

fn actor_walk_frame_tics(sprite: [u8; 4]) -> f32 {
    match &sprite {
        b"TROO" | b"SPOS" => 6.0,
        b"POSS" => 8.0,
        b"SARG" => 4.0,
        _ => 6.0,
    }
}

fn actor_walk_frame(sprite: [u8; 4], animation_time: f32) -> usize {
    let frame_tics = actor_walk_frame_tics(sprite);
    let cycle_seconds = frame_tics * 4.0 / 35.0;
    (animation_time.rem_euclid(cycle_seconds) * 35.0 / frame_tics).floor() as usize
}

fn actor_idle_frame(animation_time: f32) -> usize {
    (animation_time.rem_euclid(ACTOR_IDLE_CYCLE) * 35.0 / 10.0).floor() as usize
}

fn actor_view_rotation(actor_angle: f32, viewer_to_actor_angle: f32) -> usize {
    ((viewer_to_actor_angle - actor_angle + 202.5).rem_euclid(360.0) / 45.0).floor() as usize
}

fn actor_attack_profile(sprite: [u8; 4]) -> Option<([usize; 3], [f32; 3])> {
    match &sprite {
        b"TROO" => Some(([4, 5, 6], [8.0, 8.0, 6.0])),
        b"SARG" => Some(([4, 5, 6], [8.0, 8.0, 8.0])),
        b"POSS" => Some(([4, 5, 4], [10.0, 8.0, 8.0])),
        b"SPOS" => Some(([4, 5, 4], [10.0, 10.0, 10.0])),
        _ => None,
    }
}

fn actor_attack_duration(sprite: [u8; 4]) -> f32 {
    actor_attack_profile(sprite)
        .map(|(_, tics)| tics.into_iter().sum::<f32>() / 35.0)
        .unwrap_or_default()
}

fn actor_attack_frame(sprite: [u8; 4], remaining: f32) -> Option<usize> {
    let (frames, tics) = actor_attack_profile(sprite)?;
    if remaining <= 0.0 {
        return None;
    }
    let elapsed = (tics.into_iter().sum::<f32>() - remaining * 35.0).max(0.0);
    let mut end = 0.0;
    for (frame, duration) in frames.into_iter().zip(tics) {
        end += duration;
        if elapsed < end {
            return Some(frame);
        }
    }
    None
}

fn actor_pain_profile(sprite: [u8; 4]) -> Option<(usize, f32, u8)> {
    match &sprite {
        b"TROO" => Some((7, 4.0, 200)),
        b"SARG" => Some((7, 4.0, 180)),
        b"POSS" => Some((6, 6.0, 200)),
        b"SPOS" => Some((6, 6.0, 170)),
        _ => None,
    }
}

fn actor_pain_duration(sprite: [u8; 4]) -> f32 {
    actor_pain_profile(sprite).map_or(0.0, |(_, tics, _)| tics / 35.0)
}

fn actor_pain_frame(sprite: [u8; 4], remaining: f32) -> Option<usize> {
    actor_pain_profile(sprite)
        .filter(|_| remaining > 0.0)
        .map(|(frame, _, _)| frame)
}

fn actor_pain_triggered(sprite: [u8; 4], roll: u8) -> bool {
    actor_pain_profile(sprite).is_some_and(|(_, _, chance)| roll < chance)
}

// ponytail: one deterministic xorshift keeps the gameplay repeatable; use Doom's global RNG if exact replay compatibility is required.
fn gameplay_random_byte(state: &mut u32) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 24) as u8
}

fn actor_death_profile(sprite: [u8; 4]) -> Option<(&'static [usize], &'static [f32])> {
    match &sprite {
        b"TROO" => Some((&[8, 9, 10, 11, 12], &[8.0, 8.0, 6.0, 6.0])),
        b"SARG" => Some((&[8, 9, 10, 11, 12, 13], &[8.0, 8.0, 4.0, 4.0, 4.0])),
        b"POSS" | b"SPOS" => Some((&[7, 8, 9, 10, 11], &[5.0, 5.0, 5.0, 5.0])),
        _ => None,
    }
}

fn actor_death_duration(sprite: [u8; 4]) -> f32 {
    actor_death_profile(sprite)
        .map(|(_, tics)| tics.iter().sum::<f32>() / 35.0)
        .unwrap_or_default()
}

fn actor_death_frame(sprite: [u8; 4], elapsed: f32) -> Option<usize> {
    let (frames, durations) = actor_death_profile(sprite)?;
    let mut elapsed_tics = elapsed * 35.0;
    for (index, &duration) in durations.iter().enumerate() {
        if elapsed_tics < duration {
            return Some(frames[index]);
        }
        elapsed_tics -= duration;
    }
    frames.last().copied()
}

fn projectile_explosion_frame(elapsed: f32) -> Option<usize> {
    let mut elapsed_tics = elapsed * 35.0;
    for (frame, duration) in [6.0, 6.0, 6.0].into_iter().enumerate() {
        if elapsed_tics < duration {
            return Some(frame);
        }
        elapsed_tics -= duration;
    }
    None
}

fn projectile_explosion_duration() -> f32 {
    18.0 / 35.0
}

fn sprite_patch_texture(data: &[u8], name: [u8; 8]) -> api::Result<SpriteTexture> {
    let lumps = wad_lumps(data)?;
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
        horizontal_flip: false,
    })
}

fn world(point: Vertex2, height: f32) -> Vec3 {
    Vec3::new(point.x, height, -point.y)
}

fn geometry_bounds(vertices: &[Vertex]) -> Option<Bounds3> {
    let mut positions = vertices.iter().map(|vertex| vertex.position);
    let first = positions.next()?;
    let mut bounds = Bounds3 {
        min: first,
        max: first,
    };
    for point in positions {
        bounds.min.x = bounds.min.x.min(point.x);
        bounds.min.y = bounds.min.y.min(point.y);
        bounds.min.z = bounds.min.z.min(point.z);
        bounds.max.x = bounds.max.x.max(point.x);
        bounds.max.y = bounds.max.y.max(point.y);
        bounds.max.z = bounds.max.z.max(point.z);
    }
    Some(bounds)
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

fn clip_bsp_polygon(polygon: &[Vertex2], node: Node, side: usize) -> Vec<Vertex2> {
    let distance = |point: Vertex2| {
        node.dx as f64 * (point.y as f64 - node.y as f64)
            - node.dy as f64 * (point.x as f64 - node.x as f64)
    };
    let inside = |distance: f64| {
        if side == 0 {
            distance <= 0.0
        } else {
            distance >= 0.0
        }
    };
    let mut clipped = Vec::with_capacity(polygon.len() + 1);
    let Some(&last) = polygon.last() else {
        return clipped;
    };
    let mut previous = last;
    let mut previous_distance = distance(previous);
    let mut previous_inside = inside(previous_distance);
    for &current in polygon {
        let current_distance = distance(current);
        let current_inside = inside(current_distance);
        if previous_inside != current_inside {
            let t = previous_distance / (previous_distance - current_distance);
            clipped.push(Vertex2 {
                x: (previous.x as f64 + (current.x as f64 - previous.x as f64) * t) as f32,
                y: (previous.y as f64 + (current.y as f64 - previous.y as f64) * t) as f32,
            });
        }
        if current_inside {
            clipped.push(current);
        }
        previous = current;
        previous_distance = current_distance;
        previous_inside = current_inside;
    }
    clipped
}

fn bsp_subsector_polygons(map: &Map) -> Vec<Vec<Vertex2>> {
    let mut polygons = vec![Vec::new(); map.subsectors.len()];
    if map.nodes.is_empty() || map.vertices.is_empty() {
        return polygons;
    }
    let bounds = map.vertices.iter().fold(
        Bounds2 {
            min_x: f32::INFINITY,
            min_y: f32::INFINITY,
            max_x: f32::NEG_INFINITY,
            max_y: f32::NEG_INFINITY,
        },
        |mut bounds, point| {
            bounds.min_x = bounds.min_x.min(point.x);
            bounds.min_y = bounds.min_y.min(point.y);
            bounds.max_x = bounds.max_x.max(point.x);
            bounds.max_y = bounds.max_y.max(point.y);
            bounds
        },
    );
    if bounds.min_x >= bounds.max_x || bounds.min_y >= bounds.max_y {
        return polygons;
    }
    let rectangle = [
        Vertex2 {
            x: bounds.min_x,
            y: bounds.min_y,
        },
        Vertex2 {
            x: bounds.max_x,
            y: bounds.min_y,
        },
        Vertex2 {
            x: bounds.max_x,
            y: bounds.max_y,
        },
        Vertex2 {
            x: bounds.min_x,
            y: bounds.max_y,
        },
    ];
    let mut visited_nodes = vec![false; map.nodes.len()];
    let mut visited_leaves = vec![false; map.subsectors.len()];
    let mut pending = vec![(map.nodes.len() - 1, rectangle.to_vec())];
    while let Some((child, polygon)) = pending.pop() {
        if child & 0x8000 != 0 {
            let leaf = child & 0x7fff;
            if leaf < polygons.len() && !std::mem::replace(&mut visited_leaves[leaf], true) {
                polygons[leaf] = polygon;
            }
            continue;
        }
        let Some(seen) = visited_nodes.get_mut(child) else {
            continue;
        };
        if std::mem::replace(seen, true) {
            continue;
        }
        let node = map.nodes[child];
        for side in [1, 0] {
            let polygon = clip_bsp_polygon(&polygon, node, side);
            if polygon.len() >= 3 {
                pending.push((node.children[side] as usize, polygon));
            }
        }
    }
    polygons
}

fn sector_contains_point(map: &Map, sector: u16, point: Vertex2) -> bool {
    let mut inside = false;
    for line in &map.lines {
        let front = map.sides.get(line[3] as usize).map(|side| side.sector) == Some(sector);
        let back = map.sides.get(line[4] as usize).map(|side| side.sector) == Some(sector);
        if front == back {
            continue;
        }
        let (Some(&a), Some(&b)) = (
            map.vertices.get(line[0] as usize),
            map.vertices.get(line[1] as usize),
        ) else {
            continue;
        };
        if (a.y > point.y) != (b.y > point.y) {
            let crossing_x = a.x + (point.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if point.x < crossing_x {
                inside = !inside;
            }
        }
    }
    inside
}

fn bsp_polygon_belongs_to_sector(map: &Map, polygon: &[Vertex2], sector: u16) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let center = polygon
        .iter()
        .fold(Vertex2 { x: 0.0, y: 0.0 }, |sum, point| Vertex2 {
            x: sum.x + point.x / polygon.len() as f32,
            y: sum.y + point.y / polygon.len() as f32,
        });
    if !sector_contains_point(map, sector, center) {
        return false;
    }
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[(i + 1) % polygon.len()];
        for point in [
            a,
            Vertex2 {
                x: (a.x + b.x) * 0.5,
                y: (a.y + b.y) * 0.5,
            },
        ] {
            let sample = Vertex2 {
                x: center.x + (point.x - center.x) * 0.99,
                y: center.y + (point.y - center.y) * 0.99,
            };
            if !sector_contains_point(map, sector, sample) {
                return false;
            }
        }
    }
    true
}

fn line_side(a: Vertex2, b: Vertex2, point: Vertex2) -> f64 {
    (b.x - a.x) as f64 * (point.y - a.y) as f64 - (b.y - a.y) as f64 * (point.x - a.x) as f64
}

fn point_inside_convex_polygon(point: Vertex2, polygon: &[Vertex2]) -> bool {
    let mut positive = false;
    let mut negative = false;
    for i in 0..polygon.len() {
        let side = line_side(polygon[i], polygon[(i + 1) % polygon.len()], point);
        positive |= side > 0.0;
        negative |= side < 0.0;
    }
    !(positive && negative) && (positive || negative)
}

fn sector_boundary_crosses_polygon(a: Vertex2, b: Vertex2, polygon: &[Vertex2]) -> bool {
    if point_inside_convex_polygon(a, polygon) || point_inside_convex_polygon(b, polygon) {
        return true;
    }
    for i in 0..polygon.len() {
        let c = polygon[i];
        let d = polygon[(i + 1) % polygon.len()];
        if (line_side(a, b, c) > 0.0) != (line_side(a, b, d) > 0.0)
            && (line_side(c, d, a) > 0.0) != (line_side(c, d, b) > 0.0)
        {
            return true;
        }
    }
    false
}

fn clip_polygon_to_line(
    polygon: &[Vertex2],
    a: Vertex2,
    b: Vertex2,
    positive: bool,
) -> Vec<Vertex2> {
    let distance = |point| line_side(a, b, point) * if positive { 1.0 } else { -1.0 };
    let Some(&last) = polygon.last() else {
        return Vec::new();
    };
    let mut clipped = Vec::with_capacity(polygon.len() + 1);
    let mut previous = last;
    let mut previous_distance = distance(previous);
    for &current in polygon {
        let current_distance = distance(current);
        if (previous_distance < 0.0) != (current_distance < 0.0) {
            let t = previous_distance / (previous_distance - current_distance);
            clipped.push(Vertex2 {
                x: (previous.x as f64 + (current.x as f64 - previous.x as f64) * t) as f32,
                y: (previous.y as f64 + (current.y as f64 - previous.y as f64) * t) as f32,
            });
        }
        if current_distance >= 0.0 {
            clipped.push(current);
        }
        previous = current;
        previous_distance = current_distance;
    }
    clipped
}

fn split_polygon_by_line(
    polygon: &[Vertex2],
    a: Vertex2,
    b: Vertex2,
) -> Option<(Vec<Vertex2>, Vec<Vertex2>)> {
    let positive = polygon.iter().any(|&point| line_side(a, b, point) > 0.0);
    let negative = polygon.iter().any(|&point| line_side(a, b, point) < 0.0);
    (positive && negative).then(|| {
        (
            convex_hull(clip_polygon_to_line(polygon, a, b, true)),
            convex_hull(clip_polygon_to_line(polygon, a, b, false)),
        )
    })
}

fn sector_clipped_bsp_polygons(map: &Map, polygon: &[Vertex2], sector: u16) -> Vec<Vec<Vertex2>> {
    if polygon.len() < 3 {
        return Vec::new();
    }
    let mut polygons = vec![polygon.to_vec()];
    for line in &map.lines {
        let front = map.sides.get(line[3] as usize).map(|side| side.sector);
        let back = map.sides.get(line[4] as usize).map(|side| side.sector);
        if (front == Some(sector)) == (back == Some(sector)) {
            continue;
        }
        let (Some(&a), Some(&b)) = (
            map.vertices.get(line[0] as usize),
            map.vertices.get(line[1] as usize),
        ) else {
            continue;
        };
        let mut split = Vec::with_capacity(polygons.len() + 1);
        for piece in polygons {
            if sector_boundary_crosses_polygon(a, b, &piece)
                && let Some((positive, negative)) = split_polygon_by_line(&piece, a, b)
            {
                split.push(positive);
                split.push(negative);
                continue;
            }
            split.push(piece);
        }
        polygons = split;
    }
    polygons.retain(|piece| bsp_polygon_belongs_to_sector(map, piece, sector));
    polygons
}

#[derive(Default)]
struct Geometry {
    flats: BTreeMap<[u8; 8], Vec<Vertex>>,
    walls: BTreeMap<[u8; 8], Vec<Vertex>>,
    masked: BTreeMap<[u8; 8], Vec<Vertex>>,
}

#[derive(Clone, Copy)]
struct Player {
    x: f32,
    y: f32,
    angle: f32,
}

fn sky_texture_name(map: &Map, map_name: &str) -> Option<[u8; 8]> {
    if !map
        .sectors
        .iter()
        .any(|sector| sector.ceiling_flat == *b"F_SKY1\0\0")
    {
        return None;
    }
    let texture = if map_name.starts_with('E') {
        match map_name.as_bytes().get(1).copied() {
            Some(b'2') => *b"SKY2\0\0\0\0",
            Some(b'3') => *b"SKY3\0\0\0\0",
            Some(b'4') => *b"SKY4\0\0\0\0",
            _ => *b"SKY1\0\0\0\0",
        }
    } else if map_name.starts_with("MAP") {
        let map_number = map_name
            .as_bytes()
            .get(3..5)
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| digits.parse::<u8>().ok())
            .unwrap_or(1);
        if map_number < 12 {
            *b"SKY1\0\0\0\0"
        } else if map_number < 21 {
            *b"SKY2\0\0\0\0"
        } else {
            *b"SKY3\0\0\0\0"
        }
    } else {
        *b"SKY1\0\0\0\0"
    };
    Some(texture)
}

fn sky_uv(
    player: Player,
    ndc_x: f32,
    ndc_y: f32,
    texture_width: u32,
    texture_height: u32,
    output_width: u32,
    output_height: u32,
) -> Vec2 {
    let half_vertical_fov = 1.22_f32 * 0.5;
    let camera_x = ndc_x * half_vertical_fov.tan() * output_width as f32 / output_height as f32;
    let yaw = player.angle.to_radians() - camera_x.atan();
    let source_y = DOOM_SKY_MID
        - ndc_y * output_height as f32 * 0.5 * DOOM_SKY_BASE_WIDTH / output_width as f32;
    Vec2::new(
        yaw * DOOM_SKY_ANGLE_COLUMNS / (std::f32::consts::TAU * texture_width as f32),
        source_y / texture_height as f32,
    )
}

fn sky_vertices(
    player: Player,
    eye_height: f32,
    texture_size: (u32, u32),
    output_size: (u32, u32),
) -> Vec<Vertex> {
    let (texture_width, texture_height) = texture_size;
    let (output_width, output_height) = output_size;
    if texture_width == 0 || texture_height == 0 || output_width == 0 || output_height == 0 {
        return Vec::new();
    }
    let angle = player.angle.to_radians();
    let forward = Vec3::new(angle.cos(), 0.0, -angle.sin());
    let up = Vec3::new(0.0, 1.0, 0.0);
    let right = forward.cross(up);
    let distance = 4096.0;
    let center = Vec3::new(player.x, eye_height, -player.y) + forward * distance;
    let half_height = distance * (1.22_f32 * 0.5).tan();
    let aspect = output_width as f32 / output_height as f32;
    let mut vertices = Vec::with_capacity(SKY_MESH_SEGMENTS * 6);
    for segment in 0..SKY_MESH_SEGMENTS {
        let ndc_left = segment as f32 / SKY_MESH_SEGMENTS as f32 * 2.0 - 1.0;
        let ndc_right = (segment + 1) as f32 / SKY_MESH_SEGMENTS as f32 * 2.0 - 1.0;
        let camera_left = ndc_left * (1.22_f32 * 0.5).tan() * aspect;
        let camera_right = ndc_right * (1.22_f32 * 0.5).tan() * aspect;
        let left_center = center + right * (camera_left * distance);
        let right_center = center + right * (camera_right * distance);
        let top_left = left_center + up * half_height;
        let bottom_left = left_center - up * half_height;
        let top_right = right_center + up * half_height;
        let bottom_right = right_center - up * half_height;
        let uv_top_left = sky_uv(
            player,
            ndc_left,
            1.0,
            texture_width,
            texture_height,
            output_width,
            output_height,
        );
        let uv_bottom_left = sky_uv(
            player,
            ndc_left,
            -1.0,
            texture_width,
            texture_height,
            output_width,
            output_height,
        );
        let uv_top_right = sky_uv(
            player,
            ndc_right,
            1.0,
            texture_width,
            texture_height,
            output_width,
            output_height,
        );
        let uv_bottom_right = sky_uv(
            player,
            ndc_right,
            -1.0,
            texture_width,
            texture_height,
            output_width,
            output_height,
        );
        push_triangle_uv(
            &mut vertices,
            [
                (top_left, uv_top_left),
                (bottom_left, uv_bottom_left),
                (bottom_right, uv_bottom_right),
            ],
            Vec4::new(1.0, 1.0, 1.0, 1.0),
        );
        push_triangle_uv(
            &mut vertices,
            [
                (top_left, uv_top_left),
                (bottom_right, uv_bottom_right),
                (top_right, uv_top_right),
            ],
            Vec4::new(1.0, 1.0, 1.0, 1.0),
        );
    }
    vertices
}

struct Door {
    sector: u16,
    top: f32,
    speed: f32,
    wait: f32,
    direction: i8,
    auto_close: bool,
}

struct Platform {
    sector: u16,
    low: f32,
    high: f32,
    speed: f32,
    wait: f32,
    direction: i8,
    return_to_high: bool,
}

struct SectorLight {
    sector: usize,
    min: u8,
    max: u8,
    tics: f32,
    bright: bool,
    random: bool,
}

#[derive(Clone, Copy)]
struct Controls {
    forward: f32,
    strafe: f32,
    turn: f32,
    speed: f32,
}

struct Draw {
    name: [u8; 8],
    wall: bool,
    masked: bool,
    texture: Arc<Texture>,
    vertices: Vec<Vertex>,
    bounds: Option<Bounds3>,
}

struct PreparedScene {
    map_name: String,
    map: Map,
    flat_textures: BTreeMap<[u8; 8], Arc<Texture>>,
    wall_textures: BTreeMap<[u8; 8], Arc<Texture>>,
    sky_texture: Option<Arc<Texture>>,
    device: Device,
    pipeline: Arc<ShaderPipeline>,
    sky_pipeline: Option<Arc<ShaderPipeline>>,
    sprite_pipeline: Arc<ShaderPipeline>,
    weapon_pipeline: Arc<ShaderPipeline>,
    sampler: Sampler,
    draws: Vec<Vec<Draw>>,
    visibility_fallbacks: Vec<(usize, Bounds2)>,
    sprites: BTreeMap<[u8; 4], Vec<Vec<SpriteTexture>>>,
    pickup_sprites: BTreeMap<[u8; 4], SpriteTexture>,
    weapon_idle: SpriteTexture,
    weapon_fire: SpriteTexture,
    projectile_sprite: SpriteTexture,
    projectile_explosion: [SpriteTexture; 3],
    actors: Vec<Actor>,
    pickups: Vec<Pickup>,
    start: Player,
}

#[derive(Clone)]
struct SpriteTexture {
    texture: Arc<Texture>,
    width: f32,
    height: f32,
    left_offset: f32,
    horizontal_flip: bool,
}

#[derive(Clone, Copy)]
struct Actor {
    sprite: [u8; 4],
    x: f32,
    y: f32,
    health: i32,
    target_time_remaining: f32,
    attack_cooldown: f32,
    attack_animation_remaining: f32,
    pain_animation_remaining: f32,
    death_animation_time: Option<f32>,
    animation_time: f32,
    angle: f32,
}

struct Projectile {
    x: f32,
    y: f32,
    z: f32,
    velocity_x: f32,
    velocity_y: f32,
    velocity_z: f32,
    lifetime: f32,
    explosion_time: Option<f32>,
}

#[derive(Clone, Copy)]
struct Pickup {
    sprite: [u8; 4],
    x: f32,
    y: f32,
    health: i32,
    ammo: i32,
    blue_key: bool,
    active: bool,
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

fn billboard_vertices(
    left: Vertex2,
    axis: Vertex2,
    width: f32,
    bottom: f32,
    height: f32,
    sector: Sector,
    horizontal_flip: bool,
) -> Vec<Vertex> {
    let right = Vertex2 {
        x: left.x + axis.x * width,
        y: left.y + axis.y * width,
    };
    let bottom_left = world(left, bottom);
    let bottom_right = world(right, bottom);
    let top_left = world(left, bottom + height);
    let top_right = world(right, bottom + height);
    let color = shaded([1.0; 3], sector);
    let (left_u, right_u) = if horizontal_flip {
        (1.0, 0.0)
    } else {
        (0.0, 1.0)
    };
    let mut vertices = Vec::with_capacity(6);
    push_triangle_uv(
        &mut vertices,
        [
            (bottom_left, Vec2::new(left_u, 1.0)),
            (bottom_right, Vec2::new(right_u, 1.0)),
            (top_right, Vec2::new(right_u, 0.0)),
        ],
        color,
    );
    push_triangle_uv(
        &mut vertices,
        [
            (bottom_left, Vec2::new(left_u, 1.0)),
            (top_right, Vec2::new(right_u, 0.0)),
            (top_left, Vec2::new(left_u, 0.0)),
        ],
        color,
    );
    vertices
}

fn sprite_vertices(
    x: f32,
    y: f32,
    sprite: &SpriteTexture,
    camera_angle: f32,
    sector: Sector,
    bottom: f32,
) -> Vec<Vertex> {
    let radians = camera_angle.to_radians();
    let axis = Vertex2 {
        x: radians.sin(),
        y: -radians.cos(),
    };
    let left = Vertex2 {
        x: x - axis.x * sprite.left_offset,
        y: y - axis.y * sprite.left_offset,
    };
    billboard_vertices(
        left,
        axis,
        sprite.width,
        bottom,
        sprite.height,
        sector,
        sprite.horizontal_flip,
    )
}

fn projectile_vertices(
    projectile: &Projectile,
    sprite: &SpriteTexture,
    camera_angle: f32,
    sector: Sector,
) -> Vec<Vertex> {
    sprite_vertices(
        projectile.x,
        projectile.y,
        sprite,
        camera_angle,
        sector,
        projectile.z,
    )
}

fn pickup_definition(kind: u16) -> Option<([u8; 4], i32, i32, bool)> {
    match kind {
        5 => Some((*b"BKEY", 0, 0, true)),
        2011 => Some((*b"STIM", 10, 0, false)),
        2012 => Some((*b"MEDI", 25, 0, false)),
        2007 => Some((*b"CLIP", 0, 10, false)),
        2048 => Some((*b"AMMO", 0, 50, false)),
        _ => None,
    }
}

fn collect_pickups(
    map: &Map,
    pickups: &mut [Pickup],
    player: Player,
    health: &mut i32,
    ammo: &mut i32,
    blue_key: &mut bool,
) -> usize {
    let mut collected = 0;
    for pickup in pickups.iter_mut().filter(|pickup| pickup.active) {
        let dx = pickup.x - player.x;
        let dy = pickup.y - player.y;
        if dx * dx + dy * dy > 24.0 * 24.0
            || !has_line_of_sight(
                map,
                Vertex2 {
                    x: player.x,
                    y: player.y,
                },
                Vertex2 {
                    x: pickup.x,
                    y: pickup.y,
                },
            )
        {
            continue;
        }
        let next_health = (*health + pickup.health).min(100);
        let next_ammo = (*ammo + pickup.ammo).min(200);
        let next_blue_key = *blue_key || pickup.blue_key;
        if next_health == *health && next_ammo == *ammo && next_blue_key == *blue_key {
            continue;
        }
        *health = next_health;
        *ammo = next_ammo;
        *blue_key = next_blue_key;
        pickup.active = false;
        collected += 1;
    }
    collected
}

fn weapon_vertices(player: Player, sprite: &SpriteTexture, sector: Sector) -> Vec<Vertex> {
    let radians = player.angle.to_radians();
    let forward = Vertex2 {
        x: radians.cos(),
        y: radians.sin(),
    };
    let axis = Vertex2 {
        x: radians.sin(),
        y: -radians.cos(),
    };
    let width = sprite.width * 0.4;
    let height = sprite.height * 0.4;
    let center = Vertex2 {
        x: player.x + forward.x * 64.0 + axis.x * 14.0,
        y: player.y + forward.y * 64.0 + axis.y * 14.0,
    };
    let left = Vertex2 {
        x: center.x - axis.x * width * 0.5,
        y: center.y - axis.y * width * 0.5,
    };
    billboard_vertices(left, axis, width, sector.floor + 5.0, height, sector, false)
}

fn geometry(
    map: &Map,
    textures: &BTreeMap<[u8; 8], Arc<Texture>>,
) -> Result<Vec<Geometry>, io::Error> {
    let bsp_polygons = bsp_subsector_polygons(map);
    let mut all = Vec::with_capacity(map.subsectors.len());
    for (leaf_index, leaf) in map.subsectors.iter().enumerate() {
        let mut out = Geometry::default();
        let segs = &map.segs[leaf[1] as usize..leaf[1] as usize + leaf[0] as usize];
        if let Some(first) = segs.first() {
            let front_side = map.lines[first[2] as usize][3 + first[3] as usize];
            let sector_index = map.sides[front_side as usize].sector;
            let sector = map.sectors[sector_index as usize];
            let polygon = convex_hull(
                segs.iter()
                    .flat_map(|seg| [map.vertices[seg[0] as usize], map.vertices[seg[1] as usize]])
                    .collect(),
            );
            let cell = convex_hull(bsp_polygons[leaf_index].clone());
            let mut polygons = sector_clipped_bsp_polygons(map, &cell, sector_index);
            if polygons.is_empty() {
                polygons.push(polygon);
            }
            for polygon in polygons {
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
        }

        for seg in segs {
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
            if let Some(texture) = textures.get(&front_sidedef.middle) {
                let texture_height = texture.levels[0].height as f32;
                let (low, high, anchor) = if line[2] & 16 != 0 {
                    let low = front.floor.max(back.floor);
                    (low, low + texture_height, low + texture_height)
                } else {
                    let high = front.ceiling.min(back.ceiling);
                    (high - texture_height, high, high)
                };
                push_wall_quad(
                    &mut out.masked,
                    texture,
                    WallSection {
                        name: front_sidedef.middle,
                        side: front_sidedef,
                        seg: *seg,
                        endpoints: [a, b],
                        heights: [low, high, anchor],
                        sector: front,
                    },
                );
            }
        }
        all.push(out);
    }
    Ok(all)
}

fn build_draws(
    geometry: Vec<Geometry>,
    flat_textures: &BTreeMap<[u8; 8], Arc<Texture>>,
    wall_textures: &BTreeMap<[u8; 8], Arc<Texture>>,
) -> api::Result<Vec<Vec<Draw>>> {
    geometry
        .into_iter()
        .map(|leaf| {
            let mut draws = Vec::new();
            for (name, vertices) in leaf.flats {
                let texture = flat_textures
                    .get(&name)
                    .ok_or_else(|| {
                        invalid(format!(
                            "flat {} was not decoded",
                            String::from_utf8_lossy(&name)
                        ))
                    })?
                    .clone();
                let bounds = geometry_bounds(&vertices);
                draws.push(Draw {
                    name,
                    wall: false,
                    masked: false,
                    texture,
                    vertices,
                    bounds,
                });
            }
            for (name, vertices) in leaf.walls {
                let texture = wall_textures
                    .get(&name)
                    .ok_or_else(|| {
                        invalid(format!(
                            "wall texture {} was not decoded",
                            String::from_utf8_lossy(&name)
                        ))
                    })?
                    .clone();
                let bounds = geometry_bounds(&vertices);
                draws.push(Draw {
                    name,
                    wall: true,
                    masked: false,
                    texture,
                    vertices,
                    bounds,
                });
            }
            for (name, vertices) in leaf.masked {
                let texture = wall_textures
                    .get(&name)
                    .ok_or_else(|| {
                        invalid(format!(
                            "masked wall texture {} was not decoded",
                            String::from_utf8_lossy(&name)
                        ))
                    })?
                    .clone();
                let bounds = geometry_bounds(&vertices);
                draws.push(Draw {
                    name,
                    wall: true,
                    masked: true,
                    texture,
                    vertices,
                    bounds,
                });
            }
            Ok(draws)
        })
        .collect()
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

fn bounds_in_view(bounds: Bounds2, player: Player) -> bool {
    let angle = player.angle.to_radians();
    let (sin, cos) = angle.sin_cos();
    let tan_half_fov = (1.22_f32 * 0.5).tan() * (4.0 / 3.0);
    let mut minimum = [f32::INFINITY; 4];
    for (x, y) in [
        (bounds.min_x, bounds.min_y),
        (bounds.min_x, bounds.max_y),
        (bounds.max_x, bounds.min_y),
        (bounds.max_x, bounds.max_y),
    ] {
        let dx = x - player.x;
        let dy = y - player.y;
        let forward = dx * cos + dy * sin;
        let right = dx * sin - dy * cos;
        for (index, plane) in [
            1.0 - forward,
            forward - 8192.0,
            right - forward * tan_half_fov,
            -right - forward * tan_half_fov,
        ]
        .into_iter()
        .enumerate()
        {
            minimum[index] = minimum[index].min(plane);
        }
    }
    minimum.into_iter().all(|distance| distance <= 0.0)
}

fn bounds3_in_view(bounds: Bounds3, player: Player, eye_height: f32) -> bool {
    let (sin, cos) = player.angle.to_radians().sin_cos();
    let tan_half_horizontal = (1.22_f32 * 0.5).tan() * (4.0 / 3.0);
    let tan_half_vertical = (1.22_f32 * 0.5).tan();
    let mut minimum = [f32::INFINITY; 6];
    for x in [bounds.min.x, bounds.max.x] {
        for y in [bounds.min.y, bounds.max.y] {
            for z in [bounds.min.z, bounds.max.z] {
                let dx = x - player.x;
                let dy = -z - player.y;
                let forward = dx * cos + dy * sin;
                let right = dx * sin - dy * cos;
                let vertical = y - eye_height;
                for (index, plane) in [
                    1.0 - forward,
                    forward - 8192.0,
                    right - forward * tan_half_horizontal,
                    -right - forward * tan_half_horizontal,
                    vertical - forward * tan_half_vertical,
                    -vertical - forward * tan_half_vertical,
                ]
                .into_iter()
                .enumerate()
                {
                    minimum[index] = minimum[index].min(plane);
                }
            }
        }
    }
    minimum.into_iter().all(|distance| distance <= 0.0)
}

fn depth_bucket(bounds: Bounds3, player: Player) -> u32 {
    let (sin, cos) = player.angle.to_radians().sin_cos();
    let nearest = [bounds.min.x, bounds.max.x]
        .into_iter()
        .flat_map(|x| {
            [bounds.min.z, bounds.max.z]
                .into_iter()
                .map(move |z| (x - player.x) * cos + (-z - player.y) * sin)
        })
        .fold(f32::INFINITY, f32::min)
        .max(0.0);
    (nearest / DEPTH_BUCKET_SIZE) as u32
}

fn visible_subsector_order(map: &Map, player: Player) -> Vec<usize> {
    if map.nodes.is_empty() {
        return subsector_at(
            map,
            Vertex2 {
                x: player.x,
                y: player.y,
            },
        )
        .into_iter()
        .collect();
    }
    let mut visible = Vec::new();
    let mut visited = vec![false; map.nodes.len()];
    let mut visited_leaves = vec![false; map.subsectors.len()];
    let mut pending = vec![(map.nodes.len() - 1, None)];
    while let Some((child, bounds)) = pending.pop() {
        if bounds.is_some_and(|bounds| !bounds_in_view(bounds, player)) {
            continue;
        }
        if child & 0x8000 != 0 {
            let leaf = child & 0x7fff;
            if leaf < map.subsectors.len() && !std::mem::replace(&mut visited_leaves[leaf], true) {
                visible.push(leaf);
            }
            continue;
        }
        let Some(seen) = visited.get_mut(child) else {
            continue;
        };
        if std::mem::replace(seen, true) {
            continue;
        }
        let node = map.nodes[child];
        let near = point_on_node_side(
            Vertex2 {
                x: player.x,
                y: player.y,
            },
            node,
        );
        for side in [1 - near, near] {
            pending.push((node.children[side] as usize, Some(node.child_bounds[side])));
        }
    }
    if let Some(leaf) = subsector_at(
        map,
        Vertex2 {
            x: player.x,
            y: player.y,
        },
    ) && !std::mem::replace(&mut visited_leaves[leaf], true)
    {
        visible.push(leaf);
    }
    visible
}

fn horizontal_bounds(bounds: Bounds3) -> Bounds2 {
    Bounds2 {
        min_x: bounds.min.x,
        min_y: -bounds.max.z,
        max_x: bounds.max.x,
        max_y: -bounds.min.z,
    }
}

fn contains_bounds(outer: Bounds2, inner: Bounds2) -> bool {
    outer.min_x <= inner.min_x
        && outer.min_y <= inner.min_y
        && outer.max_x >= inner.max_x
        && outer.max_y >= inner.max_y
}

fn intersect_bounds(a: Bounds2, b: Bounds2) -> Bounds2 {
    Bounds2 {
        min_x: a.min_x.max(b.min_x),
        min_y: a.min_y.max(b.min_y),
        max_x: a.max_x.min(b.max_x),
        max_y: a.max_y.min(b.max_y),
    }
}

fn visibility_fallback_bounds(map: &Map, draws: &[Vec<Draw>]) -> Vec<(usize, Bounds2)> {
    if map.nodes.is_empty() {
        return Vec::new();
    }
    let mut geometry_bounds = vec![None; map.subsectors.len()];
    for (leaf, leaf_draws) in draws.iter().enumerate().take(geometry_bounds.len()) {
        for bounds in leaf_draws
            .iter()
            .filter_map(|draw| draw.bounds.map(horizontal_bounds))
        {
            let combined = geometry_bounds[leaf].get_or_insert(bounds);
            combined.min_x = combined.min_x.min(bounds.min_x);
            combined.min_y = combined.min_y.min(bounds.min_y);
            combined.max_x = combined.max_x.max(bounds.max_x);
            combined.max_y = combined.max_y.max(bounds.max_y);
        }
    }
    let mut fallback = Vec::new();
    let mut visited_nodes = vec![false; map.nodes.len()];
    let mut visited_leaves = vec![false; map.subsectors.len()];
    let mut pending = vec![(map.nodes.len() - 1, None)];
    while let Some((child, bounds)) = pending.pop() {
        if child & 0x8000 != 0 {
            let leaf = child & 0x7fff;
            if leaf < geometry_bounds.len()
                && !std::mem::replace(&mut visited_leaves[leaf], true)
                && let (Some(actual), Some(node_bounds)) = (geometry_bounds[leaf], bounds)
                && !contains_bounds(node_bounds, actual)
            {
                fallback.push((leaf, actual));
            }
            continue;
        }
        let Some(seen) = visited_nodes.get_mut(child) else {
            continue;
        };
        if std::mem::replace(seen, true) {
            continue;
        }
        let node = map.nodes[child];
        for side in 0..2 {
            let child_bounds = node.child_bounds[side];
            let bounds = bounds.map_or(child_bounds, |parent| {
                intersect_bounds(parent, child_bounds)
            });
            pending.push((node.children[side] as usize, Some(bounds)));
        }
    }
    fallback
}

fn visible_geometry_order(
    map: &Map,
    player: Player,
    fallback_bounds: &[(usize, Bounds2)],
) -> Vec<usize> {
    let mut visible = visible_subsector_order(map, player);
    let mut included = vec![false; map.subsectors.len()];
    for &leaf in &visible {
        included[leaf] = true;
    }
    for &(leaf, bounds) in fallback_bounds {
        if leaf >= included.len() || included[leaf] || !bounds_in_view(bounds, player) {
            continue;
        }
        included[leaf] = true;
        visible.push(leaf);
    }
    visible
}

fn bsp_sector_at(map: &Map, x: f32, y: f32) -> Option<Sector> {
    map.sectors
        .get(bsp_sector_index_at(map, x, y)? as usize)
        .copied()
}

fn bsp_sector_index_at(map: &Map, x: f32, y: f32) -> Option<u16> {
    let leaf = map.subsectors.get(subsector_at(map, Vertex2 { x, y })?)?;
    if leaf[0] == 0 {
        return None;
    }
    let seg = map.segs.get(leaf[1] as usize)?;
    let line = map.lines.get(seg[2] as usize)?;
    let sidedef = map.sides.get(*line.get(3 + seg[3] as usize)? as usize)?;
    map.sectors
        .get(sidedef.sector as usize)
        .map(|_| sidedef.sector)
}

fn discover_secret(map: &mut Map, player: Player) -> bool {
    let Some(index) = bsp_sector_index_at(map, player.x, player.y) else {
        return false;
    };
    map.sectors.get_mut(index as usize).is_some_and(|sector| {
        if sector.special != SECTOR_SECRET {
            return false;
        }
        sector.special = 0;
        true
    })
}

fn update_floor_damage(
    map: &Map,
    player: Player,
    health: &mut i32,
    elapsed_tics: &mut f32,
    delta: f32,
) {
    if bsp_sector_at(map, player.x, player.y)
        .is_none_or(|sector| sector.special != SECTOR_NUKAGE_DAMAGE)
    {
        *elapsed_tics = 0.0;
        return;
    }
    *elapsed_tics += delta * DOOM_TICS_PER_SECOND;
    while *elapsed_tics >= NUKAGE_DAMAGE_TICS {
        *elapsed_tics -= NUKAGE_DAMAGE_TICS;
        *health = (*health - NUKAGE_DAMAGE).max(0);
    }
}

fn lowest_surrounding_light(map: &Map, sector: usize) -> u8 {
    let mut lowest = map.sectors[sector].light;
    for line in &map.lines {
        let front = map
            .sides
            .get(line[3] as usize)
            .map(|side| side.sector as usize);
        let back = map
            .sides
            .get(line[4] as usize)
            .map(|side| side.sector as usize);
        let neighbor = if front == Some(sector) {
            back
        } else if back == Some(sector) {
            front
        } else {
            None
        };
        if let Some(neighbor) = neighbor.and_then(|index| map.sectors.get(index)) {
            lowest = lowest.min(neighbor.light);
        }
    }
    lowest
}

fn spawn_sector_lights(map: &mut Map, rng: &mut u32) -> Vec<SectorLight> {
    let mut lights = Vec::new();
    for sector_index in 0..map.sectors.len() {
        let sector = map.sectors[sector_index];
        let (random, min) = match sector.special {
            SECTOR_LIGHT_FLASH => (true, lowest_surrounding_light(map, sector_index)),
            SECTOR_LIGHT_STROBE_SLOW => {
                let min = lowest_surrounding_light(map, sector_index);
                (false, if min == sector.light { 0 } else { min })
            }
            _ => continue,
        };
        lights.push(SectorLight {
            sector: sector_index,
            min,
            max: sector.light,
            tics: if random {
                f32::from(gameplay_random_byte(rng) & 64) + 1.0
            } else {
                1.0
            },
            bright: true,
            random,
        });
        map.sectors[sector_index].special = 0;
    }
    lights
}

fn update_sector_lights(
    map: &mut Map,
    lights: &mut [SectorLight],
    rng: &mut u32,
    delta: f32,
) -> bool {
    let mut changed = false;
    for light in lights {
        light.tics -= delta * DOOM_TICS_PER_SECOND;
        while light.tics <= 0.0 {
            let sector = &mut map.sectors[light.sector];
            if light.bright {
                sector.light = light.min;
                light.bright = false;
                light.tics += if light.random {
                    f32::from(gameplay_random_byte(rng) & 7) + 1.0
                } else {
                    STROBE_SLOW_DARK_TICS
                };
            } else {
                sector.light = light.max;
                light.bright = true;
                light.tics += if light.random {
                    f32::from(gameplay_random_byte(rng) & 64) + 1.0
                } else {
                    STROBE_BRIGHT_TICS
                };
            }
            changed = true;
        }
    }
    changed
}

fn portal_is_walkable(map: &Map, line: [u16; 7], from: u16, to: u16) -> bool {
    if line[2] & 1 != 0 || line[4] == u16::MAX {
        return false;
    }
    let Some(from_sector) = map.sectors.get(from as usize) else {
        return false;
    };
    let Some(to_sector) = map.sectors.get(to as usize) else {
        return false;
    };
    to_sector.floor <= from_sector.floor + ACTOR_STEP_HEIGHT
        && from_sector.ceiling.min(to_sector.ceiling)
            >= from_sector.floor.max(to_sector.floor) + ACTOR_HEIGHT
}

fn line_has_walkable_opening(map: &Map, line: [u16; 7]) -> bool {
    if line[2] & 1 != 0 {
        return false;
    }
    let (Some(side0), Some(side1)) = (
        map.sides.get(line[3] as usize),
        map.sides.get(line[4] as usize),
    ) else {
        return false;
    };
    portal_is_walkable(map, line, side0.sector, side1.sector)
        || portal_is_walkable(map, line, side1.sector, side0.sector)
}

fn sector_routes(map: &Map) -> Vec<Vec<(u16, usize)>> {
    let mut routes = vec![Vec::new(); map.sectors.len()];
    for (line_index, &line) in map.lines.iter().enumerate() {
        let Some(side0) = map.sides.get(line[3] as usize) else {
            continue;
        };
        let Some(side1) = map.sides.get(line[4] as usize) else {
            continue;
        };
        if portal_is_walkable(map, line, side0.sector, side1.sector) {
            routes[side0.sector as usize].push((side1.sector, line_index));
        }
        if portal_is_walkable(map, line, side1.sector, side0.sector) {
            routes[side1.sector as usize].push((side0.sector, line_index));
        }
    }
    routes
}

fn first_route_portal(routes: &[Vec<(u16, usize)>], start: u16, goal: u16) -> Option<(usize, u16)> {
    if start == goal || routes.get(start as usize).is_none() || routes.get(goal as usize).is_none()
    {
        return None;
    }

    // ponytail: unweighted sector hops; use portal-distance costs if routes look unnatural.
    let mut visited = vec![false; routes.len()];
    let mut parent = vec![None; routes.len()];
    let mut pending = vec![start];
    visited[start as usize] = true;
    let mut head = 0;
    while head < pending.len() && !visited[goal as usize] {
        let current = pending[head];
        head += 1;
        for &(next, line_index) in &routes[current as usize] {
            if !visited[next as usize] {
                visited[next as usize] = true;
                parent[next as usize] = Some((current, line_index));
                pending.push(next);
            }
        }
    }
    if !visited[goal as usize] {
        return None;
    }

    let mut sector = goal;
    while let Some((previous, line)) = parent[sector as usize] {
        if previous == start {
            return Some((line, sector));
        }
        sector = previous;
    }
    None
}

fn chase_waypoint(
    map: &Map,
    routes: &[Vec<(u16, usize)>],
    from: Vertex2,
    goal: Vertex2,
) -> Option<Vertex2> {
    let start = bsp_sector_index_at(map, from.x, from.y)?;
    let goal_sector = bsp_sector_index_at(map, goal.x, goal.y)?;
    let (line_index, next_sector) = first_route_portal(routes, start, goal_sector)?;
    let line = *map.lines.get(line_index)?;
    let a = *map.vertices.get(line[0] as usize)?;
    let b = *map.vertices.get(line[1] as usize)?;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length = (dx * dx + dy * dy).sqrt();
    if length == 0.0 {
        return None;
    }
    // ponytail: five samples keep waypoint search cheap; increase density if maps strand actors.
    [0.5, 0.25, 0.75, 0.125, 0.875]
        .into_iter()
        .flat_map(|along| {
            let midpoint = Vertex2 {
                x: a.x + dx * along,
                y: a.y + dy * along,
            };
            [ACTOR_RADIUS + 8.0, -ACTOR_RADIUS - 8.0]
                .into_iter()
                .map(move |offset| (midpoint, offset))
        })
        .find_map(|(midpoint, offset)| {
            let point = Vertex2 {
                x: midpoint.x - dy / length * offset,
                y: midpoint.y + dx / length * offset,
            };
            (bsp_sector_index_at(map, point.x, point.y) == Some(next_sector)
                && actor_path_clear(map, from, point))
            .then_some(point)
        })
}

fn floor_at(map: &Map, x: f32, y: f32) -> f32 {
    bsp_sector_at(map, x, y).map_or(0.0, |sector| sector.floor)
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

fn segment_distance_squared(a: Vertex2, b: Vertex2, c: Vertex2, d: Vertex2) -> f32 {
    let direction = Vertex2 {
        x: b.x - a.x,
        y: b.y - a.y,
    };
    if ray_segment_distance(a, direction, c, d).is_some_and(|distance| distance <= 1.0) {
        0.0
    } else {
        distance_to_segment_squared(a, c, d)
            .min(distance_to_segment_squared(b, c, d))
            .min(distance_to_segment_squared(c, a, b))
            .min(distance_to_segment_squared(d, a, b))
    }
}

fn actor_path_clear(map: &Map, from: Vertex2, to: Vertex2) -> bool {
    map.lines.iter().all(|line| {
        if line_has_walkable_opening(map, *line) {
            return true;
        }
        let a = map.vertices[line[0] as usize];
        let b = map.vertices[line[1] as usize];
        segment_distance_squared(from, to, a, b) >= ACTOR_RADIUS * ACTOR_RADIUS
    })
}

fn can_occupy(map: &Map, player: Player, from: Sector) -> bool {
    let Some(sector) = bsp_sector_at(map, player.x, player.y) else {
        return false;
    };
    if sector.floor > from.floor + ACTOR_STEP_HEIGHT || sector.ceiling < sector.floor + ACTOR_HEIGHT
    {
        return false;
    }
    let point = Vertex2 {
        x: player.x,
        y: player.y,
    };
    actor_path_clear(map, point, point)
}

fn crossed_line(map: &Map, from: Vertex2, to: Vertex2, line: [u16; 7]) -> bool {
    let a = map.vertices[line[0] as usize];
    let b = map.vertices[line[1] as usize];
    let side = |point: Vertex2| (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x);
    let from_side = side(from);
    let to_side = side(to);
    if !((from_side < 0.0 && to_side >= 0.0) || (from_side > 0.0 && to_side <= 0.0)) {
        return false;
    }
    ray_segment_distance(
        from,
        Vertex2 {
            x: to.x - from.x,
            y: to.y - from.y,
        },
        a,
        b,
    )
    .is_some_and(|distance| distance <= 1.0)
}

fn move_player(map: &Map, player: &mut Player, controls: Controls, delta: f32) -> Vec<usize> {
    let mut crossed = Vec::new();
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
            let origin = Vertex2 {
                x: player.x,
                y: player.y,
            };
            let destination = Vertex2 {
                x: candidate.x,
                y: candidate.y,
            };
            crossed.extend(map.lines.iter().enumerate().filter_map(|(index, &line)| {
                (matches!(line[5], LINE_WALK_OPEN_DOOR | LINE_PLAT_DOWN_WAIT_UP)
                    && crossed_line(map, origin, destination, line))
                .then_some(index)
            }));
            *player = candidate;
        }
    }
    crossed
}

fn use_line(map: &Map, player: Player) -> Option<(usize, u16)> {
    let origin = Vertex2 {
        x: player.x,
        y: player.y,
    };
    let angle = player.angle.to_radians();
    let direction = Vertex2 {
        x: angle.cos(),
        y: angle.sin(),
    };
    let mut intersections = map
        .lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let distance = ray_segment_distance(
                origin,
                direction,
                map.vertices[line[0] as usize],
                map.vertices[line[1] as usize],
            )?;
            (distance <= USE_RANGE).then_some((distance, index))
        })
        .collect::<Vec<_>>();
    intersections.sort_by(|a, b| a.0.total_cmp(&b.0));

    for (_, index) in intersections {
        let line = map.lines[index];
        if line[5] != 0 {
            let a = map.vertices[line[0] as usize];
            let b = map.vertices[line[1] as usize];
            let side = (b.x - a.x) * (player.y - a.y) - (b.y - a.y) * (player.x - a.x);
            return (side < 0.0).then_some((index, line[5]));
        }
        if line[4] == u16::MAX {
            return None;
        }
        let (Some(side0), Some(side1)) = (
            map.sides.get(line[3] as usize),
            map.sides.get(line[4] as usize),
        ) else {
            return None;
        };
        let (Some(sector0), Some(sector1)) = (
            map.sectors.get(side0.sector as usize),
            map.sectors.get(side1.sector as usize),
        ) else {
            return None;
        };
        if sector0.ceiling.min(sector1.ceiling) <= sector0.floor.max(sector1.floor) {
            return None;
        }
    }
    None
}

fn manual_door(map: &Map, line_index: usize, blue_key: bool) -> Option<Door> {
    let line = *map.lines.get(line_index)?;
    if !matches!(
        line[5],
        LINE_DOOR_RAISE | LINE_BLUE_LOCKED_DOOR | LINE_BLAZING_DOOR_RAISE
    ) || (line[5] == LINE_BLUE_LOCKED_DOOR && !blue_key)
        || line[4] == u16::MAX
    {
        return None;
    }
    let mut door = sector_door(map, map.sides.get(line[4] as usize)?.sector, true)?;
    if line[5] == LINE_BLAZING_DOOR_RAISE {
        door.speed = BLAZING_DOOR_SPEED;
    }
    Some(door)
}

fn sector_door(map: &Map, door_sector: u16, auto_close: bool) -> Option<Door> {
    let sector = map.sectors.get(door_sector as usize)?;
    let top = map
        .lines
        .iter()
        .filter_map(|adjacent| {
            let side0 = map.sides.get(adjacent[3] as usize)?;
            let side1 = map.sides.get(adjacent[4] as usize)?;
            if side0.sector == door_sector {
                Some(map.sectors.get(side1.sector as usize)?.ceiling)
            } else if side1.sector == door_sector {
                Some(map.sectors.get(side0.sector as usize)?.ceiling)
            } else {
                None
            }
        })
        .min_by(f32::total_cmp)?
        - 4.0;
    (top >= sector.floor + ACTOR_HEIGHT && sector.ceiling < top).then_some(Door {
        sector: door_sector,
        top,
        speed: DOOR_SPEED,
        wait: DOOR_WAIT,
        direction: 1,
        auto_close,
    })
}

fn walk_open_doors(map: &mut Map, line_index: usize, active: &[Door]) -> Vec<Door> {
    let Some(line) = map.lines.get(line_index).copied() else {
        return Vec::new();
    };
    if line[5] != LINE_WALK_OPEN_DOOR {
        return Vec::new();
    }
    map.lines[line_index][5] = 0;
    if line[6] == 0 {
        return Vec::new();
    }
    map.sectors
        .iter()
        .enumerate()
        .filter(|(_, sector)| sector.tag == line[6])
        .filter_map(|(index, _)| {
            let sector = u16::try_from(index).ok()?;
            if active.iter().any(|door| door.sector == sector) {
                None
            } else {
                sector_door(map, sector, false)
            }
        })
        .collect()
}

fn sector_platform(map: &Map, platform_sector: u16) -> Option<Platform> {
    let sector = map.sectors.get(platform_sector as usize)?;
    let low = map
        .lines
        .iter()
        .filter_map(|adjacent| {
            let side0 = map.sides.get(adjacent[3] as usize)?;
            let side1 = map.sides.get(adjacent[4] as usize)?;
            if side0.sector == platform_sector {
                Some(map.sectors.get(side1.sector as usize)?.floor)
            } else if side1.sector == platform_sector {
                Some(map.sectors.get(side0.sector as usize)?.floor)
            } else {
                None
            }
        })
        .min_by(f32::total_cmp)?
        .min(sector.floor);
    (low < sector.floor).then_some(Platform {
        sector: platform_sector,
        low,
        high: sector.floor,
        speed: PLATFORM_SPEED,
        wait: PLATFORM_WAIT,
        direction: -1,
        return_to_high: true,
    })
}

fn lower_to_lowest_floors(map: &mut Map, line_index: usize, active: &[Platform]) -> Vec<Platform> {
    let Some(line) = map.lines.get(line_index).copied() else {
        return Vec::new();
    };
    if line[5] != LINE_USE_LOWER_FLOOR_TO_LOWEST || line[6] == 0 {
        return Vec::new();
    }
    let started = map
        .sectors
        .iter()
        .enumerate()
        .filter(|(_, sector)| sector.tag == line[6])
        .filter_map(|(index, _)| {
            let sector = u16::try_from(index).ok()?;
            if active.iter().any(|platform| platform.sector == sector) {
                return None;
            }
            let mut floor = sector_platform(map, sector)?;
            floor.speed = FLOOR_SPEED;
            floor.wait = 0.0;
            floor.return_to_high = false;
            Some(floor)
        })
        .collect::<Vec<_>>();
    if !started.is_empty() {
        map.lines[line_index][5] = 0;
    }
    started
}

fn down_wait_up_platforms(map: &Map, line_index: usize, active: &[Platform]) -> Vec<Platform> {
    let Some(line) = map.lines.get(line_index) else {
        return Vec::new();
    };
    if !matches!(
        line[5],
        LINE_USE_DOWN_WAIT_UP_PLATFORM | LINE_PLAT_DOWN_WAIT_UP
    ) || line[6] == 0
    {
        return Vec::new();
    }
    map.sectors
        .iter()
        .enumerate()
        .filter(|(_, sector)| sector.tag == line[6])
        .filter_map(|(index, _)| {
            let sector = u16::try_from(index).ok()?;
            if active.iter().any(|platform| platform.sector == sector) {
                None
            } else {
                sector_platform(map, sector)
            }
        })
        .collect()
}

fn update_platforms(map: &mut Map, platforms: &mut Vec<Platform>, delta: f32) -> bool {
    let mut changed = false;
    let mut index = 0;
    while index < platforms.len() {
        if platforms[index].direction == 0 {
            platforms[index].wait -= delta;
            if platforms[index].wait <= 0.0 {
                platforms[index].direction = 1;
            }
            index += 1;
            continue;
        }
        let platform = &mut platforms[index];
        let speed = platform.speed;
        let sector = &mut map.sectors[platform.sector as usize];
        let direction = platform.direction;
        let target = if direction < 0 {
            platform.low
        } else {
            platform.high
        };
        let previous = sector.floor;
        let floor = if direction < 0 {
            (previous - speed * delta).max(target)
        } else {
            (previous + speed * delta).min(target)
        };
        if direction > 0 && floor + ACTOR_HEIGHT > sector.ceiling {
            platform.direction = -1;
            index += 1;
            continue;
        }
        sector.floor = floor;
        changed |= floor != previous;
        if direction < 0 && floor <= target {
            if platform.return_to_high {
                platform.direction = 0;
                platform.wait = PLATFORM_WAIT;
            } else {
                platforms.remove(index);
                continue;
            }
        } else if direction > 0 && floor >= target {
            platforms.remove(index);
            continue;
        }
        index += 1;
    }
    changed
}

fn update_doors(
    map: &mut Map,
    doors: &mut Vec<Door>,
    player: Player,
    actors: &[Actor],
    delta: f32,
) -> bool {
    let mut changed = false;
    let mut index = 0;
    while index < doors.len() {
        if doors[index].direction == 0 {
            doors[index].wait -= delta;
            if doors[index].wait <= 0.0 {
                doors[index].direction = -1;
            }
        }
        let sector_index = doors[index].sector;
        if doors[index].direction < 0
            && (bsp_sector_index_at(map, player.x, player.y) == Some(sector_index)
                || actors.iter().any(|actor| {
                    actor.health > 0
                        && bsp_sector_index_at(map, actor.x, actor.y) == Some(sector_index)
                }))
        {
            doors[index].direction = 1;
        }
        let target = if doors[index].direction < 0 {
            map.sectors[sector_index as usize].floor
        } else {
            doors[index].top
        };
        let direction = doors[index].direction;
        let top = doors[index].top;
        let sector = &mut map.sectors[sector_index as usize];
        let previous = sector.ceiling;
        sector.ceiling = if direction < 0 {
            (previous - doors[index].speed * delta).max(target)
        } else {
            (previous + doors[index].speed * delta).min(target)
        };
        changed |= sector.ceiling != previous;
        if direction > 0 && sector.ceiling >= top {
            if doors[index].auto_close {
                doors[index].direction = 0;
                doors[index].wait = DOOR_WAIT;
            } else {
                doors.remove(index);
                continue;
            }
        } else if direction < 0 && sector.ceiling <= sector.floor {
            doors.remove(index);
            continue;
        }
        index += 1;
    }
    changed
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

fn nearest_blocking_wall(map: &Map, origin: Vertex2, direction: Vertex2) -> f32 {
    map.lines
        .iter()
        .filter(|line| !line_has_walkable_opening(map, **line))
        .filter_map(|line| {
            ray_segment_distance(
                origin,
                direction,
                map.vertices[line[0] as usize],
                map.vertices[line[1] as usize],
            )
        })
        .fold(f32::INFINITY, f32::min)
}

fn has_line_of_sight(map: &Map, from: Vertex2, to: Vertex2) -> bool {
    let direction = Vertex2 {
        x: to.x - from.x,
        y: to.y - from.y,
    };
    let distance = (direction.x * direction.x + direction.y * direction.y).sqrt();
    if distance <= 0.001 {
        return true;
    }
    let direction = Vertex2 {
        x: direction.x / distance,
        y: direction.y / distance,
    };
    nearest_blocking_wall(map, from, direction) >= distance
}

fn sound_reachable_sectors(map: &Map, source: u16) -> Vec<bool> {
    let mut neighbors = vec![Vec::new(); map.sectors.len()];
    for &line in &map.lines {
        if line[2] & LINE_TWO_SIDED == 0 || line[4] == u16::MAX {
            continue;
        }
        let (Some(side0), Some(side1)) = (
            map.sides.get(line[3] as usize),
            map.sides.get(line[4] as usize),
        ) else {
            continue;
        };
        let (Some(sector0), Some(sector1)) = (
            map.sectors.get(side0.sector as usize),
            map.sectors.get(side1.sector as usize),
        ) else {
            continue;
        };
        if sector0.ceiling.min(sector1.ceiling) <= sector0.floor.max(sector1.floor) {
            continue;
        }
        let sound_blocking = line[2] & LINE_SOUND_BLOCK != 0;
        neighbors[side0.sector as usize].push((side1.sector, sound_blocking));
        neighbors[side1.sector as usize].push((side0.sector, sound_blocking));
    }

    let mut sound_blocks = vec![u8::MAX; map.sectors.len()];
    let Some(source_blocks) = sound_blocks.get_mut(source as usize) else {
        return vec![false; map.sectors.len()];
    };
    *source_blocks = 0;
    let mut pending = vec![source];
    let mut head = 0;
    while head < pending.len() {
        let current = pending[head];
        head += 1;
        for &(next, sound_blocking) in &neighbors[current as usize] {
            let blocks = sound_blocks[current as usize] + u8::from(sound_blocking);
            if blocks <= 1 && blocks < sound_blocks[next as usize] {
                sound_blocks[next as usize] = blocks;
                pending.push(next);
            }
        }
    }
    sound_blocks
        .into_iter()
        .map(|blocks| blocks != u8::MAX)
        .collect()
}

fn alert_actors_on_noise(map: &Map, actors: &mut [Actor], source: Vertex2) {
    let Some(source_sector) = bsp_sector_index_at(map, source.x, source.y) else {
        return;
    };
    let audible = sound_reachable_sectors(map, source_sector);
    for actor in actors.iter_mut().filter(|actor| actor.health > 0) {
        let Some(sector) = bsp_sector_index_at(map, actor.x, actor.y) else {
            continue;
        };
        if audible[sector as usize] {
            if actor.target_time_remaining == 0.0 {
                actor.animation_time = 0.0;
            }
            actor.target_time_remaining = ACTOR_TARGET_THRESHOLD;
        }
    }
}

fn fire_weapon(map: &Map, actors: &mut [Actor], player: Player, pain_rng: &mut u32) -> bool {
    let origin = Vertex2 {
        x: player.x,
        y: player.y,
    };
    alert_actors_on_noise(map, actors, origin);
    let radians = player.angle.to_radians();
    let direction = Vertex2 {
        x: radians.cos(),
        y: radians.sin(),
    };
    let nearest_wall = nearest_blocking_wall(map, origin, direction);
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
        if actors[index].health <= 0 {
            actors[index].pain_animation_remaining = 0.0;
            actors[index].death_animation_time = Some(0.0);
            true
        } else {
            actors[index].target_time_remaining = ACTOR_TARGET_THRESHOLD;
            if actor_pain_triggered(actors[index].sprite, gameplay_random_byte(pain_rng)) {
                actors[index].pain_animation_remaining = actor_pain_duration(actors[index].sprite);
            }
            false
        }
    } else {
        false
    }
}

fn update_actors(
    map: &Map,
    actors: &mut [Actor],
    projectiles: &mut Vec<Projectile>,
    player: Player,
    health: &mut i32,
    delta: f32,
) {
    let routes = sector_routes(map);
    for actor in actors.iter_mut() {
        if actor.health <= 0 {
            if let Some(time) = actor.death_animation_time {
                actor.death_animation_time =
                    Some((time + delta).min(actor_death_duration(actor.sprite)));
            }
            continue;
        }
        let was_awake = actor.target_time_remaining > 0.0;
        actor.target_time_remaining = (actor.target_time_remaining - delta).max(0.0);
        actor.attack_animation_remaining = (actor.attack_animation_remaining - delta).max(0.0);
        actor.pain_animation_remaining = (actor.pain_animation_remaining - delta).max(0.0);
        if actor.pain_animation_remaining > 0.0 {
            continue;
        }
        if *health <= 0 {
            continue;
        }
        let previous_position = (actor.x, actor.y);
        actor.attack_cooldown = (actor.attack_cooldown - delta).max(0.0);
        let dx = player.x - actor.x;
        let dy = player.y - actor.y;
        let distance = (dx * dx + dy * dy).sqrt();
        let can_see_player = distance <= ACTOR_WAKE_RANGE
            && has_line_of_sight(
                map,
                Vertex2 {
                    x: actor.x,
                    y: actor.y,
                },
                Vertex2 {
                    x: player.x,
                    y: player.y,
                },
            );
        if can_see_player {
            actor.target_time_remaining = ACTOR_TARGET_THRESHOLD;
        }
        let awake = actor.target_time_remaining > 0.0;
        if !was_awake && awake {
            actor.animation_time = 0.0;
        }
        if awake {
            actor.angle = dy.atan2(dx).to_degrees().rem_euclid(360.0);
        }
        if distance < 48.0 && can_see_player {
            if actor.attack_cooldown == 0.0 {
                *health -= 8;
                actor.attack_cooldown = 0.85;
                actor.attack_animation_remaining = actor_attack_duration(actor.sprite);
            }
        } else if (actor.sprite == *b"POSS" || actor.sprite == *b"SPOS")
            && distance <= 512.0
            && can_see_player
        {
            if actor.attack_cooldown == 0.0 {
                *health -= if actor.sprite == *b"SPOS" { 6 } else { 3 };
                actor.attack_cooldown = 1.4;
                actor.attack_animation_remaining = actor_attack_duration(actor.sprite);
            }
        } else if actor.sprite == *b"TROO" && distance <= 512.0 && can_see_player {
            if actor.attack_cooldown == 0.0 {
                let z = floor_at(map, actor.x, actor.y) + ACTOR_HEIGHT * 0.5;
                let dz = floor_at(map, player.x, player.y) + ACTOR_HEIGHT * 0.5 - z;
                let speed = 180.0 / (distance * distance + dz * dz).sqrt().max(1.0);
                projectiles.push(Projectile {
                    x: actor.x,
                    y: actor.y,
                    z,
                    velocity_x: dx * speed,
                    velocity_y: dy * speed,
                    velocity_z: dz * speed,
                    lifetime: 3.0,
                    explosion_time: None,
                });
                actor.attack_cooldown = 2.0;
                actor.attack_animation_remaining = actor_attack_duration(actor.sprite);
            }
        } else if awake {
            let player_position = Vertex2 {
                x: player.x,
                y: player.y,
            };
            let target = chase_waypoint(
                map,
                &routes,
                Vertex2 {
                    x: actor.x,
                    y: actor.y,
                },
                player_position,
            )
            .unwrap_or(player_position);
            let mut enemy = Player {
                x: actor.x,
                y: actor.y,
                angle: (target.y - actor.y).atan2(target.x - actor.x).to_degrees(),
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
        if (actor.x, actor.y) != previous_position {
            let cycle_seconds = actor_walk_frame_tics(actor.sprite) * 4.0 / 35.0;
            actor.animation_time = (actor.animation_time + delta) % cycle_seconds;
        } else if awake {
            actor.animation_time = 0.0;
        } else {
            actor.animation_time = (actor.animation_time + delta) % ACTOR_IDLE_CYCLE;
        }
    }
}

fn update_projectiles(
    map: &Map,
    projectiles: &mut Vec<Projectile>,
    player: Player,
    health: &mut i32,
    delta: f32,
) {
    projectiles.retain_mut(|projectile| {
        if let Some(time) = &mut projectile.explosion_time {
            *time += delta;
            return *time < projectile_explosion_duration();
        }
        projectile.lifetime -= delta;
        if projectile.lifetime <= 0.0 {
            return false;
        }
        let from = Vertex2 {
            x: projectile.x,
            y: projectile.y,
        };
        let to = Vertex2 {
            x: from.x + projectile.velocity_x * delta,
            y: from.y + projectile.velocity_y * delta,
        };
        let next_z = projectile.z + projectile.velocity_z * delta;
        let step_x = to.x - from.x;
        let step_y = to.y - from.y;
        let step_distance = (step_x * step_x + step_y * step_y).sqrt();
        if !has_line_of_sight(map, from, to) {
            let direction = Vertex2 {
                x: step_x / step_distance,
                y: step_y / step_distance,
            };
            let travel = nearest_blocking_wall(map, from, direction).min(step_distance);
            let fraction = if step_distance > 0.0 {
                (travel / step_distance).clamp(0.0, 1.0)
            } else {
                0.0
            };
            projectile.x = from.x + direction.x * travel;
            projectile.y = from.y + direction.y * travel;
            projectile.z += (next_z - projectile.z) * fraction;
            projectile.explosion_time = Some(0.0);
            return true;
        }
        projectile.x = to.x;
        projectile.y = to.y;
        projectile.z = next_z;
        let dx = player.x - to.x;
        let dy = player.y - to.y;
        let player_floor = floor_at(map, player.x, player.y);
        if dx * dx + dy * dy <= 16.0 * 16.0
            && (player_floor..=player_floor + ACTOR_HEIGHT).contains(&projectile.z)
        {
            if *health > 0 {
                *health -= 8;
            }
            projectile.explosion_time = Some(0.0);
        }
        true
    });
}

impl PreparedScene {
    fn load(path: &Path, map_name: &str) -> api::Result<Self> {
        if fs::metadata(path)?.len() > MAX_WAD_BYTES {
            return Err(invalid("WAD exceeds the 128 MiB sample limit").into());
        }
        let data = fs::read(path)?;
        let map = parse_map(&data, map_name)?;
        let sky_name = sky_texture_name(&map, map_name);
        let (x, y, angle, _, _) = map
            .things
            .iter()
            .copied()
            .find(|thing| thing.3 == 1)
            .ok_or_else(|| invalid(format!("{map_name} has no player-1 start")))?;
        let wall_textures = wall_textures(&data, &map, sky_name)?;
        let sky_texture = sky_name.and_then(|name| wall_textures.get(&name).cloned());
        let flat_textures = flat_textures(&data, &map)?;
        let geometry = geometry(&map, &wall_textures)?;
        if geometry
            .iter()
            .all(|leaf| leaf.walls.is_empty() && leaf.flats.is_empty())
        {
            return Err(invalid(format!("{map_name} produced no renderable geometry")).into());
        }
        let device = Device::new();
        let vertex_shader =
            device.create_shader(include_bytes!("../assets/shaders/textured.vert.spv"))?;
        let fragment_shader =
            device.create_shader(include_bytes!("../assets/shaders/freedoom_map.frag.spv"))?;
        let pipeline =
            device.create_pipeline(&vertex_shader, &fragment_shader, Pipeline::default())?;
        let sky_pipeline = if sky_texture.is_some() {
            Some(device.create_pipeline(
                &vertex_shader,
                &fragment_shader,
                Pipeline {
                    depth_compare: api::Compare::LessEqual,
                    depth_write: false,
                    ..Pipeline::default()
                },
            )?)
        } else {
            None
        };
        let sprite_fragment =
            device.create_shader(include_bytes!("../assets/shaders/freedoom_sprite.frag.spv"))?;
        let sprite_pipeline =
            device.create_pipeline(&vertex_shader, &sprite_fragment, Pipeline::default())?;
        let weapon_pipeline = device.create_pipeline(
            &vertex_shader,
            &sprite_fragment,
            Pipeline {
                depth_compare: api::Compare::Always,
                depth_write: false,
                ..Pipeline::default()
            },
        )?;
        let weapon_idle = sprite_patch_texture(&data, *b"PISGA0\0\0")?;
        let weapon_fire = sprite_patch_texture(&data, *b"PISGC0\0\0")?;
        let projectile_sprite = sprite_patch_texture(&data, *b"BAL1A0\0\0")?;
        let projectile_explosion = [
            sprite_patch_texture(&data, *b"BAL1C0\0\0")?,
            sprite_patch_texture(&data, *b"BAL1D0\0\0")?,
            sprite_patch_texture(&data, *b"BAL1E0\0\0")?,
        ];
        let sampler = Sampler {
            filter: Filter::Nearest,
            address: Address::Repeat,
            mip: MipFilter::None,
        };
        let mut sprites = BTreeMap::new();
        let mut pickup_sprites = BTreeMap::new();
        let mut actors = Vec::new();
        let mut pickups = Vec::new();
        for &(x, y, thing_angle, kind, flags) in &map.things {
            if flags & 2 == 0 || flags & 16 != 0 {
                continue;
            }
            if bsp_sector_at(&map, x as f32, y as f32).is_none() {
                continue;
            }
            if let Some((prefix, health)) = monster_sprite(kind) {
                if let Entry::Vacant(entry) = sprites.entry(prefix) {
                    entry.insert(sprite_actor_textures(&data, prefix)?);
                }
                actors.push(Actor {
                    sprite: prefix,
                    x: x as f32,
                    y: y as f32,
                    health,
                    target_time_remaining: 0.0,
                    attack_cooldown: 0.0,
                    attack_animation_remaining: 0.0,
                    pain_animation_remaining: 0.0,
                    death_animation_time: None,
                    animation_time: 0.0,
                    angle: thing_angle as f32,
                });
            } else if let Some((prefix, health, ammo, blue_key)) = pickup_definition(kind) {
                if let Entry::Vacant(entry) = pickup_sprites.entry(prefix) {
                    let mut name = [0; 8];
                    name[..4].copy_from_slice(&prefix);
                    name[4..6].copy_from_slice(b"A0");
                    entry.insert(sprite_patch_texture(&data, name)?);
                }
                pickups.push(Pickup {
                    sprite: prefix,
                    x: x as f32,
                    y: y as f32,
                    health,
                    ammo,
                    blue_key,
                    active: true,
                });
            }
        }
        let draws = build_draws(geometry, &flat_textures, &wall_textures)?;
        let visibility_fallbacks = visibility_fallback_bounds(&map, &draws);
        Ok(Self {
            map_name: map_name.to_owned(),
            map,
            flat_textures,
            wall_textures,
            sky_texture,
            device,
            pipeline,
            sky_pipeline,
            sprite_pipeline,
            weapon_pipeline,
            sampler,
            draws,
            visibility_fallbacks,
            sprites,
            pickup_sprites,
            weapon_idle,
            weapon_fire,
            projectile_sprite,
            projectile_explosion,
            actors,
            pickups,
            start: Player {
                x: x as f32,
                y: y as f32,
                angle: angle as f32,
            },
        })
    }

    fn rebuild_draws(&mut self) -> api::Result<()> {
        self.draws = build_draws(
            geometry(&self.map, &self.wall_textures)?,
            &self.flat_textures,
            &self.wall_textures,
        )?;
        Ok(())
    }

    fn draw(
        &self,
        player: Player,
        actors: &[Actor],
        projectiles: &[Projectile],
        pickups: &[Pickup],
        weapon_firing: bool,
        renderer: &mut Renderer,
    ) -> api::Result<(api::Submission, usize)> {
        let sector = bsp_sector_at(&self.map, player.x, player.y).ok_or_else(|| {
            invalid(format!(
                "player is outside every {} BSP leaf",
                self.map_name
            ))
        })?;
        let visible_order = visible_geometry_order(&self.map, player, &self.visibility_fallbacks);
        let mut visible = vec![false; self.map.subsectors.len()];
        for &leaf in &visible_order {
            visible[leaf] = true;
        }
        let eye = Vec3::new(player.x, sector.floor + 41.0, -player.y);
        let eye_height = sector.floor + 41.0;
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
        let mut batches = BTreeMap::new();
        for (leaf, draws) in self.draws.iter().enumerate() {
            if !visible[leaf] {
                continue;
            }
            for draw in draws {
                if !draw
                    .bounds
                    .is_some_and(|bounds| bounds3_in_view(bounds, player, eye_height))
                {
                    continue;
                }
                let depth = draw
                    .bounds
                    .map(|bounds| depth_bucket(bounds, player))
                    .unwrap_or_default();
                let batch = batches
                    .entry((depth, draw.wall, draw.masked, draw.name))
                    .or_insert_with(|| (Arc::clone(&draw.texture), Vec::new()));
                batch.1.extend_from_slice(&draw.vertices);
            }
        }
        let static_draws = batches.len();
        for ((_, _, masked, _), (texture, vertices)) in batches {
            commands.bind_pipeline(if masked {
                self.sprite_pipeline.clone()
            } else {
                self.pipeline.clone()
            });
            let count = u32::try_from(vertices.len()).map_err(|_| {
                invalid(format!(
                    "visible {} geometry exceeds SILICON draw range",
                    self.map_name
                ))
            })?;
            commands.bind_texture(0, texture, self.sampler);
            commands.bind_vertex_buffer(self.device.create_vertex_buffer(vertices)?);
            commands.draw(0, count);
        }
        if let (Some(texture), Some(pipeline)) = (&self.sky_texture, &self.sky_pipeline) {
            let vertices = sky_vertices(
                player,
                eye_height,
                (texture.levels[0].width, texture.levels[0].height),
                (960, 720),
            );
            let count = u32::try_from(vertices.len())
                .map_err(|_| invalid("sky vertex count exceeds SILICON draw range"))?;
            if count > 0 {
                commands.bind_pipeline(pipeline.clone());
                commands.bind_texture(0, Arc::clone(texture), self.sampler);
                commands.bind_vertex_buffer(self.device.create_vertex_buffer(vertices)?);
                commands.draw(0, count);
            }
        }
        commands.bind_pipeline(self.sprite_pipeline.clone());
        commands.bind_uniform_buffer(uniform_buffer.clone());
        for actor in actors
            .iter()
            .filter(|actor| actor.health > 0 || actor.death_animation_time.is_some())
        {
            let Some(sector) = bsp_sector_at(&self.map, actor.x, actor.y) else {
                continue;
            };
            let Some(frames) = self.sprites.get(&actor.sprite) else {
                continue;
            };
            let view_to_actor = (actor.y - player.y).atan2(actor.x - player.x).to_degrees();
            let frame = if actor.health <= 0 {
                actor_death_frame(actor.sprite, actor.death_animation_time.unwrap_or_default())
                    .unwrap_or(0)
            } else {
                actor_pain_frame(actor.sprite, actor.pain_animation_remaining)
                    .or_else(|| actor_attack_frame(actor.sprite, actor.attack_animation_remaining))
                    .unwrap_or_else(|| {
                        if actor.target_time_remaining > 0.0 {
                            actor_walk_frame(actor.sprite, actor.animation_time)
                        } else {
                            actor_idle_frame(actor.animation_time)
                        }
                    })
            };
            let sprite = &frames[frame][actor_view_rotation(actor.angle, view_to_actor)];
            let vertices =
                sprite_vertices(actor.x, actor.y, sprite, player.angle, sector, sector.floor);
            if vertices.is_empty() {
                continue;
            }
            let count = u32::try_from(vertices.len()).map_err(|_| {
                invalid(format!(
                    "{} sprite vertex count exceeds SILICON draw range",
                    self.map_name
                ))
            })?;
            commands.bind_texture(0, sprite.texture.clone(), self.sampler);
            commands.bind_vertex_buffer(self.device.create_vertex_buffer(vertices)?);
            commands.draw(0, count);
        }
        for pickup in pickups.iter().filter(|pickup| pickup.active) {
            let Some(sector) = bsp_sector_at(&self.map, pickup.x, pickup.y) else {
                continue;
            };
            let Some(sprite) = self.pickup_sprites.get(&pickup.sprite) else {
                continue;
            };
            let vertices = sprite_vertices(
                pickup.x,
                pickup.y,
                sprite,
                player.angle,
                sector,
                sector.floor,
            );
            let count = u32::try_from(vertices.len()).map_err(|_| {
                invalid(format!(
                    "{} pickup vertex count exceeds SILICON draw range",
                    self.map_name
                ))
            })?;
            commands.bind_texture(0, sprite.texture.clone(), self.sampler);
            commands.bind_vertex_buffer(self.device.create_vertex_buffer(vertices)?);
            commands.draw(0, count);
        }
        for projectile in projectiles {
            let Some(sector) = bsp_sector_at(&self.map, projectile.x, projectile.y) else {
                continue;
            };
            let sprite = if let Some(time) = projectile.explosion_time {
                let Some(frame) = projectile_explosion_frame(time) else {
                    continue;
                };
                &self.projectile_explosion[frame]
            } else {
                &self.projectile_sprite
            };
            let vertices = projectile_vertices(projectile, sprite, player.angle, sector);
            let count = u32::try_from(vertices.len()).map_err(|_| {
                invalid(format!(
                    "{} projectile vertex count exceeds SILICON draw range",
                    self.map_name
                ))
            })?;
            commands.bind_texture(0, sprite.texture.clone(), self.sampler);
            commands.bind_vertex_buffer(self.device.create_vertex_buffer(vertices)?);
            commands.draw(0, count);
        }
        let weapon = if weapon_firing {
            &self.weapon_fire
        } else {
            &self.weapon_idle
        };
        let vertices = weapon_vertices(player, weapon, sector);
        let count = u32::try_from(vertices.len()).map_err(|_| {
            invalid(format!(
                "{} weapon vertex count exceeds SILICON draw range",
                self.map_name
            ))
        })?;
        commands.bind_pipeline(self.weapon_pipeline.clone());
        commands.bind_uniform_buffer(uniform_buffer);
        commands.bind_texture(0, weapon.texture.clone(), self.sampler);
        commands.bind_vertex_buffer(self.device.create_vertex_buffer(vertices)?);
        commands.draw(0, count);
        commands.end_render_pass();
        Ok((self.device.submit(&commands, renderer)?, static_draws))
    }
}

fn save_frame(renderer: &Renderer, output: &Path) -> api::Result<()> {
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    renderer.framebuffer.save_png(output)
}

fn frame_triangles(
    scene: &PreparedScene,
    player: Player,
    static_draws: usize,
    draws: u64,
) -> api::Result<usize> {
    let draws =
        usize::try_from(draws).map_err(|_| invalid("SILICON draw count exceeds host range"))?;
    let sector = bsp_sector_at(&scene.map, player.x, player.y).ok_or_else(|| {
        invalid(format!(
            "player is outside every {} BSP leaf",
            scene.map_name
        ))
    })?;
    let eye_height = sector.floor + 41.0;
    let visible = visible_geometry_order(&scene.map, player, &scene.visibility_fallbacks);
    let static_triangles = visible
        .iter()
        .flat_map(|&leaf| &scene.draws[leaf])
        .filter(|draw| {
            draw.bounds
                .is_some_and(|bounds| bounds3_in_view(bounds, player, eye_height))
        })
        .map(|draw| draw.vertices.len() / 3)
        .sum::<usize>();
    let sky_triangles = usize::from(scene.sky_texture.is_some()) * SKY_MESH_SEGMENTS * 2;
    let sky_draws = usize::from(scene.sky_texture.is_some());
    Ok(static_triangles + sky_triangles + draws.saturating_sub(static_draws + sky_draws) * 2)
}

fn render(path: &Path, map_name: &str, output: &Path) -> api::Result<()> {
    let scene = PreparedScene::load(path, map_name)?;
    let mut renderer = Renderer::new(960, 720)?;
    let (submission, static_draws) = scene.draw(
        scene.start,
        &scene.actors,
        &[],
        &scene.pickups,
        false,
        &mut renderer,
    )?;
    save_frame(&renderer, output)?;
    let triangles = frame_triangles(&scene, scene.start, static_draws, submission.draws)?;
    let visible =
        visible_geometry_order(&scene.map, scene.start, &scene.visibility_fallbacks).len();
    let exits = scene
        .map
        .lines
        .iter()
        .filter(|line| line[5] == LINE_EXIT_USE)
        .count();
    let secrets = scene
        .map
        .sectors
        .iter()
        .filter(|sector| sector.special == SECTOR_SECRET)
        .count();
    println!(
        "{}: {triangles} triangles, {} SILICON draw(s), {visible}/{} horizontal BSP leaves, {exits} use-exit line(s), {secrets} secret sector(s), player start ({}, {}, {}°)",
        scene.map_name,
        submission.draws,
        scene.map.subsectors.len(),
        scene.start.x,
        scene.start.y,
        scene.start.angle
    );
    Ok(())
}

fn next_normal_map(map_name: &str) -> Option<String> {
    let map = map_name.as_bytes();
    if map.len() != 4
        || map[0] != b'E'
        || !(b'1'..=b'4').contains(&map[1])
        || map[2] != b'M'
        || !(b'1'..=b'7').contains(&map[3])
    {
        return None;
    }
    let episode = map[1] - b'0';
    let level = map[3] - b'0';
    Some(format!("E{episode}M{}", level + 1))
}

fn wad_has_map(path: &Path, map_name: &str) -> Result<bool, io::Error> {
    if fs::metadata(path)?.len() > MAX_WAD_BYTES {
        return Err(invalid("WAD exceeds the 128 MiB sample limit"));
    }
    Ok(wad_lumps(&fs::read(path)?)?
        .iter()
        .any(|lump| lump.name() == map_name))
}

fn run_interactive(path: &Path, map_name: &str, output: &Path) -> api::Result<()> {
    let mut scene = PreparedScene::load(path, map_name)?;
    let mut renderer = Renderer::new(960, 720)?;
    let mut window = Window::new(
        &format!("SILICON | Freedoom {}", scene.map_name),
        960,
        720,
        WindowOptions::default(),
    )?;
    window.set_target_fps(60);
    let mut pixels = vec![0; 960 * 720];
    let mut player = scene.start;
    let mut actors = scene.actors.clone();
    let mut projectiles = Vec::new();
    let mut doors = Vec::new();
    let mut platforms = Vec::new();
    let mut pickups = scene.pickups.clone();
    let mut health = 100;
    let mut ammo = 50;
    let mut blue_key = false;
    let mut light_rng = 0x4c49_4748u32;
    let mut sector_lights = spawn_sector_lights(&mut scene.map, &mut light_rng);
    let mut nukage_damage_tics = 0.0;
    let mut total_secrets = scene
        .map
        .sectors
        .iter()
        .filter(|sector| sector.special == SECTOR_SECRET)
        .count();
    let mut secrets_found = 0;
    let mut session_secrets_found = 0;
    let mut session_secret_total = 0;
    let mut levels_completed = 0;
    let mut kills = 0;
    let mut collected = 0;
    let mut activated_sectors = 0;
    let mut shot_cooldown = 0.0f32;
    let mut weapon_flash = 0.0f32;
    let mut exited = false;
    let mut pain_rng = 0x5349_4c49u32;
    let mut last = std::time::Instant::now();
    let mut frames = 0u64;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        if exited {
            let Some(next_map) = next_normal_map(&scene.map_name) else {
                break;
            };
            if !wad_has_map(path, &next_map)? {
                break;
            }
            session_secrets_found += secrets_found;
            session_secret_total += total_secrets;
            scene = PreparedScene::load(path, &next_map)?;
            player = scene.start;
            actors = scene.actors.clone();
            projectiles.clear();
            doors.clear();
            platforms.clear();
            pickups = scene.pickups.clone();
            sector_lights = spawn_sector_lights(&mut scene.map, &mut light_rng);
            total_secrets = scene
                .map
                .sectors
                .iter()
                .filter(|sector| sector.special == SECTOR_SECRET)
                .count();
            secrets_found = 0;
            nukage_damage_tics = 0.0;
            shot_cooldown = 0.0;
            weapon_flash = 0.0;
            levels_completed += 1;
            exited = false;
        }
        let now = std::time::Instant::now();
        let delta = now.duration_since(last).as_secs_f32().min(0.05);
        last = now;
        let doors_changed = update_doors(&mut scene.map, &mut doors, player, &actors, delta);
        let platforms_changed = update_platforms(&mut scene.map, &mut platforms, delta);
        let lights_changed =
            update_sector_lights(&mut scene.map, &mut sector_lights, &mut light_rng, delta);
        if doors_changed || platforms_changed || lights_changed {
            scene.rebuild_draws()?;
        }
        if health > 0 && !exited {
            let axis = |positive, negative| {
                (window.is_key_down(positive) as i8 - window.is_key_down(negative) as i8) as f32
            };
            let crossed = move_player(
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
            for line in crossed {
                match scene.map.lines[line][5] {
                    LINE_WALK_OPEN_DOOR => {
                        let started = walk_open_doors(&mut scene.map, line, &doors);
                        activated_sectors += started.len();
                        doors.extend(started);
                    }
                    LINE_PLAT_DOWN_WAIT_UP => {
                        let started = down_wait_up_platforms(&scene.map, line, &platforms);
                        activated_sectors += started.len();
                        platforms.extend(started);
                    }
                    _ => {}
                }
            }
            secrets_found += usize::from(discover_secret(&mut scene.map, player));
            if window.is_key_pressed(Key::E, KeyRepeat::No)
                && let Some((line, special)) = use_line(&scene.map, player)
            {
                if special == LINE_EXIT_USE {
                    exited = true;
                } else if matches!(special, LINE_DOOR_RAISE | LINE_BLAZING_DOOR_RAISE)
                    || special == LINE_BLUE_LOCKED_DOOR
                {
                    if let Some(door) = manual_door(&scene.map, line, blue_key)
                        && !doors
                            .iter()
                            .any(|active: &Door| active.sector == door.sector)
                    {
                        doors.push(door);
                        activated_sectors += 1;
                    }
                } else if special == LINE_USE_LOWER_FLOOR_TO_LOWEST {
                    let started = lower_to_lowest_floors(&mut scene.map, line, &platforms);
                    activated_sectors += started.len();
                    platforms.extend(started);
                } else if special == LINE_USE_DOWN_WAIT_UP_PLATFORM {
                    let started = down_wait_up_platforms(&scene.map, line, &platforms);
                    activated_sectors += started.len();
                    platforms.extend(started);
                }
            }
            if !exited {
                collected += collect_pickups(
                    &scene.map,
                    &mut pickups,
                    player,
                    &mut health,
                    &mut ammo,
                    &mut blue_key,
                );
                shot_cooldown = (shot_cooldown - delta).max(0.0);
                weapon_flash = (weapon_flash - delta).max(0.0);
                if window.is_key_pressed(Key::Space, KeyRepeat::No)
                    && shot_cooldown == 0.0
                    && ammo > 0
                {
                    ammo -= 1;
                    shot_cooldown = 0.35;
                    weapon_flash = 0.16;
                    kills +=
                        usize::from(fire_weapon(&scene.map, &mut actors, player, &mut pain_rng));
                }
            }
        }
        if health > 0 && !exited {
            update_floor_damage(
                &scene.map,
                player,
                &mut health,
                &mut nukage_damage_tics,
                delta,
            );
        }
        if !exited {
            update_actors(
                &scene.map,
                &mut actors,
                &mut projectiles,
                player,
                &mut health,
                delta,
            );
            update_projectiles(&scene.map, &mut projectiles, player, &mut health, delta);
        }
        health = health.max(0);
        let (submission, static_draws) = scene.draw(
            player,
            &actors,
            &projectiles,
            &pickups,
            weapon_flash > 0.0,
            &mut renderer,
        )?;
        renderer.framebuffer.present_into(&mut pixels)?;
        let remaining = actors.iter().filter(|actor| actor.health > 0).count();
        let state = if health == 0 {
            "DEAD"
        } else if exited {
            "EXITED"
        } else if remaining == 0 {
            "CLEAR"
        } else {
            "PLAYING"
        };
        let triangles = frame_triangles(&scene, player, static_draws, submission.draws)?;
        let visible = visible_geometry_order(&scene.map, player, &scene.visibility_fallbacks).len();
        window.set_title(&format!(
            "SILICON | {} {state} | WASD move, arrows turn, Shift run, Space fire, E open/use | HP {health} | ammo {ammo} | blue key {blue_key} | secrets {secrets_found}/{total_secrets} | maps {} | items {collected} | kills {kills} | {} triangles, {} draws, {visible}/{} BSP leaves",
            scene.map_name,
            levels_completed + 1,
            triangles,
            submission.draws,
            scene.map.subsectors.len()
        ));
        window.update_with_buffer(&pixels, 960, 720)?;
        frames += 1;
    }
    save_frame(&renderer, output)?;
    println!(
        "{} session: {frames} SILICON-rendered frames, {} map(s), {kills} kills, {collected} pickups, {activated_sectors} sector actions, {} / {} secrets, health {health}, blue key {blue_key}, exited {exited}; saved {}",
        scene.map_name,
        levels_completed + 1,
        session_secrets_found + secrets_found,
        session_secret_total + total_secrets,
        output.display()
    );
    Ok(())
}

fn main() -> api::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let wad = args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: cargo run --release --example freedoom_map -- <freedoom1.wad> [--map E1M2] [output.png | --interactive]",
        )
    })?;
    let mut map_name = String::from("E1M1");
    let mut output = None;
    let mut interactive = false;
    while let Some(argument) = args.next() {
        if argument == "--interactive" {
            interactive = true;
        } else if argument == "--map" {
            let name = args.next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "--map requires a WAD map name")
            })?;
            let name = name
                .to_str()
                .filter(|name| {
                    !name.is_empty()
                        && name.len() <= 8
                        && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
                })
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid WAD map name")
                })?;
            map_name = name.to_ascii_uppercase();
        } else if output.is_none() {
            output = Some(argument);
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected one output path, --interactive, or --map <name>",
            )
            .into());
        }
    }
    let output = output.unwrap_or_else(|| "output/freedoom_map.png".into());
    if interactive {
        run_interactive(Path::new(&wad), &map_name, Path::new(&output))
    } else {
        render(Path::new(&wad), &map_name, Path::new(&output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doom_sky_uses_angle_columns_and_episode_texture() {
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 90.0,
        };
        let center = sky_uv(player, 0.0, 0.0, 256, 128, 960, 720);
        let left = sky_uv(player, -1.0, 0.0, 256, 128, 960, 720);
        let right = sky_uv(player, 1.0, 0.0, 256, 128, 960, 720);
        assert!((center.x - 1.0).abs() < 0.001);
        assert!((center.y - 100.0 / 128.0).abs() < 0.001);
        assert!(left.x > center.x && center.x > right.x);
        assert!((sky_uv(player, 0.0, 1.0, 256, 128, 960, 720).y + 20.0 / 128.0).abs() < 0.001);
        assert!((sky_uv(player, 0.0, -1.0, 256, 128, 960, 720).y - 220.0 / 128.0).abs() < 0.001);

        let map = Map {
            vertices: vec![],
            sectors: vec![Sector {
                floor: 0.0,
                ceiling: 128.0,
                special: 0,
                light: 255,
                tag: 0,
                floor_flat: [0; 8],
                ceiling_flat: *b"F_SKY1\0\0",
            }],
            sides: vec![],
            lines: vec![],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        assert_eq!(sky_texture_name(&map, "E1M2"), Some(*b"SKY1\0\0\0\0"));
        assert_eq!(sky_texture_name(&map, "E2M1"), Some(*b"SKY2\0\0\0\0"));
        assert_eq!(sky_texture_name(&map, "E3M1"), Some(*b"SKY3\0\0\0\0"));
        assert_eq!(sky_texture_name(&map, "E4M1"), Some(*b"SKY4\0\0\0\0"));
        assert_eq!(sky_texture_name(&map, "MAP01"), Some(*b"SKY1\0\0\0\0"));
        assert_eq!(sky_texture_name(&map, "MAP12"), Some(*b"SKY2\0\0\0\0"));
        assert_eq!(sky_texture_name(&map, "MAP21"), Some(*b"SKY3\0\0\0\0"));
        let sky = sky_vertices(player, 41.0, (256, 128), (960, 720));
        assert_eq!(sky.len(), SKY_MESH_SEGMENTS * 6);
        let mut indoor_map = map;
        indoor_map.sectors[0].ceiling_flat = [0; 8];
        assert_eq!(sky_texture_name(&indoor_map, "E1M1"), None);
    }

    #[test]
    fn batches_order_by_nearest_view_depth() {
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        let bounds = |min_x, max_x| Bounds3 {
            min: Vec3::new(min_x, 0.0, -16.0),
            max: Vec3::new(max_x, 128.0, 16.0),
        };
        assert_eq!(depth_bucket(bounds(256.0, 512.0), player), 0);
        assert_eq!(depth_bucket(bounds(2048.0, 2304.0), player), 1);
        assert_eq!(
            depth_bucket(
                Bounds3 {
                    min: Vec3::new(-16.0, 0.0, -2560.0),
                    max: Vec3::new(16.0, 128.0, -2304.0),
                },
                Player {
                    angle: 90.0,
                    ..player
                }
            ),
            1
        );
    }

    #[test]
    fn rejects_truncated_and_out_of_range_wads() {
        assert!(parse_map(b"IWAD", "E1M1").is_err());
        let mut wad = b"IWAD\x01\x00\x00\x00\x0c\x00\x00\x00".to_vec();
        wad.extend_from_slice(&100u32.to_le_bytes());
        wad.extend_from_slice(&4u32.to_le_bytes());
        wad.extend_from_slice(b"E1M1\0\0\0");
        assert!(parse_map(&wad, "E1M1").is_err());
    }

    #[test]
    fn parses_the_requested_map_marker_from_a_multi_map_wad() {
        let mut wad = vec![0; 12];
        wad[..4].copy_from_slice(b"IWAD");
        let mut entries = Vec::new();
        for (marker, light) in [("E1M1", 100i16), ("E1M2", 200i16)] {
            for name in std::iter::once(marker).chain(MAP_LUMPS_AFTER_MARKER) {
                let mut data = Vec::new();
                if name == "SECTORS" {
                    data.resize(26, 0);
                    data[2..4].copy_from_slice(&128i16.to_le_bytes());
                    data[20..22].copy_from_slice(&light.to_le_bytes());
                } else if name == "SSECTORS" {
                    data.resize(4, 0);
                }
                let mut entry = [0; 16];
                entry[..4].copy_from_slice(&(wad.len() as u32).to_le_bytes());
                entry[4..8].copy_from_slice(&(data.len() as u32).to_le_bytes());
                entry[8..8 + name.len()].copy_from_slice(name.as_bytes());
                wad.extend_from_slice(&data);
                entries.push(entry);
            }
        }
        let directory = wad.len() as u32;
        for entry in entries.iter().flatten() {
            wad.push(*entry);
        }
        wad[4..8].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        wad[8..12].copy_from_slice(&directory.to_le_bytes());

        assert_eq!(parse_map(&wad, "E1M2").unwrap().sectors[0].light, 200);
        assert!(parse_map(&wad, "E1M3").is_err());
    }

    #[test]
    fn normal_episode_maps_advance_but_finals_and_secret_maps_stop() {
        assert_eq!(next_normal_map("E1M1").as_deref(), Some("E1M2"));
        assert_eq!(next_normal_map("E3M7").as_deref(), Some("E3M8"));
        assert_eq!(next_normal_map("E1M8"), None);
        assert_eq!(next_normal_map("E4M8"), None);
        assert_eq!(next_normal_map("E1M9"), None);
        assert_eq!(next_normal_map("MAP01"), None);
    }

    #[test]
    fn enemy_walk_cycles_use_their_doom_state_durations() {
        for (sprite, frame_tics) in [
            (*b"TROO", 6.0),
            (*b"SPOS", 6.0),
            (*b"POSS", 8.0),
            (*b"SARG", 4.0),
        ] {
            for (frame, expected) in [0, 1, 2, 3, 0].into_iter().enumerate() {
                let elapsed = frame as f32 * frame_tics / 35.0 + 0.001;
                assert_eq!(actor_walk_frame(sprite, elapsed), expected);
            }
        }
    }

    #[test]
    fn enemy_idle_frames_follow_the_ten_tic_stand_states() {
        assert_eq!(actor_idle_frame(0.0), 0);
        assert_eq!(actor_idle_frame(9.0 / 35.0), 0);
        assert_eq!(actor_idle_frame(10.0 / 35.0 + 0.001), 1);
        assert_eq!(actor_idle_frame(20.0 / 35.0), 0);
    }

    #[test]
    fn enemies_idle_behind_walls_and_keep_targets_for_one_hundred_tics() {
        let map = Map {
            vertices: vec![Vertex2 { x: 50.0, y: -32.0 }, Vertex2 { x: 50.0, y: 32.0 }],
            sectors: vec![],
            sides: vec![],
            lines: vec![[0, 1, 1, u16::MAX, u16::MAX, 0, 0]],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        let mut actors = [Actor {
            sprite: *b"TROO",
            x: 100.0,
            y: 0.0,
            health: 60,
            target_time_remaining: 0.0,
            attack_cooldown: 0.0,
            attack_animation_remaining: 0.0,
            pain_animation_remaining: 0.0,
            death_animation_time: None,
            animation_time: 0.0,
            angle: 90.0,
        }];
        let mut projectiles = Vec::new();
        let mut health = 100;
        let mut player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };

        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            10.0 / 35.0,
        );
        assert_eq!(actors[0].x, 100.0);
        assert_eq!(actors[0].target_time_remaining, 0.0);
        assert_eq!(actor_idle_frame(actors[0].animation_time), 1);
        assert_eq!(actors[0].angle, 90.0);

        player.x = 160.0;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert_eq!(actors[0].target_time_remaining, ACTOR_TARGET_THRESHOLD);
        assert_eq!(actors[0].animation_time, 0.0);
        assert_eq!(actors[0].angle, 0.0);

        player.x = 0.0;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.1,
        );
        assert!((actors[0].target_time_remaining - (ACTOR_TARGET_THRESHOLD - 0.1)).abs() < 0.0001);
        assert_eq!(actors[0].angle, 180.0);

        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            ACTOR_TARGET_THRESHOLD,
        );
        assert_eq!(actors[0].target_time_remaining, 0.0);

        actors[0].x = 60.0;
        player.x = 40.0;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert_eq!(health, 100);
    }

    #[test]
    fn enemy_sprite_views_use_doom_names_and_view_angle_buckets() {
        assert_eq!(sprite_view_name(*b"TROO", b'A', 1), (*b"TROOA1\0\0", false));
        assert_eq!(sprite_view_name(*b"TROO", b'A', 8), (*b"TROOA2A8", true));
        assert_eq!(sprite_view_name(*b"TROO", b'A', 7), (*b"TROOA3A7", true));
        assert_eq!(sprite_view_name(*b"TROO", b'A', 6), (*b"TROOA4A6", true));
        assert_eq!(sprite_view_name(*b"TROO", b'A', 5), (*b"TROOA5\0\0", false));
        assert_eq!(sprite_view_name(*b"SARG", b'B', 8), (*b"SARGB2B8", true));
        assert_eq!(actor_view_rotation(0.0, 180.0), 0);
        assert_eq!(actor_view_rotation(0.0, 90.0), 6);
        assert_eq!(actor_view_rotation(0.0, 0.0), 4);
        assert_eq!(actor_view_rotation(0.0, 270.0), 2);
    }

    #[test]
    fn enemy_attack_poses_follow_their_doom_state_sequences() {
        for (sprite, total, second, third, third_frame) in [
            (*b"TROO", 22.0, 14.0, 6.0, 6),
            (*b"SARG", 24.0, 16.0, 8.0, 6),
            (*b"POSS", 26.0, 16.0, 8.0, 4),
            (*b"SPOS", 30.0, 20.0, 10.0, 4),
        ] {
            let remaining = |tics: f32| tics / 35.0;
            assert_eq!(actor_attack_frame(sprite, remaining(total)), Some(4));
            assert_eq!(actor_attack_frame(sprite, remaining(second)), Some(5));
            assert_eq!(
                actor_attack_frame(sprite, remaining(third)),
                Some(third_frame)
            );
            assert_eq!(actor_attack_frame(sprite, 0.0), None);
        }
        assert_eq!(actor_attack_duration(*b"TROO"), 22.0 / 35.0);
        assert_eq!(actor_attack_duration(*b"SARG"), 24.0 / 35.0);
        assert_eq!(actor_attack_duration(*b"POSS"), 26.0 / 35.0);
        assert_eq!(actor_attack_duration(*b"SPOS"), 30.0 / 35.0);
    }

    #[test]
    fn enemy_death_states_advance_and_hold_the_final_corpse_frame() {
        let profiles: [([u8; 4], &[usize], &[f32]); 4] = [
            (*b"TROO", &[8, 9, 10, 11, 12], &[8.0, 8.0, 6.0, 6.0]),
            (
                *b"SARG",
                &[8, 9, 10, 11, 12, 13],
                &[8.0, 8.0, 4.0, 4.0, 4.0],
            ),
            (*b"POSS", &[7, 8, 9, 10, 11], &[5.0, 5.0, 5.0, 5.0]),
            (*b"SPOS", &[7, 8, 9, 10, 11], &[5.0, 5.0, 5.0, 5.0]),
        ];
        for (sprite, frames, durations) in profiles {
            let mut elapsed = 0.0;
            for (&frame, &duration) in frames.iter().zip(durations) {
                assert_eq!(actor_death_frame(sprite, elapsed / 35.0), Some(frame));
                elapsed += duration;
            }
            let corpse = frames.last().copied();
            assert_eq!(actor_death_frame(sprite, elapsed / 35.0), corpse);
            assert_eq!(actor_death_frame(sprite, 100.0), corpse);
            assert_eq!(actor_death_duration(sprite), elapsed / 35.0);
        }
    }

    #[test]
    fn fireball_impact_frames_last_six_doom_tics_each() {
        assert_eq!(projectile_explosion_frame(0.0), Some(0));
        assert_eq!(projectile_explosion_frame(6.0 / 35.0 + 0.001), Some(1));
        assert_eq!(projectile_explosion_frame(12.0 / 35.0 + 0.001), Some(2));
        assert_eq!(
            projectile_explosion_frame(projectile_explosion_duration()),
            None
        );
    }

    #[test]
    fn mirrored_sprite_views_reverse_billboard_texture_coordinates() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
        };
        let vertices = billboard_vertices(
            Vertex2 { x: 0.0, y: 0.0 },
            Vertex2 { x: 1.0, y: 0.0 },
            10.0,
            0.0,
            10.0,
            sector,
            true,
        );
        assert_eq!(vertices[0].uv.x, 1.0);
        assert_eq!(vertices[1].uv.x, 0.0);
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
    fn validated_bsp_cells_complete_subsector_flat_geometry() {
        let map = Map {
            vertices: vec![
                Vertex2 { x: 0.0, y: 0.0 },
                Vertex2 { x: 10.0, y: 0.0 },
                Vertex2 { x: 10.0, y: 10.0 },
                Vertex2 { x: 0.0, y: 10.0 },
                Vertex2 { x: 0.0, y: 5.0 },
                Vertex2 { x: 10.0, y: 5.0 },
            ],
            sectors: vec![Sector {
                floor: 0.0,
                ceiling: 128.0,
                special: 0,
                light: 255,
                tag: 0,
                floor_flat: *b"FLOOR0_1",
                ceiling_flat: *b"CEIL0_1\0",
            }],
            sides: vec![
                SideDef {
                    x_offset: 0,
                    y_offset: 0,
                    upper: [0; 8],
                    lower: [0; 8],
                    middle: [0; 8],
                    sector: 0,
                };
                5
            ],
            lines: vec![
                [0, 1, 0, 0, u16::MAX, 0, 0],
                [1, 2, 0, 1, u16::MAX, 0, 0],
                [2, 3, 0, 2, u16::MAX, 0, 0],
                [3, 4, 0, 3, u16::MAX, 0, 0],
                [4, 0, 0, 4, u16::MAX, 0, 0],
                [4, 5, 0, 0, 0, 0, 0],
            ],
            segs: vec![[0, 1, 0, 0, 0], [4, 0, 4, 0, 0], [4, 5, 5, 0, 0]],
            subsectors: vec![[2, 0], [1, 2]],
            nodes: vec![Node {
                x: 0,
                y: 5,
                dx: 1,
                dy: 0,
                child_bounds: [Bounds2::default(); 2],
                children: [0x8000, 0x8001],
            }],
            things: vec![],
        };
        let cell = bsp_subsector_polygons(&map);
        assert!(bsp_polygon_belongs_to_sector(&map, &cell[0], 0));
        assert!(bsp_polygon_belongs_to_sector(&map, &cell[1], 0));
        assert!(!bsp_polygon_belongs_to_sector(
            &map,
            &[
                Vertex2 { x: 0.0, y: 12.0 },
                Vertex2 { x: 10.0, y: 12.0 },
                Vertex2 { x: 10.0, y: 20.0 },
                Vertex2 { x: 0.0, y: 20.0 },
            ],
            0,
        ));
        let pixels = [255; 4];
        let texture = Arc::new(Texture::new(1, 1, TextureFormat::Rgba8, &pixels).unwrap());
        let textures = BTreeMap::from([
            (*b"FLOOR0_1", Arc::clone(&texture)),
            (*b"CEIL0_1\0", texture),
        ]);

        let geometry = geometry(&map, &textures).unwrap();
        assert_eq!(geometry[0].flats[b"FLOOR0_1"].len(), 6);
        assert_eq!(geometry[0].flats[b"CEIL0_1\0"].len(), 6);
        assert_eq!(geometry[1].flats[b"FLOOR0_1"].len(), 6);
        assert_eq!(geometry[1].flats[b"CEIL0_1\0"].len(), 6);
    }

    #[test]
    fn bsp_cells_are_split_at_concave_sector_boundaries_before_flat_fill() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 10.0,
            special: 0,
            light: 255,
            tag: 0,
            floor_flat: *b"FLOOR0_1",
            ceiling_flat: *b"CEIL0_1\0",
        };
        let map = Map {
            vertices: vec![
                Vertex2 { x: 0.0, y: 0.0 },
                Vertex2 { x: 10.0, y: 0.0 },
                Vertex2 { x: 10.0, y: 5.0 },
                Vertex2 { x: 5.0, y: 5.0 },
                Vertex2 { x: 5.0, y: 10.0 },
                Vertex2 { x: 0.0, y: 10.0 },
            ],
            sectors: vec![sector],
            sides: (0..6)
                .map(|_| SideDef {
                    x_offset: 0,
                    y_offset: 0,
                    upper: [0; 8],
                    lower: [0; 8],
                    middle: [0; 8],
                    sector: 0,
                })
                .collect(),
            lines: vec![
                [0, 1, 0, 0, u16::MAX, 0, 0],
                [1, 2, 0, 1, u16::MAX, 0, 0],
                [2, 3, 0, 2, u16::MAX, 0, 0],
                [3, 4, 0, 3, u16::MAX, 0, 0],
                [4, 5, 0, 4, u16::MAX, 0, 0],
                [5, 0, 0, 5, u16::MAX, 0, 0],
            ],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        let cell = [
            Vertex2 { x: 0.0, y: 0.0 },
            Vertex2 { x: 10.0, y: 0.0 },
            Vertex2 { x: 10.0, y: 10.0 },
            Vertex2 { x: 0.0, y: 10.0 },
        ];

        let clipped = sector_clipped_bsp_polygons(&map, &cell, 0);
        assert_eq!(clipped.len(), 3);
        assert!(
            clipped
                .iter()
                .all(|piece| bsp_polygon_belongs_to_sector(&map, piece, 0))
        );
        let area = clipped
            .iter()
            .map(|polygon| {
                (0..polygon.len())
                    .map(|i| {
                        let a = polygon[i];
                        let b = polygon[(i + 1) % polygon.len()];
                        a.x * b.y - b.x * a.y
                    })
                    .sum::<f32>()
                    .abs()
                    * 0.5
            })
            .sum::<f32>();
        assert_eq!(area, 75.0);
        assert!(!sector_contains_point(&map, 0, Vertex2 { x: 8.0, y: 8.0 }));
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
    fn two_sided_middle_textures_follow_peg_flags_and_offsets() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
        };
        let side = |sector| SideDef {
            x_offset: 5,
            y_offset: 3,
            upper: *b"-\0\0\0\0\0\0\0",
            lower: *b"-\0\0\0\0\0\0\0",
            middle: *b"MASK\0\0\0\0",
            sector,
        };
        let mut map = Map {
            vertices: vec![Vertex2 { x: 96.0, y: 0.0 }, Vertex2 { x: 96.0, y: 64.0 }],
            sectors: vec![
                sector,
                Sector {
                    floor: 16.0,
                    ceiling: 112.0,
                    special: 0,
                    ..sector
                },
            ],
            sides: vec![side(0), side(1)],
            lines: vec![[0, 1, 4, 0, 1, 0, 0]],
            segs: vec![[0, 1, 0, 0, 7], [0, 1, 0, 1, 7]],
            subsectors: vec![[1, 0], [1, 1]],
            nodes: vec![],
            things: vec![],
        };
        let pixels = vec![255; 16 * 32 * 4];
        let texture = Arc::new(Texture::new(16, 32, TextureFormat::Rgba8, &pixels).unwrap());
        let textures = BTreeMap::from([(*b"MASK\0\0\0\0", texture)]);
        let y_bounds = |vertices: &[Vertex]| {
            vertices
                .iter()
                .map(|v| v.position.y)
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), y| {
                    (low.min(y), high.max(y))
                })
        };

        let top_pegged = geometry(&map, &textures).unwrap();
        let vertices = &top_pegged[0].masked[b"MASK\0\0\0\0"];
        assert_eq!(vertices.len(), 6);
        assert_eq!(y_bounds(vertices), (80.0, 112.0));
        assert_eq!(vertices[0].uv.x, 0.75);
        assert_eq!(vertices[1].uv.x, 4.75);
        assert_eq!(vertices[0].uv.y, 35.0 / 32.0);
        assert_eq!(vertices[2].uv.y, 3.0 / 32.0);

        map.lines[0][2] |= 16;
        let bottom_pegged = geometry(&map, &textures).unwrap();
        let vertices = &bottom_pegged[0].masked[b"MASK\0\0\0\0"];
        assert_eq!(y_bounds(vertices), (16.0, 48.0));
    }

    #[test]
    fn bsp_selects_the_sector_and_rejects_solid_wall_overlap() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: *b"FLOOR0_1",
            ceiling_flat: *b"CEIL1_1\0",
            tag: 0,
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
            lines: vec![[0, 1, 1, 0, u16::MAX, 0, 0], [2, 3, 1, 1, u16::MAX, 0, 0]],
            segs: vec![[0, 1, 0, 0, 0], [2, 3, 1, 0, 0]],
            subsectors: vec![[1, 0], [1, 1]],
            nodes: vec![Node {
                x: 128,
                y: 0,
                dx: 0,
                dy: 1,
                child_bounds: [Bounds2::default(); 2],
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
            child_bounds: [Bounds2::default(); 2],
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
        let crossed = move_player(
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
        assert!(crossed.is_empty());
        assert_eq!(player.x, 80.0);
        assert_eq!(player.angle, 0.5);

        let mut cyclic = map;
        cyclic.nodes[0].children = [0, 0];
        assert_eq!(subsector_at(&cyclic, Vertex2 { x: 0.0, y: 0.0 }), None);
    }

    #[test]
    fn secret_and_nukage_sector_specials_follow_doom_rules() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: SECTOR_NUKAGE_DAMAGE,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
        };
        let map = &mut Map {
            vertices: vec![Vertex2 { x: 0.0, y: 0.0 }, Vertex2 { x: 32.0, y: 0.0 }],
            sectors: vec![sector],
            sides: vec![SideDef {
                x_offset: 0,
                y_offset: 0,
                upper: [0; 8],
                lower: [0; 8],
                middle: [0; 8],
                sector: 0,
            }],
            lines: vec![[0, 1, 0, 0, u16::MAX, 0, 0]],
            segs: vec![[0, 1, 0, 0, 0]],
            subsectors: vec![[1, 0]],
            nodes: vec![],
            things: vec![],
        };
        let player = Player {
            x: 8.0,
            y: 8.0,
            angle: 0.0,
        };
        let mut health = 100;
        let mut elapsed_tics = 0.0;
        update_floor_damage(map, player, &mut health, &mut elapsed_tics, 0.5);
        assert_eq!(health, 100);
        update_floor_damage(map, player, &mut health, &mut elapsed_tics, 0.4);
        assert_eq!(health, 100);
        update_floor_damage(map, player, &mut health, &mut elapsed_tics, 0.03);
        assert_eq!(health, 95);
        update_floor_damage(map, player, &mut health, &mut elapsed_tics, 0.9);
        assert_eq!(health, 90);

        map.sectors[0].special = 0;
        update_floor_damage(map, player, &mut health, &mut elapsed_tics, 0.1);
        assert_eq!(elapsed_tics, 0.0);
        map.sectors[0].special = SECTOR_SECRET;
        assert!(discover_secret(map, player));
        assert_eq!(map.sectors[0].special, 0);
        assert!(!discover_secret(map, player));
    }

    #[test]
    fn sector_blinks_and_slow_strobes_change_light_on_doom_tics() {
        let sector = |special, light| Sector {
            floor: 0.0,
            ceiling: 128.0,
            special,
            light,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
        };
        let side = |sector| SideDef {
            x_offset: 0,
            y_offset: 0,
            upper: [0; 8],
            lower: [0; 8],
            middle: [0; 8],
            sector,
        };
        let mut map = Map {
            vertices: vec![Vertex2 { x: 0.0, y: 0.0 }, Vertex2 { x: 32.0, y: 0.0 }],
            sectors: vec![
                sector(SECTOR_LIGHT_FLASH, 180),
                sector(SECTOR_LIGHT_STROBE_SLOW, 220),
                sector(0, 80),
            ],
            sides: vec![side(0), side(2), side(1), side(2)],
            lines: vec![
                [0, 1, LINE_TWO_SIDED, 0, 1, 0, 0],
                [0, 1, LINE_TWO_SIDED, 2, 3, 0, 0],
            ],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        let mut rng = 0x4c49_4748;
        let mut lights = spawn_sector_lights(&mut map, &mut rng);

        assert_eq!(map.sectors[0].special, 0);
        assert_eq!(map.sectors[1].special, 0);
        assert_eq!(lights[0].min, 80);
        assert!((1.0..=65.0).contains(&lights[0].tics));
        assert_eq!(lights[1].min, 80);
        assert_eq!(lights[1].tics, 1.0);

        let blink_wait = lights[0].tics;
        assert!(update_sector_lights(
            &mut map,
            &mut lights[..1],
            &mut rng,
            (blink_wait + 0.01) / DOOM_TICS_PER_SECOND,
        ));
        assert_eq!(map.sectors[0].light, 80);
        let dark_wait = lights[0].tics;
        update_sector_lights(
            &mut map,
            &mut lights[..1],
            &mut rng,
            (dark_wait + 0.01) / DOOM_TICS_PER_SECOND,
        );
        assert_eq!(map.sectors[0].light, 180);

        update_sector_lights(
            &mut map,
            &mut lights[1..],
            &mut rng,
            1.01 / DOOM_TICS_PER_SECOND,
        );
        assert_eq!(map.sectors[1].light, 80);
        update_sector_lights(
            &mut map,
            &mut lights[1..],
            &mut rng,
            (STROBE_SLOW_DARK_TICS + 0.01) / DOOM_TICS_PER_SECOND,
        );
        assert_eq!(map.sectors[1].light, 220);
        update_sector_lights(
            &mut map,
            &mut lights[1..],
            &mut rng,
            (STROBE_BRIGHT_TICS + 0.01) / DOOM_TICS_PER_SECOND,
        );
        assert_eq!(map.sectors[1].light, 80);
    }

    #[test]
    fn use_activates_the_front_of_an_exit_line_through_open_space() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
        };
        let side = SideDef {
            x_offset: 0,
            y_offset: 0,
            upper: [0; 8],
            lower: [0; 8],
            middle: [0; 8],
            sector: 0,
        };
        let mut map = Map {
            vertices: vec![
                Vertex2 { x: 20.0, y: 16.0 },
                Vertex2 { x: 20.0, y: -16.0 },
                Vertex2 { x: 40.0, y: 16.0 },
                Vertex2 { x: 40.0, y: -16.0 },
            ],
            sectors: vec![sector],
            sides: vec![side; 2],
            lines: vec![
                [0, 1, LINE_TWO_SIDED, 0, 1, 0, 0],
                [2, 3, 1, 0, u16::MAX, LINE_EXIT_USE, 0],
            ],
            segs: vec![[0, 1, 0, 0, 0]],
            subsectors: vec![[1, 0]],
            nodes: vec![],
            things: vec![],
        };
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        assert_eq!(use_line(&map, player), Some((1, LINE_EXIT_USE)));
        assert_eq!(
            use_line(
                &map,
                Player {
                    x: 80.0,
                    angle: 180.0,
                    ..player
                }
            ),
            None
        );
        map.vertices[2].x = 100.0;
        map.vertices[3].x = 100.0;
        assert_eq!(use_line(&map, player), None);
        map.lines[0][4] = u16::MAX;
        map.vertices[2].x = 40.0;
        map.vertices[3].x = 40.0;
        assert_eq!(use_line(&map, player), None);
    }

    #[test]
    fn manual_doors_raise_reopen_the_portal_and_blue_locks_require_the_card() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
        };
        let closed_door = Sector {
            ceiling: 0.0,
            special: 0,
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
        let mut map = Map {
            vertices: vec![Vertex2 { x: 0.0, y: -32.0 }, Vertex2 { x: 0.0, y: 32.0 }],
            sectors: vec![sector, closed_door],
            sides: vec![side(0), side(1)],
            lines: vec![[0, 1, LINE_TWO_SIDED, 0, 1, LINE_DOOR_RAISE, 0]],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        assert_eq!(
            use_line(
                &map,
                Player {
                    x: 32.0,
                    y: 0.0,
                    angle: 180.0,
                }
            ),
            Some((0, LINE_DOOR_RAISE))
        );
        assert_eq!(
            use_line(
                &map,
                Player {
                    x: -32.0,
                    y: 0.0,
                    angle: 0.0,
                }
            ),
            None
        );
        let point = Vertex2 { x: 0.0, y: 0.0 };
        assert!(!actor_path_clear(&map, point, point));
        let mut doors = vec![manual_door(&map, 0, false).unwrap()];
        assert!(update_doors(
            &mut map,
            &mut doors,
            Player {
                x: 200.0,
                y: 0.0,
                angle: 0.0
            },
            &[],
            2.0,
        ));
        assert_eq!(map.sectors[1].ceiling, 124.0);
        assert!(actor_path_clear(&map, point, point));
        assert!(update_doors(
            &mut map,
            &mut doors,
            Player {
                x: 200.0,
                y: 0.0,
                angle: 0.0
            },
            &[],
            DOOR_WAIT + 2.0,
        ));
        assert_eq!(map.sectors[1].ceiling, 0.0);
        assert!(doors.is_empty());

        map.lines[0][5] = LINE_BLUE_LOCKED_DOOR;
        assert!(manual_door(&map, 0, false).is_none());
        assert_eq!(manual_door(&map, 0, true).unwrap().speed, DOOR_SPEED);

        map.lines[0][5] = LINE_BLAZING_DOOR_RAISE;
        let player = Player {
            x: 32.0,
            y: 0.0,
            angle: 180.0,
        };
        assert_eq!(use_line(&map, player), Some((0, LINE_BLAZING_DOOR_RAISE)));
        let mut doors = vec![manual_door(&map, 0, false).unwrap()];
        assert_eq!(doors[0].speed, BLAZING_DOOR_SPEED);
        assert!(update_doors(
            &mut map,
            &mut doors,
            Player { x: 200.0, ..player },
            &[],
            0.1
        ));
        assert_eq!(map.sectors[1].ceiling, 28.0);
        assert!(update_doors(
            &mut map,
            &mut doors,
            Player { x: 200.0, ..player },
            &[],
            0.4
        ));
        assert_eq!(map.sectors[1].ceiling, 124.0);
        assert_eq!(doors[0].direction, 0);
    }

    #[test]
    fn walk_open_door_crossings_are_reported_only_after_valid_movement() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
        };
        let side = SideDef {
            x_offset: 0,
            y_offset: 0,
            upper: [0; 8],
            lower: [0; 8],
            middle: [0; 8],
            sector: 0,
        };
        let mut map = Map {
            vertices: vec![Vertex2 { x: 4.0, y: -32.0 }, Vertex2 { x: 4.0, y: 32.0 }],
            sectors: vec![sector],
            sides: vec![side; 2],
            lines: vec![[0, 1, LINE_TWO_SIDED, 0, 1, LINE_WALK_OPEN_DOOR, 5]],
            segs: vec![[0, 1, 0, 0, 0]],
            subsectors: vec![[1, 0]],
            nodes: vec![],
            things: vec![],
        };
        let controls = Controls {
            forward: 1.0,
            strafe: 0.0,
            turn: 0.0,
            speed: 160.0,
        };
        let mut player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        assert_eq!(move_player(&map, &mut player, controls, 0.05), vec![0]);
        assert_eq!(player.x, 8.0);

        map.lines[0][5] = LINE_PLAT_DOWN_WAIT_UP;
        player.x = 0.0;
        assert_eq!(move_player(&map, &mut player, controls, 0.05), vec![0]);
        assert_eq!(map.lines[0][5], LINE_PLAT_DOWN_WAIT_UP);

        map.lines[0][2] = 0;
        map.lines[0][4] = u16::MAX;
        player.x = 0.0;
        assert!(move_player(&map, &mut player, controls, 0.05).is_empty());
        assert_eq!(player.x, 0.0);
    }

    #[test]
    fn walk_open_doors_open_only_the_matching_tag_and_stay_open() {
        let open = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
        };
        let side = |sector| SideDef {
            x_offset: 0,
            y_offset: 0,
            upper: [0; 8],
            lower: [0; 8],
            middle: [0; 8],
            sector,
        };
        let mut map = Map {
            vertices: vec![
                Vertex2 { x: 0.0, y: -32.0 },
                Vertex2 { x: 0.0, y: 32.0 },
                Vertex2 { x: 64.0, y: -32.0 },
                Vertex2 { x: 64.0, y: 32.0 },
            ],
            sectors: vec![
                open,
                Sector {
                    ceiling: 0.0,
                    special: 0,
                    tag: 5,
                    ..open
                },
                Sector {
                    ceiling: 0.0,
                    special: 0,
                    tag: 7,
                    ..open
                },
            ],
            sides: vec![side(0), side(1), side(0), side(2)],
            lines: vec![
                [0, 1, LINE_TWO_SIDED, 0, 1, LINE_WALK_OPEN_DOOR, 5],
                [2, 3, LINE_TWO_SIDED, 2, 3, 0, 0],
            ],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        let mut doors = walk_open_doors(&mut map, 0, &[]);
        assert_eq!(map.lines[0][5], 0);
        assert_eq!(doors.len(), 1);
        assert!(!doors[0].auto_close);
        assert_eq!(map.sectors[2].ceiling, 0.0);
        assert!(update_doors(
            &mut map,
            &mut doors,
            Player {
                x: 200.0,
                y: 0.0,
                angle: 0.0,
            },
            &[],
            2.0,
        ));
        assert_eq!(map.sectors[1].ceiling, 124.0);
        assert_eq!(map.sectors[2].ceiling, 0.0);
        assert!(doors.is_empty());
    }

    #[test]
    fn platforms_and_lower_floor_actions_update_tagged_sector_geometry() {
        let open = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
        };
        let side = |sector| SideDef {
            x_offset: 0,
            y_offset: 0,
            upper: [0; 8],
            lower: [0; 8],
            middle: [0; 8],
            sector,
        };
        let mut map = Map {
            vertices: vec![
                Vertex2 { x: 0.0, y: -32.0 },
                Vertex2 { x: 0.0, y: 32.0 },
                Vertex2 { x: 64.0, y: -32.0 },
                Vertex2 { x: 64.0, y: 32.0 },
            ],
            sectors: vec![
                open,
                Sector {
                    floor: 64.0,
                    tag: 5,
                    ..open
                },
                Sector {
                    floor: 32.0,
                    tag: 7,
                    ..open
                },
            ],
            sides: vec![side(0), side(1), side(0), side(2)],
            lines: vec![
                [0, 1, LINE_TWO_SIDED, 0, 1, LINE_PLAT_DOWN_WAIT_UP, 5],
                [2, 3, LINE_TWO_SIDED, 2, 3, 0, 0],
            ],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        let mut platforms = down_wait_up_platforms(&map, 0, &[]);
        assert_eq!(platforms.len(), 1);
        assert_eq!((platforms[0].low, platforms[0].high), (0.0, 64.0));
        assert!(!portal_is_walkable(&map, map.lines[0], 0, 1));
        assert!(update_platforms(&mut map, &mut platforms, 0.1));
        assert_eq!(map.sectors[1].floor, 50.0);
        assert!(update_platforms(&mut map, &mut platforms, 1.0));
        assert_eq!(map.sectors[1].floor, 0.0);
        assert!(portal_is_walkable(&map, map.lines[0], 0, 1));
        assert_eq!(platforms[0].direction, 0);
        assert!(!update_platforms(&mut map, &mut platforms, 2.9));
        assert_eq!(platforms[0].direction, 0);
        assert!(!update_platforms(&mut map, &mut platforms, 0.1));
        assert_eq!(platforms[0].direction, 1);
        assert!(update_platforms(&mut map, &mut platforms, 0.1));
        assert_eq!(map.sectors[1].floor, 14.0);
        assert!(update_platforms(&mut map, &mut platforms, 0.4));
        assert_eq!(map.sectors[1].floor, 64.0);
        assert!(!portal_is_walkable(&map, map.lines[0], 0, 1));
        assert!(platforms.is_empty());
        assert_eq!(map.sectors[2].floor, 32.0);
        assert_eq!(map.lines[0][5], LINE_PLAT_DOWN_WAIT_UP);
        assert_eq!(down_wait_up_platforms(&map, 0, &[]).len(), 1);

        map.lines[0][5] = LINE_USE_DOWN_WAIT_UP_PLATFORM;
        let player = Player {
            x: 20.0,
            y: 0.0,
            angle: 180.0,
        };
        assert_eq!(
            use_line(&map, player),
            Some((0, LINE_USE_DOWN_WAIT_UP_PLATFORM))
        );
        let manual = down_wait_up_platforms(&map, 0, &[]);
        assert_eq!(manual.len(), 1);
        assert!(down_wait_up_platforms(&map, 0, &manual).is_empty());

        map.sectors[2].tag = 5;
        map.lines[0][5] = LINE_USE_LOWER_FLOOR_TO_LOWEST;
        assert_eq!(
            use_line(&map, player),
            Some((0, LINE_USE_LOWER_FLOOR_TO_LOWEST))
        );
        let mut floors = lower_to_lowest_floors(&mut map, 0, &[]);
        assert_eq!(floors.len(), 2);
        assert!(
            floors
                .iter()
                .all(|floor| floor.speed == FLOOR_SPEED && !floor.return_to_high)
        );
        assert_eq!(map.lines[0][5], 0);
        assert!(lower_to_lowest_floors(&mut map, 0, &[]).is_empty());
        assert!(update_platforms(&mut map, &mut floors, 0.5));
        assert_eq!(map.sectors[1].floor, 46.5);
        assert_eq!(map.sectors[2].floor, 14.5);
        assert!(update_platforms(&mut map, &mut floors, 1.0));
        assert_eq!(map.sectors[1].floor, 11.5);
        assert_eq!(map.sectors[2].floor, 0.0);
        assert!(update_platforms(&mut map, &mut floors, 0.4));
        assert_eq!(map.sectors[1].floor, 0.0);
        assert_eq!(map.sectors[2].floor, 0.0);
        assert!(floors.is_empty());
    }

    #[test]
    fn enemy_routes_use_open_sector_portals_and_respect_steps_and_clearance() {
        let open = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
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
            vertices: vec![],
            sectors: vec![
                open,
                open,
                open,
                Sector {
                    ceiling: 55.0,
                    special: 0,
                    ..open
                },
                Sector {
                    floor: 40.0,
                    ..open
                },
            ],
            sides: (0..5).map(side).collect(),
            lines: vec![
                [0, 1, 0, 0, 1, 0, 0], // sector 0 -> 1
                [0, 1, 0, 1, 2, 0, 0], // sector 1 -> 2
                [0, 1, 1, 0, 2, 0, 0], // blocked shortcut
                [0, 1, 0, 0, 3, 0, 0], // too little actor clearance
                [0, 1, 0, 0, 4, 0, 0], // step exceeds the limit
            ],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        let routes = sector_routes(&map);

        assert_eq!(first_route_portal(&routes, 0, 2), Some((0, 1)));
        assert_eq!(first_route_portal(&routes, 1, 2), Some((1, 2)));
        assert_eq!(first_route_portal(&routes, 0, 3), None);
        assert_eq!(first_route_portal(&routes, 0, 4), None);
    }

    #[test]
    fn missed_pistol_shots_alert_monsters_through_one_sound_blocking_line() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
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
            vertices: [30.0, 40.0, 50.0, 60.0]
                .into_iter()
                .flat_map(|x| [Vertex2 { x, y: -64.0 }, Vertex2 { x, y: 64.0 }])
                .collect(),
            sectors: vec![sector; 5],
            sides: vec![
                side(0),
                side(1),
                side(1),
                side(2),
                side(2),
                side(3),
                side(3),
                side(4),
            ],
            lines: vec![
                [0, 1, LINE_TWO_SIDED, 0, 1, 0, 0],
                [2, 3, LINE_TWO_SIDED | LINE_SOUND_BLOCK, 2, 3, 0, 0],
                [4, 5, LINE_TWO_SIDED | LINE_SOUND_BLOCK, 4, 5, 0, 0],
                [6, 7, LINE_TWO_SIDED, 6, 7, 0, 0],
            ],
            segs: vec![
                [0, 1, 0, 0, 0],
                [0, 1, 0, 1, 0],
                [2, 3, 1, 1, 0],
                [4, 5, 2, 1, 0],
                [6, 7, 3, 1, 0],
            ],
            subsectors: vec![[1, 0], [1, 1], [1, 2], [1, 3], [1, 4]],
            nodes: vec![
                Node {
                    x: 60,
                    y: 0,
                    dx: 0,
                    dy: 1,
                    child_bounds: [Bounds2::default(); 2],
                    children: [0x8004, 0x8003],
                },
                Node {
                    x: 50,
                    y: 0,
                    dx: 0,
                    dy: 1,
                    child_bounds: [Bounds2::default(); 2],
                    children: [0, 0x8002],
                },
                Node {
                    x: 40,
                    y: 0,
                    dx: 0,
                    dy: 1,
                    child_bounds: [Bounds2::default(); 2],
                    children: [1, 0x8001],
                },
                Node {
                    x: 30,
                    y: 0,
                    dx: 0,
                    dy: 1,
                    child_bounds: [Bounds2::default(); 2],
                    children: [2, 0x8000],
                },
            ],
            things: vec![],
        };
        assert_eq!(
            sound_reachable_sectors(&map, 0),
            [true, true, true, false, false]
        );

        let actor = |x| Actor {
            sprite: *b"SARG",
            x,
            y: 0.0,
            health: 60,
            target_time_remaining: 0.0,
            attack_cooldown: 0.0,
            attack_animation_remaining: 0.0,
            pain_animation_remaining: 0.0,
            death_animation_time: None,
            animation_time: 0.25,
            angle: 0.0,
        };
        let mut actors = [actor(45.0), actor(55.0)];
        let mut pain_rng = 1;
        assert!(!fire_weapon(
            &map,
            &mut actors,
            Player {
                x: 15.0,
                y: 0.0,
                angle: 90.0,
            },
            &mut pain_rng,
        ));
        assert_eq!(actors[0].target_time_remaining, ACTOR_TARGET_THRESHOLD);
        assert_eq!(actors[0].animation_time, 0.0);
        assert_eq!(actors[1].target_time_remaining, 0.0);
    }

    #[test]
    fn enemy_pursuit_follows_a_portal_route_around_a_blocking_wall() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
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
                Vertex2 { x: 10.0, y: -64.0 },
                Vertex2 { x: 10.0, y: 64.0 },
                Vertex2 { x: 10.0, y: 10.0 },
                Vertex2 { x: 30.0, y: 10.0 },
                Vertex2 { x: 10.0, y: 14.0 },
                Vertex2 { x: 10.0, y: 18.0 },
            ],
            sectors: vec![sector; 3],
            sides: vec![side(0), side(1), side(1), side(2), side(0)],
            lines: vec![
                [0, 1, 0, 0, 1, 0, 0],
                [2, 3, 0, 2, 3, 0, 0],
                [4, 5, 1, 4, u16::MAX, 0, 0],
            ],
            segs: vec![[0, 1, 0, 0, 0], [0, 1, 0, 1, 0], [2, 3, 1, 1, 0]],
            subsectors: vec![[1, 0], [1, 1], [1, 2]],
            nodes: vec![
                Node {
                    x: 0,
                    y: 10,
                    dx: 1,
                    dy: 0,
                    child_bounds: [Bounds2::default(); 2],
                    children: [0x8001, 0x8002],
                },
                Node {
                    x: 10,
                    y: 0,
                    dx: 0,
                    dy: 1,
                    child_bounds: [Bounds2::default(); 2],
                    children: [0, 0x8000],
                },
            ],
            things: vec![],
        };
        let routes = sector_routes(&map);
        let waypoint = chase_waypoint(
            &map,
            &routes,
            Vertex2 { x: 0.0, y: -10.0 },
            Vertex2 { x: 20.0, y: 40.0 },
        )
        .unwrap();

        assert_eq!(bsp_sector_index_at(&map, 0.0, -10.0), Some(0));
        assert_eq!(bsp_sector_index_at(&map, 20.0, 40.0), Some(2));
        assert_eq!(bsp_sector_index_at(&map, waypoint.x, waypoint.y), Some(1));
        assert!(actor_path_clear(
            &map,
            Vertex2 { x: 0.0, y: -10.0 },
            waypoint
        ));
        assert!(!has_line_of_sight(
            &map,
            Vertex2 { x: 0.0, y: -10.0 },
            Vertex2 { x: 20.0, y: 40.0 },
        ));

        let mut actors = [Actor {
            sprite: *b"SARG",
            x: 0.0,
            y: -10.0,
            health: 60,
            target_time_remaining: 2.0,
            attack_cooldown: 0.0,
            attack_animation_remaining: 0.0,
            pain_animation_remaining: 0.0,
            death_animation_time: None,
            animation_time: 0.0,
            angle: 0.0,
        }];
        let mut projectiles = Vec::new();
        let mut health = 100;
        let player = Player {
            x: 20.0,
            y: 40.0,
            angle: 0.0,
        };
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.5,
        );
        assert!(
            actors[0].x > 10.0,
            "enemy stopped at ({}, {})",
            actors[0].x,
            actors[0].y
        );
        assert_eq!(health, 100);
    }

    #[test]
    fn bsp_view_traversal_culls_child_bounds_and_stops_cycles() {
        let bounds = |min_x, min_y, max_x, max_y| Bounds2 {
            min_x,
            min_y,
            max_x,
            max_y,
        };
        let mut map = Map {
            vertices: vec![],
            sectors: vec![],
            sides: vec![],
            lines: vec![],
            segs: vec![],
            subsectors: vec![[0, 0], [0, 0]],
            nodes: vec![Node {
                x: 0,
                y: 0,
                dx: 0,
                dy: 1,
                child_bounds: [
                    bounds(-300.0, -20.0, -100.0, 20.0),
                    bounds(100.0, -20.0, 300.0, 20.0),
                ],
                children: [0x8000, 0x8001],
            }],
            things: vec![],
        };
        let player = Player {
            x: 1.0,
            y: 0.0,
            angle: 0.0,
        };
        assert_eq!(visible_subsector_order(&map, player), vec![1, 0]);
        map.nodes[0].child_bounds[0] = bounds(400.0, -20.0, 600.0, 20.0);
        assert_eq!(visible_subsector_order(&map, player), vec![0, 1]);
        map.nodes[0].children = [0, 0];
        map.nodes[0].child_bounds = [bounds(-10.0, -10.0, 10.0, 10.0); 2];
        assert!(visible_subsector_order(&map, player).is_empty());
        map.nodes[0].children = [0x8000, 0x8000];
        assert_eq!(visible_subsector_order(&map, player), vec![0]);
    }

    #[test]
    fn visible_mesh_bounds_recover_leaves_culled_by_bad_bsp_bounds() {
        let bounds = |min_x, min_y, max_x, max_y| Bounds2 {
            min_x,
            min_y,
            max_x,
            max_y,
        };
        let map = Map {
            vertices: vec![],
            sectors: vec![],
            sides: vec![],
            lines: vec![],
            segs: vec![],
            subsectors: vec![[0, 0], [0, 0]],
            nodes: vec![Node {
                x: 0,
                y: 0,
                dx: 0,
                dy: 1,
                child_bounds: [
                    bounds(-300.0, -20.0, -100.0, 20.0),
                    bounds(100.0, -20.0, 300.0, 20.0),
                ],
                children: [0x8000, 0x8001],
            }],
            things: vec![],
        };
        let player = Player {
            x: -1.0,
            y: 0.0,
            angle: 0.0,
        };
        let texture = Arc::new(Texture::new(1, 1, TextureFormat::Rgba8, &[0, 0, 0, 255]).unwrap());
        let draws = vec![
            vec![Draw {
                name: [0; 8],
                wall: false,
                masked: false,
                texture,
                vertices: vec![],
                bounds: Some(Bounds3 {
                    min: Vec3::new(30.0, 0.0, -4.0),
                    max: Vec3::new(80.0, 10.0, 4.0),
                }),
            }],
            vec![],
        ];

        assert_eq!(visible_subsector_order(&map, player), vec![1]);
        let fallback = visibility_fallback_bounds(&map, &draws);
        assert_eq!(fallback.len(), 1);
        assert_eq!(fallback[0].0, 0);
        assert_eq!(
            (
                fallback[0].1.min_x,
                fallback[0].1.min_y,
                fallback[0].1.max_x,
                fallback[0].1.max_y,
            ),
            (30.0, -4.0, 80.0, 4.0)
        );
        assert_eq!(visible_geometry_order(&map, player, &fallback), vec![1, 0]);
    }

    #[test]
    fn view_bounds_follow_all_cardinal_camera_headings() {
        for angle in [0.0_f32, 90.0, 180.0, 270.0] {
            let player = Player {
                x: 0.0,
                y: 0.0,
                angle,
            };
            let radians = angle.to_radians();
            let forward = Vertex2 {
                x: radians.cos() * 120.0,
                y: radians.sin() * 120.0,
            };
            let bounds = |center: Vertex2| Bounds2 {
                min_x: center.x - 12.0,
                min_y: center.y - 12.0,
                max_x: center.x + 12.0,
                max_y: center.y + 12.0,
            };
            assert!(bounds_in_view(bounds(forward), player), "angle {angle}");
            assert!(
                !bounds_in_view(
                    bounds(Vertex2 {
                        x: -forward.x,
                        y: -forward.y,
                    }),
                    player,
                ),
                "angle {angle}"
            );
        }
    }

    #[test]
    fn geometry_bounds_reject_vertical_near_and_far_frustum_exits() {
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        let around = |x, y, z, half| Bounds3 {
            min: Vec3::new(x - half, y - half, z - half),
            max: Vec3::new(x + half, y + half, z + half),
        };
        assert!(bounds3_in_view(around(120.0, 41.0, 0.0, 8.0), player, 41.0));
        assert!(!bounds3_in_view(
            around(120.0, 150.0, 0.0, 5.0),
            player,
            41.0
        ));
        assert!(!bounds3_in_view(
            around(120.0, -70.0, 0.0, 5.0),
            player,
            41.0
        ));
        assert!(!bounds3_in_view(around(0.0, 41.0, 0.0, 0.1), player, 41.0));
        assert!(!bounds3_in_view(
            around(9000.0, 41.0, 0.0, 10.0),
            player,
            41.0
        ));
        for angle in [90.0_f32, 180.0, 270.0] {
            let player = Player { angle, ..player };
            let radians = angle.to_radians();
            let x = radians.cos() * 120.0;
            let z = -radians.sin() * 120.0;
            assert!(bounds3_in_view(around(x, 41.0, z, 8.0), player, 41.0));
        }
    }

    #[test]
    fn hitscans_damage_an_exposed_enemy_but_stop_at_a_blocking_line() {
        let mut actors = [Actor {
            sprite: *b"TROO",
            x: 100.0,
            y: 0.0,
            health: 20,
            target_time_remaining: 0.0,
            attack_cooldown: 0.0,
            attack_animation_remaining: 0.0,
            pain_animation_remaining: 0.0,
            death_animation_time: None,
            animation_time: 0.0,
            angle: 0.0,
        }];
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        let mut pain_rng = 1;
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
        let mut wounded = actors;
        wounded[0].health = 60;
        assert!(!fire_weapon(&map, &mut wounded, player, &mut pain_rng));
        assert_eq!(wounded[0].health, 40);
        assert_eq!(wounded[0].target_time_remaining, ACTOR_TARGET_THRESHOLD);
        assert_eq!(
            wounded[0].pain_animation_remaining,
            actor_pain_duration(*b"TROO")
        );
        let mut projectiles = Vec::new();
        let mut health = 100;
        update_actors(
            &map,
            &mut wounded,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert!((wounded[0].pain_animation_remaining - (4.0 / 35.0 - 0.05)).abs() < 0.0001);
        assert_eq!(wounded[0].x, 100.0);

        let mut unreacting = actors;
        unreacting[0].health = 60;
        let mut no_pain_rng = 12_800;
        assert!(!fire_weapon(
            &map,
            &mut unreacting,
            player,
            &mut no_pain_rng
        ));
        assert_eq!(unreacting[0].health, 40);
        assert_eq!(unreacting[0].pain_animation_remaining, 0.0);

        assert!(fire_weapon(&map, &mut actors, player, &mut pain_rng));
        assert_eq!(actors[0].health, 0);
        assert_eq!(actors[0].death_animation_time, Some(0.0));
        let mut projectiles = Vec::new();
        let mut health = 100;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.1,
        );
        assert_eq!(actors[0].death_animation_time, Some(0.1));
        health = 0;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.1,
        );
        assert_eq!(health, 0);
        assert_eq!(actors[0].death_animation_time, Some(0.2));
        actors[0].health = 20;
        actors[0].death_animation_time = None;
        actors[0].attack_animation_remaining = 0.1;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert_eq!(health, 0);
        assert_eq!(actors[0].attack_animation_remaining, 0.05);

        let map = Map {
            vertices: vec![Vertex2 { x: 50.0, y: -32.0 }, Vertex2 { x: 50.0, y: 32.0 }],
            lines: vec![[0, 1, 1, u16::MAX, u16::MAX, 0, 0]],
            ..map
        };
        assert!(!fire_weapon(&map, &mut actors, player, &mut pain_rng));
        assert_eq!(actors[0].health, 20);
    }

    #[test]
    fn enemy_pain_reactions_match_doom_frames_tics_and_chance_thresholds() {
        for (sprite, frame, tics, chance) in [
            (*b"TROO", 7, 4.0, 200),
            (*b"SARG", 7, 4.0, 180),
            (*b"POSS", 6, 6.0, 200),
            (*b"SPOS", 6, 6.0, 170),
        ] {
            let duration = tics / 35.0;
            assert_eq!(actor_pain_duration(sprite), duration);
            assert_eq!(actor_pain_frame(sprite, duration), Some(frame));
            assert_eq!(actor_pain_frame(sprite, 0.0), None);
            assert!(actor_pain_triggered(sprite, chance - 1));
            assert!(!actor_pain_triggered(sprite, chance));
        }
        assert_eq!(actor_pain_duration(*b"none"), 0.0);
        assert!(!actor_pain_triggered(*b"none", 0));
        let mut random = 1;
        assert_eq!(gameplay_random_byte(&mut random), 0);
    }

    #[test]
    fn enemies_chase_and_melee_on_a_cooldown() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
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
            lines: vec![[0, 1, 1, 0, u16::MAX, 0, 0]],
            segs: vec![[0, 1, 0, 0, 0]],
            subsectors: vec![[1, 0]],
            nodes: vec![],
            things: vec![],
        };
        let mut actors = [Actor {
            sprite: *b"SARG",
            x: 100.0,
            y: 0.0,
            health: 60,
            target_time_remaining: 0.0,
            attack_cooldown: 0.0,
            attack_animation_remaining: 0.0,
            pain_animation_remaining: 0.0,
            death_animation_time: None,
            animation_time: 0.0,
            angle: 0.0,
        }];
        let mut projectiles = Vec::new();
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        let mut health = 100;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            1.0,
        );
        assert_eq!(actors[0].x, 64.0);
        assert_eq!(health, 100);
        assert_eq!(actors[0].angle, 180.0);
        assert!(actors[0].animation_time > 0.0);

        actors[0].x = 40.0;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert_eq!(actors[0].animation_time, 0.0);
        assert_eq!(
            actors[0].attack_animation_remaining,
            actor_attack_duration(*b"SARG")
        );
        assert_eq!(
            actor_attack_frame(*b"SARG", actors[0].attack_animation_remaining),
            Some(4)
        );
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert!(actors[0].attack_animation_remaining < actor_attack_duration(*b"SARG"));
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.79,
        );
        assert_eq!(health, 92);
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.02,
        );
        assert_eq!(health, 84);
    }

    #[test]
    fn ranged_enemies_attack_only_with_clear_sight_and_respect_cooldown() {
        let mut map = Map {
            vertices: vec![Vertex2 { x: 50.0, y: -32.0 }, Vertex2 { x: 50.0, y: 32.0 }],
            sectors: vec![],
            sides: vec![],
            lines: vec![[0, 1, 1, u16::MAX, u16::MAX, 0, 0]],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        let mut actors = [Actor {
            sprite: *b"POSS",
            x: 100.0,
            y: 0.0,
            health: 20,
            target_time_remaining: 0.0,
            attack_cooldown: 0.0,
            attack_animation_remaining: 0.0,
            pain_animation_remaining: 0.0,
            death_animation_time: None,
            animation_time: 0.0,
            angle: 0.0,
        }];
        let mut projectiles = Vec::new();
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        let mut health = 100;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert_eq!(health, 100);

        map.lines.clear();
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert_eq!(health, 97);
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.2,
        );
        assert_eq!(health, 97);

        actors[0].sprite = *b"SPOS";
        actors[0].attack_cooldown = 0.0;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert_eq!(health, 91);
    }

    #[test]
    fn imps_launch_visible_fireballs_that_hit_and_stop_at_walls() {
        let sector = Sector {
            floor: 0.0,
            ceiling: 128.0,
            special: 0,
            light: 255,
            floor_flat: [0; 8],
            ceiling_flat: [0; 8],
            tag: 0,
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
        let mut map = Map {
            vertices: vec![
                Vertex2 { x: 10.0, y: -32.0 },
                Vertex2 { x: 10.0, y: 32.0 },
                Vertex2 { x: 90.0, y: -32.0 },
                Vertex2 { x: 90.0, y: 32.0 },
                Vertex2 { x: 50.0, y: -32.0 },
                Vertex2 { x: 50.0, y: 32.0 },
            ],
            sectors: vec![sector, raised],
            sides: vec![side(0), side(1)],
            lines: vec![
                [0, 1, 0, 0, 1, 0, 0],
                [2, 3, 0, 1, 0, 0, 0],
                [4, 5, 1, 0, 1, 0, 0],
            ],
            segs: vec![[0, 1, 0, 0, 0], [2, 3, 1, 0, 0]],
            subsectors: vec![[1, 0], [1, 1]],
            nodes: vec![Node {
                x: 50,
                y: 0,
                dx: 0,
                dy: 1,
                child_bounds: [Bounds2::default(); 2],
                children: [0x8001, 0x8000],
            }],
            things: vec![],
        };
        let mut actors = [Actor {
            sprite: *b"TROO",
            x: 100.0,
            y: 0.0,
            health: 60,
            target_time_remaining: 0.0,
            attack_cooldown: 0.0,
            attack_animation_remaining: 0.0,
            pain_animation_remaining: 0.0,
            death_animation_time: None,
            animation_time: 0.0,
            angle: 0.0,
        }];
        let mut projectiles = Vec::new();
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        let mut health = 100;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert!(projectiles.is_empty());

        map.lines[2][2] = 0;
        update_actors(
            &map,
            &mut actors,
            &mut projectiles,
            player,
            &mut health,
            0.05,
        );
        assert_eq!(projectiles.len(), 1);
        assert_eq!(projectiles[0].z, 60.0);
        assert!(projectiles[0].velocity_z < 0.0);
        update_projectiles(&map, &mut projectiles, player, &mut health, 0.4);
        assert_eq!(health, 100);
        assert!(projectiles[0].z < 60.0);
        update_projectiles(&map, &mut projectiles, player, &mut health, 0.1);
        assert_eq!(health, 92);
        assert_eq!(projectiles.len(), 1);
        assert_eq!(projectiles[0].explosion_time, Some(0.0));
        update_projectiles(&map, &mut projectiles, player, &mut health, 0.2);
        assert_eq!(health, 92);
        assert_eq!(
            projectile_explosion_frame(projectiles[0].explosion_time.unwrap()),
            Some(1)
        );
        update_projectiles(&map, &mut projectiles, player, &mut health, 0.4);
        assert!(projectiles.is_empty());

        projectiles.push(Projectile {
            x: 100.0,
            y: 0.0,
            z: 60.0,
            velocity_x: -180.0,
            velocity_y: 0.0,
            velocity_z: -55.0,
            lifetime: 3.0,
            explosion_time: None,
        });
        map.lines[2][2] = 1;
        update_projectiles(&map, &mut projectiles, player, &mut health, 0.3);
        assert_eq!(projectiles.len(), 1);
        assert_eq!(projectiles[0].explosion_time, Some(0.0));
        assert!((projectiles[0].x - 50.0).abs() < 0.01);
        assert!((projectiles[0].z - 44.722).abs() < 0.05);
        assert_eq!(health, 92);
        health = 0;
        update_projectiles(&map, &mut projectiles, player, &mut health, 0.2);
        assert_eq!(health, 0);
        assert_eq!(projectiles[0].explosion_time, Some(0.2));
    }

    #[test]
    fn health_ammo_and_blue_key_pickups_apply_caps_and_stay_when_unneeded() {
        assert_eq!(pickup_definition(5), Some((*b"BKEY", 0, 0, true)));
        let mut map = Map {
            vertices: vec![Vertex2 { x: 12.0, y: -32.0 }, Vertex2 { x: 12.0, y: 32.0 }],
            sectors: vec![],
            sides: vec![],
            lines: vec![],
            segs: vec![],
            subsectors: vec![],
            nodes: vec![],
            things: vec![],
        };
        let player = Player {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
        };
        let mut pickups = [
            Pickup {
                sprite: *b"STIM",
                x: 0.0,
                y: 0.0,
                health: 10,
                ammo: 0,
                blue_key: false,
                active: true,
            },
            Pickup {
                sprite: *b"CLIP",
                x: 0.0,
                y: 0.0,
                health: 0,
                ammo: 10,
                blue_key: false,
                active: true,
            },
            Pickup {
                sprite: *b"MEDI",
                x: 100.0,
                y: 0.0,
                health: 25,
                ammo: 0,
                blue_key: false,
                active: true,
            },
            Pickup {
                sprite: *b"BKEY",
                x: 0.0,
                y: 0.0,
                health: 0,
                ammo: 0,
                blue_key: true,
                active: true,
            },
        ];
        let mut health = 95;
        let mut ammo = 195;
        let mut blue_key = false;
        assert_eq!(
            collect_pickups(
                &map,
                &mut pickups,
                player,
                &mut health,
                &mut ammo,
                &mut blue_key
            ),
            3
        );
        assert_eq!((health, ammo), (100, 200));
        assert!(blue_key);
        assert!(!pickups[0].active && !pickups[1].active && pickups[2].active);
        assert!(!pickups[3].active);
        assert_eq!(
            collect_pickups(
                &map,
                &mut pickups,
                player,
                &mut health,
                &mut ammo,
                &mut blue_key
            ),
            0
        );
        pickups[2].x = 0.0;
        assert_eq!(
            collect_pickups(
                &map,
                &mut pickups,
                player,
                &mut health,
                &mut ammo,
                &mut blue_key
            ),
            0
        );
        assert!(pickups[2].active);

        map.lines.push([0, 1, 1, u16::MAX, u16::MAX, 0, 0]);
        let mut hidden = [Pickup {
            sprite: *b"STIM",
            x: 20.0,
            y: 0.0,
            health: 10,
            ammo: 0,
            blue_key: false,
            active: true,
        }];
        health = 80;
        assert_eq!(
            collect_pickups(
                &map,
                &mut hidden,
                player,
                &mut health,
                &mut ammo,
                &mut blue_key
            ),
            0
        );
        assert_eq!(health, 80);
        assert!(hidden[0].active);
    }
}
