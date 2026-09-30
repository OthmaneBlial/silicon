//! SIR: a bounded, validated vec4 register machine shared by both shader stages.
use serde::{Deserialize, Serialize};
use silicon_math::Vec4;
mod lanes;
mod packet;
pub mod spirv;
pub type Result<T> = std::result::Result<T, String>;
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
    Output {
        slot: u8,
        src: u8,
    },
    Neg {
        dst: u8,
        src: u8,
    },
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
            If { condition } => {
                *condition = f(*condition, false)?;
                return Ok(());
            }
            Else | EndIf | Return | Discard => return Ok(()),
            Normalize3 { dst, src }
            | Not { dst, src }
            | Neg { dst, src }
            | Saturate { dst, src }
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
            Sample { dst, uv, .. } | SampleImplicit { dst, uv, .. } => {
                *uv = f(*uv, false)?;
                dst
            }
            Output { src, .. } => {
                *src = f(*src, false)?;
                return Ok(());
            }
        };
        *dst = f(*dst, true)?;
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "Vec<Instruction>", into = "Vec<Instruction>")]
pub struct Program {
    ops: Vec<Instruction>,
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
#[derive(Clone, Copy)]
struct Definitions {
    registers: [bool; 64],
    outputs: [bool; 8],
    live: bool,
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
impl Program {
    pub fn new(ops: Vec<Instruction>) -> Result<Self> {
        if ops.is_empty() || ops.len() > 4096 {
            return Err("SIR requires 1..4096 instructions".into());
        }
        let mut state = Definitions {
            registers: [false; 64],
            outputs: [false; 8],
            live: true,
        };
        let mut selections: Vec<(Definitions, Option<Definitions>)> = Vec::new();
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
                Instruction::Return => {
                    if state.live && !state.outputs[0] {
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
                | Instruction::SampleImplicit { dst, uv, texture } => {
                    source(uv)?;
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
        if state.live && !state.outputs[0] {
            return Err("SIR must write output slot 0 on every live path".into());
        }
        Ok(Self { ops })
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
        let mut regs = [Vec4::ZERO; 64];
        let mut result = Execution {
            outputs: [Vec4::ZERO; 8],
            instructions: 0,
            samples: 0,
            trace: Vec::new(),
            discarded: false,
        };
        let (mut active, mut live, mut choice) = (true, true, false);
        let mut selections = Vec::new();
        for (pc, op) in self.ops.iter().enumerate() {
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
                Instruction::Return | Instruction::Discard => {
                    result.discarded = matches!(op, Instruction::Discard);
                    live = false;
                    active = false;
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
        }
        Ok(result)
    }
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
    }
}
