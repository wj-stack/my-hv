# Driver Interface Contract

## Overview

该契约描述 Rust 驱动对外暴露的 VMCALL 超级调用接口、控制请求以及返回语义。接口以“可演进、易诊断、可验证”为原则设计。

## Interface Identity

- **Contract Version**: `0.1.0`（与 `shared-contract` 同步）
- **Hypercall Key**: 由 `shared-contract` 统一定义，用户态与内核态保持一致

## IOCTL Control Codes

| IOCTL | Purpose |
|-------|---------|
| `IOCTL_PING` | 模板层连通性（返回 `PING_RESPONSE_U32`） |
| `IOCTL_ECHO` | 模板层回显 |
| `IOCTL_HV_START` | 在所有活动逻辑处理器上执行 VMXON + VMCLEAR/VMPTRLD，进入 VMX root |
| `IOCTL_HV_STOP` | 每 CPU `VMCLEAR` + `VMXOFF` 并释放资源 |
| `IOCTL_HV_HYPERCALL` | 缓冲的 `HvHypercallIn` / `HvHypercallOut` 超级调用桥（与 VMCALL 语义对齐） |

## Hypercall Requests

| Operation | Purpose | Input | Output | Failure Modes |
|-----------|---------|-------|--------|---------------|
| `PING` | 验证 hypervisor 存活 | 无 | 固定签名 | key 错误、未运行 |
| `UNLOAD` | 请求退出虚拟化 | 无 | 成功标志 | 状态非法 |
| `TEST` | 诊断/测试 | 可选参数 | 诊断输出 | key 错误 |
| `READ_PHYS_MEM` | 读取物理内存 | 目标地址、大小 | 字节数 | 参数无效、访问失败 |
| `WRITE_PHYS_MEM` | 写入物理内存 | 目标地址、数据、大小 | 字节数 | 参数无效、访问失败 |
| `READ_VIRT_MEM` | 读取虚拟内存 | 目标 CR3、地址、大小 | 字节数 | 参数无效、访问失败 |
| `WRITE_VIRT_MEM` | 写入虚拟内存 | 目标 CR3、地址、数据、大小 | 字节数 | 参数无效、访问失败 |
| `QUERY_PROCESS_CR3` | 查询进程 CR3 | PID | CR3 | PID 无效 |
| `INSTALL_EPT_HOOK` | 安装 EPT hook | 原页 PFN、执行页 PFN | 成功标志 | 资源不足 |
| `REMOVE_EPT_HOOK` | 移除 EPT hook | 原页 PFN | 成功标志 | 未找到 |
| `FLUSH_LOGS` | 刷新日志 | 目标缓冲区、数量 | 实际数量 | 缓冲区无效 |
| `GET_PHYSICAL_ADDRESS` | 查询虚拟地址物理映射 | 目标 CR3、地址 | 物理地址 | 参数无效 |
| `HIDE_PHYSICAL_PAGE` | 隐藏物理页 | PFN | 成功标志 | 资源不足 |
| `UNHIDE_PHYSICAL_PAGE` | 取消隐藏 | PFN | 成功标志 | 未找到 |
| `GET_HV_BASE` | 获取 HV 基址 | 无 | 基址 | 未运行 |
| `INSTALL_MMR` | 安装监控内存范围 | 地址、大小、模式 | 句柄 | 资源不足 |
| `REMOVE_MMR` | 移除监控范围 | 句柄 | 成功标志 | 句柄无效 |
| `REMOVE_ALL_MMRS` | 清空监控范围 | 无 | 成功标志 | 状态非法 |

## Error Semantics

- 所有失败返回必须可映射为明确错误类型（如“不支持 VT-x”、“状态非法”、“资源不足”）。
- key 校验失败必须拒绝执行并返回可诊断错误。
- 缓冲区越界或地址不可访问需返回“参数无效”或“访问失败”。
- 在运行状态下卸载请求应拒绝或执行安全退出路径并返回明确状态。

## Compatibility Notes

此接口为新设计，旧接口不被保证兼容。用户态工具必须使用新契约。
