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
        if inner.count <= 0 {
            if let Some(task) = inner.wait_queue.pop_front() {
                // 当信号量释放时，从持有人列表中移除当前任务
                if let Some(current_task) = current_task() {
                    inner.holders.retain(|holder| holder.get_tid() != current_task.get_tid());
                }
                wakeup_task(task);
            }
        }
    }

    /// down operation of semaphore
    /// acquire the semaphore
    pub fn down(&self) {
        trace!("kernel: Semaphore::down");
        let mut inner = self.inner.exclusive_access();
        inner.count -= 1;
        if inner.count < 0 {
            inner.wait_queue.push_back(current_task().unwrap());
            drop(inner);
            block_current_and_run_next();
        } else {
            inner.holders.push_back(current_task().unwrap()); // 添加持有人

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
        let inner = self.inner.exclusive_access();
        inner.holders.clone()
    }

    /// accessor of all resource count
    pub fn all_resource_count(&self) -> usize {
        trace!("kernel: Semaphore::all_resource_count");
        let inner = self.inner.exclusive_access();
        if self.remaining() > 0 {
            //if count>0, then the resource is not exhausted
            self.remaining() as usize + inner.holders.len()
        } else {
            // count<0, then the resource is exhausted
            inner.holders.len()
        }
    }


    /// accessor of waiting tasks
    pub fn waiting_tasks(&self) -> VecDeque<Arc<TaskControlBlock>> {
        trace!("kernel: Semaphore::waitting_tasks");
        let inner = self.inner.exclusive_access();
        inner.wait_queue.clone()
    }
}