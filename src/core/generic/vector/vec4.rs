use std::ops::{Add, AddAssign, Sub, SubAssign, Mul, MulAssign, Div, DivAssign, Index, IndexMut};

/// A 4-element vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector4<T> {
    pub x: T,
    pub y: T,
    pub z: T,
    pub w: T,
}

impl<T> Vector4<T> {
    pub fn new(x: T, y: T, z: T, w: T) -> Self
    {
        Self { x, y, z, w }
    }

    pub fn dot(&self, other: Self) -> T
    where T: Copy + Mul<Output = T> + Add<Output = T>
    {
        (self.x * other.x) + (self.y * other.y) + (self.z * other.z) + (self.w * other.w)
    }
}

/// Array to Vector4
impl<T> From<[T; 4]> for Vector4<T> {
    fn from(arr: [T; 4]) -> Self
    {
        let [x, y, z, w] = arr;
        Self { x, y, z, w }
    }
}
/// Vector4 to array
impl<T> From<Vector4<T>> for [T; 4] {
    fn from(v: Vector4<T>) -> [T; 4]
    {
        [v.x, v.y, v.z, v.w]
    }
}

/// Arithmetic operation `Add`
impl<T: Add<Output = T>> Add for Vector4<T> {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
            w: self.w + rhs.w,
        }
    }
}
impl<T: AddAssign> AddAssign for Vector4<T> {
    fn add_assign(&mut self, rhs: Self)
    {
        self.x += rhs.x;
        self.y += rhs.y;
        self.z += rhs.z;
        self.w += rhs.w;
    }
}

/// Arithmetic operation `Sub`
impl<T: Sub<Output = T>> Sub for Vector4<T> {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
            w: self.w - rhs.w,
        }
    }
}
impl<T: SubAssign> SubAssign for Vector4<T> {
    fn sub_assign(&mut self, rhs: Self)
    {
        self.x -= rhs.x;
        self.y -= rhs.y;
        self.z -= rhs.z;
        self.w -= rhs.w;
    }
}

/// Arithmetic operation `Mul`
impl<T: Mul<Output = T>> Mul for Vector4<T> {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x * rhs.x,
            y: self.y * rhs.y,
            z: self.z * rhs.z,
            w: self.w * rhs.w,
        }
    }
}
impl<T: MulAssign> MulAssign for Vector4<T> {
    fn mul_assign(&mut self, rhs: Self)
    {
        self.x *= rhs.x;
        self.y *= rhs.y;
        self.z *= rhs.z;
        self.w *= rhs.w;
    }
}

/// Arithmetic operation `Div`
impl<T: Div<Output = T>> Div for Vector4<T> {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x / rhs.x,
            y: self.y / rhs.y,
            z: self.z / rhs.z,
            w: self.w / rhs.w,
        }
    }
}
impl<T: DivAssign> DivAssign for Vector4<T> {
    fn div_assign(&mut self, rhs: Self)
    {
        self.x /= rhs.x;
        self.y /= rhs.y;
        self.z /= rhs.z;
        self.w /= rhs.w;
    }
}

/// Indexing Vector3
impl<T> Index<usize> for Vector4<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output
    {
        match index {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            3 => &self.w,
            _ => panic!("Index out of bounds for Vector2")
        }
    }
}
impl<T> IndexMut<usize> for Vector4<T> {
    //type Output = T;

    fn index_mut(&mut self, index: usize) -> &mut T
    {
        match index {
            0 => &mut self.x,
            1 => &mut self.y,
            2 => &mut self.z,
            3 => &mut self.w,
            _ => panic!("Index out of bounds for Vector2")
        }
    }
}

impl<T: Eq> Eq for Vector4<T> {}

#[cfg(test)]
mod tests {
    use crate::core::generic::vector::Vector4;

    #[test]
    fn test_vector4_initialization() {
        let vec4 = Vector4::new(1, 2, 3, 4);
        assert_eq!(vec4.x, 1);
        assert_eq!(vec4.y, 2);
        assert_eq!(vec4.z, 3);
        assert_eq!(vec4.w, 4);
    }

    #[test]
    fn test_vector4_from_array() {
        let arr = [5.0, 6.0, 7.0, 8.0];
        let vec4_from_array: Vector4<f64> = arr.into();
        assert_eq!(vec4_from_array.x, 5.0);
        assert_eq!(vec4_from_array.y, 6.0);
        assert_eq!(vec4_from_array.z, 7.0);
        assert_eq!(vec4_from_array.w, 8.0);

        let array_back: [f64; 4] = vec4_from_array.into();
        assert_eq!(array_back, [5.0, 6.0, 7.0, 8.0]);
    }

    #[test]
    fn test_addition() {
        let a = Vector4::new(1, 2, 3, 4);
        let b = Vector4::new(5, 6, 7, 8);
        let result = a + b;
        assert_eq!(result, Vector4::new(6, 8, 10, 12));
    }

    #[test]
    fn test_subtraction() {
        let a = Vector4::new(10, 9, 8, 7);
        let b = Vector4::new(1, 2, 3, 4);
        let result = a - b;
        assert_eq!(result, Vector4::new(9, 7, 5, 3));
    }

    #[test]
    fn test_dot_product() {
        let a = Vector4::new(1, 2, 3, 4);
        let b = Vector4::new(5, 6, 7, 8);
        let result = a.dot(b);
        assert_eq!(result, 70); // (1*5) + (2*6) + (3*7) + (4*8) = 70
    }

    #[test]
    fn test_add_assign() {
        let mut a = Vector4::new(1, 2, 3, 4);
        let b = Vector4::new(5, 6, 7, 8);
        a += b;
        assert_eq!(a, Vector4::new(6, 8, 10, 12));
    }

    #[test]
    fn test_sub_assign() {
        let mut a = Vector4::new(10, 10, 10, 10);
        let b = Vector4::new(1, 2, 3, 4);
        a -= b;
        assert_eq!(a, Vector4::new(9, 8, 7, 6));
    }

    #[test]
    fn test_mul_assign() {
        let mut a = Vector4::new(1, 1, 1, 1);
        let b = Vector4::new(2, 3, 4, 5);
        a *= b;
        assert_eq!(a, Vector4::new(2, 3, 4, 5)); // Element-wise multiplication
    }

    #[test]
    fn test_div() {
        let a = Vector4::new(8, 8, 8, 8);
        let b = Vector4::new(2, 4, 2, 4);
        let result = a / b;
        assert_eq!(result, Vector4::new(4, 2, 4, 2)); // Element-wise division
    }

    #[test]
    fn test_div_assign() {
        let mut a = Vector4::new(8, 8, 8, 8);
        let b = Vector4::new(2, 4, 2, 4);
        a /= b;
        assert_eq!(a, Vector4::new(4, 2, 4, 2)); // Element-wise division
    }
}
