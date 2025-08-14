use core::time::Duration;

use derive_new::new;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "std")]
#[allow(unused_imports)]
pub use std_mod::*;

use crate::request::MessageMetadata;

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SwitchState {
    Off = 0,
    On = 1,
    Unknown = 2,
    Faulty = 3,
}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SwitchStateBinary {
    Off = 0,
    On = 1,
}

impl TryFrom<SwitchState> for SwitchStateBinary {
    type Error = ();
    fn try_from(value: SwitchState) -> Result<Self, Self::Error> {
        match value {
            SwitchState::Off => Ok(SwitchStateBinary::Off),
            SwitchState::On => Ok(SwitchStateBinary::On),
            _ => Err(()),
        }
    }
}

impl<T: Into<u64>> From<T> for SwitchStateBinary {
    fn from(value: T) -> Self {
        if value.into() == 0 {
            return SwitchStateBinary::Off;
        }
        SwitchStateBinary::On
    }
}

impl From<SwitchStateBinary> for SwitchState {
    fn from(value: SwitchStateBinary) -> Self {
        match value {
            SwitchStateBinary::Off => SwitchState::Off,
            SwitchStateBinary::On => SwitchState::On,
        }
    }
}

pub type SwitchId = u16;

/// Generic trait for a device capable of turning on and off switches.
pub trait PowerSwitcherCommandSender<SwitchType: Into<u16>> {
    type Error: core::fmt::Debug;

    fn send_switch_on_cmd(
        &self,
        requestor_info: MessageMetadata,
        switch_id: SwitchType,
    ) -> Result<(), Self::Error>;
    fn send_switch_off_cmd(
        &self,
        requestor_info: MessageMetadata,
        switch_id: SwitchType,
    ) -> Result<(), Self::Error>;
}

pub trait PowerSwitchInfo<SwitchType> {
    type Error: core::fmt::Debug;

    /// Retrieve the switch state
    fn switch_state(&self, switch_id: SwitchType) -> Result<SwitchState, Self::Error>;

    fn is_switch_on(&self, switch_id: SwitchType) -> Result<bool, Self::Error> {
        Ok(self.switch_state(switch_id)? == SwitchState::On)
    }

    /// The maximum delay it will take to change a switch.
    ///
    /// This may take into account the time to send a command, wait for it to be executed, and
    /// see the switch changed.
    fn switch_delay_ms(&self) -> Duration;
}

#[derive(new)]
pub struct SwitchRequest {
    switch_id: SwitchId,
    target_state: SwitchStateBinary,
}

impl SwitchRequest {
    pub fn switch_id(&self) -> SwitchId {
        self.switch_id
    }

    pub fn target_state(&self) -> SwitchStateBinary {
        self.target_state
    }
}

#[cfg(feature = "std")]
pub mod std_mod {
    use std::sync::mpsc;

    use crate::{
        queue::GenericSendError,
        request::{GenericMessage, MessageMetadata},
    };

    use super::*;

    pub type MpscSwitchCmdSender = mpsc::Sender<GenericMessage<SwitchRequest>>;
    pub type MpscSwitchCmdSenderBounded = mpsc::SyncSender<GenericMessage<SwitchRequest>>;

    impl<SwitchType: Into<u16>> PowerSwitcherCommandSender<SwitchType> for MpscSwitchCmdSender {
        type Error = GenericSendError;

        fn send_switch_on_cmd(
            &self,
            requestor_info: MessageMetadata,
            switch_id: SwitchType,
        ) -> Result<(), Self::Error> {
            self.send(GenericMessage::new(
                requestor_info,
                SwitchRequest::new(switch_id.into(), SwitchStateBinary::On),
            ))
            .map_err(|_| GenericSendError::RxDisconnected)
        }

        fn send_switch_off_cmd(
            &self,
            requestor_info: MessageMetadata,
            switch_id: SwitchType,
        ) -> Result<(), Self::Error> {
            self.send(GenericMessage::new(
                requestor_info,
                SwitchRequest::new(switch_id.into(), SwitchStateBinary::Off),
            ))
            .map_err(|_| GenericSendError::RxDisconnected)
        }
    }

    impl<SwitchType: Into<u16>> PowerSwitcherCommandSender<SwitchType> for MpscSwitchCmdSenderBounded {
        type Error = GenericSendError;

        fn send_switch_on_cmd(
            &self,
            requestor_info: MessageMetadata,
            switch_id: SwitchType,
        ) -> Result<(), Self::Error> {
            self.try_send(GenericMessage::new(
                requestor_info,
                SwitchRequest::new(switch_id.into(), SwitchStateBinary::On),
            ))
            .map_err(|e| match e {
                mpsc::TrySendError::Full(_) => GenericSendError::QueueFull(None),
                mpsc::TrySendError::Disconnected(_) => GenericSendError::RxDisconnected,
            })
        }

