//! Host IDT 入口的 Rust 侧，对应 `hv/hv/vcpu.cpp::handle_host_interrupt`。

use crate::arch;
use crate::ia32;
use crate::logger;
use crate::vmcs::{self, VmcsField};
use crate::vcpu::PerCpuState;

const NMI_VECTOR: u64 = 2;

/// 与 `interrupt-handlers.asm` / `trap_frame` 布局一致（15 个 GPR + vector + error + machine frame）。
#[repr(C)]
pub struct HostTrapFrame {
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub vector: u64,
    pub error: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// 由 `host_interrupts` 中 `host_generic_interrupt` 调用。
#[unsafe(no_mangle)]
pub extern "C" fn dispatch_host_interrupt(frame: *mut HostTrapFrame) {
    if frame.is_null() {
        return;
    }
    let f = unsafe { &mut *frame };
    match f.vector {
        NMI_VECTOR => {
            let Ok(ctrl) = (unsafe { vmcs::vmread(VmcsField::CTRL_CPU_BASED) }) else {
                return;
            };
            let new_ctrl = ctrl | u64::from(ia32::CPU_BASED_NMI_WINDOW_EXITING);
            let _ = unsafe { vmcs::vmwrite(VmcsField::CTRL_CPU_BASED, new_ctrl) };

            let cpu = unsafe { arch::read_fsbase() as *mut PerCpuState };
            if !cpu.is_null() {
                unsafe {
                    (*cpu).queued_nmis = (*cpu).queued_nmis.saturating_add(1);
                }
            }
        }
        _ => {
            logger::log_host_exception(f.vector, f.rip, f.error);
        }
    }
}
