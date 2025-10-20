// core/math/vector.rs

use crate::core::generic::vector::{Vector2, Vector3, Vector4};

/// Returns true if `self_` is equal to `other` within `epsilon`.
pub fn float_equal_epsilon(self_: f64, other: f64, epsilon: f64) -> bool
{
    (other - self_).abs() < epsilon
}

/// Returns the value midway between `self` and `other`.
pub fn float_mid(self_: f64, other: f64) -> f64
{
    (self_ + other) * 0.5
}

/// Returns `f` rounded to the nearest integer. Note that this is not the same behaviour as casting from float to int.
pub fn float_to_int(f: f64) -> i64
{
    f.round() as i64
}

/// Returns `f` rounded to the nearest multiple of `snap`.
pub fn float_snapped(f: f64, snap: f64) -> f64
{
    if snap == 0.0 {
        f
    }
    else {
        (f / snap).round() * snap
    }
}

/// Returns `f` rounded to zero if less than `snap`.
pub fn float_snapped_to_zero(f: f64, snap: f64) -> f64
{
    if f.abs() < snap {
        0.0
    }
    else {
        f
    }
}

/// Returns true if `f` has no decimal fraction part.
pub fn float_is_int(f: f64) -> bool
{
    f == f.round()
}

/// Returns `f` modulated by the range [0, `modulus`)
/// `f` must be in the range [`-modulus`, `modulus`)
pub fn float_mod_range(f: f64, modulus: f64) -> f64
{
    if f < 0.0 {
        f + modulus
    }
    else {
        f
    }
}

/// Returns `f` modulated by the range [0, `modulus`)
pub fn float_mod(f: f64, modulus: f64) -> f64
{
    if modulus == 0.0 {
        f  // TODO: define what should happen; C++ fmod gives NaN or undefined?
    } else {
        let r = f % modulus;
        float_mod_range(r, modulus)
    }
}

/// Approximate equality
impl Vector2<f64> {
    pub fn equal_eps(&self, o: &Self, e: f64) -> bool
    {
        float_equal_epsilon(self.x, o.x, e) && float_equal_epsilon(self.y, o.y, e)
    }
}
/// Approximate equality
impl Vector3<f64> {
    pub fn equal_eps(&self, o: &Self, e: f64) -> bool
    {
        float_equal_epsilon(self.x, o.x, e) && float_equal_epsilon(self.y, o.y, e) &&
        float_equal_epsilon(self.z, o.z, e)
    }
}
/// Approximate equality
impl Vector4<f64> {
    pub fn equal_eps(&self, o: &Self, e: f64) -> bool
    {
        float_equal_epsilon(self.x, o.x, e) && float_equal_epsilon(self.y, o.y, e) &&
        float_equal_epsilon(self.z, o.z, e) && float_equal_epsilon(self.w, o.w, e)
    }
}
