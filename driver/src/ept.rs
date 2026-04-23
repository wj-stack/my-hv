//! EPT 基础状态与最小初始化。对应 `hv/hv/ept.*`（PML4→PDPT→PD，2MiB 大页恒等映射）。
#![allow(unsafe_op_in_unsafe_fn)]

use alloc::vec::Vec;

use crate::mm::{self, free_contiguous_page};
use crate::mtrr;
use crate::vmx;

pub const EPT_MEMTYPE_WB: u64 = 6;

const EPT_R: u64 = 1;
const EPT_W: u64 = 2;
const EPT_X: u64 = 4;
/// 与 `hv/hv/ept.cpp` 中 `user_mode_execute = 1` 对应（EPT 大页项 bit 10）。
const EPT_USER_MODE_EXECUTE: u64 = 1 << 10;
/// 非叶表项：bits 2:0 不能全为 1（全 1 表示叶项映射页）。
const EPT_NONLEAF: u64 = EPT_R | EPT_W;
const EPT_LARGE: u64 = 1 << 7;
const EPT_IGNORE_PAT: u64 = 1 << 6;
const SIZE_2M: u64 = 2 * 1024 * 1024;

/// 与参考 `hv` 的 `ept_pd_count` 对齐：64 个 PD → 64GiB 恒等范围。
pub const EPT_PD_COUNT: usize = 64;
const TABLE_PAGES: usize = 2 + EPT_PD_COUNT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EptPointer(pub u64);

impl EptPointer {
    pub fn new(pml4_pa: u64) -> Self {
        let walk_len_minus_one = 3u64 << 3;
        let mem_type = EPT_MEMTYPE_WB;
        Self((pml4_pa & !0xFFF) | walk_len_minus_one | mem_type)
    }
}

/// EPT 页钩（4Ki 拆分后关执行，参考 `hv/hv/ept.cpp`）。
#[derive(Clone, Copy, Debug)]
pub struct EptHookEntry {
    pub orig_page_pfn: u32,
    pub exec_page_pfn: u32,
    pub active: bool,
}

/// 监控读写的 GPA 区（`hv` MMR 简化：固定条数，按 guest 页对齐）。
#[derive(Clone, Copy, Debug)]
pub struct EptMmr {
    pub start_gpa: u64,
    pub size: u32,
    pub read: bool,
    pub write: bool,
    pub execute: bool,
    pub in_use: bool,
}

const MAX_HOOKS: usize = 32;
const MAX_MMR: usize = 16;

pub struct EptState {
    /// 连续区域：[PML4][PDPT][PD_0]…[PD_{EPT_PD_COUNT-1}]，每页 4KiB。
    pub table_base: *mut u8,
    pub table_pages: usize,
    pub pml4_pa: u64,
    pub eptp: EptPointer,
    /// 拆分/隐藏页时额外分配的表页，在 `release` 中释放。
    pub extra_pages: Vec<*mut u8>,
    /// 隐藏页时使用的黑页（全零）。
    pub dummy_page: *mut u8,
    pub hooks: [EptHookEntry; MAX_HOOKS],
    pub mmr: [EptMmr; MAX_MMR],
}

/// 与 C++ `ept` 2MiB 大叶一致：`memory_type` 来自 MTRR（`ignore_pat` 不置位）。
#[inline]
fn ept_leaf_2mb_from_mtrr(gpa: u64, m: &mtrr::MtrrData) -> u64 {
    let aligned = gpa & !(SIZE_2M - 1);
    let mt = u64::from(mtrr::calc_mtrr_mem_type(m, gpa, SIZE_2M)) & 0x7;
    aligned | EPT_R | EPT_W | EPT_X | EPT_USER_MODE_EXECUTE | EPT_LARGE | (mt << 3)
}

impl EptState {
    /// 建立 64GiB 恒等 EPT（2MiB 大页），供启用 `ENABLE_EPT` 时使用。
    pub unsafe fn build_identity_skeleton() -> Option<Self> {
        let base = unsafe { mm::alloc_contiguous_pages(TABLE_PAGES)? };
        unsafe { core::ptr::write_bytes(base, 0, 4096 * TABLE_PAGES) };

        let base_pa = unsafe { mm::physical_address(base) };
        let pml4_pa = base_pa;
        let pdpt_pa = base_pa + 4096;

        let pml4 = base.cast::<u64>();
        unsafe { pml4.write(pdpt_pa | EPT_NONLEAF) };

        let pdpt = unsafe { base.add(4096).cast::<u64>() };
        for i in 0..EPT_PD_COUNT {
            let pd_page_pa = base_pa + 4096 * (2 + i as u64);
            unsafe {
                pdpt.add(i).write(pd_page_pa | EPT_NONLEAF);
            }
        }

        let mtrr = mtrr::read_mtrr_data();
        for pd_idx in 0..EPT_PD_COUNT {
            let pd = unsafe { base.add(4096 * (2 + pd_idx)).cast::<u64>() };
            for slot in 0..512 {
                let gpa = (pd_idx as u64 * 512 + slot as u64).saturating_mul(SIZE_2M);
                unsafe {
                    pd.add(slot).write(ept_leaf_2mb_from_mtrr(gpa, &mtrr));
                }
            }
        }

        let dummy_page = match unsafe { mm::alloc_contiguous_page() } {
            Some(p) => {
                unsafe { core::ptr::write_bytes(p, 0, 4096) };
                p
            }
            None => return None,
        };

        Some(Self {
            table_base: base,
            table_pages: TABLE_PAGES,
            pml4_pa,
            eptp: EptPointer::new(pml4_pa),
            extra_pages: Vec::new(),
            dummy_page,
            hooks: [EptHookEntry {
                orig_page_pfn: 0,
                exec_page_pfn: 0,
                active: false,
            }; MAX_HOOKS],
            mmr: [EptMmr {
                start_gpa: 0,
                size: 0,
                read: false,
                write: false,
                execute: false,
                in_use: false,
            }; MAX_MMR],
        })
    }

