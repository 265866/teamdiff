//! Number conversions with the results of `as` casts, for numbers read from JSON responses.

/// `n` as a `u32`, keeping its low 32 bits as an `as` cast does.
#[expect(
    clippy::cast_possible_truncation,
    reason = "counts from Riot and tracker.gg fit in 32 bits; one that doesn't comes from a \
              malformed response and keeps its low bits, as in every earlier release, and the \
              sums it goes into wrap rather than abort"
)]
pub(crate) fn low_u32(n: u64) -> u32 {
    n as u32
}

/// `n` as an `i32`, keeping its low 32 bits as an `as` cast does.
#[expect(
    clippy::cast_possible_truncation,
    reason = "RR changes fit in 32 bits; one that doesn't comes from a malformed response and \
              keeps its low bits, as in every earlier release"
)]
pub(crate) fn low_i32(n: i64) -> i32 {
    n as i32
}

/// `x` rounded to the nearest `u32`: an `as` cast saturates, so a negative value is 0, one past
/// `u32::MAX` is `u32::MAX`, and NaN is 0.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the saturating float-to-integer cast is the clamp these averages, percentages and \
              tracker.gg values need, with NaN read as 0"
)]
pub(crate) fn round_u32(x: f64) -> u32 {
    x.round() as u32
}
