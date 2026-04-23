//! 进程/内存自省。对应 `hv/hv/hv.cpp::find_offsets` / `ghv.system_cr3`。
//!
//! **偏移来源**：优先按 [`RtlGetVersion`] 的 `dwBuildNumber` 在 [`crate::offset_table`] 中查表（避免导出点
//! `E9` 跳板导致 `Ps*` 序言探测失败）；未命中时再对 `PsGetProcessId` 等做与 `hv.cpp` 相同的定长机器码探测。
//!
//! **System CR3**：读 `EPROCESS::DirectoryTableBase` 后，若与当前 `CR3` 的 PML4 物理基址不一致且
//! `PsGetCurrentProcessId()==4`（DriverEntry 常见情况），则改用 `read_cr3()`，避免错误 DTB 在
//! [`crate::host_page_tables`] 中触发 `PAGE_FAULT_IN_NONPAGED_AREA`。
//!
//! `PsInitialSystemProcess` 为 ntoskrnl 导出**数据**符号，Rust/`link.exe` 侧常无法解析（见
//! [microsoft/windows-drivers-rs#338](https://github.com/microsoft/windows-drivers-rs/issues/338)）。
//! 此处采用 issue 中与 C 侧等价的 `PsLookupProcessByProcessId(4)` + 原子缓存 + `ObfDereferenceObject`
//! 取得与 `reinterpret_cast<uint8_t*>(PsInitialSystemProcess)` 相同的 System `EPROCESS` 指针。

extern crate alloc;
use alloc::boxed::Box;

use core::ffi::c_void;
use core::mem::size_of;
use core::sync::atomic::{AtomicPtr, Ordering};

use wdk::{nt_success, println};
use wdk_sys::ntddk::PsLookupProcessByProcessId;
use wdk_sys::{HANDLE, LONG, NTSTATUS, PEPROCESS};

use crate::arch;
use crate::offset_table;

unsafe extern "C" {
    fn ObDereferenceObject(object: *mut c_void) -> LONG;
    fn ObfDereferenceObject(object: *mut c_void);
    fn PsGetProcessId(process: *mut c_void) -> *mut c_void;
    fn PsGetProcessImageFileName(process: *mut c_void) -> *mut u8;
    fn PsGetCurrentThreadProcess() -> *mut c_void;
    fn PsGetCurrentProcessId() -> HANDLE;
    fn RtlGetVersion(version_information: *mut RtlOsVersionInfoW) -> NTSTATUS;
}

#[repr(C)]
struct RtlOsVersionInfoW {
    dw_os_version_info_size: u32,
    dw_major_version: u32,
    dw_minor_version: u32,
    dw_build_number: u32,
    dw_platform_id: u32,
    sz_csd_version: [u16; 128],
}

fn nt_build_number() -> Option<u32> {
    let mut info = RtlOsVersionInfoW {
        dw_os_version_info_size: size_of::<RtlOsVersionInfoW>() as u32,
        dw_major_version: 0,
        dw_minor_version: 0,
        dw_build_number: 0,
        dw_platform_id: 0,
        sz_csd_version: [0; 128],
    };
    let st = unsafe { RtlGetVersion(&mut info) };
    if !nt_success(st) {
        return None;
    }
    Some(info.dw_build_number)
}

/// 与 `hv::hypervisor` 中由 `find_offsets` 填充的 Windows 偏移字段对齐。
#[derive(Clone, Copy)]
pub struct HvOffsets {
    pub system_eprocess: usize,
    pub kprocess_directory_table_base_offset: usize,
    pub kpcr_pcrb_offset: usize,
    pub kprcb_current_thread_offset: usize,
    pub kthread_apc_state_offset: usize,
    pub eprocess_unique_process_id_offset: usize,
    pub eprocess_image_file_name: usize,
    pub kapc_state_process_offset: usize,
    pub cached_system_cr3: u64,
}

static CACHED_SYSTEM_EPROCESS: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static HV_OFFSETS_PTR: AtomicPtr<HvOffsets> = AtomicPtr::new(core::ptr::null_mut());

