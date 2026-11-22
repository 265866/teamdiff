//! Number conversions with the results of `as` casts, for numbers read from JSON responses.

/// `n` as a `u32`, keeping its low 32 bits as an `as` cast does.
#[expect(
    clippy::cast_possible_truncation,
    reason = "counts from Riot and tracker.gg fit in 32 bits, and one that doesn't comes from a \
              malformed response, which keeps its low bits rather than failing the whole response"
)]
pub(crate) fn low_u32(n: u64) -> u32 {
    n as u32
}
