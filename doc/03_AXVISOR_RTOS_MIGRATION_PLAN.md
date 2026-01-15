# AxVisor 调度器 RTOS 改造方案

## 一、改造目标

将 AxVisor 从当前的 ArceOS 混合调度模型改造为完整的 RTOS 调度系统，实现：
1. **实时性保证**：优先级管理、时间片配额、截止时间监控
2. **资源隔离**：VM 间 CPU 配额管理、优先级隔离
3. **可靠性增强**：优先级继承、死锁检测、WCET 监控
4. **可配置性**：支持多种调度策略、灵活的 VM 优先级配置

---

## 二、核心概念转换

### 2.1 调度模型转换

```
当前 AxVisor 模型:
┌─────────────────────────────────────┐
│ VCpu Task (由 axtask 调度)          │
│  ├─ 运行 vm.run_vcpu()             │
│  ├─ 等待 wait/notify               │
│  └─ 检查 VM 状态                    │
└─────────────────────────────────────┘
             ↓ 转换为

RTOS 模型:
┌─────────────────────────────────────┐
│ RTOS Scheduler                       │
│  ├─ 优先级队列管理 (256 级)         │
│  ├─ 显式任务状态机                  │
│  ├─ 时间片轮转 + 优先级抢占         │
│  ├─ 死锁检测                        │
│  └─ CPU 配额管理 (按 VM)           │
│                                     │
│ VCpu Task 变为:                      │
│  - 优先级任务（192-223 级）         │
│  - 周期性任务（可选）               │
│  - 有 CPU 配额限制                  │
│  - 可以被高优先级任务抢占           │
└─────────────────────────────────────┘
```

### 2.2 关键数据结构设计

#### Task 结构增强

```rust
pub struct Task {
    // 基础信息
    pub id: TaskId,
    pub name: String,
    pub entry_point: fn(),
    
    // 优先级管理
    pub priority: u8,           // 静态优先级
    pub dynamic_priority: u8,   // 动态优先级（用于继承）
    
    // 状态管理
    pub state: TaskState,
    pub state_reason: Option<BlockReason>,
    
    // 时间管理
    pub time_slice_ticks: u16,
    pub time_slice_remaining: u16,
    pub deadline: Option<u64>,  // 纳秒
    pub period: Option<u64>,    // 周期任务的周期
    pub wcet_us: u64,          // 最坏情况执行时间
    pub executed_time_us: u64, // 当前周期执行时间
    
    // CPU 配额（针对 VCpu 任务的 VM）
    pub cpu_quota_us: Option<u64>,
    pub cpu_used_us: Option<u64>,
    pub vm_id: Option<usize>,  // 所属 VM
    
    // 栈和上下文
    pub stack: Vec<u8>,
    pub stack_pointer: usize,
    pub saved_context: Option<Context>,
    
    // 统计信息
    pub run_count: usize,       // 被调度的次数
    pub preempt_count: usize,   // 被抢占的次数
    pub deadline_miss_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked(BlockReason),
    Preempted,
    Suspended,
    Terminated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    Semaphore(SemaphoreId),
    Mutex(MutexId),
    CondVar(CondVarId),
    IO,
    Other(u32),
}
```

#### RTOS 调度器结构

```rust
pub struct RTOSScheduler {
    // 优先级队列
    priority_bitmap: u32,
    priority_groups: [u32; 8],
    ready_queues: [VecDeque<TaskId>; 256],
    
    // 任务管理
    all_tasks: BTreeMap<TaskId, Box<Task>>,
    task_names: BTreeMap<String, TaskId>,
    
    // 当前运行任务（每 CPU 一个）
    current_task: [Option<TaskId>; MAX_CPUS],
    previous_task: [Option<TaskId>; MAX_CPUS],
    
    // 同步原语
    semaphores: BTreeMap<SemaphoreId, Semaphore>,
    mutexes: BTreeMap<MutexId, Mutex>,
    cond_vars: BTreeMap<CondVarId, ConditionVariable>,
    
    // VM 分组
    vm_groups: BTreeMap<usize, TaskGroup>,
    
    // 统计和监控
    stats: SchedulerStats,
    deadlock_detector: DeadlockDetector,
}

pub struct SchedulerStats {
    total_context_switches: u64,
    total_preemptions: u64,
    deadline_misses: u64,
    deadlocks_detected: u64,
}
```

