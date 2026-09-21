mod scalar;
mod simd;

pub use scalar::Xoshiro256PlusPlus;
pub use simd::{detect_backend, fill_u64s, Backend};

/// Fast entropy source used by the decision engine.
pub trait Entropy {
    fn next_u64(&mut self) -> u64;

    #[inline]
    fn next_f64(&mut self) -> f64 {
        u64_to_unit(self.next_u64())
    }

    /// Fast almost-uniform index in `0..n`. Bias is accepted for speed.
    #[inline]
    fn bounded_usize(&mut self, n: usize) -> usize {
        debug_assert!(n > 0);
        if n <= 1 {
            return 0;
        }
        if n.is_power_of_two() {
            return (self.next_u64() as usize) & (n - 1);
        }
        // Lemire high-multiply mapping.
        let x = self.next_u64();
        ((x as u128 * n as u128) >> 64) as usize
    }
}

#[inline]
pub fn u64_to_unit(x: u64) -> f64 {
    const SCALE: f64 = 1.0 / ((1u64 << 53) as f64);
    ((x >> 11) as f64) * SCALE
}

impl Entropy for Xoshiro256PlusPlus {
    #[inline]
    fn next_u64(&mut self) -> u64 {
        Xoshiro256PlusPlus::next_u64(self)
    }
}
