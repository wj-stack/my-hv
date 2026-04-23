//! 段描述符与选择子辅助。对应 `hv/hv/segment.*`。
#![allow(dead_code)]

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
        _ => value = 0,
    }
    value
}

pub fn read_segment_selectors() -> SegmentSelectors {
    let tr: u16;
    // SAFETY: 当前 CPU 上读取段寄存器与 TR。
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