---

## 三、详细改造步骤

### 3.1 创建 RTOS 调度器模块

#### 目录结构

```bash
mkdir -p kernel/src/rtos
touch kernel/src/rtos/mod.rs
touch kernel/src/rtos/scheduler.rs
touch kernel/src/rtos/task.rs
touch kernel/src/rtos/sync.rs
touch kernel/src/rtos/time.rs
touch kernel/src/rtos/deadlock.rs
touch kernel/src/rtos/vcpu_integration.rs
```

#### 模块入口 (kernel/src/rtos/mod.rs)

```rust
pub mod scheduler;
pub mod task;
pub mod sync;
pub mod time;
pub mod deadlock;
pub mod vcpu_integration;

pub use scheduler::{RTOSScheduler, SCHEDULER};
pub use task::{Task, TaskId, TaskState, BlockReason};
pub use sync::{Mutex, Semaphore, ConditionVariable};
pub use time::{TimeManager, get_system_time_ns};
pub use deadlock::DeadlockDetector;

// 全局初始化
pub fn init_rtos() {
    RTOSScheduler::init();
    time::init_time_manager();
    log::info!("RTOS scheduler initialized");
}
```

### 3.2 实现优先级位图队列

**kernel/src/rtos/scheduler.rs**

```rust
use alloc::collections::VecDeque;
use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicUsize, Ordering};

/// 256 级优先级的位图快速队列
pub struct ReadyQueue {
    // 8 个 u32，共 256 位
    bitmap: [u32; 8],
    // 256 个 FIFO 队列
    queues: [VecDeque<TaskId>; 256],
}

impl ReadyQueue {
    pub const fn new() -> Self {
        unsafe {
            Self {
                bitmap: [0; 8],
                queues: core::array::from_fn(|_| VecDeque::new()),
            }
        }
    }

    /// 将任务加入指定优先级队列
    /// priority: 0 (highest) - 255 (lowest)
    #[inline]
    pub fn enqueue(&mut self, priority: u8, task_id: TaskId) {
        let group = (priority >> 5) as usize;  // / 32
        let bit = (priority & 0x1F) as usize;  // % 32

        self.bitmap[group] |= 1 << bit;
        self.queues[priority as usize].push_back(task_id);
    }

    /// 获取最高优先级的就绪任务
    #[inline]
    pub fn dequeue(&mut self) -> Option<TaskId> {
        for group in 0..8 {
            if self.bitmap[group] != 0 {
                // 找最低位（最高优先级）
                let bit = self.bitmap[group].trailing_zeros() as usize;
                let priority = group * 32 + bit;

                if let Some(task_id) = self.queues[priority].pop_front() {
                    // 如果队列为空，清除位
                    if self.queues[priority].is_empty() {
                        self.bitmap[group] &= !(1 << bit);
                    }
                    return Some(task_id);
                }
            }
        }
        None
    }

    /// 检查是否有就绪任务
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bitmap.iter().all(|&x| x == 0)
    }

    /// 获取最高优先级（不出队）
    #[inline]
    pub fn peek_priority(&self) -> Option<u8> {
        for group in 0..8 {
            if self.bitmap[group] != 0 {
                let bit = self.bitmap[group].trailing_zeros() as usize;
                return Some((group * 32 + bit) as u8);
            }
        }
        None
    }
}
```

### 3.3 实现任务管理

**kernel/src/rtos/task.rs**

