use crate::ComponentId;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum HealthState {
    Healthy = 1,
    Faulty = 2,
    PermanentFaulty = 3,
    ExternalControl = 4,
    NeedsRecovery = 5,
}

pub trait HealthTableProvider {
    fn health(&self, id: ComponentId) -> Option<HealthState>;
    fn set_health(&mut self, id: ComponentId, health: HealthState);
}

#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub struct HealthTableMapSync(
    std::sync::Arc<std::sync::Mutex<hashbrown::HashMap<ComponentId, HealthState>>>,
);

#[cfg(feature = "std")]
impl HealthTableMapSync {
    pub fn new(health_table: hashbrown::HashMap<ComponentId, HealthState>) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(health_table)))
    }
}

#[cfg(feature = "std")]
impl HealthTableProvider for HealthTableMapSync {
    fn health(&self, id: ComponentId) -> Option<HealthState> {
        self.0.lock().unwrap().get(&id).copied()
    }

    fn set_health(&mut self, id: ComponentId, health: HealthState) {
        self.0.lock().unwrap().insert(id, health);
    }
}
