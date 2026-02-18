# 依赖感知的测试目标选择

## 背景与动机

AxVisor 是一个运行在多种硬件平台上的 Hypervisor，其集成测试需要在 QEMU 模拟器和真实开发板上执行。在此之前，每次代码提交（push/PR）都会触发**全部**测试配置（QEMU aarch64、QEMU x86_64、飞腾派、RK3568），即使只修改了一行文档或某个板级驱动也是如此。

这带来了两个问题：

1. **硬件资源浪费**：自托管 Runner 连接的开发板是稀缺资源，不必要的测试会阻塞其他任务。
2. **反馈延迟**：全量测试耗时长，开发者等待时间增加。

与此同时，AxVisor 采用 Cargo workspace 组织多个 crate，crate 之间存在依赖关系。当一个底层模块（如 `axruntime`）被修改时，所有依赖它的上层模块都应该被重新测试——这就是**依赖感知测试**的核心需求。

## 设计概述

### 三阶段分析流程

```
┌─────────────────────────────────────────────────────────┐
│  阶段 1：变更检测                                         │
│  git diff --name-only <base_ref>                         │
│  → 获取变更文件列表                                       │
│  → 过滤非代码文件（文档、图片等）                           │
└─────────────────────┬───────────────────────────────────┘
                      ▼
┌─────────────────────────────────────────────────────────┐
│  阶段 2：依赖传播                                         │
│  cargo metadata → 构建 workspace 反向依赖图                │
│  BFS 遍历 → 找出所有间接受影响的 crate                     │
└─────────────────────┬───────────────────────────────────┘
                      ▼
┌─────────────────────────────────────────────────────────┐
│  阶段 3：目标映射                                         │
│  10 条规则将受影响的 crate + 变更文件                      │
│  映射到具体的测试目标（QEMU/开发板）                        │
└─────────────────────────────────────────────────────────┘
```

### Workspace 内部依赖图

通过 `cargo metadata` 自动提取的 workspace 内部反向依赖关系：

```
axconfig     ← axruntime, axvisor
axruntime    ← axvisor
axfs         ← (axruntime 间接依赖)
driver       ← axvisor
axplat-x86-qemu-q35 ← axruntime (仅 x86_64 目标)
```

当某个 crate 被修改时，沿着反向依赖链向上传播。例如：

- 修改 `axconfig` → `axruntime` 受影响 → `axvisor` 受影响
- 修改 `driver` → `axvisor` 受影响
- 修改 `axplat-x86-qemu-q35` → `axruntime` 受影响（但这是条件编译依赖，仅 x86_64）

### 测试目标

| 目标 ID | 说明 | Runner 标签 |
|---------|------|-------------|
| `qemu_aarch64` | QEMU AArch64 模拟测试 | `[self-hosted, linux, intel]` |
| `qemu_x86_64` | QEMU x86_64 模拟测试 | `[self-hosted, linux, intel]` |
| `board_phytiumpi` | 飞腾派开发板测试 | `[self-hosted, linux, phytiumpi]` |
| `board_rk3568` | ROC-RK3568-PC 开发板测试 | `[self-hosted, linux, roc-rk3568-pc]` |

## 映射规则

分析引擎按以下 10 条规则（优先级从高到低）将变更映射到测试目标：

### 全量触发规则（返回所有目标）

| 规则 | 触发条件 | 理由 |
|------|----------|------|
| Rule 1 | 根构建配置变更：`Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml` | 依赖或工具链变更影响所有构建 |
| Rule 2 | `xtask/` 源码被**直接修改** | 构建工具变更可能影响所有构建流程 |
| Rule 3 | `axruntime` 或 `axconfig` 被**直接修改** | 核心基础模块，所有平台都依赖 |
| Rule 4 | `kernel/` 下非架构特定的代码变更（不在 `kernel/src/hal/arch/` 下） | VMM、Shell、调度等通用逻辑 |

### 精确触发规则

| 规则 | 触发条件 | 触发目标 |
|------|----------|----------|
| Rule 5 | `kernel/src/hal/arch/aarch64/` 变更 | `qemu_aarch64` + `board_phytiumpi` + `board_rk3568` |
| Rule 5 | `kernel/src/hal/arch/x86_64/` 变更 | `qemu_x86_64` |
| Rule 6 | `axplat-x86-qemu-q35` crate 受影响 | `qemu_x86_64` |
| Rule 7 | `axfs` crate 受影响 | `qemu_aarch64` + `board_phytiumpi` + `board_rk3568` |
| Rule 8 | `driver` crate 受影响 — 飞腾派相关文件 | `board_phytiumpi` |
| Rule 8 | `driver` crate 受影响 — Rockchip 相关文件 | `board_rk3568` |
| Rule 8 | `driver` crate 受影响 — 通用驱动文件 | `board_phytiumpi` + `board_rk3568` |
| Rule 9 | `.github/workflows/` 下 QEMU 相关配置 | `qemu_aarch64` + `qemu_x86_64` |
| Rule 9 | `.github/workflows/` 下 Board/UBoot 相关配置 | `board_phytiumpi` + `board_rk3568` |
| Rule 10 | `configs/board/` 或 `configs/vms/` 下的配置文件 | 对应的特定目标 |

### 跳过规则

以下文件变更不触发任何测试（`skip_all=true`）：