```rust
use alloc::vec::Vec;
use alloc::string::String;
use core::sync::atomic::{AtomicU8, AtomicU16, AtomicUsize, Ordering};
use core::cell::RefCell;

pub type TaskId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked(BlockReason),
    Preempted,
    Suspended,
    Terminated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    Semaphore(usize),
    Mutex(usize),
    CondVar(usize),
    IO,
    Other(u32),
}

pub struct Task {
    pub id: TaskId,
    pub name: String,
    pub entry_fn: fn(),
    
    // 优先级管理
    pub static_priority: u8,
    pub dynamic_priority: AtomicU8,
    
    // 状态管理
    pub state: RefCell<TaskState>,
    pub state_reason: RefCell<Option<BlockReason>>,
    
    // 时间管理
    pub time_slice_ticks: u16,
    pub time_slice_remaining: AtomicU16,
    pub deadline_ns: Option<u64>,
    pub period_ns: Option<u64>,
    pub wcet_us: u64,
    pub exec_time_us: AtomicUsize,
    
    // CPU 配额（VCpu 专用）
    pub cpu_quota_us: Option<u64>,
    pub cpu_used_us: AtomicUsize,
    pub vm_id: Option<usize>,
    
    // 栈和上下文
    pub stack: Vec<u8>,
    pub stack_ptr: usize,
    
    // 统计信息
    pub run_count: AtomicUsize,
    pub preempt_count: AtomicUsize,
    pub deadline_miss_count: AtomicUsize,
}

impl Task {
    pub fn new(
        id: TaskId,
        name: String,
        entry_fn: fn(),
        priority: u8,
        stack_size: usize,
    ) -> Self {
        let mut stack = Vec::with_capacity(stack_size);
        stack.resize(stack_size, 0);
        
        Task {
            id,
            name,
            entry_fn,
            static_priority: priority,
            dynamic_priority: AtomicU8::new(priority),
            state: RefCell::new(TaskState::Ready),
            state_reason: RefCell::new(None),
            time_slice_ticks: 10,
            time_slice_remaining: AtomicU16::new(10),
            deadline_ns: None,
            period_ns: None,
            wcet_us: 10_000,
            exec_time_us: AtomicUsize::new(0),
            cpu_quota_us: None,
            cpu_used_us: AtomicUsize::new(0),
            vm_id: None,
            stack,
            stack_ptr: 0,
            run_count: AtomicUsize::new(0),
            preempt_count: AtomicUsize::new(0),
            deadline_miss_count: AtomicUsize::new(0),
        }
    }

    pub fn get_state(&self) -> TaskState {
        *self.state.borrow()
    }

    pub fn set_state(&self, state: TaskState) {
        *self.state.borrow_mut() = state;
    }

    pub fn get_priority(&self) -> u8 {
        self.dynamic_priority.load(Ordering::Acquire)
    }

    pub fn set_priority(&self, priority: u8) {
        self.dynamic_priority.store(priority, Ordering::Release);
    }
}
```

### 3.4 实现 RTOS 调度器核心

**kernel/src/rtos/scheduler.rs (续)**

