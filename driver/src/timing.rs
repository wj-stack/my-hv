//! TSC / 计时相关最小实现。对应 `hv/hv/timing.*`。
#![allow(dead_code)]

#[inline]
pub fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: RDTSC 仅读取时间戳计数器。
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags)
        );
    }
    ((hi as u64) << 32) | lo as u64
}

pub fn vm_exit_overhead_estimate(iterations: u32) -> u64 {
    let mut i = 0u32;
    let start = rdtsc();
    while i < iterations {
        core::hint::spin_loop();
        i += 1;
    }
    rdtsc().saturating_sub(start)
}
