use launchpad::core::executable::{Executable, ExecutionType, OpResult};
use std::error::Error;
use std::fmt;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

struct ExampleTask {}

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

impl Executable for ExampleTask {
    type Error = ExampleError;
    const EXEC_TYPE: ExecutionType = ExecutionType::OneShot;
    const TASK_FREQ: Option<Duration> = Some(Duration::from_millis(500));
    const TASK_NAME: &'static str = "Test Task";

    fn periodic_op(&mut self, op_code: u32) -> Result<OpResult, ExampleError> {
        if op_code == 0 {
            println!("Periodic Operation OK!");
            Ok(OpResult::Ok)
        } else {
            println!("Periodic Operation Fail!");
            Err(ExampleError::new("Example Task Failure"))
        }
    }
}

fn main() {
    let exec_task = ExampleTask {};
    let jhandle = test_thread(exec_task, 0);
    let exec_task2 = ExampleTask {};
    let jhandle2 = test_thread(exec_task2, 1);
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
    mut executable: T,
    op_code: u32,
) -> JoinHandle<Result<OpResult, E>> {
    thread::spawn(move || match T::EXEC_TYPE {
        ExecutionType::OneShot => executable.periodic_op(op_code),
        ExecutionType::Infinite => loop {
            executable.periodic_op(op_code)?;
            let freq = T::TASK_FREQ
                .unwrap_or_else(|| panic!("No task frequency specified for task {}", T::TASK_NAME));
            thread::sleep(freq)
        },
        ExecutionType::Cycles(cycles) => {
            for _i in 0..cycles - 1 {
                executable.periodic_op(op_code)?;
                let freq = T::TASK_FREQ.unwrap_or_else(|| {
                    panic!("No task frequency specified for task {}", T::TASK_NAME)
                });
                thread::sleep(freq)
            }
            Ok(OpResult::Ok)
        }
    })
}
