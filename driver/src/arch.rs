//! Low-level CPU helpers (CPUID, MSRs, control registers). Maps to intrinsics in `hv/hv/arch*`.
//! Reference: `hv/extern/ia32-doc` and the legacy `hv` VMX path.

use core::arch::x86_64::{CpuidResult, __cpuid, __cpuid_count};

use crate::ia32;

/// CPUID leaf 1, ECX bit 5: VMX support in hardware.
pub fn has_vmx_in_cpuid() -> bool {
    let CpuidResult { ecx, .. } = cpuid(1, 0);
    (ecx & (1 << 5)) != 0
}

/// CPUID. Returns EAX, EBX, ECX, EDX for a given leaf; `sub` is the ECX sub-leaf.
pub fn cpuid(leaf: u32, sub: u32) -> CpuidResult {
    if sub == 0 {
        __cpuid(leaf)
    } else {
        __cpuid_count(leaf, sub)
    }
}

pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (hi, lo): (u32, u32);
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") lo,
            out("edx") hi,
        );
    }
    ((hi as u64) << 32) | (lo as u64)
}

#[allow(dead_code)]
pub unsafe fn wrmsr(msr: u32, value: u64) {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    unsafe { core::arch::asm!("wrmsr", in("ecx") msr, in("eax") lo, in("edx") hi) };
}

pub fn read_cr0() -> u64 {
    let v: u64;
    // SAFETY: no preconditions; reading cr0.
    unsafe { core::arch::asm!("mov {}, cr0", out(reg) v) };
    v
}

#[allow(dead_code)]
pub fn read_cr3() -> u64 {
    let v: u64;
    // SAFETY: no preconditions.
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) v) };
    v
}

/// 写 `CR3`（仅在离开 VMX 前从 guest 视图恢复，参考 `handle_vm_exit` 卸载块）。
pub unsafe fn write_cr3(v: u64) {
    unsafe { core::arch::asm!("mov cr3, {}", in(reg) v) };
}

pub fn read_cr4() -> u64 {
    let v: u64;
    // SAFETY: no preconditions.
    unsafe { core::arch::asm!("mov {}, cr4", out(reg) v) };
    v
}

/// 与 `hv` 中 `__readeflags()` / `RFLAGS` 一致。
#[inline]
pub fn read_rflags() -> u64 {
    let v: u64;
    unsafe {
        core::arch::asm!("pushfq", "pop {v}", v = out(reg) v);
    }
    v
}

/// 当前 `DR7`（与 `hv/hv/vmcs.cpp::write_vmcs_guest_fields` 中 `__readdr(7)` 一致）。
pub fn read_dr7() -> u64 {
    let v: u64;
    unsafe { core::arch::asm!("mov {}, dr7", out(reg) v) };
    v
}

/// 与 `hv` 中 `_readfsbase_u64` 一致（当前 `IA32_FS_BASE`）。
#[inline]
pub fn read_fsbase() -> u64 {
    unsafe { rdmsr(ia32::IA32_FS_BASE) }
}

pub unsafe fn write_cr0(v: u64) {
    unsafe { core::arch::asm!("mov cr0, {}", in(reg) v) };
}

pub unsafe fn write_cr4(v: u64) {
    unsafe { core::arch::asm!("mov cr4, {}", in(reg) v) };
}

const CR4_VMXE: u64 = 1 << 13;

/// 关闭 `CR4.VMXE`（在 `VMXOFF` 之后调用，与 `hv` 中恢复路径一致）。
pub unsafe fn disable_vmx_hardware() {
    let mut cr4 = read_cr4();
    cr4 &= !CR4_VMXE;
    unsafe { write_cr4(cr4) };
}

const EFER_LMA: u64 = 1 << 10;

pub unsafe fn read_msr_efer() -> u64 {
    unsafe { rdmsr(0xC000_0080) }
}

/// IA32_FEATURE_CONTROL: lock and VMX outside SMX must be set in firmware for our bring-up.
pub fn feature_control_allows_vmx() -> bool {
    // SAFETY: MSR 0x3A is always defined on x64.
    let v = unsafe { rdmsr(ia32::IA32_FEATURE_CONTROL) };
    (v & 0x1) != 0 && (v & 0x4) != 0
}

/// Long mode active (EFER.LMA).
pub fn is_long_mode() -> bool {
    // SAFETY: reading EFER
    (unsafe { read_msr_efer() } & EFER_LMA) != 0
}

/// Enable CR4.VMXE after fixing CR0/CR4 against IA32_VMX_{CR0,CR4}_FIXED* (see Intel SDM 3.23.6–8).
pub unsafe fn enable_vmx_in_hardware(cached: &VmxFixedMsrs) -> bool {
    if !has_vmx_in_cpuid() {
        return false;
    }
    if !is_long_mode() {
        return false;
    }
    if !feature_control_allows_vmx() {
        return false;
    }

    let mut cr0 = read_cr0();
    let mut cr4 = read_cr4();
    cr0 |= cached.vmx_cr0_fixed0;
    cr0 &= cached.vmx_cr0_fixed1;
    cr4 |= CR4_VMXE;
    cr4 |= cached.vmx_cr4_fixed0;
    cr4 &= cached.vmx_cr4_fixed1;
    unsafe {
        write_cr0(cr0);
        write_cr4(cr4);
    }
    true
}

pub struct VmxFixedMsrs {
    pub vmx_cr0_fixed0: u64,
    pub vmx_cr0_fixed1: u64,
    pub vmx_cr4_fixed0: u64,
    pub vmx_cr4_fixed1: u64,
}

/// `RDTSCP`：返回 EAX、EDX、ECX（Aux）。
pub fn read_rdtscp() -> (u32, u32, u32) {
    let eax: u32;
    let edx: u32;
    let ecx: u32;
    unsafe {
        core::arch::asm!(
            "rdtscp",
            out("eax") eax,
            out("edx") edx,
            out("ecx") ecx,
        );
    }
    (eax, edx, ecx)
}

impl VmxFixedMsrs {
    /// Pull fixed-0/1 values after VMX is available in CPUID. Mirrors `cache_cpu_data` in `hv/hv/vcpu.cpp`.
    pub fn read() -> Option<Self> {
        if !has_vmx_in_cpuid() {
            return None;
        }
        // SAFETY: well-known MSRs when VMX present
        Some(Self {
            vmx_cr0_fixed0: unsafe { rdmsr(ia32::IA32_VMX_CR0_FIXED0) },
            vmx_cr0_fixed1: unsafe { rdmsr(ia32::IA32_VMX_CR0_FIXED1) },
            vmx_cr4_fixed0: unsafe { rdmsr(ia32::IA32_VMX_CR4_FIXED0) },
            vmx_cr4_fixed1: unsafe { rdmsr(ia32::IA32_VMX_CR4_FIXED1) },
        })
    }
}
