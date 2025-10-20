use std::ops::{Add, AddAssign, Sub, SubAssign, Mul, MulAssign, Div, DivAssign, Index, IndexMut};

/// A 3-element vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector3<T> {
    pub x: T,
    pub y: T,
    pub z: T,
}

impl<T> Vector3<T> {
    pub fn new(x: T, y: T, z: T) -> Self
    {
        Self { x, y, z }
    }

    pub fn dot(&self, other: Self) -> T
    where T: Copy + Mul<Output = T> + Add<Output = T>
    {
        (self.x * other.x) + (self.y * other.y) + (self.z * other.z)
    }

    pub fn cross(&self, other: Self) -> Self
    where T: Copy + Mul<Output = T> + Sub<Output = T>
    {
        Self {
            x: (self.y * other.z) - (self.z * other.y),
            y: (self.z * other.x) - (self.x * other.z),
            z: (self.x * other.y) - (self.y * other.x),
        }
    }
}

/// Array to Vector3
impl<T> From<[T; 3]> for Vector3<T> {
    fn from(arr: [T; 3]) -> Self
    {
        let [x, y, z] = arr;
        Self { x, y, z }
    }
}
/// Vector3 to array
impl<T> From<Vector3<T>> for [T; 3] {
    fn from(v: Vector3<T>) -> [T; 3]
    {
        [v.x, v.y, v.z]
    }
}

/// Arithmetic operation `Add`
impl<T: Add<Output = T>> Add for Vector3<T> {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}
impl<T: AddAssign> AddAssign for Vector3<T> {
    fn add_assign(&mut self, rhs: Self)
    {
        self.x += rhs.x;
        self.y += rhs.y;
        self.z += rhs.z;
    }
}

/// Arithmetic operation `Sub`
impl<T: Sub<Output = T>> Sub for Vector3<T> {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}
impl<T: SubAssign> SubAssign for Vector3<T> {
    fn sub_assign(&mut self, rhs: Self)
    {
        self.x -= rhs.x;
        self.y -= rhs.y;
        self.z -= rhs.z;
    }
}

/// Arithmetic operation `Mul`
impl<T: Mul<Output = T>> Mul for Vector3<T> {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x * rhs.x,
            y: self.y * rhs.y,
            z: self.z * rhs.z,
        }
    }
}
impl<T: MulAssign> MulAssign for Vector3<T> {
    fn mul_assign(&mut self, rhs: Self)
    {
        self.x *= rhs.x;
        self.y *= rhs.y;
        self.z *= rhs.z;
    }
}

/// Arithmetic operation `Div`
impl<T: Div<Output = T>> Div for Vector3<T> {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output
    {
        Self {
            x: self.x / rhs.x,
            y: self.y / rhs.y,
            z: self.z / rhs.z,
        }
    }
}
impl<T: DivAssign> DivAssign for Vector3<T> {
    fn div_assign(&mut self, rhs: Self)
    {
        self.x /= rhs.x;
        self.y /= rhs.y;
        self.z /= rhs.z;
    }
}

/// Indexing Vector3
impl<T> Index<usize> for Vector3<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output
    {
        match index {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            _ => panic!("Index out of bounds for Vector2")
        }
    }
}
impl<T> IndexMut<usize> for Vector3<T> {
    //type Output = T;

    fn index_mut(&mut self, index: usize) -> &mut T
    {
        match index {
            0 => &mut self.x,
            1 => &mut self.y,
            2 => &mut self.z,
            _ => panic!("Index out of bounds for Vector2")
        }
    }
}

impl<T: Eq> Eq for Vector3<T> {}

#[cfg(test)]
mod tests {
    use crate::core::generic::vector::Vector3;

    #[test]
    fn test_vector3_initialization() {
        let vec3 = Vector3::new(1, 2, 3);
        assert_eq!(vec3.x, 1);
        assert_eq!(vec3.y, 2);
        assert_eq!(vec3.z, 3);
    }

    #[test]
    fn test_vector3_from_array() {
        let arr = [4, 5, 6];
        let vec3_from_array: Vector3<i64> = arr.into();
        assert_eq!(vec3_from_array.x, 4);
        assert_eq!(vec3_from_array.y, 5);
        assert_eq!(vec3_from_array.z, 6);

        let array_back: [i64; 3] = vec3_from_array.into();
        assert_eq!(array_back, [4, 5, 6]);
    }

    #[test]
    fn test_addition() {
        let a = Vector3::new(1, 2, 3);
        let b = Vector3::new(4, 5, 6);
        let result = a + b;
        assert_eq!(result, Vector3::new(5, 7, 9));
    }

    #[test]
    fn test_subtraction() {
        let a = Vector3::new(7, 8, 9);
        let b = Vector3::new(3, 2, 1);
        let result = a - b;
        assert_eq!(result, Vector3::new(4, 6, 8));
    }

    #[test]
    fn test_dot_product() {
        let a = Vector3::new(1, 2, 3);
        let b = Vector3::new(4, 5, 6);
        let result = a.dot(b);
        assert_eq!(result, 32); // (1*4) + (2*5) + (3*6) = 32
    }

    #[test]
    fn test_cross_product() {
        let a = Vector3::new(1, 2, 3);
        let b = Vector3::new(4, 5, 6);
        let result = a.cross(b);
        assert_eq!(result, Vector3::new(-3, 6, -3)); // Cross product calculation
    }

    #[test]
    fn test_add_assign() {
        let mut a = Vector3::new(1, 2, 3);
        let b = Vector3::new(4, 5, 6);
        a += b;
        assert_eq!(a, Vector3::new(5, 7, 9));
    }

    #[test]
    fn test_sub_assign() {
        let mut a = Vector3::new(7, 8, 9);
        let b = Vector3::new(3, 2, 1);
        a -= b;
        assert_eq!(a, Vector3::new(4, 6, 8));
    }

    #[test]
    fn test_mul_assign() {
        let mut a = Vector3::new(2, 3, 4);
        let b = Vector3::new(5, 6, 7);
        a *= b;
        assert_eq!(a, Vector3::new(10, 18, 28)); // Element-wise multiplication
    }

    #[test]
    fn test_div() {
        let a = Vector3::new(10, 20, 30);
        let b = Vector3::new(2, 5, 10);
        let result = a / b;
        assert_eq!(result, Vector3::new(5, 4, 3)); // Element-wise division
    }

    #[test]
    fn test_div_assign() {
        let mut a = Vector3::new(10, 20, 30);
        let b = Vector3::new(2, 5, 10);
         a /= b;
        assert_eq!(a, Vector3::new(5, 4, 3)); // Element-wise division
    }
}
