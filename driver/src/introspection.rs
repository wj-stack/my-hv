//! 进程/内存自省。对应 `hv/hv/introspection.*` 与 `hv/hv/hv.cpp` 中 `find_offsets` 的硬编码/动态解析子集。

use wdk::nt_success;
use wdk_sys::ntddk::PsLookupProcessByProcessId;
use wdk_sys::{HANDLE, LONG, NTSTATUS, PEPROCESS};

use crate::arch;

unsafe extern "C" {
    /// `ntoskrnl` 导出名（`ObDerefObject` 为头文件宏，链接名为 `ObDereferenceObject`）。
    fn ObDereferenceObject(object: *mut core::ffi::c_void) -> LONG;
}

/// 与 `hv` 在 `find_offsets` 中使用的 `KPROCESS::DirectoryTableBase` 在 `EPROCESS` 内偏移（x64 常见为 0x28）。
const EPROCESS_DIRECTORY_TABLE_BASE: usize = 0x28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub cr3: u64,
}

pub fn current_process_snapshot() -> ProcessSnapshot {
    ProcessSnapshot {
        cr3: arch::read_cr3(),
    }
}

/// 按 `PsLookupProcessByProcessId` 查询目录表，失败时回退为 `None`。
pub fn query_process_cr3(pid: u64) -> Option<u64> {
    if pid == 0 {
        return Some(arch::read_cr3());
    }
    let mut eprocess: PEPROCESS = core::ptr::null_mut();
    let ph = pid as HANDLE;
    let st: NTSTATUS = unsafe { PsLookupProcessByProcessId(ph, &mut eprocess) };
    if !nt_success(st) || eprocess.is_null() {
        return None;
    }
    let e = eprocess.cast::<u8>();
    let cr3 = unsafe { *(e.add(EPROCESS_DIRECTORY_TABLE_BASE) as *const u64) };
    unsafe { ObDereferenceObject(eprocess.cast()) };
    Some(cr3)
}
