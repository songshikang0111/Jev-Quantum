//! Jev Quantum core: TypeSafe-compatible protocol, fast PRNG, and decision engine.
//!
//! Random numbers are **not** cryptographically secure. Do not use this crate
//! for keys, lotteries, or security tokens.

pub mod engine;
pub mod pool;
pub mod protocol;
pub mod rng;

pub use engine::{DecisionEngine, EngineConfig, RngMode};
pub use protocol::{
    Answer, ApiError, ErrorBody, ModelInfo, ModelsResponse, ProtocolError, Question,
    SystemOneRequest, SystemOneResponse, Usage,
};
pub use rng::{detect_backend, Backend};

/// SplitMix64 mixer used to derive independent streams from a base seed.
#[inline]
pub fn mix_seed(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Combine a base seed with a stream identifier.
#[inline]
pub fn stream_seed(base: u64, stream: u64) -> u64 {
    mix_seed(base ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// Cheap process-wide seed when the caller does not supply one.
pub fn process_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xA5A5_A5A5_A5A5_A5A5);
    mix_seed(nanos ^ (std::process::id() as u64).wrapping_mul(0xD1B5_4A32_D192_ED03))
}
