// core/generic/vector.rs

/// A 2-element vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicVector2<T> {
    pub x: T,
    pub y: T,
}

impl<T> BasicVector2<T> {
    pub fn new(x: T, y: T) -> Self
    {
        Self { x, y}
    }

    /*
    pub fn from_array(arr: [T; 2]) -> Self
    {
        Self { x: arr[0], y: arr[1] }
    }

    pub fn to_array(vec2: Self) -> [T; 2]
    {
        [vec2.x, vec2.y]
    }
    */
}

/// A 3-element vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicVector3<T> {
    pub x: T,
    pub y: T,
    pub z: T,
}

impl<T> BasicVector3<T> {
    pub fn new(x: T, y: T, z: T) -> Self
    {
        Self { x, y, z }
    }
    /*
    pub fn from_array(arr: [T; 3]) -> Self
    {
        Self { x: arr[0], y: arr[1], z: arr[2] }
    }

    pub fn to_array(vec3: Self) -> [T; 3]
    {
        [vec3.x, vec3.y, vec3.z]
    }
    */
}

/// A 4-element vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicVector4<T> {
    pub x: T,
    pub y: T,
    pub z: T,
    pub w: T,
}

impl<T> BasicVector4<T> {
    pub fn new(x: T, y: T, z: T, w: T) -> Self
    {
        Self { x, y, z, w }
    }
    /*
    pub fn from_array(arr: [T; 4]) -> Self
    {
        Self { x: arr[0], y: arr[1], z: arr[2], w: arr[3] }
    }

    pub fn to_array(vec4: Self) -> [T; 4]
    {
        [vec4.x, vec4.y, vec4.z, vec4.w]
    }
    */
}

/// Array to BasicVector2
impl<T> From<[T; 2]> for BasicVector2<T> {
    fn from(arr: [T; 2]) -> Self
    {
        let [x, y] = arr;
        Self { x, y }
    }
}

/// BasicVector2 to array
impl<T> From<BasicVector2<T>> for [T; 2] {
    fn from(v: BasicVector2<T>) -> [T; 2]
    {
        [v.x, v.y]
    }
}

/// Array to BasicVector3
impl<T> From<[T; 3]> for BasicVector3<T> {
    fn from(arr: [T; 3]) -> Self
    {
        let [x, y, z] = arr;
        Self { x, y, z }
    }
}

/// BasicVector3 to array
impl<T> From<BasicVector3<T>> for [T; 3] {
    fn from(v: BasicVector3<T>) -> [T; 3]
    {
        [v.x, v.y, v.z]
    }
}

/// Array to BasicVector4
impl<T> From<[T; 4]> for BasicVector4<T> {
    fn from(arr: [T; 4]) -> Self
    {
        let [x, y, z, w] = arr;
        Self { x, y, z, w }
    }
}

/// BasicVector4 to array
impl<T> From<BasicVector4<T>> for [T; 4] {
    fn from(v: BasicVector4<T>) -> [T; 4]
    {
        [v.x, v.y, v.z, v.w]
    }
}

impl<T: Eq> Eq for BasicVector2<T> {}
impl<T: Eq> Eq for BasicVector3<T> {}
impl<T: Eq> Eq for BasicVector4<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_vector2()
    {
        let vec2 = BasicVector2::new(1.0, 2.0);
        assert_eq!(vec2.x, 1.0);
        assert_eq!(vec2.y, 2.0);

        let arr = [3, 4];
        let vec2_from_array: BasicVector2<i64> = arr.into();
        assert_eq!(vec2_from_array.x, 3);
        assert_eq!(vec2_from_array.y, 4);

        let array_back: [f64; 2] = vec2.into();
        assert_eq!(array_back, [1.0, 2.0]);
    }

    #[test]
    fn test_basic_vector3()
    {
        let vec3 = BasicVector3::new(1, 2, 3);
        assert_eq!(vec3.x, 1);
        assert_eq!(vec3.y, 2);
        assert_eq!(vec3.z, 3);

        let arr = [4, 5, 6];
        let vec3_from_array: BasicVector3<i64> = arr.into();
        assert_eq!(vec3_from_array.x, 4);
        assert_eq!(vec3_from_array.y, 5);
        assert_eq!(vec3_from_array.z, 6);

        let array_back: [i64; 3] = vec3.into();
        assert_eq!(array_back, [1, 2, 3]);
    }

    #[test]
    fn test_basic_vector4()
    {
        let vec4 = BasicVector4::new(1, 2, 3, 4);
        assert_eq!(vec4.x, 1);
        assert_eq!(vec4.y, 2);
        assert_eq!(vec4.z, 3);
        assert_eq!(vec4.w, 4);

        let arr = [5, 6, 7, 8];
        let vec4_from_array: BasicVector4<i64> = arr.into();
        assert_eq!(vec4_from_array.x, 5);
        assert_eq!(vec4_from_array.y, 6);
        assert_eq!(vec4_from_array.z, 7);
        assert_eq!(vec4_from_array.w, 8);

        let array_back: [i64; 4] = vec4.into();
        assert_eq!(array_back, [1, 2, 3, 4]);
    }
}

