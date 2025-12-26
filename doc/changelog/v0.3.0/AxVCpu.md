# 重构 AxVCpu

## 1. 背景与动机

当前 axvisor 项目包含三个核心 crate：
- axvisor：顶层虚拟化与平台管理逻辑
- axvm：虚拟机（VMM / guest execution）相关实现
- axdevice：虚拟/直通设备抽象与注册

随着对多架构（如 RISC-V、AArch64、x86_64 等）与多平台（不同 SoC / 板级）的支持增加，项目内部出现以下问题：
1. arch 相关条件编译（`#[cfg(...)]`）在 axvm 与 axdevice 中大量散落，难以维护。
2. 配置项（arch、平台、虚拟设备开关、内存/核数限制等）分布于：
   - axvm（如启动参数、MMU/内存模型）
   - axdevice（设备选择、初始化）
   - axvmconfig / axvisor_api（跨 crate 传递结构）
   造成信息路径割裂。
3. 配置“流向”不清晰：谁定义 / 谁持有 / 谁消费不明确。
4. 架构耦合阻碍复用：axvm、axdevice 难以在其他宿主或测试环境中独立使用（例如单元测试 / 模拟）。
5. 虚拟设备在axdevice直接注册，造成配置需经过axvisor->axvm->axdevice，跨多个crate，重复传递。
6. 架构感知逻辑与“功能逻辑”耦合，增加新增架构或改动现有策略的风险与成本。
7. API接口不统一，同一个库中同时使用多种不同的接口风格，包括trait直接作为接口、axvisor-api外部函数和extern "C"函数。
8. axvcpu本意是设计为抹除架构间vcpu差异，但实际是各架构差异较大，在这一层抹除差异造成接口过度复杂，无法根据特性优化。
**vCPU核心功能差异：**

| 功能特性 | ARM64 | x86_64 | RISC-V | 抽象复杂度 |
|---------|-------|--------|--------|-----------|
| **虚拟化扩展** | VHE/VNCR | VMX | H-extension | ⭐⭐⭐ |
| **上下文切换** | EL2/EL1切换 | VMCS管理 | HSTATE管理 | ⭐⭐⭐ |
| **异常处理** | Syndrome解析 | Exit Reason | Exit Cause | ⭐⭐ |
| **内存管理** | Stage-2页表 | EPT/NPT | Sv-39页表 | ⭐⭐⭐ |
| **寄存器模型** | 通用+系统寄存器 | 通用+MSR | 通用+CSR | ⭐⭐ |

**虚拟中断控制器差异对比：**

| 特性 | ARM vGIC | Intel APIC | RISC-V IMSIC | 接口统一难度 |
|------|----------|------------|--------------|-------------|
| **消息传递** | SGI/ID/PPI | LAPIC/IOAPIC | MSI | ⭐⭐⭐⭐⭐ |
| **路由机制** | Affinity/Target | Fixed/Lowest | AIA | ⭐⭐⭐⭐ |
| **优先级** | 32级优先级 | 8级优先级 | 配置优先级 | ⭐⭐⭐ |
| **虚拟化支持** | vGICv2/vGICv3 | APICv | IMSIC+HVIP | ⭐⭐⭐⭐⭐ |
| **配置接口** | Distributor | MSR访问 | MMIO访问 | ⭐⭐⭐⭐ |


![dependency-problems](architecture-old.png)


## 2. 目标

| 目标类别 | 目标描述 | 衡量指标 |
|----------|----------|----------|
| 架构解耦 | axvm、axdevice 不再直接包含 arch 特定条件编译 | arch 条件编译迁移到 axvisor |
| 配置集中 | 所有外部可调配置集中在 axvisor | 单一入口（Configuration Root） |
| 接口稳定 | axvm / axdevice 通过清晰 API 接收已解析配置 | API 文档化，函数/类型变更频率降低 |
| 可测试性 | 可在无真实硬件 / 无特定 arch 条件下运行核心逻辑测试 | CI 引入“generic host” profile |
| 可扩展性 | 支持新增 arch / 虚拟设备时最小化侵入 | 新增 arch 时仅修改 axvisor 层及少量 adapter |

## 3. 优化方案

![new-architecture](layout.svg)

新的设计理念：从"vCPU层抹除差异"转变为"VM层抹除差异，vCPU求同存异"。核心相似功能抽象，架构特性独立，最小公约的函数组合，最终在VM层抹除架构差异

### 1. 单一配置源（Single Source of Truth）

所有运行参数（arch、平台、设备编排、资源限制等）在 axvisor 中构建并冻结。

### 2. 明确数据流方向

配置只“下行”到 axvm、axdevice；运行期状态可“上行”以供监控。

### 3. Arch 隔离, 模块化设计

arch 专属代码集中于 axvm::arch::*。

`common` 提供跨架构通用逻辑、模块与接口定义。

每个架构实现其特定的 VCPU、内存模型、中断注入等，组合`common`中的模块，如 `device`、`addrspace` 等。最终通过 `trait ArchVm` 适配器暴露统一接口供上层调用。从而实现增加或修改某个架构时不影响其他架构。

### 4. `AxVm` 通过状态机管理虚拟机生命周期与资源分配

- `AxVisor` 内核维护 `vmm` 容器，承载多个 `Vm` 实例。
- `VmData` 记录基础信息（ID、Name）与状态机 `VmMachineState`。
- `VmMachineState` 为枚举，涵盖状态数据：

```rust
pub enum VmMachineState {
    Uninit(VmMachineUninit),
    Inited(VmMachineInited),
    Running(VmMachineRunning),
    Switching,
    #[allow(unused)]
    Stopping(VmStatusStopping),
    Stopped,
}
```

确保不混淆相应状态的数据与行为。状态转换通过 `Switching` 中间态进行，用 `Switching` 换出前一状态，使数据所有权可以 `move` 到下一状态。
