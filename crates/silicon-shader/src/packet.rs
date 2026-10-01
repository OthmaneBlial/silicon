//! Masked SIR execution: each component register holds four independent fragments.
use crate::{Execution, Instruction, Program, Result, Trace, lanes::Lanes, storage_index};
use silicon_math::Vec4;
type Register = [Lanes; 4];
fn splat(v: Vec4) -> Register {
    v.to_array().map(Lanes::splat)
}
fn lane(v: Register, i: usize) -> Vec4 {
    Vec4::from_array(v.map(|v| v.0[i]))
}
fn count_instructions(results: &mut [Execution; 4], mask: u8, count: usize) {
    for (i, result) in results.iter_mut().enumerate() {
        if mask & (1 << i) != 0 {
            result.instructions += count;
        }
    }
}
#[inline]
fn dot(a: Register, b: Register, n: usize) -> Lanes {
    (0..n).fold(Lanes::splat(0.), |sum, i| sum + a[i] * b[i])
}
#[inline]
fn normalize(v: Register, n: usize, preserve_w: bool) -> Register {
    let length = dot(v, v, n).sqrt();
    // No reciprocal approximation or fused multiply-add: retain scalar operation order.
    let divisor = Lanes(length.0.map(|n| if n > 0. { n } else { 1. }));
    std::array::from_fn(|i| {
        if preserve_w && i == 3 {
            return v[3];
        }
        if i >= n && n != 1 {
            return Lanes::splat(0.);
        }
        let divided = v[if n == 1 { 0 } else { i }] / divisor;
        Lanes(std::array::from_fn(|j| {
            if length.0[j] > 0. { divided.0[j] } else { 0. }
        }))
    })
}
impl Program {
    /// A bit per active lane. Inactive inputs, LODs and samples are never accessed.
    /// Arithmetic spans fragments with NEON/SSE; pow/min/max and texture callbacks remain scalar.
    pub fn execute4<S: FnMut(usize, usize, Vec4) -> Result<Vec4>>(
        &self,
        inputs: [&[Vec4]; 4],
        uniforms: &[Vec4],
        implicit_lods: [&[f32]; 4],
        active: u8,
        mut sample: S,
        tracing: [bool; 4],
    ) -> Result<[Execution; 4]> {
        self.execute4_with_storage(
            inputs,
            uniforms,
            implicit_lods,
            active,
            &mut sample,
            |_, _, _| Err("SIR storage operation used outside compute".into()),
            |_, _, _| Err("SIR storage operation used outside compute".into()),
            tracing,
        )
    }

