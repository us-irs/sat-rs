use spacepackets::{
    cfdp::{
        tlv::{GenericTlv, Tlv, TlvType},
        SegmentationControl, TransmissionMode,
    },
    util::UnsignedByteField,
};

#[cfg(feature = "alloc")]
pub use alloc_mod::*;

#[derive(Debug, PartialEq, Eq)]
pub struct FilePathTooLarge(pub usize);

/// This trait is an abstraction for different Put Request structures which can be used
/// by Put Request consumers.
pub trait ReadablePutRequest {
    fn destination_id(&self) -> UnsignedByteField;
    fn source_file(&self) -> Option<&str>;
    fn dest_file(&self) -> Option<&str>;
    fn trans_mode(&self) -> Option<TransmissionMode>;
    fn closure_requested(&self) -> Option<bool>;
    fn seg_ctrl(&self) -> Option<SegmentationControl>;
    fn has_msgs_to_user(&self) -> bool;
    fn msgs_to_user(&self, f: impl FnMut(&Tlv));
    fn has_fault_handler_overrides(&self) -> bool;
    fn fault_handler_overrides(&self, f: impl FnMut(&Tlv));
    fn flow_label(&self) -> Option<Tlv>;
    fn has_fs_requests(&self) -> bool;
    fn fs_requests(&self, f: impl FnMut(&Tlv));
}

#[derive(Debug, PartialEq, Eq)]
pub struct PutRequest<'src_file, 'dest_file, 'msgs_to_user, 'fh_ovrds, 'flow_label, 'fs_requests> {
    pub destination_id: UnsignedByteField,
    source_file: Option<&'src_file str>,
    dest_file: Option<&'dest_file str>,
    pub trans_mode: Option<TransmissionMode>,
    pub closure_requested: Option<bool>,
    pub seg_ctrl: Option<SegmentationControl>,
    pub msgs_to_user: Option<&'msgs_to_user [Tlv<'msgs_to_user>]>,
    pub fault_handler_overrides: Option<&'fh_ovrds [Tlv<'fh_ovrds>]>,
    pub flow_label: Option<Tlv<'flow_label>>,
    pub fs_requests: Option<&'fs_requests [Tlv<'fs_requests>]>,
}

impl<'src_file, 'dest_file, 'msgs_to_user, 'fh_ovrds, 'flow_label, 'fs_requests>
    PutRequest<'src_file, 'dest_file, 'msgs_to_user, 'fh_ovrds, 'flow_label, 'fs_requests>
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        destination_id: UnsignedByteField,
        source_file: Option<&'src_file str>,
        dest_file: Option<&'dest_file str>,
        trans_mode: Option<TransmissionMode>,
        closure_requested: Option<bool>,
        seg_ctrl: Option<SegmentationControl>,
        msgs_to_user: Option<&'msgs_to_user [Tlv<'msgs_to_user>]>,
        fault_handler_overrides: Option<&'fh_ovrds [Tlv<'fh_ovrds>]>,
        flow_label: Option<Tlv<'flow_label>>,
        fs_requests: Option<&'fs_requests [Tlv<'fs_requests>]>,
    ) -> Result<Self, FilePathTooLarge> {
        generic_path_checks(source_file, dest_file)?;
        Ok(Self {
            destination_id,
            source_file,
            dest_file,
            trans_mode,
            closure_requested,
            seg_ctrl,
            msgs_to_user,
            fault_handler_overrides,
            flow_label,
            fs_requests,
        })
    }
}

impl ReadablePutRequest for PutRequest<'_, '_, '_, '_, '_, '_> {
    fn destination_id(&self) -> UnsignedByteField {
        self.destination_id
    }

    fn source_file(&self) -> Option<&str> {
        self.source_file
    }

    fn dest_file(&self) -> Option<&str> {
        self.dest_file
    }

    fn trans_mode(&self) -> Option<TransmissionMode> {
        self.trans_mode
    }

    fn closure_requested(&self) -> Option<bool> {
        self.closure_requested
    }

    fn seg_ctrl(&self) -> Option<SegmentationControl> {
        self.seg_ctrl
    }

    fn has_msgs_to_user(&self) -> bool {
        self.msgs_to_user.is_some() && self.msgs_to_user.unwrap().is_empty()
    }

