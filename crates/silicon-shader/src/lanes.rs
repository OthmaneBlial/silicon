//! Four independent float lanes. Only fixed-size host arrays reach intrinsics.
use std::ops::{Add, Div, Mul, Neg, Sub};
#[derive(Clone, Copy, Default)]
pub(crate) struct Lanes(pub [f32; 4]);
/// One bit per fragment containing an infinity or NaN in any component.
#[inline]
pub(crate) fn non_finite(value: [Lanes; 4]) -> u8 {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: mandatory NEON; every load accesses a fixed four-element array.
    unsafe {
        use std::arch::aarch64::*;
        let exponent = vdupq_n_u32(0x7f80_0000);
        let mut invalid = vdupq_n_u32(0);
        for component in value {
            let bits = vreinterpretq_u32_f32(vld1q_f32(component.0.as_ptr()));
            invalid = vorrq_u32(invalid, vceqq_u32(vandq_u32(bits, exponent), exponent));
        }
        vaddvq_u32(vandq_u32(invalid, vld1q_u32([1, 2, 4, 8].as_ptr()))) as u8
    }
    #[cfg(target_arch = "x86_64")]
    // SAFETY: SSE2 is mandatory on x86-64; unaligned loads access four floats.
    unsafe {
        use std::arch::x86_64::*;
        let exponent = _mm_set1_epi32(0x7f80_0000);
        let mut invalid = _mm_setzero_si128();
        for component in value {
            let bits = _mm_castps_si128(_mm_loadu_ps(component.0.as_ptr()));
            invalid = _mm_or_si128(
                invalid,
                _mm_cmpeq_epi32(_mm_and_si128(bits, exponent), exponent),
            );
        }
        _mm_movemask_ps(_mm_castsi128_ps(invalid)) as u8
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    (0..4).fold(0, |mask, i| {
        mask | (u8::from(value.iter().any(|v| !v.0[i].is_finite())) << i)
    })
}
impl Lanes {
    pub fn splat(v: f32) -> Self {
        Self([v; 4])
    }
    #[inline]
    pub fn sqrt(self) -> Self {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: NEON is mandatory on aarch64; load/store each access four valid floats.
        unsafe {
            use std::arch::aarch64::*;
            let mut out = [0.; 4];
            vst1q_f32(out.as_mut_ptr(), vsqrtq_f32(vld1q_f32(self.0.as_ptr())));
            Self(out)
        }
        #[cfg(target_arch = "x86_64")]
        // SAFETY: SSE is mandatory on x86-64; unaligned loads/stores access four valid floats.
        unsafe {
            use std::arch::x86_64::*;
            let mut out = [0.; 4];
            _mm_storeu_ps(out.as_mut_ptr(), _mm_sqrt_ps(_mm_loadu_ps(self.0.as_ptr())));
            Self(out)
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        Self(self.0.map(f32::sqrt))
    }
}
macro_rules! arithmetic {
    ($trait:ident, $method:ident, $operator:tt, $neon:ident, $sse:ident) => {
        impl $trait for Lanes {
            type Output = Self;
            #[inline]
            fn $method(self, rhs: Self) -> Self {
                #[cfg(target_arch = "aarch64")]
                // SAFETY: mandatory NEON; pointers belong to fixed four-float arrays.
                unsafe {
                    use std::arch::aarch64::*;
                    let mut out = [0.; 4];
                    vst1q_f32(out.as_mut_ptr(), $neon(vld1q_f32(self.0.as_ptr()), vld1q_f32(rhs.0.as_ptr())));
                    Self(out)
                }
                #[cfg(target_arch = "x86_64")]
                // SAFETY: mandatory SSE; unaligned pointers have four valid floats.
                unsafe {
                    use std::arch::x86_64::*;
                    let mut out = [0.; 4];
                    _mm_storeu_ps(out.as_mut_ptr(), $sse(_mm_loadu_ps(self.0.as_ptr()), _mm_loadu_ps(rhs.0.as_ptr())));
                    Self(out)
                }
                #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
                Self(std::array::from_fn(|i| self.0[i] $operator rhs.0[i]))
            }
        }
    };
}
arithmetic!(Add, add, +, vaddq_f32, _mm_add_ps);
arithmetic!(Sub, sub, -, vsubq_f32, _mm_sub_ps);
arithmetic!(Mul, mul, *, vmulq_f32, _mm_mul_ps);
arithmetic!(Div, div, /, vdivq_f32, _mm_div_ps);
impl Neg for Lanes {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        #[cfg(target_arch = "aarch64")]
        // SAFETY: mandatory NEON; load/store each access four valid floats.
        unsafe {
            use std::arch::aarch64::*;
            let mut out = [0.; 4];
            let bits = vreinterpretq_u32_f32(vld1q_f32(self.0.as_ptr()));
            let sign = vdupq_n_u32(0x8000_0000);
            vst1q_f32(
                out.as_mut_ptr(),
                vreinterpretq_f32_u32(veorq_u32(bits, sign)),
            );
            Self(out)
        }
        #[cfg(target_arch = "x86_64")]
        // SAFETY: SSE2 is mandatory on x86-64; unaligned load/store accesses four floats.
        unsafe {
            use std::arch::x86_64::*;
            let mut out = [0.; 4];
            let value = _mm_loadu_ps(self.0.as_ptr());
            let sign = _mm_castsi128_ps(_mm_set1_epi32(i32::MIN));
            _mm_storeu_ps(out.as_mut_ptr(), _mm_xor_ps(value, sign));
            Self(out)
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        Self(self.0.map(|v| f32::from_bits(v.to_bits() ^ 0x8000_0000)))
    }
}
