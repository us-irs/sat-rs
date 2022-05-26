pub enum OpResult {
    Ok,
}

pub enum ExecutionType {
    Infinite,
    Cycles(u32),
    OneShot,
}

pub trait Executable: Send {
    type Error;

    fn exec_type(&self) -> ExecutionType;
    fn task_name(&self) -> &'static str;
    fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, Self::Error>;
}
