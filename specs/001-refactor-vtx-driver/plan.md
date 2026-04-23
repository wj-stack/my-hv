# Implementation Plan: Rust 重构 VT-x 虚拟化驱动

**Branch**: `001-refactor-vtx-driver` | **Date**: 2026-04-23 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/001-refactor-vtx-driver/spec.md`

**Note**: This template is filled in by the `/speckit.plan` command. See `.specify/templates/plan-template.md` for the execution workflow.

## Summary

将 `./hv` 中的 C++ VT-x 驱动重构为 Rust 实现，并以 VMCALL 超级调用为核心重新设计对外接口契约（不保留旧接口约束）。覆盖现有虚拟化能力、VM-exit 处理、EPT 机制、日志与内存操作等能力，并补齐最小验收验证路径与用户态工具说明。

## Technical Context

<!--
  ACTION REQUIRED: Replace the content in this section with the technical details
  for the project. The structure here is presented in advisory capacity to guide
  the iteration process.
-->

**Language/Version**: Rust 2024 edition（no_std 内核驱动）  
**Primary Dependencies**: wdk / wdk-sys / wdk-build / wdk-alloc / wdk-panic / shared-contract  
**Storage**: N/A（内核内存态数据结构）  
**Testing**: `build.bat` 构建 + 目标硬件上的驱动加载/卸载与 VMCALL 调用验证  
**Target Platform**: Windows 10/11 x64 内核驱动（WDM），需要 Intel VT-x  
**Project Type**: Windows 内核驱动 + 用户态客户端/管理工具  
**Performance Goals**: 关键虚拟化操作平均耗时相对旧驱动偏差 ≤ 5%  
**Constraints**: 必须通过 `build.bat` 构建；不新增功能；支持现有目标硬件与 OS 范围  
**Scale/Scope**: 单一驱动替换 `./hv` 现有实现，覆盖核心虚拟化生命周期

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

未发现可执行的宪章原则（当前为占位模板）。无强制门禁，视为通过；后续完善宪章后需要复核。

## Project Structure

### Documentation (this feature)

```text
specs/001-refactor-vtx-driver/
├── plan.md              # This file (/speckit.plan command output)
├── research.md          # Phase 0 output (/speckit.plan command)
├── data-model.md        # Phase 1 output (/speckit.plan command)
├── quickstart.md        # Phase 1 output (/speckit.plan command)
├── contracts/           # Phase 1 output (/speckit.plan command)
└── tasks.md             # Phase 2 output (/speckit.tasks command - NOT created by /speckit.plan)
```

### Source Code (repository root)
<!--
  ACTION REQUIRED: Replace the placeholder tree below with the concrete layout
  for this feature. Delete unused options and expand the chosen structure with
  real paths (e.g., apps/admin, packages/something). The delivered plan must
  not include Option labels.
-->

```text
hv/                 # 现有 C++ VT-x 驱动实现与用户态工具（参考）
driver/             # Rust 驱动实现（WDM）
client/             # Rust 用户态工具（可选）
shared-contract/    # 新接口契约与常量
build.bat           # 统一构建入口
Cargo.toml
```

**Structure Decision**: 采用多 crate 工作区结构，`driver` 作为 Rust 内核驱动实现，`shared-contract` 记录新的 VMCALL 契约；`client` 或 `hv/um` 作为用户态工具实现验证路径；`hv` 保留为旧实现参考。

## Complexity Tracking

宪章未定义可执行门禁，本次无复杂度违规需要记录。
