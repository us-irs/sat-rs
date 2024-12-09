use alloc::vec::Vec;
use hashbrown::HashMap;

use crate::{
    mode::{Mode, ModeAndSubmode, ModeReply, ModeRequest, Submode},
    request::MessageSenderProvider,
    ComponentId,
};

#[cfg(feature = "alloc")]
pub use alloc_mod::*;

/// Common trait for node modes which can have mode parents or mode children.
pub trait ModeNode {
    fn id(&self) -> ComponentId;
}
/// Trait which denotes that an object is a parent in a mode tree.
///
/// A mode parent is capable of sending mode requests to child objects and has a unique component
/// ID.
pub trait ModeParent: ModeNode {
    type Sender: MessageSenderProvider<ModeRequest>;

    fn add_mode_child(&mut self, id: ComponentId, request_sender: Self::Sender);
}

/// Trait which denotes that an object is a child in a mode tree.
///
/// A child is capable of sending mode replies to parent objects and has a unique component ID.
pub trait ModeChild: ModeNode {
    type Sender: MessageSenderProvider<ModeReply>;

    fn add_mode_parent(&mut self, id: ComponentId, reply_sender: Self::Sender);
}

/// Utility method which connects a mode tree parent object to a child object by calling
/// [ModeParent::add_mode_child] on the [parent][ModeParent] and calling
/// [ModeChild::add_mode_parent] on the [child][ModeChild].
///
/// # Arguments
///
/// * `parent` - The parent object which implements [ModeParent].
/// * `request_sender` - Sender object to send mode requests to the child.
/// * `child` - The child object which implements [ModeChild].
/// * `reply_sender` - Sender object to send mode replies to the parent.
pub fn connect_mode_nodes<ReqSender, ReplySender>(
    parent: &mut impl ModeParent<Sender = ReqSender>,
    request_sender: ReqSender,
    child: &mut impl ModeChild<Sender = ReplySender>,
    reply_sender: ReplySender,
) {
    parent.add_mode_child(child.id(), request_sender);
    child.add_mode_parent(parent.id(), reply_sender);
}

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
        allowed_submode_mask: Option<Submode>,
    ) -> Self {
        Self {
            common: ModeTableEntryCommon {
                name,
                target_id,
                mode_submode,
                allowed_submode_mask,
            },
            monitor_state: true,
        }
    }

    pub fn new_with_precise_submode(
        name: &'static str,
        target_id: ComponentId,
        mode_submode: ModeAndSubmode,
    ) -> Self {
        Self {
            common: ModeTableEntryCommon {
                name,
                target_id,
                mode_submode,
                allowed_submode_mask: None,
            },
            monitor_state: true,
        }
    }

    delegate::delegate! {
        to self.common {
            pub fn set_allowed_submode_mask(&mut self, mask: Submode);
            pub fn allowed_submode_mask(&self) -> Option<Submode>;
        }
    }
}

/// An entry for the sequence tables.
///
/// The `check_success` field instructs the mode sequence executor to verify that the
/// target mode was actually reached before executing the next sequence.
#[derive(Debug)]
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

#[derive(Debug, thiserror::Error)]
#[error("target {0} not in mode store")]
pub struct TargetNotInModeStoreError(pub ComponentId);

pub trait ModeStoreProvider {
    fn add_component(&mut self, target_id: ComponentId, mode: ModeAndSubmode);

    fn has_component(&self, target_id: ComponentId) -> bool;

    fn get_mode(&self, target_id: ComponentId) -> Option<ModeAndSubmode>;

    fn set_mode_for_contained_component(&mut self, target_id: ComponentId, mode: ModeAndSubmode);

    fn set_mode(
        &mut self,
        target_id: ComponentId,
        mode: ModeAndSubmode,
    ) -> Result<(), TargetNotInModeStoreError> {
        if !self.has_component(target_id) {
            return Err(TargetNotInModeStoreError(target_id));
        }
        self.set_mode_for_contained_component(target_id, mode);
        Ok(())
    }
}