        fn send_switch_off_cmd(
            &self,
            requestor_info: MessageMetadata,
            switch_id: SwitchType,
        ) -> Result<(), Self::Error> {
            self.try_send(GenericMessage::new(
                requestor_info,
                SwitchRequest::new(switch_id.into(), SwitchStateBinary::Off),
            ))
            .map_err(|e| match e {
                mpsc::TrySendError::Full(_) => GenericSendError::QueueFull(None),
                mpsc::TrySendError::Disconnected(_) => GenericSendError::RxDisconnected,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    // TODO: Add unittests for PowerSwitcherCommandSender impls for mpsc.

    use std::sync::mpsc::{self, TryRecvError};

    use crate::{ComponentId, queue::GenericSendError, request::GenericMessage};

    use super::*;

    const TEST_REQ_ID: u32 = 2;
    const TEST_SENDER_ID: ComponentId = 5;

    const TEST_SWITCH_ID: u16 = 0x1ff;

    fn common_checks(request: &GenericMessage<SwitchRequest>) {
        assert_eq!(request.requestor_info.sender_id(), TEST_SENDER_ID);
        assert_eq!(request.requestor_info.request_id(), TEST_REQ_ID);
        assert_eq!(request.message.switch_id(), TEST_SWITCH_ID);
    }

    #[test]
    fn test_comand_switch_sending_mpsc_regular_on_cmd() {
        let (switch_cmd_tx, switch_cmd_rx) = mpsc::channel::<GenericMessage<SwitchRequest>>();
        switch_cmd_tx
            .send_switch_on_cmd(
                MessageMetadata::new(TEST_REQ_ID, TEST_SENDER_ID),
                TEST_SWITCH_ID,
            )
            .expect("sending switch cmd failed");
        let request = switch_cmd_rx
            .recv()
            .expect("receiving switch request failed");
        common_checks(&request);
        assert_eq!(request.message.target_state(), SwitchStateBinary::On);
    }

    #[test]
    fn test_comand_switch_sending_mpsc_regular_off_cmd() {
        let (switch_cmd_tx, switch_cmd_rx) = mpsc::channel::<GenericMessage<SwitchRequest>>();
        switch_cmd_tx
            .send_switch_off_cmd(
                MessageMetadata::new(TEST_REQ_ID, TEST_SENDER_ID),
                TEST_SWITCH_ID,
            )
            .expect("sending switch cmd failed");
        let request = switch_cmd_rx
            .recv()
            .expect("receiving switch request failed");
        common_checks(&request);
        assert_eq!(request.message.target_state(), SwitchStateBinary::Off);
    }

    #[test]
    fn test_comand_switch_sending_mpsc_regular_rx_disconnected() {
        let (switch_cmd_tx, switch_cmd_rx) = mpsc::channel::<GenericMessage<SwitchRequest>>();
        drop(switch_cmd_rx);
        let result = switch_cmd_tx.send_switch_off_cmd(
            MessageMetadata::new(TEST_REQ_ID, TEST_SENDER_ID),
            TEST_SWITCH_ID,
        );
        assert!(result.is_err());
        matches!(result.unwrap_err(), GenericSendError::RxDisconnected);
    }

    #[test]
    fn test_comand_switch_sending_mpsc_sync_on_cmd() {
        let (switch_cmd_tx, switch_cmd_rx) = mpsc::sync_channel::<GenericMessage<SwitchRequest>>(3);
        switch_cmd_tx
            .send_switch_on_cmd(
                MessageMetadata::new(TEST_REQ_ID, TEST_SENDER_ID),
                TEST_SWITCH_ID,
            )
            .expect("sending switch cmd failed");
        let request = switch_cmd_rx
            .recv()
            .expect("receiving switch request failed");
        common_checks(&request);
        assert_eq!(request.message.target_state(), SwitchStateBinary::On);
    }

    #[test]
    fn test_comand_switch_sending_mpsc_sync_off_cmd() {
        let (switch_cmd_tx, switch_cmd_rx) = mpsc::sync_channel::<GenericMessage<SwitchRequest>>(3);
        switch_cmd_tx
            .send_switch_off_cmd(
                MessageMetadata::new(TEST_REQ_ID, TEST_SENDER_ID),
                TEST_SWITCH_ID,
            )
            .expect("sending switch cmd failed");
        let request = switch_cmd_rx
            .recv()
            .expect("receiving switch request failed");
        common_checks(&request);
        assert_eq!(request.message.target_state(), SwitchStateBinary::Off);
    }

    #[test]
    fn test_comand_switch_sending_mpsc_sync_rx_disconnected() {
        let (switch_cmd_tx, switch_cmd_rx) = mpsc::sync_channel::<GenericMessage<SwitchRequest>>(1);
        drop(switch_cmd_rx);
        let result = switch_cmd_tx.send_switch_off_cmd(
            MessageMetadata::new(TEST_REQ_ID, TEST_SENDER_ID),
            TEST_SWITCH_ID,
        );
        assert!(result.is_err());
        matches!(result.unwrap_err(), GenericSendError::RxDisconnected);
    }

    #[test]
    fn test_comand_switch_sending_mpsc_sync_queue_full() {
        let (switch_cmd_tx, switch_cmd_rx) = mpsc::sync_channel::<GenericMessage<SwitchRequest>>(1);
        let mut result = switch_cmd_tx.send_switch_off_cmd(
            MessageMetadata::new(TEST_REQ_ID, TEST_SENDER_ID),
            TEST_SWITCH_ID,
        );
        assert!(result.is_ok());
        result = switch_cmd_tx.send_switch_off_cmd(
            MessageMetadata::new(TEST_REQ_ID, TEST_SENDER_ID),
            TEST_SWITCH_ID,
        );
        assert!(result.is_err());
        matches!(result.unwrap_err(), GenericSendError::QueueFull(None));
        matches!(switch_cmd_rx.try_recv(), Err(TryRecvError::Empty));
    }
}
