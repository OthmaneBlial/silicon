//! SIR: a bounded, validated vec4 register machine shared by both shader stages.
use serde::{Deserialize, Serialize};
use silicon_math::Vec4;
use std::collections::HashMap;
mod lanes;
mod packet;
pub mod spirv;
pub type Result<T> = std::result::Result<T, String>;
const MAX_WORKGROUP_INVOCATIONS: usize = 1024;
const MAX_DYNAMIC_INSTRUCTIONS: usize = 65_536;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}
impl Comparison {
    fn apply(self, a: f32, b: f32) -> bool {
        match self {
            Self::Equal => a == b,
            Self::NotEqual => a != b,
            Self::Less => a < b,
            Self::LessEqual => a <= b,
            Self::Greater => a > b,
            Self::GreaterEqual => a >= b,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Logic {
    And,
    Or,
    Equal,
    NotEqual,
}
impl Logic {
    fn apply(self, a: f32, b: f32) -> bool {
        let (a, b) = (a != 0., b != 0.);
        match self {
            Self::And => a && b,
            Self::Or => a || b,
            Self::Equal => a == b,
            Self::NotEqual => a != b,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum UnaryMath {
    Round,
    RoundEven,
    Trunc,
    Floor,
    Ceil,
    Fract,
    Sin,
    Cos,
    Exp,
    Log,
    Exp2,
    Log2,
    Sqrt,
    InverseSqrt,
}
impl UnaryMath {
    fn apply(self, value: f32) -> f32 {
        match self {
            Self::Round => value.round(),
            Self::RoundEven => value.round_ties_even(),
            Self::Trunc => value.trunc(),
            Self::Floor => value.floor(),
            Self::Ceil => value.ceil(),
            Self::Fract => value - value.floor(),
            Self::Sin => value.sin(),
            Self::Cos => value.cos(),
            Self::Exp => value.exp(),
            Self::Log => value.ln(),
            Self::Exp2 => value.exp2(),
            Self::Log2 => value.log2(),
            Self::Sqrt => value.sqrt(),
            Self::InverseSqrt => value.sqrt().recip(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Instruction {
    Compare {
        dst: u8,
        a: u8,
        b: u8,
        kind: Comparison,
    },
    Logical {
        dst: u8,
        a: u8,
        b: u8,
        kind: Logic,
    },
    Not {
        dst: u8,
        src: u8,
    },
    Select {
        dst: u8,
        condition: u8,
        a: u8,
        b: u8,
    },
    If {
        condition: u8,
    },
    Else,
    EndIf,
    /// Coalesce the preceding selection's true/false values; must immediately follow EndIf/Merge.
    Merge {
        dst: u8,
        a: u8,
        b: u8,
    },
    /// Copy a register value, including mutable loop-carried values.
    Move {
        dst: u8,
        src: u8,
    },
    /// Mark the entry point for loop header calculations that must be repeated.
    LoopHeader,
    /// Repeat the enclosed instruction range while `condition.x` is nonzero.
    LoopStart {
        condition: u8,
    },
    LoopEnd,
    Return,
    Discard,
    Input {
        dst: u8,
        slot: u8,
    },
    Uniform {
        dst: u8,
        slot: u8,
    },
    Const {
        dst: u8,
        value: Vec4,
    },
    Add {
        dst: u8,
        a: u8,
        b: u8,
    },
    Sub {
        dst: u8,
        a: u8,
        b: u8,
    },
    Mul {
        dst: u8,
        a: u8,
        b: u8,
    },
    Div {
        dst: u8,
        a: u8,
        b: u8,
    },
    Dot3 {
        dst: u8,
        a: u8,
        b: u8,
    },
    Dot4 {
        dst: u8,
        a: u8,
        b: u8,
    },
    Pow {
        dst: u8,
        a: u8,
        b: u8,
    },
    Math {
        dst: u8,
        src: u8,
        operation: UnaryMath,
    },
    Min {
        dst: u8,
        a: u8,
        b: u8,
    },
    Max {
        dst: u8,
        a: u8,
        b: u8,
    },
    Mix {
        dst: u8,
        a: u8,
        b: u8,
        t: u8,
    },
    Length {
        dst: u8,
        src: u8,
        components: u8,
    },
    Normalize {
        dst: u8,
        src: u8,
        components: u8,
    },
    Normalize3 {
        dst: u8,
        src: u8,
    },
    Saturate {
        dst: u8,
        src: u8,
    },
    Swizzle {
        dst: u8,
        src: u8,
        lanes: [u8; 4],
    },
    /// Each output lane selects a lane from its corresponding source register.
    Compose {
        dst: u8,
        sources: [u8; 4],
        lanes: [u8; 4],
    },
    /// Four consecutive row-major uniform vec4s form the matrix.
    Mat4 {
        dst: u8,
        src: u8,
        uniform: u8,
    },
    Sample {
        dst: u8,
        uv: u8,
        texture: u8,
    },
    SampleImplicit {
        dst: u8,
        uv: u8,
        texture: u8,
    },
    /// Explicit-LOD cube lookup; xyz is the direction and w is the LOD.
    SampleCube {
        dst: u8,
        direction: u8,
        texture: u8,
    },
    SampleCubeImplicit {
        dst: u8,
        direction: u8,
        texture: u8,
    },
    /// Sample a 2D array: xy is UV, z is the integer layer and w is explicit LOD.
    SampleArray {
        dst: u8,
        uv: u8,
        texture: u8,
    },
    /// Sample a 2D array with derivative-derived LOD; z remains the layer.
    SampleArrayImplicit {
        dst: u8,
        uv: u8,
        texture: u8,
    },
    /// Sample a 3D volume: xyz are coordinates and w is explicit LOD.
    Sample3D {
        dst: u8,
        coordinate: u8,
        texture: u8,
    },
    /// Sample a 3D volume with derivative-derived LOD.
    Sample3DImplicit {
        dst: u8,
        coordinate: u8,
        texture: u8,
    },
    Output {
        slot: u8,
        src: u8,
    },
    Neg {
        dst: u8,
        src: u8,
    },
    /// Load a vec4 from the selected compute input buffer at `index.x`.
    StorageLoad {
        dst: u8,
        buffer: u8,
        index: u8,
    },
    /// Stage a vec4 write to the compute output buffer at `index.x`.
    StorageStore {
        index: u8,
        src: u8,
    },
    /// Atomically add `value.x` to a vec4 storage element's x component.
    AtomicAdd {
        dst: u8,
        buffer: u8,
        index: u8,
        value: u8,
    },
    /// Atomically replace a vec4 storage element's x component with `value.x`.
    AtomicExchange {
        dst: u8,
        buffer: u8,
        index: u8,
        value: u8,
    },
    /// Compare the x component's bits with `expected.x` and replace on a match.
    AtomicCompareExchange {
        dst: u8,
        buffer: u8,
        index: u8,
        expected: u8,
        replacement: u8,
    },
    /// Load one vec4 from workgroup memory at `index.x`.
    SharedLoad {
        dst: u8,
        index: u8,
    },
    /// Store one vec4 to workgroup memory at `index.x`.
    SharedStore {
        index: u8,
        src: u8,
    },
    /// Synchronize all invocations in the current workgroup.
    WorkgroupBarrier,
    /// Exit the innermost loop and resume immediately after its matching `LoopEnd`.
    LoopBreak,
}

/// A scalar f32 atomic operation on a vec4 storage element's x component.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AtomicOperation {
    Add(f32),
    Exchange(f32),
    CompareExchange { expected: f32, replacement: f32 },
}

impl Instruction {
    /// Visit sources before the destination, including repeated source operands.
    fn map_registers(&mut self, mut f: impl FnMut(u8, bool) -> Result<u8>) -> Result<()> {
        use Instruction::*;
        let dst = match self {
            Input { dst, .. } | Uniform { dst, .. } | Const { dst, .. } => dst,
            Add { dst, a, b }
            | Sub { dst, a, b }
            | Mul { dst, a, b }
            | Div { dst, a, b }
            | Dot3 { dst, a, b }
            | Dot4 { dst, a, b }
            | Compare { dst, a, b, .. }
            | Logical { dst, a, b, .. }
            | Merge { dst, a, b }
            | Pow { dst, a, b }
            | Min { dst, a, b }
            | Max { dst, a, b } => {
                *a = f(*a, false)?;
                *b = f(*b, false)?;
                dst
            }
            Move { dst, src } => {
                *src = f(*src, false)?;
                dst
            }
            Mix { dst, a, b, t } => {
                *a = f(*a, false)?;
                *b = f(*b, false)?;
                *t = f(*t, false)?;
                dst
            }
            Select {
                dst,
                condition,
                a,
                b,
            } => {
                *condition = f(*condition, false)?;
                *a = f(*a, false)?;
                *b = f(*b, false)?;
                dst
            }
            If { condition } | LoopStart { condition } => {
                *condition = f(*condition, false)?;
                return Ok(());
            }
            Else | EndIf | LoopHeader | LoopEnd | LoopBreak | Return | Discard => return Ok(()),
            Normalize3 { dst, src }
            | Not { dst, src }
            | Neg { dst, src }
            | Saturate { dst, src }
            | Math { dst, src, .. }
            | Normalize { dst, src, .. }
            | Length { dst, src, .. }
            | Swizzle { dst, src, .. }
            | Mat4 { dst, src, .. } => {
                *src = f(*src, false)?;
                dst
            }
            Compose { dst, sources, .. } => {
                for src in sources {
                    *src = f(*src, false)?;
                }
                dst
            }
            Sample { dst, uv, .. }
            | SampleImplicit { dst, uv, .. }
            | SampleArray { dst, uv, .. }
            | SampleArrayImplicit { dst, uv, .. } => {
                *uv = f(*uv, false)?;
                dst
            }
            SampleCube { dst, direction, .. } | SampleCubeImplicit { dst, direction, .. } => {
                *direction = f(*direction, false)?;
                dst
            }
            Sample3D {
                dst, coordinate, ..
            }
            | Sample3DImplicit {
                dst, coordinate, ..
            } => {
                *coordinate = f(*coordinate, false)?;
                dst
            }
            Output { src, .. } => {
                *src = f(*src, false)?;
                return Ok(());
            }
            StorageLoad { dst, index, .. } => {
                *index = f(*index, false)?;
                dst
            }
            StorageStore { index, src } => {
                *index = f(*index, false)?;
                *src = f(*src, false)?;
                return Ok(());
            }
            AtomicAdd {
                dst, index, value, ..
            }
            | AtomicExchange {
                dst, index, value, ..
            } => {
                *index = f(*index, false)?;
                *value = f(*value, false)?;
                dst
            }
            AtomicCompareExchange {
                dst,
                index,
                expected,
                replacement,
                ..
            } => {
                *index = f(*index, false)?;
                *expected = f(*expected, false)?;
                *replacement = f(*replacement, false)?;
                dst
            }
            SharedLoad { dst, index } => {
                *index = f(*index, false)?;
                dst
            }
            SharedStore { index, src } => {
                *index = f(*index, false)?;
                *src = f(*src, false)?;
                return Ok(());
            }
            WorkgroupBarrier => return Ok(()),
        };
        *dst = f(*dst, true)?;
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "Vec<Instruction>", into = "Vec<Instruction>")]
pub struct Program {
    ops: Vec<Instruction>,
    loop_pairs: Vec<Option<usize>>,
    loop_entries: Vec<Option<usize>>,
}
impl TryFrom<Vec<Instruction>> for Program {
    type Error = String;
    fn try_from(ops: Vec<Instruction>) -> Result<Self> {
        Self::new(ops)
    }
}
impl From<Program> for Vec<Instruction> {
    fn from(p: Program) -> Self {
        p.ops
    }
}
#[derive(Clone, Debug)]
pub struct Trace {
    pub instruction: usize,
    pub operation: Instruction,
    pub value: Vec4,
}
#[derive(Clone, Debug)]
pub struct Execution {
    pub outputs: [Vec4; 8],
    pub instructions: usize,
    pub samples: usize,
    pub trace: Vec<Trace>,
    pub discarded: bool,
}

fn empty_execution() -> Execution {
    Execution {
        outputs: [Vec4::ZERO; 8],
        instructions: 0,
        samples: 0,
        trace: Vec::new(),
        discarded: false,
    }
}

struct ExecutionState {
    registers: [Vec4; 64],
    result: Execution,
    active: bool,
    live: bool,
    choice: bool,
    selections: Vec<(bool, bool)>,
    loops: Vec<LoopFrame>,
    dynamic_instructions: usize,
    pc: usize,
}

#[derive(Clone, Copy)]
struct LoopFrame {
    start: usize,
    end: usize,
    selection_depth: usize,
    active: bool,
    choice: bool,
}

impl ExecutionState {
    fn new() -> Self {
        Self {
            registers: [Vec4::ZERO; 64],
            result: empty_execution(),
            active: true,
            live: true,
            choice: false,
            selections: Vec::new(),
            loops: Vec::new(),
            dynamic_instructions: 0,
            pc: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExecutionStatus {
    Complete,
    Barrier(usize),
}

#[derive(Default)]
struct SharedAccess {
    writer: Option<usize>,
    readers: Vec<usize>,
}

impl SharedAccess {
    fn read(&mut self, invocation: usize, index: usize) -> Result<()> {
        if let Some(writer) = self.writer.filter(|&writer| writer != invocation) {
            return Err(format!(
                "shared-memory race at vec4 {index}: invocation {invocation} reads invocation {writer}'s write before a barrier"
            ));
        }
        if !self.readers.contains(&invocation) {
            self.readers.push(invocation);
        }
        Ok(())
    }

    fn write(&mut self, invocation: usize, index: usize) -> Result<()> {
        if let Some(writer) = self.writer.filter(|&writer| writer != invocation) {
            return Err(format!(
                "shared-memory race at vec4 {index}: invocations {writer} and {invocation} write before a barrier"
            ));
        }
        if let Some(reader) = self
            .readers
            .iter()
            .copied()
            .find(|&reader| reader != invocation)
        {
            return Err(format!(
                "shared-memory race at vec4 {index}: invocation {invocation} writes after invocation {reader} reads before a barrier"
            ));
        }
        self.writer = Some(invocation);
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct Definitions {
    registers: [bool; 64],
    outputs: [bool; 8],
    live: bool,
}
struct ValidationLoop {
    entry: Definitions,
    selection_path: Vec<(usize, bool)>,
    breaks: Vec<Definitions>,
}
impl Definitions {
    fn join(self, other: Self) -> Self {
        if !self.live {
            return other;
        }
        if !other.live {
            return self;
        }
        Self {
            registers: std::array::from_fn(|i| self.registers[i] && other.registers[i]),
            outputs: std::array::from_fn(|i| self.outputs[i] && other.outputs[i]),
            live: true,
        }
    }
}

type LoopMap = (Vec<Option<usize>>, Vec<Option<usize>>);
fn pair_loops(ops: &[Instruction]) -> Result<LoopMap> {
    let mut pairs = vec![None; ops.len()];
    let mut entries = vec![None; ops.len()];
    let mut starts = Vec::new();
    let mut header = None;
    for (pc, op) in ops.iter().enumerate() {
        match op {
            Instruction::LoopHeader if header.replace(pc).is_some() => {
                return Err("SIR LoopHeader without a following LoopStart".into());
            }
            Instruction::LoopHeader => header = Some(pc),
            Instruction::LoopStart { .. } => {
                if starts.len() == 64 {
                    return Err("SIR loop nesting exceeds 64".into());
                }
                starts.push(pc);
                entries[pc] = Some(header.take().unwrap_or(pc));
            }
            Instruction::LoopEnd => {
                let start = starts.pop().ok_or("SIR LoopEnd without LoopStart")?;
                pairs[start] = Some(pc);
                pairs[pc] = Some(start);
                entries[pc] = entries[start];
            }
            _ => {}
        }
    }
    if header.is_some() {
        return Err("SIR LoopHeader without a following LoopStart".into());
    }
    if !starts.is_empty() {
        return Err("SIR unclosed loop".into());
    }
    Ok((pairs, entries))
}

impl Program {
    pub fn new(ops: Vec<Instruction>) -> Result<Self> {
        Self::validate(ops, true)
    }

    /// Validate a compute program that writes through storage rather than graphics output.
    pub(crate) fn new_compute(ops: Vec<Instruction>) -> Result<Self> {
        Self::validate(ops, false)
    }

    fn validate(ops: Vec<Instruction>, requires_output: bool) -> Result<Self> {
        if ops.is_empty() || ops.len() > 4096 {
            return Err("SIR requires 1..4096 instructions".into());
        }
        let (loop_pairs, loop_entries) = pair_loops(&ops)?;
        let mut state = Definitions {
            registers: [false; 64],
            outputs: [false; 8],
            live: true,
        };
        let mut selections: Vec<(Definitions, Option<Definitions>)> = Vec::new();
        let mut selection_ids = Vec::new();
        let mut loops: Vec<ValidationLoop> = Vec::new();
        if ops.iter().any(|op| matches!(op, Instruction::LoopBreak))
            && ops
                .iter()
                .any(|op| matches!(op, Instruction::WorkgroupBarrier))
        {
            // ponytail: barrier generations are not tracked yet; keep breaking workgroups bounded.
            return Err("SIR LoopBreak cannot share a program with a workgroup barrier".into());
        }
        let mut merging: Option<(Definitions, Definitions)> = None;
        for (pc, op) in ops.iter().enumerate() {
            if !matches!(op, Instruction::Merge { .. }) {
                merging = None;
            }
            let source = |r: u8| -> Result<()> {
                if r >= 64 || !state.registers[r as usize] {
                    Err(format!("SIR instruction {pc}: undefined register r{r}"))
                } else {
                    Ok(())
                }
            };
            let dst = match *op {
                Instruction::If { condition } => {
                    source(condition)?;
                    if selections.len() >= 64 {
                        return Err("SIR selection nesting exceeds 64".into());
                    }
                    selections.push((state, None));
                    selection_ids.push(pc);
                    None
                }
                Instruction::Else => {
                    let (entry, branch) = selections.last_mut().ok_or("SIR Else without If")?;
                    if branch.is_some() {
                        return Err("SIR duplicate Else".into());
                    }
                    *branch = Some(state);
                    state = *entry;
                    None
                }
                Instruction::EndIf => {
                    let (_, branch) = selections.pop().ok_or("SIR EndIf without If")?;
                    let branch = branch.ok_or("SIR If requires Else before EndIf")?;
                    selection_ids.pop();
                    merging = Some((branch, state));
                    state = branch.join(state);
                    None
                }
                Instruction::Merge { dst, a, b } => {
                    let (yes, no) =
                        merging.ok_or("SIR Merge must immediately follow a selection")?;
                    if a >= 64
                        || b >= 64
                        || (yes.live && !yes.registers[a as usize])
                        || (no.live && !no.registers[b as usize])
                    {
                        return Err(format!("SIR instruction {pc}: undefined selection input"));
                    }
                    Some(dst)
                }
                Instruction::LoopStart { condition } => {
                    source(condition)?;
                    loops.push(ValidationLoop {
                        entry: state,
                        selection_path: selection_ids
                            .iter()
                            .zip(&selections)
                            .map(|(&id, (_, branch))| (id, branch.is_some()))
                            .collect(),
                        breaks: Vec::new(),
                    });
                    None
                }
                Instruction::Move { dst, src } => {
                    source(src)?;
                    Some(dst)
                }
                Instruction::LoopHeader => None,
                Instruction::LoopEnd => {
                    let validation_loop = loops.pop().ok_or("SIR LoopEnd without LoopStart")?;
                    if validation_loop.selection_path
                        != selection_ids
                            .iter()
                            .zip(&selections)
                            .map(|(&id, (_, branch))| (id, branch.is_some()))
                            .collect::<Vec<_>>()
                    {
                        return Err(format!(
                            "SIR instruction {pc}: loop crosses a selection boundary"
                        ));
                    }
                    state = validation_loop.entry.join(state);
                    for broken in validation_loop.breaks {
                        state = state.join(broken);
                    }
                    None
                }
                Instruction::LoopBreak => {
                    let Some(validation_loop) = loops.last_mut() else {
                        return Err(format!("SIR instruction {pc}: LoopBreak without LoopStart"));
                    };
                    validation_loop.breaks.push(state);
                    state.live = false;
                    None
                }
                Instruction::Return => {
                    if requires_output && state.live && !state.outputs[0] {
                        return Err(
                            "SIR Return must follow output slot 0 on every live path".into()
                        );
                    }
                    state.live = false;
                    None
                }
                Instruction::Discard => {
                    state.live = false;
                    None
                }
                Instruction::Input { dst, slot } => {
                    if slot >= 16 {
                        return Err(format!("SIR instruction {pc}: input slot exceeds 15"));
                    }
                    Some(dst)
                }
                Instruction::StorageLoad { dst, buffer, index } => {
                    source(index)?;
                    if buffer >= 16 {
                        return Err(format!("SIR instruction {pc}: storage buffer exceeds 15"));
                    }
                    Some(dst)
                }
                Instruction::StorageStore { index, src } => {
                    source(index)?;
                    source(src)?;
                    None
                }
                Instruction::AtomicAdd {
                    dst,
                    buffer,
                    index,
                    value,
                }
                | Instruction::AtomicExchange {
                    dst,
                    buffer,
                    index,
                    value,
                } => {
                    source(index)?;
                    source(value)?;
                    if buffer >= 16 {
                        return Err(format!("SIR instruction {pc}: atomic buffer exceeds 15"));
                    }
                    Some(dst)
                }
                Instruction::AtomicCompareExchange {
                    dst,
                    buffer,
                    index,
                    expected,
                    replacement,
                } => {
                    source(index)?;
                    source(expected)?;
                    source(replacement)?;
                    if buffer >= 16 {
                        return Err(format!("SIR instruction {pc}: atomic buffer exceeds 15"));
                    }
                    Some(dst)
                }
                Instruction::SharedLoad { dst, index } => {
                    source(index)?;
                    Some(dst)
                }
                Instruction::SharedStore { index, src } => {
                    source(index)?;
                    source(src)?;
                    None
                }
                Instruction::WorkgroupBarrier => None,
                Instruction::Uniform { dst, slot } => {
                    if slot >= 64 {
                        return Err("SIR uniform slot exceeds 63".into());
                    }
                    Some(dst)
                }
                Instruction::Const { dst, value } => {
                    if !value.is_finite() {
                        return Err("SIR constant must be finite".into());
                    }
                    Some(dst)
                }
                Instruction::Add { dst, a, b }
                | Instruction::Sub { dst, a, b }
                | Instruction::Mul { dst, a, b }
                | Instruction::Div { dst, a, b }
                | Instruction::Dot3 { dst, a, b }
                | Instruction::Dot4 { dst, a, b }
                | Instruction::Min { dst, a, b }
                | Instruction::Max { dst, a, b }
                | Instruction::Pow { dst, a, b } => {
                    source(a)?;
                    source(b)?;
                    Some(dst)
                }
                Instruction::Math { dst, src, .. } => {
                    source(src)?;
                    Some(dst)
                }
                Instruction::Compare { dst, a, b, .. } | Instruction::Logical { dst, a, b, .. } => {
                    source(a)?;
                    source(b)?;
                    Some(dst)
                }
                Instruction::Select {
                    dst,
                    condition,
                    a,
                    b,
                } => {
                    source(condition)?;
                    source(a)?;
                    source(b)?;
                    Some(dst)
                }
                Instruction::Mix { dst, a, b, t } => {
                    source(a)?;
                    source(b)?;
                    source(t)?;
                    Some(dst)
                }
                Instruction::Length {
                    dst,
                    src,
                    components,
                }
                | Instruction::Normalize {
                    dst,
                    src,
                    components,
                } => {
                    source(src)?;
                    if !(1..=4).contains(&components) {
                        return Err("SIR vector component count requires 1..4".into());
                    }
                    Some(dst)
                }
                Instruction::Normalize3 { dst, src }
                | Instruction::Saturate { dst, src }
                | Instruction::Not { dst, src }
                | Instruction::Neg { dst, src } => {
                    source(src)?;
                    Some(dst)
                }
                Instruction::Swizzle { dst, src, lanes } => {
                    source(src)?;
                    if lanes.iter().any(|&i| i >= 4) {
                        return Err("SIR swizzle lane exceeds 3".into());
                    }
                    Some(dst)
                }
                Instruction::Compose {
                    dst,
                    sources,
                    lanes,
                } => {
                    for src in sources {
                        source(src)?;
                    }
                    if lanes.iter().any(|&i| i >= 4) {
                        return Err("SIR compose lane exceeds 3".into());
                    }
                    Some(dst)
                }
                Instruction::Mat4 { dst, src, uniform } => {
                    source(src)?;
                    if uniform > 60 {
                        return Err("SIR matrix exceeds uniform range".into());
                    }
                    Some(dst)
                }
                Instruction::Sample { dst, uv, texture }
                | Instruction::SampleImplicit { dst, uv, texture }
                | Instruction::SampleArray { dst, uv, texture }
                | Instruction::SampleArrayImplicit { dst, uv, texture } => {
                    source(uv)?;
                    if texture >= 16 {
                        return Err("SIR texture slot exceeds 15".into());
                    }
                    Some(dst)
                }
                Instruction::Sample3D {
                    dst,
                    coordinate,
                    texture,
                }
                | Instruction::Sample3DImplicit {
                    dst,
                    coordinate,
                    texture,
                } => {
                    source(coordinate)?;
                    if texture >= 16 {
                        return Err("SIR texture slot exceeds 15".into());
                    }
                    Some(dst)
                }
                Instruction::SampleCube {
                    dst,
                    direction,
                    texture,
                }
                | Instruction::SampleCubeImplicit {
                    dst,
                    direction,
                    texture,
                } => {
                    source(direction)?;
                    if texture >= 16 {
                        return Err("SIR texture slot exceeds 15".into());
                    }
                    Some(dst)
                }
                Instruction::Output { slot, src } => {
                    source(src)?;
                    if slot >= 8 {
                        return Err("SIR output slot exceeds 7".into());
                    }
                    state.outputs[slot as usize] = true;
                    None
                }
            };
            if let Some(dst) = dst {
                if dst >= 64 {
                    return Err(format!("SIR instruction {pc}: register exceeds r63"));
                }
                state.registers[dst as usize] = true;
            }
        }
        if !selections.is_empty() {
            return Err("SIR unclosed selection".into());
        }
        if !loops.is_empty() {
            return Err("SIR unclosed loop".into());
        }
        if requires_output && state.live && !state.outputs[0] {
            return Err("SIR must write output slot 0 on every live path".into());
        }
        Ok(Self {
            ops,
            loop_pairs,
            loop_entries,
        })
    }
    pub fn instructions(&self) -> &[Instruction] {
        &self.ops
    }
    pub fn execute<S: FnMut(usize, Vec4) -> Result<Vec4>>(
        &self,
        inputs: &[Vec4],
        uniforms: &[Vec4],
        sample: S,
        tracing: bool,
    ) -> Result<Execution> {
        self.execute_with_lod(inputs, uniforms, &[], sample, tracing)
    }
    pub fn execute_with_lod<S: FnMut(usize, Vec4) -> Result<Vec4>>(
        &self,
        inputs: &[Vec4],
        uniforms: &[Vec4],
        implicit_lods: &[f32],
        mut sample: S,
        tracing: bool,
    ) -> Result<Execution> {
        self.execute_with_lod_and_storage(
            inputs,
            uniforms,
            implicit_lods,
            &mut sample,
            |_, _| Err("SIR storage operation used outside compute".into()),
            |_, _| Err("SIR storage operation used outside compute".into()),
            tracing,
        )
    }
    /// Run all local invocations through synchronized shared-memory barriers.
    pub fn execute_workgroup<L, W, A>(
        &self,
        inputs: &[&[Vec4]],
        shared: &mut [Vec4],
        mut load_storage: L,
        mut store_storage: W,
        mut atomic_storage: A,
    ) -> Result<Vec<Execution>>
    where
        L: FnMut(usize, usize, usize) -> Result<Vec4>,
        W: FnMut(usize, usize, Vec4) -> Result<()>,
        A: FnMut(usize, usize, usize, AtomicOperation) -> Result<Vec4>,
    {
        if inputs.is_empty() || inputs.len() > MAX_WORKGROUP_INVOCATIONS {
            return Err("SIR workgroup requires 1..1024 local invocations".into());
        }
        let mut states: Vec<_> = (0..inputs.len()).map(|_| ExecutionState::new()).collect();
        let mut accesses = HashMap::new();
        loop {
            let (mut barrier_pc, mut barrier_count, mut complete_count) = (None, 0, 0);
            for (local_id, state) in states.iter_mut().enumerate() {
                let mut sample = |_, _| Err("compute programs do not sample textures".into());
                let mut load = |buffer, index| load_storage(local_id, buffer, index);
                let mut store = |index, value| store_storage(local_id, index, value);
                let mut atomic =
                    |buffer, index, operation| atomic_storage(local_id, buffer, index, operation);
                match self.execute_until_barrier(
                    state,
                    inputs[local_id],
                    &[],
                    &[],
                    shared,
                    &mut accesses,
                    local_id,
                    &mut sample,
                    &mut load,
                    &mut store,
                    &mut atomic,
                    false,
                )? {
                    ExecutionStatus::Complete => complete_count += 1,
                    ExecutionStatus::Barrier(pc) => {
                        if let Some(expected) = barrier_pc
                            && expected != pc
                        {
                            return Err(format!(
                                "workgroup barrier divergence: invocations reached instructions {expected} and {pc}"
                            ));
                        }
                        barrier_pc = Some(pc);
                        barrier_count += 1;
                    }
                }
            }
            if complete_count == states.len() {
                return Ok(states.into_iter().map(|state| state.result).collect());
            }
            if barrier_count == states.len() {
                accesses.clear();
                continue;
            }
            return Err(format!(
                "workgroup barrier divergence: {complete_count} invocations completed and {barrier_count} reached instruction {}",
                barrier_pc.map_or_else(|| "none".to_string(), |pc| pc.to_string())
            ));
        }
    }
    /// Execute with explicit compute storage access handlers.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_with_lod_and_storage<
        S: FnMut(usize, Vec4) -> Result<Vec4>,
        L: FnMut(usize, usize) -> Result<Vec4>,
        W: FnMut(usize, Vec4) -> Result<()>,
    >(
        &self,
        inputs: &[Vec4],
        uniforms: &[Vec4],
        implicit_lods: &[f32],
        sample: S,
        load_storage: L,
        store_storage: W,
        tracing: bool,
    ) -> Result<Execution> {
        self.execute_with_lod_storage_and_atomics(
            inputs,
            uniforms,
            implicit_lods,
            sample,
            load_storage,
            store_storage,
            |_, _, _| Err("SIR atomic operation used outside compute".into()),
            tracing,
        )
    }

    /// Execute with explicit compute storage and atomic access handlers.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_with_lod_storage_and_atomics<
        S: FnMut(usize, Vec4) -> Result<Vec4>,
        L: FnMut(usize, usize) -> Result<Vec4>,
        W: FnMut(usize, Vec4) -> Result<()>,
        A: FnMut(usize, usize, AtomicOperation) -> Result<Vec4>,
    >(
        &self,
        inputs: &[Vec4],
        uniforms: &[Vec4],
        implicit_lods: &[f32],
        sample: S,
        load_storage: L,
        store_storage: W,
        atomic_storage: A,
        tracing: bool,
    ) -> Result<Execution> {
        let mut state = ExecutionState::new();
        let mut shared = [];
        let mut accesses = HashMap::new();
        match self.execute_until_barrier(
            &mut state,
            inputs,
            uniforms,
            implicit_lods,
            &mut shared,
            &mut accesses,
            0,
            sample,
            load_storage,
            store_storage,
            atomic_storage,
            tracing,
        )? {
            ExecutionStatus::Complete => Ok(state.result),
            ExecutionStatus::Barrier(pc) => Err(format!(
                "SIR instruction {pc}: workgroup barrier used outside compute workgroup execution"
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_until_barrier<
        S: FnMut(usize, Vec4) -> Result<Vec4>,
        L: FnMut(usize, usize) -> Result<Vec4>,
        W: FnMut(usize, Vec4) -> Result<()>,
        A: FnMut(usize, usize, AtomicOperation) -> Result<Vec4>,
    >(
        &self,
        state: &mut ExecutionState,
        inputs: &[Vec4],
        uniforms: &[Vec4],
        implicit_lods: &[f32],
        shared: &mut [Vec4],
        accesses: &mut HashMap<usize, SharedAccess>,
        local_id: usize,
        mut sample: S,
        mut load_storage: L,
        mut store_storage: W,
        mut atomic_storage: A,
        tracing: bool,
    ) -> Result<ExecutionStatus> {
        let mut regs = state.registers;
        let mut result = std::mem::replace(&mut state.result, empty_execution());
        let (mut active, mut live, mut choice) = (state.active, state.live, state.choice);
        let mut selections = std::mem::take(&mut state.selections);
        let mut loops = std::mem::take(&mut state.loops);
        let mut dynamic_instructions = state.dynamic_instructions;
        let mut next_pc = state.pc;
        while next_pc < self.ops.len() {
            if dynamic_instructions == MAX_DYNAMIC_INSTRUCTIONS {
                return Err(format!(
                    "SIR dynamic instruction limit ({MAX_DYNAMIC_INSTRUCTIONS}) exceeded"
                ));
            }
            dynamic_instructions += 1;
            let pc = next_pc;
            next_pc += 1;
            let op = &self.ops[pc];
            let executing = if matches!(op, Instruction::Else | Instruction::EndIf) {
                selections.last().is_some_and(|&(parent, _)| parent && live)
            } else {
                active
            };
            if !executing
                && !matches!(
                    op,
                    Instruction::If { .. } | Instruction::Else | Instruction::EndIf
                )
            {
                continue;
            }
            let get = |slot: u8| -> Result<Vec4> {
                uniforms
                    .get(slot as usize)
                    .copied()
                    .ok_or_else(|| format!("SIR instruction {pc}: missing uniform {slot}"))
            };
            let (dst, value) = match *op {
                Instruction::If { condition } => {
                    let value = if executing {
                        regs[condition as usize]
                    } else {
                        Vec4::ZERO
                    };
                    let yes = executing && value.x != 0.;
                    selections.push((active, yes));
                    active = yes;
                    (None, value)
                }
                Instruction::Else => {
                    let (parent, yes) = *selections.last().unwrap();
                    active = parent && !yes && live;
                    (None, Vec4::ZERO)
                }
                Instruction::EndIf => {
                    let (parent, yes) = selections.pop().unwrap();
                    active = parent && live;
                    choice = yes;
                    (None, Vec4::ZERO)
                }
                Instruction::Merge { dst, a, b } => {
                    (Some(dst), regs[if choice { a } else { b } as usize])
                }
                Instruction::Move { dst, src } => (Some(dst), regs[src as usize]),
                Instruction::LoopHeader => (None, Vec4::ZERO),
                Instruction::LoopStart { condition } => {
                    let end = self.loop_pairs[pc].expect("validated loop pair");
                    let value = regs[condition as usize];
                    if value.x == 0. {
                        if loops.last().is_some_and(|frame| frame.start == pc) {
                            loops.pop();
                        }
                        next_pc = end + 1;
                    } else if loops.last().is_none_or(|frame| frame.start != pc) {
                        loops.push(LoopFrame {
                            start: pc,
                            end,
                            selection_depth: selections.len(),
                            active,
                            choice,
                        });
                    }
                    (None, value)
                }
                Instruction::LoopEnd => {
                    let frame = *loops.last().ok_or("SIR LoopEnd without active loop")?;
                    if frame.end != pc {
                        return Err(format!("SIR instruction {pc}: mismatched LoopEnd"));
                    }
                    next_pc = self.loop_entries[pc].expect("validated loop entry");
                    (None, Vec4::ZERO)
                }
                Instruction::LoopBreak => {
                    let frame = loops.pop().ok_or("SIR LoopBreak without active loop")?;
                    next_pc = frame.end + 1;
                    selections.truncate(frame.selection_depth);
                    active = frame.active && live;
                    choice = frame.choice;
                    (None, Vec4::ZERO)
                }
                Instruction::Return | Instruction::Discard => {
                    result.discarded = matches!(op, Instruction::Discard);
                    live = false;
                    active = false;
                    next_pc = self.ops.len();
                    (None, Vec4::ZERO)
                }
                Instruction::Compare { dst, a, b, kind } => (
                    Some(dst),
                    Vec4::from_array(std::array::from_fn(|i| {
                        u8::from(kind.apply(
                            regs[a as usize].to_array()[i],
                            regs[b as usize].to_array()[i],
                        )) as f32
                    })),
                ),
                Instruction::Logical { dst, a, b, kind } => (
                    Some(dst),
                    Vec4::from_array(std::array::from_fn(|i| {
                        u8::from(kind.apply(
                            regs[a as usize].to_array()[i],
                            regs[b as usize].to_array()[i],
                        )) as f32
                    })),
                ),
                Instruction::Not { dst, src } => (
                    Some(dst),
                    Vec4::from_array(
                        regs[src as usize]
                            .to_array()
                            .map(|v| u8::from(v == 0.) as f32),
                    ),
                ),
                Instruction::Select {
                    dst,
                    condition,
                    a,
                    b,
                } => (
                    Some(dst),
                    Vec4::from_array(std::array::from_fn(|i| {
                        regs[if regs[condition as usize].to_array()[i] != 0. {
                            a
                        } else {
                            b
                        } as usize]
                            .to_array()[i]
                    })),
                ),
                Instruction::Input { dst, slot } => (
                    Some(dst),
                    inputs
                        .get(slot as usize)
                        .copied()
                        .ok_or_else(|| format!("SIR instruction {pc}: missing input {slot}"))?,
                ),
                Instruction::StorageLoad { dst, buffer, index } => {
                    let index = storage_index(regs[index as usize], pc)?;
                    (
                        Some(dst),
                        load_storage(buffer as usize, index)
                            .map_err(|e| format!("SIR instruction {pc}: {e}"))?,
                    )
                }
                Instruction::StorageStore { index, src } => {
                    let index = storage_index(regs[index as usize], pc)?;
                    let value = regs[src as usize];
                    store_storage(index, value)
                        .map_err(|e| format!("SIR instruction {pc}: {e}"))?;
                    (None, value)
                }
                Instruction::AtomicAdd {
                    dst,
                    buffer,
                    index,
                    value,
                } => {
                    let index = storage_index(regs[index as usize], pc)?;
                    let value = regs[value as usize].x;
                    let old = atomic_storage(buffer as usize, index, AtomicOperation::Add(value))
                        .map_err(|e| format!("SIR instruction {pc}: {e}"))?;
                    (Some(dst), old)
                }
                Instruction::AtomicExchange {
                    dst,
                    buffer,
                    index,
                    value,
                } => {
                    let index = storage_index(regs[index as usize], pc)?;
                    let value = regs[value as usize].x;
                    let old =
                        atomic_storage(buffer as usize, index, AtomicOperation::Exchange(value))
                            .map_err(|e| format!("SIR instruction {pc}: {e}"))?;
                    (Some(dst), old)
                }
                Instruction::AtomicCompareExchange {
                    dst,
                    buffer,
                    index,
                    expected,
                    replacement,
                } => {
                    let index = storage_index(regs[index as usize], pc)?;
                    let expected = regs[expected as usize].x;
                    let replacement = regs[replacement as usize].x;
                    let old = atomic_storage(
                        buffer as usize,
                        index,
                        AtomicOperation::CompareExchange {
                            expected,
                            replacement,
                        },
                    )
                    .map_err(|e| format!("SIR instruction {pc}: {e}"))?;
                    (Some(dst), old)
                }
                Instruction::SharedLoad { dst, index } => {
                    let index = vector_index(regs[index as usize], pc, "shared-memory")?;
                    let value = shared.get(index).copied().ok_or_else(|| {
                        format!(
                            "SIR instruction {pc}: shared-memory vec4 {index} is out of bounds for length {}",
                            shared.len()
                        )
                    })?;
                    accesses
                        .entry(index)
                        .or_default()
                        .read(local_id, index)
                        .map_err(|e| format!("SIR instruction {pc}: {e}"))?;
                    (Some(dst), value)
                }
                Instruction::SharedStore { index, src } => {
                    let index = vector_index(regs[index as usize], pc, "shared-memory")?;
                    if index >= shared.len() {
                        return Err(format!(
                            "SIR instruction {pc}: shared-memory vec4 {index} is out of bounds for length {}",
                            shared.len()
                        ));
                    }
                    accesses
                        .entry(index)
                        .or_default()
                        .write(local_id, index)
                        .map_err(|e| format!("SIR instruction {pc}: {e}"))?;
                    let value = regs[src as usize];
                    shared[index] = value;
                    (None, value)
                }
                Instruction::WorkgroupBarrier => (None, Vec4::ZERO),
                Instruction::Uniform { dst, slot } => (Some(dst), get(slot)?),
                Instruction::Const { dst, value } => (Some(dst), value),
                Instruction::Neg { dst, src } => (
                    Some(dst),
                    Vec4::from_array(
                        regs[src as usize]
                            .to_array()
                            .map(|v| f32::from_bits(v.to_bits() ^ 0x8000_0000)),
                    ),
                ),
                Instruction::Math {
                    dst,
                    src,
                    operation,
                } => (
                    Some(dst),
                    Vec4::from_array(regs[src as usize].to_array().map(|v| operation.apply(v))),
                ),
                Instruction::Add { dst, a, b } => (Some(dst), regs[a as usize] + regs[b as usize]),
                Instruction::Sub { dst, a, b } => (Some(dst), regs[a as usize] - regs[b as usize]),
                Instruction::Mul { dst, a, b } => {
                    (Some(dst), regs[a as usize].component_mul(regs[b as usize]))
                }
                Instruction::Div { dst, a, b } => {
                    let a = regs[a as usize].to_array();
                    let b = regs[b as usize].to_array();
                    (
                        Some(dst),
                        Vec4::from_array(std::array::from_fn(|i| a[i] / b[i])),
                    )
                }
                Instruction::Dot3 { dst, a, b } => {
                    let d = regs[a as usize].xyz().dot(regs[b as usize].xyz());
                    (Some(dst), Vec4::new(d, d, d, d))
                }
                Instruction::Dot4 { dst, a, b } => {
                    let d = regs[a as usize].dot(regs[b as usize]);
                    (Some(dst), Vec4::new(d, d, d, d))
                }
                Instruction::Pow { dst, a, b } => {
                    let a = regs[a as usize].to_array();
                    let b = regs[b as usize].to_array();
                    (
                        Some(dst),
                        Vec4::from_array(std::array::from_fn(|i| a[i].powf(b[i]))),
                    )
                }
                Instruction::Min { dst, a, b } | Instruction::Max { dst, a, b } => {
                    let a = regs[a as usize].to_array();
                    let b = regs[b as usize].to_array();
                    (
                        Some(dst),
                        Vec4::from_array(std::array::from_fn(|i| {
                            if matches!(op, Instruction::Min { .. }) {
                                a[i].min(b[i])
                            } else {
                                a[i].max(b[i])
                            }
                        })),
                    )
                }
                Instruction::Mix { dst, a, b, t } => {
                    let a = regs[a as usize].to_array();
                    let b = regs[b as usize].to_array();
                    let t = regs[t as usize].to_array();
                    (
                        Some(dst),
                        Vec4::from_array(std::array::from_fn(|i| a[i] * (1. - t[i]) + b[i] * t[i])),
                    )
                }
                Instruction::Length {
                    dst,
                    src,
                    components,
                }
                | Instruction::Normalize {
                    dst,
                    src,
                    components,
                } => {
                    let v = regs[src as usize].to_array();
                    let length = v[..components as usize]
                        .iter()
                        .map(|x| x * x)
                        .sum::<f32>()
                        .sqrt();
                    let value = if matches!(op, Instruction::Length { .. }) {
                        Vec4::new(length, length, length, length)
                    } else {
                        Vec4::from_array(std::array::from_fn(|i| {
                            if length > 0. && (i < components as usize || components == 1) {
                                v[if components == 1 { 0 } else { i }] / length
                            } else {
                                0.
                            }
                        }))
                    };
                    (Some(dst), value)
                }
                Instruction::Normalize3 { dst, src } => (
                    Some(dst),
                    regs[src as usize]
                        .xyz()
                        .normalize()
                        .extend(regs[src as usize].w),
                ),
                Instruction::Saturate { dst, src } => (
                    Some(dst),
                    Vec4::from_array(regs[src as usize].to_array().map(|v| v.clamp(0., 1.))),
                ),
                Instruction::Swizzle { dst, src, lanes } => {
                    let v = regs[src as usize].to_array();
                    (Some(dst), Vec4::from_array(lanes.map(|i| v[i as usize])))
                }
                Instruction::Compose {
                    dst,
                    sources,
                    lanes,
                } => (
                    Some(dst),
                    Vec4::from_array(std::array::from_fn(|i| {
                        regs[sources[i] as usize].to_array()[lanes[i] as usize]
                    })),
                ),
                Instruction::Mat4 { dst, src, uniform } => {
                    let v = regs[src as usize];
                    (
                        Some(dst),
                        Vec4::new(
                            get(uniform)?.dot(v),
                            get(uniform + 1)?.dot(v),
                            get(uniform + 2)?.dot(v),
                            get(uniform + 3)?.dot(v),
                        ),
                    )
                }
                Instruction::Sample { dst, uv, texture }
                | Instruction::SampleImplicit { dst, uv, texture } => {
                    result.samples += 1;
                    let mut coordinate = regs[uv as usize];
                    if matches!(op, Instruction::SampleImplicit { .. }) {
                        coordinate.z = *implicit_lods.get(texture as usize).ok_or_else(|| {
                            format!(
                                "SIR instruction {pc}: missing implicit LOD for texture {texture}"
                            )
                        })?;
                        if !coordinate.z.is_finite() {
                            return Err(format!("SIR instruction {pc}: non-finite implicit LOD"));
                        }
                    }
                    (
                        Some(dst),
                        sample(texture as usize, coordinate)
                            .map_err(|e| format!("SIR instruction {pc}: {e}"))?,
                    )
                }
                Instruction::SampleArray { dst, uv, texture }
                | Instruction::SampleArrayImplicit { dst, uv, texture } => {
                    result.samples += 1;
                    let mut coordinate = regs[uv as usize];
                    if matches!(op, Instruction::SampleArrayImplicit { .. }) {
                        coordinate.w = *implicit_lods.get(texture as usize).ok_or_else(|| {
                            format!(
                                "SIR instruction {pc}: missing implicit LOD for texture {texture}"
                            )
                        })?;
                        if !coordinate.w.is_finite() {
                            return Err(format!("SIR instruction {pc}: non-finite implicit LOD"));
                        }
                    }
                    (
                        Some(dst),
                        sample(texture as usize, coordinate)
                            .map_err(|e| format!("SIR instruction {pc}: {e}"))?,
                    )
                }
                Instruction::SampleCube {
                    dst,
                    direction,
                    texture,
                }
                | Instruction::SampleCubeImplicit {
                    dst,
                    direction,
                    texture,
                } => {
                    result.samples += 1;
                    let mut coordinate = regs[direction as usize];
                    if matches!(op, Instruction::SampleCubeImplicit { .. }) {
                        coordinate.w = *implicit_lods.get(texture as usize).ok_or_else(|| {
                            format!("SIR instruction {pc}: missing implicit LOD for cube texture {texture}")
                        })?;
                        if !coordinate.w.is_finite() {
                            return Err(format!(
                                "SIR instruction {pc}: non-finite implicit LOD for cube texture {texture}"
                            ));
                        }
                    }
                    (
                        Some(dst),
                        sample(texture as usize, coordinate)
                            .map_err(|e| format!("SIR instruction {pc}: {e}"))?,
                    )
                }
                Instruction::Sample3D {
                    dst,
                    coordinate,
                    texture,
                }
                | Instruction::Sample3DImplicit {
                    dst,
                    coordinate,
                    texture,
                } => {
                    result.samples += 1;
                    let mut coordinate = regs[coordinate as usize];
                    if matches!(op, Instruction::Sample3DImplicit { .. }) {
                        coordinate.w = *implicit_lods.get(texture as usize).ok_or_else(|| {
                            format!(
                                "SIR instruction {pc}: missing implicit LOD for texture {texture}"
                            )
                        })?;
                        if !coordinate.w.is_finite() {
                            return Err(format!("SIR instruction {pc}: non-finite implicit LOD"));
                        }
                    }
                    (
                        Some(dst),
                        sample(texture as usize, coordinate)
                            .map_err(|e| format!("SIR instruction {pc}: {e}"))?,
                    )
                }
                Instruction::Output { slot, src } => {
                    let v = regs[src as usize];
                    result.outputs[slot as usize] = v;
                    (None, v)
                }
            };
            if !executing {
                continue;
            }
            result.instructions += 1;
            if !value.is_finite() {
                return Err(format!(
                    "SIR instruction {pc}: non-finite arithmetic result"
                ));
            }
            if let Some(dst) = dst {
                regs[dst as usize] = value;
            }
            if tracing {
                result.trace.push(Trace {
                    instruction: pc,
                    operation: op.clone(),
                    value,
                });
            }
            if matches!(op, Instruction::WorkgroupBarrier) {
                state.registers = regs;
                state.result = result;
                state.active = active;
                state.live = live;
                state.choice = choice;
                state.selections = selections;
                state.loops = loops;
                state.dynamic_instructions = dynamic_instructions;
                state.pc = next_pc;
                return Ok(ExecutionStatus::Barrier(pc));
            }
        }
        state.registers = regs;
        state.result = result;
        state.active = active;
        state.live = live;
        state.choice = choice;
        state.selections = selections;
        state.loops = loops;
        state.dynamic_instructions = dynamic_instructions;
        state.pc = next_pc;
        Ok(ExecutionStatus::Complete)
    }
}

fn storage_index(value: Vec4, instruction: usize) -> Result<usize> {
    vector_index(value, instruction, "storage")
}

fn vector_index(value: Vec4, instruction: usize, resource: &str) -> Result<usize> {
    let index = value.x;
    if !index.is_finite() || index < 0.0 || index.fract() != 0.0 {
        return Err(format!(
            "SIR instruction {instruction}: {resource} index must be a finite non-negative integer in x"
        ));
    }
    Ok(index as usize)
}
/// Float arithmetic across four independent SIR invocations.
pub fn packet_backend_name() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "NEON shader4"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "SSE shader4"
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        "portable shader packet4"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn executes_and_rejects_invalid_programs() {
        use Instruction::*;
        let p = Program::new(vec![
            Input { dst: 0, slot: 0 },
            Const {
                dst: 1,
                value: Vec4::new(2., 2., 2., 2.),
            },
            Mul { dst: 2, a: 0, b: 1 },
            Output { slot: 0, src: 2 },
        ])
        .unwrap();
        let e = p
            .execute(
                &[Vec4::new(1., 2., 3., 4.)],
                &[],
                |_, _| Err("no textures".into()),
                true,
            )
            .unwrap();
        assert_eq!(e.outputs[0], Vec4::new(2., 4., 6., 8.));
        assert_eq!(e.trace.len(), 4);
        assert!(Program::new(vec![Output { slot: 0, src: 64 }]).is_err());
        assert!(
            Program::new(vec![Add { dst: 1, a: 0, b: 0 }, Output { slot: 0, src: 1 }]).is_err()
        );
        assert!(p.execute(&[], &[], |_, _| Ok(Vec4::ZERO), false).is_err());

        let atomic = Program::new(vec![
            Const {
                dst: 0,
                value: Vec4::ZERO,
            },
            Const {
                dst: 1,
                value: Vec4::new(1.0, 0.0, 0.0, 0.0),
            },
            AtomicAdd {
                dst: 2,
                buffer: 0,
                index: 0,
                value: 1,
            },
            Output { slot: 0, src: 2 },
        ])
        .unwrap();
        assert!(
            atomic
                .execute(&[], &[], |_, _| Ok(Vec4::ZERO), false)
                .unwrap_err()
                .contains("atomic operation used outside compute")
        );
        assert!(
            atomic
                .execute4(
                    [&[], &[], &[], &[]],
                    &[],
                    [&[], &[], &[], &[]],
                    1,
                    |_, _, _| Ok(Vec4::ZERO),
                    [false; 4],
                )
                .unwrap_err()
                .contains("require scalar execution")
        );
    }

    #[test]
    fn loop_break_programs_cannot_contain_workgroup_barriers() {
        let error =
            Program::new_compute(vec![Instruction::WorkgroupBarrier, Instruction::LoopBreak])
                .unwrap_err();
        assert!(error.contains("LoopBreak cannot share a program with a workgroup barrier"));
    }

    #[test]
    fn unary_math_matches_scalar_and_packet_execution() {
        let program = Program::new(vec![
            Instruction::Input { dst: 0, slot: 0 },
            Instruction::Math {
                dst: 1,
                src: 0,
                operation: UnaryMath::Floor,
            },
            Instruction::Math {
                dst: 2,
                src: 0,
                operation: UnaryMath::Fract,
            },
            Instruction::Math {
                dst: 3,
                src: 0,
                operation: UnaryMath::Sin,
            },
            Instruction::Math {
                dst: 4,
                src: 0,
                operation: UnaryMath::Cos,
            },
            Instruction::Output { slot: 0, src: 1 },
            Instruction::Output { slot: 1, src: 2 },
            Instruction::Output { slot: 2, src: 3 },
            Instruction::Output { slot: 3, src: 4 },
        ])
        .unwrap();
        let inputs = [
            Vec4::new(-1.25, -0.25, 0.25, 1.2),
            Vec4::new(2.75, 1.25, -0.5, -1.1),
            Vec4::new(0.9, -3.75, 2.0, 0.3),
            Vec4::new(-2.1, 4.5, -2.5, 0.7),
        ];
        let scalar: Vec<_> = inputs
            .iter()
            .map(|&input| {
                program
                    .execute(&[input], &[], |_, _| Err("no textures".into()), false)
                    .unwrap()
                    .outputs
            })
            .collect();
        let packet = program
            .execute4(
                std::array::from_fn(|lane| std::slice::from_ref(&inputs[lane])),
                &[],
                [&[], &[], &[], &[]],
                0b1111,
                |_, _, _| Err("no textures".into()),
                [false; 4],
            )
            .unwrap();
        for lane in 0..4 {
            let source = inputs[lane].to_array();
            let expected = [
                source.map(f32::floor),
                source.map(|value| value - value.floor()),
                source.map(f32::sin),
                source.map(f32::cos),
            ];
            for output in 0..4 {
                assert_eq!(
                    packet[lane].outputs[output].to_array(),
                    scalar[lane][output].to_array()
                );
                for (actual, expected) in packet[lane].outputs[output]
                    .to_array()
                    .into_iter()
                    .zip(expected[output])
                {
                    assert!((actual - expected).abs() < 1e-6);
                }
            }
        }
        assert_eq!(scalar[0][1].x, 0.75);
    }
}
