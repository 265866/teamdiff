//! Number conversions with the results of `as` casts, for screen positions and colors.

/// `n` as a `u16`, keeping its low 16 bits as an `as` cast does.
#[expect(
    clippy::cast_possible_truncation,
    reason = "row counts, text widths and offsets on a screen stay far below 65 536, and a \
              terminal's own size is a u16, so nothing larger could be drawn anyway"
)]
pub(crate) const fn low_u16(n: usize) -> u16 {
    n as u16
}

/// `ms` as an `f64`, rounding to the nearest value an `f64` holds as an `as` cast does.
#[expect(
    clippy::cast_precision_loss,
    reason = "the animations pass a phase within one cycle of a few seconds, which an f64 holds \
              exactly"
)]
pub(crate) fn millis_f64(ms: u128) -> f64 {
    ms as f64
}

/// `x` as a `u8`, rounded toward zero and saturating as an `as` cast does.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a mix of two color channels lies between them, from 0 to 255, and is truncated \
              toward zero as the fade always drew it"
)]
pub(crate) fn truncate_u8(x: f64) -> u8 {
    x as u8
}

/// `x` rounded to the nearest `usize`, saturating as an `as` cast does.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bar lengths come from percentages from 0 to 100, so they are small and never \
              negative"
)]
pub(crate) fn round_usize(x: f64) -> usize {
    x.round() as usize
}
