//! 向客户机注入事件（#GP、#UD 等），对应 `hv/hv/vmx` 的 `inject_hw_exception` 与 VM-entry 字段。
#![allow(unsafe_op_in_unsafe_fn)]

use crate::ia32;
use crate::vmcs::{self, VmcsField};

/// SDM 表 24-15：类型 2 = NMI；向量固定为 2。
pub unsafe fn inject_nmi() {
    let info = (0x8000_0000u32 | 2u32 << 8 | (ia32::X86_VECTOR_NMI & 0xFF)) as u64;
    let _ = vmcs::vmwrite(VmcsField::VMENTRY_INTERRUPTION_INFO, info);
    let _ = vmcs::vmwrite(VmcsField::VMENTRY_EXCEPTION_ERROR_CODE, 0);
}

/// SDM 表 24-15：类型 3 = 硬件异常；valid=31；有 error 时置 bit 11。
unsafe fn vmentry_interruption_info(vector: u32, error_code: Option<u32>) -> u32 {
    let mut v = 0x8000_0000u32 | 3u32 << 8 | (vector & 0xFF);
    if let Some(_e) = error_code {
        v |= 1 << 11;
    }
    v
}

/// 注入后由硬件在下一次 `VMRESUME` 进入客户机时递送；成功则清除其它路径应写入的 0 值在调用前于 `handle_vm_exit` 开头处理。
pub unsafe fn inject_hw_exception_with_code(vector: u32, error_code: Option<u32>) {
    let info = vmentry_interruption_info(vector, error_code) as u64;
    let _ = vmcs::vmwrite(VmcsField::VMENTRY_INTERRUPTION_INFO, info);
    if let Some(ec) = error_code {
        let _ = vmcs::vmwrite(VmcsField::VMENTRY_EXCEPTION_ERROR_CODE, u64::from(ec));
    } else {
        let _ = vmcs::vmwrite(VmcsField::VMENTRY_EXCEPTION_ERROR_CODE, 0);
    }
}

pub unsafe fn inject_invalid_opcode() {
    unsafe { inject_hw_exception_with_code(ia32::X86_VECTOR_UD, None) };
}

/// 与 `exit-handlers` 中默认未处理 path 的 `#GP(0)` 一致。
pub unsafe fn inject_general_protection_0() {
    unsafe { inject_hw_exception_with_code(ia32::X86_VECTOR_GP, Some(0)) };
}

/// 清除 VM-entry 注入域，避免本次 exit 的残留影响下一次 vmentry。
pub unsafe fn clear_vmentry_injection() {
    let _ = vmcs::vmwrite(VmcsField::VMENTRY_INTERRUPTION_INFO, 0);
    let _ = vmcs::vmwrite(VmcsField::VMENTRY_EXCEPTION_ERROR_CODE, 0);
}
