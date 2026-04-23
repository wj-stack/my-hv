# Phase 1 Data Model: Rust 重构 VT-x 虚拟化驱动

## Entities

### 虚拟化会话 (VirtualizationSession)
- **Purpose**: 表示一次虚拟化生命周期管理的顶层对象。
- **Key Fields**:
  - `session_id`: 会话唯一标识。
  - `state`: 初始化/运行/停止/清理状态。
  - `cpu_features`: VT-x 支持能力摘要（如 VMX 支持与约束）。
  - `last_error`: 最近一次失败的错误码或描述。
- **Relationships**: 关联一个或多个虚拟机上下文。

### 虚拟机上下文 (VirtualMachineContext)
- **Purpose**: 表示单个虚拟机的运行状态与配置。
- **Key Fields**:
  - `vm_id`: 虚拟机标识。
  - `vcpu_count`: 虚拟 CPU 数量。
  - `vm_state`: 运行/暂停/停止等状态。
  - `vmcs_status`: VMCS 初始化与运行状态摘要。
  - `memory_layout`: 客户机内存布局摘要与限制。
- **Relationships**: 归属于虚拟化会话；与控制请求交互。

### 控制请求 (ControlRequest)
- **Purpose**: 用户态到驱动的控制命令载体。
- **Key Fields**:
  - `request_id`: 请求标识或序号。
  - `opcode`: 操作类型（如初始化、查询、关闭）。
  - `input_size` / `output_size`: 输入输出大小约束。
  - `status`: 执行结果状态。
- **Relationships**: 作用于虚拟化会话或虚拟机上下文。

## Validation Rules
- 设备不支持 VT-x 时，初始化类请求必须失败并返回可识别错误。
- 输入输出缓冲区超出允许范围时必须拒绝并返回错误。
- 在运行状态下拒绝非法卸载或提供安全退出路径。

## State Transitions
- **虚拟化会话**: `Uninitialized -> Initialized -> Running -> Stopped -> Cleaned`
- **虚拟机上下文**: `Created -> Running -> Paused -> Stopped`