- `doc/` 目录下的文件
- `*.md`、`*.txt`、`*.png`、`*.jpg`、`*.svg` 等
- `LICENSE`、`.gitignore`、`.gitattributes`

### 关于"直接修改"与"间接受影响"的区分

Rule 2 和 Rule 3 特意使用"直接修改的 crate"（`changed_crates`）而非"所有受影响的 crate"（`affected_crates`）进行判断。这是因为 `cargo metadata` 的依赖解析不区分条件编译依赖（`[target.'cfg(...)'.dependencies]`）。例如 `axruntime` 对 `axplat-x86-qemu-q35` 的依赖仅在 x86_64 目标下生效，但 `cargo metadata` 会无条件地将其包含在依赖图中。如果不区分，修改 x86 平台 crate 就会通过 `axruntime` 间接触发全量测试。

## 文件变更清单

| 文件 | 变更类型 | 说明 |
|------|----------|------|
| `xtask/src/affected.rs` | 新增 | 核心分析引擎（约 400 行） |
| `xtask/src/main.rs` | 修改 | 注册 `Affected` 子命令 |
| `.github/workflows/test-qemu.yml` | 修改 | 添加 `detect` job，动态构建测试矩阵 |
| `.github/workflows/test-board.yml` | 修改 | 添加 `detect` job，动态构建测试矩阵 |

## CI 工作流变更

### 改动前

```
push/PR → test-qemu job (固定 3 个矩阵项) → 全部在 self-hosted Runner 上执行
push/PR → test-board job (固定 4 个矩阵项) → 全部在 self-hosted Runner 上执行
```

### 改动后

```
push/PR → detect job (ubuntu-latest, 轻量级)
              │
              ├─ 分析影响范围
              ├─ 动态构建测试矩阵（仅包含受影响的目标）
              │
              └──→ test job (self-hosted Runner)
                   仅运行矩阵中的配置项
                   如果矩阵为空则整个 job 被跳过
```

`detect` job 运行在 GitHub 提供的标准 `ubuntu-latest` Runner 上，不占用稀缺的硬件 Runner 资源。通过 `actions/cache` 缓存 xtask 的编译产物，后续运行接近零开销。

## 使用方法

### 本地使用

```bash
# 对比 main 分支，查看需要运行哪些测试
cargo xtask affected --base origin/main

# 对比上一个 commit
cargo xtask affected --base HEAD~1

# 对比某个特定 commit
cargo xtask affected --base abc1234
```

输出示例：

```json
{
  "skip_all": false,
  "qemu_aarch64": true,
  "qemu_x86_64": false,
  "board_phytiumpi": false,
  "board_rk3568": false,
  "changed_crates": [
    "axvisor"
  ],
  "affected_crates": [
    "axvisor"
  ]
}
```

同时 `stderr` 会输出详细的分析过程，便于调试：

```
[affected] changed files (1):
  kernel/src/hal/arch/aarch64/api.rs
[affected] workspace crates: ["axvisor", "nop", "axconfig", ...]
[affected] reverse deps:
  axconfig ← {"axruntime", "axvisor"}
  axruntime ← {"axvisor"}
  driver ← {"axvisor"}
[affected] directly changed crates: {"axvisor"}
[affected] all affected crates:     {"axvisor"}
[affected] test scope: qemu_aarch64=true qemu_x86_64=false board_phytiumpi=false board_rk3568=false
```

### CI 中自动执行

无需手动操作。当 push 或创建 PR 时，CI 工作流会自动：

1. 运行 `detect` job 分析影响范围
2. 将分析结果写入 `$GITHUB_OUTPUT`
3. 根据结果动态构建测试矩阵
4. 仅在受影响的硬件 Runner 上执行测试

## 验证结果

以下场景已在本地通过验证：

| 场景 | 变更文件 | 结果 |
|------|----------|------|
| 只改文档 | `doc/shell.md` | `skip_all=true`，跳过所有测试 |
| 改 aarch64 HAL | `kernel/src/hal/arch/aarch64/api.rs` | QEMU aarch64 + 两块 ARM 开发板 |
| 改飞腾派驱动 | `modules/driver/src/blk/phytium.rs` | 仅飞腾派开发板 |
| 改 x86 平台 crate | `platform/x86-qemu-q35/src/lib.rs` | 仅 QEMU x86_64 |
| 改 axruntime | `modules/axruntime/src/lib.rs` | 全部测试（核心模块） |
| 改 kernel 通用代码 | `kernel/src/main.rs` | 全部测试 |
| 改 Rockchip 驱动 | `modules/driver/src/soc/rockchip/pm.rs` | 仅 RK3568 开发板 |

## 扩展指南

### 添加新的开发板

当添加新的开发板支持时，需要：

1. 在 `xtask/src/affected.rs` 的 `TestScope` 结构体中添加新的布尔字段
2. 在 `determine_targets()` 中添加对应的规则
3. 在 `run()` 中将新字段写入 `$GITHUB_OUTPUT`
4. 在 CI 工作流的 `Build board test matrix` 步骤中添加对应的矩阵项

### 添加新的 workspace crate

无需额外操作。`cargo metadata` 会自动发现新的 workspace 成员及其依赖关系。如果新 crate 是平台特定的，需要在 `determine_targets()` 中添加对应的映射规则。

### 修改规则

所有映射规则集中在 `xtask/src/affected.rs` 的 `determine_targets()` 函数中，便于统一维护。
