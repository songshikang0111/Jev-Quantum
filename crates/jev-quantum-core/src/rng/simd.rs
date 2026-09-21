use super::scalar::Xoshiro256PlusPlus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Scalar,
    Avx2,
    Neon,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            Self::Avx2 => "avx2",
            Self::Neon => "neon",
        }
    }
}

pub fn detect_backend() -> Backend {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            Backend::Avx2
        } else {
            Backend::Scalar
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        Backend::Neon
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        Backend::Scalar
    }
}

/// Fill `out` from independent xoshiro streams, using SIMD when available.
pub fn fill_u64s(out: &mut [u64], seeder: &mut Xoshiro256PlusPlus) {
    match detect_backend() {
        Backend::Avx2 => fill_avx2(out, seeder),
        Backend::Neon => fill_neon(out, seeder),
        Backend::Scalar => fill_scalar(out, seeder),
    }
}

fn fill_scalar(out: &mut [u64], seeder: &mut Xoshiro256PlusPlus) {
    let mut rng = Xoshiro256PlusPlus::from_seed(seeder.next_u64());
    for slot in out {
        *slot = rng.next_u64();
    }
}

fn fill_avx2(out: &mut [u64], seeder: &mut Xoshiro256PlusPlus) {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe {
                fill_avx2_inner(out, seeder);
            }
            return;
        }
    }
    fill_scalar(out, seeder);
}

fn fill_neon(out: &mut [u64], seeder: &mut Xoshiro256PlusPlus) {
    #[cfg(target_arch = "aarch64")]
    {
        fill_neon_inner(out, seeder);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        fill_scalar(out, seeder);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn fill_avx2_inner(out: &mut [u64], seeder: &mut Xoshiro256PlusPlus) {
    use std::arch::x86_64::*;

    let streams = [
        Xoshiro256PlusPlus::from_seed(seeder.next_u64()),
        Xoshiro256PlusPlus::from_seed(seeder.next_u64()),
        Xoshiro256PlusPlus::from_seed(seeder.next_u64()),
        Xoshiro256PlusPlus::from_seed(seeder.next_u64()),
    ];
    let st = [
        streams[0].state(),
        streams[1].state(),
        streams[2].state(),
        streams[3].state(),
    ];

    let mut s0 = _mm256_set_epi64x(
        st[3][0] as i64,
        st[2][0] as i64,
        st[1][0] as i64,
        st[0][0] as i64,
    );
    let mut s1 = _mm256_set_epi64x(
        st[3][1] as i64,
        st[2][1] as i64,
        st[1][1] as i64,
        st[0][1] as i64,
    );
    let mut s2 = _mm256_set_epi64x(
        st[3][2] as i64,
        st[2][2] as i64,
        st[1][2] as i64,
        st[0][2] as i64,
    );
    let mut s3 = _mm256_set_epi64x(
        st[3][3] as i64,
        st[2][3] as i64,
        st[1][3] as i64,
        st[0][3] as i64,
    );

    let mut chunks = out.chunks_exact_mut(4);
    for chunk in &mut chunks {
        let sum = _mm256_add_epi64(s0, s3);
        let rotated = rotl23_avx2(sum);
        let result = _mm256_add_epi64(rotated, s0);
        _mm256_storeu_si256(chunk.as_mut_ptr().cast(), result);

        let t = _mm256_slli_epi64(s1, 17);
        s2 = _mm256_xor_si256(s2, s0);
        s3 = _mm256_xor_si256(s3, s1);
        s1 = _mm256_xor_si256(s1, s2);
        s0 = _mm256_xor_si256(s0, s3);
        s2 = _mm256_xor_si256(s2, t);
        s3 = rotl45_avx2(s3);
    }

    let rem = chunks.into_remainder();
    if !rem.is_empty() {
        let mut tmp = [0u64; 4];
        let sum = _mm256_add_epi64(s0, s3);
        let rotated = rotl23_avx2(sum);
        let result = _mm256_add_epi64(rotated, s0);
        _mm256_storeu_si256(tmp.as_mut_ptr().cast(), result);
        rem.copy_from_slice(&tmp[..rem.len()]);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn rotl23_avx2(x: std::arch::x86_64::__m256i) -> std::arch::x86_64::__m256i {
    use std::arch::x86_64::*;
    _mm256_or_si256(_mm256_slli_epi64(x, 23), _mm256_srli_epi64(x, 41))
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn rotl45_avx2(x: std::arch::x86_64::__m256i) -> std::arch::x86_64::__m256i {
    use std::arch::x86_64::*;
    _mm256_or_si256(_mm256_slli_epi64(x, 45), _mm256_srli_epi64(x, 19))
}

#[cfg(target_arch = "aarch64")]
fn fill_neon_inner(out: &mut [u64], seeder: &mut Xoshiro256PlusPlus) {
    use std::arch::aarch64::*;

    let a = Xoshiro256PlusPlus::from_seed(seeder.next_u64());
    let b = Xoshiro256PlusPlus::from_seed(seeder.next_u64());
    let da = a.state();
    let db = b.state();

    unsafe {
        let mut s0 = vsetq_lane_u64::<1>(db[0], vdupq_n_u64(da[0]));
        let mut s1 = vsetq_lane_u64::<1>(db[1], vdupq_n_u64(da[1]));
        let mut s2 = vsetq_lane_u64::<1>(db[2], vdupq_n_u64(da[2]));
        let mut s3 = vsetq_lane_u64::<1>(db[3], vdupq_n_u64(da[3]));

        let mut chunks = out.chunks_exact_mut(2);
        for chunk in &mut chunks {
            let sum = vaddq_u64(s0, s3);
            let rotated = rotl_neon(sum, 23);
            let result = vaddq_u64(rotated, s0);
            vst1q_u64(chunk.as_mut_ptr(), result);

            let t = vshlq_n_u64::<17>(s1);
            s2 = veorq_u64(s2, s0);
            s3 = veorq_u64(s3, s1);
            s1 = veorq_u64(s1, s2);
            s0 = veorq_u64(s0, s3);
            s2 = veorq_u64(s2, t);
            s3 = rotl_neon(s3, 45);
        }

        let rem = chunks.into_remainder();
        if !rem.is_empty() {
            rem[0] = seeder.next_u64();
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
unsafe fn rotl_neon(x: std::arch::aarch64::uint64x2_t, k: i32) -> std::arch::aarch64::uint64x2_t {
    use std::arch::aarch64::*;
    let left = vshlq_u64(x, vdupq_n_s64(i64::from(k)));
    let right = vshlq_u64(x, vdupq_n_s64(i64::from(k) - 64));
    vorrq_u64(left, right)
}
