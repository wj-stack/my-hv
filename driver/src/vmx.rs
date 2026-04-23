//! Intel VMX 指令封装与能力检查。对应 `hv/hv/vmx.h`、`hv/hv/vmx.asm`、`hv/hv/vmx.inl`。
#![allow(dead_code)]

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
    unsafe {
        core::arch::asm!(
            "vmxon qword ptr [{op}]",
            "setc {fail}",
            op = in(reg) phys_operand,
            fail = out(reg_byte) fail,
            options(nostack),
        );
    }
    fail == 0
}

#[inline]
pub unsafe fn vmxoff() -> bool {
    let mut fail: u8;
    unsafe {
        core::arch::asm!(
            "vmxoff",
            "setc {fail}",
            fail = out(reg_byte) fail,
            options(nostack),
        );
    }
    fail == 0
}

#[inline]
pub unsafe fn vmclear(phys_operand: *const u64) -> bool {
    let mut fail: u8;
    unsafe {
        core::arch::asm!(
            "vmclear qword ptr [{op}]",
            "setc {fail}",
            op = in(reg) phys_operand,
            fail = out(reg_byte) fail,
            options(nostack),
        );
    }
    fail == 0
}

#[inline]
pub unsafe fn vmptrld(phys_operand: *const u64) -> bool {
    let mut fail: u8;
    unsafe {
        core::arch::asm!(
            "vmptrld qword ptr [{op}]",
            "setc {fail}",
            op = in(reg) phys_operand,
            fail = out(reg_byte) fail,
            options(nostack),
        );
    }
    fail == 0
}

#[inline]
pub unsafe fn vmlaunch() -> bool {
    let mut fail: u8;
    unsafe {
        core::arch::asm!(
            "vmlaunch",
            "setc {fail}",
            fail = out(reg_byte) fail,
            options(nostack),
        );
    }
    fail == 0
}

#[inline]
pub unsafe fn vmresume() -> bool {
    let mut fail: u8;
    unsafe {
        core::arch::asm!(
            "vmresume",
            "setc {fail}",
            fail = out(reg_byte) fail,
            options(nostack),
        );
    }
    fail == 0
}

#[inline]
pub unsafe fn vmcall(rax: u64, rcx: u64, rdx: u64) -> (bool, u64) {
    let mut out_rax = rax;
    let mut fail: u8;
    unsafe {
        core::arch::asm!(
            "vmcall",
            "setc {fail}",
            inout("rax") out_rax,
            in("rcx") rcx,
            in("rdx") rdx,
            fail = out(reg_byte) fail,
            options(nostack),
        );
    }
    (fail == 0, out_rax)
}

#[repr(C)]
pub struct InveptDescriptor {
    pub eptp: u64,
    pub reserved: u64,
}

#[inline]
pub unsafe fn invept_single_context(eptp: u64) -> bool {
    let desc = InveptDescriptor { eptp, reserved: 0 };
    let mut ok: u8;
    unsafe {
        // 使用显式寄存器；`oword ptr [reg]` 在 LLVM IAS 下易触发解析错误。
        core::arch::asm!(
            "invept rcx, xmmword ptr [rax]",
            "setnc {ok}",
            in("rax") &raw const desc,
            in("rcx") 1u64,
            ok = out(reg_byte) ok,
            options(nostack),
        );
    }
    ok != 0
}

/// `invept_all_context`（`hv` 中 `invept_type::invept_all_context` = 2），在 `VMXON` 后使全局 EPT 缓存失效。
#[inline]
pub unsafe fn invept_all_contexts() -> bool {
    let desc = InveptDescriptor {
        eptp: 0,
        reserved: 0,
    };
    let mut ok: u8;
    unsafe {
        core::arch::asm!(
            "invept rcx, xmmword ptr [rax]",
            "setnc {ok}",
            in("rax") &raw const desc,
            in("rcx") 2u64,
            ok = out(reg_byte) ok,
            options(nostack),
        );
    }
    ok != 0
}
