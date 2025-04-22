//! Process management syscalls

use crate::task::{change_program_brk, exit_current_and_run_next, get_syscall_count, suspend_current_and_run_next, task_read_trace, task_sys_mmap, task_sys_unmmap, task_write_trace, write_bytes_buffer};
use crate::timer::get_time_us;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");

    let us = get_time_us();
    let timeval = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let ptr = ts as usize;
    let len = core::mem::size_of::<TimeVal>();
    
    let bytes = unsafe {
        core::slice::from_raw_parts(
            &timeval as *const _ as *const u8,
            len,
        )
    };
    write_bytes_buffer(ptr, len, bytes)
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");

    match trace_request {
        // 读取：尝试从用户地址读取一个字节
        0 => {
            task_read_trace(id)
        }

        // 写入：尝试向用户地址写入一个字节
        1 => {
            task_write_trace(id, data)
        }

        // 获取系统调用计数
        2 => get_syscall_count(id),

        // 未知的 trace_request 类型
        _ => -1,
    }
}

pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");

    task_sys_mmap(start, len, prot)
}

pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    
    task_sys_unmmap(start, len)
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
