pub enum FsrcGroupIds {
    Tmtc = 0,
}

pub struct FsrcErrorRaw {
    pub group_id: u8,
    pub unique_id: u8,
    pub group_name: &'static str,
    pub info: &'static str,
}

pub trait FsrcErrorHandler {
    fn error(&mut self, e: FsrcErrorRaw);
    fn error_with_one_param(&mut self, e: FsrcErrorRaw, _p1: u32) {
        self.error(e);
    }
    fn error_with_two_params(&mut self, e: FsrcErrorRaw, _p1: u32, _p2: u32) {
        self.error(e);
    }
}

impl FsrcErrorRaw {
    pub const fn new(
        group_id: u8,
        unique_id: u8,
        group_name: &'static str,
        info: &'static str,
    ) -> Self {
        FsrcErrorRaw {
            group_id,
            unique_id,
            group_name,
            info,
        }
    }
}

pub struct SimpleStdErrorHandler {}

#[cfg(feature = "use_std")]
impl FsrcErrorHandler for SimpleStdErrorHandler {
    fn error(&mut self, e: FsrcErrorRaw) {
        println!(
            "Received error from group {} with ID ({},{}): {}",
            e.group_name, e.group_id, e.unique_id, e.info
        );
    }
}
