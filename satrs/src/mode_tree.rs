use alloc::vec::Vec;
use hashbrown::HashMap;

use crate::{
    mode::{Mode, ModeAndSubmode, Submode},
    ComponentId,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TableEntryType {
    /// Target table containing information of the expected children modes for  given mode.
    Target,
    /// Sequence table which contains information about how to reach a target table, including
    /// the order of the sequences.
    Sequence,
}

pub struct ModeTableEntry {
    /// Name of respective table entry.
    pub name: &'static str,
    /// Target channel ID.
    pub channel_id: ComponentId,
    pub mode_submode: ModeAndSubmode,
    pub allowed_submode_mask: Option<Submode>,
    pub check_success: bool,
}

pub struct ModeTableMapValue {
    /// Name for a given mode table entry.
    pub name: &'static str,
    pub entries: Vec<ModeTableEntry>,
}

pub type ModeTable = HashMap<Mode, ModeTableMapValue>;

#[cfg(test)]
mod tests {}
