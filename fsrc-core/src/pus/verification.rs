use crate::pus::SendStoredTmError;
use alloc::boxed::Box;
use alloc::vec::Vec;
use spacepackets::tc::PusTc;
use spacepackets::time::{TimeWriter, TimestampError};
use spacepackets::tm::{PusTm, PusTmSecondaryHeader};
use spacepackets::SpHeader;
use spacepackets::{CcsdsPacket, PacketId, PacketSequenceCtrl};
use std::marker::PhantomData;

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct RequestId {
    version_number: u8,
    packet_id: PacketId,
    psc: PacketSequenceCtrl,
}

impl RequestId {
    pub fn new(tc: &PusTc) -> Self {
        RequestId {
            version_number: tc.ccsds_version(),
            packet_id: tc.packet_id(),
            psc: tc.psc(),
        }
    }
}

pub trait VerificationSender<E> {
    fn send_verification_tm(&mut self, tm: PusTm) -> Result<(), SendStoredTmError<E>>;
}

pub struct VerificationReporter {
    apid: u16,
    msg_count: u16,
    dest_id: u16,
    time_stamper: Box<dyn TimeWriter>,
    time_stamp_buf: Vec<u8>,
}

impl VerificationReporter {
    pub fn add_tc(&mut self, req_id: RequestId) -> VerificationToken<StateNone> {
        VerificationToken::<StateNone>::new(req_id)
    }

    pub fn acceptance_success<E>(
        &mut self,
        token: VerificationToken<StateNone>,
        sender: &mut impl VerificationSender<E>,
    ) -> Result<VerificationToken<StateAccepted>, SendStoredTmError<E>> {
        let tm = self
            .create_tm(1, 1)
            .map_err(|e| SendStoredTmError::TimeStampError(e))?;
        sender.send_verification_tm(tm)?;
        self.msg_count += 1;
        Ok(VerificationToken {
            state: PhantomData,
            req_id: token.req_id,
        })
    }

    pub fn acceptance_failure(self, _token: VerificationToken<StateNone>) {
        unimplemented!();
    }

    pub fn start_success(
        &mut self,
        token: VerificationToken<StateAccepted>,
    ) -> VerificationToken<StateStarted> {
        VerificationToken {
            state: PhantomData,
            req_id: token.req_id,
        }
    }

    pub fn start_failure(&mut self, _token: VerificationToken<StateAccepted>) {
        unimplemented!();
    }

    pub fn step_success(&mut self, _token: &VerificationToken<StateAccepted>) {
        unimplemented!();
    }

    pub fn step_failure(&mut self, _token: VerificationToken<StateAccepted>) {
        unimplemented!();
    }

    pub fn completion_success(&mut self, _token: VerificationToken<StateAccepted>) {
        unimplemented!();
    }
    pub fn completion_failure(&mut self, _token: VerificationToken<StateAccepted>) {
        unimplemented!();
    }

    fn create_tm(&mut self, service: u8, subservice: u8) -> Result<PusTm, TimestampError> {
        let mut sp_header = SpHeader::tm(self.apid, 0, 0).unwrap();
        // I think it is okay to panic here. This error should never happen, I consider
        // this a configuration error.
        self.time_stamper
            .write_to_bytes(self.time_stamp_buf.as_mut_slice())?;
        let tm_sec_header = PusTmSecondaryHeader::new(
            service,
            subservice,
            self.msg_count,
            self.dest_id,
            &self.time_stamp_buf,
        );
        Ok(PusTm::new(&mut sp_header, tm_sec_header, None, true))
    }
}

pub struct VerificationToken<STATE> {
    state: PhantomData<STATE>,
    req_id: RequestId,
}

pub struct StateNone;
pub struct StateAccepted;
pub struct StateStarted;

impl<STATE> VerificationToken<STATE> {
    fn new(req_id: RequestId) -> VerificationToken<StateNone> {
        VerificationToken {
            state: PhantomData,
            req_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pus::verification::VerificationToken;

    #[test]
    pub fn test_basic_type_state() {
        //let mut reporter = VerificationToken::new();
        //let mut accepted = reporter.acceptance_success();
        //let started = accepted.start_success();
        //started.completion_success();
    }
}
