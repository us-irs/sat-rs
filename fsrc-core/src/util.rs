//! Utility types and enums.
//!
//! This module contains various helper types. This includes wrapper for primitive rust types
//! using the newtype pattern. This was also done for pairs and triplets of these primitive types.
//! The [ToBeBytes] was implemented for those types as well, which allows to easily convert them
//! into a network friendly raw byte format.
//!
//! The module also contains generic parameter enumerations.
//!
//! # Example for primitive type wrapper
//!
//! ```
//! use fsrc_core::util::{ParamsRaw, ToBeBytes, U32Pair};
//!
//! let u32_pair = U32Pair(0x1010, 25);
//! assert_eq!(u32_pair.0, 0x1010);
//! assert_eq!(u32_pair.1, 25);
//! let raw_buf = u32_pair.to_be_bytes();
//! assert_eq!(raw_buf, [0, 0, 0x10, 0x10, 0, 0, 0, 25]);
//!
//! let params_raw: ParamsRaw = u32_pair.into();
//! assert_eq!(params_raw, (0x1010_u32, 25_u32).into());
//! ```
use crate::pool::StoreAddr;
#[cfg(feature = "alloc")]
use alloc::string::String;
#[cfg(feature = "alloc")]
use alloc::string::ToString;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::fmt::Debug;
use core::mem::size_of;
use paste::paste;
pub use spacepackets::ecss::ToBeBytes;
use spacepackets::ecss::{EcssEnumU16, EcssEnumU32, EcssEnumU64, EcssEnumU8};

macro_rules! primitive_newtypes_with_eq {
    ($($ty: ty,)+) => {
        $(
            paste! {
                #[derive(Debug, Copy, Clone, PartialEq, Eq)]
                pub struct [<$ty:upper>](pub $ty);
                #[derive(Debug, Copy, Clone, PartialEq, Eq)]
                pub struct [<$ty:upper Pair>](pub $ty, pub $ty);
                #[derive(Debug, Copy, Clone, PartialEq, Eq)]
                pub struct [<$ty:upper Triplet>](pub $ty, pub $ty, pub $ty);

                impl From<$ty> for [<$ty:upper>] {
                    fn from(v: $ty) -> Self {
                        Self(v)
                    }
                }
                impl From<($ty, $ty)> for [<$ty:upper Pair>] {
                    fn from(v: ($ty, $ty)) -> Self {
                        Self(v.0, v.1)
                    }
                }
                impl From<($ty, $ty, $ty)> for [<$ty:upper Triplet>] {
                    fn from(v: ($ty, $ty, $ty)) -> Self {
                        Self(v.0, v.1, v.2)
                    }
                }
            }
        )+
    }
}

macro_rules! primitive_newtypes {
    ($($ty: ty,)+) => {
        $(
            paste! {
                #[derive(Debug, Copy, Clone, PartialEq)]
                pub struct [<$ty:upper>](pub $ty);
                #[derive(Debug, Copy, Clone, PartialEq)]
                pub struct [<$ty:upper Pair>](pub $ty, pub $ty);
                #[derive(Debug, Copy, Clone, PartialEq)]
                pub struct [<$ty:upper Triplet>](pub $ty, pub $ty, pub $ty);

                impl From<$ty> for [<$ty:upper>] {
                    fn from(v: $ty) -> Self {
                        Self(v)
                    }
                }
                impl From<($ty, $ty)> for [<$ty:upper Pair>] {
                    fn from(v: ($ty, $ty)) -> Self {
                        Self(v.0, v.1)
                    }
                }
                impl From<($ty, $ty, $ty)> for [<$ty:upper Triplet>] {
                    fn from(v: ($ty, $ty, $ty)) -> Self {
                        Self(v.0, v.1, v.2)
                    }
                }
            }
        )+
    }
}

primitive_newtypes_with_eq!(u8, u16, u32, u64, i8, i16, i32, i64,);
primitive_newtypes!(f32, f64,);

macro_rules! scalar_to_be_bytes_impl {
    ($($ty: ty,)+) => {
        $(
            paste! {
                impl ToBeBytes for [<$ty:upper>] {
                    type ByteArray = [u8; size_of::<$ty>()];
                    fn to_be_bytes(&self) -> Self::ByteArray {
                        self.0.to_be_bytes()
                    }
                }
            }
        )+
    }
}

macro_rules! pair_to_be_bytes_impl {
    ($($ty: ty,)+) => {
        $(
            paste! {
                impl ToBeBytes for [<$ty:upper Pair>] {
                    type ByteArray = [u8; size_of::<$ty>() * 2];
                    fn to_be_bytes(&self) -> Self::ByteArray {
                        let mut array = [0; size_of::<$ty>() * 2];
                        array[0..size_of::<$ty>()].copy_from_slice(&self.0.to_be_bytes());
                        array[
                            size_of::<$ty>()..2 * size_of::<$ty>()
                        ].copy_from_slice(&self.1.to_be_bytes());
                        array
                    }
                }
            }
        )+
    }
}