```rust
pub struct RTOSScheduler {
    ready_queue: ReadyQueue,
    tasks: BTreeMap<TaskId, Task>,
    next_task_id: AtomicUsize,
    current_task: [Option<TaskId>; 4],
    previous_task: [Option<TaskId>; 4],
    total_switches: u64,
    total_preemptions: u64,
}

static mut GLOBAL_SCHEDULER: Option<RTOSScheduler> = None;

impl RTOSScheduler {
    pub fn new() -> Self {
        Self {
            ready_queue: ReadyQueue::new(),
            tasks: BTreeMap::new(),
            next_task_id: AtomicUsize::new(1),
            current_task: [None; 4],
            previous_task: [None; 4],
            total_switches: 0,
            total_preemptions: 0,
        }
    }

    pub fn init() {
        unsafe {
            GLOBAL_SCHEDULER = Some(Self::new());
        }
    }

    pub fn global() -> &'static mut Self {
        unsafe {
            GLOBAL_SCHEDULER.as_mut().expect("Scheduler not initialized")
        }
    }

    pub fn create_task(
        &mut self,
        name: String,
        entry_fn: fn(),
        priority: u8,
        stack_size: usize,
    ) -> TaskId {
        let task_id = self.next_task_id.fetch_add(1, Ordering::SeqCst);
        let task = Task::new(task_id, name, entry_fn, priority, stack_size);
        self.tasks.insert(task_id, task);
        
        self.ready_queue.enqueue(priority, task_id);
        
        task_id
    }

    pub fn schedule_next(&mut self, cpu_id: usize) -> Option<TaskId> {
        if let Some(next_task_id) = self.ready_queue.dequeue() {
            self.previous_task[cpu_id] = self.current_task[cpu_id];
            self.current_task[cpu_id] = Some(next_task_id);
            
            if let Some(task) = self.tasks.get(&next_task_id) {
                task.set_state(TaskState::Running);
                task.run_count.fetch_add(1, Ordering::Relaxed);
                task.time_slice_remaining
                    .store(task.time_slice_ticks, Ordering::Relaxed);
            }
            
            self.total_switches += 1;
            Some(next_task_id)
        } else {
            None
        }
    }

    pub fn block_task(&mut self, task_id: TaskId, reason: BlockReason) {
        if let Some(task) = self.tasks.get(&task_id) {
            task.set_state(TaskState::Blocked(reason));
            *task.state_reason.borrow_mut() = Some(reason);
        }
        
        let cpu_id = 0;
        if let Some(next_id) = self.schedule_next(cpu_id) {
            self.context_switch(cpu_id, next_id);
        }
    }

    pub fn unblock_task(&mut self, task_id: TaskId) {
        if let Some(task) = self.tasks.get(&task_id) {
            if matches!(task.get_state(), TaskState::Blocked(_)) {
                task.set_state(TaskState::Ready);
                *task.state_reason.borrow_mut() = None;
                
                self.ready_queue
                    .enqueue(task.get_priority(), task_id);
            }
        }
    }

    pub fn should_preempt(&self, cpu_id: usize) -> bool {
        if let Some(current_id) = self.current_task[cpu_id] {
            if let Some(next_priority) = self.ready_queue.peek_priority() {
                if let Some(current) = self.tasks.get(&current_id) {
                    return next_priority < current.get_priority();
                }
            }
        }
        false
    }

    fn context_switch(&self, _cpu_id: usize, _next_task_id: TaskId) {
        // 平台相关的实现
    }
}
```

### 3.5 实现互斥锁与优先级继承

**kernel/src/rtos/sync.rs**

```rust
use alloc::vec::Vec;
use core::cell::RefCell;
use alloc::collections::VecDeque;
use crate::rtos::{TaskId, RTOSScheduler, BlockReason};

pub struct Mutex {
    pub id: usize,
    owner: RefCell<Option<TaskId>>,
    count: RefCell<usize>,
    waiting: RefCell<VecDeque<TaskId>>,
    saved_priority: RefCell<Option<u8>>,
}

impl Mutex {
    pub fn new(id: usize) -> Self {
        Mutex {
            id,
            owner: RefCell::new(None),
            count: RefCell::new(0),
            waiting: RefCell::new(VecDeque::new()),
            saved_priority: RefCell::new(None),
        }
    }

    pub fn lock(&self, task_id: TaskId) {
        let mut scheduler = RTOSScheduler::global();
        
        loop {
            let mut owner = self.owner.borrow_mut();
            
            if owner.is_none() {
                *owner = Some(task_id);
                *self.count.borrow_mut() = 1;
                return;
            }
            
            if *owner == Some(task_id) {
                *self.count.borrow_mut() += 1;
                return;
            }
            
            // 优先级继承
            if let Some(owner_id) = *owner {
                let current_priority = scheduler
                    .tasks.get(&task_id)
                    .map(|t| t.get_priority())
                    .unwrap_or(255);
                
                let owner_priority = scheduler
                    .tasks.get(&owner_id)
                    .map(|t| t.get_priority())
                    .unwrap_or(255);
                
                if current_priority < owner_priority {
                    if self.saved_priority.borrow().is_none() {
                        *self.saved_priority.borrow_mut() = Some(owner_priority);
                    }
                    
                    if let Some(owner_task) = scheduler.tasks.get(&owner_id) {
                        owner_task.set_priority(current_priority);
                    }
                }
            }
            
            self.waiting.borrow_mut().push_back(task_id);
            
            drop(owner);
            scheduler.block_task(task_id, BlockReason::Mutex(self.id));
        }
    }

    pub fn unlock(&self, task_id: TaskId) -> Result<(), &'static str> {
        let mut owner = self.owner.borrow_mut();
        
        if *owner != Some(task_id) {
            return Err("Mutex not owned by this task");
        }
        
        let mut count = self.count.borrow_mut();
        *count -= 1;
        
        if *count == 0 {
            *owner = None;
            
            let mut scheduler = RTOSScheduler::global();
            if let Some(saved_priority) = self.saved_priority.borrow_mut().take() {
                if let Some(task) = scheduler.tasks.get(&task_id) {
                    task.set_priority(saved_priority);
                }
            }
            
            if let Some(waiter_id) = self.waiting.borrow_mut().pop_front() {
                scheduler.unblock_task(waiter_id);
                
                if scheduler.should_preempt(0) {
                    scheduler.preempt(0);
                }
            }
        }
        
        Ok(())
    }
}
```

