//! 主机 GDT 辅助。对应 `hv/hv/gdt.*`。

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default)]
pub struct DescriptorTablePointer {
    pub limit: u16,
    pub base: u64,
}

pub fn read_gdtr() -> DescriptorTablePointer {
    let mut gdtr = DescriptorTablePointer::default();
    // SAFETY: SGDT 仅读取当前 CPU GDTR。
    unsafe {
        core::arch::asm!(
            "sgdt [{ptr}]",
            ptr = in(reg) &mut gdtr,
            options(nostack, preserves_flags)
        );
    }
    gdtr
}
