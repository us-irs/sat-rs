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
    const TASK_NAME: &'static str;
    fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, Self::Error>;
}

trait ExecutableWithAssociatedFuncs {
    type Error;
    fn exec_type(&self) -> ExecutionType;
    fn task_name(&self) -> &'static str;
    fn periodic_op(&mut self) -> Result<OpResult, Self::Error>;
}
