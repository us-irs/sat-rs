use std::error::Error;
use std::fmt;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

enum OpResult {
    Ok,
}

enum ExecutionType {
    Infinite,
    Cycles(u32),
    OneShot,
}

trait ExecutableWithAssocConst {
    type Error;
    const EXEC_TYPE: ExecutionType;
    const TASK_FREQ: Option<Duration>;
    const TASK_NAME: &'static str;
    fn periodic_op(&mut self) -> Result<OpResult, Self::Error>;
}

trait Executable {
    type Error;
    fn exec_type(&self) -> ExecutionType;
    fn task_freq(&self) -> Option<Duration>;
    fn task_name(&self) -> &'static str;
    fn periodic_op(&mut self) -> Result<OpResult, Self::Error>;
}

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

impl ExecutableWithAssocConst for ExampleTask {
    type Error = ExampleError;
    const EXEC_TYPE: ExecutionType = ExecutionType::OneShot;
    const TASK_FREQ: Option<Duration> = Some(Duration::from_millis(500));
    const TASK_NAME: &'static str = "Test Task";

    fn periodic_op(&mut self) -> Result<OpResult, ExampleError> {
        println!("Periodic Operation!");
        Ok(OpResult::Ok)
    }
}

fn main() {
    let mut exec_task = ExampleTask {};
    let jhandle = thread::spawn(move || match ExampleTask::EXEC_TYPE {
        ExecutionType::OneShot => exec_task.periodic_op(),
        ExecutionType::Infinite => loop {
            exec_task.periodic_op().expect("Periodic Op failed");
            let freq = ExampleTask::TASK_FREQ.unwrap_or_else(|| {
                panic!(
                    "No task frequency specified for task {}",
                    ExampleTask::TASK_NAME
                )
            });
            thread::sleep(freq)
        },
        ExecutionType::Cycles(cycles) => {
            for _i in 0..cycles - 1 {
                exec_task.periodic_op().unwrap();
                let freq = ExampleTask::TASK_FREQ.unwrap_or_else(|| {
                    panic!(
                        "No task frequency specified for task {}",
                        ExampleTask::TASK_NAME
                    )
                });
                thread::sleep(freq)
            }
            Ok(OpResult::Ok)
        }
    });
    let mut exec_task2 = ExampleTask {};
    let jhandle2 = test_thread(exec_task2);
    jhandle
        .join()
        .expect("Joining thread failed")
        .expect("Task failed");
    jhandle2
        .join()
        .expect("Joining thread 2 failed")
        .expect("Task 2 failed");
}

fn test_thread<
    T: ExecutableWithAssocConst<Error = E> + Send + 'static,
    E: Error + Send + 'static,
>(
    mut executable: T,
) -> JoinHandle<Result<OpResult, E>> {
    // let executable = Arc::new(executable_unprotected);
    thread::spawn(move || match T::EXEC_TYPE {
        ExecutionType::OneShot => executable.periodic_op(),
        ExecutionType::Infinite => loop {
            executable.periodic_op().expect("Periodic Op failed");
            let freq = T::TASK_FREQ
                .unwrap_or_else(|| panic!("No task frequency specified for task {}", T::TASK_NAME));
            thread::sleep(freq)
        },
        ExecutionType::Cycles(cycles) => {
            for _i in 0..cycles - 1 {
                executable.periodic_op().unwrap();
                let freq = T::TASK_FREQ.unwrap_or_else(|| {
                    panic!("No task frequency specified for task {}", T::TASK_NAME)
                });
                thread::sleep(freq)
            }
            Ok(OpResult::Ok)
        }
    })
}