### 3.6 VCpu 任务改造

**kernel/src/rtos/vcpu_integration.rs**

```rust
use crate::rtos::{Task, TaskId, TaskState, RTOSScheduler};
use crate::vmm::{VM, VCpuRef};

const VCPU_PRIORITY_BASE: u8 = 192;

pub struct VCpuRTOSTask {
    pub task_id: TaskId,
    pub priority: u8,
    pub vm_id: usize,
    pub vcpu_id: usize,
    pub vm: alloc::sync::Arc<VM>,
    pub vcpu: VCpuRef,
    pub cpu_quota_per_period: u64,
    pub cpu_used_this_period: u64,
    pub period_start: u64,
    pub period_ns: u64,
    pub wcet_us: u64,
    pub deadline: u64,
    pub deadline_miss_count: u32,
}

pub fn create_vcpu_task(
    vm: &alloc::sync::Arc<VM>,
    vcpu: &VCpuRef,
    vm_priority: u8,
    cpu_quota_ms: u64,
) -> TaskId {
    let scheduler = RTOSScheduler::global();
    
    let priority = VCPU_PRIORITY_BASE + vm_priority;
    let cpu_quota_us = cpu_quota_ms * 1000;
    
    let task_id = scheduler.create_task(
        format!("VM[{}]-VCpu[{}]", vm.id(), vcpu.id()),
        vcpu_run_entry,
        priority,
        256 * 1024,
    );
    
    if let Some(task) = scheduler.tasks.get(&task_id) {
        task.cpu_quota_us = Some(cpu_quota_us);
        task.vm_id = Some(vm.id());
        task.deadline_ns = Some(get_system_time_ns() + 100_000_000);
    }
    
    task_id
}

fn vcpu_run_entry() {
    // 获取当前任务的 VCpu 上下文
    let scheduler = RTOSScheduler::global();
    let current_id = scheduler.current_task[0].unwrap();
    
    let vm_id = scheduler.tasks.get(&current_id).unwrap().vm_id.unwrap();
    let vm = get_vm_by_id(vm_id);
    
    loop {
        // 检查 CPU 配额
        let task = scheduler.tasks.get(&current_id).unwrap();
        if let Some(used) = task.cpu_used_us.load(Ordering::Relaxed) {
            if let Some(quota) = task.cpu_quota_us {
                if used >= quota {
                    scheduler_yield();
                    continue;
                }
            }
        }
        
        // 运行 VCpu
        match vm.run_vcpu(0) {
            Ok(exit_reason) => {
                handle_vcpu_exit(vm.clone(), exit_reason);
            }
            Err(e) => {
                log::error!("VCpu run failed: {:?}", e);
                break;
            }
        }
    }
}
```

### 3.7 时间管理改造

**kernel/src/rtos/time.rs**

