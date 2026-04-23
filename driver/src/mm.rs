//! 内核内存与物理地址辅助。对应参考实现 `hv/hv/mm.cpp` / `hv/hv/page-tables.cpp`。

use wdk_sys::ntddk::{MmAllocateContiguousMemorySpecifyCache, MmFreeContiguousMemory, MmGetPhysicalAddress};
use wdk_sys::{MEMORY_CACHING_TYPE, PHYSICAL_ADDRESS, SIZE_T};

const PAGE_SIZE: SIZE_T = 4096;

fn phys_zero() -> PHYSICAL_ADDRESS {
    PHYSICAL_ADDRESS { QuadPart: 0 }
}

fn phys_max() -> PHYSICAL_ADDRESS {
    PHYSICAL_ADDRESS { QuadPart: -1 }
}

/// 分配单页连续物理内存（4KiB 对齐），适用于 VMXON / VMCS 区域。
pub unsafe fn alloc_contiguous_page() -> Option<*mut u8> {
    let p = unsafe {
        MmAllocateContiguousMemorySpecifyCache(
            PAGE_SIZE,
            phys_zero(),
            phys_max(),
            phys_zero(),
            1 as MEMORY_CACHING_TYPE,
        )
    };
    if p.is_null() {
        None
    } else {
        Some(p.cast())
    }
}

pub unsafe fn free_contiguous_page(ptr: *mut u8) {
    if !ptr.is_null() {
        unsafe { MmFreeContiguousMemory(ptr.cast()) };
    }
}

pub unsafe fn physical_address(ptr: *const u8) -> u64 {
    unsafe { MmGetPhysicalAddress(ptr.cast_mut().cast()).QuadPart as u64 }
}
