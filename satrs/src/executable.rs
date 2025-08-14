//! Task scheduling module
use alloc::string::String;
use bus::BusReader;
use std::boxed::Box;
use std::sync::mpsc::TryRecvError;
use std::thread::JoinHandle;
use std::time::Duration;
use std::vec;
use std::vec::Vec;
use std::{io, thread};

#[derive(Debug, PartialEq, Eq)]
pub enum OpResult {
    Ok,
    TerminationRequested,
}

#[derive(Debug)]
pub enum ExecutionType {
    Infinite,
    Cycles(u32),
    OneShot,
}

pub trait Executable {
    type Error;

    fn task_name(&self) -> &'static str;
    fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, Self::Error>;
}

pub trait ExecutableWithType: Executable {
    fn exec_type(&self) -> ExecutionType;
}

/// This function allows executing one task which implements the [Executable] trait
///
/// # Arguments
///
/// * `executable`: Executable task
/// * `task_freq`: Optional frequency of task. Required for periodic and fixed cycle tasks.
///   If [None] is passed, no sleeping will be performed.
/// * `op_code`: Operation code which is passed to the executable task
///   [operation call][Executable::periodic_op]
/// * `termination`: Optional termination handler which can cancel threads with a broadcast
pub fn exec_sched_single<
    T: ExecutableWithType<Error = E> + Send + 'static + ?Sized,
    E: Send + 'static,
>(
    mut executable: Box<T>,
    task_freq: Option<Duration>,
    op_code: i32,
    mut termination: Option<BusReader<()>>,
) -> Result<JoinHandle<Result<OpResult, E>>, io::Error> {
    let mut cycle_count = 0;
    thread::Builder::new()
        .name(String::from(executable.task_name()))
        .spawn(move || {
            loop {
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
                if let Some(freq) = task_freq {
                    thread::sleep(freq);
                }
            }
        })
}

/// This function allows executing multiple tasks as long as the tasks implement the
/// [Executable] trait
///
/// # Arguments
///
/// * `executable_vec`: Vector of executable objects
/// * `task_freq`: Optional frequency of task. Required for periodic and fixed cycle tasks
/// * `op_code`: Operation code which is passed to the executable task [operation call][Executable::periodic_op]
/// * `termination`: Optional termination handler which can cancel threads with a broadcast
pub fn exec_sched_multi<
    T: ExecutableWithType<Error = E> + Send + 'static + ?Sized,
    E: Send + 'static,
>(
    task_name: &'static str,
    mut executable_vec: Vec<Box<T>>,
    task_freq: Option<Duration>,
    op_code: i32,
    mut termination: Option<BusReader<()>>,
) -> Result<JoinHandle<Result<OpResult, E>>, io::Error> {
    let mut cycle_counts = vec![0; executable_vec.len()];
    let mut removal_flags = vec![false; executable_vec.len()];

    thread::Builder::new()
        .name(String::from(task_name))
        .spawn(move || {
            loop {
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
            }
        })
}

#[cfg(test)]
mod tests {
    use super::{
        Executable, ExecutableWithType, ExecutionType, OpResult, exec_sched_multi,
        exec_sched_single,
    };
    use bus::Bus;
    use std::boxed::Box;
    use std::error::Error;
    use std::string::{String, ToString};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use std::vec::Vec;
    use std::{fmt, thread, vec};

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

    #[derive(Clone, Debug)]
    struct ExampleError {
        kind: ErrorKind,
    }

    /// The kind of an error that can occur.
    #[derive(Clone, Debug)]
    pub enum ErrorKind {
        Generic(String, i32),
    }

    impl ExampleError {
        fn new(msg: &str, code: i32) -> ExampleError {
            ExampleError {
                kind: ErrorKind::Generic(msg.to_string(), code),
            }
        }

        /// Return the kind of this error.
        pub fn kind(&self) -> &ErrorKind {
            &self.kind
        }
    }

