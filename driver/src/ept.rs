//! EPT 基础状态与最小初始化。对应 `hv/hv/ept.*`（PML4→PDPT→PD，2MiB 大页恒等映射）。

use crate::mm;
use crate::vmx;

pub const EPT_MEMTYPE_WB: u64 = 6;

const EPT_R: u64 = 1;
const EPT_W: u64 = 2;
const EPT_X: u64 = 4;
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

pub struct EptState {
    /// 连续区域：[PML4][PDPT][PD_0]…[PD_{EPT_PD_COUNT-1}]，每页 4KiB。
    pub table_base: *mut u8,
    pub table_pages: usize,
    pub pml4_pa: u64,
    pub eptp: EptPointer,
}

#[inline]
fn ept_leaf_2mb(phys_2m_base: u64) -> u64 {
    let aligned = phys_2m_base & !(SIZE_2M - 1);
    aligned | EPT_R | EPT_W | EPT_X | EPT_LARGE | (EPT_MEMTYPE_WB << 3) | EPT_IGNORE_PAT
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

        for pd_idx in 0..EPT_PD_COUNT {
            let pd = unsafe { base.add(4096 * (2 + pd_idx)).cast::<u64>() };
            for slot in 0..512 {
                let gpa = (pd_idx as u64 * 512 + slot as u64).saturating_mul(SIZE_2M);
                unsafe {
                    pd.add(slot).write(ept_leaf_2mb(gpa));
                }
            }
        }

        Some(Self {
            table_base: base,
            table_pages: TABLE_PAGES,
            pml4_pa,
            eptp: EptPointer::new(pml4_pa),
        })
    }

    /// 在 VMX root 下对当前 EPTP 做单上下文失效（修改页表后应调用）。
    pub unsafe fn invept_single_context(&self) -> bool {
        unsafe { vmx::invept_single_context(self.eptp.0) }
    }

    pub unsafe fn release(&mut self) {
        if !self.table_base.is_null() && self.table_pages != 0 {
            unsafe { mm::free_contiguous(self.table_base) };
        }
        self.table_base = core::ptr::null_mut();
        self.table_pages = 0;
        self.pml4_pa = 0;
        self.eptp = EptPointer(0);
    }
}
