#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ResultU16 {
    group_id: u8,
    unique_id: u8,
}

impl ResultU16 {
    pub const fn const_new(group_id: u8, unique_id: u8) -> Self {
        Self {
            group_id,
            unique_id,
        }
    }
    pub fn raw(&self) -> u16 {
        ((self.group_id as u16) << 8) | self.unique_id as u16
    }
    pub fn group_id(&self) -> u8 {
        self.group_id
    }
    pub fn unique_id(&self) -> u8 {
        self.unique_id
    }
}

#[derive(Debug)]
pub struct ResultU16Ext {
    pub name: &'static str,
    pub result: ResultU16,
    pub info: &'static str,
}

impl ResultU16Ext {
    pub const fn const_new(name: &'static str, result: ResultU16, info: &'static str) -> Self {
        Self { name, result, info }
    }
}
