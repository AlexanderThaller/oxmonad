//! Identifiers and geometry shared by the whole model.
//!
//! Rectangles use river's logical coordinate space. Fractions use an exact
//! [`Ratio`] so the ported xmonad arithmetic (`floor (fromIntegral w * f)`
//! on Haskell `Rational`s) rounds exactly like the original.

use serde::{
    Deserialize,
    Serialize,
};
use std::{
    cmp::Ordering,
    fmt,
    ops::{
        Add,
        Mul,
        Neg,
        Sub,
    },
};

/// Opaque identifier of a managed window, assigned by the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u64);

/// Opaque identifier of a screen (output), assigned by the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenId(pub u64);

/// A width/height pair in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Size {
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
}

impl Size {
    /// Creates a size.
    #[must_use]
    pub const fn new(w: i32, h: i32) -> Self {
        Self { w, h }
    }
}

/// An axis-aligned rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub w: i32,
    /// Height.
    pub h: i32,
}

impl Rect {
    /// Creates a rectangle.
    #[must_use]
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Whether the point lies inside the rectangle (right/bottom edges
    /// excluded).
    #[must_use]
    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    /// The centre point, rounded towards the top-left.
    #[must_use]
    pub const fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    /// Shrinks the rectangle by `by` pixels on every edge, never below a
    /// 1x1 size.
    #[must_use]
    pub fn inset(&self, by: i32) -> Self {
        Self {
            x: self.x + by,
            y: self.y + by,
            w: (self.w - 2 * by).max(1),
            h: (self.h - 2 * by).max(1),
        }
    }

    /// Grows the rectangle by `by` pixels on every edge (the inverse of
    /// [`Self::inset`] for rectangles larger than `2 * by`).
    #[must_use]
    pub const fn outset(&self, by: i32) -> Self {
        Self {
            x: self.x - by,
            y: self.y - by,
            w: self.w + 2 * by,
            h: self.h + 2 * by,
        }
    }

    /// xmonad's `mirrorRect`: swaps the axes.
    #[must_use]
    pub const fn transposed(&self) -> Self {
        Self {
            x: self.y,
            y: self.x,
            w: self.h,
            h: self.w,
        }
    }

    /// A rectangle of `size` centred in `self`.
    #[must_use]
    pub fn centered(&self, size: Size) -> Self {
        let w = size.w.clamp(1, self.w.max(1));
        let h = size.h.clamp(1, self.h.max(1));
        Self {
            x: self.x + (self.w - w) / 2,
            y: self.y + (self.h - h) / 2,
            w,
            h,
        }
    }
}

/// An exact rational number, always stored reduced with a positive
/// denominator (a small stand-in for Haskell's `Rational`). Serialized as
/// `[numerator, denominator]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "(i64, i64)", try_from = "(i64, i64)")]
pub struct Ratio {
    num: i64,
    den: i64,
}

impl From<Ratio> for (i64, i64) {
    fn from(ratio: Ratio) -> Self {
        (ratio.num, ratio.den)
    }
}

impl TryFrom<(i64, i64)> for Ratio {
    type Error = &'static str;

    fn try_from((num, den): (i64, i64)) -> Result<Self, Self::Error> {
        if den == 0 {
            Err("ratio with zero denominator")
        } else if num == i64::MIN || den == i64::MIN {
            Err("ratio out of range")
        } else {
            Ok(Self::new(num, den))
        }
    }
}

impl Ratio {
    /// Zero.
    pub const ZERO: Self = Self { num: 0, den: 1 };
    /// One.
    pub const ONE: Self = Self { num: 1, den: 1 };

    /// Creates `num / den`, reduced.
    ///
    /// # Panics
    ///
    /// Panics if `den` is zero.
    #[must_use]
    pub fn new(num: i64, den: i64) -> Self {
        assert!(den != 0, "Ratio with zero denominator");
        let sign = if den < 0 { -1 } else { 1 };
        let g = gcd(num.unsigned_abs(), den.unsigned_abs()).max(1);
        let g = i64::try_from(g).unwrap_or(1);
        Self {
            num: sign * num / g,
            den: sign * den / g,
        }
    }

