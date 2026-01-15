# 主流 RTOS 调度器设计要点

## 一、RTOS 调度的核心功能点

### 1.1 优先级管理

#### 优先级模型

```
优先级范围：0-255（0 最高，255 最低）
┌─────────────────────────────────┐
│  优先级 0-31   │ 中断处理       │ 最高
├─────────────────────────────────┤
│  优先级 32-63  │ 系统任务       │
├─────────────────────────────────┤
│  优先级 64-191 │ 应用任务       │
├─────────────────────────────────┤
│  优先级 192-223│ VCpu 任务      │ <- AxVisor VCpu 范围
├─────────────────────────────────┤
│  优先级 224-254│ 空闲任务       │
├─────────────────────────────────┤
│  优先级 255    │ 空闲轮询       │ 最低
└─────────────────────────────────┘

static const u8 VCPU_PRIORITY_BASE = 192;  // VCpu 优先级基数
```

#### 优先级继承

```
互斥锁持有期间，任务优先级临时提升至阻塞者的优先级
用于解决优先级反转问题

场景：
L_task(低优先级) 持有 mutex
H_task(高优先级) 等待 mutex
  -> L_task 优先级临时升至 H_task 的优先级，完成工作后恢复

bool mutex_lock_with_inheritance(mutex_t *m, task_t *task);
task_t *current_task = get_current_task();
u8 original_priority = current_task->priority;
current_task->priority = max(current_task->priority, waiting_task->priority);
```

**实现要点**：
- 锁获取时检查优先级
- 临时提升锁持有者优先级
- 锁释放时恢复原始优先级
- 支持嵌套锁的优先级传播

### 1.2 任务状态管理

#### 完整状态机

```
         ┌─────────────┐
    ┌───→│   Ready     │◄────────┐
    │    └─────┬───────┘         │
    │          │ 调度选中        │
    │          ▼                 │
    │    ┌─────────────┐         │
    ├────│   Running   │─────────┤
    │    └─────┬───────┘         │
    │          │                 │
    │     ┌────┴─────┬─────┬─────┬──────┐
    │     │          │     │     │      │
    │     ▼          ▼     ▼     ▼      ▼
    │  Blocked   Preempted Yield Suspended Terminated
    │     │          │     │     │
    │     └────┬─────┴─────┘     │
    └─────────┘                  │
              Ready list <────────┘ (VM Resume)
```

#### 状态转移表

| 源状态 | 事件 | 目标状态 | 触发条件 |
|--------|------|--------|--------|
| Ready | 调度器选中 | Running | CPU 时间片可用 |
| Running | 时间片耗尽 | Ready | 发生时钟中断 |
| Running | 等待资源 | Blocked | 调用阻塞系统调用 |
| Running | 高优先级就绪 | Preempted | 有更高优先级任务就绪 |
| Blocked | 资源就绪 | Ready | 信号量/锁被释放 |
| Running | VM Suspend | Suspended | VM 暂停命令 |
| Suspended | VM Resume | Ready | VM 恢复命令 |
| Running | exit() | Terminated | 任务完成或被杀死 |

### 1.3 优先级队列管理

#### 位图快速查找算法

```rust
struct RTOSScheduler {
    // 优先级位图（支持快速查找最高优先级）
    ready_bitmap: u32,          // 每一位代表一个优先级组
    ready_groups: [u32; 8],     // 8 组，共 256 个优先级
    
    // 优先级队列（每个优先级一个 FIFO 队列）
    ready_queues: [VecDeque<TaskHandle>; 256],
    
    // 当前运行的任务（每个 CPU 一个）
    current_tasks: [Option<TaskHandle>; MAX_CPUS],
    
    // 全局任务表
    tasks: BTreeMap<TaskId, Task>,
    
    // 同步原语（锁等）
    locks: BTreeMap<LockId, Lock>,
}

// 快速查找最高优先级就绪任务
fn get_highest_priority_ready_task(&self) -> Option<TaskHandle> {
    // 找到位图中最高的 1 位
    let group = leading_zeros(self.ready_bitmap);  // O(1)
    let priority = find_highest_bit(self.ready_groups[group]);  // O(1)
    self.ready_queues[priority].pop_front()  // O(1)
}
```

**时间复杂度**：
- 入队：O(1)
- 出队：O(1)
- 查找最高优先级：O(1)

### 1.4 时间管理

#### 时钟驱动调度

