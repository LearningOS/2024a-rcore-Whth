//! Semaphore

use crate::sync::UPSafeCell;
use crate::task::{block_current_and_run_next, current_task, wakeup_task, TaskControlBlock};
use alloc::{collections::VecDeque, sync::Arc};

/// semaphore structure
pub struct Semaphore {
    /// semaphore inner
    pub inner: UPSafeCell<SemaphoreInner>,
}

pub struct SemaphoreInner {
    pub count: isize,
    pub wait_queue: VecDeque<Arc<TaskControlBlock>>,
    pub holders: VecDeque<Arc<TaskControlBlock>>, // 新增字段：记录持有该信号量的任务列表
}

impl Semaphore {
    /// Create a new semaphore
    pub fn new(res_count: usize) -> Self {
        trace!("kernel: Semaphore::new");
        Self {
            inner: unsafe {
                UPSafeCell::new(SemaphoreInner {
                    count: res_count as isize,
                    wait_queue: VecDeque::new(),
                    holders: VecDeque::new(), // 初始化持有人列表
                })
            },
        }
    }

    /// up operation of semaphore
    /// release the semaphore
    pub fn up(&self) {
        trace!("kernel: Semaphore::up");
        let mut inner = self.inner.exclusive_access();
        inner.count += 1;
        let cur_task = current_task().unwrap(); // task that will release the semaphore
        inner.holders.retain(|t| t.get_tid() != cur_task.get_tid()); // remove the current task from holder list
        if inner.count <= 0 {
            if let Some(waiting_task) = inner.wait_queue.pop_front() {
                // add the task to holder list
                inner.holders.push_back(waiting_task.clone());

                wakeup_task(waiting_task);
            }
        }
    }


    /// down operation of semaphore
    /// acquire the semaphore
    pub fn down(&self) {
        trace!("kernel: Semaphore::down");
        let mut inner = self.inner.exclusive_access();
        inner.count -= 1;
        let task = current_task().unwrap();
        if inner.count < 0 {
            inner.wait_queue.push_back(task);
            drop(inner);
            trace!("kernel: Semaphore::down .. block_current_and_run_next");
            block_current_and_run_next();
            trace!("kernel: Semaphore::down .. waked up task")
        } else {
            inner.holders.push_back(task);
        }
    }
    /// accessor of count
    pub fn remaining(&self) -> isize {
        trace!("kernel: Semaphore::count");
        self.inner.exclusive_access().count
    }

    /// if resource is exhausted, check using `count()`
    pub fn is_exhausted(&self) -> bool {
        trace!("kernel: Semaphore::is_exhausted");
        self.remaining() == 0
    }

    /// accessor of holders
    pub fn holders(&self) -> VecDeque<Arc<TaskControlBlock>> {
        trace!("kernel: Semaphore::holders");
        self.inner.exclusive_access().holders.clone()
    }

    /// accessor of all resource count
    pub fn all_resource_count(&self) -> usize {
        trace!("kernel: Semaphore::all_resource_count");
        let inner = self.inner.exclusive_access().holders.len();
        if self.remaining() > 0 {
            //if count>0, then the resource is not exhausted
            self.remaining() as usize + inner
        } else {
            // count<0, then the resource is exhausted
            inner
        }
    }


    /// accessor of waiting tasks
    pub fn waiting_tasks(&self) -> VecDeque<Arc<TaskControlBlock>> {
        trace!("kernel: Semaphore::waitting_tasks");
        let inner = self.inner.exclusive_access();
        inner.wait_queue.clone()
    }

    /// remove the thread from the semaphore entirely,both from the holder list and the waiting list
    pub fn unregister_thread(&self, thread: Arc<TaskControlBlock>) {
        trace!("kernel: Semaphore::unregister_thread");
        println!(" unregister start");

        let mut inner = self.inner.exclusive_access();
        println!("wait_queue unregister start");

        inner.wait_queue.retain(|t| Arc::as_ptr(t) != Arc::as_ptr(&thread));
        println!("holders unregister start");

        inner.holders.retain(|t| Arc::as_ptr(t) != Arc::as_ptr(&thread));
    }
}