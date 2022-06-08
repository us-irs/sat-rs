use bus::BusReader;
use std::error::Error;
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum OpResult {
    Ok,
    TerminationRequested,
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

/// This function allows executing one task which implements the [Executable][Executable] trait
/// # Arguments
///
/// * `executable`: Executable task
/// * `task_freq`: Optional frequency of task. Required for periodic and fixed cycle tasks
/// * `op_code`: Operation code which is passed to the executable task [operation call][Executable::periodic_op]
/// * `termination`: Optional termination handler which can cancel threads with a broadcast
pub fn exec_sched_single<
    T: Executable<Error = E> + Send + 'static + ?Sized,
    E: Error + Send + 'static,
>(
    mut executable: Box<T>,
    task_freq: Option<Duration>,
    op_code: i32,
    mut termination: Option<BusReader<()>>,
) -> JoinHandle<Result<OpResult, E>> {
    let mut cycle_count = 0;
    thread::spawn(move || loop {
        if let Some(ref mut terminator) = termination {
            match terminator.try_recv() {
                Ok(_) | Err(TryRecvError::Disconnected) => {
                    return Ok(OpResult::Ok);
                }
                Err(TryRecvError::Empty) => (),
            }
        }
        match executable.exec_type() {
            ExecutionType::OneShot => {
                executable.periodic_op(op_code)?;
                return Ok(OpResult::Ok);
            }
            ExecutionType::Infinite => {
                executable.periodic_op(op_code)?;
            }
            ExecutionType::Cycles(cycles) => {
                executable.periodic_op(op_code)?;
                cycle_count += 1;
                if cycle_count == cycles {
                    return Ok(OpResult::Ok);
                }
            }
        }
        let freq = task_freq.unwrap_or_else(|| panic!("No task frequency specified"));
        thread::sleep(freq);
    })
}

/// This function allows executing multiple tasks as long as the tasks implement the
/// [Executable][Executable] trait
/// # Arguments
///
/// * `executable_vec`: Vector of executable objects
/// * `task_freq`: Optional frequency of task. Required for periodic and fixed cycle tasks
/// * `op_code`: Operation code which is passed to the executable task periodic_op call
/// * `termination`: Optional termination handler which can cancel threads with a broadcast
pub fn exec_sched_multi<
    T: Executable<Error = E> + Send + 'static + ?Sized,
    E: Error + Send + 'static,
>(
    mut executable_vec: Vec<Box<T>>,
    task_freq: Option<Duration>,
    op_code: i32,
    mut termination: Option<BusReader<()>>,
) -> JoinHandle<Result<OpResult, E>> {
    let mut cycle_counts = vec![0; executable_vec.len()];
    let mut removal_flags = vec![false; executable_vec.len()];
    thread::spawn(move || loop {
        if let Some(ref mut terminator) = termination {
            match terminator.try_recv() {
                Ok(_) | Err(TryRecvError::Disconnected) => {
                    removal_flags.iter_mut().for_each(|x| *x = true);
                }
                Err(TryRecvError::Empty) => (),
            }
        }
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

#[cfg(test)]
mod tests {
    use super::{exec_sched_multi, exec_sched_single, Executable, ExecutionType, OpResult};
    use std::error::Error;
    use std::fmt;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct TestInfo {
        exec_num: u32,
        op_code: i32,
    }
    struct OneShotTask {
        exec_num: Arc<Mutex<TestInfo>>,
    }
    struct FixedCyclesTask {
        cycles: u32,
        exec_num: Arc<Mutex<TestInfo>>,
    }
    struct PeriodicTask {
        exec_num: Arc<Mutex<TestInfo>>,
    }

    #[derive(Debug, PartialEq)]
    struct ExampleError {
        details: String,
        code: i32,
    }

    impl ExampleError {
        fn new(msg: &str, code: i32) -> ExampleError {
            ExampleError {
                details: msg.to_string(),
                code,
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

    const ONE_SHOT_TASK_NAME: &'static str = "One Shot Task";

    impl Executable for OneShotTask {
        type Error = ExampleError;

        fn exec_type(&self) -> ExecutionType {
            ExecutionType::OneShot
        }

        fn task_name(&self) -> &'static str {
            return ONE_SHOT_TASK_NAME;
        }

        fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, ExampleError> {
            let mut data = self.exec_num.lock().expect("Locking Mutex failed");
            data.exec_num += 1;
            data.op_code = op_code;
            std::mem::drop(data);
            if op_code >= 0 {
                Ok(OpResult::Ok)
            } else {
                Err(ExampleError::new("One Shot Task Failure", op_code))
            }
        }
    }

    const CYCLE_TASK_NAME: &'static str = "Fixed Cycles Task";

    impl Executable for FixedCyclesTask {
        type Error = ExampleError;

        fn exec_type(&self) -> ExecutionType {
            ExecutionType::Cycles(self.cycles)
        }

        fn task_name(&self) -> &'static str {
            return CYCLE_TASK_NAME
        }

        fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, ExampleError> {
            let mut data = self.exec_num.lock().expect("Locking Mutex failed");
            data.exec_num += 1;
            data.op_code = op_code;
            std::mem::drop(data);
            if op_code >= 0 {
                Ok(OpResult::Ok)
            } else {
                Err(ExampleError::new("Fixed Cycle Task Failure", op_code))
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
            let mut data = self.exec_num.lock().expect("Locking Mutex failed");
            data.exec_num += 1;
            data.op_code = op_code;
            std::mem::drop(data);
            if op_code >= 0 {
                Ok(OpResult::Ok)
            } else {
                Err(ExampleError::new("Example Task Failure", op_code))
            }
        }
    }

    #[test]
    fn test_simple_one_shot() {
        let expected_op_code = 42;
        let shared = Arc::new(Mutex::new(TestInfo {
            exec_num: 0,
            op_code: 0,
        }));
        let exec_task = OneShotTask {
            exec_num: shared.clone(),
        };
        let task = Box::new(exec_task);
        let jhandle = exec_sched_single(
            task,
            Some(Duration::from_millis(100)),
            expected_op_code,
            None,
        );
        let thread_res = jhandle.join().expect("One Shot Task failed");
        assert!(thread_res.is_ok());
        assert_eq!(thread_res.unwrap(), OpResult::Ok);
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(data.exec_num, 1);
        assert_eq!(data.op_code, expected_op_code);
    }

    #[test]
    fn test_simple_multi_one_shot() {
        let expected_op_code = 43;
        let shared = Arc::new(Mutex::new(TestInfo {
            exec_num: 0,
            op_code: 0,
        }));
        let exec_task_0 = OneShotTask {
            exec_num: shared.clone(),
        };
        let exec_task_1 = OneShotTask {
            exec_num: shared.clone(),
        };
        let task_vec = vec![Box::new(exec_task_0), Box::new(exec_task_1)];
        for task in task_vec.iter() {
            assert_eq!(task.task_name(), ONE_SHOT_TASK_NAME);
        }
        let jhandle = exec_sched_multi(
            task_vec,
            Some(Duration::from_millis(100)),
            expected_op_code,
            None,
        );
        let thread_res = jhandle.join().expect("One Shot Task failed");
        assert!(thread_res.is_ok());
        assert_eq!(thread_res.unwrap(), OpResult::Ok);
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(data.exec_num, 2);
        assert_eq!(data.op_code, expected_op_code);
    }

    #[test]
    fn test_cycles() {
        let expected_op_code = 44;
        let shared = Arc::new(Mutex::new(TestInfo {
            exec_num: 0,
            op_code: 0,
        }));
        let cycled_task = Box::new(FixedCyclesTask {
            exec_num: shared.clone(),
            cycles: 1,
        });
        assert_eq!(cycled_task.task_name(), CYCLE_TASK_NAME);
        let jh = exec_sched_single(cycled_task, Some(Duration::from_millis(100)), expected_op_code, None);
        let thread_res = jh.join().expect("Cycles Task failed");
        assert!(thread_res.is_ok());
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(thread_res.unwrap(), OpResult::Ok);
        assert_eq!(data.exec_num, 1);
        assert_eq!(data.op_code, expected_op_code);
    }

    #[test]
    #[ignore]
    fn test_periodic() {
        let expected_op_code = 45;
        let shared = Arc::new(Mutex::new(TestInfo {
            exec_num: 0,
            op_code: 0,
        }));
        let periodic_task = Box::new(PeriodicTask {
            exec_num: shared.clone()
        });
        let jh = exec_sched_single(periodic_task, Some(Duration::from_millis(50)), expected_op_code, None);
        let thread_res = jh.join().expect("Periodic Task failed");
        assert!(thread_res.is_ok());
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(thread_res.unwrap(), OpResult::Ok);
        assert_eq!(data.op_code, expected_op_code);
    }
}
