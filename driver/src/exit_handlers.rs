//! VM-exit 分发骨架。对应 `hv/hv/exit-handlers.cpp` 中 `dispatch_vm_exit` / `handle_vm_exit`。

use crate::hypercalls;
use crate::ia32;
use crate::logger;
use crate::vcpu::VmxCluster;
use shared_contract::{HvHypercallIn, HvHypercallOut};

/// 当 VM-exit 原因为 `VMCALL` 时，将客户机寄存器打包为 `HvHypercallIn` 并复用 IOCTL 路径的分发逻辑。
///
/// 完整实现需在汇编 VM-exit 桩中填充 `guest_rax`..`guest_r11` 并调用此函数。
pub fn dispatch_vm_exit_hypercall(
    cluster: &mut Option<VmxCluster>,
    guest_rax: u64,
    args: [u64; 6],
) -> HvHypercallOut {
    let inp = HvHypercallIn { rax: guest_rax, args };
    logger::log("VM-exit: VMCALL -> hypercalls::dispatch");
    hypercalls::dispatch(cluster, &inp)
}

/// 按基本退出原因枚举做一级分发（占位：除 `VMCALL` 外仅记录日志）。
pub fn log_basic_exit(reason: u32) {
    if reason == ia32::VMX_EXIT_REASON_EXECUTE_VMCALL {
        logger::log("VM-exit reason: VMCALL");
    } else {
        logger::log("VM-exit reason (numeric)");
    }
}
