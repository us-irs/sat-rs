use core::{convert::Infallible, fmt::Debug, time::Duration};
use std::time::Instant;

use thiserror::Error;

use crate::executable::Executable;

#[cfg(feature = "std")]
pub use std_mod::*;

#[derive(Debug, Default)]
pub struct SchedulingTable {
    execution_frequency: Duration,
    pub table: alloc::vec::Vec<u32>,
}

#[derive(Debug, Error)]
pub enum InvalidSlotError {
    #[error("slot time is larger than the execution frequency")]
    SlotTimeLargerThanFrequency,
    #[error("slot time is smaller than previous slot")]
    SmallerThanPreviousSlot {
        slot_time_ms: u32,
        prev_slot_time_ms: u32,
    },
}

impl SchedulingTable {
    pub fn new(execution_frequency: Duration) -> Self {
        Self {
            execution_frequency,
            table: Default::default(),
        }
    }

    pub fn add_slot(&mut self, relative_execution_time_ms: u32) -> Result<(), InvalidSlotError> {
        if relative_execution_time_ms > self.execution_frequency.as_millis() as u32 {
            return Err(InvalidSlotError::SlotTimeLargerThanFrequency);
        }

        if !self.table.is_empty() {
            let prev_slot_ms = *self.table.last().unwrap();
            if relative_execution_time_ms < prev_slot_ms {
                return Err(InvalidSlotError::SmallerThanPreviousSlot {
                    slot_time_ms: relative_execution_time_ms,
                    prev_slot_time_ms: *self.table.last().unwrap(),
                });
            }
        }
        self.table.push(relative_execution_time_ms);
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum TaskWithSchedulingTableError {
    #[error("scheudlig table error: {0}")]
    InvalidSlot(#[from] InvalidSlotError),
    #[error("task lock error")]
    LockError,
    #[error("task borrow error")]
    BorrowError,
}

pub trait DeadlineMissedHandler {
    fn deadline_missed_callback(&mut self, task_name: &'static str, op_code: i32);
}

pub trait TaskExecutor {
    fn with_task<F: FnOnce(&mut dyn Executable<Error = Infallible>)>(&self, f: F);
}

#[cfg(feature = "std")]
pub mod std_mod {
    use core::cell::RefCell;
    use std::{
        rc::Rc,
        sync::{Arc, Mutex},
        vec::Vec,
    };

    use super::*;

    impl TaskExecutor for Arc<Mutex<dyn Executable<Error = Infallible> + Send>> {
        fn with_task<F: FnOnce(&mut dyn Executable<Error = Infallible>)>(&self, f: F) {
            let mut task = self.lock().unwrap();
            f(&mut *task);
        }
    }

    impl TaskExecutor for Rc<RefCell<dyn Executable<Error = Infallible>>> {
        fn with_task<F: FnOnce(&mut dyn Executable<Error = Infallible>)>(&self, f: F) {
            let mut task = self.borrow_mut();
            f(&mut *task);
        }
    }

    pub struct TaskWithOpCode<T: TaskExecutor> {
        task: T,
        op_code: i32,
    }

    pub struct TaskWithSchedulingTable<T: TaskExecutor> {
        start_of_slot: Instant,
        end_of_slot: Instant,
        deadline_missed_ms_count: u32,
        table: SchedulingTable,
        tasks: Vec<TaskWithOpCode<T>>,
    }

    impl TaskWithSchedulingTable<Rc<RefCell<dyn Executable<Error = Infallible>>>> {
        /// Add a new task to the scheduling table
        ///
        /// The task needs to be wrapped inside [Rc] and [RefCell]. The task is not sendable and
        /// needs to be created inside the target thread.
        pub fn add_task(
            &mut self,
            relative_execution_time_ms: u32,
            task: Rc<RefCell<dyn Executable<Error = Infallible>>>,
            op_code: i32,
        ) -> Result<(), TaskWithSchedulingTableError> {
            self.table.add_slot(relative_execution_time_ms)?;
            self.tasks.push(TaskWithOpCode { task, op_code });
            Ok(())
        }
    }

    impl TaskWithSchedulingTable<Arc<Mutex<dyn Executable<Error = Infallible> + Send>>> {
        /// Add a new task to the scheduling table
        ///
        /// The task needs to be wrapped inside [Arc] and [Mutex], but the task can be sent to
        /// a different thread.
        pub fn add_task_sendable(
            &mut self,
            relative_execution_time_ms: u32,
            task: Arc<Mutex<dyn Executable<Error = Infallible> + Send>>,
            op_code: i32,
        ) -> Result<(), TaskWithSchedulingTableError> {
            self.table.add_slot(relative_execution_time_ms)?;
            self.tasks.push(TaskWithOpCode { task, op_code });
            Ok(())
        }
    }

    impl<T: TaskExecutor> TaskWithSchedulingTable<T> {
        pub fn new(execution_frequency: Duration) -> Self {
            Self {
                start_of_slot: Instant::now(),
                end_of_slot: Instant::now(),
                deadline_missed_ms_count: 10,
                table: SchedulingTable::new(execution_frequency),
                tasks: Default::default(),
            }
        }

        /// Can be used to set the start of the slot to the current time. This is useful if a custom
        /// runner implementation is used instead of the [Self::start] method.
        pub fn init_start_of_slot(&mut self) {
            self.start_of_slot = Instant::now();
        }

        pub fn run_one_task_cycle(
            &mut self,
            deadline_missed_cb: &mut impl DeadlineMissedHandler,
        ) -> Result<(), TaskWithSchedulingTableError> {
            self.end_of_slot = self.start_of_slot + self.table.execution_frequency;

            for (&relative_execution_time_ms, task_with_op_code) in
                self.table.table.iter().zip(self.tasks.iter_mut())
            {
                let scheduled_execution_time = self.start_of_slot
                    + core::time::Duration::from_millis(relative_execution_time_ms as u64);
                let now = Instant::now();

                if now < scheduled_execution_time {
                    std::thread::sleep(scheduled_execution_time - now);
                } else if (now - scheduled_execution_time).as_millis()
                    > self.deadline_missed_ms_count.into()
                {
                    task_with_op_code.task.with_task(|task| {
                        deadline_missed_cb
                            .deadline_missed_callback(task.task_name(), task_with_op_code.op_code);
                    });
                }

                task_with_op_code.task.with_task(|task| {
                    // Unwrapping is okay here because we constrain the tasks to be infallible.
                    task.periodic_op(task_with_op_code.op_code).unwrap();
                });
            }

            let now = Instant::now();
            if now <= self.end_of_slot {
                let diff = self.end_of_slot - now;
                std::thread::sleep(diff);
                self.start_of_slot = self.end_of_slot;
            } else if now > self.end_of_slot + self.table.execution_frequency {
                // We're getting strongly out of sync. Set the new start timt to now.
                self.start_of_slot = now;
            }

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use core::{cell::RefCell, convert::Infallible, time::Duration};
    use std::{
        println,
        rc::Rc,
        sync::{
            mpsc::{self, TryRecvError},
            Arc, Mutex,
        },
        time::Instant,
    };

    use crate::executable::{Executable, OpResult};

    use super::{DeadlineMissedHandler, TaskWithSchedulingTable};

    #[derive(Debug)]
    pub struct CallInfo {
        time: std::time::Instant,
        op_code: i32,
    }

    pub struct Task1 {
        called_queue: mpsc::Sender<CallInfo>,
    }

    impl Executable for Task1 {
        type Error = Infallible;
        fn task_name(&self) -> &'static str {
            "Task1"
        }
        fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, Self::Error> {
            self.called_queue
                .send(CallInfo {
                    time: Instant::now(),
                    op_code,
                })
                .unwrap();
            Ok(OpResult::Ok)
        }
    }

    pub struct Task2 {
        called_queue: mpsc::Sender<CallInfo>,
    }

    impl Executable for Task2 {
        type Error = Infallible;
        fn task_name(&self) -> &'static str {
            "Task2"
        }
        fn periodic_op(&mut self, op_code: i32) -> Result<OpResult, Self::Error> {
            self.called_queue
                .send(CallInfo {
                    time: Instant::now(),
                    op_code,
                })
                .unwrap();
            Ok(OpResult::Ok)
        }
    }

    #[derive(Default)]
    pub struct DeadlineMissed {
        call_count: u32,
    }

    impl DeadlineMissedHandler for DeadlineMissed {
        fn deadline_missed_callback(&mut self, task_name: &'static str, _op_code: i32) {
            println!("task name {task_name} missed the deadline");
            self.call_count += 1;
        }
    }

    #[test]
    pub fn basic_test() {
        let (tx_t1, rx_t1) = mpsc::channel();
        let (tx_t2, rx_t2) = mpsc::channel();
        let t1 = Task1 {
            called_queue: tx_t1,
        };
        let t2 = Task2 {
            called_queue: tx_t2,
        };
        let mut deadline_missed_cb = DeadlineMissed::default();
        let mut exec_task = TaskWithSchedulingTable::new(Duration::from_millis(200));
        let t1_first_slot = Rc::new(RefCell::new(t1));
        let t1_second_slot = t1_first_slot.clone();
        let t2_first_slot = Rc::new(RefCell::new(t2));
        let t2_second_slot = t2_first_slot.clone();

        exec_task.add_task(0, t1_first_slot, 0).unwrap();
        exec_task.add_task(50, t1_second_slot, -1).unwrap();
        exec_task.add_task(100, t2_first_slot, 1).unwrap();
        exec_task.add_task(150, t2_second_slot, 2).unwrap();
        let now = Instant::now();
        exec_task.init_start_of_slot();
        exec_task
            .run_one_task_cycle(&mut deadline_missed_cb)
            .unwrap();
        let mut call_info = rx_t1.try_recv().unwrap();
        assert_eq!(call_info.op_code, 0);
        let diff_call_to_start = call_info.time - now;
        assert!(diff_call_to_start.as_millis() < 30);
        call_info = rx_t1.try_recv().unwrap();
        assert_eq!(call_info.op_code, -1);
        let diff_call_to_start = call_info.time - now;
        assert!(diff_call_to_start.as_millis() < 80);
        assert!(diff_call_to_start.as_millis() >= 50);
        matches!(rx_t1.try_recv().unwrap_err(), TryRecvError::Empty);

        call_info = rx_t2.try_recv().unwrap();
        assert_eq!(call_info.op_code, 1);
        let diff_call_to_start = call_info.time - now;
        assert!(diff_call_to_start.as_millis() < 120);
        assert!(diff_call_to_start.as_millis() >= 100);
        call_info = rx_t2.try_recv().unwrap();
        assert_eq!(call_info.op_code, 2);
        let diff_call_to_start = call_info.time - now;
        assert!(diff_call_to_start.as_millis() < 180);
        assert!(diff_call_to_start.as_millis() >= 150);
        matches!(rx_t2.try_recv().unwrap_err(), TryRecvError::Empty);
        assert_eq!(deadline_missed_cb.call_count, 0);
    }

    #[test]
    pub fn basic_test_with_arc_mutex() {
        let (tx_t1, rx_t1) = mpsc::channel();
        let (tx_t2, rx_t2) = mpsc::channel();
        let t1 = Task1 {
            called_queue: tx_t1,
        };
        let t2 = Task2 {
            called_queue: tx_t2,
        };
        let mut deadline_missed_cb = DeadlineMissed::default();
        let mut exec_task = TaskWithSchedulingTable::new(Duration::from_millis(200));
        let t1_first_slot = Arc::new(Mutex::new(t1));
        let t1_second_slot = t1_first_slot.clone();
        let t2_first_slot = Arc::new(Mutex::new(t2));
        let t2_second_slot = t2_first_slot.clone();

        exec_task.add_task_sendable(0, t1_first_slot, 0).unwrap();
        exec_task.add_task_sendable(50, t1_second_slot, -1).unwrap();
        exec_task.add_task_sendable(100, t2_first_slot, 1).unwrap();
        exec_task.add_task_sendable(150, t2_second_slot, 2).unwrap();
        let now = Instant::now();
        exec_task.init_start_of_slot();
        exec_task
            .run_one_task_cycle(&mut deadline_missed_cb)
            .unwrap();
        let mut call_info = rx_t1.try_recv().unwrap();
        assert_eq!(call_info.op_code, 0);
        let diff_call_to_start = call_info.time - now;
        assert!(diff_call_to_start.as_millis() < 30);
        call_info = rx_t1.try_recv().unwrap();
        assert_eq!(call_info.op_code, -1);
        let diff_call_to_start = call_info.time - now;
        assert!(diff_call_to_start.as_millis() < 80);
        assert!(diff_call_to_start.as_millis() >= 50);
        matches!(rx_t1.try_recv().unwrap_err(), TryRecvError::Empty);

        call_info = rx_t2.try_recv().unwrap();
        assert_eq!(call_info.op_code, 1);
        let diff_call_to_start = call_info.time - now;
        assert!(diff_call_to_start.as_millis() < 120);
        assert!(diff_call_to_start.as_millis() >= 100);
        call_info = rx_t2.try_recv().unwrap();
        assert_eq!(call_info.op_code, 2);
        let diff_call_to_start = call_info.time - now;
        assert!(diff_call_to_start.as_millis() < 180);
        assert!(diff_call_to_start.as_millis() >= 150);
        matches!(rx_t2.try_recv().unwrap_err(), TryRecvError::Empty);
        assert_eq!(deadline_missed_cb.call_count, 0);
    }

    #[test]
    pub fn basic_test_in_thread() {
        let mut deadline_missed_cb = DeadlineMissed::default();
        std::thread::spawn(move || {
            let (tx_t1, _rx_t1) = mpsc::channel();
            let t1 = Task1 {
                called_queue: tx_t1,
            };
            // Need to construct this in the thread, the task table in not [Send]
            let mut exec_task = TaskWithSchedulingTable::new(Duration::from_millis(200));
            let t1_wrapper = Rc::new(RefCell::new(t1));
            exec_task.add_task(0, t1_wrapper, 0).unwrap();
            exec_task
                .run_one_task_cycle(&mut deadline_missed_cb)
                .unwrap();
        });

        let mut deadline_missed_cb = DeadlineMissed::default();
        let (tx_t1, _rx_t1) = mpsc::channel();
        let t1 = Task1 {
            called_queue: tx_t1,
        };
        let mut exec_task_sendable = TaskWithSchedulingTable::new(Duration::from_millis(200));
        exec_task_sendable
            .add_task_sendable(0, Arc::new(Mutex::new(t1)), 1)
            .unwrap();
        std::thread::spawn(move || {
            exec_task_sendable
                .run_one_task_cycle(&mut deadline_missed_cb)
                .unwrap();
        });
    }
}