    /// An integer as a ratio.
    #[must_use]
    pub const fn from_int(n: i64) -> Self {
        Self { num: n, den: 1 }
    }

    /// The closest ratio with denominator 1,000,000 to `value` (used for
    /// fractions written as decimals in the config).
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "config fractions are small; the value is rounded first"
    )]
    pub fn from_f64(value: f64) -> Self {
        const DEN: i64 = 1_000_000;
        let num = (value * 1_000_000.0).round() as i64;
        Self::new(num, DEN)
    }

    /// The numerator.
    #[must_use]
    pub const fn numer(self) -> i64 {
        self.num
    }

    /// The (positive) denominator.
    #[must_use]
    pub const fn denom(self) -> i64 {
        self.den
    }

    /// Haskell's `floor`: the greatest integer not above the value.
    #[must_use]
    pub const fn floor(self) -> i64 {
        self.num.div_euclid(self.den)
    }

    /// Haskell's `ceiling`: the least integer not below the value.
    #[must_use]
    pub const fn ceil(self) -> i64 {
        -((-self.num).div_euclid(self.den))
    }

    /// Whether the value is negative.
    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.num < 0
    }

    /// Approximates the ratio as a float (for display only).
    #[must_use]
    #[expect(clippy::cast_precision_loss, reason = "display-only approximation")]
    pub fn to_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// `floor (fromIntegral n * self)` clamped into `i32`.
    #[must_use]
    pub fn floor_mul(self, n: i32) -> i32 {
        clamp_i32((self * Self::from_int(i64::from(n))).floor())
    }

    /// `ceiling (fromIntegral n * self)` clamped into `i32`.
    #[must_use]
    pub fn ceil_mul(self, n: i32) -> i32 {
        clamp_i32((self * Self::from_int(i64::from(n))).ceil())
    }
}

impl Default for Ratio {
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

impl Add for Ratio {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.num * rhs.den + rhs.num * self.den, self.den * rhs.den)
    }
}

impl Sub for Ratio {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        self + (-rhs)
    }
}

impl Neg for Ratio {
    type Output = Self;

    fn neg(self) -> Self {
        Self {
            num: -self.num,
            den: self.den,
        }
    }
}

impl Mul for Ratio {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        Self::new(self.num * rhs.num, self.den * rhs.den)
    }
}

impl PartialOrd for Ratio {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ratio {
    fn cmp(&self, other: &Self) -> Ordering {
        (i128::from(self.num) * i128::from(other.den))
            .cmp(&(i128::from(other.num) * i128::from(self.den)))
    }
}

/// A rectangle expressed as fractions of a screen (xmonad's
/// `RationalRect`), so floating windows keep their relative position when
/// their workspace moves to a screen of a different size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RationalRect {
    /// Left edge as a fraction of the screen width.
    pub x: Ratio,
    /// Top edge as a fraction of the screen height.
    pub y: Ratio,
    /// Width as a fraction of the screen width.
    pub w: Ratio,
    /// Height as a fraction of the screen height.
    pub h: Ratio,
}

impl RationalRect {
    /// Creates a rational rectangle.
    #[must_use]
    pub const fn new(x: Ratio, y: Ratio, w: Ratio, h: Ratio) -> Self {
        Self { x, y, w, h }
    }

    /// Expresses `rect` relative to `screen` (the inverse of
    /// [`Self::scale`]).
    #[must_use]
    pub fn relative_to(rect: Rect, screen: Rect) -> Self {
        let sw = i64::from(screen.w.max(1));
        let sh = i64::from(screen.h.max(1));
        Self {
            x: Ratio::new(i64::from(rect.x - screen.x), sw),
            y: Ratio::new(i64::from(rect.y - screen.y), sh),
            w: Ratio::new(i64::from(rect.w), sw),
            h: Ratio::new(i64::from(rect.h), sh),
        }
    }

