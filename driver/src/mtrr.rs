//! MTRR 与 EPT 内存类型同步（参考 `hv/hv/mtrr.*`、`ept::update_ept_memory_type`）。

use crate::arch;

const IA32_MTRR_CAP: u32 = 0x0000_00FE;
const IA32_MTRR_DEF_TYPE: u32 = 0x0000_02FF;
const IA32_MTRR_PHYS_BASE0: u32 = 0x0000_0200;
const IA32_MTRR_PHYS_MASK0: u32 = 0x0000_0201;

const MTRR_MEM_UC: u8 = 0;
const MTRR_MEM_INVALID: u8 = 0xFF;

const EPT_4K: u64 = 0x1000;

/// 与 C++ `mtrr_data` 等价的 MTRR 镜像（只保留恒等 EPT 所需的 variable-range 部分）。
pub struct MtrrData {
    pub cap: u64,
    pub def_type: u64,
    pub variable_base: [u64; 8],
    pub variable_mask: [u64; 8],
    pub var_count: usize,
}

/// 与 C++ `read_mtrr_data` 对齐。
pub fn read_mtrr_data() -> MtrrData {
    let cap = unsafe { arch::rdmsr(IA32_MTRR_CAP) };
    let def_type = unsafe { arch::rdmsr(IA32_MTRR_DEF_TYPE) };
    let n = (cap & 0xFF) as usize;
    let n = n.min(8);
    let mut variable_base = [0u64; 8];
    let mut variable_mask = [0u64; 8];
    let mut var_count = 0usize;
    for i in 0..n {
        let mask = unsafe { arch::rdmsr(IA32_MTRR_PHYS_MASK0 + (i as u32) * 2) };
        if (mask & (1 << 11)) == 0 {
            continue;
        }
        if var_count < 8 {
            variable_base[var_count] = unsafe { arch::rdmsr(IA32_MTRR_PHYS_BASE0 + (i as u32) * 2) };
            variable_mask[var_count] = mask;
            var_count += 1;
        }
    }
    MtrrData {
        cap,
        def_type,
        variable_base,
        variable_mask,
        var_count,
    }
}

/// 单 4K 页帧号 `pfn` 的 MTRR 派生类型。对应 C++ 内部 `calc_mtrr_mem_type`（fixed-range 分支与参考实现一样返回 `UC` 占位）。
fn mem_type_for_pfn(m: &MtrrData, pfn: u64) -> u8 {
    if (m.def_type & (1 << 11)) == 0 {
        return MTRR_MEM_UC;
    }
    if pfn < 0x100
        && (m.cap & (1 << 8)) != 0
        && (m.def_type & (1 << 10)) != 0
    {
        return MTRR_MEM_UC;
    }
    let mut curr = MTRR_MEM_INVALID;
    for i in 0..m.var_count {
        let base = m.variable_base[i];
        let mask = m.variable_mask[i];
        let base_pfn = (base >> 12) & 0xFFFF_FFFF_FFFFF;
        let mask_pfn = (mask >> 12) & 0xFFFF_FFFF_FFFFF;
        if (pfn & mask_pfn) == (base_pfn & mask_pfn) {
            let t = (base as u8) & 0x7;
            if t == MTRR_MEM_UC {
                return MTRR_MEM_UC;
            }
            if t < curr {
                curr = t;
            }
        }
    }
    if curr == MTRR_MEM_INVALID {
        (m.def_type as u8) & 0x7
    } else {
        curr
    }
}

/// 对 `[address, address+size)` 内逐 4K 求交集规则下的最差类型。对应 C++ `calc_mtrr_mem_type(mtrrs, address, size)`。
pub fn calc_mtrr_mem_type(m: &MtrrData, address: u64, size: u64) -> u8 {
    let addr = address & !0xFFF;
    let mut sz = (size + 0xFFF) & !0xFFF;
    if sz < EPT_4K {
        sz = EPT_4K;
    }
    let end = addr.saturating_add(sz);
    let mut curr = MTRR_MEM_INVALID;
    let mut a = addr;
    while a < end {
        let t = mem_type_for_pfn(m, a >> 12);
        if t == MTRR_MEM_UC {
            return MTRR_MEM_UC;
        }
        if t < curr {
            curr = t;
        }
        a = a.saturating_add(EPT_4K);
    }
    if curr == MTRR_MEM_INVALID {
        MTRR_MEM_UC
    } else {
        curr
    }
}

/// 是否为 MTRR/相关 MSR（写拦截需重刷 EPT）。
pub fn is_mtrr_msr(msr: u32) -> bool {
    if msr == 0xFE || msr == 0x2FF {
        return true;
    }
    if (0x200..=0x20F).contains(&msr) {
        return true;
    }
    if (0x250..=0x26F).contains(&msr) {
        return true;
    }
    false
}
