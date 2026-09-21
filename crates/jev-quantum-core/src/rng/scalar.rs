/// xoshiro256++ 1.0 — small state, fast, not cryptographic.
#[derive(Clone, Debug)]
pub struct Xoshiro256PlusPlus {
    s: [u64; 4],
}

impl Xoshiro256PlusPlus {
    pub fn from_seed(seed: u64) -> Self {
        let mut mixer = seed;
        let mut s = [0u64; 4];
        for slot in &mut s {
            mixer = splitmix64(mixer);
            *slot = mixer;
        }
        // Avoid the all-zero state, which is a fixed point.
        if s.iter().all(|&v| v == 0) {
            s[0] = 1;
        }
        Self { s }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[0]
            .wrapping_add(self.s[3])
            .rotate_left(23)
            .wrapping_add(self.s[0]);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    pub fn jump(&mut self) {
        const JUMP: [u64; 4] = [
            0x180E_C6D3_3CFD_0ABA,
            0xD5A6_1266_F0C9_392C,
            0xA958_2618_E03F_C2AA,
            0x39AB_DC45_29B1_66C2,
        ];
        let mut s0 = 0u64;
        let mut s1 = 0u64;
        let mut s2 = 0u64;
        let mut s3 = 0u64;
        for &jump in &JUMP {
            for bit in 0..64 {
                if (jump & (1u64 << bit)) != 0 {
                    s0 ^= self.s[0];
                    s1 ^= self.s[1];
                    s2 ^= self.s[2];
                    s3 ^= self.s[3];
                }
                self.next_u64();
            }
        }
        self.s = [s0, s1, s2, s3];
    }

    pub(crate) fn state(&self) -> [u64; 4] {
        self.s
    }
}

#[inline]
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::Xoshiro256PlusPlus;

    #[test]
    fn deterministic_sequence() {
        let mut a = Xoshiro256PlusPlus::from_seed(1);
        let mut b = Xoshiro256PlusPlus::from_seed(1);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Xoshiro256PlusPlus::from_seed(1);
        let mut b = Xoshiro256PlusPlus::from_seed(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn never_all_zero() {
        let rng = Xoshiro256PlusPlus::from_seed(0);
        assert!(rng.s.iter().any(|&v| v != 0));
    }
}
