# `hv/hv` → `my-hv-driver` 参考映射

| 参考 C++ | Rust 模块 |
|----------|------------|
| `hv/vcpu.cpp` | `driver/src/vcpu.rs`, `vmexit.rs`, `exit_handlers.rs` |
| `hv/exit-handlers.cpp` | `exit_handlers.rs`, `exception_inject.rs` |
| `hv/hypercalls.cpp` | `hypercalls.rs` |
| `hv/ept.cpp` | `ept.rs` |
| `hv/mtrr.*` + EPT 更新 | `mtrr.rs` + `EptState::refresh_all_memory_types` |
| `hv/vmx.inl` MSR bitmap | `msr_bitmap.rs` |
| `hv/timing.*` | `timing.rs` |
| `hv/introspection.*` + `hv.cpp` offsets | `introspection.rs` |
| `hv/mm.cpp` | `mm.rs` |

## 宿主机 NMI/陷阱帧

参考 `handle_host_interrupt` 依赖 VM 启动的 trap 路径；**本 WDM 驱动**的 unload 为 IOCTL 在 host 上直接 `VMCLEAR`/`VMXOFF`，**不**在 guest 中恢复整段 GDT/IDT（与 `handle_vm_exit` 中 `stop_virtualization` 全量恢复区分；若将来从 guest 内卸载需补汇编返回路径）。

## 构建

在仓库根目录使用 `build.bat`（已配置 eWDK）。
