//! Implementation of  [`ProcessControlBlock`]

use super::id::RecycleAllocator;
use super::manager::insert_into_pid2process;
use super::TaskControlBlock;
use super::{add_task, SignalFlags};
use super::{pid_alloc, PidHandle};
use crate::fs::{File, Stdin, Stdout};
use crate::mm::{translated_refmut, MemorySet, KERNEL_SPACE};
use crate::sync::{Condvar, Mutex, Semaphore, UPSafeCell};
use crate::trap::{trap_handler, TrapContext};
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefMut;

/// Process Control Block
pub struct ProcessControlBlock {
    /// immutable
    pub pid: PidHandle,
    /// mutable
    inner: UPSafeCell<ProcessControlBlockInner>,
}


impl Default for ProcessControlBlockInner {
    fn default() -> Self {
        Self {
            is_zombie: false,
            memory_set: MemorySet::new_bare(),
            parent: None,
            children: Vec::new(),
            exit_code: 0,
            fd_table: vec![None, Some(Arc::new(Stdin)), Some(Arc::new(Stdout))],
            signals: SignalFlags::empty(),
            tasks: Vec::new(),
            task_res_allocator: RecycleAllocator::new(),
            mutex_list: Vec::new(),
            semaphore_list: Vec::new(),
            condvar_list: Vec::new(),
            enabled_dead_lock_detection: false,
        }
    }
}

/// Inner of Process Control Block
pub struct ProcessControlBlockInner {
    /// is zombie?
    pub is_zombie: bool,
    /// memory set(address space)
    pub memory_set: MemorySet,
    /// parent process
    pub parent: Option<Weak<ProcessControlBlock>>,
    /// children process
    pub children: Vec<Arc<ProcessControlBlock>>,
    /// exit code
    pub exit_code: i32,
    /// file descriptor table
    pub fd_table: Vec<Option<Arc<dyn File + Send + Sync>>>,
    /// signal flags
    pub signals: SignalFlags,
    /// tasks(also known as threads)
    pub tasks: Vec<Option<Arc<TaskControlBlock>>>,
    /// task resource allocator
    pub task_res_allocator: RecycleAllocator,
    /// mutex list
    pub mutex_list: Vec<Option<Arc<dyn Mutex<TaskControlBlock>>>>,
    /// semaphore list
    pub semaphore_list: Vec<Option<Arc<Semaphore>>>,
    /// condvar list
    pub condvar_list: Vec<Option<Arc<Condvar>>>,

    /// enable dead lock detection
    pub enabled_dead_lock_detection: bool,
}

impl ProcessControlBlockInner {
    #[allow(unused)]
    /// get the address of app's page table
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    /// allocate a new file descriptor
    pub fn alloc_fd(&mut self) -> usize {
        if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
            fd
        } else {
            self.fd_table.push(None);
            self.fd_table.len() - 1
        }
    }
    /// allocate a new task id
    pub fn alloc_tid(&mut self) -> usize {
        self.task_res_allocator.alloc()
    }
    /// deallocate a task id
    pub fn dealloc_tid(&mut self, tid: usize) {
        self.task_res_allocator.dealloc(tid)
    }
    /// the count of tasks(threads) in this process
    pub fn thread_count(&self) -> usize {
        self.tasks.len()
    }
    /// get a task with tid in this process
    pub fn get_task(&self, tid: usize) -> Arc<TaskControlBlock> {
        self.tasks[tid].as_ref().unwrap().clone()
    }
}