```rust
// 系统时钟周期（通常 1ms）
const TICK_PERIOD_MS: u64 = 1;
const TICK_PERIOD_NS: u64 = TICK_PERIOD_MS * 1_000_000;

struct Task {
    priority: u8,
    time_slice_ticks: u16,          // 时间片（单位：tick）
    time_slice_remaining: u16,      // 剩余时间片
    deadline: Option<u64>,          // 绝对截止时间（纳秒）
    period: Option<u64>,            // 周期（对于周期任务）
    last_schedule_time: u64,        // 上次调度的系统时间
}

// 时钟中断处理
#[interrupt]
fn system_tick_handler() {
    current_task.time_slice_remaining -= 1;
    
    if current_task.time_slice_remaining == 0 {
        // 时间片耗尽，触发重新调度
        trigger_reschedule();
    }
    
    // 检查任务截止时间
    check_deadlines();
    
    // 唤醒周期任务
    wake_periodic_tasks();
}
```

#### 截止时间监控

```rust
fn check_deadlines() {
    let now = get_system_time();
    for (task_id, task) in &mut tasks {
        if let Some(deadline) = task.deadline {
            if now > deadline {
                panic!("Task {} missed deadline!", task_id);  // 硬实时
                // 或记录违反
                task.deadline_miss_count += 1;  // 软实时
            }
        }
    }
}
```

### 1.5 同步原语

#### 信号量（Semaphore）

```rust
pub struct Semaphore {
    count: AtomicI32,
    max_count: i32,
    waiting_tasks: VecDeque<TaskId>,
}

pub fn sem_wait(sem: &mut Semaphore) -> Result<()> {
    loop {
        if sem.count > 0 {
            sem.count -= 1;
            return Ok(());
        }
        // 阻塞当前任务
        let current = current_task();
        sem.waiting_tasks.push_back(current.id);
        current.state = TaskState::Blocked;
        trigger_reschedule();  // 让出 CPU
    }
}

pub fn sem_post(sem: &mut Semaphore) {
    sem.count += 1;
    if let Some(task_id) = sem.waiting_tasks.pop_front() {
        // 唤醒等待的任务
        if let Some(task) = get_task_mut(task_id) {
            task.state = TaskState::Ready;
            add_to_ready_queue(task);
        }
        trigger_reschedule();  // 如果唤醒的任务更高优先级
    }
}
```

#### 互斥锁（Mutex with Priority Inheritance）

```rust
pub struct Mutex {
    owner: Option<TaskId>,
    count: usize,  // 递归锁计数
    waiting_tasks: VecDeque<TaskId>,
    original_priority: Option<u8>,  // 用于恢复优先级
}

pub fn mutex_lock(mutex: &mut Mutex) -> Result<()> {
    let current = current_task();
    
    loop {
        if mutex.owner.is_none() {
            // 锁可用，获取所有权
            mutex.owner = Some(current.id);
            mutex.count = 1;
            return Ok(());
        }
        
        if mutex.owner == Some(current.id) {
            // 递归锁
            mutex.count += 1;
            return Ok(());
        }
        
        // 锁被占有，检查是否需要优先级继承
        if let Some(owner_id) = mutex.owner {
            if let Some(owner) = get_task_mut(owner_id) {
                // 优先级继承
                if current.priority < owner.priority {
                    if mutex.original_priority.is_none() {
                        mutex.original_priority = Some(owner.priority);
                    }
                    owner.priority = current.priority;
                    reschedule_if_needed();
                }
            }
        }
        
        // 等待锁释放
        mutex.waiting_tasks.push_back(current.id);
        current.state = TaskState::Blocked;
        trigger_reschedule();
    }
}
```

#### 条件变量（Condition Variable）

```rust
pub struct ConditionVariable {
    waiting_tasks: VecDeque<TaskId>,
    associated_mutex: Option<MutexId>,
}

pub fn cond_wait(cond: &mut ConditionVariable, mutex: &mut Mutex) -> Result<()> {
    // 1. 释放互斥锁
    mutex_unlock(mutex)?;
    
    // 2. 等待信号
    let current = current_task();
    cond.waiting_tasks.push_back(current.id);
    current.state = TaskState::Blocked;
    trigger_reschedule();
    
    // 3. 被唤醒后重新获取锁
    mutex_lock(mutex)?;
    Ok(())
}

pub fn cond_signal(cond: &mut ConditionVariable) {
    if let Some(task_id) = cond.waiting_tasks.pop_front() {
        if let Some(task) = get_task_mut(task_id) {
            task.state = TaskState::Ready;
            add_to_ready_queue(task);
        }
    }
}

pub fn cond_broadcast(cond: &mut ConditionVariable) {
    while let Some(task_id) = cond.waiting_tasks.pop_front() {
        if let Some(task) = get_task_mut(task_id) {
            task.state = TaskState::Ready;
            add_to_ready_queue(task);
        }
    }
}
```

