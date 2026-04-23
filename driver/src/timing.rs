//! TSC / 计时相关最小实现。对应 `hv/hv/timing.*`。
#![allow(dead_code)]

use crate::vcpu::VmxCluster;
use crate::vmcs::{self, VmcsField};

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

/// 每次 VM-exit 入口（可扩展为 TSC 差分测量，参考 `hide_vm_exit_overhead`）。
pub fn on_vm_exit(_cluster: &mut VmxCluster) {}

/// 抢占定时器到期（软禁：置 VMCS 为最大值，参考 C++ 路径）。
pub fn on_preemption_timer(_cluster: &mut VmxCluster) {
    let _ = unsafe { crate::vmcs::vmwrite(crate::vmcs::VmcsField::GUEST_VMX_PREEMPTION_TIMER, !0u64) };
}

/// 写回 TSC offset 与预抢占（如有缓存）。
pub fn post_dispatch_vmexit(cluster: &mut VmxCluster, _basic: u32) {
    if let Some(cpu) = cluster.current_cpu_mut() {
        if cpu.cache.tsc_offset != 0 {
            let _ = unsafe { vmcs::vmwrite(VmcsField::CTRL_TSC_OFFSET, cpu.cache.tsc_offset) };
        }
    }
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