impl ProcessControlBlock {
    /// inner_exclusive_access
    pub fn inner_exclusive_access(&self) -> RefMut<'_, ProcessControlBlockInner> {
        self.inner.exclusive_access()
    }
    /// new process from elf file
    pub fn new(elf_data: &[u8]) -> Arc<Self> {
        trace!("kernel: ProcessControlBlock::new");
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, ustack_base, entry_point) = MemorySet::from_elf(elf_data);
        // allocate a pid
        let pid_handle = pid_alloc();
        let process = Arc::new(Self {
            pid: pid_handle,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    memory_set,
                    fd_table: vec![
                        // 0 -> stdin
                        Some(Arc::new(Stdin)),
                        // 1 -> stdout
                        Some(Arc::new(Stdout)),
                        // 2 -> stderr
                        Some(Arc::new(Stdout)),
                    ],
                    ..ProcessControlBlockInner::default()
                })
            },
        });
        // create a main thread, we should allocate ustack and trap_cx here
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&process),
            ustack_base,
            true,
        ));
        // prepare trap_cx of main thread
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        let ustack_top = task_inner.res.as_ref().unwrap().ustack_top();
        let kstack_top = task.kstack.get_top();
        drop(task_inner);
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            ustack_top,
            KERNEL_SPACE.exclusive_access().token(),
            kstack_top,
            trap_handler as usize,
        );
        // add main thread to the process
        let mut process_inner = process.inner_exclusive_access();
        process_inner.tasks.push(Some(Arc::clone(&task)));
        drop(process_inner);
        insert_into_pid2process(process.getpid(), Arc::clone(&process));
        // add main thread to scheduler
        add_task(task);
        process
    }

    /// Only support processes with a single thread.
    pub fn exec(self: &Arc<Self>, elf_data: &[u8], args: Vec<String>) {
        trace!("kernel: exec");
        assert_eq!(self.inner_exclusive_access().thread_count(), 1);
        // memory_set with elf program headers/trampoline/trap context/user stack
        trace!("kernel: exec .. MemorySet::from_elf");
        let (memory_set, ustack_base, entry_point) = MemorySet::from_elf(elf_data);
        let new_token = memory_set.token();
        // substitute memory_set
        trace!("kernel: exec .. substitute memory_set");
        self.inner_exclusive_access().memory_set = memory_set;
        // then we alloc user resource for main thread again
        // since memory_set has been changed
        trace!("kernel: exec .. alloc user resource for main thread again");
        let task = self.inner_exclusive_access().get_task(0);
        let mut task_inner = task.inner_exclusive_access();
        task_inner.res.as_mut().unwrap().ustack_base = ustack_base;
        task_inner.res.as_mut().unwrap().alloc_user_res();
        task_inner.trap_cx_ppn = task_inner.res.as_mut().unwrap().trap_cx_ppn();
        // push arguments on user stack
        trace!("kernel: exec .. push arguments on user stack");
        let mut user_sp = task_inner.res.as_mut().unwrap().ustack_top();
        user_sp -= (args.len() + 1) * core::mem::size_of::<usize>();
        let argv_base = user_sp;
        let mut argv: Vec<_> = (0..=args.len())
            .map(|arg| {
                translated_refmut(
                    new_token,
                    (argv_base + arg * core::mem::size_of::<usize>()) as *mut usize,
                )
            })
            .collect();
        *argv[args.len()] = 0;
        for i in 0..args.len() {
            user_sp -= args[i].len() + 1;
            *argv[i] = user_sp;
            let mut p = user_sp;
            for c in args[i].as_bytes() {
                *translated_refmut(new_token, p as *mut u8) = *c;
                p += 1;
            }
            *translated_refmut(new_token, p as *mut u8) = 0;
        }
        // make the user_sp aligned to 8B for k210 platform
        user_sp -= user_sp % core::mem::size_of::<usize>();
        // initialize trap_cx
        trace!("kernel: exec .. initialize trap_cx");
        let mut trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            task.kstack.get_top(),
            trap_handler as usize,
        );
        trap_cx.x[10] = args.len();
        trap_cx.x[11] = argv_base;
        *task_inner.get_trap_cx() = trap_cx;
    }

    /// Only support processes with a single thread.
    pub fn fork(self: &Arc<Self>) -> Arc<Self> {
        trace!("kernel: fork");
        let mut parent = self.inner_exclusive_access();
        assert_eq!(parent.thread_count(), 1);
        // clone parent's memory_set completely including trampoline/ustacks/trap_cxs
        let memory_set = MemorySet::from_existed_user(&parent.memory_set);
        // alloc a pid
        let pid = pid_alloc();
        // copy fd table
        let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
        for fd in parent.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        // create child process pcb
        let child = Arc::new(Self {
            pid,
            inner: unsafe {
                UPSafeCell::new(ProcessControlBlockInner {
                    memory_set,
                    parent: Some(Arc::downgrade(self)),
                    fd_table: new_fd_table,
                    ..ProcessControlBlockInner::default()
                })
            },
        });
        // add child
        parent.children.push(Arc::clone(&child));
        // create main thread of child process
        let task = Arc::new(TaskControlBlock::new(
            Arc::clone(&child),
            parent
                .get_task(0)
                .inner_exclusive_access()
                .res
                .as_ref()
                .unwrap()
                .ustack_base(),
            // here we do not allocate trap_cx or ustack again
            // but mention that we allocate a new kstack here
            false,
        ));
        // attach task to child process
        let mut child_inner = child.inner_exclusive_access();
        child_inner.tasks.push(Some(Arc::clone(&task)));
        drop(child_inner);
        // modify kstack_top in trap_cx of this thread
        let task_inner = task.inner_exclusive_access();
        let trap_cx = task_inner.get_trap_cx();
        trap_cx.kernel_sp = task.kstack.get_top();
        drop(task_inner);
        insert_into_pid2process(child.getpid(), Arc::clone(&child));
        // add this thread to scheduler
        add_task(task);
        child
    }
    /// get pid
    pub fn getpid(&self) -> usize {
        self.pid.0
    }


    /// enable deadlock detection
    pub fn enable_deadlock_detect(&self) {
        trace!("kernel: enable deadlock detection");
        self.inner_exclusive_access().enabled_dead_lock_detection = true;
    }


    /// disable deadlock detection

    pub fn disable_deadlock_detect(&self) {
        trace!("kernel: disable deadlock detection");
        self.inner_exclusive_access().enabled_dead_lock_detection = false;
    }

    /// check if deadlock detection is enabled
    pub fn is_deadlock_detect_enabled(&self) -> bool {
        self.inner_exclusive_access().enabled_dead_lock_detection
    }

    /// resolve the dependency of the mutex for the given task
    /// that 's say check if the given task can acquire the mutex without causing a deadlock
    /// - true for do cause deadlock
    /// - false for do not cause deadlock
    /// # Using BFS to detect loop dependency
    /// check if that the thread with tid get into the waitqueue of the mutex in `mutex_id`  will cause
    /// the owner of the mutex in `mutex_id` depends on the mutex processed by the given thread tid
    ///
    /// ** note: the check will not allocate mutex to the thread during the check,
    /// *and assume there is no deadlock in the system already, this check is a pre-test to check if
    /// user will cause a deadlock if the given thread does have acquired the mutex in `mutex_id`
    pub fn resolve_mutex_dependency(&self, tid: usize, mutex_id: usize) -> bool {
        trace!("kernel: resolve mutex dependency");
        assert!(self.is_deadlock_detect_enabled());

        // Get the current holder of the mutex that the thread is trying to acquire
        if let Some(holder_tid) = self.get_mutex_holder(mutex_id) {
            debug!("kernel: resolve: mutex {} is held by thread {}", mutex_id, holder_tid);

            // A set to keep track of visited threads to avoid cycles
            // A queue for BFS, starting with the holder of the mutex
            let mut queue = VecDeque::new();
            queue.push_back(holder_tid);


            while let Some(current_tid) = queue.pop_front() {
                // Get all mutexes held by the current thread
                if self.possessed_mutexes(current_tid)
                    .iter()
                    .any(
                        |&possessed_mutex|
                            {
                                debug!("kernel: resolve: thread {} is holding mutex {}", current_tid, possessed_mutex);
                                self.get_mutex_waiting_tasks(possessed_mutex)
                                    .iter()
                                    .map(|&other_waiting_tid| {
                                        // add the waiting thread to the queue if it hasn't been marked as visited
                                        queue.push_back(other_waiting_tid);

                                        // return the waiting thread intact
                                        other_waiting_tid
                                    })
                                    .any(
                                        |waiting_tid| {
                                            // If the waiting thread is the thread we're trying to acquire,
                                            // then we have a cycle and a deadlock will occur
                                            waiting_tid == tid
                                        }
                                    )
                            }
                    ) {
                    return true;
                }
            }
        }
        // If we exit the loop without finding a cycle, no deadlock will be caused
        false
    }

    /// get the holder of the given mutex
    pub fn get_mutex_holder(&self, mutex_id: usize) -> Option<usize> {
        let inner = self.inner_exclusive_access();
        if let Some(Some(lock)) = inner.mutex_list.get(mutex_id) {
            lock.trace_owner().map(|t| t.get_tid())
        } else {
            None
        }
    }

    /// get the tasks that are waiting for the given mutex
    pub fn get_mutex_waiting_tasks(&self, mutex_id: usize) -> Vec<usize> {
        trace!("kernel: get waiting tasks");
        let inner = self.inner_exclusive_access();
        if let Some(Some(lock)) = inner.mutex_list.get(mutex_id) {
            lock.trace_waiters().iter().map(|t| t.get_tid()).collect()
        } else {
            vec![]
        }
    }

    /// get the mutexes that the given task has possessed
    /// *return a vector of mutex id
    pub fn possessed_mutexes(&self, tid: usize) -> Vec<usize> {
        trace!("kernel: get processed mutexes");
        let inner = self.inner_exclusive_access();
        inner.mutex_list.iter().enumerate().filter_map(|(i, lock)| {
            match lock {
                Some(ava_lock) if ava_lock.trace_owner()?.get_tid() == tid => {
                    Some(i)
                }
                _ => {
                    None
                }
            }
        }).collect()
    }

    /// get the mutexes that the given task is waiting for
    pub fn waiting_mutexes(&self, tid: usize) -> Vec<usize> {
        trace!("kernel: get waiting mutexes");
        let inner = self.inner_exclusive_access();
        inner.mutex_list.iter().enumerate().filter_map(|(i, lock)| {
            return match lock {
                Some(ava_lock)
                // check if the given task is in the wait queue of the mutex
                if ava_lock.trace_waiters().iter().any(|waiting_task| waiting_task.get_tid() == tid) => {
                    Some(i)
                }
                _ => {
                    None
                }
            };
        }).collect()
    }

    /// get the semaphores that the given task has possessed

    pub fn possessed_semaphores(&self, tid: usize) -> Vec<usize> {
        trace!("kernel: get processed semaspheres");
        let inner = self.inner_exclusive_access();
        inner.semaphore_list.iter().enumerate().filter_map(|(i, sem)| {
            match sem {
                Some(sem) if
                sem.holders().iter().any(|task| task.get_tid() == tid) => {
                    Some(i)
                }
                _ => { None }
            }
        }).collect()
    }
    /// get the count of the given task has possessed the given semaphore

    pub fn possessed_semaphore_count(&self, tid: usize, sem_id: usize) -> usize {
        trace!("kernel: get processed semaspheres");
        let inner = self.inner_exclusive_access();
        inner.semaphore_list[sem_id].as_ref().unwrap().holders().iter().filter(|task| task.get_tid() == tid).count()
    }

    pub fn all_counts(&self, sem_id: usize) -> usize {
        trace!("kernel: get all counts");
        let inner = self.inner_exclusive_access();
        inner.semaphore_list[sem_id].as_ref().unwrap().all_resource_count()
    }

    /// get the semaphores that the given task is waiting for
    pub fn waiting_semaphores(&self, tid: usize) -> Vec<usize> {
        trace!("kernel: get waiting semaspheres");
        let inner = self.inner_exclusive_access();
        inner.semaphore_list.iter().enumerate().filter_map(|(i, sem)| {
            match sem {
                Some(sem)
                // check if the given task is in the wait queue of the semaphore
                if sem.waiting_tasks().iter().any(|waiting_task| waiting_task.get_tid() == tid) => {
                    Some(i)
                }
                _ => {
                    None
                }
            }
        }).collect()
    }


    /// get the holders of the given semaphore
    pub fn get_sem_holders(&self, sem_id: usize) -> Vec<usize> {
        trace!("kernel: get semaphore holders");
        let inner = self.inner_exclusive_access();
        inner.semaphore_list[sem_id].as_ref().unwrap().holders().iter().map(|t| t.get_tid()).collect()
    }


    /// get the tasks that are waiting for the given semaphore
    pub fn get_sem_waiting_tasks(&self, sem_id: usize) -> Vec<usize> {
        trace!("kernel: get semaphore waiting tasks");
        let inner = self.inner_exclusive_access();
        inner.semaphore_list[sem_id].as_ref().unwrap().waiting_tasks().iter().map(|t| t.get_tid()).collect()
    }


    /// get the remaining count of the given semaphore
    pub fn get_sem_remaining_count(&self, sem_id: usize) -> isize {
        trace!("kernel: get semaphore remaining count");
        let inner = self.inner_exclusive_access();
        inner.semaphore_list[sem_id].as_ref().unwrap().remaining()
    }


    /// resolve the dependency of the given semaphore
    /// ** Check if that the semaphore with `sid` is downed by 1 by the task with `tid` will cause a deadlock
    pub fn resolve_semaphore_dependency(&self, tid: usize, sid: usize) -> bool {
        trace!("kernel: resolve semaphore dependency");
        assert!(self.is_deadlock_detect_enabled());

        // Get the current holders of the semaphore that the thread is trying to acquire
        let holders = self.get_sem_holders(sid);
        debug!("kernel: resolve: checking dependency of semaphore {tid}|{sid}, sid holders {:?}", holders);

        if self.get_sem_remaining_count(sid) <= 0 {
            debug!("kernel: resolve: semaphore {} is held by threads {:?}", sid, holders);

            // A queue for BFS, starting with the holders of the semaphore
            let mut queue = VecDeque::from_iter(holders.clone());
            // A set to keep track of visited threads to avoid cycles
            while let Some(sem_holder) = queue.pop_front() {
                // Get all semaphores held by the current thread
                if self.waiting_semaphores(sem_holder)
                    .iter()
                    .any(|&sem_id| {
                        self.get_sem_holders(sem_id)
                            .iter()
                            .any(|&holder| {
                                if holder == tid {
                                    trace!("kernel: resolve: {} matches,dead lock detected", holder);
                                    true
                                } else if self.waiting_semaphores(holder).is_empty() {
                                    trace!("kernel: resolve: {} has no waiting semaphores, no dead lock", holder);
                                    false
                                } else {
                                    trace!("kernel: resolve: {} is waiting for semaphores {:?}", holder, self.waiting_semaphores(holder));
                                    queue.push_back(holder);
                                    false
                                }
                            })
                    }) {
                    return true;
                }
            }
        }
        // If we exit the loop without finding a cycle, no deadlock will be caused
        false
    }
}
