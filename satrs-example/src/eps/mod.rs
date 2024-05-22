use derive_new::new;
use std::{
    borrow::BorrowMut,
    cell::RefCell,
    collections::VecDeque,
    sync::mpsc,
    time::{Duration, Instant},
};

use satrs::{
    power::{
        PowerSwitchInfo, PowerSwitcherCommandSender, SwitchId, SwitchRequest, SwitchState,
        SwitchStateBinary,
    },
    queue::GenericSendError,
    request::{GenericMessage, MessageMetadata},
};
use satrs_minisim::eps::{PcduSwitch, SwitchMapWrapper};
use thiserror::Error;

use self::pcdu::SharedSwitchSet;

pub mod pcdu;

#[derive(new, Clone)]
pub struct PowerSwitchHelper {
    switcher_tx: mpsc::SyncSender<GenericMessage<SwitchRequest>>,
    shared_switch_set: SharedSwitchSet,
    #[new(default)]
    switch_cmd_sent_instant: Option<Instant>,
}

#[derive(Debug, Error, Copy, Clone, PartialEq, Eq)]
pub enum SwitchCommandingError {
    #[error("invalid switch id")]
    InvalidSwitchId(SwitchId),
    #[error("send error: {0}")]
    Send(#[from] GenericSendError),
}
#[derive(Debug, Error, Copy, Clone, PartialEq, Eq)]
pub enum SwitchInfoError {
    /// This is a configuration error which should not occur.
    #[error("switch ID not in map")]
    SwitchIdNotInMap(PcduSwitch),
    #[error("switch set invalid")]
    SwitchSetInvalid,
}

impl PowerSwitchInfo<PcduSwitch> for PowerSwitchHelper {
    type Error = SwitchInfoError;

    fn switch_state(
        &self,
        switch_id: PcduSwitch,
    ) -> Result<satrs::power::SwitchState, Self::Error> {
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

impl PowerSwitcherCommandSender<PcduSwitch> for PowerSwitchHelper {
    type Error = SwitchCommandingError;

    fn send_switch_on_cmd(
        &self,
        requestor_info: satrs::request::MessageMetadata,
        switch_id: PcduSwitch,
    ) -> Result<(), Self::Error> {
        self.switcher_tx
            .send_switch_on_cmd(requestor_info, switch_id)?;
        Ok(())
    }

    fn send_switch_off_cmd(
        &self,
        requestor_info: satrs::request::MessageMetadata,
        switch_id: PcduSwitch,
    ) -> Result<(), Self::Error> {
        self.switcher_tx
            .send_switch_off_cmd(requestor_info, switch_id)?;
        Ok(())
    }
}

#[derive(new)]
pub struct SwitchRequestInfo {
    pub requestor_info: MessageMetadata,
    pub switch_id: PcduSwitch,
    pub target_state: satrs::power::SwitchStateBinary,
}

// Test switch helper which can be used for unittests.
pub struct TestSwitchHelper {
    pub switch_requests: RefCell<VecDeque<SwitchRequestInfo>>,
    pub switch_info_requests: RefCell<VecDeque<PcduSwitch>>,
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

impl PowerSwitchInfo<PcduSwitch> for TestSwitchHelper {
    type Error = SwitchInfoError;

    fn switch_state(
        &self,
        switch_id: PcduSwitch,
    ) -> Result<satrs::power::SwitchState, Self::Error> {
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

impl PowerSwitcherCommandSender<PcduSwitch> for TestSwitchHelper {
    type Error = SwitchCommandingError;

    fn send_switch_on_cmd(
        &self,
        requestor_info: MessageMetadata,
        switch_id: PcduSwitch,
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
        switch_id: PcduSwitch,
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

impl TestSwitchHelper {
    // Helper function which can be used to force a switch to another state for test purposes.
    pub fn set_switch_state(&mut self, switch: PcduSwitch, state: SwitchState) {
        self.switch_map.get_mut().0.insert(switch, state);
    }
}
