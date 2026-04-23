//! VM-exit 汇编入口与 `VMRESUME` 前 Rust 处理。对应参考 `hv` 的 `vm-exit.asm` + `handle_vm_exit`。

use core::arch::global_asm;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::exit_handlers;
use crate::guest_context::GuestRegs;
use crate::ia32;
use crate::logger;
use crate::vmcs::{self, VmcsField, VmExitReason};
use crate::vcpu::VmxCluster;

static VMEXIT_SESSION: AtomicPtr<Option<VmxCluster>> = AtomicPtr::new(core::ptr::null_mut());

/// 在发起 `VmxCluster::start()` **之前**调用，指向 `UnsafeCell<Option<VmxCluster>>` 的裸指针。
pub fn install_vmexit_session(session_cell: *mut Option<VmxCluster>) {
    VMEXIT_SESSION.store(session_cell, Ordering::Release);
}

pub fn clear_vmexit_session() {
    VMEXIT_SESSION.store(core::ptr::null_mut(), Ordering::Release);
}

/// 供尚未进入 guest 时的诊断路径读取（与 `handle_vm_exit` 使用同一会话单元）。
pub fn peek_session_option_mut() -> Option<&'static mut Option<VmxCluster>> {
    unsafe { session_option_mut() }
}

unsafe fn session_option_mut() -> Option<&'static mut Option<VmxCluster>> {
    let p = VMEXIT_SESSION.load(Ordering::Acquire);
    if p.is_null() {
        return None;
    }
    Some(unsafe { &mut *p })
}

global_asm!(
    ".text",
    ".globl vmexit_host_stub",
    "vmexit_host_stub:",
    "push r15",
    "push r14",
    "push r13",
    "push r12",
    "push r11",
    "push r10",
    "push r9",
    "push r8",
    "push rdi",
    "push rsi",
    "push rbp",
    "push rbx",
    "push rdx",
    "push rcx",
    "push rax",
    "mov rcx, rsp",
    "sub rsp, 0x28",
    "call {handler}",
    "add rsp, 0x28",
    "pop rax",
    "pop rcx",
    "pop rdx",
    "pop rbx",
    "pop rbp",
    "pop rsi",
    "pop rdi",
    "pop r8",
    "pop r9",
    "pop r10",
    "pop r11",
    "pop r12",
    "pop r13",
    "pop r14",
    "pop r15",
    "vmresume",
    "jc 1f",
    "ret",
    "1:",
    "ud2",
    handler = sym handle_vm_exit,
);

unsafe extern "C" {
    pub fn vmexit_host_stub();
}

/// VM-exit C 入口：由 `vmexit_host_stub` 以 `rcx = &GuestRegs` 调用。
#[unsafe(no_mangle)]
pub extern "C" fn handle_vm_exit(regs: *mut GuestRegs) {
    if regs.is_null() {
        return;
    }
    let regs = unsafe { &mut *regs };

    let reason_raw = match unsafe { vmcs::vmread(VmcsField::EXIT_REASON) } {
        Ok(v) => v as u32,
        Err(_) => return,
    };
    let reason = VmExitReason::from_raw(reason_raw);
    let guest_rip = unsafe { vmcs::vmread(VmcsField::GUEST_RIP) }.unwrap_or(0);
    let guest_rsp = unsafe { vmcs::vmread(VmcsField::GUEST_RSP) }.unwrap_or(0);
    let guest_rflags = unsafe { vmcs::vmread(VmcsField::GUEST_RFLAGS) }.unwrap_or(0);

    let session_cell = unsafe { session_option_mut() };
    if let Some(opt) = session_cell {
        exit_handlers::dispatch_vm_exit(opt, regs, &reason, guest_rip, guest_rsp);
    } else {
        logger::log_vm_exit_reason(reason.basic, reason.raw);
    }

    let _ = (guest_rflags, ia32::MAXULONG64);
}
