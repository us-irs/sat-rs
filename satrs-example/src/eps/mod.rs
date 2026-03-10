use derive_new::new;
use models::pcdu::{SwitchId, SwitchRequest, SwitchState, SwitchStateBinary};
use std::{cell::RefCell, collections::VecDeque, sync::mpsc, time::Duration};

use satrs::{
    queue::GenericSendError,
    request::{GenericMessage, MessageMetadata},
};
use thiserror::Error;

use crate::eps::pcdu::SwitchMapWrapper;

use self::pcdu::SharedSwitchSet;

pub mod pcdu;

#[derive(new, Clone)]
pub struct PowerSwitchHelper {
    switcher_tx: mpsc::SyncSender<GenericMessage<SwitchRequest>>,
    shared_switch_set: SharedSwitchSet,
}

#[derive(Debug, Error, Copy, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SwitchCommandingError {
    #[error("send error: {0}")]
    Send(#[from] GenericSendError),
}

#[derive(Debug, Error, Copy, Clone, PartialEq, Eq)]
pub enum SwitchInfoError {
    /// This is a configuration error which should not occur.
    #[error("switch ID not in map")]
    SwitchIdNotInMap(SwitchId),
    #[error("switch set invalid")]
    SwitchSetInvalid,
}

impl PowerSwitchHelper {
    pub fn send_switch_on_cmd(
        &self,
        requestor_info: satrs::request::MessageMetadata,
        switch_id: SwitchId,
    ) -> Result<(), GenericSendError> {
        self.switcher_tx.send(GenericMessage::new(
            requestor_info,
            SwitchRequest::new(switch_id, SwitchStateBinary::On),
        ))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn send_switch_off_cmd(
        &self,
        requestor_info: satrs::request::MessageMetadata,
        switch_id: SwitchId,
    ) -> Result<(), GenericSendError> {
        self.switcher_tx.send(GenericMessage::new(
            requestor_info,
            SwitchRequest::new(switch_id, SwitchStateBinary::Off),
        ))?;
        Ok(())
    }

    pub fn switch_state(&self, switch_id: SwitchId) -> Result<SwitchState, SwitchInfoError> {
        let switch_set = self
            .shared_switch_set
            .lock()
            .expect("failed to lock switch set");
        if !switch_set.valid {
            return Err(SwitchInfoError::SwitchSetInvalid);
        }

        if let Some(state) = switch_set.switch_map.get(&switch_id) {
            return Ok(*state);
        }
        Err(SwitchInfoError::SwitchIdNotInMap(switch_id))
    }

    #[allow(dead_code)]
    fn switch_delay_ms(&self) -> Duration {
        // Here, we could set device specific switch delays theoretically. Set it to this value
        // for now.
        Duration::from_millis(1000)
    }

    pub fn is_switch_on(&self, switch_id: SwitchId) -> bool {
        if let Ok(state) = self.switch_state(switch_id) {
            state == SwitchState::On
        } else {
            false
        }
    }
}
/*
impl PowerSwitchInfo<SwitchId> for PowerSwitchHelper {
    type Error = SwitchInfoError;

    fn switch_state(&self, switch_id: SwitchId) -> Result<SwitchState, Self::Error> {
        let switch_set = self
            .shared_switch_set
            .lock()
            .expect("failed to lock switch set");
        if !switch_set.valid {
            return Err(SwitchInfoError::SwitchSetInvalid);
        }

        if let Some(state) = switch_set.switch_map.get(&switch_id) {
            return Ok(*state);
        }
        Err(SwitchInfoError::SwitchIdNotInMap(switch_id))
    }

    fn switch_delay_ms(&self) -> Duration {
        // Here, we could set device specific switch delays theoretically. Set it to this value
        // for now.
        Duration::from_millis(1000)
    }
}
*/

/*
impl PowerSwitcherCommandSender<SwitchId> for PowerSwitchHelper {
    type Error = SwitchCommandingError;

    fn send_switch_on_cmd(
        &self,
        requestor_info: satrs::request::MessageMetadata,
        switch_id: SwitchId,
    ) -> Result<(), Self::Error> {
        self.switcher_tx.send(GenericMessage::new(
            requestor_info,
            SwitchRequest::new(switch_id, SwitchStateBinary::On),
        ));
        Ok(())
    }

    fn send_switch_off_cmd(
        &self,
        requestor_info: satrs::request::MessageMetadata,
        switch_id: SwitchId,
    ) -> Result<(), Self::Error> {
        self.switcher_tx.send(GenericMessage::new(
            requestor_info,
            SwitchRequest::new(switch_id, SwitchStateBinary::Off),
        ));
        Ok(())
    }
}
*/

#[allow(dead_code)]
#[derive(new)]
pub struct SwitchRequestInfo {
    pub requestor_info: MessageMetadata,
    pub switch_id: SwitchId,
    pub target_state: SwitchStateBinary,
}

// Test switch helper which can be used for unittests.
#[allow(dead_code)]
pub struct TestSwitchHelper {
    pub switch_requests: RefCell<VecDeque<SwitchRequestInfo>>,
    pub switch_info_requests: RefCell<VecDeque<SwitchId>>,
    #[allow(dead_code)]
    pub switch_delay_request_count: u32,
    pub next_switch_delay: Duration,
    pub switch_map: RefCell<SwitchMapWrapper>,
    pub switch_map_valid: bool,
}

impl Default for TestSwitchHelper {
    fn default() -> Self {
        Self {
            switch_requests: Default::default(),
            switch_info_requests: Default::default(),
            switch_delay_request_count: Default::default(),
            next_switch_delay: Duration::from_millis(1000),
            switch_map: Default::default(),
            switch_map_valid: true,
        }
    }
}

/*
impl PowerSwitchInfo<SwitchId> for TestSwitchHelper {
    type Error = SwitchInfoError;

    fn switch_state(&self, switch_id: SwitchId) -> Result<satrs::power::SwitchState, Self::Error> {
        let mut switch_info_requests_mut = self.switch_info_requests.borrow_mut();
        switch_info_requests_mut.push_back(switch_id);
        if !self.switch_map_valid {
            return Err(SwitchInfoError::SwitchSetInvalid);
        }
        let switch_map_mut = self.switch_map.borrow_mut();
        if let Some(state) = switch_map_mut.0.get(&switch_id) {
            return Ok(*state);
        }
        Err(SwitchInfoError::SwitchIdNotInMap(switch_id))
    }

    fn switch_delay_ms(&self) -> Duration {
        self.next_switch_delay
    }
}

impl PowerSwitcherCommandSender<SwitchId> for TestSwitchHelper {
    type Error = SwitchCommandingError;

    fn send_switch_on_cmd(
        &self,
        requestor_info: MessageMetadata,
        switch_id: SwitchId,
    ) -> Result<(), Self::Error> {
        let mut switch_requests_mut = self.switch_requests.borrow_mut();
        switch_requests_mut.push_back(SwitchRequestInfo {
            requestor_info,
            switch_id,
            target_state: SwitchStateBinary::On,
        });
        // By default, the test helper immediately acknowledges the switch request by setting
        // the appropriate switch state in the internal switch map.
        let mut switch_map_mut = self.switch_map.borrow_mut();
        if let Some(switch_state) = switch_map_mut.0.get_mut(&switch_id) {
            *switch_state = SwitchState::On;
        }
        Ok(())
    }

    fn send_switch_off_cmd(
        &self,
        requestor_info: MessageMetadata,
        switch_id: SwitchId,
    ) -> Result<(), Self::Error> {
        let mut switch_requests_mut = self.switch_requests.borrow_mut();
        switch_requests_mut.push_back(SwitchRequestInfo {
            requestor_info,
            switch_id,
            target_state: SwitchStateBinary::Off,
        });
        // By default, the test helper immediately acknowledges the switch request by setting
        // the appropriate switch state in the internal switch map.
        let mut switch_map_mut = self.switch_map.borrow_mut();
        if let Some(switch_state) = switch_map_mut.0.get_mut(&switch_id) {
            *switch_state = SwitchState::Off;
        }
        Ok(())
    }
}
*/

#[allow(dead_code)]
impl TestSwitchHelper {
    // Helper function which can be used to force a switch to another state for test purposes.
    pub fn set_switch_state(&mut self, switch: SwitchId, state: SwitchState) {
        self.switch_map.get_mut().0.insert(switch, state);
    }
}