    fn msgs_to_user(&self, mut f: impl FnMut(&Tlv)) {
        if let Some(msgs_to_user) = self.msgs_to_user {
            for msg_to_user in msgs_to_user {
                f(msg_to_user)
            }
        }
    }

    fn has_fault_handler_overrides(&self) -> bool {
        self.fault_handler_overrides.is_some() && self.fault_handler_overrides.unwrap().is_empty()
    }

    fn fault_handler_overrides(&self, mut f: impl FnMut(&Tlv)) {
        if let Some(fh_overrides) = self.fault_handler_overrides {
            for fh_override in fh_overrides {
                f(fh_override)
            }
        }
    }

    fn flow_label(&self) -> Option<Tlv> {
        self.flow_label
    }

    fn has_fs_requests(&self) -> bool {
        self.fs_requests.is_some() && self.fs_requests.unwrap().is_empty()
    }

    fn fs_requests(&self, mut f: impl FnMut(&Tlv)) {
        if let Some(fs_requests) = self.fs_requests {
            for fs_request in fs_requests {
                f(fs_request)
            }
        }
    }
}

pub fn generic_path_checks(
    source_file: Option<&str>,
    dest_file: Option<&str>,
) -> Result<(), FilePathTooLarge> {
    if let Some(src_file) = source_file {
        if src_file.len() > u8::MAX as usize {
            return Err(FilePathTooLarge(src_file.len()));
        }
    }
    if let Some(dest_file) = dest_file {
        if dest_file.len() > u8::MAX as usize {
            return Err(FilePathTooLarge(dest_file.len()));
        }
    }
    Ok(())
}