/// 等价于 C 侧 `PsInitialSystemProcess` 所指向的 System 进程 `EPROCESS`（内核虚拟地址）。
unsafe fn ps_initial_system_process_bytes() -> *mut u8 {
    let mut cached = CACHED_SYSTEM_EPROCESS.load(Ordering::Acquire);
    if !cached.is_null() {
        return cached.cast::<u8>();
    }

    let mut process: PEPROCESS = core::ptr::null_mut();
    let st = unsafe { PsLookupProcessByProcessId(4usize as HANDLE, &mut process) };
    if !nt_success(st) || process.is_null() {
        return core::ptr::null_mut();
    }

    match CACHED_SYSTEM_EPROCESS.compare_exchange(
        core::ptr::null_mut(),
        process.cast(),
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(_) => {
            unsafe { ObfDereferenceObject(process.cast()) };
            process.cast()
        }
        Err(other) => {
            unsafe { ObDereferenceObject(process.cast()) };
            other.cast::<u8>()
        }
    }
}

/// `Ps*` 导出地址处的定长序言探测；与 `hv.cpp` 一致。返回
/// `(EPROCESS::UniqueProcessId, EPROCESS::ImageFileName, KAPC_STATE::Process)` 偏移。
unsafe fn probe_ps_star_offsets() -> Option<(usize, usize, usize)> {
    let ps_get_process_id = PsGetProcessId as *const u8;
    if unsafe {
        *ps_get_process_id != 0x48
            || *ps_get_process_id.add(1) != 0x8B
            || *ps_get_process_id.add(2) != 0x81
            || *ps_get_process_id.add(7) != 0xC3
    } {
        return None;
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
        return None;
    }

    let eprocess_image_file_name =
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
        return None;
    }

    let kapc_state_process_offset =
        unsafe { *ps_get_current_thread_process.add(12).cast::<u32>() as usize };

    Some((
        eprocess_unique_process_id_offset,
        eprocess_image_file_name,
        kapc_state_process_offset,
    ))
}