    /// xmonad's `scaleRationalRect`: the absolute rectangle on `screen`,
    /// each component floored.
    #[must_use]
    pub fn scale(&self, screen: Rect) -> Rect {
        Rect {
            x: screen.x + self.x.floor_mul(screen.w),
            y: screen.y + self.y.floor_mul(screen.h),
            w: self.w.floor_mul(screen.w),
            h: self.h.floor_mul(screen.h),
        }
    }
}

/// Haskell's `round` for a non-negative float: round half to even
/// ("banker's rounding"), as used by xmonad-contrib's `Grid`.
#[must_use]
pub fn round_half_even(value: f64) -> i64 {
    let rounded = value.round_ties_even();
    // Layout inputs are screen-sized; the float fits in an i64.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "screen-sized values always fit"
    )]
    let result = rounded as i64;
    result
}

/// Clamps an `i64` into the `i32` range.
#[must_use]
pub fn clamp_i32(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_reduces_and_orders() {
        assert_eq!(Ratio::new(6, 10), Ratio::new(3, 5));
        assert_eq!(Ratio::new(3, -5), Ratio::new(-3, 5));
        assert!(Ratio::new(1, 3) < Ratio::new(1, 2));
        assert_eq!(Ratio::new(3, 5) + Ratio::new(3, 100), Ratio::new(63, 100));
        assert_eq!(Ratio::new(3, 5) - Ratio::new(3, 100), Ratio::new(57, 100));
    }

    #[test]
    fn ratio_serializes_as_a_pair() {
        let json = serde_json::to_string(&Ratio::new(3, 5)).expect("serializes");
        assert_eq!(json, "[3,5]");
        let read: Ratio = serde_json::from_str("[6,-10]").expect("deserializes");
        assert_eq!(read, Ratio::new(-3, 5));
        assert_eq!((read.numer(), read.denom()), (-3, 5));
        assert!(serde_json::from_str::<Ratio>("[1,0]").is_err());
        assert!(serde_json::from_str::<Ratio>(&format!("[{},1]", i64::MIN)).is_err());
        assert!(serde_json::from_str::<Ratio>(&format!("[1,{}]", i64::MIN)).is_err());
        assert!(serde_json::from_str::<Ratio>("[1]").is_err());
        let rect = RationalRect::new(
            Ratio::new(1, 4),
            Ratio::new(1, 3),
            Ratio::new(1, 2),
            Ratio::ONE,
        );
        let json = serde_json::to_string(&rect).expect("serializes");
        assert_eq!(serde_json::from_str::<RationalRect>(&json).ok(), Some(rect));
    }

    #[test]
    fn ratio_floor_and_ceiling_match_haskell() {
        assert_eq!(Ratio::new(7, 2).floor(), 3);
        assert_eq!(Ratio::new(-7, 2).floor(), -4);
        assert_eq!(Ratio::new(7, 2).ceil(), 4);
        assert_eq!(Ratio::new(-7, 2).ceil(), -3);
        assert_eq!(Ratio::new(3, 5).floor_mul(1920), 1152);
        assert_eq!(Ratio::new(1, 3).ceil_mul(1000), 334);
    }

    #[test]
    fn rational_rect_round_trips() {
        let screen = Rect::new(1920, 0, 2560, 1440);
        let rect = Rect::new(2000, 100, 777, 333);
        let rr = RationalRect::relative_to(rect, screen);
        assert_eq!(rr.scale(screen), rect);
    }

    #[test]
    fn banker_rounding() {
        assert_eq!(round_half_even(2.5), 2);
        assert_eq!(round_half_even(3.5), 4);
        assert_eq!(round_half_even(1.2), 1);
    }

    #[test]
    fn rect_helpers() {
        let r = Rect::new(0, 0, 100, 50);
        assert!(r.contains(99, 49));
        assert!(!r.contains(100, 0));
        assert_eq!(r.inset(1), Rect::new(1, 1, 98, 48));
        assert_eq!(r.centered(Size::new(20, 10)), Rect::new(40, 20, 20, 10));
        assert_eq!(r.transposed(), Rect::new(0, 0, 50, 100));
    }
}
