//! 与 `hv/hv/page-tables.{h,cpp}` 一致的 host CR3 页表：恒等物理映射 + 拷贝 System 进程 PML4 高半区（内核 VA）。
#![allow(unsafe_op_in_unsafe_fn)]

extern crate alloc;
use alloc::boxed::Box;

use crate::mm::{physical_address, virtual_for_physical};

/// 与 `host_physical_memory_pd_count` 一致：64 个 PD → 64GiB 2MiB 恒等映射。
pub const HOST_PHYS_PD_COUNT: usize = 64;
/// `host_physical_memory_pml4_idx`（255）。
pub const HOST_PHYS_PML4_IDX: usize = 255;
/// 用户态恒等物理窗口基址（`host_physical_memory_base`）。
pub const HOST_PHYS_BASE_VA: u64 = (HOST_PHYS_PML4_IDX as u64) << 39;

#[repr(C, align(4096))]
pub struct HostPageTables {
    pub pml4: [u64; 512],
    pub phys_pdpt: [u64; 512],
    pub phys_pds: [[u64; 512]; HOST_PHYS_PD_COUNT],
}

impl HostPageTables {
    /// `prepare_host_page_tables`：清零后映射物理区并拷贝内核 PML4[256..512]。
    pub fn prepare(system_cr3: u64) -> Option<Box<Self>> {
        let mut pt = Box::new(Self {
            pml4: [0; 512],
            phys_pdpt: [0; 512],
            phys_pds: [[0; 512]; HOST_PHYS_PD_COUNT],
        });
        unsafe {
            map_physical_memory(&mut pt)?;
            copy_kernel_half(&mut pt, system_cr3)?;
        }
        Some(pt)
    }

    /// 写入 `VMCS_HOST_CR3` 的完整 CR3（页对齐物理地址，低 12 位为 0）。
    #[inline]
    pub fn cr3_value(&self) -> u64 {
        unsafe { physical_address(self.pml4.as_ptr().cast()) }
    }
}

unsafe fn map_physical_memory(pt: &mut HostPageTables) -> Option<()> {
    let pdpt_pa = unsafe { physical_address(pt.phys_pdpt.as_ptr().cast()) };
    pt.pml4[HOST_PHYS_PML4_IDX] = pdpt_pa | 0x7;

    for i in 0..HOST_PHYS_PD_COUNT {
        let pd_pa = unsafe { physical_address(pt.phys_pds[i].as_ptr().cast()) };
        pt.phys_pdpt[i] = pd_pa | 0x7;
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