/// 填充 [`HvOffsets`]：先查表，失败则 `Ps*` 探测（与 `hv::find_offsets` 探测规则相同）。
pub unsafe fn find_offsets() -> bool {
    let existing = HV_OFFSETS_PTR.load(Ordering::Acquire);
    if !existing.is_null() {
        println!("[my-hv-driver] find_offsets: HV_OFFSETS_PTR already exists; skipping duplicate init.");
        return true;
    }

    println!("[my-hv-driver] find_offsets: starting offsets initialization...");

    let system_eprocess = unsafe { ps_initial_system_process_bytes() };
    if system_eprocess.is_null() {
        println!("[my-hv-driver] find_offsets: failed to get System EPROCESS; cannot init offsets.");
        return false;
    }

    let build = nt_build_number();
    match build {
        Some(b) => {
            println!(
                "[my-hv-driver] find_offsets: kernel build number: {b} (0x{b:x})"
            );
        }
        None => {
            println!(
                "[my-hv-driver] find_offsets: failed to detect NT build number (RtlGetVersion failed?)"
            );
        }
    }
    let from_table = build.and_then(offset_table::lookup);

    let (
        kprocess_directory_table_base_offset,
        kpcr_pcrb_offset,
        kprcb_current_thread_offset,
        kthread_apc_state_offset,
        eprocess_unique_process_id_offset,
        eprocess_image_file_name,
        kapc_state_process_offset,
    ) = if let Some(row) = from_table {
        println!("[my-hv-driver] find_offsets: offset_table lookup hit: {:?}", row);
        (
            row.kprocess_directory_table_base_offset,
            row.kpcr_pcrb_offset,
            row.kprcb_current_thread_offset,
            row.kthread_apc_state_offset,
            row.eprocess_unique_process_id_offset,
            row.eprocess_image_file_name,
            row.kapc_state_process_offset,
        )
    } else if let Some((u, img, kapc)) = unsafe { probe_ps_star_offsets() } {
        println!(
            "[my-hv-driver] find_offsets: offset_table miss; probing Ps* for offsets \
            => UniqueProcessId: 0x{:x}, ImageFileName: 0x{:x}, KAPC_STATE::Process: 0x{:x}",
            u, img, kapc
        );
        // 与 `hv.cpp` 在仅探测路径下的常量一致；`kthread_apc_state_offset` 参考工程未初始化，保持 0。
        (0x28usize, 0x180usize, 0x8usize, 0usize, u, img, kapc)
    } else {
        if let Some(b) = build {
            println!(
                "[my-hv-driver] find_offsets: offset_table and Ps* probe both failed; dwBuildNumber={b} (0x{b:x}). \
                Add the matching BuildRange in offset_table.rs."
            );
        } else {
            println!(
                "[my-hv-driver] find_offsets: RtlGetVersion and Ps* probe both failed."
            );
        }
        return false;
    };

    println!(
        "[my-hv-driver] find_offsets: using offsets: KProcess.DirectoryTableBase: 0x{:x}, Kpcr.Prcb: 0x{:x}, KPrcb.CurrentThread: 0x{:x}, \
        KThread.ApcState: 0x{:x}, EPROCESS.UniqueProcessId: 0x{:x}, EPROCESS.ImageFileName: 0x{:x}, KAPC_STATE.Process: 0x{:x}",
        kprocess_directory_table_base_offset,
        kpcr_pcrb_offset,
        kprcb_current_thread_offset,
        kthread_apc_state_offset,
        eprocess_unique_process_id_offset,
        eprocess_image_file_name,
        kapc_state_process_offset,
    );

    let mut cached_system_cr3 = unsafe {
        *system_eprocess
            .add(kprocess_directory_table_base_offset)
            .cast::<u64>()
    };
    println!(
        "[my-hv-driver] find_offsets: sampled System EPROCESS CR3 = 0x{:x}",
        cached_system_cr3
    );

    // `HostPageTables::copy_kernel_half` 用 `DirectoryTableBase` 的低 12 位清 0 后作 PML4 物理基址。
    // DriverEntry 路径上当前一般为 System（PID 4），此时 `CR3` 与 System `EPROCESS` 中该字段应对应同一 PML4。
    // 若查表/采样与真实内核不一致，会映射到错误物理页并在拷贝 PML4 高半区时触发 0x50。
    let cr3_here = arch::read_cr3();
    println!(
        "[my-hv-driver] find_offsets: current CR3=0x{:x}, System EPROCESS CR3=0x{:x}",
        cr3_here, cached_system_cr3
    );
    if (cached_system_cr3 & !0xFFFu64) != (cr3_here & !0xFFFu64) {
        println!(
            "[my-hv-driver] find_offsets: System EPROCESS CR3 differs from current CR3; checking current PID."
        );
        let pid = unsafe { PsGetCurrentProcessId() };
        println!(
            "[my-hv-driver] find_offsets: current process PID = {}{}",
            pid as usize,
            if pid == (4usize as HANDLE) { " (System)" } else { "" }
        );
        if pid == (4usize as HANDLE) {
            println!(
                "[my-hv-driver] find_offsets: current process is System; overriding sampled CR3 with {:#x}.",
                cr3_here
            );
            cached_system_cr3 = cr3_here;
        }
    }

    let off = HvOffsets {
        system_eprocess: system_eprocess as usize,
        kprocess_directory_table_base_offset,
        kpcr_pcrb_offset,
        kprcb_current_thread_offset,
        kthread_apc_state_offset,
        eprocess_unique_process_id_offset,
        eprocess_image_file_name,
        kapc_state_process_offset,
        cached_system_cr3,
    };

    let p = Box::into_raw(Box::new(off));
    println!("[my-hv-driver] find_offsets: publishing new offsets pointer (CAS init)...");
    match HV_OFFSETS_PTR.compare_exchange(
        core::ptr::null_mut(),
        p,
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => {
            println!("[my-hv-driver] find_offsets: initialization succeeded.");
            true
        }
        Err(other) => {
            println!("[my-hv-driver] find_offsets: concurrent CAS race; dropping allocated object.");
            unsafe { drop(Box::from_raw(p)) };
            let already = !other.is_null();
            println!(
                "[my-hv-driver] find_offsets: init {}; offsets already present.",
                if already { "idempotent" } else { "aborted" }
            );
            already
        }
    }
}

/// `hv::ghv.system_cr3.flags`：System 进程页目录寄存器值。
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
    let cr3 = unsafe { *(e.add(off.kprocess_directory_table_base_offset).cast::<u64>()) };
    unsafe { ObDereferenceObject(eprocess.cast()) };
    Some(cr3)
}
