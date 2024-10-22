//! Process management syscalls

use crate::task::{get_current_task_status, get_current_task_syscall_times};
use crate::timer::get_time_ms;
use crate::{
    config::MAX_SYSCALL_NUM,
    task::{exit_current_and_run_next, get_current_task_start_time, suspend_current_and_run_next, TaskStatus},
    timer::get_time_us,
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// Task information
#[allow(dead_code)]
pub struct TaskInfo {
    /// Task status in it's life cycle
    status: TaskStatus,
    /// The numbers of syscall called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}


impl TaskInfo {
    pub fn set_status(&mut self, status: TaskStatus) -> &mut Self {
        self.status = status;
        self
    }

    pub fn set_syscall_times(&mut self, syscall_times: [u32; MAX_SYSCALL_NUM]) -> &mut Self {
        self.syscall_times = syscall_times;
        self
    }

    pub fn set_time(&mut self, time: usize) -> &mut Self {
        self.time = time;
        self
    }
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("[kernel] Application exited with code {}", exit_code);
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// get time with second and microsecond
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    unsafe {
        *ts = TimeVal {
            sec: us / 1_000_000,
            usec: us % 1_000_000,
        };
    }
    0
}

/// YOUR JOB: Finish sys_task_info to pass testcases
pub fn sys_task_info(ti_ptr: *mut TaskInfo) -> isize {
    trace!("kernel: sys_task_info");

    match unsafe { ti_ptr.as_mut() } {
        None => { -1 }
        Some(ti) => {

            // Fill the TaskInfo structure
            let current_task_status = get_current_task_status();
            let elapsed_time = get_time_ms() - get_current_task_start_time();
            let syscall_count = get_current_task_syscall_times();

            // Set the fields of TaskInfo
            ti.set_status(current_task_status)
                .set_time(elapsed_time)
                .set_syscall_times(syscall_count);
            0
        }
    }
    // Check if the pointer is valid
}
