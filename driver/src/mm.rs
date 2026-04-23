//! 内核内存与物理地址辅助。对应参考实现 `hv/hv/mm.cpp` / `hv/hv/page-tables.cpp`。
#![allow(dead_code)]

use wdk_sys::ntddk::{
    MmAllocateContiguousMemorySpecifyCache, MmFreeContiguousMemory, MmGetPhysicalAddress,
    MmGetVirtualForPhysical,
};
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

pub unsafe fn alloc_contiguous_pages(page_count: usize) -> Option<*mut u8> {
    if page_count == 0 {
        return None;
    }
    let bytes = (PAGE_SIZE as usize).saturating_mul(page_count) as SIZE_T;
    let p = unsafe {
        MmAllocateContiguousMemorySpecifyCache(
            bytes,
            phys_zero(),
            phys_max(),
            phys_zero(),
            1 as MEMORY_CACHING_TYPE,
        )
    };
    if p.is_null() { None } else { Some(p.cast()) }
}

pub unsafe fn free_contiguous(ptr: *mut u8) {
    if !ptr.is_null() {
        unsafe { MmFreeContiguousMemory(ptr.cast()) };
    }
}

pub unsafe fn virtual_for_physical(pa: u64) -> *mut u8 {
    let p = PHYSICAL_ADDRESS {
        QuadPart: pa as i64,
    };
    unsafe { MmGetVirtualForPhysical(p).cast() }
}

pub unsafe fn copy_from_physical(pa: u64, dst: *mut u8, len: usize) -> bool {
    if dst.is_null() || len == 0 {
        return false;
    }
    let src = unsafe { virtual_for_physical(pa) };
    if src.is_null() {
        return false;
    }
    unsafe { core::ptr::copy_nonoverlapping(src, dst, len) };
    true
}

pub unsafe fn copy_to_physical(pa: u64, src: *const u8, len: usize) -> bool {
    if src.is_null() || len == 0 {
        return false;
    }
    let dst = unsafe { virtual_for_physical(pa) };
    if dst.is_null() {
        return false;
    }
    unsafe { core::ptr::copy_nonoverlapping(src, dst, len) };
    true
}
