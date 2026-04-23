//! VM-exit 一级分发，与 `hv/hv/vcpu.cpp` 的 `dispatch_vm_exit` 同构。

use crate::arch;
use crate::exception_inject;
use crate::guest_context::GuestRegs;
use crate::hypercalls;
use crate::ia32;
use crate::logger;
use crate::mtrr;
use crate::timing;
use crate::vmcs::{self, VmcsField, VmExitReason};
use crate::vcpu::VcpuCache;
use crate::vcpu::VmxCluster;
use shared_contract::{HvHypercallIn, HvHypercallOut, HYPERCALL_KEY, STATUS_INVALID_PARAMETER};

fn clear_injection() {
    unsafe { exception_inject::clear_vmentry_injection() };
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

fn advance_guest_rip(len: u64) {
    if let Ok(rip) = unsafe { vmcs::vmread(VmcsField::GUEST_RIP) } {
        let _ = unsafe { vmcs::vmwrite(VmcsField::GUEST_RIP, rip.saturating_add(len)) };
    }
}

/// 当 VM-exit 原因为 `VMCALL` 时，复用 `hypercalls::dispatch`。
pub fn dispatch_vm_exit_hypercall(
    cluster: &mut Option<VmxCluster>,
    guest_rax: u64,
    args: [u64; 6],
) -> HvHypercallOut {
    if (guest_rax >> 8) != HYPERCALL_KEY {
        unsafe { exception_inject::inject_invalid_opcode() };
        return HvHypercallOut {
            status: STATUS_INVALID_PARAMETER,
            rax: 0,
            _reserved: 0,
        };
    }
    let inp = HvHypercallIn { rax: guest_rax, args };
    logger::log("VM-exit: VMCALL -> hypercalls::dispatch");
    hypercalls::dispatch(cluster, &inp)
}

fn handle_cpuid(regs: &mut GuestRegs) {
    let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(2);
    let (eax, ebx, ecx, edx) = emulate_cpuid(regs.rax as u32, regs.rcx as u32);
    regs.rax = u64::from(eax);
    regs.rbx = u64::from(ebx);
    regs.rcx = u64::from(ecx);
    regs.rdx = u64::from(edx);
    advance_guest_rip(len);
}

fn handle_rdmsr(cache: &VcpuCache, regs: &mut GuestRegs) {
    let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(2);
    let msr = regs.rcx as u32;
    if let Some(v) = emulate_rdmsr(msr, cache) {
        regs.rax = v & 0xFFFF_FFFF;
        regs.rdx = v >> 32;
    } else {
        unsafe { exception_inject::inject_general_protection_0() };
    }
    advance_guest_rip(len);
}

mod msr_port {
    use crate::arch;
    use crate::ia32;

    pub fn guest_rdmsr_ok(msr: u32) -> Option<u64> {
        if msr == 0 {
            return None;
        }
        if msr <= 0x0000_1FFF
            || (0xC000_0000..=0xC000_1FFF).contains(&msr)
            || (0xC001_0000..=0xC001_1FFF).contains(&msr)
        {
            return Some(unsafe { arch::rdmsr(msr) });
        }
        if matches!(msr, 0x10 | 0x1B | 0x1D9 | 0x277 | ia32::IA32_FEATURE_CONTROL) {
            return Some(unsafe { arch::rdmsr(msr) });
        }
        None
    }

    pub fn guest_wrmsr_ok(msr: u32, value: u64) -> bool {
        if guest_rdmsr_ok(msr).is_none() {
            return false;
        }
        unsafe { arch::wrmsr(msr, value) };
        true
    }
}

fn handle_wrmsr(cl: &mut VmxCluster, regs: &mut GuestRegs) {
    let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(2);
    let msr = regs.rcx as u32;
    let value = (regs.rdx << 32) | (regs.rax & 0xFFFF_FFFF);
    if msr == 0 {
        advance_guest_rip(len);
        return;
    }
    if !msr_port::guest_wrmsr_ok(msr, value) {
        unsafe { exception_inject::inject_general_protection_0() };
        advance_guest_rip(len);
        return;
    }
    if mtrr::is_mtrr_msr(msr) {
        if let Some(cpu) = cl.current_cpu_mut() {
            if let Some(ref mut e) = cpu.ept {
                unsafe { mtrr::on_mtrr_msr_write(e) };
            }
        }
    }
    advance_guest_rip(len);
}

fn handle_xsetbv(regs: &mut GuestRegs) {
    let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(2);
    let ecx = regs.rcx as u32;
    if let Ok(cr4) = unsafe { vmcs::vmread(VmcsField::GUEST_CR4) } {
        if (cr4 & (1 << 18)) == 0 {
            unsafe { exception_inject::inject_general_protection_0() };
            advance_guest_rip(len);
            return;
        }
    }
    let v = (regs.rdx << 32) | (regs.rax & 0xFFFF_FFFF);
    unsafe { core::arch::asm!("xsetbv", in("ecx") ecx, in("edx") (v >> 32) as u32, in("eax") v as u32) };
    advance_guest_rip(len);
}

fn handle_rdtsc(regs: &mut GuestRegs) {
    let v = timing::rdtsc();
    regs.rax = (v as u32) as u64;
    regs.rdx = (v >> 32) as u64;
    let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(2);
    advance_guest_rip(len);
}

fn handle_rdtscp(regs: &mut GuestRegs) {
    let (lo, hi, aux) = arch::read_rdtscp();
    regs.rax = u64::from(lo);
    regs.rdx = u64::from(hi);
    regs.rcx = u64::from(aux);
    let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(2);
    advance_guest_rip(len);
}

fn handle_mov_cr(regs: &mut GuestRegs, guest_rsp: u64) {
    let qual = match unsafe { vmcs::vmread(VmcsField::EXIT_QUALIFICATION) } {
        Ok(v) => v,
        Err(_) => return,
    };
    let cr = (qual & 0xF) as u8;
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
        _ => logger::log("MOV_CR: unsupported CR index"),
    }
    advance_guest_rip(len);
}

