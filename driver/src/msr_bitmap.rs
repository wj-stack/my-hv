//! MSR Bitmap 布局与 `hv/hv/vmx.inl` 中 `enable_exit_for_msr_read/write` 一致：
//! 每页 4096 字节：`rdmsr_low[1024]`、`wrmsr_low[1024]`、`rdmsr_high[1024]`、`wrmsr_high[1024]`。

use crate::ia32;

const REGION: usize = 1024;

/// `msr <= 0x1FFF` 与 `0xC0000000..=0xC0001FFF` 两片 Intel 定义区间。
#[inline]
fn enable_read(msr: u32, page: *mut u8, on: bool) {
    // SAFETY: `page` 指向整页，且索引在 REGION 内。
    unsafe {
        if msr <= 0x1FFF {
            let idx = (msr as usize / 8).min(REGION - 1);
            let bit = 1u8 << (msr & 7);
            let p = page.add(idx);
            if on {
                *p |= bit;
            } else {
                *p &= !bit;
            }
        } else if (0xC000_0000..=0xC000_1FFF).contains(&msr) {
            let off = (msr - 0xC000_0000) as usize;
            let idx = (off / 8).min(REGION - 1);
            let bit = 1u8 << ((off as u32) & 7);
            let p = page.add(REGION * 2 + idx);
            if on {
                *p |= bit;
            } else {
                *p &= !bit;
            }
        }
    }
}

#[inline]
fn enable_write(msr: u32, page: *mut u8, on: bool) {
    unsafe {
        if msr <= 0x1FFF {
            let idx = (msr as usize / 8).min(REGION - 1);
            let bit = 1u8 << (msr & 7);
            let p = page.add(REGION + idx);
            if on {
                *p |= bit;
            } else {
                *p &= !bit;
            }
        } else if (0xC000_0000..=0xC000_1FFF).contains(&msr) {
            let off = (msr - 0xC000_0000) as usize;
            let idx = (off / 8).min(REGION - 1);
            let bit = 1u8 << ((off as u32) & 7);
            let p = page.add(REGION * 3 + idx);
            if on {
                *p |= bit;
            } else {
                *p &= !bit;
            }
        }
    }
}

/// 清零整页并按 `prepare_external_structures` 置位：FEATURE_CONTROL 读拦截 + MTRR 写拦截。
pub unsafe fn prepare_msr_bitmap_page(page: *mut u8) {
    unsafe {
        core::ptr::write_bytes(page, 0, 4096);
    }
    enable_read(ia32::IA32_FEATURE_CONTROL, page, true);
    enable_mtrr_writes(page);
}

fn enable_mtrr_writes(page: *mut u8) {
    let cap = unsafe { crate::arch::rdmsr(0xFE) };
    let var_count = (cap & 0xFF) as u32;

    enable_write(0x2FF, page, true);

    if (cap & (1 << 8)) != 0 {
        enable_write(0x250, page, true);
        enable_write(0x258, page, true);
        enable_write(0x259, page, true);
        for i in 0..8 {
            enable_write(0x268 + i, page, true);
        }
    }

    for i in 0..var_count {
        enable_write(0x200 + i * 2, page, true);
        enable_write(0x201 + i * 2, page, true);
    }
}
