//! 与 `hv/hv/page-tables.{h,cpp}` 一致的 host CR3 页表：恒等物理映射 + 拷贝 System 进程 PML4 高半区（内核 VA）。
//!
//! **不得**用默认 `#[global_allocator] WdkAllocator` + `Box::new(HostPageTables)`：`wdk-alloc` 目前
//! **忽略 `Layout::align()`**（见 `WdkAllocator` 源码 FIXME），池块通常仅 ~16 字节对齐。本结构要求
//! **4KiB 对齐**，否则 `pml4` / `phys_pdpt` / 各 `phys_pds[i]` 会跨页，`MmGetPhysicalAddress` 与真实
//! 页表布局不一致，易在 `copy_kernel_half` 等处触发 `PAGE_FAULT_IN_NONPAGED_AREA`。
#![allow(unsafe_op_in_unsafe_fn)]

use core::mem::size_of;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

use crate::mm::{free_contiguous, physical_address, virtual_for_physical};

/// 与 `host_physical_memory_pd_count` 一致：64 个 PD → 64GiB 2MiB 恒等映射。
pub const HOST_PHYS_PD_COUNT: usize = 64;
/// `host_physical_memory_pml4_idx`（255）。
pub const HOST_PHYS_PML4_IDX: usize = 255;
/// 用户态恒等物理窗口基址（`host_physical_memory_base`）。
pub const HOST_PHYS_BASE_VA: u64 = (HOST_PHYS_PML4_IDX as u64) << 39;

const PAGE_SIZE: usize = 4096;

#[repr(C, align(4096))]
pub struct HostPageTables {
    pub pml4: [u64; 512],
    pub phys_pdpt: [u64; 512],
    pub phys_pds: [[u64; 512]; HOST_PHYS_PD_COUNT],
}

/// 由 [`HostPageTables::prepare`] 分配：连续、页对齐物理内存（与 C++ 静态 `alignas(0x1000)` 布局等价）。
pub struct HostPageTablesBox {
    ptr: NonNull<HostPageTables>,
}

impl Deref for HostPageTablesBox {
    type Target = HostPageTables;

    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

impl DerefMut for HostPageTablesBox {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { self.ptr.as_mut() }
    }
}

impl Drop for HostPageTablesBox {
    fn drop(&mut self) {
        unsafe {
            free_contiguous(self.ptr.as_ptr().cast());
        }
    }
}

impl HostPageTables {
    fn alloc_size_bytes() -> usize {
        let sz = size_of::<HostPageTables>();
        let n_pages = sz.div_ceil(PAGE_SIZE);
        n_pages.saturating_mul(PAGE_SIZE)
    }

    /// `prepare_host_page_tables`：在页对齐连续内存上清零，再映射物理区并拷贝内核 PML4[256..512]。
    pub fn prepare(system_cr3: u64) -> Option<HostPageTablesBox> {
        let alloc_bytes = Self::alloc_size_bytes();
        let n_pages = alloc_bytes / PAGE_SIZE;
        let raw = unsafe { crate::mm::alloc_contiguous_pages(n_pages)? };
        debug_assert_eq!(raw.align_offset(PAGE_SIZE), 0);

        unsafe {
            core::ptr::write_bytes(raw, 0, alloc_bytes);
        }
        let pt = unsafe { &mut *raw.cast::<HostPageTables>() };
        let ok = unsafe {
            map_physical_memory(pt).is_some() && copy_kernel_half(pt, system_cr3).is_some()
        };
        if !ok {
            unsafe {
                free_contiguous(raw);
            }
            return None;
        }
        Some(HostPageTablesBox {
            ptr: unsafe { NonNull::new_unchecked(raw.cast()) },
        })
    }

    /// 写入 `VMCS_HOST_CR3` 的完整 CR3（页对齐物理地址，低 12 位为 0）。
    #[inline]
    pub fn cr3_value(&self) -> u64 {
        unsafe { physical_address(self.pml4.as_ptr().cast()) }
    }
}

unsafe fn map_physical_memory(pt: &mut HostPageTables) -> Option<()> {
    // Match `hv/page-tables.cpp::map_physical_memory`: present+write, U/S=0 (supervisor only).
    // Using `| 0x7` would set bit 2 and allow user-mode access to the whole physical window.
    const PRESENT_RW_SUPERVISOR: u64 = 0x3;
    let pdpt_pa = unsafe { physical_address(pt.phys_pdpt.as_ptr().cast()) };
    pt.pml4[HOST_PHYS_PML4_IDX] = pdpt_pa | PRESENT_RW_SUPERVISOR;

    for i in 0..HOST_PHYS_PD_COUNT {
        let pd_pa = unsafe { physical_address(pt.phys_pds[i].as_ptr().cast()) };
        pt.phys_pdpt[i] = pd_pa | PRESENT_RW_SUPERVISOR;
        for j in 0..512 {
            let pfn = ((i as u64) << 9) + (j as u64);
            pt.phys_pds[i][j] = (pfn << 21) | (1 << 7) | 0x83;
        }
    }
    Some(())
}

unsafe fn copy_kernel_half(pt: &mut HostPageTables, system_cr3: u64) -> Option<()> {
    let pml4_pa = system_cr3 & !0xFFFu64;
    let guest_pml4 = unsafe { virtual_for_physical(pml4_pa) };
    if guest_pml4.is_null() {
        return None;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(
            guest_pml4.add(256 * 8),
            pt.pml4.as_mut_ptr().add(256).cast(),
            256 * 8,
        );
    }
    Some(())
}
