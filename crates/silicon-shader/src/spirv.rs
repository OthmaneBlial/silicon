//! A deliberately strict SPIR-V 1.0 graphics subset, lowered into the SIR VM.
//! No guest pointers, compiler dependency, dynamic code, or host GPU execution.
use crate::{Instruction as Sir, Program, Result};
use silicon_math::Vec4;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
}
#[derive(Clone, Debug)]
pub struct Op {
    pub word: usize,
    pub opcode: u16,
    pub operands: Vec<u32>,
}
#[derive(Clone, Debug)]
pub struct Module {
    version: u32,
    bound: u32,
    instructions: Vec<Op>,
}
#[derive(Clone, Debug)]
pub struct Compiled {
    pub stage: Stage,
    pub program: Program,
    /// Location -> number of float components. Built-in Position is separate.
    pub inputs: BTreeMap<u8, u8>,
    pub outputs: BTreeMap<u8, u8>,
}
pub fn name(op: u16) -> &'static str {
    match op {
        0 => "OpNop",
        3 => "OpSource",
        5 => "OpName",
        6 => "OpMemberName",
        11 => "OpExtInstImport",
        12 => "OpExtInst",
        14 => "OpMemoryModel",
        15 => "OpEntryPoint",
        16 => "OpExecutionMode",
        17 => "OpCapability",
        19 => "OpTypeVoid",
        21 => "OpTypeInt",
        22 => "OpTypeFloat",
        23 => "OpTypeVector",
        24 => "OpTypeMatrix",
        25 => "OpTypeImage",
        27 => "OpTypeSampledImage",
        30 => "OpTypeStruct",
        32 => "OpTypePointer",
        33 => "OpTypeFunction",
        43 => "OpConstant",
        44 => "OpConstantComposite",
        54 => "OpFunction",
        56 => "OpFunctionEnd",
        59 => "OpVariable",
        61 => "OpLoad",
        62 => "OpStore",
        65 => "OpAccessChain",
        71 => "OpDecorate",
        72 => "OpMemberDecorate",
        79 => "OpVectorShuffle",
        80 => "OpCompositeConstruct",
        81 => "OpCompositeExtract",
        83 => "OpCopyObject",
        87 => "OpImageSampleImplicitLod",
        88 => "OpImageSampleExplicitLod",
        129 => "OpFAdd",
        131 => "OpFSub",
        133 => "OpFMul",
        136 => "OpFDiv",
        142 => "OpVectorTimesScalar",
        145 => "OpMatrixTimesVector",
        148 => "OpDot",
        245 => "OpPhi",
        247 => "OpSelectionMerge",
        248 => "OpLabel",
        249 => "OpBranch",
        250 => "OpBranchConditional",
        252 => "OpKill",
        253 => "OpReturn",
        _ => "unknown opcode",
    }
}
fn string(words: &[u32]) -> Result<(String, usize)> {
    let mut bytes = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let b = word.to_le_bytes();
        for (j, &c) in b.iter().enumerate() {
            if c == 0 {
                if b[j..].iter().any(|&v| v != 0) {
                    return Err("nonzero string padding".into());
                }
                return Ok((
                    String::from_utf8(bytes).map_err(|_| "invalid UTF-8 string")?,
                    i + 1,
                ));
            }
            bytes.push(c);
        }
    }
    Err("unterminated SPIR-V string".into())
}
impl Op {
    fn error(&self, message: impl std::fmt::Display) -> String {
        format!(
            "SPIR-V word {}: {} (opcode {}): {message}",
            self.word,
            name(self.opcode),
            self.opcode
        )
    }
    /// Operand layouts are from Khronos' core grammar; optional forms not supported are rejected.
    fn ids(&self) -> Result<(Option<u32>, Vec<u32>)> {
        let a = &self.operands;
        let n = a.len();
        let (min, max) = match self.opcode {
            0 | 56 | 253 => (0, 0),
            19 | 17 | 248 => (1, 1),
            14 | 16 | 22 => (2, 2),
            21 | 23 | 24 | 32 | 43 | 61 | 83 => (3, 3),
            59 => (3, 4),
            65 => (4, 5),
            12 => (5, 7),
            25 => (8, 8),
            27 => (2, 2),
            54 | 81 | 87 | 129 | 131 | 133 | 136 | 142 | 145 | 148 => (4, 4),
            62 => (2, 2),
            3 => (2, 2),
            5 | 11 => (2, usize::MAX),
            6 | 15 => (3, usize::MAX),
            30 => (2, usize::MAX),
            33 => (2, 2),
            44 | 80 => (3, 6),
            79 => (6, 8),
            71 => (2, 3),
            72 => (3, 4),
            _ => return Err(self.error("unsupported instruction in the straight-line subset")),
        };
        if !(min..=max).contains(&n) {
            return Err(self.error(format!("expected {min}..{max} operands, found {n}")));
        }
        let (result, refs) = match self.opcode {
            5 => {
                let (_, end) = string(&a[1..])?;
                if end + 1 != n {
                    return Err(self.error("trailing name operands"));
                }
                (None, vec![a[0]])
            }
            6 => {
                let (_, end) = string(&a[2..])?;
                if end + 2 != n {
                    return Err(self.error("trailing member name operands"));
                }
                (None, vec![a[0]])
            }
            11 => {
                let (_, end) = string(&a[1..])?;
                if end + 1 != n {
                    return Err(self.error("trailing import operands"));
                }
                (Some(a[0]), vec![])
            }
            15 => {
                let (_, end) = string(&a[2..])?;
                let mut r = vec![a[1]];
                r.extend_from_slice(&a[end + 2..]);
                (None, r)
            }
            16 | 71 | 72 => (None, vec![a[0]]),
            19 | 21 | 22 | 248 => (Some(a[0]), vec![]),
            23 | 24 | 27 | 33 => (Some(a[0]), vec![a[1]]),
            25 => (Some(a[0]), vec![a[1]]),
            30 => (Some(a[0]), a[1..].to_vec()),
            32 => (Some(a[0]), vec![a[2]]),
            43 => (Some(a[1]), vec![a[0]]),
            59 => (
                Some(a[1]),
                std::iter::once(a[0])
                    .chain(a[3..].iter().copied())
                    .collect(),
            ),
            12 => (
                Some(a[1]),
                [a[0], a[2]]
                    .into_iter()
                    .chain(a[4..].iter().copied())
                    .collect(),
            ),
            54 => (Some(a[1]), vec![a[0], a[3]]),
            44 | 80 | 65 => {
                let mut r = vec![a[0]];
                r.extend_from_slice(&a[2..]);
                (Some(a[1]), r)
            }
            61 | 81 | 83 => (Some(a[1]), vec![a[0], a[2]]),
            62 => (None, a.clone()),
            79 | 87 | 129 | 131 | 133 | 136 | 142 | 145 | 148 => {
                (Some(a[1]), vec![a[0], a[2], a[3]])
            }
            _ => (None, vec![]),
        };
        Ok((result, refs))
    }
}
impl Module {
    /// Checks bounded binary framing, instruction shapes, unique IDs and all references.
    /// `translate` additionally checks types, decorations, entry point and function semantics.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 20 || bytes.len() > 1024 * 1024 || !bytes.len().is_multiple_of(4) {
            return Err("SPIR-V requires 20 bytes..1 MiB, aligned to four bytes".into());
        }
        let mut words: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        if words[0] == 0x03022307 {
            words.iter_mut().for_each(|w| *w = w.swap_bytes());
        }
        if words[0] != 0x07230203 {
            return Err("invalid SPIR-V magic".into());
        }
        if words[1] != 0x00010000 {
            return Err(format!(
                "SPIR-V version {:#x} unsupported; this subset accepts 1.0",
                words[1]
            ));
        }
        if !(1..=65536).contains(&words[3]) || words[4] != 0 {
            return Err("invalid SPIR-V ID bound or nonzero schema".into());
        }
        let mut defined = BTreeSet::new();
        let mut refs = Vec::new();
        let mut instructions = Vec::new();
        let mut i = 5;
        while i < words.len() {
            let count = (words[i] >> 16) as usize;
            if count == 0 || count > words.len() - i {
                return Err(format!(
                    "SPIR-V word {i}: invalid instruction word count {count}"
                ));
            }
            let op = Op {
                word: i,
                opcode: words[i] as u16,
                operands: words[i + 1..i + count].to_vec(),
            };
            let (result, used) = op.ids().map_err(|e| {
                if e.starts_with("SPIR-V word") {
                    e
                } else {
                    op.error(e)
                }
            })?;
            for id in used.iter().copied().chain(result) {
                if id == 0 || id >= words[3] {
                    return Err(op.error(format!("ID %{id} outside bound {}", words[3])));
                }
            }
            if let Some(id) = result
                && !defined.insert(id)
            {
                return Err(op.error(format!("duplicate ID %{id}")));
            }
            refs.extend(used.into_iter().map(|id| (id, i)));
            instructions.push(op);
            i += count;
        }
        for (id, word) in refs {
            if !defined.contains(&id) {
                return Err(format!("SPIR-V word {word}: undefined ID %{id}"));
            }
        }
        Ok(Self {
            version: words[1],
            bound: words[3],
            instructions,
        })
    }
    pub fn translate(&self) -> Result<Compiled> {
        Compiler::new(self)?.run()
    }
    pub fn version(&self) -> u32 {
        self.version
    }
    pub fn bound(&self) -> u32 {
        self.bound
    }
    pub fn instructions(&self) -> &[Op] {
        &self.instructions
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Ty {
    Void,
    Int,
    Float,
    Vector(u8),
    Matrix,
    Image,
    Sampled,
    Struct(Vec<u32>),
    Pointer(u32, u32),
    Function(u32),
}
#[derive(Clone, Debug)]
enum Value {
    Reg(u8, bool),
    Int(u32),
    Pointer { root: u32, path: Vec<usize> },
    Matrix(u8),
    Texture(u8),
}
#[derive(Clone, Debug)]
struct Typed {
    ty: u32,
    value: Value,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Decoration {
    block: bool,
    location: Option<u32>,
    binding: Option<u32>,
    set: Option<u32>,
    builtin: Option<u32>,
    col_major: bool,
    stride: Option<u32>,
    offset: Option<u32>,
}
struct Compiler<'a> {
    module: &'a Module,
    stage: Stage,
    entry: u32,
    interfaces: BTreeSet<u32>,
    types: BTreeMap<u32, Ty>,
    values: BTreeMap<u32, Typed>,
    locals: BTreeMap<u32, Typed>,
    decorations: BTreeMap<(u32, Option<usize>), Decoration>,
    ops: Vec<Sir>,
    next: u16,
    inputs: BTreeMap<u8, u8>,
    outputs: BTreeMap<u8, u8>,
    written: BTreeSet<u8>,
}
impl<'a> Compiler<'a> {
    fn new(module: &'a Module) -> Result<Self> {
        let mut entry = None;
        let mut memory = false;
        let mut capability = false;
        let mut origin = false;
        let mut decorations = BTreeMap::new();
        for op in &module.instructions {
            let a = &op.operands;
            match op.opcode {
                15 => {
                    let stage = match a[0] {
                        0 => Stage::Vertex,
                        4 => Stage::Fragment,
                        _ => {
                            return Err(
                                op.error("only Vertex and Fragment entry points are supported")
                            );
                        }
                    };
                    let (s, n) = string(&a[2..])?;
                    if s != "main" || entry.is_some() {
                        return Err(op.error("requires exactly one entry point named main"));
                    }
                    let interfaces: BTreeSet<_> = a[n + 2..].iter().copied().collect();
                    if interfaces.len() != a.len() - n - 2 {
                        return Err(op.error("duplicate entry interface"));
                    }
                    entry = Some((stage, a[1], interfaces));
                }
                14 => {
                    if memory || a.as_slice() != [0, 1] {
                        return Err(op.error("requires one Logical GLSL450 memory model"));
                    }
                    memory = true;
                }
                17 => {
                    if capability || a[0] != 1 {
                        return Err(op.error("only the Shader capability is supported"));
                    }
                    capability = true;
                }
                16 => {
                    if origin || a[1] != 7 {
                        return Err(op.error("only OriginUpperLeft execution mode is supported"));
                    }
                    origin = true;
                }
                11 if string(&a[1..])?.0 != "GLSL.std.450" => {
                    return Err(op.error("unsupported extended instruction import"));
                }
                71 | 72 => {
                    let member = (op.opcode == 72).then(|| a[1] as usize);
                    let start = if member.is_some() { 2 } else { 1 };
                    let d: &mut Decoration = decorations.entry((a[0], member)).or_default();
                    let v = if a.len() == start + 2 {
                        Some(a[start + 1])
                    } else {
                        None
                    };
                    let set = |field: &mut Option<u32>| -> Result<()> {
                        if field.is_some() || v.is_none() {
                            return Err(op.error("duplicate decoration or missing operand"));
                        }
                        *field = v;
                        Ok(())
                    };
                    match a[start] {
                        2 if member.is_none() && v.is_none() && !d.block => d.block = true,
                        5 if member.is_some() && v.is_none() && !d.col_major => d.col_major = true,
                        7 if member.is_some() => set(&mut d.stride)?,
                        11 => set(&mut d.builtin)?,
                        30 if member.is_none() => set(&mut d.location)?,
                        33 if member.is_none() => set(&mut d.binding)?,
                        34 if member.is_none() => set(&mut d.set)?,
                        35 if member.is_some() => set(&mut d.offset)?,
                        _ => {
                            return Err(op.error(format!(
                                "unsupported or duplicate decoration {}",
                                a[start]
                            )));
                        }
                    }
                }
                _ => {}
            }
        }
        let (stage, entry, interfaces) = entry.ok_or("SPIR-V missing main entry point")?;
        if !memory || !capability || origin != (stage == Stage::Fragment) {
            return Err(
                "SPIR-V missing memory model, Shader capability, or required fragment origin mode"
                    .into(),
            );
        }
        for op in &module.instructions {
            if op.opcode == 16 && op.operands[0] != entry {
                return Err(op.error("execution mode targets another function"));
            }
        }
        Ok(Self {
            module,
            stage,
            entry,
            interfaces,
            types: BTreeMap::new(),
            values: BTreeMap::new(),
            locals: BTreeMap::new(),
            decorations,
            ops: vec![Sir::Const {
                dst: 0,
                value: Vec4::ZERO,
            }],
            next: 1,
            inputs: BTreeMap::new(),
            outputs: BTreeMap::new(),
            written: BTreeSet::new(),
        })
    }
    fn ty(&self, id: u32) -> Result<Ty> {
        self.types
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("missing type %{id}"))
    }
    fn lanes(&self, id: u32) -> Result<u8> {
        match self.ty(id)? {
            Ty::Float => Ok(1),
            Ty::Vector(n) => Ok(n),
            _ => Err(format!("type %{id} is not float32 or a float vector")),
        }
    }
    fn value(&self, id: u32) -> Result<Typed> {
        self.values
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("value %{id} is undefined or used before definition"))
    }
    fn reg(&self, id: u32) -> Result<(u8, u32, bool)> {
        let v = self.value(id)?;
        match v.value {
            Value::Reg(r, uv) => Ok((r, v.ty, uv)),
            _ => Err(format!("value %{id} is not a float register")),
        }
    }
    fn emit(&mut self, f: impl FnOnce(u8) -> Sir) -> Result<u8> {
        // ponytail: bounded virtual SSA IDs; widen only when a real shader needs more.
        if self.next >= 256 {
            return Err("SPIR-V lowering exceeds 256 virtual registers".into());
        }
        let r = self.next as u8;
        self.next += 1;
        self.ops.push(f(r));
        Ok(r)
    }
    fn decoration(&self, id: u32, member: Option<usize>) -> Decoration {
        self.decorations
            .get(&(id, member))
            .cloned()
            .unwrap_or_default()
    }
    fn compose(&mut self, components: &[(u8, u8)], n: u8) -> Result<u8> {
        if components.len() != n as usize {
            return Err("composite component count/type mismatch".into());
        }
        let sources = std::array::from_fn(|i| components.get(i).map_or(0, |v| v.0));
        let lanes = std::array::from_fn(|i| components.get(i).map_or(0, |v| v.1));
        self.emit(|dst| Sir::Compose {
            dst,
            sources,
            lanes,
        })
    }
    fn pointer(&self, id: u32) -> Result<(u32, Vec<usize>, u32, u32)> {
        let v = self.value(id)?;
        let Ty::Pointer(storage, base) = self.ty(v.ty)? else {
            return Err("expected pointer type".into());
        };
        let Value::Pointer { root, path } = v.value else {
            return Err("expected logical variable pointer".into());
        };
        Ok((root, path, storage, base))
    }
    fn slot(&self, root: u32, member: Option<usize>) -> Result<u8> {
        let (_, _, _, base) = self.pointer(root)?;
        let d = self.decoration(root, None);
        if let Some(member) = member {
            if self.stage != Stage::Vertex || self.decoration(base, Some(member)).builtin != Some(0)
            {
                return Err("only vertex BuiltIn Position can be stored through a block".into());
            }
            return Ok(0);
        }
        if d.builtin == Some(0) && self.stage == Stage::Vertex {
            return Ok(0);
        }
        let loc = d.location.ok_or("output requires Location or Position")? as u8;
        Ok(loc + u8::from(self.stage == Stage::Vertex))
    }
    fn variable(&mut self, a: &[u32]) -> Result<()> {
        let Ty::Pointer(storage, base) = self.ty(a[0])? else {
            return Err("OpVariable result must be a pointer".into());
        };
        if storage != a[2] {
            return Err("variable/pointer storage class mismatch".into());
        }
        let d = self.decoration(a[1], None);
        if d.block || d.col_major || d.stride.is_some() || d.offset.is_some() {
            return Err("block/member decorations cannot target variables".into());
        }
        if [0, 2].contains(&storage) && (d.location.is_some() || d.builtin.is_some()) {
            return Err("resource variables cannot carry Location or BuiltIn".into());
        }
        if [1, 3].contains(&storage) && (d.binding.is_some() || d.set.is_some()) {
            return Err("interface variables cannot carry descriptor decorations".into());
        }
        let t = self.ty(base)?;
        if a.len() == 4 && storage != 7 {
            return Err("only local variables may have an initializer".into());
        }
        match storage {
            0 => {
                if t != Ty::Sampled || d.set != Some(1) || d.binding.is_none_or(|b| b > 15) {
                    return Err(
                        "samplers require sampler2D, descriptor set 1, binding 0..15".into(),
                    );
                }
            }
            1 => {
                let loc = d.location.ok_or("input requires Location")?;
                let n = self.lanes(base)?;
                if d.builtin.is_some() || loc > 3 || !self.interfaces.contains(&a[1]) {
                    return Err("input outside locations 0..3 or entry interface".into());
                }
                if self.stage == Stage::Vertex && n != [3, 4, 2, 3][loc as usize] {
                    return Err("vertex inputs require position:vec3, color:vec4, UV:vec2, normal:vec3 at locations 0..3".into());
                }
                if self.inputs.insert(loc as u8, n).is_some() {
                    return Err("duplicate input location".into());
                }
            }
            2 => {
                let Ty::Struct(m) = t else {
                    return Err("uniform requires a one-member float/vector/mat4 block".into());
                };
                let md = self.decoration(base, Some(0));
                if m.len() != 1
                    || !self.decoration(base, None).block
                    || d.set != Some(0)
                    || d.binding.is_none_or(|b| b > 15)
                    || md != self.uniform_layout(m[0])?
                {
                    return Err("uniform requires offset 0, set 0, binding 0..15; mat4 requires col-major stride 16".into());
                }
            }
            3 => {
                if !self.interfaces.contains(&a[1]) {
                    return Err("output missing from entry interface".into());
                }
                match t {
                    Ty::Struct(m) if self.stage == Stage::Vertex => {
                        if m.len() != 1
                            || self.lanes(m[0])? != 4
                            || !self.decoration(base, None).block
                            || self.decoration(base, Some(0))
                                != (Decoration {
                                    builtin: Some(0),
                                    ..Default::default()
                                })
                            || d.location.is_some()
                            || d.builtin.is_some()
                        {
                            return Err("output block supports only gl_Position:vec4".into());
                        }
                    }
                    _ => {
                        let n = self.lanes(base)?;
                        if d.builtin == Some(0)
                            && self.stage == Stage::Vertex
                            && n == 4
                            && d.location.is_none()
                        {
                        } else {
                            let loc = d.location.ok_or("output requires Location")?;
                            if d.builtin.is_some()
                                || loc > 3
                                || (self.stage == Stage::Fragment && (loc != 0 || n != 4))
                            {
                                return Err("output location/type unsupported".into());
                            }
                            if self.outputs.insert(loc as u8, n).is_some() {
                                return Err("duplicate output location".into());
                            }
                        }
                    }
                }
            }
            7 => {
                self.lanes(base)?;
                if d != Decoration::default() {
                    return Err("local variables cannot have resource/interface decorations".into());
                }
                if a.len() == 4 {
                    let v = self.value(a[3])?;
                    if v.ty != base || !matches!(v.value, Value::Reg(..)) {
                        return Err("local initializer type mismatch".into());
                    }
                    self.locals.insert(a[1], v);
                }
            }
            _ => return Err(format!("unsupported storage class {storage}")),
        }
        self.values.insert(
            a[1],
            Typed {
                ty: a[0],
                value: Value::Pointer {
                    root: a[1],
                    path: vec![],
                },
            },
        );
        Ok(())
    }
    fn uniform_layout(&self, ty: u32) -> Result<Decoration> {
        match self.ty(ty)? {
            Ty::Matrix => Ok(Decoration {
                col_major: true,
                stride: Some(16),
                offset: Some(0),
                ..Default::default()
            }),
            Ty::Float | Ty::Vector(_) => Ok(Decoration {
                offset: Some(0),
                ..Default::default()
            }),
            _ => Err("unsupported uniform member type".into()),
        }
    }
    fn canonical(&mut self, r: u8, n: u8) -> Result<u8> {
        if n == 1 {
            self.emit(|dst| Sir::Swizzle {
                dst,
                src: r,
                lanes: [0; 4],
            })
        } else if n < 4 {
            self.compose(&(0..n).map(|i| (r, i)).collect::<Vec<_>>(), n)
        } else {
            Ok(r)
        }
    }
    fn instruction(&mut self, op: &Op) -> Result<()> {
        let a = &op.operands;
        match op.opcode {
            19 => {
                self.types.insert(a[0], Ty::Void);
            }
            21 => {
                if a[1] != 32 || a[2] > 1 {
                    return Err(
                        "only 32-bit integer constants for access indices are supported".into(),
                    );
                }
                self.types.insert(a[0], Ty::Int);
            }
            22 => {
                if a[1] != 32 {
                    return Err("only float32 is supported".into());
                }
                self.types.insert(a[0], Ty::Float);
            }
            23 => {
                if self.ty(a[1])? != Ty::Float || !(2..=4).contains(&a[2]) {
                    return Err("requires vec2/vec3/vec4 of float32".into());
                }
                self.types.insert(a[0], Ty::Vector(a[2] as u8));
            }
            24 => {
                if self.ty(a[1])? != Ty::Vector(4) || a[2] != 4 {
                    return Err("only mat4 is supported".into());
                }
                self.types.insert(a[0], Ty::Matrix);
            }
            25 => {
                if self.ty(a[1])? != Ty::Float || a[2..] != [1, 0, 0, 0, 1, 0] {
                    return Err(
                        "only sampled float32 2D non-array/non-depth/non-MS images are supported"
                            .into(),
                    );
                }
                self.types.insert(a[0], Ty::Image);
            }
            27 => {
                if self.ty(a[1])? != Ty::Image {
                    return Err("sampled image requires supported image type".into());
                }
                self.types.insert(a[0], Ty::Sampled);
            }
            30 => {
                if a.len() != 2 || !matches!(self.ty(a[1])?, Ty::Float | Ty::Vector(_) | Ty::Matrix)
                {
                    return Err("struct supports one float/vector/mat4 member".into());
                }
                self.types.insert(a[0], Ty::Struct(a[1..].to_vec()));
            }
            32 => {
                self.ty(a[2])?;
                if ![0, 1, 2, 3, 7].contains(&a[1]) {
                    return Err("unsupported pointer storage class".into());
                }
                self.types.insert(a[0], Ty::Pointer(a[1], a[2]));
            }
            33 => {
                if self.ty(a[1])? != Ty::Void {
                    return Err("only void() functions are supported".into());
                }
                self.types.insert(a[0], Ty::Function(a[1]));
            }
            43 => {
                let value = match self.ty(a[0])? {
                    Ty::Int => Value::Int(a[2]),
                    Ty::Float => {
                        let f = f32::from_bits(a[2]);
                        if !f.is_finite() {
                            return Err("non-finite constant".into());
                        }
                        Value::Reg(
                            self.emit(|dst| Sir::Const {
                                dst,
                                value: Vec4::new(f, f, f, f),
                            })?,
                            false,
                        )
                    }
                    _ => return Err("constant must be float32 or int32".into()),
                };
                self.values.insert(a[1], Typed { ty: a[0], value });
            }
            59 => self.variable(a)?,
            65 => {
                let (root, mut path, storage, mut base) = self.pointer(a[2])?;
                let Ty::Pointer(result_storage, target) = self.ty(a[0])? else {
                    return Err("access chain result must be a pointer".into());
                };
                for &id in &a[3..] {
                    let Value::Int(index) = self.value(id)?.value else {
                        return Err("access chain index requires an integer constant".into());
                    };
                    base = match self.ty(base)? {
                        Ty::Struct(m) if [2, 3].contains(&storage) => *m.get(index as usize).ok_or("struct member index out of bounds")?,
                        Ty::Vector(n) if [2, 7].contains(&storage) && index < u32::from(n) && self.ty(target)? == Ty::Float => target,
                        _ => return Err("access chain supports uniform members and uniform/local vector components".into()),
                    };
                    path.push(index as usize);
                }
                if path.len() > 2 || storage != result_storage || base != target {
                    return Err("access chain result pointer type/storage mismatch".into());
                }
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Pointer { root, path },
                    },
                );
            }
            61 => {
                let (root, path, storage, base) = self.pointer(a[2])?;
                if a[0] != base {
                    return Err("load result/pointee type mismatch".into());
                }
                let d = self.decoration(root, None);
                let value =
                    match storage {
                        0 if path.is_empty() => {
                            Value::Texture(d.binding.ok_or("sampler lacks binding")? as u8)
                        }
                        1 if path.is_empty() => {
                            let slot = d.location.ok_or("input lacks location")? as u8;
                            let n = self.lanes(base)?;
                            let r = self.emit(|dst| Sir::Input { dst, slot })?;
                            let r = self.canonical(r, n)?;
                            Value::Reg(r, self.stage == Stage::Fragment && slot == 1 && n == 2)
                        }
                        2 if path.first() == Some(&0) => {
                            let uniform = d.binding.ok_or("uniform lacks binding")? as u8 * 4;
                            if self.ty(base)? == Ty::Matrix && path.len() == 1 {
                                Value::Matrix(uniform)
                            } else {
                                let r = self.emit(|dst| Sir::Uniform { dst, slot: uniform })?;
                                let r = if path.len() == 2 {
                                    let lane = path[1] as u8;
                                    self.emit(|dst| Sir::Swizzle {
                                        dst,
                                        src: r,
                                        lanes: [lane; 4],
                                    })?
                                } else {
                                    self.canonical(r, self.lanes(base)?)?
                                };
                                Value::Reg(r, false)
                            }
                        }
                        7 => {
                            let local = self
                                .locals
                                .get(&root)
                                .cloned()
                                .ok_or("local variable loaded before initialization")?;
                            let Value::Reg(r, uv) = local.value else {
                                unreachable!()
                            };
                            if path.is_empty() {
                                Value::Reg(r, uv)
                            } else if path.len() == 1 {
                                let lane = path[0] as u8;
                                Value::Reg(
                                    self.emit(|dst| Sir::Swizzle {
                                        dst,
                                        src: r,
                                        lanes: [lane; 4],
                                    })?,
                                    false,
                                )
                            } else {
                                return Err("nested local access unsupported".into());
                            }
                        }
                        _ => return Err(
                            "load supports inputs, sampler/uniform bindings and local variables"
                                .into(),
                        ),
                    };
                self.values.insert(a[1], Typed { ty: a[0], value });
            }
            62 => {
                let (root, path, storage, base) = self.pointer(a[0])?;
                let (r, t, uv) = self.reg(a[1])?;
                if t != base {
                    return Err("store value/pointee type mismatch".into());
                }
                match storage {
                    3 if path.len() <= 1 => {
                        let slot = self.slot(root, path.first().copied())?;
                        self.ops.push(Sir::Output { slot, src: r });
                        self.written.insert(slot);
                    }
                    7 if path.is_empty() => {
                        self.locals.insert(
                            root,
                            Typed {
                                ty: t,
                                value: Value::Reg(r, uv),
                            },
                        );
                    }
                    7 if path.len() == 1 => {
                        let old = self
                            .locals
                            .get(&root)
                            .cloned()
                            .ok_or("component store requires an initialized local vector")?;
                        let Value::Reg(previous, _) = old.value else {
                            unreachable!()
                        };
                        let n = self.lanes(old.ty)?;
                        let components: Vec<_> = (0..n)
                            .map(|i| {
                                if i as usize == path[0] {
                                    (r, 0)
                                } else {
                                    (previous, i)
                                }
                            })
                            .collect();
                        let r = self.compose(&components, n)?;
                        self.locals.insert(
                            root,
                            Typed {
                                ty: old.ty,
                                value: Value::Reg(r, false),
                            },
                        );
                    }
                    _ => {
                        return Err(
                            "store supports matching outputs and local float/vector variables"
                                .into(),
                        );
                    }
                }
            }
            44 | 80 => {
                let n = self.lanes(a[0])?;
                if n == 1 {
                    return Err("composite requires a vector".into());
                }
                let mut c = Vec::new();
                for id in &a[2..] {
                    let (r, t, _) = self.reg(*id)?;
                    let lanes = self.lanes(t)?;
                    c.extend((0..lanes).map(|i| (r, i)));
                }
                let r = self.compose(&c, n)?;
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(r, false),
                    },
                );
            }
            81 => {
                let (r, t, _) = self.reg(a[2])?;
                if self.ty(a[0])? != Ty::Float
                    || a[3] >= self.lanes(t)? as u32
                    || self.lanes(t)? == 1
                {
                    return Err(
                        "extract requires an in-range vector lane and scalar float result".into(),
                    );
                }
                let lane = a[3] as u8;
                let r = self.emit(|dst| Sir::Swizzle {
                    dst,
                    src: r,
                    lanes: [lane; 4],
                })?;
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(r, false),
                    },
                );
            }
            79 => {
                let n = self.lanes(a[0])?;
                let (x, xt, _) = self.reg(a[2])?;
                let (y, yt, _) = self.reg(a[3])?;
                let xn = self.lanes(xt)?;
                let yn = self.lanes(yt)?;
                if n < 2 || xn < 2 || yn < 2 || a.len() != 4 + n as usize {
                    return Err("vector shuffle type/component count mismatch".into());
                }
                let mut c = Vec::new();
                for &i in &a[4..] {
                    if i >= u32::from(xn + yn) {
                        return Err("shuffle component out of bounds or undefined".into());
                    }
                    c.push(if i < u32::from(xn) {
                        (x, i as u8)
                    } else {
                        (y, i as u8 - xn)
                    });
                }
                let r = self.compose(&c, n)?;
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(r, false),
                    },
                );
            }
            83 => {
                let v = self.value(a[2])?;
                if v.ty != a[0] || matches!(v.value, Value::Pointer { .. } | Value::Int(_)) {
                    return Err("copy object type mismatch".into());
                }
                self.values.insert(a[1], v);
            }
            129 | 131 | 133 | 136 | 142 | 148 => {
                let (x, xt, _) = self.reg(a[2])?;
                let (mut y, yt, _) = self.reg(a[3])?;
                let n = self.lanes(xt)?;
                if op.opcode == 142 {
                    if n < 2 || self.ty(yt)? != Ty::Float || a[0] != xt {
                        return Err("vector-times-scalar type mismatch".into());
                    }
                } else if op.opcode == 148 {
                    if n < 2 || xt != yt || self.ty(a[0])? != Ty::Float {
                        return Err("dot type mismatch".into());
                    }
                } else if xt != yt || xt != a[0] {
                    return Err("float arithmetic type mismatch".into());
                }
                if op.opcode == 136 && n > 1 && n < 4 {
                    let one = self.emit(|dst| Sir::Const {
                        dst,
                        value: Vec4::new(1., 1., 1., 1.),
                    })?;
                    let sources = std::array::from_fn(|i| if i < n as usize { y } else { one });
                    let lanes = std::array::from_fn(|i| if i < n as usize { i as u8 } else { 0 });
                    y = self.emit(|dst| Sir::Compose {
                        dst,
                        sources,
                        lanes,
                    })?;
                }
                let r = self.emit(|dst| match op.opcode {
                    129 => Sir::Add { dst, a: x, b: y },
                    131 => Sir::Sub { dst, a: x, b: y },
                    133 | 142 => Sir::Mul { dst, a: x, b: y },
                    136 => Sir::Div { dst, a: x, b: y },
                    148 if n == 4 => Sir::Dot4 { dst, a: x, b: y },
                    _ => Sir::Dot3 { dst, a: x, b: y },
                })?;
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(r, false),
                    },
                );
            }
            145 => {
                let m = self.value(a[2])?;
                let Value::Matrix(uniform) = m.value else {
                    return Err("matrix must be loaded from a supported uniform block".into());
                };
                let (r, t, _) = self.reg(a[3])?;
                if self.ty(m.ty)? != Ty::Matrix || self.ty(t)? != Ty::Vector(4) || a[0] != t {
                    return Err("matrix-times-vector requires mat4 and vec4".into());
                }
                let r = self.emit(|dst| Sir::Mat4 {
                    dst,
                    src: r,
                    uniform,
                })?;
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(r, false),
                    },
                );
            }
            12 => {
                if !self
                    .module
                    .instructions
                    .iter()
                    .any(|op| op.opcode == 11 && op.operands[0] == a[2])
                {
                    return Err("extended instruction set must be GLSL.std.450 import".into());
                }
                let count = match a[3] {
                    66 | 69 => 1,
                    26 | 37 | 40 => 2,
                    43 | 46 => 3,
                    _ => return Err(format!("unsupported GLSL.std.450 instruction {}", a[3])),
                };
                if a.len() != 4 + count {
                    return Err("extended instruction operand count mismatch".into());
                }
                let mut regs = Vec::new();
                let mut ty = None;
                for &id in &a[4..] {
                    let (r, t, _) = self.reg(id)?;
                    if ty.is_some_and(|expected| expected != t) {
                        return Err("extended instruction argument type mismatch".into());
                    }
                    ty = Some(t);
                    regs.push(r);
                }
                let ty = ty.unwrap();
                let n = self.lanes(ty)?;
                if (a[3] == 66 && self.ty(a[0])? != Ty::Float) || (a[3] != 66 && a[0] != ty) {
                    return Err("extended instruction result type mismatch".into());
                }
                let x = regs[0];
                let r = match a[3] {
                    26 => self.emit(|dst| Sir::Pow {
                        dst,
                        a: x,
                        b: regs[1],
                    })?,
                    37 => self.emit(|dst| Sir::Min {
                        dst,
                        a: x,
                        b: regs[1],
                    })?,
                    40 => self.emit(|dst| Sir::Max {
                        dst,
                        a: x,
                        b: regs[1],
                    })?,
                    43 => {
                        let r = self.emit(|dst| Sir::Max {
                            dst,
                            a: x,
                            b: regs[1],
                        })?;
                        self.emit(|dst| Sir::Min {
                            dst,
                            a: r,
                            b: regs[2],
                        })?
                    }
                    46 => self.emit(|dst| Sir::Mix {
                        dst,
                        a: x,
                        b: regs[1],
                        t: regs[2],
                    })?,
                    66 => self.emit(|dst| Sir::Length {
                        dst,
                        src: x,
                        components: n,
                    })?,
                    69 => self.emit(|dst| Sir::Normalize {
                        dst,
                        src: x,
                        components: n,
                    })?,
                    _ => unreachable!(),
                };
                let r = if a[3] == 66 { r } else { self.canonical(r, n)? };
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(r, false),
                    },
                );
            }
            87 => {
                let s = self.value(a[2])?;
                let Value::Texture(texture) = s.value else {
                    return Err("sampled image must be loaded from a sampler binding".into());
                };
                let (uv, t, direct_uv) = self.reg(a[3])?;
                if self.stage != Stage::Fragment
                    || self.ty(a[0])? != Ty::Vector(4)
                    || self.ty(t)? != Ty::Vector(2)
                    || !direct_uv
                {
                    return Err("implicit sampling currently requires the unmodified vec2 fragment input at location 1".into());
                }
                let r = self.emit(|dst| Sir::SampleImplicit { dst, uv, texture })?;
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(r, false),
                    },
                );
            }
            _ => {}
        }
        Ok(())
    }
    fn run(mut self) -> Result<Compiled> {
        let mut phase = 0;
        let mut function = false;
        let mut executable = false;
        for op in &self.module.instructions {
            match op.opcode {
                54 => {
                    if phase != 0
                        || function
                        || op.operands[1] != self.entry
                        || self.ty(op.operands[0])? != Ty::Void
                        || op.operands[2] != 0
                        || self.ty(op.operands[3])? != Ty::Function(op.operands[0])
                    {
                        return Err(op.error("requires one void main() function without controls"));
                    }
                    function = true;
                    phase = 1;
                }
                248 => {
                    if phase != 1 {
                        return Err(op.error("requires exactly one basic block"));
                    }
                    phase = 2;
                }
                253 => {
                    if phase != 2 {
                        return Err(op.error("return outside the main block"));
                    }
                    phase = 3;
                }
                56 => {
                    if phase != 3 {
                        return Err(op.error("function must end immediately after return"));
                    }
                    phase = 4;
                }
                59 if phase == 2 => {
                    if executable || op.operands[2] != 7 {
                        return Err(op.error("Function variables must start the first block"));
                    }
                    self.instruction(op).map_err(|e| op.error(e))?;
                }
                19..=44 | 59 => {
                    if phase != 0 {
                        return Err(op.error("declaration outside module scope"));
                    }
                    if op.opcode == 59 && op.operands[2] == 7 {
                        return Err(op.error("Function variable outside function"));
                    }
                    self.instruction(op).map_err(|e| op.error(e))?;
                }
                12 | 61..=65 | 79..=87 | 129..=148 => {
                    if phase != 2 {
                        return Err(op.error("executable instruction outside main block"));
                    }
                    executable = true;
                    self.instruction(op).map_err(|e| op.error(e))?;
                }
                _ => {
                    if phase != 0 && op.opcode != 0 {
                        return Err(op.error("metadata inside or after function"));
                    }
                }
            }
        }
        if !function || phase != 4 {
            return Err("SPIR-V missing complete main function/block/return".into());
        }
        for id in &self.interfaces {
            let (_, _, s, _) = self.pointer(*id)?;
            if ![1, 3].contains(&s) {
                return Err("entry interface must name input/output variables".into());
            }
        }
        for ((id, member), d) in &self.decorations {
            if let Some(i) = member {
                let Ty::Struct(m) = self.ty(*id)? else {
                    return Err("member decoration targets a non-struct".into());
                };
                if *i >= m.len() {
                    return Err("member decoration index out of bounds".into());
                }
                let expected = match self.ty(m[*i])? {
                    Ty::Vector(4) if d.builtin == Some(0) => Decoration {
                        builtin: Some(0),
                        ..Default::default()
                    },
                    Ty::Matrix => Decoration {
                        col_major: true,
                        stride: Some(16),
                        offset: Some(0),
                        ..Default::default()
                    },
                    Ty::Float | Ty::Vector(_) => self.uniform_layout(m[*i])?,
                    _ => return Err("unsupported decorated struct member type".into()),
                };
                if *d != expected {
                    return Err("unsupported member decoration combination".into());
                }
            } else if d.block {
                if !matches!(self.ty(*id)?, Ty::Struct(_))
                    || *d
                        != (Decoration {
                            block: true,
                            ..Default::default()
                        })
                {
                    return Err("Block decoration requires a struct".into());
                }
            } else if !self.values.contains_key(id) {
                return Err("variable decoration requires a variable".into());
            }
        }
        let mut positions = 0;
        for (&id, v) in &self.values {
            if matches!(&v.value, Value::Pointer { root, path } if *root == id && path.is_empty())
                && let Ty::Pointer(3, base) = self.ty(v.ty)?
                && (self.decoration(id, None).builtin == Some(0)
                    || self.decoration(base, Some(0)).builtin == Some(0))
            {
                positions += 1;
            }
        }
        if self.stage == Stage::Vertex && positions != 1 {
            return Err("vertex stage requires exactly one Position output".into());
        }
        for op in &self.module.instructions {
            if op.opcode == 6 {
                let Ty::Struct(m) = self.ty(op.operands[0])? else {
                    return Err(op.error("member name targets a non-struct"));
                };
                if op.operands[1] as usize >= m.len() {
                    return Err(op.error("member name index out of bounds"));
                }
            }
        }
        if !self.written.contains(&0) {
            return Err(
                "SPIR-V shader must write Position (vertex) or location 0 (fragment)".into(),
            );
        }
        for &loc in self.outputs.keys() {
            if !self
                .written
                .contains(&(loc + u8::from(self.stage == Stage::Vertex)))
            {
                return Err(format!("output location {loc} is never written"));
            }
        }
        Ok(Compiled {
            stage: self.stage,
            program: Program::new(allocate_registers(self.ops)?)?,
            inputs: self.inputs,
            outputs: self.outputs,
        })
    }
}
/// Reuse dead SSA temporaries; the runtime remains a 64-register machine.
fn allocate_registers(mut ops: Vec<Sir>) -> Result<Vec<Sir>> {
    let mut last = [None; 256];
    for (pc, op) in ops.iter().enumerate() {
        op.clone().map_registers(|r, dst| {
            if !dst {
                last[r as usize] = Some(pc);
            }
            Ok(r)
        })?;
    }
    let mut assigned = [None; 256];
    let mut free: Vec<u8> = (0..64).rev().collect();
    for (pc, op) in ops.iter_mut().enumerate() {
        let mut expired: BTreeSet<usize> = BTreeSet::new();
        let mut destination = None;
        op.map_registers(|r, dst| {
            if dst {
                for old in std::mem::take(&mut expired) {
                    free.push(assigned[old].take().ok_or("invalid SSA lifetime")?);
                }
                if assigned[r as usize].is_some() {
                    return Err("duplicate SSA definition".into());
                }
                let physical = free
                    .pop()
                    .ok_or("SPIR-V exceeds 64 simultaneously live SIR registers")?;
                assigned[r as usize] = Some(physical);
                destination = Some(r as usize);
                Ok(physical)
            } else {
                let physical = assigned[r as usize].ok_or("SSA source used before definition")?;
                if last[r as usize] == Some(pc) {
                    expired.insert(r as usize);
                }
                Ok(physical)
            }
        })?;
        for old in expired
            .into_iter()
            .chain(destination.filter(|&r| last[r].is_none()))
        {
            free.push(assigned[old].take().ok_or("invalid SSA lifetime")?);
        }
    }
    Ok(ops)
}
/// Check stage order and every consumed varying before constructing a graphics pipeline.
pub fn link(vertex: &Compiled, fragment: &Compiled) -> Result<()> {
    if vertex.stage != Stage::Vertex || fragment.stage != Stage::Fragment {
        return Err("SPIR-V pipeline requires vertex then fragment stages".into());
    }
    for (loc, n) in &fragment.inputs {
        if vertex.outputs.get(loc) != Some(n) {
            return Err(format!(
                "SPIR-V varying location {loc}: vertex/fragment interface type mismatch"
            ));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocator_reuses_dead_values_and_rejects_excess_live_values() {
        let mut ops: Vec<_> = (0..65)
            .map(|dst| Sir::Const {
                dst,
                value: Vec4::new(dst as f32, 0., 0., 0.),
            })
            .collect();
        ops.extend((0..65).map(|src| Sir::Output { slot: 0, src }));
        assert!(
            allocate_registers(ops)
                .unwrap_err()
                .contains("simultaneously live")
        );
        let mut ops = vec![Sir::Const {
            dst: 0,
            value: Vec4::new(1., 1., 1., 1.),
        }];
        for dst in 1..200 {
            ops.push(Sir::Add {
                dst,
                a: dst - 1,
                b: dst - 1,
            });
        }
        ops.push(Sir::Output { slot: 0, src: 199 });
        let ops = allocate_registers(ops).unwrap();
        assert!(
            ops.iter()
                .all(|op| !matches!(op, Sir::Add { dst, .. } if *dst > 1))
        );
        Program::new(ops).unwrap();
    }
}
