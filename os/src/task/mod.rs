//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the operating system.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.

mod context;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use crate::loader::{get_app_data, get_num_app};
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::vec::Vec;
use lazy_static::*;
use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus};

use crate::config::PAGE_SIZE;
use crate::mm::{MapPermission, VirtAddr};
pub use context::TaskContext;

/// The task manager, where all the tasks are managed.
///
/// Functions implemented on `TaskManager` deals with all task state transitions
/// and task context switching. For convenience, you can find wrappers around it
/// in the module level.
///
/// Most of `TaskManager` are hidden behind the field `inner`, to defer
/// borrowing checks to runtime. You can see examples on how to use `inner` in
/// existing functions on `TaskManager`.
pub struct TaskManager {
    /// total number of tasks
    num_app: usize,
    /// use inner value to get mutable access
    inner: UPSafeCell<TaskManagerInner>,
}

/// The task manager inner in 'UPSafeCell'
struct TaskManagerInner {
    /// task list
    tasks: Vec<TaskControlBlock>,
    /// id of current `Running` task
    current_task: usize,
}

lazy_static! {
    /// a `TaskManager` global instance through lazy_static!
    pub static ref TASK_MANAGER: TaskManager = {
        println!("init TASK_MANAGER");
        let num_app = get_num_app();
        println!("num_app = {}", num_app);
        let mut tasks: Vec<TaskControlBlock> = Vec::new();
        for i in 0..num_app {
            tasks.push(TaskControlBlock::new(get_app_data(i), i));
        }
        TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks,
                    current_task: 0,
                })
            },
        }
    };
}

impl TaskManager {
    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch4, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let next_task = &mut inner.tasks[0];
        next_task.task_status = TaskStatus::Running;
        let next_task_cx_ptr = &next_task.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
        unsafe {
            __switch(&mut _unused as *mut _, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Ready;
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Exited;
    }

    /// Find next task to run and return task id.
    ///
    /// In this case, we only return the first `Ready` task in task list.
    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        (current + 1..current + self.num_app + 1)
            .map(|id| id % self.num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    /// Get the current 'Running' task's token.
    fn get_current_token(&self) -> usize {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_user_token()
    }

    /// Get the current 'Running' task's trap contexts.
    fn get_current_trap_cx(&self) -> &'static mut TrapContext {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_trap_cx()
    }

    /// Change the current 'Running' task's program break
    pub fn change_current_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].change_program_brk(size)
    }

    /// Switch current `Running` task to the task we have found,
    /// or there is no `Ready` task and we can exit with all applications completed
    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            inner.current_task = next;
            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            drop(inner);
            // before this, we should drop local variables that must be dropped manually
            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
            // go back to user mode
        } else {
            panic!("All applications completed!");
        }
    }

    fn add_syscall_count(&self, syscall_id: usize) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].syscall_count[syscall_id] += 1;
    }

    fn get_syscall_count(&self, syscall_id: usize) -> usize {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;

        let count = inner.tasks[current].syscall_count[syscall_id];

        count
    }

    fn sys_mmap(&self, start: usize, len: usize, prot: usize) -> isize {
        // 检查合法性
        if len == 0 || start % PAGE_SIZE != 0 || prot & !0x7 != 0 || prot & 0x7 == 0 {
            return -1;
        }

        let start_va: VirtAddr = start.into();
        let end_va: VirtAddr = (start + len).into();

        // 检查是否与已有区域重叠
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        let memory_set = &mut inner.tasks[current].memory_set;

        if memory_set.is_overlap(start_va, end_va) {
            return -1;
        }

        // 解析 prot -> MapPermission
        let mut map_perm = MapPermission::U;
        if prot & 0x1 != 0 {
            map_perm |= MapPermission::R;
        }
        if prot & 0x2 != 0 {
            map_perm |= MapPermission::W;
        }
        if prot & 0x4 != 0 {
            map_perm |= MapPermission::X;
        }

        // 插入映射区域（自动 map + 加入 areas）
        memory_set.insert_framed_area(start_va, end_va, map_perm);

        0
    }

    fn sys_munmap(&self, start: usize, len: usize) -> isize {
        // 参数基本检查（页对齐）
        if len == 0 || start % PAGE_SIZE != 0 {
            return -1;
        }

        let start_va: VirtAddr = start.into();
        let end_va: VirtAddr = (start + len).into();

        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        let memory_set = &mut inner.tasks[current].memory_set;

        // 只支持完全匹配的区域
        if memory_set.remove_area(start_va, end_va) {
            0
        } else {
            -1
        }
    }

    fn read_trace(&self, id: usize) -> isize {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        let memory_set = &mut inner.tasks[current].memory_set;

        let va = VirtAddr::from(id);

        if !memory_set.is_user_accessible(va, false) {
            return -1;
        }

        match memory_set.get_byte_from_user_space(va) {
            Some(value) => value as isize, // 返回读取的字节
            None => -1,                    // 无法读取
        }
    }

    fn write_trace(&self, id: usize, data: usize) -> isize {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        let memory_set = &mut inner.tasks[current].memory_set;

        let va = VirtAddr::from(id);

        if !memory_set.is_user_accessible(va, true) {
            return -1;
        }

        if memory_set.write_byte_to_user_space(va, data as u8) {
            0 // 成功写入
        } else {
            -1 // 写入失败
        }
    }
    
    fn write_bytes_buffer(&self, ptr: usize, len: usize, bytes: &[u8]) -> isize {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        let memory_set = &mut inner.tasks[current].memory_set;
        for i in 0..len {
            let va = VirtAddr::from(ptr + i);
            if !memory_set.is_user_accessible(va, true) {
                return -1;
            }
            if !memory_set.write_byte_to_user_space(va, bytes[i]) {
                return -1;
            }
        }
        0
        
    }
}