macro_rules! triplet_to_be_bytes_impl {
    ($($ty: ty,)+) => {
        $(
            paste! {
                impl ToBeBytes for [<$ty:upper Triplet>] {
                    type ByteArray = [u8; size_of::<$ty>() * 3];
                    fn to_be_bytes(&self) -> Self::ByteArray {
                        let mut array = [0; size_of::<$ty>() * 3];
                        array[0..size_of::<$ty>()].copy_from_slice(&self.0.to_be_bytes());
                        array[
                            size_of::<$ty>()..2*  size_of::<$ty>()
                        ].copy_from_slice(&self.1.to_be_bytes());
                        array[
                            2 * size_of::<$ty>()..3*  size_of::<$ty>()
                        ].copy_from_slice(&self.2.to_be_bytes());
                        array
                    }
                }
            }
        )+
    }
}

scalar_to_be_bytes_impl!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64,);

impl ToBeBytes for U8Pair {
    type ByteArray = [u8; 2];

    fn to_be_bytes(&self) -> Self::ByteArray {
        let mut array = [0; 2];
        array[0] = self.0;
        array[1] = self.1;
        array
    }
}

impl ToBeBytes for I8Pair {
    type ByteArray = [u8; 2];

    fn to_be_bytes(&self) -> Self::ByteArray {
        let mut array = [0; 2];
        array[0] = self.0 as u8;
        array[1] = self.1 as u8;
        array
    }
}

impl ToBeBytes for U8Triplet {
    type ByteArray = [u8; 3];

    fn to_be_bytes(&self) -> Self::ByteArray {
        let mut array = [0; 3];
        array[0] = self.0;
        array[1] = self.1;
        array[2] = self.2;
        array
    }
}

impl ToBeBytes for I8Triplet {
    type ByteArray = [u8; 3];

    fn to_be_bytes(&self) -> Self::ByteArray {
        let mut array = [0; 3];
        array[0] = self.0 as u8;
        array[1] = self.1 as u8;
        array[2] = self.2 as u8;
        array
    }
}

pair_to_be_bytes_impl!(u16, u32, u64, i16, i32, i64, f32, f64,);
triplet_to_be_bytes_impl!(u16, u32, u64, i16, i32, i64, f32, f64,);

/// Generic enumeration for additonal parameters only consisting of primitive data types.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ParamsRaw {
    U8(U8),
    U8Pair(U8Pair),
    U8Triplet(U8Triplet),
    I8(I8),
    I8Pair(I8Pair),
    I8Triplet(I8Triplet),
    U16(U16),
    U16Pair(U16Pair),
    U16Triplet(U16Triplet),
    I16(I16),
    I16Pair(I16Pair),
    I16Triplet(I16Triplet),
    U32(U32),
    U32Pair(U32Pair),
    U32Triplet(U32Triplet),
    I32(I32),
    I32Pair(I32Pair),
    I32Triplet(I32Triplet),
    F32(F32),
    F32Pair(F32Pair),
    F32Triplet(F32Triplet),
    U64(U64),
    I64(I64),
    F64(F64),
}

macro_rules! params_raw_from_newtype {
    ($($newtype: ident,)+) => {
        $(
            impl From<$newtype> for ParamsRaw {
                fn from(v: $newtype) -> Self {
                    Self::$newtype(v)
                }
            }
        )+
    }
}

params_raw_from_newtype!(
    U8, U8Pair, U8Triplet, U16, U16Pair, U16Triplet, U32, U32Pair, U32Triplet, I8, I8Pair,
    I8Triplet, I16, I16Pair, I16Triplet, I32, I32Pair, I32Triplet, F32, F32Pair, F32Triplet, U64,
    I64, F64,
);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EcssEnumParams {
    U8(EcssEnumU8),
    U16(EcssEnumU16),
    U32(EcssEnumU32),
    U64(EcssEnumU64),
}

/// Generic enumeration for parameters which do not rely on heap allocations.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ParamsHeapless {
    Raw(ParamsRaw),
    EcssEnum(EcssEnumParams),
    Store(StoreAddr),
}

impl From<StoreAddr> for ParamsHeapless {
    fn from(x: StoreAddr) -> Self {
        Self::Store(x)
    }
}

