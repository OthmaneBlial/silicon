//! Four adjacent coverage lanes; fixed-point i64 edges stay bit-exact to scalar.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Backend {
    #[default]
    Scalar,
    Simd,
}
pub fn name(backend: Backend) -> &'static str {
    if backend == Backend::Scalar {
        return "scalar";
    }
    #[cfg(target_arch = "aarch64")]
    {
        "NEON coverage4"
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            "AVX2 coverage4"
        } else {
            "scalar (AVX2 unavailable)"
        }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        "scalar (no SIMD backend)"
    }
}
pub fn coverage4(base: [i64; 3], step: [i64; 3], inclusive: [bool; 3], backend: Backend) -> u8 {
    if backend == Backend::Simd {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: NEON is mandatory on aarch64. The helper only loads fixed arrays.
        unsafe {
            return neon(base, step, inclusive);
        }
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx2") {
            // SAFETY: AVX2 checked at runtime; helper has no guest-address loads.
            unsafe {
                return avx2(base, step, inclusive);
            }
        }
    }
    let mut mask = 0;
    for lane in 0..4 {
        if (0..3).all(|i| base[i] + step[i] * lane >= i64::from(!inclusive[i])) {
            mask |= 1 << lane;
        }
    }
    mask
}
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn neon(base: [i64; 3], step: [i64; 3], inclusive: [bool; 3]) -> u8 {
    use std::arch::aarch64::*;
    // SAFETY: every load/store has two valid lanes. Edge arithmetic was bounded
    // by framebuffer limits and six-plane clipping before reaching this function.
    unsafe {
        let mut lo = vdupq_n_u64(u64::MAX);
        let mut hi = lo;
        for i in 0..3 {
            let a = [base[i], base[i] + step[i]];
            let b = [base[i] + step[i] * 2, base[i] + step[i] * 3];
            let bias = vdupq_n_s64(i64::from(!inclusive[i]));
            lo = vandq_u64(lo, vcgeq_s64(vld1q_s64(a.as_ptr()), bias));
            hi = vandq_u64(hi, vcgeq_s64(vld1q_s64(b.as_ptr()), bias));
        }
        let mut lanes = [0u64; 4];
        vst1q_u64(lanes.as_mut_ptr(), lo);
        vst1q_u64(lanes.as_mut_ptr().add(2), hi);
        lanes
            .iter()
            .enumerate()
            .fold(0, |m, (i, &v)| m | ((v != 0) as u8) << i)
    }
}
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn avx2(base: [i64; 3], step: [i64; 3], inclusive: [bool; 3]) -> u8 {
    use std::arch::x86_64::*;
    let mut mask = _mm256_set1_epi64x(-1);
    for i in 0..3 {
        let values = _mm256_setr_epi64x(
            base[i],
            base[i] + step[i],
            base[i] + step[i] * 2,
            base[i] + step[i] * 3,
        );
        let bias = _mm256_set1_epi64x(if inclusive[i] { -1 } else { 0 });
        mask = _mm256_and_si256(mask, _mm256_cmpgt_epi64(values, bias));
    }
    _mm256_movemask_pd(_mm256_castsi256_pd(mask)) as u8
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lanes_match_reference() {
        for x in -64..64 {
            for s in -5..5 {
                for bits in 0..8 {
                    let b = [x, x * 2, -x];
                    let step = [s, -s, s * 2];
                    let inclusive = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
                    assert_eq!(
                        coverage4(b, step, inclusive, Backend::Scalar),
                        coverage4(b, step, inclusive, Backend::Simd)
                    );
                }
            }
        }
    }
}