/// 与 `dispatch_vm_exit` 一致。
pub fn dispatch_vm_exit(
    cluster: &mut Option<VmxCluster>,
    regs: &mut GuestRegs,
    reason: &VmExitReason,
    _guest_rip: u64,
    guest_rsp: u64,
) {
    clear_injection();
    logger::log_vm_exit_reason(reason.basic, reason.raw);

    if let Some(cl) = cluster.as_mut() {
        if let Some(cpu) = cl.current_cpu_mut() {
            cpu.cache.hide_vm_exit_overhead = false;
        }
        timing::on_vm_exit(cl);
    }

    let basic = u32::from(reason.basic);
    match basic {
        ia32::VMX_EXIT_REASON_EXCEPTION_NMI => {
            logger::log("VM-exit: EXCEPTION_NMI (stub)");
            unsafe { exception_inject::inject_general_protection_0() };
        }
        ia32::VMX_EXIT_REASON_VMX_GETSEC | ia32::VMX_EXIT_REASON_INVD => {
            unsafe { exception_inject::inject_general_protection_0() };
        }
        ia32::VMX_EXIT_REASON_NMI_WINDOW => {
            logger::log("VM-exit: NMI_WINDOW");
        }
        ia32::VMX_EXIT_REASON_CPUID => handle_cpuid(regs),
        ia32::VMX_EXIT_REASON_MOV_CR => handle_mov_cr(regs, guest_rsp),
        ia32::VMX_EXIT_REASON_RDMSR => {
            if let Some(c) = cluster {
                if let Some(cpu) = c.current_cpu_mut() {
                    handle_rdmsr(&cpu.cache, regs);
                } else {
                    handle_rdmsr(&VcpuCache::empty(), regs);
                }
            } else {
                handle_rdmsr(&VcpuCache::empty(), regs);
            }
        }
        ia32::VMX_EXIT_REASON_WRMSR => {
            if let Some(c) = cluster.as_mut() {
                handle_wrmsr(c, regs);
            }
        }
        ia32::VMX_EXIT_REASON_XSETBV => handle_xsetbv(regs),
        ia32::VMX_EXIT_REASON_VMXON => {
            unsafe { exception_inject::inject_general_protection_0() };
        }
        ia32::VMX_EXIT_REASON_VMCALL => {
            if cluster.is_some() {
                let args = [regs.rbx, regs.rcx, regs.rdx, regs.r8, regs.r9, regs.r10];
                let out = dispatch_vm_exit_hypercall(cluster, regs.rax, args);
                regs.rax = out.rax;
            } else {
                logger::log("VM-exit VMCALL: no session");
            }
        }
        ia32::VMX_EXIT_REASON_VMX_PREEMPTION_TIMER => {
            if let Some(c) = cluster {
                timing::on_preemption_timer(c);
            }
        }
        ia32::VMX_EXIT_REASON_EPT_VIOLATION => {
            logger::log("VM-exit EPT_VIOLATION (see EPT/hooks)");
            unsafe { exception_inject::inject_general_protection_0() };
        }
        ia32::VMX_EXIT_REASON_EPT_MISCONFIG => {
            unsafe { exception_inject::inject_general_protection_0() };
        }
        ia32::VMX_EXIT_REASON_RDTSC => handle_rdtsc(regs),
        ia32::VMX_EXIT_REASON_RDTSCP => handle_rdtscp(regs),
        ia32::VMX_EXIT_REASON_MONITOR_TRAP_FLAG => {
            logger::log("VM-exit: MTF");
        }
        ia32::VMX_EXIT_REASON_INVEPT
        | ia32::VMX_EXIT_REASON_INVVPID
        | ia32::VMX_EXIT_REASON_VMCLEAR
        | ia32::VMX_EXIT_REASON_VMLAUNCH
        | ia32::VMX_EXIT_REASON_VMPTRLD
        | ia32::VMX_EXIT_REASON_VMPTRST
        | ia32::VMX_EXIT_REASON_VMREAD
        | ia32::VMX_EXIT_REASON_VMRESUME
        | ia32::VMX_EXIT_REASON_VMWRITE
        | ia32::VMX_EXIT_REASON_VMXOFF
        | ia32::VMX_EXIT_REASON_VMFUNC => {
            unsafe { exception_inject::inject_general_protection_0() };
        }
        ia32::VMX_EXIT_REASON_HLT => {
            let len = unsafe { vmcs::vmread(VmcsField::VMEXIT_INSTRUCTION_LEN) }.unwrap_or(1);
            advance_guest_rip(len);
        }
        _ => {
            logger::log_vm_exit_reason(reason.basic, reason.raw);
            unsafe { exception_inject::inject_general_protection_0() };
        }
    }

    if let Some(c) = cluster.as_mut() {
        timing::post_dispatch_vmexit(c, basic);
    }
    let _ = guest_rsp;
}

pub fn emulate_cpuid(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    let r = arch::cpuid(leaf, subleaf);
    (r.eax, r.ebx, r.ecx, r.edx)
}

pub fn emulate_rdmsr(msr: u32, cache: &VcpuCache) -> Option<u64> {
    if msr == ia32::IA32_FEATURE_CONTROL {
        return Some(cache.guest_feature_control);
    }
    msr_port::guest_rdmsr_ok(msr)
}

pub fn log_basic_exit(reason: u32) {
    match reason {
        ia32::VMX_EXIT_REASON_VMCALL => logger::log("VM-exit reason: VMCALL"),
        ia32::VMX_EXIT_REASON_CPUID => logger::log("VM-exit reason: CPUID"),
        ia32::VMX_EXIT_REASON_RDMSR => logger::log("VM-exit reason: RDMSR"),
        ia32::VMX_EXIT_REASON_WRMSR => logger::log("VM-exit reason: WRMSR"),
        ia32::VMX_EXIT_REASON_MOV_CR => logger::log("VM-exit reason: MOV_CR"),
        ia32::VMX_EXIT_REASON_EPT_VIOLATION => logger::log("VM-exit reason: EPT_VIOLATION"),
        _ => logger::log("VM-exit reason: OTHER"),
    }
}
