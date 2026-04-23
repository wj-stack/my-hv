//! 进程/内存自省。对应 `hv/hv/introspection.*` 与 `hv/hv/hv.cpp` 中 `find_offsets` / `ghv.system_cr3`。

extern crate alloc;
use alloc::boxed::Box;

use core::sync::atomic::{AtomicPtr, Ordering};

use wdk::nt_success;
use wdk_sys::ntddk::PsLookupProcessByProcessId;
use wdk_sys::{HANDLE, LONG, NTSTATUS, PEPROCESS};

use crate::arch;

unsafe extern "C" {
    fn ObDereferenceObject(object: *mut core::ffi::c_void) -> LONG;
    fn PsGetProcessId(process: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    fn PsGetProcessImageFileName(process: *mut core::ffi::c_void) -> *mut u8;
    fn PsGetCurrentThreadProcess() -> *mut core::ffi::c_void;
}

/// 与 `hv::ghv.kprocess_directory_table_base_offset` 一致（x64 常见值，与其它字段一起在 `find_offsets` 中参与 System CR3 读取）。
const KPROCESS_DIRECTORY_TABLE_BASE_OFFSET: usize = 0x28;

#[derive(Clone, Copy)]
pub struct HvOffsets {
    pub kprocess_directory_table_base: usize,
    pub eprocess_unique_process_id_offset: usize,
    pub eprocess_image_file_name_offset: usize,
    pub kapc_state_process_offset: usize,
    /// 与 `hv::ghv.system_cr3` 等价，在 `find_offsets` 内通过 `PsLookupProcessByProcessId(SystemPid)` 采样。
    pub cached_system_cr3: u64,
}

static HV_OFFSETS_PTR: AtomicPtr<HvOffsets> = AtomicPtr::new(core::ptr::null_mut());

/// 与 `hv::find_offsets()` 等价；须在 bring-up 单线程路径调用成功后再依赖 [`system_cr3`] / [`query_process_cr3`]。
pub unsafe fn find_offsets() -> bool {
    let existing = HV_OFFSETS_PTR.load(Ordering::Acquire);
    if !existing.is_null() {
        return true;
    }

    let kprocess_directory_table_base = KPROCESS_DIRECTORY_TABLE_BASE_OFFSET;

    let mut system_eprocess: PEPROCESS = core::ptr::null_mut();
    let st_sys = unsafe {
        PsLookupProcessByProcessId(4usize as HANDLE, &mut system_eprocess)
    };
    if !nt_success(st_sys) || system_eprocess.is_null() {
        return false;
    }
    let sys_ep = system_eprocess.cast::<u8>();
    let cached_system_cr3 =
        unsafe { *sys_ep.add(kprocess_directory_table_base).cast::<u64>() };
    unsafe { ObDereferenceObject(system_eprocess.cast()) };

    let ps_get_process_id = PsGetProcessId as *const u8;
    if unsafe {
        *ps_get_process_id != 0x48
            || *ps_get_process_id.add(1) != 0x8B
            || *ps_get_process_id.add(2) != 0x81
            || *ps_get_process_id.add(7) != 0xC3
    } {
        return false;
    }
    let eprocess_unique_process_id_offset =
        unsafe { *ps_get_process_id.add(3).cast::<u32>() as usize };

    let ps_get_process_image_file_name = PsGetProcessImageFileName as *const u8;
    if unsafe {
        *ps_get_process_image_file_name != 0x48
            || *ps_get_process_image_file_name.add(1) != 0x8D
            || *ps_get_process_image_file_name.add(2) != 0x81
            || *ps_get_process_image_file_name.add(7) != 0xC3
    } {
        return false;
    }
    let eprocess_image_file_name_offset =
        unsafe { *ps_get_process_image_file_name.add(3).cast::<u32>() as usize };

    let ps_get_current_thread_process = PsGetCurrentThreadProcess as *const u8;
    if unsafe {
        *ps_get_current_thread_process != 0x65
            || *ps_get_current_thread_process.add(1) != 0x48
            || *ps_get_current_thread_process.add(2) != 0x8B
            || *ps_get_current_thread_process.add(3) != 0x04
            || *ps_get_current_thread_process.add(4) != 0x25
            || *ps_get_current_thread_process.add(9) != 0x48
            || *ps_get_current_thread_process.add(10) != 0x8B
            || *ps_get_current_thread_process.add(11) != 0x80
    } {
        return false;
    }
    let kapc_state_process_offset =
        unsafe { *ps_get_current_thread_process.add(12).cast::<u32>() as usize };

    let off = HvOffsets {
        kprocess_directory_table_base,
        eprocess_unique_process_id_offset,
        eprocess_image_file_name_offset,
        kapc_state_process_offset,
        cached_system_cr3,
    };
    let p = Box::into_raw(Box::new(off));
    match HV_OFFSETS_PTR.compare_exchange(
        core::ptr::null_mut(),
        p,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => true,
        Err(other) => {
            unsafe { drop(Box::from_raw(p)) };
            !other.is_null()
        }
    }
}

/// `hv::ghv.system_cr3`：System 进程页目录物理地址。
pub fn system_cr3() -> Option<u64> {
    let p = HV_OFFSETS_PTR.load(Ordering::Acquire);
    if p.is_null() {
        return None;
    }
    unsafe { Some((*p).cached_system_cr3) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub cr3: u64,
}

pub fn current_process_snapshot() -> ProcessSnapshot {
    ProcessSnapshot {
        cr3: arch::read_cr3(),
    }
}

/// 按 `PsLookupProcessByProcessId` 查询目录表；DTB 偏移与 `find_offsets` 中一致。
pub fn query_process_cr3(pid: u64) -> Option<u64> {
    if pid == 0 {
        return Some(arch::read_cr3());
    }
    let p = HV_OFFSETS_PTR.load(Ordering::Acquire);
    if p.is_null() {
        return None;
    }
    let off = unsafe { &*p };
    let mut eprocess: PEPROCESS = core::ptr::null_mut();
    let ph = pid as HANDLE;
    let st: NTSTATUS = unsafe { PsLookupProcessByProcessId(ph, &mut eprocess) };
    if !nt_success(st) || eprocess.is_null() {
        return None;
    }
    let e = eprocess.cast::<u8>();
    let cr3 = unsafe { *(e.add(off.kprocess_directory_table_base).cast::<u64>()) };
    unsafe { ObDereferenceObject(eprocess.cast()) };
    Some(cr3)
}