    impl fmt::Display for ExampleError {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            match self.kind() {
                ErrorKind::Generic(str, code) => {
                    write!(f, "{str} with code {code}")
                }
            }
        }
    }

    impl Error for ExampleError {}

    const ONE_SHOT_TASK_NAME: &str = "One Shot Task";

    impl Executable for OneShotTask {
        type Error = ExampleError;

        fn task_name(&self) -> &'static str {
            ONE_SHOT_TASK_NAME
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

    impl ExecutableWithType for OneShotTask {
        fn exec_type(&self) -> ExecutionType {
            ExecutionType::OneShot
        }
    }

    const CYCLE_TASK_NAME: &str = "Fixed Cycles Task";

    impl Executable for FixedCyclesTask {
        type Error = ExampleError;

        fn task_name(&self) -> &'static str {
            CYCLE_TASK_NAME
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

    impl ExecutableWithType for FixedCyclesTask {
        fn exec_type(&self) -> ExecutionType {
            ExecutionType::Cycles(self.cycles)
        }
    }

    const PERIODIC_TASK_NAME: &str = "Periodic Task";

    impl Executable for PeriodicTask {
        type Error = ExampleError;

        fn task_name(&self) -> &'static str {
            PERIODIC_TASK_NAME
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

    impl ExecutableWithType for PeriodicTask {
        fn exec_type(&self) -> ExecutionType {
            ExecutionType::Infinite
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
        )
        .expect("thread creation failed");
        let thread_res = jhandle.join().expect("One Shot Task failed");
        assert!(thread_res.is_ok());
        assert_eq!(thread_res.unwrap(), OpResult::Ok);
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(data.exec_num, 1);
        assert_eq!(data.op_code, expected_op_code);
    }

    #[test]
    fn test_failed_one_shot() {
        let op_code_inducing_failure = -1;
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
            op_code_inducing_failure,
            None,
        )
        .expect("thread creation failed");
        let thread_res = jhandle.join().expect("One Shot Task failed");
        assert!(thread_res.is_err());
        let error = thread_res.unwrap_err();
        let err = error.kind();
        assert!(matches!(err, &ErrorKind::Generic { .. }));
        match err {
            ErrorKind::Generic(str, op_code) => {
                assert_eq!(str, &String::from("One Shot Task Failure"));
                assert_eq!(op_code, &op_code_inducing_failure);
            }
        }
        let error_display = error.to_string();
        assert_eq!(error_display, "One Shot Task Failure with code -1");
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(data.exec_num, 1);
        assert_eq!(data.op_code, op_code_inducing_failure);
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
            "multi-task-name",
            task_vec,
            Some(Duration::from_millis(100)),
            expected_op_code,
            None,
        )
        .expect("thread creation failed");
        let thread_res = jhandle.join().expect("One Shot Task failed");
        assert!(thread_res.is_ok());
        assert_eq!(thread_res.unwrap(), OpResult::Ok);
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(data.exec_num, 2);
        assert_eq!(data.op_code, expected_op_code);
    }

    #[test]
    fn test_cycles_single() {
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
        let jh = exec_sched_single(
            cycled_task,
            Some(Duration::from_millis(100)),
            expected_op_code,
            None,
        )
        .expect("thread creation failed");
        let thread_res = jh.join().expect("Cycles Task failed");
        assert!(thread_res.is_ok());
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(thread_res.unwrap(), OpResult::Ok);
        assert_eq!(data.exec_num, 1);
        assert_eq!(data.op_code, expected_op_code);
    }

    #[test]
    fn test_single_and_cycles() {
        let expected_op_code = 50;
        let shared = Arc::new(Mutex::new(TestInfo {
            exec_num: 0,
            op_code: 0,
        }));
        let one_shot_task = Box::new(OneShotTask {
            exec_num: shared.clone(),
        });
        let cycled_task_0 = Box::new(FixedCyclesTask {
            exec_num: shared.clone(),
            cycles: 1,
        });
        let cycled_task_1 = Box::new(FixedCyclesTask {
            exec_num: shared.clone(),
            cycles: 1,
        });
        assert_eq!(cycled_task_0.task_name(), CYCLE_TASK_NAME);
        assert_eq!(one_shot_task.task_name(), ONE_SHOT_TASK_NAME);
        let task_vec: Vec<Box<dyn ExecutableWithType<Error = ExampleError> + Send>> =
            vec![one_shot_task, cycled_task_0, cycled_task_1];
        let jh = exec_sched_multi(
            "multi-task-name",
            task_vec,
            Some(Duration::from_millis(100)),
            expected_op_code,
            None,
        )
        .expect("thread creation failed");
        let thread_res = jh.join().expect("Cycles Task failed");
        assert!(thread_res.is_ok());
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(thread_res.unwrap(), OpResult::Ok);
        assert_eq!(data.exec_num, 3);
        assert_eq!(data.op_code, expected_op_code);
    }

    #[test]
    #[ignore]
    fn test_periodic_single() {
        let mut terminator = Bus::new(5);
        let expected_op_code = 45;
        let shared = Arc::new(Mutex::new(TestInfo {
            exec_num: 0,
            op_code: 0,
        }));
        let periodic_task = Box::new(PeriodicTask {
            exec_num: shared.clone(),
        });
        assert_eq!(periodic_task.task_name(), PERIODIC_TASK_NAME);
        let jh = exec_sched_single(
            periodic_task,
            Some(Duration::from_millis(20)),
            expected_op_code,
            Some(terminator.add_rx()),
        )
        .expect("thread creation failed");
        thread::sleep(Duration::from_millis(40));
        terminator.broadcast(());
        let thread_res = jh.join().expect("Periodic Task failed");
        assert!(thread_res.is_ok());
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(thread_res.unwrap(), OpResult::Ok);
        let range = 2..4;
        assert!(range.contains(&data.exec_num));
        assert_eq!(data.op_code, expected_op_code);
    }

    #[test]
    #[ignore]
    fn test_periodic_multi() {
        let mut terminator = Bus::new(5);
        let expected_op_code = 46;
        let shared = Arc::new(Mutex::new(TestInfo {
            exec_num: 0,
            op_code: 0,
        }));
        let cycled_task = Box::new(FixedCyclesTask {
            exec_num: shared.clone(),
            cycles: 1,
        });
        let periodic_task_0 = Box::new(PeriodicTask {
            exec_num: shared.clone(),
        });
        let periodic_task_1 = Box::new(PeriodicTask {
            exec_num: shared.clone(),
        });
        assert_eq!(periodic_task_0.task_name(), PERIODIC_TASK_NAME);
        assert_eq!(periodic_task_1.task_name(), PERIODIC_TASK_NAME);
        let task_vec: Vec<Box<dyn ExecutableWithType<Error = ExampleError> + Send>> =
            vec![cycled_task, periodic_task_0, periodic_task_1];
        let jh = exec_sched_multi(
            "multi-task-name",
            task_vec,
            Some(Duration::from_millis(20)),
            expected_op_code,
            Some(terminator.add_rx()),
        )
        .expect("thread creation failed");
        thread::sleep(Duration::from_millis(60));
        terminator.broadcast(());
        let thread_res = jh.join().expect("Periodic Task failed");
        assert!(thread_res.is_ok());
        let data = shared.lock().expect("Locking Mutex failed");
        assert_eq!(thread_res.unwrap(), OpResult::Ok);
        let range = 7..11;
        assert!(range.contains(&data.exec_num));
        assert_eq!(data.op_code, expected_op_code);
    }
}
