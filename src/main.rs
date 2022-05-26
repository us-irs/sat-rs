use bus::{Bus, BusReader};
use crossbeam_channel::{unbounded, Receiver, Sender};
use launchpad::core::executable::{executable_scheduler, Executable, ExecutionType, OpResult};
use std::error::Error;
use std::fmt;
use std::mem::transmute;
use std::thread;
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

    fn exec_type(&self) -> ExecutionType {
        ExecutionType::OneShot
    }

    fn task_name(&self) -> &'static str {
        "One Shot Task"
    }

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

    fn exec_type(&self) -> ExecutionType {
        ExecutionType::Cycles(3)
    }

    fn task_name(&self) -> &'static str {
        "Fixed Cycles Task"
    }

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

    fn exec_type(&self) -> ExecutionType {
        ExecutionType::Infinite
    }

    fn task_name(&self) -> &'static str {
        "Periodic Task"
    }

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

fn test0(term_bus: &mut Bus<()>) {
    let exec_task = OneShotTask {};
    let task_vec = vec![Box::new(exec_task)];
    let jhandle = executable_scheduler(
        task_vec,
        Some(Duration::from_millis(100)),
        0,
        term_bus.add_rx(),
    );
    let exec_task2 = FixedCyclesTask {};
    let task_vec2: Vec<Box<dyn Executable<Error = ExampleError> + Send>> =
        vec![Box::new(exec_task2)];
    let jhandle2 = executable_scheduler(
        task_vec2,
        Some(Duration::from_millis(100)),
        1,
        term_bus.add_rx(),
    );

    jhandle
        .join()
        .expect("Joining thread failed")
        .expect("Task failed");
    jhandle2
        .join()
        .expect("Joining thread 2 failed")
        .expect("Task 2 failed");
}

fn test1(term_bus: &mut Bus<()>) {
    let one_shot_in_vec = OneShotTask {};
    let cycles_in_vec = FixedCyclesTask {};
    let periodic_in_vec = PeriodicTask {};
    let test_vec: Vec<Box<dyn Executable<Error = ExampleError>>> = vec![
        Box::new(one_shot_in_vec),
        Box::new(cycles_in_vec),
        Box::new(periodic_in_vec),
    ];
    let jhandle3 = executable_scheduler(
        test_vec,
        Some(Duration::from_millis(100)),
        3,
        term_bus.add_rx(),
    );
    thread::sleep(Duration::from_millis(5000));
    println!("Broadcasting cancel");
    term_bus.broadcast(());
    jhandle3
        .join()
        .expect("Joining thread 3 failed")
        .expect("Task 3 failed");
}

fn main() {
    let mut tx = Bus::new(5);
    test0(&mut tx);
    test1(&mut tx);
}
