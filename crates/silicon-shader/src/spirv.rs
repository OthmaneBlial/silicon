//! A deliberately strict SPIR-V 1.0 graphics subset, lowered into the SIR VM.
//! No guest pointers, compiler dependency, dynamic code, or host GPU execution.
use crate::{Comparison, Instruction as Sir, Logic, Program, Result};
mod control;
use silicon_math::Vec4;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
    Compute,
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
    /// Workgroup dimensions declared by a compute entry point.
    pub local_size: [u32; 3],
    /// Number of read-only vec4 storage bindings, numbered from zero.
    pub storage_input_count: u8,
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
        20 => "OpTypeBool",
        21 => "OpTypeInt",
        22 => "OpTypeFloat",
        23 => "OpTypeVector",
        24 => "OpTypeMatrix",
        25 => "OpTypeImage",
        29 => "OpTypeRuntimeArray",
        27 => "OpTypeSampledImage",
        30 => "OpTypeStruct",
        32 => "OpTypePointer",
        33 => "OpTypeFunction",
        41 => "OpConstantTrue",
        42 => "OpConstantFalse",
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
        112 => "OpConvertUToF",
        127 => "OpFNegate",
        129 => "OpFAdd",
        131 => "OpFSub",
        133 => "OpFMul",
        136 => "OpFDiv",
        142 => "OpVectorTimesScalar",
        145 => "OpMatrixTimesVector",
        148 => "OpDot",
        164 => "OpLogicalEqual",
        165 => "OpLogicalNotEqual",
        166 => "OpLogicalOr",
        167 => "OpLogicalAnd",
        168 => "OpLogicalNot",
        169 => "OpSelect",
        180 => "OpFOrdEqual",
        182 => "OpFOrdNotEqual",
        183 => "OpFUnordNotEqual",
        184 => "OpFOrdLessThan",
        186 => "OpFOrdGreaterThan",
        188 => "OpFOrdLessThanEqual",
        190 => "OpFOrdGreaterThanEqual",
        245 => "OpPhi",
        247 => "OpSelectionMerge",
        248 => "OpLabel",
        249 => "OpBranch",
        250 => "OpBranchConditional",
        252 => "OpKill",
        253 => "OpReturn",
        255 => "OpUnreachable",
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
            0 | 56 | 252 | 253 | 255 => (0, 0),
            19 | 20 | 17 | 248 | 249 => (1, 1),
            14 | 22 | 41 | 42 | 247 => (2, 2),
            16 => (2, 5),
            21 | 23 | 24 | 32 | 43 | 61 | 83 | 112 | 127 | 168 | 250 => (3, 3),
            29 => (2, 2),
            59 => (3, 4),
            65 => (4, 5),
            12 => (5, 7),
            25 => (8, 8),
            27 => (2, 2),
            54 | 81 | 87 | 129 | 131 | 133 | 136 | 142 | 145 | 148 => (4, 4),
            88 => (6, 6),
            164..=167 | 180 | 182..=184 | 186 | 188 | 190 => (4, 4),
            169 => (5, 5),
            245 => (4, 6),
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
            _ => return Err(self.error("unsupported instruction in the graphics subset")),
        };
        if !(min..=max).contains(&n) {
            return Err(self.error(format!("expected {min}..{max} operands, found {n}")));
        }
        if self.opcode == 16 && !matches!(n, 2 | 5) {
            return Err(self.error("execution mode requires two or five operands"));
        }
        if self.opcode == 245 && !n.is_multiple_of(2) {
            return Err(self.error("Phi requires value/predecessor pairs"));
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
            19 | 20 | 21 | 22 | 248 => (Some(a[0]), vec![]),
            23 | 24 | 27 | 29 | 33 => (Some(a[0]), vec![a[1]]),
            25 => (Some(a[0]), vec![a[1]]),
            30 => (Some(a[0]), a[1..].to_vec()),
            32 => (Some(a[0]), vec![a[2]]),
            41..=43 => (Some(a[1]), vec![a[0]]),
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
            44 | 80 | 65 | 169 | 245 => {
                let mut r = vec![a[0]];
                r.extend_from_slice(&a[2..]);
                (Some(a[1]), r)
            }
            61 | 81 | 83 | 112 | 127 | 168 => (Some(a[1]), vec![a[0], a[2]]),
            62 => (None, a.clone()),
            79
            | 87
            | 88
            | 129
            | 131
            | 133
            | 136
            | 142
            | 145
            | 148
            | 164..=167
            | 180
            | 182..=184
            | 186
            | 188
            | 190 => (Some(a[1]), vec![a[0], a[2], a[3]]),
            247 | 249 => (None, vec![a[0]]),
            250 => (None, a.clone()),
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
    Bool,
    Int,
    UInt,
    Float,
    Vector(u8),
    IntVector(u8),
    UIntVector(u8),
    Matrix,
    Image(u32),
    Sampled(u32),
    Struct(Vec<u32>),
    RuntimeArray(u32),
    Pointer(u32, u32),
    Function(u32),
}
#[derive(Clone, Debug)]
enum Value {
    Reg(u8, bool),
    Int(u32),
    IntVector(Vec<u32>),
    Pointer {
        root: u32,
        path: Vec<usize>,
        dynamic_index: Option<u8>,
    },
    Matrix(u8),
    Texture(u8),
}
type PointerInfo = (u32, Vec<usize>, Option<u8>, u32, u32);
#[derive(Clone, Debug)]
struct Typed {
    ty: u32,
    value: Value,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Decoration {
    block: bool,
    buffer_block: bool,
    location: Option<u32>,
    binding: Option<u32>,
    set: Option<u32>,
    builtin: Option<u32>,
    col_major: bool,
    stride: Option<u32>,
    array_stride: Option<u32>,
    offset: Option<u32>,
    non_readable: bool,
    non_writable: bool,
}
struct Compiler<'a> {
    module: &'a Module,
    stage: Stage,
    entry: u32,
    local_size: Option<[u32; 3]>,
    interfaces: BTreeSet<u32>,
    types: BTreeMap<u32, Ty>,
    values: BTreeMap<u32, Typed>,
    locals: BTreeMap<u32, Typed>,
    globals: BTreeSet<u32>,
    available: Option<BTreeSet<u32>>,
    decorations: BTreeMap<(u32, Option<usize>), Decoration>,
    ops: Vec<Sir>,
    next: u16,
    inputs: BTreeMap<u8, u8>,
    outputs: BTreeMap<u8, u8>,
    written: BTreeSet<u8>,
    storage_inputs: BTreeSet<u8>,
    storage_output: Option<u8>,
}
impl<'a> Compiler<'a> {
    fn new(module: &'a Module) -> Result<Self> {
        let mut entry = None;
        let mut memory = false;
        let mut capability = false;
        let mut origin = false;
        let mut local_size = None;
        let mut decorations = BTreeMap::new();
        for op in &module.instructions {
            let a = &op.operands;
            match op.opcode {
                15 => {
                    let stage = match a[0] {
                        0 => Stage::Vertex,
                        4 => Stage::Fragment,
                        5 => Stage::Compute,
                        _ => {
                            return Err(op.error(
                                "only Vertex, Fragment and Compute entry points are supported",
                            ));
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
                16 => match (a[1], a.get(2..)) {
                    (7, Some([])) if !origin => origin = true,
                    (17, Some([x, y, z])) if local_size.is_none() => {
                        if [*x, *y, *z].contains(&0)
                            || x.checked_mul(*y)
                                .and_then(|n| n.checked_mul(*z))
                                .is_none_or(|n| n > 1024)
                        {
                            return Err(op.error("LocalSize requires 1..1024 local invocations"));
                        }
                        local_size = Some([*x, *y, *z]);
                    }
                    _ => {
                        return Err(
                            op.error("supports OriginUpperLeft or LocalSize execution mode")
                        );
                    }
                },
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
                        3 if member.is_none() && v.is_none() && !d.buffer_block => {
                            d.buffer_block = true
                        }
                        5 if member.is_some() && v.is_none() && !d.col_major => d.col_major = true,
                        6 if member.is_none() => set(&mut d.array_stride)?,
                        7 if member.is_some() => set(&mut d.stride)?,
                        11 => set(&mut d.builtin)?,
                        30 if member.is_none() => set(&mut d.location)?,
                        33 if member.is_none() => set(&mut d.binding)?,
                        34 if member.is_none() => set(&mut d.set)?,
                        35 if member.is_some() => set(&mut d.offset)?,
                        24 if v.is_none() && !d.non_writable => d.non_writable = true,
                        25 if v.is_none() && !d.non_readable => d.non_readable = true,
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
        if !memory
            || !capability
            || origin != (stage == Stage::Fragment)
            || local_size.is_some() != (stage == Stage::Compute)
        {
            return Err("SPIR-V requires a memory model, Shader capability, and stage-appropriate execution mode".into());
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
            local_size,
            interfaces,
            types: BTreeMap::new(),
            values: BTreeMap::new(),
            locals: BTreeMap::new(),
            globals: BTreeSet::new(),
            available: None,
            decorations,
            ops: vec![Sir::Const {
                dst: 0,
                value: Vec4::ZERO,
            }],
            next: 1,
            inputs: BTreeMap::new(),
            outputs: BTreeMap::new(),
            written: BTreeSet::new(),
            storage_inputs: BTreeSet::new(),
            storage_output: None,
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
            Ty::Float | Ty::Int | Ty::UInt => Ok(1),
            Ty::Vector(n) | Ty::IntVector(n) | Ty::UIntVector(n) => Ok(n),
            _ => Err(format!("type %{id} is not a supported scalar or vector")),
        }
    }
    fn value(&self, id: u32) -> Result<Typed> {
        if self
            .available
            .as_ref()
            .is_some_and(|ids| !ids.contains(&id) && !self.globals.contains(&id))
        {
            return Err(format!(
                "value %{id} does not dominate this control-flow path"
            ));
        }
        self.values
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("value %{id} is undefined or used before definition"))
    }
    fn reg(&self, id: u32) -> Result<(u8, u32, bool)> {
        let v = self.value(id)?;
        match v.value {
            Value::Reg(r, uv) => Ok((r, v.ty, uv)),
            _ => Err(format!("value %{id} is not a VM register")),
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
    fn pointer(&self, id: u32) -> Result<PointerInfo> {
        let v = self.value(id)?;
        let Ty::Pointer(storage, base) = self.ty(v.ty)? else {
            return Err("expected pointer type".into());
        };
        let Value::Pointer {
            root,
            path,
            dynamic_index,
        } = v.value
        else {
            return Err("expected logical variable pointer".into());
        };
        Ok((root, path, dynamic_index, storage, base))
    }
    fn slot(&self, root: u32, member: Option<usize>) -> Result<u8> {
        let (_, _, _, _, base) = self.pointer(root)?;
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
        if d.block
            || d.buffer_block
            || d.col_major
            || d.stride.is_some()
            || d.array_stride.is_some()
            || d.offset.is_some()
        {
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
                if self.stage == Stage::Compute {
                    return Err("compute shaders do not bind sampled images".into());
                }
                if !matches!(t, Ty::Sampled(1 | 3))
                    || d.set != Some(1)
                    || d.binding.is_none_or(|b| b > 15)
                {
                    return Err("samplers require sampler2D or samplerCube, descriptor set 1, binding 0..15".into());
                }
            }
            1 => {
                if self.stage == Stage::Compute {
                    let slot = match d.builtin {
                        Some(28) => 0, // GlobalInvocationId
                        Some(27) => 1, // LocalInvocationId
                        Some(26) => 2, // WorkgroupId
                        Some(24) => 3, // NumWorkgroups
                        _ => {
                            return Err(
                                "compute inputs require a supported invocation BuiltIn".into()
                            );
                        }
                    };
                    if d.location.is_some()
                        || self.ty(base)? != Ty::UIntVector(3)
                        || !self.interfaces.contains(&a[1])
                        || self.inputs.insert(slot, 3).is_some()
                    {
                        return Err(
                            "compute invocation BuiltIns require unique uvec3 entry inputs".into(),
                        );
                    }
                } else {
                    let loc = d.location.ok_or("input requires Location")?;
                    if !matches!(self.ty(base)?, Ty::Float | Ty::Vector(_)) {
                        return Err("graphics inputs require float32 scalars or vectors".into());
                    }
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
            }
            2 => {
                if self.stage == Stage::Compute && self.decoration(base, None).buffer_block {
                    let Ty::Struct(m) = t else {
                        return Err("storage buffer requires a one-member BufferBlock".into());
                    };
                    let binding = d.binding.ok_or("storage buffer lacks Binding")?;
                    let member = self.decoration(base, Some(0));
                    let array = m.first().copied().ok_or("storage buffer block is empty")?;
                    let array_valid = matches!(self.ty(array)?, Ty::RuntimeArray(vector) if self.ty(vector)? == Ty::Vector(4))
                        && self.decoration(array, None).array_stride == Some(16);
                    let readable = d.non_writable && !d.non_readable;
                    let writable = d.non_readable && !d.non_writable;
                    if m.len() != 1
                        || self.decoration(base, None).block
                        || d.set != Some(0)
                        || binding > 12
                        || !array_valid
                        || member.offset != Some(0)
                        || member.non_writable != readable
                        || member.non_readable != writable
                        || member.builtin.is_some()
                        || member.location.is_some()
                        || member.binding.is_some()
                        || member.set.is_some()
                        || (readable == writable)
                    {
                        return Err("compute storage buffers require one set 0 vec4[] member, stride 16, offset 0, and a read-only or write-only qualifier".into());
                    }
                    let binding = binding as u8;
                    if readable {
                        if self.storage_output.is_some_and(|output| binding >= output)
                            || !self.storage_inputs.insert(binding)
                        {
                            return Err("read-only storage bindings must be unique and precede the output binding".into());
                        }
                    } else if self.storage_output.replace(binding).is_some()
                        || self.storage_inputs.iter().any(|&input| input >= binding)
                    {
                        return Err(
                            "compute supports one write-only storage output after all inputs"
                                .into(),
                        );
                    }
                } else {
                    if self.stage == Stage::Compute {
                        return Err(
                            "compute shaders support storage buffers, not uniform buffers".into(),
                        );
                    }
                    let Ty::Struct(m) = t else {
                        return Err("uniform requires a one-member float/vector/mat4 block".into());
                    };
                    let md = self.decoration(base, Some(0));
                    if m.len() != 1
                        || !self.decoration(base, None).block
                        || d.buffer_block
                        || d.non_readable
                        || d.non_writable
                        || d.set != Some(0)
                        || d.binding.is_none_or(|b| b > 15)
                        || md != self.uniform_layout(m[0])?
                    {
                        return Err("uniform requires offset 0, set 0, binding 0..15; mat4 requires col-major stride 16".into());
                    }
                }
            }
            3 => {
                if self.stage == Stage::Compute {
                    return Err("compute shaders do not write graphics outputs".into());
                }
                if !self.interfaces.contains(&a[1]) {
                    return Err("output missing from entry interface".into());
                }
                match t {
                    Ty::Struct(m) if self.stage == Stage::Vertex => {
                        if m.len() != 1
                            || self.ty(m[0])? != Ty::Vector(4)
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
                        if !matches!(self.ty(base)?, Ty::Float | Ty::Vector(_)) {
                            return Err(
                                "graphics outputs require float32 scalars or vectors".into()
                            );
                        }
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
                self.value_lanes(base)?;
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
                    dynamic_index: None,
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
    fn value_lanes(&self, id: u32) -> Result<u8> {
        match self.ty(id)? {
            Ty::Bool | Ty::Int | Ty::UInt => Ok(1),
            _ => self.lanes(id),
        }
    }
    fn compute_input_slot(&self, builtin: u32) -> Option<u8> {
        match builtin {
            28 => Some(0), // GlobalInvocationId
            27 => Some(1), // LocalInvocationId
            26 => Some(2), // WorkgroupId
            24 => Some(3), // NumWorkgroups
            _ => None,
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
            20 => {
                self.types.insert(a[0], Ty::Bool);
            }
            21 => {
                if a[1] != 32 || a[2] > 1 {
                    return Err(
                        "only 32-bit signed and unsigned integer types are supported".into(),
                    );
                }
                self.types
                    .insert(a[0], if a[2] == 0 { Ty::UInt } else { Ty::Int });
            }
            22 => {
                if a[1] != 32 {
                    return Err("only float32 is supported".into());
                }
                self.types.insert(a[0], Ty::Float);
            }
            23 => {
                if !(2..=4).contains(&a[2]) {
                    return Err("requires a two to four component vector".into());
                }
                let ty = match self.ty(a[1])? {
                    Ty::Float => Ty::Vector(a[2] as u8),
                    Ty::Int => Ty::IntVector(a[2] as u8),
                    Ty::UInt => Ty::UIntVector(a[2] as u8),
                    _ => return Err("vectors require float32 or int32 components".into()),
                };
                self.types.insert(a[0], ty);
            }
            24 => {
                if self.ty(a[1])? != Ty::Vector(4) || a[2] != 4 {
                    return Err("only mat4 is supported".into());
                }
                self.types.insert(a[0], Ty::Matrix);
            }
            25 => {
                if self.ty(a[1])? != Ty::Float
                    || !matches!(a[2], 1 | 3)
                    || a[3..] != [0, 0, 0, 1, 0]
                {
                    return Err(
                        "only sampled float32 2D or cube non-array/non-depth/non-MS images are supported"
                            .into(),
                    );
                }
                self.types.insert(a[0], Ty::Image(a[2]));
            }
            27 => {
                let Ty::Image(dimension) = self.ty(a[1])? else {
                    return Err("sampled image requires supported image type".into());
                };
                self.types.insert(a[0], Ty::Sampled(dimension));
            }
            29 => {
                if self.ty(a[1])? != Ty::Vector(4) {
                    return Err("runtime arrays support only vec4 elements".into());
                }
                self.types.insert(a[0], Ty::RuntimeArray(a[1]));
            }
            30 => {
                if a.len() != 2
                    || !matches!(
                        self.ty(a[1])?,
                        Ty::Float | Ty::Vector(_) | Ty::Matrix | Ty::RuntimeArray(_)
                    )
                {
                    return Err(
                        "struct supports one float/vector/mat4 or vec4 runtime-array member".into(),
                    );
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
            41 | 42 => {
                if self.ty(a[0])? != Ty::Bool {
                    return Err("boolean constant type mismatch".into());
                }
                let f = f32::from(u8::from(op.opcode == 41));
                let r = self.emit(|dst| Sir::Const {
                    dst,
                    value: Vec4::new(f, f, f, f),
                })?;
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(r, false),
                    },
                );
            }
            43 => {
                let value = match self.ty(a[0])? {
                    Ty::Int | Ty::UInt => Value::Int(a[2]),
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
            44 if matches!(self.ty(a[0])?, Ty::IntVector(_) | Ty::UIntVector(_)) => {
                let n = self.lanes(a[0])? as usize;
                let component_type = match self.ty(a[0])? {
                    Ty::IntVector(_) => Ty::Int,
                    Ty::UIntVector(_) => Ty::UInt,
                    _ => unreachable!(),
                };
                if a.len() != 2 + n {
                    return Err("integer vector constant component count mismatch".into());
                }
                let mut components = Vec::with_capacity(n);
                for &id in &a[2..] {
                    let value = self.value(id)?;
                    if self.ty(value.ty)? != component_type {
                        return Err("integer vector constants require int32 components".into());
                    }
                    let Value::Int(value) = value.value else {
                        return Err("integer vector constant component is not literal".into());
                    };
                    components.push(value);
                }
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::IntVector(components),
                    },
                );
            }
            59 => self.variable(a)?,
            65 => {
                let (root, mut path, mut dynamic_index, storage, mut base) = self.pointer(a[2])?;
                let Ty::Pointer(result_storage, target) = self.ty(a[0])? else {
                    return Err("access chain result must be a pointer".into());
                };
                for &id in &a[3..] {
                    let index_value = self.value(id)?;
                    base = match self.ty(base)? {
                        Ty::Struct(m) if [2, 3].contains(&storage) => {
                            let Value::Int(index) = index_value.value else {
                                return Err("struct member index requires an integer constant".into());
                            };
                            path.push(index as usize);
                            *m.get(index as usize).ok_or("struct member index out of bounds")?
                        }
                        Ty::RuntimeArray(element)
                            if storage == 2
                                && path == [0]
                                && dynamic_index.is_none()
                                && self.ty(target)? == Ty::Vector(4) =>
                        {
                            let (index, ty, _) = self.reg(id)?;
                            if !matches!(self.ty(ty)?, Ty::Int | Ty::UInt) {
                                return Err("runtime-array index requires an int32 value".into());
                            }
                            dynamic_index = Some(index);
                            element
                        }
                        Ty::Vector(n)
                            if [1, 2, 7].contains(&storage)
                                && matches!(index_value.value, Value::Int(index) if index < u32::from(n))
                                && matches!((self.ty(base)?, self.ty(target)?), (Ty::Vector(_), Ty::Float) | (Ty::IntVector(_), Ty::Int) | (Ty::UIntVector(_), Ty::UInt)) =>
                        {
                            let Value::Int(index) = index_value.value else { unreachable!() };
                            path.push(index as usize);
                            target
                        }
                        Ty::IntVector(n) | Ty::UIntVector(n)
                            if [1, 2, 7].contains(&storage)
                                && matches!(index_value.value, Value::Int(index) if index < u32::from(n))
                                && matches!((self.ty(base)?, self.ty(target)?), (Ty::IntVector(_), Ty::Int) | (Ty::UIntVector(_), Ty::UInt)) =>
                        {
                            let Value::Int(index) = index_value.value else { unreachable!() };
                            path.push(index as usize);
                            target
                        }
                        _ => return Err("access chain supports uniform members, compute storage arrays and uniform/local vector components".into()),
                    };
                }
                if path.len() > 2 || storage != result_storage || base != target {
                    return Err("access chain result pointer type/storage mismatch".into());
                }
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Pointer {
                            root,
                            path,
                            dynamic_index,
                        },
                    },
                );
            }
            61 => {
                let (root, path, dynamic_index, storage, base) = self.pointer(a[2])?;
                if a[0] != base {
                    return Err("load result/pointee type mismatch".into());
                }
                let d = self.decoration(root, None);
                let value = match storage {
                    0 if path.is_empty() => {
                        Value::Texture(d.binding.ok_or("sampler lacks binding")? as u8)
                    }
                    1 if path.len() <= 1 && dynamic_index.is_none() => {
                        let slot = if self.stage == Stage::Compute {
                            self.compute_input_slot(d.builtin.ok_or("compute input lacks BuiltIn")?)
                                .ok_or("unsupported compute input BuiltIn")?
                        } else {
                            d.location.ok_or("input lacks location")? as u8
                        };
                        let n = self.lanes(base)?;
                        let r = self.emit(|dst| Sir::Input { dst, slot })?;
                        let r = if let Some(&lane) = path.first() {
                            self.emit(|dst| Sir::Swizzle {
                                dst,
                                src: r,
                                lanes: [lane as u8; 4],
                            })?
                        } else {
                            self.canonical(r, n)?
                        };
                        Value::Reg(
                            r,
                            path.is_empty()
                                && self.stage == Stage::Fragment
                                && slot == 1
                                && matches!(n, 2 | 3),
                        )
                    }
                    2 if self.stage == Stage::Compute
                        && self.storage_inputs.contains(
                            &(d.binding.ok_or("storage buffer lacks binding")? as u8),
                        )
                        && path == [0]
                        && dynamic_index.is_some()
                        && self.ty(base)? == Ty::Vector(4) =>
                    {
                        let binding = d.binding.ok_or("storage buffer lacks binding")? as u8;
                        let r = self.emit(|dst| Sir::StorageLoad {
                            dst,
                            buffer: binding,
                            index: dynamic_index.unwrap(),
                        })?;
                        Value::Reg(r, false)
                    }
                    2 if self.stage == Stage::Compute
                        && self.decoration(base, None).buffer_block =>
                    {
                        return Err(
                            "compute shaders cannot load a write-only storage output".into()
                        );
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
                    _ => {
                        return Err(
                            "load supports inputs, sampler/uniform bindings, compute storage inputs and local variables"
                                .into(),
                        );
                    }
                };
                self.values.insert(a[1], Typed { ty: a[0], value });
            }
            62 => {
                let (root, path, dynamic_index, storage, base) = self.pointer(a[0])?;
                let (r, t, uv) = self.reg(a[1])?;
                let d = self.decoration(root, None);
                if t != base {
                    return Err("store value/pointee type mismatch".into());
                }
                match storage {
                    2 if self.stage == Stage::Compute
                        && self.storage_output
                            == Some(d.binding.ok_or("storage output lacks binding")? as u8)
                        && path == [0]
                        && dynamic_index.is_some()
                        && self.ty(base)? == Ty::Vector(4) =>
                    {
                        self.ops.push(Sir::StorageStore {
                            index: dynamic_index.unwrap(),
                            src: r,
                        });
                        self.written.insert(0);
                    }
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
                let source = self.value(a[2])?;
                let source_ty = self.ty(source.ty)?;
                let result_ty = self.ty(a[0])?;
                let n = self.lanes(source.ty)?;
                if a[3] >= n as u32 || n == 1 {
                    return Err("extract requires an in-range vector lane".into());
                }
                let integer_component = matches!(
                    (source_ty.clone(), result_ty.clone()),
                    (Ty::IntVector(_), Ty::Int) | (Ty::UIntVector(_), Ty::UInt)
                );
                if integer_component && let Value::IntVector(values) = source.value {
                    self.values.insert(
                        a[1],
                        Typed {
                            ty: a[0],
                            value: Value::Int(values[a[3] as usize]),
                        },
                    );
                    return Ok(());
                }
                if !matches!(
                    (source_ty, result_ty),
                    (Ty::Vector(_), Ty::Float)
                        | (Ty::IntVector(_), Ty::Int)
                        | (Ty::UIntVector(_), Ty::UInt)
                ) {
                    return Err("extract result type must match the vector component type".into());
                }
                let (r, _, _) = self.reg(a[2])?;
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
                if v.ty != a[0]
                    || matches!(
                        v.value,
                        Value::Pointer { .. } | Value::Int(_) | Value::IntVector(_)
                    )
                {
                    return Err("copy object type mismatch".into());
                }
                self.values.insert(a[1], v);
            }
            127 => {
                let (src, src_ty, _) = self.reg(a[2])?;
                if src_ty != a[0] || !matches!(self.ty(src_ty)?, Ty::Float | Ty::Vector(_)) {
                    return Err("float negate type mismatch".into());
                }
                let r = self.emit(|dst| Sir::Neg { dst, src })?;
                let r = self.canonical(r, self.lanes(a[0])?)?;
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(r, false),
                    },
                );
            }
            112 => {
                let (reg, source, _) = self.reg(a[2])?;
                let compatible = match (self.ty(source)?, self.ty(a[0])?) {
                    (Ty::UInt, Ty::Float) => true,
                    (Ty::UIntVector(n), Ty::Vector(m)) => n == m,
                    _ => false,
                };
                if self.stage != Stage::Compute || !compatible {
                    return Err("OpConvertUToF supports compute uint32 scalars and vectors".into());
                }
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(reg, false),
                    },
                );
            }
            129 | 131 | 133 | 136 | 142 | 148 => {
                let (x, xt, _) = self.reg(a[2])?;
                let (mut y, yt, _) = self.reg(a[3])?;
                if !matches!(self.ty(xt)?, Ty::Float | Ty::Vector(_)) {
                    return Err("float arithmetic requires float32 scalars or vectors".into());
                }
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
            164..=169 | 180 | 182..=184 | 186 | 188 | 190 => {
                let (x, xt, _) = self.reg(a[2])?;
                let r = if op.opcode == 168 {
                    if self.ty(xt)? != Ty::Bool || xt != a[0] {
                        return Err("logical Not requires bool".into());
                    }
                    self.emit(|dst| Sir::Not { dst, src: x })?
                } else {
                    let (y, yt, yuv) = self.reg(a[3])?;
                    if op.opcode == 169 {
                        let (z, zt, zuv) = self.reg(a[4])?;
                        if self.ty(xt)? != Ty::Bool || yt != zt || yt != a[0] {
                            return Err("Select requires scalar bool and matching scalar float/bool alternatives".into());
                        }
                        if self.value_lanes(yt)? != 1 {
                            return Err("SPIR-V 1.0 vector Select requires a vector bool condition, which is unsupported".into());
                        }
                        let r = self.emit(|dst| Sir::Select {
                            dst,
                            condition: x,
                            a: y,
                            b: z,
                        })?;
                        self.values.insert(
                            a[1],
                            Typed {
                                ty: a[0],
                                value: Value::Reg(r, yuv && zuv),
                            },
                        );
                        return Ok(());
                    }
                    if xt != yt || self.ty(a[0])? != Ty::Bool {
                        return Err("comparison type mismatch".into());
                    }
                    if op.opcode < 168 {
                        if self.ty(xt)? != Ty::Bool {
                            return Err("logical operation requires bool operands".into());
                        }
                        let kind = match op.opcode {
                            164 => Logic::Equal,
                            165 => Logic::NotEqual,
                            166 => Logic::Or,
                            _ => Logic::And,
                        };
                        self.emit(|dst| Sir::Logical {
                            dst,
                            a: x,
                            b: y,
                            kind,
                        })?
                    } else {
                        if self.ty(xt)? != Ty::Float {
                            return Err("comparison requires scalar float32 operands".into());
                        }
                        // SIR rejects non-finite values, so ordered/unordered != are equivalent here.
                        let kind = match op.opcode {
                            180 => Comparison::Equal,
                            182 | 183 => Comparison::NotEqual,
                            184 => Comparison::Less,
                            186 => Comparison::Greater,
                            188 => Comparison::LessEqual,
                            _ => Comparison::GreaterEqual,
                        };
                        self.emit(|dst| Sir::Compare {
                            dst,
                            a: x,
                            b: y,
                            kind,
                        })?
                    }
                };
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
                if !matches!(self.ty(ty)?, Ty::Float | Ty::Vector(_)) {
                    return Err("GLSL.std.450 operations require float32 scalars or vectors".into());
                }
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
                let Ty::Sampled(dimension) = self.ty(s.ty)? else {
                    return Err("sampled image has an unsupported type".into());
                };
                let (coordinate, t, direct_coordinate) = self.reg(a[3])?;
                let coordinate_components = if dimension == 3 { 3 } else { 2 };
                if self.stage != Stage::Fragment
                    || self.ty(a[0])? != Ty::Vector(4)
                    || self.ty(t)? != Ty::Vector(coordinate_components)
                    || !direct_coordinate
                {
                    return Err(format!(
                        "implicit sampling requires the unmodified vec{coordinate_components} fragment input at location 1"
                    ));
                }
                // ponytail: track one source varying; propagate SIR gradients when transformed samples are needed.
                let r = if dimension == 3 {
                    self.emit(|dst| Sir::SampleCubeImplicit {
                        dst,
                        direction: coordinate,
                        texture,
                    })?
                } else {
                    self.emit(|dst| Sir::SampleImplicit {
                        dst,
                        uv: coordinate,
                        texture,
                    })?
                };
                self.values.insert(
                    a[1],
                    Typed {
                        ty: a[0],
                        value: Value::Reg(r, false),
                    },
                );
            }
            88 => {
                let sampled = self.value(a[2])?;
                let Value::Texture(texture) = sampled.value else {
                    return Err("sampled image must be loaded from a sampler binding".into());
                };
                let Ty::Sampled(dimension) = self.ty(sampled.ty)? else {
                    return Err("sampled image has an unsupported type".into());
                };
                let (uv, uv_ty, _) = self.reg(a[3])?;
                let (lod, lod_ty, _) = self.reg(a[5])?;
                let coordinate_components = if dimension == 3 { 3 } else { 2 };
                if self.stage != Stage::Fragment
                    || self.ty(a[0])? != Ty::Vector(4)
                    || self.ty(uv_ty)? != Ty::Vector(coordinate_components)
                    || self.ty(lod_ty)? != Ty::Float
                {
                    return Err(format!(
                        "explicit sampling requires fragment vec{coordinate_components} coordinates and a scalar LOD"
                    ));
                }
                if a[4] != 2 {
                    return Err("explicit sampling supports only the Lod image operand".into());
                }
                let coordinate = if dimension == 3 {
                    self.emit(|dst| Sir::Compose {
                        dst,
                        sources: [uv, uv, uv, lod],
                        lanes: [0, 1, 2, 0],
                    })?
                } else {
                    self.emit(|dst| Sir::Compose {
                        dst,
                        sources: [uv, uv, lod, 0],
                        lanes: [0, 1, 0, 0],
                    })?
                };
                let r = if dimension == 3 {
                    self.emit(|dst| Sir::SampleCube {
                        dst,
                        direction: coordinate,
                        texture,
                    })?
                } else {
                    self.emit(|dst| Sir::Sample {
                        dst,
                        uv: coordinate,
                        texture,
                    })?
                };
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
        let ops = &self.module.instructions;
        let start = ops
            .iter()
            .position(|op| op.opcode == 54)
            .ok_or("SPIR-V missing main function")?;
        for op in &ops[..start] {
            match op.opcode {
                19..=44 | 59 => {
                    if op.opcode == 59 && op.operands[2] == 7 {
                        return Err(op.error("Function variable outside function"));
                    }
                    self.instruction(op).map_err(|e| op.error(e))?;
                }
                0 | 3 | 5 | 6 | 11 | 14..=17 | 71 | 72 => {}
                _ => return Err(op.error("instruction outside main function")),
            }
        }
        let function = &ops[start];
        if function.operands[1] != self.entry
            || self.ty(function.operands[0])? != Ty::Void
            || function.operands[2] != 0
            || self.ty(function.operands[3])? != Ty::Function(function.operands[0])
        {
            return Err(function.error("requires one void main() function without controls"));
        }
        if ops.last().is_none_or(|op| op.opcode != 56) {
            return Err("SPIR-V missing main FunctionEnd".into());
        }
        self.globals = self.values.keys().copied().collect();
        self.available = Some(BTreeSet::new());
        self.lower_control(&ops[start + 1..ops.len() - 1])?;
        self.available = None;
        for id in &self.interfaces {
            let (_, _, _, s, _) = self.pointer(*id)?;
            if ![1, 3].contains(&s) {
                return Err("entry interface must name stage input/output variables".into());
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
                    Ty::RuntimeArray(vector)
                        if self.ty(vector)? == Ty::Vector(4)
                            && self.decoration(*id, None).buffer_block
                            && (d.non_readable ^ d.non_writable) =>
                    {
                        Decoration {
                            offset: Some(0),
                            non_readable: d.non_readable,
                            non_writable: d.non_writable,
                            ..Default::default()
                        }
                    }
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
            } else if d.buffer_block {
                if !matches!(self.ty(*id)?, Ty::Struct(_))
                    || *d
                        != (Decoration {
                            buffer_block: true,
                            ..Default::default()
                        })
                {
                    return Err("BufferBlock decoration requires a struct".into());
                }
            } else if d.array_stride.is_some() {
                let Ty::RuntimeArray(element) = self.ty(*id)? else {
                    return Err("ArrayStride decoration requires a runtime array".into());
                };
                if self.ty(element)? != Ty::Vector(4)
                    || *d
                        != (Decoration {
                            array_stride: Some(16),
                            ..Default::default()
                        })
                {
                    return Err("compute runtime arrays require vec4 stride 16".into());
                }
            } else if d.builtin == Some(25) {
                if *d
                    != (Decoration {
                        builtin: Some(25),
                        ..Default::default()
                    })
                    || !matches!(self.values.get(id), Some(Typed { ty, value: Value::IntVector(_), .. }) if self.ty(*ty).ok() == Some(Ty::UIntVector(3)))
                {
                    return Err("WorkgroupSize BuiltIn requires a uvec3 constant".into());
                }
            } else if !self.values.contains_key(id) {
                return Err("variable decoration requires a variable".into());
            }
        }
        let mut positions = 0;
        for (&id, v) in &self.values {
            if matches!(&v.value, Value::Pointer { root, path, .. } if *root == id && path.is_empty())
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
        let storage_input_count = self.storage_inputs.len() as u8;
        if self.stage == Stage::Compute {
            let output = self
                .storage_output
                .ok_or("compute stage requires one write-only storage output")?;
            if output != storage_input_count
                || (0..storage_input_count).any(|binding| !self.storage_inputs.contains(&binding))
            {
                return Err(
                    "compute storage bindings must be inputs 0..N-1 followed by output N".into(),
                );
            }
            if !self.written.contains(&0) {
                return Err("compute main must write its storage output on every live path".into());
            }
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
        let program_ops = allocate_registers(self.ops)?;
        let program = if self.stage == Stage::Compute {
            Program::new_compute(program_ops)?
        } else {
            Program::new(program_ops)?
        };
        Ok(Compiled {
            stage: self.stage,
            program,
            inputs: self.inputs,
            outputs: self.outputs,
            local_size: self.local_size.unwrap_or([1, 1, 1]),
            storage_input_count,
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