```rust
use core::sync::atomic::{AtomicU64, Ordering};

static SYSTEM_TIME_NS: AtomicU64 = AtomicU64::new(0);
static TICK_PERIOD_NS: u64 = 1_000_000; // 1ms

pub fn init_time_manager() {
    // 初始化系统时钟
}

pub fn get_system_time_ns() -> u64 {
    SYSTEM_TIME_NS.load(Ordering::Acquire)
}

#[interrupt]
fn system_tick_handler() {
    let mut scheduler = RTOSScheduler::global();
    
    // 更新系统时间
    SYSTEM_TIME_NS.fetch_add(TICK_PERIOD_NS, Ordering::Release);
    
    // 更新当前任务时间片
    if let Some(task_id) = scheduler.current_task[0] {
        if let Some(task) = scheduler.tasks.get(&task_id) {
            let remaining = task.time_slice_remaining.fetch_sub(1, Ordering::Relaxed);
            if remaining == 1 {
                // 时间片耗尽
                task.set_state(TaskState::Ready);
                scheduler.ready_queue.enqueue(task.get_priority(), task_id);
                
                if let Some(next_id) = scheduler.schedule_next(0) {
                    scheduler.context_switch(0, next_id);
                }
            }
        }
    }
    
    // 检查截止时间
    check_deadlines(&mut scheduler);
    
    // 重置 CPU 配额（每 10ms）
    let now = get_system_time_ns();
    if now % 10_000_000 < TICK_PERIOD_NS {
        reset_cpu_quotas(&mut scheduler);
    }
}

fn check_deadlines(scheduler: &mut RTOSScheduler) {
    let now = get_system_time_ns();
    
    for (_, task) in &mut scheduler.tasks {
        if let Some(deadline) = task.deadline_ns {
            if now > deadline {
                log::warn!("Task {} missed deadline", task.id);
                task.deadline_miss_count.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn reset_cpu_quotas(scheduler: &mut RTOSScheduler) {
    for (_, task) in &mut scheduler.tasks {
        if task.cpu_quota_us.is_some() {
            task.cpu_used_us.store(0, Ordering::Relaxed);
            task.deadline_ns = Some(get_system_time_ns() + 100_000_000);
        }
    }
}
```

### 3.8 死锁检测

**kernel/src/rtos/deadlock.rs**

```rust
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use crate::rtos::TaskId;

pub struct DeadlockDetector {
    wait_graph: BTreeMap<TaskId, Vec<TaskId>>,
}

impl DeadlockDetector {
    pub fn new() -> Self {
        Self {
            wait_graph: BTreeMap::new(),
        }
    }

    pub fn update_wait(&mut self, waiter: TaskId, held_by: TaskId) {
        self.wait_graph
            .entry(waiter)
            .or_insert_with(Vec::new)
            .push(held_by);
        
        if self.has_cycle(waiter) {
            log::warn!("Deadlock detected involving task {}", waiter);
            self.handle_deadlock(waiter);
        }
    }

    pub fn remove_wait(&mut self, waiter: TaskId, held_by: TaskId) {
        if let Some(deps) = self.wait_graph.get_mut(&waiter) {
            deps.retain(|&id| id != held_by);
        }
    }

    fn has_cycle(&self, start: TaskId) -> bool {
        let mut visited = BTreeSet::new();
        let mut rec_stack = BTreeSet::new();
        self.dfs(start, &mut visited, &mut rec_stack)
    }

    fn dfs(
        &self,
        task: TaskId,
        visited: &mut BTreeSet<TaskId>,
        rec_stack: &mut BTreeSet<TaskId>,
    ) -> bool {
        if !visited.contains(&task) {
            visited.insert(task);
            rec_stack.insert(task);

            if let Some(deps) = self.wait_graph.get(&task) {
                for &dep in deps {
                    if !visited.contains(&dep) {
                        if self.dfs(dep, visited, rec_stack) {
                            return true;
                        }
                    } else if rec_stack.contains(&dep) {
                        return true;
                    }
                }
            }
        }
        rec_stack.remove(&task);
        false
    }

    fn handle_deadlock(&self, task: TaskId) {
        log::error!("Deadlock detected at task {}", task);
        // 可选：panic!() 或尝试恢复
    }
}
```

---

## 四、修改现有文件

### 4.1 修改 VCpu 任务创建

**kernel/src/vmm/vcpus.rs**