impl<'src_file, 'dest_file> PutRequest<'src_file, 'dest_file, 'static, 'static, 'static, 'static> {
    pub fn new_regular_request(
        dest_id: UnsignedByteField,
        source_file: &'src_file str,
        dest_file: &'dest_file str,
        trans_mode: Option<TransmissionMode>,
        closure_requested: Option<bool>,
    ) -> Result<Self, FilePathTooLarge> {
        generic_path_checks(Some(source_file), Some(dest_file))?;
        Ok(Self {
            destination_id: dest_id,
            source_file: Some(source_file),
            dest_file: Some(dest_file),
            trans_mode,
            closure_requested,
            seg_ctrl: None,
            msgs_to_user: None,
            fault_handler_overrides: None,
            flow_label: None,
            fs_requests: None,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct TlvWithInvalidType(pub(crate) ());

impl<'msgs_to_user> PutRequest<'static, 'static, 'msgs_to_user, 'static, 'static, 'static> {
    pub fn new_msgs_to_user_only(
        dest_id: UnsignedByteField,
        msgs_to_user: &'msgs_to_user [Tlv<'msgs_to_user>],
    ) -> Result<Self, TlvWithInvalidType> {
        Ok(Self {
            destination_id: dest_id,
            source_file: None,
            dest_file: None,
            trans_mode: None,
            closure_requested: None,
            seg_ctrl: None,
            msgs_to_user: Some(msgs_to_user),
            fault_handler_overrides: None,
            flow_label: None,
            fs_requests: None,
        })
    }

    /// Uses [generic_tlv_list_type_check] to check the TLV type validity of all TLV fields.
    pub fn check_tlv_type_validities(&self) -> bool {
        generic_tlv_list_type_check(self.msgs_to_user, TlvType::MsgToUser);
        if let Some(flow_label) = &self.flow_label {
            if flow_label.tlv_type().is_none() {
                return false;
            }
            if flow_label.tlv_type().unwrap() != TlvType::FlowLabel {
                return false;
            }
        }
        generic_tlv_list_type_check(self.fault_handler_overrides, TlvType::FaultHandler);
        generic_tlv_list_type_check(self.fs_requests, TlvType::FilestoreRequest);
        true
    }
}

pub fn generic_tlv_list_type_check<TlvProvider: GenericTlv>(
    opt_tlvs: Option<&[TlvProvider]>,
    tlv_type: TlvType,
) -> bool {
    if let Some(tlvs) = opt_tlvs {
        for tlv in tlvs {
            if tlv.tlv_type().is_none() {
                return false;
            }
            if tlv.tlv_type().unwrap() != tlv_type {
                return false;
            }
        }
    }
    true
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {
    use core::str::Utf8Error;

    use super::*;
    use alloc::string::ToString;
    use spacepackets::{
        cfdp::tlv::{msg_to_user::MsgToUserTlv, ReadableTlv, TlvOwned, WritableTlv},
        ByteConversionError,
    };

    /// Owned variant of [PutRequest] with no lifetimes which is also [Clone]able.
    #[derive(Debug, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct PutRequestOwned {
        pub destination_id: UnsignedByteField,
        source_file: Option<alloc::string::String>,
        dest_file: Option<alloc::string::String>,
        pub trans_mode: Option<TransmissionMode>,
        pub closure_requested: Option<bool>,
        pub seg_ctrl: Option<SegmentationControl>,
        pub msgs_to_user: Option<alloc::vec::Vec<TlvOwned>>,
        pub fault_handler_overrides: Option<alloc::vec::Vec<TlvOwned>>,
        pub flow_label: Option<TlvOwned>,
        pub fs_requests: Option<alloc::vec::Vec<TlvOwned>>,
    }

    impl PutRequestOwned {
        pub fn new_regular_request(
            dest_id: UnsignedByteField,
            source_file: &str,
            dest_file: &str,
            trans_mode: Option<TransmissionMode>,
            closure_requested: Option<bool>,
        ) -> Result<Self, FilePathTooLarge> {
            if source_file.len() > u8::MAX as usize {
                return Err(FilePathTooLarge(source_file.len()));
            }
            if dest_file.len() > u8::MAX as usize {
                return Err(FilePathTooLarge(dest_file.len()));
            }
            Ok(Self {
                destination_id: dest_id,
                source_file: Some(source_file.to_string()),
                dest_file: Some(dest_file.to_string()),
                trans_mode,
                closure_requested,
                seg_ctrl: None,
                msgs_to_user: None,
                fault_handler_overrides: None,
                flow_label: None,
                fs_requests: None,
            })
        }

        pub fn new_msgs_to_user_only(
            dest_id: UnsignedByteField,
            msgs_to_user: &[MsgToUserTlv<'_>],
        ) -> Result<Self, TlvWithInvalidType> {
            Ok(Self {
                destination_id: dest_id,
                source_file: None,
                dest_file: None,
                trans_mode: None,
                closure_requested: None,
                seg_ctrl: None,
                msgs_to_user: Some(msgs_to_user.iter().map(|msg| msg.tlv.to_owned()).collect()),
                fault_handler_overrides: None,
                flow_label: None,
                fs_requests: None,
            })
        }

        /// Uses [generic_tlv_list_type_check] to check the TLV type validity of all TLV fields.
        pub fn check_tlv_type_validities(&self) -> bool {
            generic_tlv_list_type_check(self.msgs_to_user.as_deref(), TlvType::MsgToUser);
            if let Some(flow_label) = &self.flow_label {
                if flow_label.tlv_type().is_none() {
                    return false;
                }
                if flow_label.tlv_type().unwrap() != TlvType::FlowLabel {
                    return false;
                }
            }
            generic_tlv_list_type_check(
                self.fault_handler_overrides.as_deref(),
                TlvType::FaultHandler,
            );
            generic_tlv_list_type_check(self.fs_requests.as_deref(), TlvType::FilestoreRequest);
            true
        }
    }

    impl From<PutRequest<'_, '_, '_, '_, '_, '_>> for PutRequestOwned {
        fn from(req: PutRequest<'_, '_, '_, '_, '_, '_>) -> Self {
            Self {
                destination_id: req.destination_id,
                source_file: req.source_file.map(|s| s.into()),
                dest_file: req.dest_file.map(|s| s.into()),
                trans_mode: req.trans_mode,
                closure_requested: req.closure_requested,
                seg_ctrl: req.seg_ctrl,
                msgs_to_user: req
                    .msgs_to_user
                    .map(|msgs_to_user| msgs_to_user.iter().map(|msg| msg.to_owned()).collect()),
                fault_handler_overrides: req
                    .msgs_to_user
                    .map(|fh_overides| fh_overides.iter().map(|msg| msg.to_owned()).collect()),
                flow_label: req
                    .flow_label
                    .map(|flow_label_tlv| flow_label_tlv.to_owned()),
                fs_requests: req
                    .fs_requests
                    .map(|fs_requests| fs_requests.iter().map(|msg| msg.to_owned()).collect()),
            }
        }
    }

    impl ReadablePutRequest for PutRequestOwned {
        fn destination_id(&self) -> UnsignedByteField {
            self.destination_id
        }

        fn source_file(&self) -> Option<&str> {
            self.source_file.as_deref()
        }

        fn dest_file(&self) -> Option<&str> {
            self.dest_file.as_deref()
        }

        fn trans_mode(&self) -> Option<TransmissionMode> {
            self.trans_mode
        }

        fn closure_requested(&self) -> Option<bool> {
            self.closure_requested
        }

        fn seg_ctrl(&self) -> Option<SegmentationControl> {
            self.seg_ctrl
        }

        fn msgs_to_user(&self, mut f: impl FnMut(&Tlv)) {
            if let Some(msgs_to_user) = &self.msgs_to_user {
                for msg_to_user in msgs_to_user {
                    f(&msg_to_user.as_tlv())
                }
            }
        }

        fn fault_handler_overrides(&self, mut f: impl FnMut(&Tlv)) {
            if let Some(fh_overrides) = &self.fault_handler_overrides {
                for fh_override in fh_overrides {
                    f(&fh_override.as_tlv())
                }
            }
        }

        fn flow_label(&self) -> Option<Tlv> {
            self.flow_label.as_ref().map(|tlv| tlv.as_tlv())
        }

        fn fs_requests(&self, mut f: impl FnMut(&Tlv)) {
            if let Some(fs_requests) = &self.fs_requests {
                for fs_request in fs_requests {
                    f(&fs_request.as_tlv())
                }
            }
        }

        fn has_msgs_to_user(&self) -> bool {
            self.msgs_to_user.is_some() && !self.msgs_to_user.as_ref().unwrap().is_empty()
        }

        fn has_fault_handler_overrides(&self) -> bool {
            self.fault_handler_overrides.is_some()
                && !self.fault_handler_overrides.as_ref().unwrap().is_empty()
        }

        fn has_fs_requests(&self) -> bool {
            self.fs_requests.is_some() && !self.fs_requests.as_ref().unwrap().is_empty()
        }
    }

    pub struct StaticPutRequestFields {
        pub destination_id: UnsignedByteField,
        /// Static buffer to store source file path.
        pub source_file_buf: [u8; u8::MAX as usize],
        /// Current source path length.
        pub source_file_len: usize,
        /// Static buffer to store dest file path.
        pub dest_file_buf: [u8; u8::MAX as usize],
        /// Current destination path length.
        pub dest_file_len: usize,
        pub trans_mode: Option<TransmissionMode>,
        pub closure_requested: Option<bool>,
        pub seg_ctrl: Option<SegmentationControl>,
    }

    impl Default for StaticPutRequestFields {
        fn default() -> Self {
            Self {
                destination_id: UnsignedByteField::new(0, 0),
                source_file_buf: [0; u8::MAX as usize],
                source_file_len: Default::default(),
                dest_file_buf: [0; u8::MAX as usize],
                dest_file_len: Default::default(),
                trans_mode: Default::default(),
                closure_requested: Default::default(),
                seg_ctrl: Default::default(),
            }
        }
    }

    impl StaticPutRequestFields {
        pub fn clear(&mut self) {
            self.destination_id = UnsignedByteField::new(0, 0);
            self.source_file_len = 0;
            self.dest_file_len = 0;
            self.trans_mode = None;
            self.closure_requested = None;
            self.seg_ctrl = None;
        }
    }

    /// This is a put request cache structure which can be used to cache [ReadablePutRequest]s
    /// without requiring run-time allocation. The user must specify the static buffer sizes used
    /// to store TLVs or list of TLVs.
    pub struct StaticPutRequestCacher {
        pub static_fields: StaticPutRequestFields,
        /// Static buffer to store file store requests.
        pub fs_requests: alloc::vec::Vec<u8>,
        /// Current total length of stored filestore requests.
        pub fs_requests_len: usize,
    }

    impl StaticPutRequestCacher {
        pub fn new(max_fs_requests_storage: usize) -> Self {
            Self {
                static_fields: StaticPutRequestFields::default(),
                fs_requests: alloc::vec![0; max_fs_requests_storage],
                fs_requests_len: 0,
            }
        }

        pub fn set(
            &mut self,
            put_request: &impl ReadablePutRequest,
        ) -> Result<(), ByteConversionError> {
            self.static_fields.destination_id = put_request.destination_id();
            if let Some(source_file) = put_request.source_file() {
                if source_file.len() > u8::MAX as usize {
                    return Err(ByteConversionError::ToSliceTooSmall {
                        found: self.static_fields.source_file_buf.len(),
                        expected: source_file.len(),
                    });
                }
                self.static_fields.source_file_buf[..source_file.len()]
                    .copy_from_slice(source_file.as_bytes());
                self.static_fields.source_file_len = source_file.len();
            }
            if let Some(dest_file) = put_request.dest_file() {
                if dest_file.len() > u8::MAX as usize {
                    return Err(ByteConversionError::ToSliceTooSmall {
                        found: self.static_fields.source_file_buf.len(),
                        expected: dest_file.len(),
                    });
                }
                self.static_fields.dest_file_buf[..dest_file.len()]
                    .copy_from_slice(dest_file.as_bytes());
                self.static_fields.dest_file_len = dest_file.len();
            }
            self.static_fields.trans_mode = put_request.trans_mode();
            self.static_fields.closure_requested = put_request.closure_requested();
            self.static_fields.seg_ctrl = put_request.seg_ctrl();
            let mut current_idx = 0;
            let mut error_if_too_large = None;
            let mut store_fs_requests = |tlv: &Tlv| {
                if current_idx + tlv.len_full() > self.fs_requests.len() {
                    error_if_too_large = Some(ByteConversionError::ToSliceTooSmall {
                        found: self.fs_requests.len(),
                        expected: current_idx + tlv.len_full(),
                    });
                    return;
                }
                // We checked the buffer lengths, so this should never fail.
                tlv.write_to_bytes(
                    &mut self.fs_requests[current_idx..current_idx + tlv.len_full()],
                )
                .unwrap();
                current_idx += tlv.len_full();
            };
            put_request.fs_requests(&mut store_fs_requests);
            if let Some(err) = error_if_too_large {
                return Err(err);
            }
            self.fs_requests_len = current_idx;
            Ok(())
        }

        pub fn source_file(&self) -> Result<&str, Utf8Error> {
            core::str::from_utf8(
                &self.static_fields.source_file_buf[0..self.static_fields.source_file_len],
            )
        }

        pub fn dest_file(&self) -> Result<&str, Utf8Error> {
            core::str::from_utf8(
                &self.static_fields.dest_file_buf[0..self.static_fields.dest_file_len],
            )
        }

        /// This clears the cacher structure. This is a cheap operation because it only
        /// sets [Option]al values to [None] and the length of stores TLVs to 0.
        ///
        /// Please note that this method will not set the values in the buffer to 0.
        pub fn clear(&mut self) {
            self.static_fields.clear();
            self.fs_requests_len = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use spacepackets::util::UbfU16;

    use super::*;

    pub const DEST_ID: UbfU16 = UbfU16::new(5);

    #[test]
    fn test_put_request_basic() {
        let src_file = "/tmp/hello.txt";
        let dest_file = "/tmp/hello2.txt";
        let put_request =
            PutRequest::new_regular_request(DEST_ID.into(), src_file, dest_file, None, None)
                .unwrap();
        assert_eq!(put_request.source_file(), Some(src_file));
        assert_eq!(put_request.dest_file(), Some(dest_file));
        assert_eq!(put_request.destination_id(), DEST_ID.into());
        assert_eq!(put_request.seg_ctrl(), None);
        assert_eq!(put_request.closure_requested(), None);
        assert_eq!(put_request.trans_mode(), None);
        assert!(!put_request.has_fs_requests());
        let dummy = |_tlv: &Tlv| {
            panic!("should not be called");
        };
        put_request.fs_requests(&dummy);
        assert!(!put_request.has_msgs_to_user());
        put_request.msgs_to_user(&dummy);
        assert!(!put_request.has_fault_handler_overrides());
        put_request.fault_handler_overrides(&dummy);
        assert!(put_request.flow_label().is_none());
    }

    #[test]
    fn test_put_request_owned_basic() {
        let src_file = "/tmp/hello.txt";
        let dest_file = "/tmp/hello2.txt";
        let put_request =
            PutRequestOwned::new_regular_request(DEST_ID.into(), src_file, dest_file, None, None)
                .unwrap();
        assert_eq!(put_request.source_file(), Some(src_file));
        assert_eq!(put_request.dest_file(), Some(dest_file));
        assert_eq!(put_request.destination_id(), DEST_ID.into());
        assert_eq!(put_request.seg_ctrl(), None);
        assert_eq!(put_request.closure_requested(), None);
        assert_eq!(put_request.trans_mode(), None);
        assert!(!put_request.has_fs_requests());
        let dummy = |_tlv: &Tlv| {
            panic!("should not be called");
        };
        put_request.fs_requests(&dummy);
        assert!(!put_request.has_msgs_to_user());
        put_request.msgs_to_user(&dummy);
        assert!(!put_request.has_fault_handler_overrides());
        put_request.fault_handler_overrides(&dummy);
        assert!(put_request.flow_label().is_none());
        let put_request_cloned = put_request.clone();
        assert_eq!(put_request, put_request_cloned);
    }

    #[test]
    fn test_put_request_cacher_basic() {
        let cacher_cfg = PutRequestCacheConfig {
            max_msgs_to_user_storage: 512,
            max_fault_handler_overrides_storage: 128,
            max_flow_label_storage: 128,
            max_fs_requests_storage: 512,
        };
        let put_request_cached = StaticPutRequestCacher::new(cacher_cfg);
        assert_eq!(put_request_cached.static_fields.source_file_len, 0);
        assert_eq!(put_request_cached.static_fields.dest_file_len, 0);
        assert_eq!(put_request_cached.fs_requests_len, 0);
        assert_eq!(put_request_cached.fs_requests.len(), 512);
    }

    #[test]
    fn test_put_request_cacher_set() {
        let cacher_cfg = PutRequestCacheConfig {
            max_msgs_to_user_storage: 512,
            max_fault_handler_overrides_storage: 128,
            max_flow_label_storage: 128,
            max_fs_requests_storage: 512,
        };
        let mut put_request_cached = StaticPutRequestCacher::new(cacher_cfg);
        let src_file = "/tmp/hello.txt";
        let dest_file = "/tmp/hello2.txt";
        let put_request =
            PutRequest::new_regular_request(DEST_ID.into(), src_file, dest_file, None, None)
                .unwrap();
        put_request_cached.set(&put_request).unwrap();
        assert_eq!(
            put_request_cached.static_fields.source_file_len,
            src_file.len()
        );
        assert_eq!(
            put_request_cached.static_fields.dest_file_len,
            dest_file.len()
        );
        assert_eq!(put_request_cached.source_file().unwrap(), src_file);
        assert_eq!(put_request_cached.dest_file().unwrap(), dest_file);
        assert_eq!(put_request_cached.fs_requests_len, 0);
    }

    #[test]
    fn test_put_request_cacher_set_and_clear() {
        let cacher_cfg = PutRequestCacheConfig {
            max_msgs_to_user_storage: 512,
            max_fault_handler_overrides_storage: 128,
            max_flow_label_storage: 128,
            max_fs_requests_storage: 512,
        };
        let mut put_request_cached = StaticPutRequestCacher::new(cacher_cfg);
        let src_file = "/tmp/hello.txt";
        let dest_file = "/tmp/hello2.txt";
        let put_request =
            PutRequest::new_regular_request(DEST_ID.into(), src_file, dest_file, None, None)
                .unwrap();
        put_request_cached.set(&put_request).unwrap();
        put_request_cached.clear();
        assert_eq!(put_request_cached.static_fields.source_file_len, 0);
        assert_eq!(put_request_cached.static_fields.dest_file_len, 0);
        assert_eq!(put_request_cached.fs_requests_len, 0);
    }
}