### 1.6 实时性保证

#### 最坏情况执行时间 (WCET)

```rust
struct Task {
    wcet_us: u64,  // 最坏情况执行时间（微秒）
    executed_time: u64,  // 当前执行周期已执行的时间
}

// 监控 WCET 超溢
fn check_wcet_overflow() {
    let current = current_task();
    if current.executed_time > current.wcet_us {
        warn!("Task {} exceeded WCET: {} > {}", 
              current.id, current.executed_time, current.wcet_us);
        // 可选：杀死任务或记录日志
        current.state = TaskState::Terminated;
        trigger_reschedule();
    }
}
```

#### 死锁检测

```rust
struct DeadlockDetector {
    wait_graph: BTreeMap<TaskId, Vec<TaskId>>,  // 等待关系图
}

fn detect_deadlock() -> Vec<Vec<TaskId>> {
    // 检测有向图中的环
    let mut deadlock_cycles = Vec::new();
    
    for start_task in wait_graph.keys() {
        if has_cycle_from(start_task, &wait_graph) {
            deadlock_cycles.push(find_cycle(start_task, &wait_graph));
        }
    }
    
    deadlock_cycles
}

fn update_wait_graph(waiting_task: TaskId, held_by: TaskId) {
    wait_graph.entry(waiting_task).or_insert_with(Vec::new).push(held_by);
    
    if detect_deadlock().len() > 0 {
        warn!("Deadlock detected!");
        // 采取措施：
        // - 记录日志
        // - 终止涉及的任务
        // - 触发软件中断（不能恢复的错误）
    }
}
```

### 1.7 任务组和分组调度

```rust
// 支持任务组，用于实现 VM 隔离
struct TaskGroup {
    group_id: usize,
    priority: u8,
    cpu_quota: u32,  // CPU 时间配额（微秒/周期）
    cpu_used: u32,   // 已使用的 CPU 时间
    tasks: Vec<TaskId>,  // 组内所有任务
}

struct RTOSScheduler {
    // 组级调度
    task_groups: BTreeMap<usize, TaskGroup>,
    group_ready_queue: VecDeque<usize>,  // 按优先级排序的组队列
}

// 分两级调度
fn schedule_next_task() -> TaskHandle {
    // 第一级：选择最高优先级的任务组
    while let Some(group_id) = group_ready_queue.front() {
        let group = &task_groups[group_id];
        
        // 检查组的 CPU 配额
        if group.cpu_used < group.cpu_quota {
            // 第二级：从组内选择任务
            if let Some(task) = select_task_from_group(group) {
                return task;
            }
        }
    }
    
    // 所有组都用尽配额，选择空闲任务
    return idle_task;
}
```

---

## 二、三大主流 RTOS 对比分析

### 2.1 Zephyr RTOS

#### 核心特点

**调度算法**：
- 支持多种调度策略：抢占式优先级、协作式、时间片轮转
- 优先级范围：0-31（0 最高，可配置到 998）
- 支持元调度（Metascheduling）- 动态切换调度策略

**关键数据结构**：
```c
struct k_thread {
    struct _thread_base base;
    char *stack_info;
    void *custom_data;
    struct k_poll_event *events;
    uint32_t swap_retval;
    // 优先级、状态、栈指针等
};

enum k_thread_state {
    _THREAD_DUMMY,    /* 未初始化 */
    _THREAD_PRESTART, /* 启动前 */
    _THREAD_READY,    /* 就绪 */
    _THREAD_RUNNING,  /* 运行中 */
    _THREAD_PENDING,  /* 待处理 */
    _THREAD_QUEUED    /* 队列中 */
};
```

**同步原语**：
- 互斥锁：支持优先级继承
- 信号量：支持计数和二进制
- 条件变量：标准 POSIX 风格
- 工作队列：延迟任务处理

**内存管理**：
- 静态内存池（K_MEM_POOL）
- 内存 slab 分配器
- 支持 C++ 新建/删除

**优点**：
- 安全认证（IEC 61508、ISO 26262、EU GDPR）
- 丰富的设备驱动和协议栈
- 模块化设计，易于裁剪
- 活跃的社区和厂商支持

**缺点**：
- 学习曲线陡峭
- 配置复杂度高
- 内存占用相对较大

### 2.2 RT-Thread

#### 核心特点

**调度算法**：
- 抢占式优先级调度
- 优先级范围：0-31（0 最高，可配置到 256）
- 支持时间片轮转（同优先级任务）
- 支持位图调度算法（O(1) 查找）

