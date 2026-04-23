//! VM-exit 分发骨架。对应 `hv/hv/exit-handlers.cpp` 中 `dispatch_vm_exit` / `handle_vm_exit`。
#![allow(dead_code)]

use crate::hypercalls;
use crate::arch;
use crate::ia32;
use crate::logger;
use crate::vmcs::VmExitReason;
use crate::vcpu::VmxCluster;
use shared_contract::{HvHypercallIn, HvHypercallOut};

/// VM-exit 分发需要的最小上下文（由未来汇编桩填充）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmExitContext {
    pub reason: VmExitReason,
    pub guest_rip: u64,
    pub guest_rsp: u64,
    pub guest_rflags: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmExitAction {
    Resume,
    Stop,
}

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
    match reason {
        ia32::VMX_EXIT_REASON_EXECUTE_VMCALL => logger::log("VM-exit reason: VMCALL"),
        ia32::VMX_EXIT_REASON_CPUID => logger::log("VM-exit reason: CPUID"),
        ia32::VMX_EXIT_REASON_RDMSR => logger::log("VM-exit reason: RDMSR"),
        ia32::VMX_EXIT_REASON_WRMSR => logger::log("VM-exit reason: WRMSR"),
        ia32::VMX_EXIT_REASON_MOV_CR => logger::log("VM-exit reason: MOV_CR"),
        ia32::VMX_EXIT_REASON_EPT_VIOLATION => logger::log("VM-exit reason: EPT_VIOLATION"),
        _ => logger::log("VM-exit reason: OTHER"),
    }
}

/// 结构化 VM-exit 入口：统一记录上下文，并在 `VMCALL` 时复用 hypercall 分发。
#[allow(dead_code)]
pub fn dispatch_vm_exit(
    cluster: &mut Option<VmxCluster>,
    ctx: &VmExitContext,
    guest_rax: u64,
    args: [u64; 6],
) -> Option<HvHypercallOut> {
    logger::log_vm_exit_reason(ctx.reason.basic, ctx.reason.raw);
    log_basic_exit(ctx.reason.basic as u32);

    if ctx.reason.basic as u32 == ia32::VMX_EXIT_REASON_EXECUTE_VMCALL {
        return Some(dispatch_vm_exit_hypercall(cluster, guest_rax, args));
    }
    None
}

pub fn emulate_cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    let r = arch::cpuid(leaf, subleaf);
    (r.eax, r.ebx, r.ecx, r.edx)
}

pub fn emulate_rdmsr(msr: u32) -> Option<u64> {
    if msr == 0 {
        return None;
    }
    // SAFETY: VM-exit 模拟路径按调用方保证 MSR 有效。
    Some(unsafe { arch::rdmsr(msr) })
}

pub fn emulate_wrmsr(msr: u32, value: u64) -> bool {
    if msr == 0 {
        return false;
    }
    // SAFETY: VM-exit 模拟路径按调用方保证 MSR 有效。
    unsafe { arch::wrmsr(msr, value) };
    true
}

pub fn emulate_mov_cr(cr: u8, value: u64) -> bool {
    match cr {
        0 => {
            // SAFETY: VM-exit 模拟路径按调用方保证写入时机。
            unsafe { arch::write_cr0(value) };
            true
        }
        4 => {
            // SAFETY: VM-exit 模拟路径按调用方保证写入时机。
            unsafe { arch::write_cr4(value) };
            true
        }
        _ => false,
    }
}
