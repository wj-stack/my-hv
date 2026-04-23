//! 主机 IDT 辅助。对应 `hv/hv/idt.*`。

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Idtr {
    pub limit: u16,
    pub base: u64,
}

pub fn read_idtr() -> Idtr {
    let mut idtr = Idtr::default();
    // SAFETY: SIDT 仅读取当前 CPU IDTR。
    unsafe {
        core::arch::asm!(
            "sidt [{ptr}]",
            ptr = in(reg) &mut idtr,
            options(nostack, preserves_flags)
        );
    }
    idtr
}
