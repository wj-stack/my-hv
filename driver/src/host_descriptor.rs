//! Host GDT / IDT 初始化。对应 `hv/hv/gdt.*`、`hv/hv/idt.*`。

unsafe extern "C" {
    fn host_isr_0();
    fn host_isr_1();
    fn host_isr_2();
    fn host_isr_3();
    fn host_isr_4();
    fn host_isr_5();
    fn host_isr_6();
    fn host_isr_7();
    fn host_isr_8();
    fn host_isr_10();
    fn host_isr_11();
    fn host_isr_12();
    fn host_isr_13();
    fn host_isr_14();
    fn host_isr_16();
    fn host_isr_17();
    fn host_isr_18();
    fn host_isr_19();
    fn host_isr_20();
    fn host_isr_30();
}

/// `host_cs_selector`：index 1 → `0x08`。
pub const HOST_CS_SELECTOR: u16 = 0x08;
/// `host_tr_selector`：index 2 → `0x10`（64 位 TSS 占 16 字节）。
pub const HOST_TR_SELECTOR: u16 = 0x10;
/// GDT 项数（含 null + CS + TSS 双字）。
pub const HOST_GDT_LIMIT: u16 = (4 * 8 - 1) as u16;
/// IDT 256 项 × 16 字节。
pub const HOST_IDT_LIMIT: u16 = (256 * 16 - 1) as u16;

const IDT_TYPE_INTERRUPT_GATE: u8 = 0xE;
const IDT_PRESENT_RING0: u8 = 0x80;

#[repr(C, packed)]
struct IdtGate64 {
    offset_lo: u16,
    selector: u16,
    ist: u8,
    attr: u8,
    offset_mid: u16,
    offset_hi: u32,
    zero: u32,
}

#[inline]
unsafe fn write_idt_gate(table: *mut u8, vector: usize, handler: u64) {
    let g = IdtGate64 {
        offset_lo: handler as u16,
        selector: HOST_CS_SELECTOR,
        ist: 0,
        attr: IDT_PRESENT_RING0 | IDT_TYPE_INTERRUPT_GATE,
        offset_mid: (handler >> 16) as u16,
        offset_hi: (handler >> 32) as u32,
        zero: 0,
    };
    unsafe {
        core::ptr::write_unaligned(table.add(vector * 16).cast(), g);
    }
}

/// `prepare_host_idt`：仅安装与 `hv/hv/idt.cpp` 相同向量。
///
/// # Safety
/// `idt` 指向至少 4096 字节可写区。
pub unsafe fn prepare_host_idt(idt: *mut u8) {
    unsafe {
        core::ptr::write_bytes(idt, 0, 4096);
        write_idt_gate(idt, 0, host_isr_0 as usize as u64);
        write_idt_gate(idt, 1, host_isr_1 as usize as u64);
        write_idt_gate(idt, 2, host_isr_2 as usize as u64);
        write_idt_gate(idt, 3, host_isr_3 as usize as u64);
        write_idt_gate(idt, 4, host_isr_4 as usize as u64);
        write_idt_gate(idt, 5, host_isr_5 as usize as u64);
        write_idt_gate(idt, 6, host_isr_6 as usize as u64);
        write_idt_gate(idt, 7, host_isr_7 as usize as u64);
        write_idt_gate(idt, 8, host_isr_8 as usize as u64);
        write_idt_gate(idt, 10, host_isr_10 as usize as u64);
        write_idt_gate(idt, 11, host_isr_11 as usize as u64);
        write_idt_gate(idt, 12, host_isr_12 as usize as u64);
        write_idt_gate(idt, 13, host_isr_13 as usize as u64);
        write_idt_gate(idt, 14, host_isr_14 as usize as u64);
        write_idt_gate(idt, 16, host_isr_16 as usize as u64);
        write_idt_gate(idt, 17, host_isr_17 as usize as u64);
        write_idt_gate(idt, 18, host_isr_18 as usize as u64);
        write_idt_gate(idt, 19, host_isr_19 as usize as u64);
        write_idt_gate(idt, 20, host_isr_20 as usize as u64);
        write_idt_gate(idt, 30, host_isr_30 as usize as u64);
    }
}

/// 64 位 TSS：与 `hv` 中 `memset(&tss,0)` 用法一致。
#[repr(C, align(64))]
pub struct HostTaskState {
    pub raw: [u8; 104],
}

impl HostTaskState {
    pub const fn new_zeroed() -> Self {
        Self { raw: [0; 104] }
    }
}

/// `prepare_host_gdt`：CS @ index1，64 位 TSS（busy）@ index2。
///
/// # Safety
/// `gdt` 至少 32 字节可写。
pub unsafe fn prepare_host_gdt(gdt: *mut u8, tss: *const HostTaskState) {
    unsafe {
        core::ptr::write_bytes(gdt, 0, 32);
    }
    let base = tss as usize as u64;
    let limit = (core::mem::size_of::<HostTaskState>() - 1) as u32;

    let mut cs = [0u8; 8];
    cs[5] = 0x9A;
    cs[6] = 0x20;
    let cs_q = u64::from_le_bytes(cs);
    unsafe {
        gdt.add(8).cast::<u64>().write_unaligned(cs_q);
    }

    let mut lo = [0u8; 8];
    lo[0..2].copy_from_slice(&(limit as u16).to_le_bytes());
    lo[2..4].copy_from_slice(&((base & 0xFFFF) as u16).to_le_bytes());
    lo[4] = ((base >> 16) & 0xFF) as u8;
    lo[5] = 0x8B;
    lo[6] = ((limit >> 16) & 0x0F) as u8;
    lo[7] = ((base >> 24) & 0xFF) as u8;
    let tss_lo = u64::from_le_bytes(lo);
    let tss_hi = base >> 32;
    unsafe {
        gdt.add(16).cast::<u64>().write_unaligned(tss_lo);
        gdt.add(24).cast::<u64>().write_unaligned(tss_hi);
    }
}
