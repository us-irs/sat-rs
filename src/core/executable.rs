use std::error::Error;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

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

pub fn executable_scheduler<
    T: Executable<Error = E> + Send + 'static + ?Sized,
    E: Error + Send + 'static,
>(
    mut executable_vec: Vec<Box<T>>,
    task_freq: Option<Duration>,
    op_code: i32,
) -> JoinHandle<Result<OpResult, E>> {
    let mut cycle_counts = vec![0; executable_vec.len()];
    let mut removal_flags = vec![false; executable_vec.len()];
    thread::spawn(move || loop {
        for (idx, executable) in executable_vec.iter_mut().enumerate() {
            match executable.exec_type() {
                ExecutionType::OneShot => {
                    executable.periodic_op(op_code)?;
                    removal_flags[idx] = true;
                }
                ExecutionType::Infinite => {
                    executable.periodic_op(op_code)?;
                }
                ExecutionType::Cycles(cycles) => {
                    executable.periodic_op(op_code)?;
                    cycle_counts[idx] += 1;
                    if cycle_counts[idx] == cycles {
                        removal_flags[idx] = true;
                    }
                }
            }
        }
        let mut removal_iter = removal_flags.iter();
        executable_vec.retain(|_| !*removal_iter.next().unwrap());
        removal_iter = removal_flags.iter();
        cycle_counts.retain(|_| !*removal_iter.next().unwrap());
        removal_flags.retain(|&i| !i);
        if executable_vec.is_empty() {
            return Ok(OpResult::Ok);
        }
        let freq = task_freq.unwrap_or_else(|| panic!("No task frequency specified"));
        thread::sleep(freq);
    })
}
