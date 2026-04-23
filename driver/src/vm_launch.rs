//! `VMLAUNCH` 进入序。与 `hv/hv/vm-launch.asm` 一致：无函数序言，在入口时把 `GUEST_RSP`/`GUEST_RIP` 设成与 `call hv_vm_launch` 栈帧及 `2f` 成功桩匹配。
//!
//! 成功进入 guest 后由桩 `2:` 上 `ret` 返回到调用方；失败时在 root 中 `xor al,al; ret`。

use core::arch::global_asm;

global_asm!(
    ".text",
    ".globl hv_vm_launch",
    "hv_vm_launch:",
    "mov rax, 0x681C",
    "vmwrite rax, rsp",
    "mov rax, 0x681E",
    "lea rdx, [rip + 2f]",
    "vmwrite rax, rdx",
    "vmlaunch",
    "xor al, al",
    "ret",
    "2:",
    "mov al, 1",
    "ret",
);

unsafe extern "C" {
    /// 与 C++ `?vm_launch@hv@@YA_NXZ` 同形：在 root 中失败时返回 `0`；若进入 guest 则由标签 `2` 上 `ret` 回到调用方且 `al != 0`。
    fn hv_vm_launch() -> u8;
}

/// # Safety
/// 已 `VMPTRLD` 当前 VMCS，且本线程处于 VMX root，执行序与 C++ `virtualize_cpu` 末尾一致。
#[inline]
pub unsafe fn vmlaunch_enter_guest() -> bool {
    unsafe { hv_vm_launch() != 0 }
}
