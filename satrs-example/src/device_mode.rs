use std::time::Duration;

use types::pcdu::SwitchId;

use crate::eps::PowerSwitchHelper;

/// Modes that distinguish a powered-off state from one or more powered-on states, so
/// [`SwitchAndModeHelper`] knows which way to drive the switch for a given target mode.
pub trait PowerSwitchedMode: Copy + PartialEq {
    fn requires_power(&self) -> bool;
}

impl PowerSwitchedMode for types::DeviceMode {
    fn requires_power(&self) -> bool {
        *self != types::DeviceMode::Off
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
enum SwitchTransitionState {
    #[default]
    Idle,
    PowerSwitching,
    Done,
}

/// Outcome of a pending mode transition, once [`SwitchAndModeHelper::handle_mode_transition`]
/// has driven it to completion. Carries back whichever TC commanded the transition, if any, so
/// the caller can reply to it -- what that reply looks like is handler-specific, so this stays
/// out of the helper.
pub enum ModeTransitionEvent {
    Reached(Option<satrs::spacepackets::CcsdsPacketIdAndPsc>),
    Failed(Option<satrs::spacepackets::CcsdsPacketIdAndPsc>),
}

/// Drives the on/off power-switch commanding state machine (Idle -> PowerSwitching -> Done)
/// shared by every device handler that owns a single power switch of its own.
///
/// Handler-specific reactions (sending telemetry, invalidating cached sensor data, reporting to
/// a parent) are not this helper's concern: [`Self::handle_mode_transition`] just reports when a
/// transition finishes (or fails) and leaves what to do about it to the caller.
pub struct SwitchAndModeHelper<Mode: PowerSwitchedMode> {
    mode_helper: satrs_example::ModeHelper<Mode, SwitchTransitionState>,
    switch_helper: PowerSwitchHelper,
    switch_id: SwitchId,
}

impl<Mode: PowerSwitchedMode> SwitchAndModeHelper<Mode> {
    pub fn new(
        init_mode: Mode,
        timeout: Duration,
        switch_helper: PowerSwitchHelper,
        switch_id: SwitchId,
    ) -> Self {
        Self {
            mode_helper: satrs_example::ModeHelper::new(init_mode, timeout),
            switch_helper,
            switch_id,
        }
    }

    #[inline]
    pub fn mode(&self) -> Mode {
        self.mode_helper.current
    }

    #[inline]
    pub fn target(&self) -> Option<Mode> {
        self.mode_helper.target
    }

    pub fn start_transition(
        &mut self,
        target_mode: Mode,
        tc_commander: Option<satrs::spacepackets::CcsdsPacketIdAndPsc>,
    ) {
        self.mode_helper.tc_commander = tc_commander;
        self.mode_helper.start(target_mode);
    }

    pub fn handle_mode_transition(&mut self) -> Option<ModeTransitionEvent> {
        let target_mode = self.mode_helper.target?;
        let switch_target_on = target_mode.requires_power();
        if self.mode_helper.transition_state == SwitchTransitionState::Idle {
            let result = if switch_target_on {
                self.switch_helper.send_switch_on_cmd(self.switch_id)
            } else {
                self.switch_helper.send_switch_off_cmd(self.switch_id)
            };
            if result.is_err() {
                // Could not send switch command.. still continue with transition.
                log::error!(
                    "failed to send switch {} command",
                    if switch_target_on { "on" } else { "off" }
                );
            }
            self.mode_helper.transition_state = SwitchTransitionState::PowerSwitching;
        }
        if self.mode_helper.transition_state == SwitchTransitionState::PowerSwitching {
            if self.switch_helper.is_switch_on(self.switch_id) == switch_target_on {
                log::info!("switch is {}", if switch_target_on { "on" } else { "off" });
                self.mode_helper.transition_state = SwitchTransitionState::Done;
            } else if self.mode_helper.timed_out() {
                return Some(ModeTransitionEvent::Failed(self.mode_helper.finish(false)));
            }
        }
        if self.mode_helper.transition_state == SwitchTransitionState::Done {
            return Some(ModeTransitionEvent::Reached(self.mode_helper.finish(true)));
        }
        None
    }
}
