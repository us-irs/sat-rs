use spacepackets::{
    cfdp::{
        tlv::{GenericTlv, Tlv, TlvType},
        SegmentationControl, TransmissionMode,
    },
    util::UnsignedByteField,
};

trait ReadablePutRequest {
    fn destination_id(&self) -> UnsignedByteField;
    fn source_file(&self) -> Option<&str>;
    fn dest_file(&self) -> Option<&str>;
    fn trans_mode(&self) -> Option<TransmissionMode>;
    fn closure_requested(&self) -> Option<bool>;
    fn seg_ctrl(&self) -> Option<SegmentationControl>;
    fn msgs_to_user(&self, f: impl FnMut(&Tlv));
    fn fault_handler_overrides(&self, f: impl FnMut(&Tlv));
    fn flow_label(&self) -> Option<Tlv>;
    fn fs_requests(&self, f: impl FnMut(&Tlv));
}

#[derive(Debug, PartialEq, Eq)]
pub struct PutRequest<'src_file, 'dest_file, 'msgs_to_user, 'fh_ovrds, 'flow_label, 'fs_requests> {
    pub destination_id: UnsignedByteField,
    pub source_file: Option<&'src_file str>,
    pub dest_file: Option<&'dest_file str>,
    pub trans_mode: Option<TransmissionMode>,
    pub closure_requested: Option<bool>,
    pub seg_ctrl: Option<SegmentationControl>,
    pub msgs_to_user: Option<&'msgs_to_user [Tlv<'msgs_to_user>]>,
    pub fault_handler_overrides: Option<&'fh_ovrds [Tlv<'fh_ovrds>]>,
    pub flow_label: Option<Tlv<'flow_label>>,
    pub fs_requests: Option<&'fs_requests [Tlv<'fs_requests>]>,
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

    fn msgs_to_user(&self, mut f: impl FnMut(&Tlv)) {
        if let Some(msgs_to_user) = self.msgs_to_user {
            for msg_to_user in msgs_to_user {
                f(msg_to_user)
            }
        }
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

    fn fs_requests(&self, mut f: impl FnMut(&Tlv)) {
        if let Some(fs_requests) = self.fs_requests {
            for fs_request in fs_requests {
                f(fs_request)
            }
        }
    }
}

impl<'src_file, 'dest_file> PutRequest<'src_file, 'dest_file, 'static, 'static, 'static, 'static> {
    pub fn new_regular_request(
        dest_id: UnsignedByteField,
        source_file: &'src_file str,
        dest_file: &'dest_file str,
        trans_mode: Option<TransmissionMode>,
        closure_requested: Option<bool>,
    ) -> Self {
        Self {
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
        }
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
        if let Some(msgs_to_user) = self.msgs_to_user {
            for msg_to_user in msgs_to_user {
                if msg_to_user.tlv_type().is_none() {
                    return false;
                }
                if msg_to_user.tlv_type().unwrap() != TlvType::MsgToUser {
                    return false;
                }
            }
        }
        generic_tlv_list_type_check(self.fault_handler_overrides, TlvType::FaultHandler);
        generic_tlv_list_type_check(self.fs_requests, TlvType::FilestoreRequest);
        true
    }
}

pub fn generic_tlv_list_type_check(opt_tlvs: Option<&[Tlv<'_>]>, tlv_type: TlvType) -> bool {
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
    use super::*;
    use alloc::string::ToString;
    use spacepackets::cfdp::tlv::{msg_to_user::MsgToUserTlv, TlvOwned};

    #[derive(Debug, Clone, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct PutRequestOwned {
        pub destination_id: UnsignedByteField,
        pub source_file: Option<alloc::string::String>,
        pub dest_file: Option<alloc::string::String>,
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
        ) -> Self {
            Self {
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
            }
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
    }
}
