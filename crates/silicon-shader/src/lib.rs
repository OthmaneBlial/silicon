//! SIR: a bounded, validated vec4 register machine shared by both shader stages.
use serde::{Deserialize, Serialize};
use silicon_math::Vec4;
pub mod spirv;
pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Instruction {
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
}
impl Program {
    pub fn new(ops: Vec<Instruction>) -> Result<Self> {
        if ops.is_empty() || ops.len() > 4096 {
            return Err("SIR requires 1..4096 instructions".into());
        }
        let mut defined = [false; 64];
        let mut outputs = [false; 8];
        for (pc, op) in ops.iter().enumerate() {
            let source = |r: u8| -> Result<()> {
                if r >= 64 || !defined[r as usize] {
                    Err(format!("SIR instruction {pc}: undefined register r{r}"))
                } else {
                    Ok(())
                }
            };
            let dst = match *op {
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
                | Instruction::Pow { dst, a, b } => {
                    source(a)?;
                    source(b)?;
                    Some(dst)
                }
                Instruction::Normalize3 { dst, src } | Instruction::Saturate { dst, src } => {
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
                    outputs[slot as usize] = true;
                    None
                }
            };
            if let Some(dst) = dst {
                if dst >= 64 {
                    return Err(format!("SIR instruction {pc}: register exceeds r63"));
                }
                defined[dst as usize] = true;
            }
        }
        if !outputs[0] {
            return Err("SIR must write output slot 0".into());
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
            instructions: self.ops.len(),
            samples: 0,
            trace: Vec::new(),
        };
        for (pc, op) in self.ops.iter().enumerate() {
            let get = |slot: u8| -> Result<Vec4> {
                uniforms
                    .get(slot as usize)
                    .copied()
                    .ok_or_else(|| format!("SIR instruction {pc}: missing uniform {slot}"))
            };
            let (dst, value) = match *op {
                Instruction::Input { dst, slot } => (
                    Some(dst),
                    inputs
                        .get(slot as usize)
                        .copied()
                        .ok_or_else(|| format!("SIR instruction {pc}: missing input {slot}"))?,
                ),
                Instruction::Uniform { dst, slot } => (Some(dst), get(slot)?),
                Instruction::Const { dst, value } => (Some(dst), value),
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
