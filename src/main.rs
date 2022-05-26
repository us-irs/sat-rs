use launchpad::core::executable::{Executable, ExecutionType, OpResult};
use std::error::Error;
use std::fmt;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

struct OneShotTask {}
struct FixedCyclesTask {}
struct PeriodicTask {}

#[derive(Debug)]
struct ExampleError {
    details: String,
}

impl ExampleError {
    fn new(msg: &str) -> ExampleError {
        ExampleError {
            details: msg.to_string(),
        }
    }
}

impl fmt::Display for ExampleError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl Error for ExampleError {
    fn description(&self) -> &str {
        &self.details
    }
}

impl Executable for OneShotTask {
    type Error = ExampleError;
    const EXEC_TYPE: ExecutionType = ExecutionType::OneShot;
    const TASK_NAME: &'static str = "One Shot Test Task";

    fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, ExampleError> {
        if op_code >= 0 {
            println!("One-shot operation with operation code {op_code} OK!");
            Ok(OpResult::Ok)
        } else {
            println!("One-shot operation failure by passing op code {op_code}!");
            Err(ExampleError::new("Example Task Failure"))
        }
    }
}

impl Executable for FixedCyclesTask {
    type Error = ExampleError;
    const EXEC_TYPE: ExecutionType = ExecutionType::Cycles(5);
    const TASK_NAME: &'static str = "Fixed Cycles Task";

    fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, ExampleError> {
        if op_code >= 0 {
            println!("Fixed-cycle operation with operation code {op_code} OK!");
            Ok(OpResult::Ok)
        } else {
            println!("Fixed-cycle operation failure by passing op code {op_code}!");
            Err(ExampleError::new("Example Task Failure"))
        }
    }
}

impl Executable for PeriodicTask {
    type Error = ExampleError;
    const EXEC_TYPE: ExecutionType = ExecutionType::Cycles(5);
    const TASK_NAME: &'static str = "Fixed Cycles Task";

    fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, ExampleError> {
        if op_code >= 0 {
            println!("Periodic operation with operation code {op_code} OK!");
            Ok(OpResult::Ok)
        } else {
            println!("Periodic operation failure by passing op code {op_code}!");
            Err(ExampleError::new("Example Task Failure"))
        }
    }
}

fn main() {
    let exec_task = OneShotTask {};
    let mut task_vec = Vec::new();
    task_vec.push(exec_task);
    let jhandle = test_thread(task_vec, Some(Duration::from_millis(500)), 0);
    let exec_task2 = FixedCyclesTask {};
    let mut task_vec2 = Vec::new();
    task_vec2.push(exec_task2);
    let jhandle2 = test_thread(task_vec2, Some(Duration::from_millis(1000)), 1);
    jhandle
        .join()
        .expect("Joining thread failed")
        .expect("Task failed");
    jhandle2
        .join()
        .expect("Joining thread 2 failed")
        .expect("Task 2 failed");
}

fn test_thread<T: Executable<Error = E> + Send + 'static, E: Error + Send + 'static>(
    mut executable_vec: Vec<T>,
    task_freq: Option<Duration>,
    op_code: i32,
) -> JoinHandle<Result<OpResult, E>> {
    let mut cycle_counts = vec![0; executable_vec.len()];
    let mut removal_flags = vec![false; executable_vec.len()];
    thread::spawn(move || loop {
        for (idx, executable) in executable_vec.iter_mut().enumerate() {
            match T::EXEC_TYPE {
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
        executable_vec.retain(|_| !*removal_flags.iter().next().unwrap());
        removal_flags.retain(|&i| i == false);
        if executable_vec.is_empty() {
            return Ok(OpResult::Ok);
        }
        let freq = task_freq
            .unwrap_or_else(|| panic!("No task frequency specified for task {}", T::TASK_NAME));
        thread::sleep(freq);
    })
}
