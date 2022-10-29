use crate::pool::StoreAddr;
#[cfg(feature = "alloc")]
use alloc::string::String;
#[cfg(feature = "alloc")]
use alloc::string::ToString;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

#[derive(Debug, Copy, Clone)]
pub enum AuxDataRaw {
    U8(u8),
    U8Pair((u8, u8)),
    U8Triplet((u8, u8, u8)),
    I8(i8),
    I8Pair((i8, i8)),
    I8Triplet((i8, i8, i8)),
    U16(u16),
    U16Pair((u16, u16)),
    U16Triplet((u16, u16, u16)),
    I16(i16),
    I16Pair((i16, i16)),
    I16Triplet((i16, i16, i16)),
    U32(u32),
    U32Pair((u32, u32)),
    U32Triplet((u32, u32, u32)),
    I32(i32),
    I32Pair((i32, i32)),
    I32Triplet((i32, i32, i32)),
    F32(f32),
    F32Pair((f32, f32)),
    F32Triplet((f32, f32, f32)),
    U64(u64),
    F64(f64),
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
                    $TargetPath(val)
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
