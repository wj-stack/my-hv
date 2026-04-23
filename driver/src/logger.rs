//! 诊断日志。对应 `hv/hv/logger.*`；当前使用 `wdk::println!`。

use wdk::println;
use crate::vmcs::{VmcsAccessError, VmcsField};

pub fn log(msg: &str) {
    println!("[my-hv-driver] {msg}");
}

pub fn log_vmcs_error(op: &str, field: VmcsField, err: VmcsAccessError) {
    println!(
        "[my-hv-driver] VMCS {op} failed: field=0x{:x}, err={:?}",
        field.raw(),
        err
    );
}

pub fn log_vmcs_guest_state(rip: u64, rsp: u64, rflags: u64) {
    println!(
        "[my-hv-driver] VMCS guest state seeded: rip=0x{:x}, rsp=0x{:x}, rflags=0x{:x}",
        rip, rsp, rflags
    );
}

pub fn log_vm_exit_reason(reason_basic: u16, raw: u32) {
    println!(
        "[my-hv-driver] VM-exit reason observed: basic={}, raw=0x{:x}",
        reason_basic, raw
    );
}

pub fn log_host_exception(vector: u64, rip: u64, error: u64) {
    println!(
        "[my-hv-driver] host exception: vector={}, rip=0x{:x}, error=0x{:x}",
        vector, rip, error
    );
}
