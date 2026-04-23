//! 进程/内存自省最小实现。对应 `hv/hv/introspection.*`。

use crate::arch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub cr3: u64,
}

pub fn current_process_snapshot() -> ProcessSnapshot {
    ProcessSnapshot {
        cr3: arch::read_cr3(),
    }
}

pub fn query_process_cr3(_pid: u64) -> Option<u64> {
    Some(current_process_snapshot().cr3)
}
