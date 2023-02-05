
pub trait PowerSwitch {
    fn switch_on(&mut self);
    fn switch_off(&mut self);

    fn is_switch_on(&self) -> bool {
        self.switch_state() == SwitchState::On
    }

    fn switch_state(&self) -> SwitchState;
}

pub enum SwitchState {
    Off = 0,
    On = 1,
    Unknown = 2
}

pub trait PowerSwitcher {
    fn send_switch_on_cmd(&mut self, switch_nr: u16);
    fn send_switch_off_cmd(&mut self, switch_nr: u16);

    fn get_switch_state(&mut self, switch_nr: u16) -> SwitchState;

    fn get_is_switch_on(&mut self) -> bool {
        self.get_switch_state() == SwitchState::On
    }
}