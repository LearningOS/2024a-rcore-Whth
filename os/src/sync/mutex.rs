//! Mutex (spin-like and blocking(sleep))

use super::UPSafeCell;
use crate::task::TaskControlBlock;
use crate::task::{block_current_and_run_next, suspend_current_and_run_next};
use crate::task::{current_task, wakeup_task};
use alloc::vec::Vec;
use alloc::{collections::VecDeque, sync::Arc};

/// Mutex trait
pub trait Mutex<T>: Sync + Send + TraceProcession<T> {
    /// Lock the mutex
    fn lock(&self);
    /// Unlock the mutex
    fn unlock(&self);
}


pub trait TraceProcession<T> {
    fn trace_owner(&self) -> Option<Arc<T>>;

    fn trace_waiters(&self) -> Vec<Arc<T>>;
}
/// Spinlock Mutex struct
pub struct MutexSpin {
    inner: UPSafeCell<MutexSpinInner>,
}

struct MutexSpinInner {
    holder: Option<Arc<TaskControlBlock>>, // 记录持有锁的线程
    waiters: VecDeque<Arc<TaskControlBlock>>, // 记录等待锁的线程队列
}

impl MutexSpin {
    /// Create a new spinlock mutex
    pub fn new() -> Self {
        Self {
            inner: unsafe {
                UPSafeCell::new(MutexSpinInner {
                    holder: None,
                    waiters: VecDeque::new(),
                })
            },
        }
    }
}

impl TraceProcession<TaskControlBlock> for MutexSpin {
    fn trace_owner(&self) -> Option<Arc<TaskControlBlock>> {
        self.inner.exclusive_access().holder.clone()
    }

    fn trace_waiters(&self) -> Vec<Arc<TaskControlBlock>> {
        self.inner.exclusive_access().waiters.iter().cloned().collect()
    }
}

impl Mutex<TaskControlBlock> for MutexSpin {
    /// Lock the spinlock mutex
    fn lock(&self) {
        trace!("kernel: MutexSpin::lock");
        loop {
            let mut inner = self.inner.exclusive_access();
            if inner.holder.is_some() {
                drop(inner);
                {
                    let mut inner = self.inner.exclusive_access();
                    inner.waiters.push_back(current_task().unwrap());
                }
                suspend_current_and_run_next();
                continue;
            } else {
                inner.holder = Some(current_task().unwrap());
                return;
            }
        }
    }

    /// Unlock the spinlock mutex
    fn unlock(&self) {
        trace!("kernel: MutexSpin::unlock");
        let mut inner = self.inner.exclusive_access();
        assert!(inner.holder.is_some());
        assert_eq!(inner.holder.clone().unwrap().get_tid(), current_task().unwrap().get_tid()); // 确保解锁的是当前持有锁的线程

        inner.holder = None;

        if let Some(waking_task) = inner.waiters.pop_front() {
            wakeup_task(waking_task.clone());
            inner.holder = Some(waking_task);
        }
    }
}

/// Blocking Mutex struct
pub struct MutexBlocking {
    inner: UPSafeCell<MutexBlockingInner>,
}

pub struct MutexBlockingInner {
    holder: Option<Arc<TaskControlBlock>>, // 记录持有锁的线程
    wait_queue: VecDeque<Arc<TaskControlBlock>>,
}

impl MutexBlocking {
    /// Create a new blocking mutex
    pub fn new() -> Self {
        trace!("kernel: MutexBlocking::new");
        Self {
            inner: unsafe {
                UPSafeCell::new(MutexBlockingInner {
                    holder: None,
                    wait_queue: VecDeque::new(),
                })
            },
        }
    }
}

impl TraceProcession<TaskControlBlock> for MutexBlocking {
    fn trace_owner(&self) -> Option<Arc<TaskControlBlock>> {
        self.inner.exclusive_access().holder.clone()
    }

    fn trace_waiters(&self) -> Vec<Arc<TaskControlBlock>> {
        self.inner.exclusive_access().wait_queue.iter().cloned().collect()
    }
}

impl Mutex<TaskControlBlock> for MutexBlocking {
    /// lock the blocking mutex
    fn lock(&self) {
        trace!("kernel: MutexBlocking::lock");
        let mut mutex_inner = self.inner.exclusive_access();


        if mutex_inner.holder.is_some() {
            // 锁已被其他线程持有，当前线程进入等待队列
            mutex_inner.wait_queue.push_back(current_task().unwrap());
            drop(mutex_inner);
            block_current_and_run_next();
        } else {
            // 锁未被任何线程持有，当前线程获取锁
            mutex_inner.holder = Some(current_task().unwrap());
        }
    }

    /// unlock the blocking mutex
    fn unlock(&self) {
        trace!("kernel: MutexBlocking::unlock");
        let mut mutex_inner = self.inner.exclusive_access();
        assert!(mutex_inner.holder.is_some());

        assert_eq!(mutex_inner.holder.clone().unwrap().get_tid(), current_task().unwrap().get_tid()); // 确保解锁的是当前持有锁的线程

        if let Some(waking_task) = mutex_inner.wait_queue.pop_front() {
            wakeup_task(waking_task.clone());
            mutex_inner.holder = Some(waking_task); // 更新持有锁的线程
        } else {
            mutex_inner.holder = None; // 没有线程持有锁
        }
    }
}