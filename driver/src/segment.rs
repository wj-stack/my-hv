//! 段描述符与选择子辅助。对应 `hv/hv/segment.*`。
#![allow(dead_code)]

use crate::gdt::DescriptorTablePointer;

/// VMCS 中使用的访问权格式（Intel SDM Table 24-3）；bit16 = unusable。
pub const VMCS_SEG_UNUSABLE: u32 = 1 << 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentSelectors {
    pub cs: u16,
    pub ss: u16,
    pub ds: u16,
    pub es: u16,
    pub fs: u16,
    pub gs: u16,
    pub tr: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedSegment {
    pub selector: u16,
    pub base: u64,
    pub limit: u32,
    pub access_rights: u32,
}

#[inline]
unsafe fn read_seg(seg: &str) -> u16 {
    let value: u16;
    match seg {
        "cs" => unsafe { core::arch::asm!("mov {0:x}, cs", out(reg) value, options(nomem, nostack, preserves_flags)) },
        "ss" => unsafe { core::arch::asm!("mov {0:x}, ss", out(reg) value, options(nomem, nostack, preserves_flags)) },
        "ds" => unsafe { core::arch::asm!("mov {0:x}, ds", out(reg) value, options(nomem, nostack, preserves_flags)) },
        "es" => unsafe { core::arch::asm!("mov {0:x}, es", out(reg) value, options(nomem, nostack, preserves_flags)) },
        "fs" => unsafe { core::arch::asm!("mov {0:x}, fs", out(reg) value, options(nomem, nostack, preserves_flags)) },
        "gs" => unsafe { core::arch::asm!("mov {0:x}, gs", out(reg) value, options(nomem, nostack, preserves_flags)) },
        _ => unsafe { core::hint::unreachable_unchecked() },
    }
    value
}

pub fn read_segment_selectors() -> SegmentSelectors {
    let tr: u16;
    unsafe {
        core::arch::asm!("str {0:x}", out(reg) tr, options(nomem, nostack, preserves_flags));
        SegmentSelectors {
            cs: read_seg("cs"),
            ss: read_seg("ss"),
            ds: read_seg("ds"),
            es: read_seg("es"),
            fs: read_seg("fs"),
            gs: read_seg("gs"),
            tr,
        }
    }
}

#[inline]
fn gdtr_limit(gdtr: &DescriptorTablePointer) -> u16 {
    unsafe { core::ptr::addr_of!(gdtr.limit).read_unaligned() }
}

#[inline]
fn gdtr_base(gdtr: &DescriptorTablePointer) -> u64 {
    unsafe { core::ptr::addr_of!(gdtr.base).read_unaligned() }
}

/// 从 GDT 解析段；`selector==0` 得到 unusable 空描述符。
pub fn parse_segment(gdtr: &DescriptorTablePointer, selector: u16) -> ParsedSegment {
    if selector == 0 {
        return ParsedSegment {
            selector: 0,
            base: 0,
            limit: 0,
            access_rights: VMCS_SEG_UNUSABLE,
        };
    }

    let idx = (selector as usize) & 0xFFF8;
    let lim = gdtr_limit(gdtr) as usize;
    if idx > lim {
        return ParsedSegment {
            selector,
            base: 0,
            limit: 0,
            access_rights: VMCS_SEG_UNUSABLE,
        };
    }

    let gdt = gdtr_base(gdtr) as *const u8;
    let mut raw = [0u8; 16];
    unsafe {
        core::ptr::copy_nonoverlapping(gdt.add(idx), raw.as_mut_ptr(), 8);
    }

    let ty = raw[5] & 0x0F;
    let s = (raw[5] >> 4) & 1;
    let present = (raw[5] >> 7) & 1;

    // 64-bit TSS（16 字节系统段）
    if s == 0 && (ty == 0x9 || ty == 0xB) {
        if idx + 16 <= lim {
            unsafe {
                core::ptr::copy_nonoverlapping(gdt.add(idx + 8), raw[8..].as_mut_ptr(), 8);
            }
        }
        return parse_system_segment_16(selector, &raw, present);
    }

    parse_legacy_segment(selector, &raw[..8], present)
}

fn parse_legacy_segment(selector: u16, raw: &[u8], present: u8) -> ParsedSegment {
    let limit_lo = u16::from_le_bytes([raw[0], raw[1]]) as u32;
    let base_lo = u16::from_le_bytes([raw[2], raw[3]]) as u64;
    let base_mid = raw[4] as u64;
    let avl_byte = raw[6];
    let base_hi = raw[7] as u64;
    let mut limit = limit_lo | (((avl_byte & 0x0F) as u32) << 16);
    let base = base_lo | (base_mid << 16) | (base_hi << 24);
    let g = (avl_byte & 0x80) != 0;
    if g {
        limit = (limit << 12) | 0xFFF;
    }

    let ar = vmcs_access_rights_from_descriptor(raw[5], avl_byte, present);

    ParsedSegment {
        selector,
        base,
        limit,
        access_rights: ar,
    }
}

fn parse_system_segment_16(selector: u16, raw: &[u8; 16], present: u8) -> ParsedSegment {
    let limit_lo = u16::from_le_bytes([raw[0], raw[1]]) as u32;
    let base_lo = u16::from_le_bytes([raw[2], raw[3]]) as u64;
    let base_mid = raw[4] as u64;
    let avl_byte = raw[6];
    let limit_hi = (avl_byte & 0x0F) as u32;
    let mut limit = limit_lo | (limit_hi << 16);
    let base_hi = raw[7] as u64;
    let base_mid2 = u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]) as u64;
    let base = base_lo | (base_mid << 16) | (base_hi << 24) | (base_mid2 << 32);
    let g = (avl_byte & 0x80) != 0;
    if g {
        limit = (limit << 12) | 0xFFF;
    }
    let ar = vmcs_access_rights_from_descriptor(raw[5], avl_byte, present);
    ParsedSegment {
        selector,
        base,
        limit,
        access_rights: ar,
    }
}

#[inline]
fn vmcs_access_rights_from_descriptor(access: u8, avl_gran: u8, present: u8) -> u32 {
    let ty = (access & 0xF) as u32;
    let s = ((access >> 4) & 1) as u32;
    let dpl = ((access >> 5) & 3) as u32;
    let p = ((access >> 7) & 1) as u32;
    let avl = ((avl_gran >> 4) & 1) as u32;
    let l = ((avl_gran >> 5) & 1) as u32;
    let db = ((avl_gran >> 6) & 1) as u32;
    let g = ((avl_gran >> 7) & 1) as u32;
    let mut ar = ty | (s << 4) | (dpl << 5) | (p << 7) | (avl << 12) | (l << 13) | (db << 14) | (g << 15);
    if present == 0 {
        ar |= VMCS_SEG_UNUSABLE;
    }
    ar
}

/// LDTR：若 `sldt` 为 0 则 unusable。
pub fn parse_ldtr(gdtr: &DescriptorTablePointer) -> ParsedSegment {
    let sel: u16;
    unsafe {
        core::arch::asm!("sldt {0:x}", out(reg) sel, options(nomem, nostack, preserves_flags));
    }
    parse_segment(gdtr, sel)
}
