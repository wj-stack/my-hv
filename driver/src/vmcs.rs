//! VMCS 区域初始化与 VMCS 字段编码常量。对应 `hv/hv/vmcs.h`、`hv/hv/vmcs.cpp`。

use crate::{arch, ia32};

/// 将 IA32_VMX_BASIC 中的 VMCS revision 写入区域首 dword，其余清零。
pub unsafe fn prepare_vmcs_region(page: *mut u8) {
    let basic = unsafe { arch::rdmsr(ia32::IA32_VMX_BASIC) };
    let revision = (basic & 0x7FFF_FFFF) as u32;
    unsafe {
        core::ptr::write_bytes(page, 0, 4096);
        page.cast::<u32>().write_unaligned(revision);
    }
}

/// 与 `prepare_vmcs_region` 相同布局要求；VMXON 区域也使用 revision id。
pub unsafe fn prepare_vmxon_region(page: *mut u8) {
    unsafe { prepare_vmcs_region(page) };
}