    /// Execute a packet with checked compute storage callbacks.
    #[allow(clippy::too_many_arguments)]
    pub fn execute4_with_storage<
        S: FnMut(usize, usize, Vec4) -> Result<Vec4>,
        L: FnMut(usize, usize, usize) -> Result<Vec4>,
        W: FnMut(usize, usize, Vec4) -> Result<()>,
    >(
        &self,
        inputs: [&[Vec4]; 4],
        uniforms: &[Vec4],
        implicit_lods: [&[f32]; 4],
        active: u8,
        mut sample: S,
        mut load_storage: L,
        mut store_storage: W,
        tracing: [bool; 4],
    ) -> Result<[Execution; 4]> {
        if active & !15 != 0 {
            return Err("SIR packet mask exceeds four lanes".into());
        }
        let mut results = std::array::from_fn(|_| Execution {
            outputs: [Vec4::ZERO; 8],
            instructions: 0,
            samples: 0,
            trace: Vec::new(),
            discarded: false,
        });
        if active == 0 {
            return Ok(results);
        }
        let mut regs = [[Lanes::splat(0.); 4]; 64];
        let (mut current, mut live, mut choice) = (active, active, 0u8);
        let mut selections = Vec::new();
        // Flush a run only when its mask changes, avoiding per-instruction result writes.
        let (mut counted_mask, mut count) = (active, 0);
        let trace_mask = (0..4).fold(0u8, |mask, i| mask | (u8::from(tracing[i]) << i));
        for (pc, op) in self.instructions().iter().enumerate() {
            use Instruction::*;
            let executing = if matches!(op, Else | EndIf) {
                selections.last().map_or(0, |&(parent, _)| parent & live)
            } else {
                current
            };
            if executing == 0 && !matches!(op, If { .. } | Else | EndIf) {
                continue;
            }
            let enabled = |i: usize| executing & (1 << i) != 0;
            let get = |slot: u8| -> Result<Vec4> {
                uniforms
                    .get(slot as usize)
                    .copied()
                    .ok_or_else(|| format!("SIR instruction {pc}: missing uniform {slot}"))
            };
            let (dst, value) = match *op {
                If { condition } => {
                    let value = if executing != 0 {
                        regs[condition as usize]
                    } else {
                        splat(Vec4::ZERO)
                    };
                    let yes = (0..4).fold(0u8, |mask, i| {
                        mask | (u8::from(enabled(i) && value[0].0[i] != 0.) << i)
                    });
                    selections.push((current, yes));
                    current = yes;
                    (None, value)
                }
                Else => {
                    let (parent, yes) = *selections.last().unwrap();
                    current = parent & !yes & live;
                    (None, splat(Vec4::ZERO))
                }
                EndIf => {
                    let (parent, yes) = selections.pop().unwrap();
                    current = parent & live;
                    choice = yes;
                    (None, splat(Vec4::ZERO))
                }
                Merge { dst, a, b } => (
                    Some(dst),
                    std::array::from_fn(|c| {
                        Lanes(std::array::from_fn(|i| {
                            regs[if choice & (1 << i) != 0 { a } else { b } as usize][c].0[i]
                        }))
                    }),
                ),
                Return | Discard => {
                    for (i, result) in results.iter_mut().enumerate() {
                        if enabled(i) {
                            result.discarded = matches!(op, Discard);
                        }
                    }
                    live &= !executing;
                    current = 0;
                    (None, splat(Vec4::ZERO))
                }
                Compare { dst, a, b, kind } => (
                    Some(dst),
                    std::array::from_fn(|c| {
                        Lanes(std::array::from_fn(|i| {
                            u8::from(kind.apply(regs[a as usize][c].0[i], regs[b as usize][c].0[i]))
                                as f32
                        }))
                    }),
                ),
                Logical { dst, a, b, kind } => (
                    Some(dst),
                    std::array::from_fn(|c| {
                        Lanes(std::array::from_fn(|i| {
                            u8::from(kind.apply(regs[a as usize][c].0[i], regs[b as usize][c].0[i]))
                                as f32
                        }))
                    }),
                ),
                Not { dst, src } => (
                    Some(dst),
                    regs[src as usize].map(|v| Lanes(v.0.map(|n| u8::from(n == 0.) as f32))),
                ),
                Select {
                    dst,
                    condition,
                    a,
                    b,
                } => (
                    Some(dst),
                    std::array::from_fn(|c| {
                        Lanes(std::array::from_fn(|i| {
                            regs[if regs[condition as usize][c].0[i] != 0. {
                                a
                            } else {
                                b
                            } as usize][c]
                                .0[i]
                        }))
                    }),
                ),
                Input { dst, slot } => {
                    let mut values = [Vec4::ZERO; 4];
                    for i in 0..4 {
                        if enabled(i) {
                            values[i] = *inputs[i].get(slot as usize).ok_or_else(|| {
                                format!("SIR instruction {pc}, lane {i}: missing input {slot}")
                            })?;
                        }
                    }
                    (
                        Some(dst),
                        std::array::from_fn(|component| {
                            Lanes(std::array::from_fn(|i| values[i].to_array()[component]))
                        }),
                    )
                }
                StorageLoad { dst, buffer, index } => {
                    let mut values = [Vec4::ZERO; 4];
                    for (i, value) in values.iter_mut().enumerate() {
                        if enabled(i) {
                            let index = storage_index(lane(regs[index as usize], i), pc)
                                .map_err(|e| format!("{e}, lane {i}"))?;
                            *value = load_storage(i, buffer as usize, index)
                                .map_err(|e| format!("SIR instruction {pc}, lane {i}: {e}"))?;
                        }
                    }
                    (
                        Some(dst),
                        std::array::from_fn(|c| {
                            Lanes(std::array::from_fn(|i| values[i].to_array()[c]))
                        }),
                    )
                }
                StorageStore { index, src } => {
                    for i in 0..4 {
                        if enabled(i) {
                            let index = storage_index(lane(regs[index as usize], i), pc)
                                .map_err(|e| format!("{e}, lane {i}"))?;
                            store_storage(i, index, lane(regs[src as usize], i))
                                .map_err(|e| format!("SIR instruction {pc}, lane {i}: {e}"))?;
                        }
                    }
                    (None, regs[src as usize])
                }
                AtomicAdd { .. }
                | AtomicExchange { .. }
                | AtomicCompareExchange { .. }
                | SharedLoad { .. }
                | SharedStore { .. }
                | WorkgroupBarrier => {
                    return Err(
                        "SIR compute atomics and workgroup operations require scalar execution"
                            .into(),
                    );
                }
                Uniform { dst, slot } => (Some(dst), splat(get(slot)?)),
                Const { dst, value } => (Some(dst), splat(value)),
                Neg { dst, src } => (Some(dst), std::array::from_fn(|i| -regs[src as usize][i])),
                Add { dst, a, b } => (
                    Some(dst),
                    std::array::from_fn(|i| regs[a as usize][i] + regs[b as usize][i]),
                ),
                Sub { dst, a, b } => (
                    Some(dst),
                    std::array::from_fn(|i| regs[a as usize][i] - regs[b as usize][i]),
                ),
                Mul { dst, a, b } => (
                    Some(dst),
                    std::array::from_fn(|i| regs[a as usize][i] * regs[b as usize][i]),
                ),
                Div { dst, a, b } => (
                    Some(dst),
                    std::array::from_fn(|i| regs[a as usize][i] / regs[b as usize][i]),
                ),
                Dot3 { dst, a, b } | Dot4 { dst, a, b } => (
                    Some(dst),
                    [dot(
                        regs[a as usize],
                        regs[b as usize],
                        if matches!(op, Dot3 { .. }) { 3 } else { 4 },
                    ); 4],
                ),
                Pow { dst, a, b } | Min { dst, a, b } | Max { dst, a, b } => {
                    let a = regs[a as usize];
                    let b = regs[b as usize];
                    (
                        Some(dst),
                        std::array::from_fn(|c| {
                            Lanes(std::array::from_fn(|i| {
                                if !enabled(i) {
                                    return 0.;
                                }
                                match op {
                                    Pow { .. } => a[c].0[i].powf(b[c].0[i]),
                                    Min { .. } => a[c].0[i].min(b[c].0[i]),
                                    _ => a[c].0[i].max(b[c].0[i]),
                                }
                            }))
                        }),
                    )
                }
                Mix { dst, a, b, t } => (
                    Some(dst),
                    std::array::from_fn(|i| {
                        regs[a as usize][i] * (Lanes::splat(1.) - regs[t as usize][i])
                            + regs[b as usize][i] * regs[t as usize][i]
                    }),
                ),
                Length {
                    dst,
                    src,
                    components,
                } => (
                    Some(dst),
                    [dot(regs[src as usize], regs[src as usize], components as usize).sqrt(); 4],
                ),
                Normalize {
                    dst,
                    src,
                    components,
                } => (
                    Some(dst),
                    normalize(regs[src as usize], components as usize, false),
                ),
                Normalize3 { dst, src } => (Some(dst), normalize(regs[src as usize], 3, true)),
                Saturate { dst, src } => (
                    Some(dst),
                    regs[src as usize].map(|v| Lanes(v.0.map(|v| v.clamp(0., 1.)))),
                ),
                Swizzle { dst, src, lanes } => {
                    (Some(dst), lanes.map(|i| regs[src as usize][i as usize]))
                }
                Compose {
                    dst,
                    sources,
                    lanes,
                } => (
                    Some(dst),
                    std::array::from_fn(|i| regs[sources[i] as usize][lanes[i] as usize]),
                ),
                Mat4 { dst, src, uniform } => {
                    let rows = [
                        get(uniform)?,
                        get(uniform + 1)?,
                        get(uniform + 2)?,
                        get(uniform + 3)?,
                    ];
                    (
                        Some(dst),
                        std::array::from_fn(|i| dot(splat(rows[i]), regs[src as usize], 4)),
                    )
                }
                Sample { dst, uv, texture } | SampleImplicit { dst, uv, texture } => {
                    let mut values = [Vec4::ZERO; 4];
                    for i in 0..4 {
                        if enabled(i) {
                            let mut coordinate = lane(regs[uv as usize], i);
                            if matches!(op, SampleImplicit { .. }) {
                                coordinate.z = *implicit_lods[i].get(texture as usize).ok_or_else(|| format!("SIR instruction {pc}, lane {i}: missing implicit LOD for texture {texture}"))?;
                                if !coordinate.z.is_finite() {
                                    return Err(format!(
                                        "SIR instruction {pc}, lane {i}: non-finite implicit LOD"
                                    ));
                                }
                            }
                            results[i].samples += 1;
                            values[i] = sample(i, texture as usize, coordinate)
                                .map_err(|e| format!("SIR instruction {pc}, lane {i}: {e}"))?;
                        }
                    }
                    (
                        Some(dst),
                        std::array::from_fn(|c| {
                            Lanes(std::array::from_fn(|i| values[i].to_array()[c]))
                        }),
                    )
                }
                SampleCube {
                    dst,
                    direction,
                    texture,
                }
                | SampleCubeImplicit {
                    dst,
                    direction,
                    texture,
                } => {
                    let mut values = [Vec4::ZERO; 4];
                    for i in 0..4 {
                        if enabled(i) {
                            let mut coordinate = lane(regs[direction as usize], i);
                            if matches!(op, SampleCubeImplicit { .. }) {
                                coordinate.w = *implicit_lods[i].get(texture as usize).ok_or_else(|| {
                                    format!("SIR instruction {pc}, lane {i}: missing implicit LOD for cube texture {texture}")
                                })?;
                                if !coordinate.w.is_finite() {
                                    return Err(format!(
                                        "SIR instruction {pc}, lane {i}: non-finite implicit LOD for cube texture {texture}"
                                    ));
                                }
                            }
                            results[i].samples += 1;
                            values[i] = sample(i, texture as usize, coordinate)
                                .map_err(|e| format!("SIR instruction {pc}, lane {i}: {e}"))?;
                        }
                    }
                    (
                        Some(dst),
                        std::array::from_fn(|c| {
                            Lanes(std::array::from_fn(|i| values[i].to_array()[c]))
                        }),
                    )
                }
                Output { slot, src } => {
                    for (i, result) in results.iter_mut().enumerate() {
                        if enabled(i) {
                            result.outputs[slot as usize] = lane(regs[src as usize], i);
                        }
                    }
                    (None, regs[src as usize])
                }
            };
            let invalid = crate::lanes::non_finite(value) & executing;
            if invalid != 0 {
                return Err(format!(
                    "SIR instruction {pc}, lane {}: non-finite arithmetic result",
                    invalid.trailing_zeros()
                ));
            }
            if executing != counted_mask {
                count_instructions(&mut results, counted_mask, count);
                counted_mask = executing;
                count = 0;
            }
            count += 1;
            if executing & trace_mask != 0 {
                for i in 0..4 {
                    if enabled(i) && tracing[i] {
                        results[i].trace.push(Trace {
                            instruction: pc,
                            operation: op.clone(),
                            value: lane(value, i),
                        });
                    }
                }
            }
            if let Some(dst) = dst {
                // Preserve the other live branch's register contents during divergence.
                regs[dst as usize] = if executing == live {
                    value
                } else {
                    std::array::from_fn(|c| {
                        Lanes(std::array::from_fn(|i| {
                            if enabled(i) {
                                value[c].0[i]
                            } else {
                                regs[dst as usize][c].0[i]
                            }
                        }))
                    })
                };
            }
        }
        count_instructions(&mut results, counted_mask, count);
        Ok(results)
    }
}
