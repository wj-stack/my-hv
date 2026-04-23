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

/// 显式 `rax`，避免 `out(reg)` 与 Windows x64 代码生成偶发读错。
#[inline(never)]
pub fn read_cr0() -> u64 {
    let v: u64;
    unsafe {
        core::arch::asm!(
            "mov rax, cr0",
            out("rax") v,
            options(nomem, nostack),
        );
    }
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
/// `#[inline(never)]`：避免内联后与 Windows x64 ABI 交织导致控制寄存器写入用错寄存器。
#[inline(never)]
pub unsafe fn write_cr3(v: u64) {
    unsafe {
        core::arch::asm!(
            "mov cr3, rax",
            in("rax") v,
            options(nostack, nomem),
        );
    }
}

#[inline(never)]
pub fn read_cr4() -> u64 {
    let v: u64;
    unsafe {
        core::arch::asm!(
            "mov rax, cr4",
            out("rax") v,
            options(nomem, nostack),
        );
    }
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

/// 写 `CR0`：源操作数固定为 `rax`（勿用 `in(reg)`，曾出现 `mov cr0, rcx` 且 `rcx==0` 的蓝屏）。
/// 开 VMX 请用 `write_cr0_then_cr4`；本函数保留供其它路径。
#[allow(dead_code)]
#[inline(never)]
pub unsafe fn write_cr0(v: u64) {
    unsafe {
        core::arch::asm!(
            "mov cr0, rax",
            in("rax") v,
            options(nostack, nomem),
        );
    }
}

/// 写 `CR4`：源操作数固定为 `rdx`。
#[inline(never)]
pub unsafe fn write_cr4(v: u64) {
    unsafe {
        core::arch::asm!(
            "mov cr4, rdx",
            in("rdx") v,
            options(nostack, nomem),
        );
    }
}

/// 与 `hv` 中 `__writecr0` 后紧跟 `__writecr4` 一致，且在同一条内联汇编序列中完成，避免 LLVM 在两次独立 `write_cr*` 之间错误分配/破坏寄存器。
#[inline(never)]
unsafe fn write_cr0_then_cr4(cr0_val: u64, cr4_val: u64) {
    unsafe {
        core::arch::asm!(
            "mov cr0, rax",
            "mov cr4, rdx",
            in("rax") cr0_val,
            in("rdx") cr4_val,
            options(nostack, nomem),
        );
    }
}

/// 与 `hv/extern/ia32-doc` 及 `vcpu.cpp` 中 `CR4_VMX_ENABLE_FLAG` 相同（bit 13）。
const CR4_VMX_ENABLE_FLAG: u64 = 0x2000;

/// 关闭 `CR4.VMXE`（在 `VMXOFF` 之后调用，与 `hv` 中恢复路径一致）。
pub unsafe fn disable_vmx_hardware() {
    let mut cr4 = read_cr4();
    cr4 &= !CR4_VMX_ENABLE_FLAG;
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

/// 与 `hv/vcpu.cpp::enable_vmx_operation` 同序同语义（Intel SDM 3.23.7–3.23.8）。
///
/// C++：`_disable` → 读 `CR0`/`CR4` → `CR4 |= CR4_VMX_ENABLE_FLAG` → 套用 FIXED MSRs →
/// `__writecr0` / `__writecr4` → `_enable`。此处 `CR0`/`CR4` 在同一条 asm 中写入（`rax`/`rdx`）。
///
/// 开中断：仅当进入前 `RFLAGS.IF` 已置位时再 `sti`。宿主内核/work item 可能在 `IF=0` 的上下文中运行，无条件 `sti` 与 MSVC `_enable()` 行为一致但与 OS 期望不一致，且可能引发异常路径问题。
pub unsafe fn enable_vmx_in_hardware(cached: &VmxFixedMsrs) -> bool {
    if !has_vmx_in_cpuid() {
        return false;
    }
    if !feature_control_allows_vmx() {
        return false;
    }

    let rflags = read_rflags();
    let if_was_set = (rflags & (1u64 << 9)) != 0;

    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }

    let mut cr0 = read_cr0();
    let mut cr4 = read_cr4();

    cr4 |= CR4_VMX_ENABLE_FLAG;

    cr0 |= cached.vmx_cr0_fixed0;
    cr0 &= cached.vmx_cr0_fixed1;
    cr4 |= cached.vmx_cr4_fixed0;
    cr4 &= cached.vmx_cr4_fixed1;

    unsafe {
        write_cr0_then_cr4(cr0, cr4);
    }

    if if_was_set {
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack));
        }
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
    /// 与 `hv/vcpu.cpp::cache_cpu_data` 中 VMX 分支一致：在 `CPUID.1:ECX.VMX` 置位时读四个 `IA32_VMX_CR*_FIXED*`。
    pub fn read() -> Option<Self> {
        if !has_vmx_in_cpuid() {
            return None;
        }
        // SAFETY: VMX fixed MSRs per Intel SDM / ia32-doc.
        Some(Self {
            vmx_cr0_fixed0: unsafe { rdmsr(ia32::IA32_VMX_CR0_FIXED0) },
            vmx_cr0_fixed1: unsafe { rdmsr(ia32::IA32_VMX_CR0_FIXED1) },
            vmx_cr4_fixed0: unsafe { rdmsr(ia32::IA32_VMX_CR4_FIXED0) },
            vmx_cr4_fixed1: unsafe { rdmsr(ia32::IA32_VMX_CR4_FIXED1) },
        })
    }
}
