# Phase 0 Research: Rust 重构 VT-x 虚拟化驱动

## Decision 1: 内核驱动 Rust 技术栈
- **Decision**: 使用现有工作区的 Rust 2024 + `wdk` 系列 crate 作为内核驱动基础（WDM）。
- **Rationale**: 仓库已配置 `wdk`/`wdk-sys`/`wdk-build`/`wdk-alloc`/`wdk-panic`，避免重复引入新依赖；符合 no_std 与内核驱动构建需求。
- **Alternatives considered**: 继续 C++（不满足重构目标）；采用 KMDF 或第三方框架（会偏离现有模板与构建链）。

## Decision 2: 对外接口重新设计与契约表达
- **Decision**: 在 `shared-contract` 中定义新的设备标识、IOCTL 语义与版本号，并在 `contracts/` 文档中描述请求/响应语义与错误范围。
- **Rationale**: 用户选择“重新设计接口”，需要可演进的契约与清晰的语义描述；`shared-contract` 便于内核与用户态共享常量。
- **Alternatives considered**: 继续复用旧 IOCTL（与“重新设计接口”冲突）；仅在代码中隐式定义（缺乏可验证契约）。

## Decision 3: 最小验收测试集合
- **Decision**: 定义最小验收为：`build.bat` 构建通过；驱动加载/卸载成功；基础 IOCTL 连通测试；支持 VT-x 的机器上完成启动与停止；不支持 VT-x 时给出可识别错误。
- **Rationale**: 用户选择“无测试资产”，需从零给出可执行最小集合，确保替换安全。
- **Alternatives considered**: 仅依赖运行时人工观察（不可重复）；等到实现阶段再补测试（不可控风险）。

## Decision 4: 性能与稳定性验证方式
- **Decision**: 以旧驱动作为基准，对关键虚拟化操作做平均耗时对比（偏差 ≤ 5%），并进行 20 次启动/停止循环稳定性验证。
- **Rationale**: 与规格的成功标准一致且可量化。
- **Alternatives considered**: 仅给出定性指标（不可验收）。