```rust
// 添加 feature 开关
#[cfg(feature = "rtos-scheduler")]
use crate::rtos::{create_vcpu_task, RTOSScheduler};

#[cfg(not(feature = "rtos-scheduler"))]
fn alloc_vcpu_task(vm: &VMRef, vcpu: VCpuRef) -> AxTaskRef {
    // 原有实现
    let mut vcpu_task = TaskInner::new(vcpu_run, ...);
    axtask::spawn_task(vcpu_task)
}

#[cfg(feature = "rtos-scheduler")]
fn alloc_vcpu_task(vm: &VMRef, vcpu: VCpuRef) -> AxTaskRef {
    // 使用 RTOS 调度器
    let vm_config = vm.get_config();
    let priority = vm_config.priority.unwrap_or(0);
    let cpu_quota = vm_config.cpu_quota_ms.unwrap_or(50);
    
    let task_id = create_vcpu_task(vm, vcpu, priority, cpu_quota);
    
    // 返回兼容的句柄
    AxTaskRef::from_rtos_task_id(task_id)
}
```

### 4.2 修改同步机制

**kernel/src/vmm/vcpus.rs (续)**

```rust
#[cfg(feature = "rtos-scheduler")]
use crate::rtos::{Mutex, Semaphore};

pub struct VMVCpus {
    _vm_id: usize,
    
    #[cfg(feature = "rtos-scheduler")]
    wait_mutex: Mutex,
    #[cfg(feature = "rtos-scheduler")]
    wait_sem: Semaphore,
    
    #[cfg(not(feature = "rtos-scheduler"))]
    wait_queue: WaitQueue,
    
    vcpu_task_list: Vec<AxTaskRef>,
    running_halting_vcpu_count: AtomicUsize,
}

#[cfg(feature = "rtos-scheduler")]
fn wait(vm_id: usize) {
    let vm_vcpus = VM_VCPU_TASK_WAIT_QUEUE.get(&vm_id).unwrap();
    let scheduler = RTOSScheduler::global();
    let current_id = scheduler.current_task[0].unwrap();
    
    vm_vcpus.wait_sem.wait(current_id, scheduler);
}

#[cfg(feature = "rtos-scheduler")]
pub fn notify_primary_vcpu(vm_id: usize) {
    if let Some(vm_vcpus) = VM_VCPU_TASK_WAIT_QUEUE.get_mut(&vm_id) {
        let scheduler = RTOSScheduler::global();
        vm_vcpus.wait_sem.post(scheduler);
    }
}
```

### 4.3 修改主循环

**kernel/src/vmm/mod.rs**

```rust
#[cfg(feature = "rtos-scheduler")]
use crate::rtos::{init_rtos, RTOSScheduler};

pub fn init() {
    config::init_guest_vms();
    
    #[cfg(feature = "rtos-scheduler")]
    init_rtos();
    
    for vm in vm_list::get_vm_list() {
        vcpus::setup_vm_primary_vcpu(&vm);
    }
}

pub fn start() {
    info!("VMM starting, booting VMs...");
    
    for vm in vm_list::get_vm_list() {
        match vm.boot() {
            Ok(_) => {
                vcpus::notify_primary_vcpu(vm.id());
                RUNNING_VM_COUNT.fetch_add(1, Ordering::Release);
            }
            Err(err) => warn!("VM[{}] boot failed: {:?}", vm.id(), err),
        }
    }

    #[cfg(feature = "rtos-scheduler")]
    {
        // RTOS 调度器会接管主循环
        RTOSScheduler::global().run_main_loop();
    }

    #[cfg(not(feature = "rtos-scheduler"))]
    {
        // 原有实现
        task::ax_wait_queue_wait_until(&VMM, || {
            RUNNING_VM_COUNT.load(Ordering::Acquire) == 0
        }, None);
    }
}
```

---

## 五、编译配置

### 5.1 Cargo.toml 修改

**kernel/Cargo.toml**

```toml
[features]
default = ["arceos-scheduler"]
rtos-scheduler = []
arceos-scheduler = []

[[bench]]
name = "scheduler_latency"
harness = false

[[test]]
name = "rtos_integration"
```

### 5.2 构建命令

```bash
# 使用 ArceOS 调度器（默认）
cargo build

# 使用 RTOS 调度器
cargo build --features rtos-scheduler

# 运行测试
cargo test --lib rtos:: --features rtos-scheduler

# 运行集成测试
cargo test --test rtos_integration --features rtos-scheduler

# 运行性能基准测试
cargo bench --bench scheduler_latency --features rtos-scheduler
```

---

## 六、测试和验证

### 6.1 单元测试