**关键数据结构**：
```c
struct rt_thread {
    char        name[RT_NAME_MAX];  /* 线程名称 */
    uint8_t     type;               /* 线程类型 */
    uint8_t     flags;              /* 线程标志 */
    rt_list_t   list;               /* 线程链表 */
    rt_list_t   tlist;              /* 线程全局链表 */
    void       *sp;                 /* 线程栈指针 */
    void       *entry;              /* 线程入口函数 */
    void       *parameter;          /* 线程参数 */
    void       *stack_addr;         /* 线程栈地址 */
    uint32_t    stack_size;         /* 线程栈大小 */
    uint8_t     priority;           /* 线程优先级 */
    uint32_t    init_priority;      /* 线程初始优先级 */
    uint32_t    number_mask;        /* 线程掩码 */
    rt_tick_t   init_tick;          /* 线程初始时间片 */
    rt_tick_t   remaining_tick;     /* 线程剩余时间片 */
    
    struct rt_thread *cleanup;      /* 清理函数 */
    rt_tick_t   user_time;          /* 用户时间 */
    rt_tick_t   system_time;        /* 系统时间 */
};
```

**同步原语**：
- 互斥锁：支持优先级继承
- 信号量：支持计数和二进制
- 事件集：多事件同步
- 邮箱和消息队列
- 读写锁

**内存管理**：
- 小内存管理算法（memheap）
- SLAB 内存分配器
- 内存池管理

**设备模型**：
- 设备驱动框架（Device Model）
- 虚拟文件系统（VFS）
- TCP/IP 协议栈（LwIP）
- 命令行 finsh

**优点**：
- 代码简洁，易于理解
- 中文文档丰富
- 低资源占用
- 丰富的中间件

**缺点**：
- 国际化程度相对较低
- 安全认证较少
- 部分功能依赖第三方库

### 2.3 FreeRTOS

#### 核心特点

**调度算法**：
- 抢占式优先级调度
- 优先级范围：0-(configMAX_PRIORITIES-1)
- 支持时间片轮转（可配置）
- 支持协程（Co-routines，轻量级任务）

**关键数据结构**：
```c
typedef struct tskTaskControlBlock {
    volatile StackType_t *pxTopOfStack;    /* 栈顶指针 */
    ListItem_t xStateListItem;              /* 状态列表项 */
    ListItem_t xEventListItem;              /* 事件列表项 */
    UBaseType_t uxPriority;                 /* 优先级 */
    StackType_t *pxStack;                   /* 栈基址 */
    uint32_t ulStackMark;                   /* 栈标记 */
    char pcTaskName[configMAX_TASK_NAME_LEN]; /* 任务名称 */
    
    /* 优先级继承相关 */
    UBaseType_t uxBasePriority;             /* 基础优先级 */
    UBaseType_t uxMutexesHeld;              /* 持有的互斥锁数量 */
    
    /* 任务通知 */
    TaskNotifyValue_t ulNotifiedValue;
    uint8_t ucNotifyState;
    
} tskTCB;
```

**同步原语**：
- 互斥锁：支持优先级继承
- 信号量：二进制、计数、互斥信号量
- 任务通知：轻量级同步机制
- 事件组
- 队列（消息传递）

**内存管理**：
- 5 种内存分配策略（heap_1 到 heap_5）
- 支持静态分配（无动态内存）
- 支持内存池

**优点**：
- 极小的内存占用（最小 3KB）
- 丰富的内核对象
- 广泛的硬件支持
- 庞大的用户社区
- MIT 许可证

**缺点**：
- 调试工具相对简单
- 高级功能需要商业支持
- 代码风格较老

---

## 三、RTOS 调度器设计要点总结

### 3.1 核心设计原则

1. **确定性**
   - 可预测的调度延迟
   - 可计算的最坏情况执行时间（WCET）
   - 明确的优先级规则

2. **实时性**
   - 快速响应中断
   - 支持硬实时和软实时
   - 截止时间监控

3. **可靠性**
   - 优先级继承防止反转
   - 死锁检测和恢复
   - 栈溢出检测

4. **效率**
   - O(1) 调度算法
   - 最小化上下文切换开销
   - 优化的同步原语

### 3.2 关键性能指标

| 指标 | Zephyr | RT-Thread | FreeRTOS | 目标值 |
|------|--------|-----------|----------|--------|
| **调度延迟** | < 10 μs | < 15 μs | < 20 μs | < 100 μs |
| **上下文切换** | < 500 ns | < 800 ns | < 1 μs | < 500 ns |
| **中断延迟** | < 5 μs | < 10 μs | < 15 μs | < 10 μs |
| **最小内存** | 8 KB | 3 KB | 3 KB | < 10 KB |
| **最大任务数** | 无限制 | 256+ | 无限制 | 256+ |