macro_rules! from_conversions_for_raw {
    ($(($raw_ty: ty, $TargetPath: path),)+) => {
        $(
            impl From<$raw_ty> for ParamsRaw {
                fn from(val: $raw_ty) -> Self {
                    $TargetPath(val.into())
                }
            }

            impl From<$raw_ty> for ParamsHeapless {
                fn from(val: $raw_ty) -> Self {
                    ParamsHeapless::Raw(val.into())
                }
            }
        )+
    };
}

from_conversions_for_raw!(
    (u8, Self::U8),
    ((u8, u8), Self::U8Pair),
    ((u8, u8, u8), Self::U8Triplet),
    (i8, Self::I8),
    ((i8, i8), Self::I8Pair),
    ((i8, i8, i8), Self::I8Triplet),
    (u16, Self::U16),
    ((u16, u16), Self::U16Pair),
    ((u16, u16, u16), Self::U16Triplet),
    (i16, Self::I16),
    ((i16, i16), Self::I16Pair),
    ((i16, i16, i16), Self::I16Triplet),
    (u32, Self::U32),
    ((u32, u32), Self::U32Pair),
    ((u32, u32, u32), Self::U32Triplet),
    (i32, Self::I32),
    ((i32, i32), Self::I32Pair),
    ((i32, i32, i32), Self::I32Triplet),
    (f32, Self::F32),
    ((f32, f32), Self::F32Pair),
    ((f32, f32, f32), Self::F32Triplet),
    (u64, Self::U64),
    (f64, Self::F64),
);

/// Generic enumeration for additional parameters, including parameters which rely on heap
/// allocations.
#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
#[derive(Debug, Clone)]
pub enum Params {
    Heapless(ParamsHeapless),
    Vec(Vec<u8>),
    String(String),
}

impl From<ParamsHeapless> for Params {
    fn from(x: ParamsHeapless) -> Self {
        Self::Heapless(x)
    }
}

impl From<Vec<u8>> for Params {
    fn from(val: Vec<u8>) -> Self {
        Self::Vec(val)
    }
}

/// Converts a byte slice into the [Params::Vec] variant
impl From<&[u8]> for Params {
    fn from(val: &[u8]) -> Self {
        Self::Vec(val.to_vec())
    }
}

impl From<String> for Params {
    fn from(val: String) -> Self {
        Self::String(val)
    }
}

/// Converts a string slice into the [Params::String] variant
impl From<&str> for Params {
    fn from(val: &str) -> Self {
        Self::String(val.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_u32_pair() {
        let u32_pair = U32Pair(4, 8);
        assert_eq!(u32_pair.0, 4);
        assert_eq!(u32_pair.1, 8);
        let raw = u32_pair.to_be_bytes();
        let mut u32_conv_back = u32::from_be_bytes(raw[0..4].try_into().unwrap());
        assert_eq!(u32_conv_back, 4);
        u32_conv_back = u32::from_be_bytes(raw[4..8].try_into().unwrap());
        assert_eq!(u32_conv_back, 8);
    }

    #[test]
    fn basic_signed_test_pair() {
        let i8_pair = I8Pair(-3, -16);
        assert_eq!(i8_pair.0, -3);
        assert_eq!(i8_pair.1, -16);
        let raw = i8_pair.to_be_bytes();
        let mut i8_conv_back = i8::from_be_bytes(raw[0..1].try_into().unwrap());
        assert_eq!(i8_conv_back, -3);
        i8_conv_back = i8::from_be_bytes(raw[1..2].try_into().unwrap());
        assert_eq!(i8_conv_back, -16);
    }

    #[test]
    fn basic_signed_test_triplet() {
        let i8_triplet = I8Triplet(-3, -16, -126);
        assert_eq!(i8_triplet.0, -3);
        assert_eq!(i8_triplet.1, -16);
        assert_eq!(i8_triplet.2, -126);
        let raw = i8_triplet.to_be_bytes();
        let mut i8_conv_back = i8::from_be_bytes(raw[0..1].try_into().unwrap());
        assert_eq!(i8_conv_back, -3);
        i8_conv_back = i8::from_be_bytes(raw[1..2].try_into().unwrap());
        assert_eq!(i8_conv_back, -16);
        i8_conv_back = i8::from_be_bytes(raw[2..3].try_into().unwrap());
        assert_eq!(i8_conv_back, -126);
    }

    #[test]
    fn conversion_test_string() {
        let param: Params = "Test String".into();
        if let Params::String(str) = param {
            assert_eq!(str, String::from("Test String"));
        } else {
            panic!("Params type is not String")
        }
    }

    #[test]
    fn conversion_from_slice() {
        let test_slice: [u8; 5] = [0; 5];
        let vec_param: Params = test_slice.as_slice().into();
        if let Params::Vec(vec) = vec_param {
            assert_eq!(vec, test_slice.to_vec());
        } else {
            panic!("Params type is not a vector")
        }
    }
}
