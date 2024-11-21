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

/// Common fields required for both target and sequence table entries.
///
/// The most important parameters here are the target ID which this entry belongs to, and the mode
/// and submode the entry either will be commanded to for sequence table entries or which will be
/// monitored for target table entries.
#[derive(Debug, Copy, Clone)]
pub struct ModeTableEntryCommon {
    /// Name of respective table entry.
    pub name: &'static str,
    /// Target component ID.
    pub target_id: ComponentId,
    /// Has a different meaning depending on whether this is a sequence table or a target table.
    ///
    ///  - For sequence tables, this denotes the mode which will be commanded
    ///  - For target tables, this is the mode which the target children should have and which
    ///    might be monitored depending on configuration.
    pub mode_submode: ModeAndSubmode,
    /// This mask allows to specify multiple allowed submodes for a given mode.
    pub allowed_submode_mask: Option<Submode>,
}

impl ModeTableEntryCommon {
    pub fn set_allowed_submode_mask(&mut self, mask: Submode) {
        self.allowed_submode_mask = Some(mask);
    }

    pub fn allowed_submode_mask(&self) -> Option<Submode> {
        self.allowed_submode_mask
    }
}

/// An entry for the target tables.
#[derive(Debug)]
pub struct TargetTableEntry {
    pub common: ModeTableEntryCommon,
    pub monitor_state: bool,
}

impl TargetTableEntry {
    pub fn new(
        name: &'static str,
        target_id: ComponentId,
        mode_submode: ModeAndSubmode,
        monitor_state: bool,
    ) -> Self {
        Self {
            common: ModeTableEntryCommon {
                name,
                target_id,
                mode_submode,
                allowed_submode_mask: None,
            },
            monitor_state,
        }
    }

    delegate::delegate! {
        to self.common {
            pub fn set_allowed_submode_mask(&mut self, mask: Submode);
            pub fn allowed_submode_mask(&self) -> Option<Submode>;
        }
    }
}

#[derive(Debug)]
pub struct TargetTableMapValue {
    /// Name for a given mode table entry.
    pub name: &'static str,
    /// These are the rows of the a target table.
    pub entries: Vec<TargetTableEntry>,
}

impl TargetTableMapValue {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            entries: Default::default(),
        }
    }

    pub fn add_entry(&mut self, entry: TargetTableEntry) {
        self.entries.push(entry);
    }
}

/// An entry for the sequence tables.
///
/// The `check_success` field instructs the mode sequence executor to verify that the
/// target mode was actually reached before executing the next sequence.
pub struct SequenceTableEntry {
    pub common: ModeTableEntryCommon,
    pub check_success: bool,
}

impl SequenceTableEntry {
    pub fn new(
        name: &'static str,
        target_id: ComponentId,
        mode_submode: ModeAndSubmode,
        check_success: bool,
    ) -> Self {
        Self {
            common: ModeTableEntryCommon {
                name,
                target_id,
                mode_submode,
                allowed_submode_mask: None,
            },
            check_success,
        }
    }

    delegate::delegate! {
        to self.common {
            pub fn set_allowed_submode_mask(&mut self, mask: Submode);
            pub fn allowed_submode_mask(&self) -> Option<Submode>;
        }
    }
}

pub struct SequenceTableMapTable {
    /// Name for a given mode sequence.
    pub name: &'static str,
    /// These are the rows of the a sequence table.
    pub entries: Vec<SequenceTableEntry>,
}

impl SequenceTableMapTable {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            entries: Default::default(),
        }
    }

    pub fn add_entry(&mut self, entry: SequenceTableEntry) {
        self.entries.push(entry);
    }
}

pub struct SequenceTableMapValue {
    /// Name for a given mode sequence.
    pub name: &'static str,
    /// Each sequence can consists of multiple sequences that are executed consecutively.
    pub entries: Vec<SequenceTableMapTable>,
}

impl SequenceTableMapValue {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            entries: Default::default(),
        }
    }

    pub fn add_sequence_table(&mut self, entry: SequenceTableMapTable) {
        self.entries.push(entry);
    }
}

pub struct TargetModeTable(pub HashMap<Mode, TargetTableMapValue>);
pub struct SequenceModeTable(pub HashMap<Mode, SequenceTableMapValue>);

#[cfg(test)]
mod tests {}