```rust
// kernel/src/rtos/tests.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_queue() {
        let mut queue = ReadyQueue::new();
        
        queue.enqueue(100, 1);
        queue.enqueue(50, 2);
        queue.enqueue(200, 3);
        
        assert_eq!(queue.dequeue(), Some(2));  // priority 50
        assert_eq!(queue.dequeue(), Some(1));  // priority 100
        assert_eq!(queue.dequeue(), Some(3));  // priority 200
    }

    #[test]
    fn test_mutex_priority_inheritance() {
        RTOSScheduler::init();
        let mut sched = RTOSScheduler::global();
        
        let mutex = Mutex::new(0);
        let t1 = sched.create_task("low".into(), || {}, 100, 4096);
        let t2 = sched.create_task("high".into(), || {}, 50, 4096);
        
        mutex.lock(t1);
        // t2 尝试获取锁，应该触发优先级继承
        // ...
    }
}
```

### 6.2 集成测试

```bash
# 1. 单 VM 启动
./task.py run --vm-config configs/vms/arceos-aarch64-qemu-smp1.toml

# 2. 多 VM 启动
./task.py run --vm-configs "vm1.toml,vm2.toml"

# 3. VM 暂停/恢复
vm pause 0
vm resume 0

# 4. VM 关闭
vm delete 0

# 5. 压力测试
# (运行会产生大量 VM Exit 的 Guest 程序)
```

### 6.3 性能测试

```rust
// benches/scheduler_latency.rs

#![feature(test)]
extern crate test;

use test::Bencher;

#[bench]
fn bench_schedule_latency(b: &mut Bencher) {
    RTOSScheduler::init();
    let mut sched = RTOSScheduler::global();
    
    // 创建 100 个任务
    for i in 0..100 {
        sched.create_task(format!("task_{}", i), || {}, i, 4096);
    }
    
    b.iter(|| {
        sched.schedule_next(0);
    });
}
```

---

## 七、实施路线图

### Phase 1: 基础架构（第 1-2 周）
- [ ] 创建 rtos 模块结构
- [ ] 实现优先级位图队列
- [ ] 实现任务和调度器基类
- [ ] 单元测试覆盖 > 80%

### Phase 2: 同步原语（第 3-4 周）
- [ ] 实现互斥锁 + 优先级继承
- [ ] 实现信号量
- [ ] 实现条件变量
- [ ] 集成测试

### Phase 3: 高级功能（第 5-6 周）
- [ ] 实现时间管理（WCET、截止时间）
- [ ] 实现死锁检测
- [ ] 性能分析工具
- [ ] 压力测试

### Phase 4: VCpu 集成（第 7-8 周）
- [ ] 改造 VCpu 任务为 RTOS 任务
- [ ] CPU 配额管理
- [ ] VM 优先级配置
- [ ] 系统集成测试

**总计**：8 周

---

## 八、性能目标

| 操作 | 目标延迟 | 实现难度 |
|------|--------|--------|
| 调度延迟 | < 100 μs | 中 |
| 上下文切换 | < 500 ns | 高 |
| 锁获取 | < 50 μs | 中 |
| 信号量 post | < 10 μs | 低 |
| 死锁检测 | < 1 ms | 高 |

---

## 九、风险和缓解措施

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| **性能下降** | 高 | 充分的性能测试和优化 |
| **兼容性问题** | 中 | 使用 feature 开关，保留原调度器 |
| **死锁** | 高 | 实现死锁检测和恢复 |
| **优先级反转** | 中 | 实现优先级继承 |
| **时间片配置** | 低 | 提供合理的默认值和配置接口 |

---

## 十、总结

本改造方案将 AxVisor 从 ArceOS 混合调度模型转换为完整的 RTOS 调度系统，主要改进包括：

1. **实时性保证**：优先级管理、时间片配额、截止时间监控
2. **资源隔离**：VM 间 CPU 配额管理、优先级隔离
3. **可靠性增强**：优先级继承、死锁检测、WCET 监控
4. **可配置性**：支持多种调度策略、灵活的 VM 优先级配置

通过分阶段实施，可以在 8 周内完成改造，并通过 feature 开关保持向后兼容性。
