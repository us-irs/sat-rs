#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Generic trait for a device capable of switching itself on or off.
pub trait PowerSwitch {
    type Error;

    fn switch_on(&mut self) -> Result<(), Self::Error>;
    fn switch_off(&mut self) -> Result<(), Self::Error>;

    fn is_switch_on(&self) -> bool {
        self.switch_state() == SwitchState::On
    }

    fn switch_state(&self) -> SwitchState;
}

#[derive(Debug, Eq, PartialEq, Copy, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum SwitchState {
    Off = 0,
    On = 1,
    Unknown = 2,
    Faulty = 3,
}

pub type SwitchId = u16;

/// Generic trait for a device capable of turning on and off switches.
pub trait PowerSwitcherCommandSender {
    type Error;

    fn send_switch_on_cmd(&mut self, switch_id: SwitchId) -> Result<(), Self::Error>;
    fn send_switch_off_cmd(&mut self, switch_id: SwitchId) -> Result<(), Self::Error>;

    fn switch_on<T: PowerSwitch>(
        &mut self,
        switch: &mut T,
    ) -> Result<(), <T as PowerSwitch>::Error> {
        switch.switch_on()
    }
    fn switch_off<T: PowerSwitch>(
        &mut self,
        switch: &mut T,
    ) -> Result<(), <T as PowerSwitch>::Error> {
        switch.switch_off()
    }
}

pub trait PowerSwitchInfo {
    type Error;

    /// Retrieve the switch state
    fn get_switch_state(&mut self, switch_id: SwitchId) -> Result<SwitchState, Self::Error>;

    fn get_is_switch_on(&mut self, switch_id: SwitchId) -> Result<bool, Self::Error> {
        Ok(self.get_switch_state(switch_id)? == SwitchState::On)
    }

    /// The maximum delay it will take to change a switch.
    ///
    /// This may take into account the time to send a command, wait for it to be executed, and
    /// see the switch changed.
    fn switch_delay_ms(&self) -> u32;
}

pub trait PowerSwitchProvider: PowerSwitcherCommandSender + PowerSwitchInfo {
    type Error;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::PowerSwitcherCommandSender;
    use std::boxed::Box;

    struct Pcdu {
        switch_rx: std::sync::mpsc::Receiver<(SwitchId, u16)>,
    }
    enum DeviceState {
        OFF,
        SwitchingPower,
        ON,
        SETUP,
        IDLE,
    }
    struct MyComplexDevice {
        power_switcher: Box<dyn PowerSwitcherCommandSender<Error = ()>>,
        switch_id: SwitchId,
        some_state: u16,
        dev_state: DeviceState,
        mode: u32,
        submode: u16,
    }

    impl MyComplexDevice {
        pub fn periodic_op(&mut self) {
            // .. mode command coming in
            let mode = 1;
            if mode == 1 {
                if self.dev_state == DeviceState::OFF {
                    self.power_switcher
                        .send_switch_on_cmd(self.switch_id)
                        .expect("sending siwthc cmd failed");
                    self.dev_state = DeviceState::SwitchingPower;
                }
                if self.dev_state == DeviceState::SwitchingPower {
                    if self.power_switcher.get_is_switch_on() {
                        self.dev_state = DeviceState::ON;
                        self.mode = 1;
                    }
                }
            }
        }
    }
}
