pub enum FsrcGroupIds {
    Tmtc = 0,
}

pub struct FsrcErrorRaw {
    pub group_id: u8,
    pub unique_id: u8,
    pub group_name: &'static str,
    pub error_info: &'static str,
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

pub struct SimpleStdErrorHandler {}

#[cfg(feature = "use_std")]
impl FsrcErrorHandler for SimpleStdErrorHandler {
    fn error(&mut self, e: FsrcErrorRaw) {
        println!(
            "Received error from group {} with ID ({},{}): {}",
            e.group_name, e.group_id, e.unique_id, e.error_info
        );
    }
}
