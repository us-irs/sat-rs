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
    //const EXEC_TYPE: ExecutionType;
    //const TASK_NAME: &'static str;

    fn exec_type(&self) -> ExecutionType;// {
    //    return Self::EXEC_TYPE;
    //}
    fn task_name(&self) -> &'static str;//{
    //    return Self::TASK_NAME;
    //}
    fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, Self::Error>;
}