/// Run the first task in task list.
pub fn run_first_task() {
    TASK_MANAGER.run_first_task();
}

/// Switch current `Running` task to the task we have found,
/// or there is no `Ready` task and we can exit with all applications completed
fn run_next_task() {
    TASK_MANAGER.run_next_task();
}

/// Change the status of current `Running` task into `Ready`.
fn mark_current_suspended() {
    TASK_MANAGER.mark_current_suspended();
}

/// Change the status of current `Running` task into `Exited`.
fn mark_current_exited() {
    TASK_MANAGER.mark_current_exited();
}

/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    mark_current_suspended();
    run_next_task();
}

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next() {
    mark_current_exited();
    run_next_task();
}

/// Get the current 'Running' task's token.
pub fn current_user_token() -> usize {
    TASK_MANAGER.get_current_token()
}

/// Get the current 'Running' task's trap contexts.
pub fn current_trap_cx() -> &'static mut TrapContext {
    TASK_MANAGER.get_current_trap_cx()
}

/// Change the current 'Running' task's program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    TASK_MANAGER.change_current_program_brk(size)
}

///
pub fn add_syscall_count(syscall_id: usize) {
    TASK_MANAGER.add_syscall_count(syscall_id);
}

///
pub fn get_syscall_count(syscall_id: usize) -> isize {
    TASK_MANAGER.get_syscall_count(syscall_id) as isize
}

///
pub fn task_sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    TASK_MANAGER.sys_mmap(start, len, prot)
}

///
pub fn task_sys_unmmap(start: usize, len: usize) -> isize {
    TASK_MANAGER.sys_munmap(start, len)
}

///
pub fn task_read_trace(id: usize) -> isize {
    TASK_MANAGER.read_trace(id)
}

///
pub fn task_write_trace(id: usize, data: usize) -> isize {
    TASK_MANAGER.write_trace(id, data)
}

///
pub fn write_bytes_buffer(ptr: usize, len: usize, bytes: &[u8]) -> isize {
    TASK_MANAGER.write_bytes_buffer(ptr, len, bytes)
}