//! Guest GPR 块（与 `vmexit_host_stub` 的 push 顺序一致）。供 `vmexit` 与 `exit_handlers` 共享，避免与 `vmexit` 模块形成循环依赖。

/// 供 `vmexit_host_stub` 保存的通用寄存器块（低地址在前：与 push 顺序一致）。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GuestRegs {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
}
