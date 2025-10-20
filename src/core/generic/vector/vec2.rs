use std::ops::{Add, AddAssign, Sub, SubAssign, Mul, MulAssign, Div, DivAssign, Index, IndexMut};

/// A 2-element vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector2<T> {
    pub x: T,
    pub y: T,
}

impl<T: Mul> Vector2<T> {
    pub fn new(x: T, y: T) -> Self
    {
        Self { x, y}
    }

    pub fn dot(&self, other: Self) -> T
    where T: Copy + Mul<Output = T> + Add<Output = T>
    {
        (self.x * other.x) + (self.y * other.y)
    }

    pub fn cross(&self, other: Self) -> T
    where T: Copy + Mul<Output = T> + Sub<Output = T>
    {
        (self.x * other.y) - (self.y * other.x)
    }
}

/// Array to Vector2
impl<T> From<[T; 2]> for Vector2<T> {
    fn from(arr: [T; 2]) -> Self
    {
        let [x, y] = arr;
        Self { x, y }
    }
}
/// Vector2 to array
impl<T> From<Vector2<T>> for [T; 2] {
    fn from(v: Vector2<T>) -> [T; 2]
    {
        [v.x, v.y]
    }
}

/// Arithmetic operation `Add`
impl<T: Add<Output = T>> Add for Vector2<T> {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}
impl<T: AddAssign> AddAssign for Vector2<T> {
    fn add_assign(&mut self, rhs: Self)
    {
        self.x += rhs.x;
        self.y += rhs.y;
    }
}

/// Arithmetic operation `Sub`
impl<T: Sub<Output = T>> Sub for Vector2<T> {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}
impl<T: SubAssign> SubAssign for Vector2<T> {
    fn sub_assign(&mut self, rhs: Self)
    {
        self.x -= rhs.x;
        self.y -= rhs.y;
    }
}

/// Arithmetic operation `Mul`
impl<T: Mul<Output = T>> Mul for Vector2<T> {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x * rhs.x,
            y: self.y * rhs.y,
        }
    }
}
impl<T: MulAssign> MulAssign for Vector2<T> {
    fn mul_assign(&mut self, rhs: Self)
    {
        self.x *= rhs.x;
        self.y *= rhs.y;
    }
}

/// Arithmetic operation `Div`
impl<T: Div<Output = T>> Div for Vector2<T> {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x / rhs.x,
            y: self.y / rhs.y,
        }
    }
}
impl<T: DivAssign> DivAssign for Vector2<T> {
    fn div_assign(&mut self, rhs: Self)
    {
        self.x /= rhs.x;
        self.y /= rhs.y;
    }
}

/// Indexing Vector2
impl<T> Index<usize> for Vector2<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output
    {
        match index {
            0 => &self.x,
            1 => &self.y,
            _ => panic!("Index out of bounds for Vector2")
        }
    }
}
impl<T> IndexMut<usize> for Vector2<T> {
    //type Output = T;

    fn index_mut(&mut self, index: usize) -> &mut T
    {
        match index {
            0 => &mut self.x,
            1 => &mut self.y,
            _ => panic!("Index out of bounds for Vector2")
        }
    }
}

impl<T: Eq> Eq for Vector2<T> {}

#[cfg(test)]
mod tests {
    use crate::core::generic::vector::Vector2;

    #[test]
    fn test_vector2_initialization() {
        let vec2 = Vector2::new(1.0, 2.0);
        assert_eq!(vec2.x, 1.0);
        assert_eq!(vec2.y, 2.0);
    }

    #[test]
    fn test_vector2_from_array() {
        let arr = [3, 4];
        let vec2_from_array: Vector2<i64> = arr.into();
        assert_eq!(vec2_from_array.x, 3);
        assert_eq!(vec2_from_array.y, 4);

        let array_back: [f64; 2] = Vector2::new(1.0, 2.0).into();
        assert_eq!(array_back, [1.0, 2.0]);
    }

    #[test]
    fn test_addition() {
        let a = Vector2::new(1.0, 2.0);
        let b = Vector2::new(3.0, 4.0);
        let result = a + b;
        assert_eq!(result, Vector2::new(4.0, 6.0));
    }

    #[test]
    fn test_subtraction() {
        let a = Vector2::new(5.0, 7.0);
        let b = Vector2::new(2.0, 4.0);
        let result = a - b;
        assert_eq!(result, Vector2::new(3.0, 3.0));
    }

    #[test]
    fn test_dot_product() {
        let a = Vector2::new(1.0, 2.0);
        let b = Vector2::new(3.0, 4.0);
        let result = a.dot(b);
        assert_eq!(result, 11.0); // (1*3) + (2*4) = 11
    }

    #[test]
    fn test_cross_product() {
        let a = Vector2::new(1.0, 2.0);
        let b = Vector2::new(3.0, 4.0);
        let result = a.cross(b);
        assert_eq!(result, -2.0); // 1*4 - 2*3 = -2
    }

    #[test]
    fn test_add_assign() {
        let mut a = Vector2::new(1.0, 2.0);
        let b = Vector2::new(3.0, 4.0);
        a += b;
        assert_eq!(a, Vector2::new(4.0, 6.0));
    }

    #[test]
    fn test_sub_assign() {
        let mut a = Vector2::new(5.0, 7.0);
        let b = Vector2::new(2.0, 4.0);
        a -= b;
        assert_eq!(a, Vector2::new(3.0, 3.0));
    }

    #[test]
    fn test_mul_assign() {
        let mut a = Vector2::new(2.0, 3.0);
        let b = Vector2::new(4.0, 5.0);
        a *= b;
        assert_eq!(a, Vector2::new(8.0, 15.0)); // Element-wise multiplication
    }

    #[test]
    fn test_div() {
        let a = Vector2::new(6.0, 9.0);
        let b = Vector2::new(2.0, 3.0);
        let result = a / b;
        assert_eq!(result, Vector2::new(3.0, 3.0)); // Element-wise division
    }
}