#[cfg(feature = "alloc")]
pub mod alloc_mod {
    use super::*;

    #[derive(Debug)]
    pub struct TargetTablesMapValue {
        /// Name for a given mode table entry.
        pub name: &'static str,
        /// Optional fallback mode if the target mode can not be kept.
        pub fallback_mode: Option<Mode>,
        /// These are the rows of the a target table.
        pub entries: Vec<TargetTableEntry>,
    }

    impl TargetTablesMapValue {
        pub fn new(name: &'static str, fallback_mode: Option<Mode>) -> Self {
            Self {
                name,
                fallback_mode,
                entries: Default::default(),
            }
        }

        pub fn add_entry(&mut self, entry: TargetTableEntry) {
            self.entries.push(entry);
        }
    }

    #[derive(Debug)]
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

    #[derive(Debug)]
    pub struct SequenceTablesMapValue {
        /// Name for a given mode sequence.
        pub name: &'static str,
        /// Each sequence can consists of multiple sequences that are executed consecutively.
        pub entries: Vec<SequenceTableMapTable>,
    }

    impl SequenceTablesMapValue {
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

    #[derive(Debug, Default)]
    pub struct TargetModeTables(pub HashMap<Mode, TargetTablesMapValue>);
    #[derive(Debug, Default)]
    pub struct SequenceModeTables(pub HashMap<Mode, SequenceTablesMapValue>);

    #[derive(Debug)]
    pub struct ModeStoreVecValue {
        id: ComponentId,
        mode_and_submode: ModeAndSubmode,
        pub awaiting_reply: bool,
    }

    impl ModeStoreVecValue {
        pub fn new(id: ComponentId, mode_and_submode: ModeAndSubmode) -> Self {
            Self {
                id,
                mode_and_submode,
                awaiting_reply: false,
            }
        }

        pub fn id(&self) -> ComponentId {
            self.id
        }

        pub fn mode_and_submode(&self) -> ModeAndSubmode {
            self.mode_and_submode
        }
    }

    #[derive(Debug, Default)]
    pub struct ModeStoreVec(pub alloc::vec::Vec<ModeStoreVecValue>);
    #[derive(Debug, Default)]
    pub struct ModeStoreMap(pub hashbrown::HashMap<ComponentId, ModeAndSubmode>);

    impl ModeStoreProvider for ModeStoreVec {
        fn add_component(&mut self, target_id: ComponentId, mode: ModeAndSubmode) {
            self.0.push(ModeStoreVecValue::new(target_id, mode));
        }

        fn has_component(&self, target_id: ComponentId) -> bool {
            self.0.iter().any(|val| val.id == target_id)
        }

        fn get_mode(&self, target_id: ComponentId) -> Option<ModeAndSubmode> {
            self.0.iter().find_map(|val| {
                if val.id == target_id {
                    return Some(val.mode_and_submode);
                }
                None
            })
        }

        fn set_mode_for_contained_component(
            &mut self,
            target_id: ComponentId,
            mode_to_set: ModeAndSubmode,
        ) {
            self.0.iter_mut().for_each(|val| {
                if val.id == target_id {
                    val.mode_and_submode = mode_to_set;
                }
            });
        }
    }

    impl ModeStoreProvider for ModeStoreMap {
        fn add_component(&mut self, target_id: ComponentId, mode: ModeAndSubmode) {
            self.0.insert(target_id, mode);
        }

        fn has_component(&self, target_id: ComponentId) -> bool {
            self.0.contains_key(&target_id)
        }

        fn get_mode(&self, target_id: ComponentId) -> Option<ModeAndSubmode> {
            self.0.get(&target_id).copied()
        }
        fn set_mode_for_contained_component(
            &mut self,
            target_id: ComponentId,
            mode_to_set: ModeAndSubmode,
        ) {
            self.0.insert(target_id, mode_to_set);
        }
    }
}

#[cfg(test)]
mod tests {}
