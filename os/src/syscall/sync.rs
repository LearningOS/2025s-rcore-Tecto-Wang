use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task, TaskControlBlock};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        process_inner.m_available[id] = 1;
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.m_available.push(1);
        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());

    let task = &current_task().unwrap();

    if process_inner.deadlock_detect_enabled {
        let mut task_inner = task.inner_exclusive_access();
        ensure_vec_len(&mut task_inner.m_need, mutex_id);
        ensure_vec_len(&mut task_inner.m_allocation, mutex_id);
        task_inner.m_need[mutex_id] += 1;

        drop(task_inner);
        let tasks = process_inner
            .tasks
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let available = &process_inner.m_available;

        if !is_safe_state(available, &tasks, true) {
            // 回滚 need
            let mut task_inner = task.inner_exclusive_access();
            task_inner.m_need[mutex_id] -= 1;
            return -0xdead;
        }

        let mut task_inner = task.inner_exclusive_access();
        task_inner.m_need[mutex_id] -= 1;
        task_inner.m_allocation[mutex_id] += 1;
        process_inner.m_available[mutex_id] -= 1;
        drop(task_inner);
    }

    drop(process_inner);
    drop(process);
    mutex.lock();
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());

    process_inner.m_available[mutex_id] += 1;

    drop(process_inner);
    drop(process);
    mutex.unlock();
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        process_inner.s_available[id] = res_count;

        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.s_available.push(res_count);
        process_inner.semaphore_list.len() - 1
    };
    drop(process_inner);

    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());

    process_inner.s_available[sem_id] += 1;

    drop(process_inner);
    sem.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    let task = &current_task().unwrap();

    if process_inner.deadlock_detect_enabled {
        let mut task_inner = task.inner_exclusive_access();
        ensure_vec_len(&mut task_inner.s_need, sem_id);
        ensure_vec_len(&mut task_inner.s_allocation, sem_id);

        task_inner.s_need[sem_id] += 1;

        // 释放锁，确保资源分配和死锁检测的分离
        drop(task_inner);

        // 模拟资源分配并进行死锁检测
        let tasks = process_inner
            .tasks
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let available = &process_inner.s_available;

        if !is_safe_state(available, &tasks, false) {
            // 死锁检测失败，回滚资源需求
            let mut task_inner = task.inner_exclusive_access();
            task_inner.s_need[sem_id] -= 1;
            return -0xdead;  // 返回死锁错误
        }

        // 如果没有死锁，更新资源分配
        let mut task_inner = task.inner_exclusive_access();
        task_inner.s_need[sem_id] -= 1;
        task_inner.s_allocation[sem_id] += 1;
        process_inner.s_available[sem_id] -= 1;
        drop(task_inner);
    }

    drop(process_inner);
    sem.down();
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    trace!("kernel: sys_enable_deadlock_detect");
    let process = current_process();
    let mut inner = process.inner_exclusive_access();
    if enabled == 1 {
        inner.deadlock_detect_enabled = true;
    } else if enabled == 0 {
        inner.deadlock_detect_enabled = false;
    } else {
        return -1;
    }
    0
}

///
fn is_safe_state(available: &[usize], tasks: &[Arc<TaskControlBlock>], use_mutex: bool) -> bool {
    let mut work = available.to_vec();
    let mut finish = vec![false; tasks.len()];

    loop {
        let mut progress = false;

        for (i, task) in tasks.iter().enumerate() {
            if finish[i] {
                continue; // 已完成的任务无需检查
            }
            
            let inner = task.inner_exclusive_access();
            let (need, allocation) = if use_mutex {
                (&inner.m_need, &inner.m_allocation)
            } else {
                (&inner.s_need, &inner.s_allocation)
            };

            // 确保长度一致，避免越界
            if need.len() > work.len() || allocation.len() > work.len() {
                continue;
            }

            // 检查 need ≤ work
            if need.iter().zip(work.iter()).all(|(&n, &w)| n <= w) {
                // 模拟释放资源：work += allocation
                for j in 0..allocation.len() {
                    work[j] += allocation[j];
                }
                finish[i] = true;
                progress = true;
            }
        }

        if !progress {
            break;
        }
    }

    finish.iter().all(|&f| f)
}

/*///
fn try_allocate_and_check_safe(
    task: &Arc<TaskControlBlock>,
    available: &mut [usize],
    use_mutex: bool,
    resource_id: usize,
    tasks: &[Arc<TaskControlBlock>],
) -> bool {
    let mut tcb_inner = task.inner_exclusive_access();

    // 检查资源ID是否合法
    if resource_id >= tcb_inner.m_need.len() || resource_id >= tcb_inner.m_allocation.len() || resource_id >= available.len() {
        return false;
    }

    // 分配资源
    if use_mutex {
        tcb_inner.m_need[resource_id] -= 1;
        tcb_inner.m_allocation[resource_id] += 1;
        available[resource_id] -= 1;
    } else {
        tcb_inner.s_need[resource_id] -= 1;
        tcb_inner.s_allocation[resource_id] += 1;
        available[resource_id] -= 1;
    }

    // 进行死锁检测
    let safe = is_safe_state(available, tasks, use_mutex);

    if !safe {
        // 回滚资源分配
        if use_mutex {
            tcb_inner.m_need[resource_id] += 1;
            tcb_inner.m_allocation[resource_id] -= 1;
        } else {
            tcb_inner.s_need[resource_id] += 1;
            tcb_inner.s_allocation[resource_id] -= 1;
        }
        available[resource_id] += 1;
    }

    safe
}*/

///
fn ensure_vec_len(vec: &mut Vec<usize>, index: usize) {
    if vec.len() <= index {
        vec.resize(index + 1, 0);
    }
}
