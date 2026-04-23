//! VM-exit 汇编入口与 `VMRESUME` 前 Rust 处理。对应参考 `hv` 的 `vm-exit.asm` + `handle_vm_exit`。

use core::arch::global_asm;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::exit_handlers;
use crate::ia32;
use crate::logger;
use crate::vmcs::{self, VmcsField, VmExitReason};
use crate::vcpu::VmxCluster;

/// 供 `vmexit_host_stub` 保存的通用寄存器块（低地址在前：与 push 顺序一致）。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GuestRegs {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
}

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
    // SAFETY: 汇编保证 `regs` 指向栈上 15 个 push 的布局。
    let regs = unsafe { &mut *regs };

    let reason_raw = match unsafe { vmcs::vmread(VmcsField::EXIT_REASON) } {
        Ok(v) => v as u32,
        Err(_) => return,
    };
    let reason = VmExitReason::from_raw(reason_raw);
    let basic = reason.basic as u32;

    let guest_rip = unsafe { vmcs::vmread(VmcsField::GUEST_RIP) }.unwrap_or(0);
    let guest_rsp = unsafe { vmcs::vmread(VmcsField::GUEST_RSP) }.unwrap_or(0);
    let guest_rflags = unsafe { vmcs::vmread(VmcsField::GUEST_RFLAGS) }.unwrap_or(0);

    let session_cell = unsafe { session_option_mut() };

    match basic {
        ia32::VMX_EXIT_REASON_EXECUTE_VMCALL => {
            let args = [
                regs.rbx,
                regs.rcx,
                regs.rdx,
                regs.r8,
                regs.r9,
                regs.r10,
            ];
            if let Some(opt) = session_cell {
                let out = exit_handlers::dispatch_vm_exit_hypercall(opt, regs.rax, args);
                regs.rax = out.rax;
            } else {
                logger::log("VM-exit VMCALL: no session");
            }
        }
        ia32::VMX_EXIT_REASON_CPUID => {
            let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(2);
            let (eax, ebx, ecx, edx) =
                exit_handlers::emulate_cpuid(regs.rax as u32, regs.rcx as u32);
            regs.rax = eax as u64;
            regs.rbx = ebx as u64;
            regs.rcx = ecx as u64;
            regs.rdx = edx as u64;
            let _ = unsafe { vmcs::vmwrite(VmcsField::GUEST_RIP, guest_rip.saturating_add(len)) };
        }
        ia32::VMX_EXIT_REASON_RDMSR => {
            let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(2);
            let msr = regs.rcx as u32;
            if let Some(v) = exit_handlers::emulate_rdmsr(msr) {
                regs.rax = v & 0xFFFF_FFFF;
                regs.rdx = v >> 32;
            }
            let _ = unsafe { vmcs::vmwrite(VmcsField::GUEST_RIP, guest_rip.saturating_add(len)) };
        }
        ia32::VMX_EXIT_REASON_WRMSR => {
            let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(2);
            let msr = regs.rcx as u32;
            let value = (regs.rdx << 32) | (regs.rax & 0xFFFF_FFFF);
            if exit_handlers::emulate_wrmsr(msr, value) {
                // emulate_wrmsr 当前写真实 MSR；bring-up 下仅用于非敏感 MSR。
            }
            let _ = unsafe { vmcs::vmwrite(VmcsField::GUEST_RIP, guest_rip.saturating_add(len)) };
        }
        ia32::VMX_EXIT_REASON_MOV_CR => {
            let qual = match unsafe { vmcs::vmread(VmcsField::EXIT_QUALIFICATION) } {
                Ok(v) => v,
                Err(_) => return,
            };
            let cr = (qual & 0xF) as u8;
            let _access = ((qual >> 4) & 0x3) as u8;
            let reg = ((qual >> 8) & 0xF) as u8;
            let val = mov_cr_read_gpr(regs, reg, guest_rsp);
            let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(0);
            match cr {
                0 => {
                    let _ = unsafe { vmcs::vmwrite(VmcsField::GUEST_CR0, val) };
                    let _ = unsafe { vmcs::vmwrite(VmcsField::CTRL_CR0_READ_SHADOW, val) };
                }
                3 => {
                    let _ = unsafe { vmcs::vmwrite(VmcsField::GUEST_CR3, val) };
                }
                4 => {
                    let _ = unsafe { vmcs::vmwrite(VmcsField::GUEST_CR4, val) };
                    let _ = unsafe { vmcs::vmwrite(VmcsField::CTRL_CR4_READ_SHADOW, val & !(1 << 13)) };
                }
                _ => {
                    logger::log("MOV_CR: unsupported CR index");
                }
            }
            let _ = unsafe { vmcs::vmwrite(VmcsField::GUEST_RIP, guest_rip.saturating_add(len)) };
        }
        ia32::VMX_EXIT_REASON_HLT => {
            logger::log("VM-exit HLT");
            let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(1);
            let _ = unsafe { vmcs::vmwrite(VmcsField::GUEST_RIP, guest_rip.saturating_add(len)) };
        }
        ia32::VMX_EXIT_REASON_EPT_VIOLATION => {
            logger::log("VM-exit EPT_VIOLATION");
        }
        _ => {
            logger::log_vm_exit_reason(reason.basic, reason.raw);
        }
    }

    let _ = (guest_rsp, guest_rflags);
}

fn mov_cr_read_gpr(regs: &GuestRegs, idx: u8, guest_rsp: u64) -> u64 {
    match idx {
        0 => regs.rax,
        1 => regs.rcx,
        2 => regs.rdx,
        3 => regs.rbx,
        4 => guest_rsp,
        5 => regs.rbp,
        6 => regs.rsi,
        7 => regs.rdi,
        8 => regs.r8,
        9 => regs.r9,
        10 => regs.r10,
        11 => regs.r11,
        12 => regs.r12,
        13 => regs.r13,
        14 => regs.r14,
        15 => regs.r15,
        _ => 0,
    }
}
