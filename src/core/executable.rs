use std::time::Duration;

pub enum OpResult {
    Ok,
}

pub enum ExecutionType {
    Infinite,
    Cycles(u32),
    OneShot,
}

pub trait Executable {
    type Error;
    const EXEC_TYPE: ExecutionType;
    const TASK_FREQ: Option<Duration>;
    const TASK_NAME: &'static str;
    fn periodic_op(&mut self, op_code: u32) -> Result<OpResult, Self::Error>;
}

trait ExecutableWithAssociatedFuncs {
    type Error;
    fn exec_type(&self) -> ExecutionType;
    fn task_freq(&self) -> Option<Duration>;
    fn task_name(&self) -> &'static str;
    fn periodic_op(&mut self) -> Result<OpResult, Self::Error>;
}