### 3.3 必备功能清单

#### 基础功能（必须）
- [x] 抢占式优先级调度
- [x] 任务状态管理（至少 6 个状态）
- [x] 时间片轮转
- [x] 互斥锁 + 优先级继承
- [x] 信号量
- [x] 任务间通信（队列/事件）

#### 高级功能（推荐）
- [ ] 条件变量
- [ ] 读写锁
- [ ] 屏障（Barrier）
- [ ] 死锁检测
- [ ] WCET 监控
- [ ] 截止时间调度
- [ ] 任务分组和配额

#### 可选功能（增强）
- [ ] 多级反馈队列（MLFQ）
- [ ] 最早截止时间优先（EDF）
- [ ] 速率单调调度（RMS）
- [ ] 能量感知调度
- [ ] 温度感知调度

### 3.4 设计权衡

| 设计选择 | 优点 | 缺点 | 适用场景 |
|---------|------|------|---------|
| **静态优先级** | 简单、可预测 | 可能饥饿 | 硬实时系统 |
| **动态优先级** | 公平性好 | 不可预测 | 通用系统 |
| **时间片轮转** | 公平 | 增加切换开销 | 分时系统 |
| **事件驱动** | 高效 | 复杂 | 低功耗系统 |
| **全局队列** | 负载均衡好 | 锁竞争 | 多核系统 |
| **每 CPU 队列** | 缓存友好 | 负载不均 | 多核系统 |

---

## 四、AxVisor RTOS 改造建议

### 4.1 优先级设计

```
AxVisor 优先级分配：
┌─────────────────────────────────┐
│  优先级 0-31   │ 中断处理       │
├─────────────────────────────────┤
│  优先级 32-63  │ 系统任务       │
├─────────────────────────────────┤
│  优先级 64-127 │ 高优先级 VM    │
├─────────────────────────────────┤
│  优先级 128-191│ 普通优先级 VM  │
├─────────────────────────────────┤
│  优先级 192-223│ VCpu 任务      │
├─────────────────────────────────┤
│  优先级 224-254│ 空闲任务       │
└─────────────────────────────────┘
```

### 4.2 同步原语选择

**必须实现**：
1. 互斥锁 + 优先级继承
2. 信号量（二进制和计数）
3. 条件变量

**可选实现**：
1. 读写锁（VM 配置读取）
2. 事件集（多事件同步）
3. 消息队列（VCpu 间通信）

### 4.3 时间管理

**时钟精度**：1ms 系统滴答
**时间片范围**：10-100 ticks（可配置）
**截止时间**：支持软实时（记录违规）
**WCET 监控**：可选，用于性能分析

### 4.4 资源隔离

**CPU 配额**：
- 每个 VM 可配置 CPU 时间配额
- 配额周期：10-100ms
- 超额使用策略：允许借用或严格限制

**内存隔离**：
- 每个 VM 独立的内存池
- 内存使用监控和限制

---

## 五、实现路线图

### Phase 1: 基础架构（1-2 周）
- [ ] 优先级位图队列
- [ ] 任务状态机
- [ ] 时间片轮转
- [ ] 基础调度器

### Phase 2: 同步原语（2-3 周）
- [ ] 互斥锁 + 优先级继承
- [ ] 信号量
- [ ] 条件变量
- [ ] 集成测试

### Phase 3: 实时性（2-3 周）
- [ ] WCET 监控
- [ ] 截止时间检查
- [ ] 死锁检测
- [ ] 性能分析工具

### Phase 4: VCpu 集成（2-3 周）
- [ ] VCpu 任务改造
- [ ] CPU 配额管理
- [ ] VM 优先级配置
- [ ] 系统测试

**总计**：7-11 周

---

## 六、参考资源

### 文档
- [Zephyr Documentation](https://docs.zephyrproject.org/)
- [RT-Thread Documentation](https://www.rt-thread.io/document/site/)
- [FreeRTOS Documentation](https://www.freertos.org/Documentation)

### 源码
- [Zephyr GitHub](https://github.com/zephyrproject-rtos/zephyr)
- [RT-Thread GitHub](https://github.com/RT-Thread/rt-thread)
- [FreeRTOS GitHub](https://github.com/FreeRTOS/FreeRTOS-Kernel)

### 论文
- "Real-Time Scheduling Algorithms: A Survey"
- "Priority Inheritance Protocols: An Approach to Real-Time Synchronization"
- "The Rate Monotonic Scheduling Algorithm"
