//! Intel VMX 指令封装与能力检查。对应 `hv/hv/vmx.h`、`hv/hv/vmx.asm`、`hv/hv/vmx.inl`。

use crate::arch;

/// 在进入 VMX 操作前调用；若硬件/固件不支持则返回 `false`。
pub fn host_supports_vmx_root() -> bool {
    arch::has_vmx_in_cpuid()
        && arch::is_long_mode()
        && arch::feature_control_allows_vmx()
}

/// `VMXON`：操作数为指向 64 位物理地址的指针。
#[inline]
pub unsafe fn vmxon(phys_operand: *const u64) -> bool {
    let mut fail: u8;
    // SAFETY: VMX root 且操作数有效。
    core::arch::asm!(
        "vmxon qword ptr [{op}]",
        "setc {fail}",
        op = in(reg) phys_operand,
        fail = out(reg_byte) fail,
        options(nostack),
    );
    fail == 0
}

#[inline]
pub unsafe fn vmxoff() -> bool {
    let mut fail: u8;
    core::arch::asm!(
        "vmxoff",
        "setc {fail}",
        fail = out(reg_byte) fail,
        options(nostack),
    );
    fail == 0
}

#[inline]
pub unsafe fn vmclear(phys_operand: *const u64) -> bool {
    let mut fail: u8;
    core::arch::asm!(
        "vmclear qword ptr [{op}]",
        "setc {fail}",
        op = in(reg) phys_operand,
        fail = out(reg_byte) fail,
        options(nostack),
    );
    fail == 0
}

#[inline]
pub unsafe fn vmptrld(phys_operand: *const u64) -> bool {
    let mut fail: u8;
    core::arch::asm!(
        "vmptrld qword ptr [{op}]",
        "setc {fail}",
        op = in(reg) phys_operand,
        fail = out(reg_byte) fail,
        options(nostack),
    );
    fail == 0
}
