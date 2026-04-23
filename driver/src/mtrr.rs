//! MTRR 与 EPT 内存类型同步（参考 `hv/hv/mtrr.*`、`ept::update_ept_memory_type`）。

use crate::ept::EptState;

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

/// 在客户机写入 MTRR 相关 MSR 后刷新 EPT 并 `INVEPT`。
pub unsafe fn on_mtrr_msr_write(ept: &mut EptState) {
    ept.refresh_all_memory_types();
}
