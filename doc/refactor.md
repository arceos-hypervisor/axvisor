# AxVisor 架构继续分析

## 原有架构

![old](old.svg)

1. 虚拟机需要大量 Arch 特定的虚拟化支持，而 axhal 统一了各 arch 接口，抹除了 arch 差异，导致每需要一个功能，都需要在 axhal 中添加对应 arch 的实现，而其他 arch 则需要添加空实现，增加了维护成本。

2. 同样的，axvcpu 也统一了 各 arch 的 vcpu 接口，导致每增加一个 vcpu 功能，都需要在 axvcpu 中添加对应 arch 的实现，增加了维护成本。

3. 全部组件依赖 `axvisor-api`, 任何组件想要使用其他组件的功能，都需要通过 `axvisor-api` 进行间接调用，而 arch 开发中，会增加 arch 相关的特有函数，这又需要在 axvcpu 或 axhal 中增加其他 arch 空实现，并修改 `axvisor-api`，进一步增加了 `axvisor-api` 修改的可能性，导致几乎任何修改，都需要修改 `axvisor-api`、`axhal`， 而修改 `axvisor-api`, `axhal`，则会导致修改所有依赖库，进而引发修改所有 `axplat` 等等几乎所有组件。

## 重构后

![new](refactor.svg)

1. 将所有架构相关（arch-specific）的实现收敛到 AxVm 模块：由 AxVm 统一负责虚拟机生命周期管理，并对外提供一致的虚拟机管理接口。AxVm 内部按架构选择对应的 VCPU 实现与地址空间实现，避免 arch-specific 代码分散在各个模块中。各架构的 VM 也不再通过 axvcpu 抹平差异，而是直接调用 arch_vcpu；由 arch_vcpu 以各自方式实现/适配 axvcpu 的能力，从而复用通用逻辑，并允许每个 ArchVm 以自己的方式组合 vcpu、vdevice、addrspace 等组件。

2. `axvisor-api` 改动：各模块不再直接依赖 `axvisor-api`，而是各自暴露最小必要接口，降低模块变更引发的全局联动修改。

3. 对 ArceOS 的依赖：行为上仅依赖 `std` 部分；虚拟化相关的特有能力通过 crate-interface 或直接依赖 HAL 层实现，尽量避免对 ArceOS 做侵入式修改。

## 深化设计axvm

![axvm](refactor-vm.svg)

### AxVm 内部设计

如图所示，`AxVm` 通过状态机管理虚拟机生命周期与资源分配。

1. **整体结构**：

 - `AxVisor` 内核维护 `vms` 容器，承载多个 `Vm` 实例。
 - `Vm` 持有静态配置（`image_loader`, `fdt`）以及共享的运行时数据 `VmData`（`Arc` 管理）。

2. **状态管理 (`VmData`)**

 - `VmData` 记录基础信息（ID、Name）与核心状态 `status: RWLock<VmMachine>`。
 - `VmMachine` 为枚举，涵盖三种状态数据：
   - **InitData**：配置、`vcpus: Vec<VCpu>`、`addrspace: Mutex<>`，启动前的准备态。
   - **VmRunData**：`start()` 时将 `addrspace` 等从 InitData `move` 进来，初始化 `v-device-manager: Mutex<>`，提供 `stop()`, `suspend()`, `is_running()` 运行时接口。
   - **StoppedData**：停止后保留统计信息 `statistic ...`。

3. **VCPU 与线程模型**

 - `VCpu` 同时包含架构相关 `arch-vcpu` 与通用 `axvcpu` 部分，持有指向 `VmData` 的 `Weak`，避免循环引用。
 - 进入运行态后，为每个 vcpu 创建线程（Thread1/Thread2...），执行 `loop fn run()`；退出时走 `Stop(drop)` 路径释放资源。

通过在状态间显式迁移资源（Move）并结合 `Arc/Weak` 引用控制，AxVm 将不同阶段的能力与数据隔离，降低跨模块耦合并确保生命周期安全。