    /// 在 VMX root 下对当前 EPTP 做单上下文失效（修改页表后应调用）。
    pub unsafe fn invept_single_context(&self) -> bool {
        unsafe { vmx::invept_single_context(self.eptp.0) }
    }

    /// 按当前 host MTRR 重算 2MiB 叶项并 `INVEPT`（对应 C++ `update_ept_memory_type` 的恒等大页路径）。
    pub fn refresh_all_memory_types(&mut self) {
        if self.table_base.is_null() {
            return;
        }
        let mtrr = mtrr::read_mtrr_data();
        for pd_idx in 0..EPT_PD_COUNT {
            let pd = unsafe { self.table_base.add(4096 * (2 + pd_idx)).cast::<u64>() };
            for slot in 0..512 {
                let gpa = (pd_idx as u64 * 512 + slot as u64) * SIZE_2M;
                let leaf = ept_leaf_2mb_from_mtrr(gpa, &mtrr);
                unsafe {
                    pd.add(slot).write(leaf);
                }
            }
        }
        unsafe {
            let _ = self.invept_single_context();
        }
    }

    /// 在 `gfn` 对应页上关执行，用于 EPT 钩；失败时返回 `false`。
    pub unsafe fn clear_execute_for_page(&mut self, gpa: u64) -> bool {
        if self.table_base.is_null() {
            return false;
        }
        if !self.split_2m_if_needed(gpa) {
            return false;
        }
        if let Some(pte) = self.mut_pte_4k(gpa) {
            let mut v = unsafe { *pte };
            v &= !(EPT_X | (1u64 << 10));
            if (v & 0x7) == 0 {
                v |= EPT_R;
            }
            unsafe { *pte = v };
        } else {
            return false;
        }
        self.invept_single_context()
    }

    /// 将 `gpa` 的 4Ki 映射到 `dummy` 物页（隐藏真页）。
    pub unsafe fn point_gpa_to_dummy(&mut self, gpa: u64) -> bool {
        if self.dummy_page.is_null() {
            return false;
        }
        let dummy = unsafe { mm::physical_address(self.dummy_page) } >> 12;
        if !self.split_2m_if_needed(gpa) {
            return false;
        }
        if let Some(pte) = self.mut_pte_4k(gpa) {
            let e = (dummy << 12) | EPT_R | EPT_W | (EPT_MEMTYPE_WB << 3) | EPT_IGNORE_PAT;
            unsafe { *pte = e };
        } else {
            return false;
        }
        self.invept_single_context()
    }

    unsafe fn split_2m_if_needed(&mut self, gpa: u64) -> bool {
        let pd_idx = (gpa >> 30) as usize;
        let slot = ((gpa >> 21) & 0x1FF) as usize;
        if pd_idx >= EPT_PD_COUNT {
            return false;
        }
        let pde = unsafe {
            self.table_base
                .add(4096 * (2 + pd_idx))
                .cast::<u64>()
                .add(slot)
        };
        let val = unsafe { *pde };
        if (val & EPT_LARGE) == 0 {
            return true;
        }
        let Some(pt) = (unsafe { mm::alloc_contiguous_page() }) else {
            return false;
        };
        unsafe { core::ptr::write_bytes(pt, 0, 4096) };
        self.extra_pages.push(pt);
        let pt_pa = unsafe { mm::physical_address(pt) };
        let base_2m = gpa & !(SIZE_2M - 1);
        let pt = pt.cast::<u64>();
        for k in 0..512 {
            let g = base_2m + (k as u64 * 4096);
            let leaf = (g >> 12) << 12 | EPT_R | EPT_W | EPT_X | (EPT_MEMTYPE_WB << 3) | EPT_IGNORE_PAT;
            unsafe {
                pt.add(k).write(leaf);
            }
        }
        let nonleaf = (pt_pa & !0xFFF) | EPT_R | EPT_W;
        unsafe { *pde = nonleaf };
        true
    }

    fn mut_pte_4k(&mut self, gpa: u64) -> Option<*mut u64> {
        let pd_idx = (gpa >> 30) as usize;
        let slot = ((gpa >> 21) & 0x1FF) as usize;
        if pd_idx >= EPT_PD_COUNT {
            return None;
        }
        let pde = unsafe {
            *self
                .table_base
                .add(4096 * (2 + pd_idx))
                .cast::<u64>()
                .add(slot)
        };
        if (pde & EPT_LARGE) != 0 {
            return None;
        }
        let pt_pfn = (pde >> 12) & 0xFFFF_FFF;
        if pt_pfn == 0 {
            return None;
        }
        let hva = unsafe { mm::virtual_for_physical(pt_pfn << 12) };
        if hva.is_null() {
            return None;
        }
        let idx = (gpa >> 12) & 0x1FF;
        Some(unsafe { hva.cast::<u64>().add(idx as usize) })
    }

    pub unsafe fn release(&mut self) {
        if !self.dummy_page.is_null() {
            unsafe { free_contiguous_page(self.dummy_page) };
        }
        self.dummy_page = core::ptr::null_mut();
        for p in self.extra_pages.drain(..) {
            if !p.is_null() {
                unsafe { free_contiguous_page(p) };
            }
        }
        if !self.table_base.is_null() && self.table_pages != 0 {
            unsafe { mm::free_contiguous(self.table_base) };
        }
        self.table_base = core::ptr::null_mut();
        self.table_pages = 0;
        self.pml4_pa = 0;
        self.eptp = EptPointer(0);
    }
}
