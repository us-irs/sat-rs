//! Utility types and enums
//!
//! This module contains helper types.
use crate::pool::StoreAddr;
#[cfg(feature = "alloc")]
use alloc::string::String;
#[cfg(feature = "alloc")]
use alloc::string::ToString;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;
use core::mem::size_of;
use paste::paste;
use spacepackets::ecss::ToBeBytes;

macro_rules! primitive_newtypes {
    ($($ty: ty,)+) => {
        $(
            paste! {
                #[derive(Debug, Copy, Clone)]
                pub struct [<$ty:upper>](pub $ty);
                #[derive(Debug, Copy, Clone)]
                pub struct [<$ty:upper Pair>](pub $ty, pub $ty);
                #[derive(Debug, Copy, Clone)]
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

primitive_newtypes!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64,);

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

#[derive(Debug, Copy, Clone)]
pub enum AuxDataRaw {
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
    F64(F64),
}

#[derive(Debug, Copy, Clone)]
pub enum AuxDataHeapless {
    Raw(AuxDataRaw),
    Store(StoreAddr),
}

impl From<StoreAddr> for AuxDataHeapless {
    fn from(x: StoreAddr) -> Self {
        Self::Store(x)
    }
}

macro_rules! from_conversions_for_raw {
    ($(($raw_ty: ty, $TargetPath: path),)+) => {
        $(
            impl From<$raw_ty> for AuxDataRaw {
                fn from(val: $raw_ty) -> Self {
                    $TargetPath(val.into())
                }
            }

            impl From<$raw_ty> for AuxDataHeapless {
                fn from(val: $raw_ty) -> Self {
                    AuxDataHeapless::Raw(val.into())
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

#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
#[derive(Debug, Clone)]
pub enum AuxData {
    Heapless(AuxDataHeapless),
    Vec(Vec<u8>),
    String(String),
}

impl From<AuxDataHeapless> for AuxData {
    fn from(x: AuxDataHeapless) -> Self {
        Self::Heapless(x)
    }
}

impl From<Vec<u8>> for AuxData {
    fn from(val: Vec<u8>) -> Self {
        Self::Vec(val)
    }
}

impl From<&[u8]> for AuxData {
    fn from(val: &[u8]) -> Self {
        Self::Vec(val.to_vec())
    }
}

impl From<String> for AuxData {
    fn from(val: String) -> Self {
        Self::String(val)
    }
}

impl From<&str> for AuxData {
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
}
