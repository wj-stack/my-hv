//! VMCALL / IOCTL 共用的超级调用分发。对应 `hv/hv/hypercalls.cpp`、`hv/um/hv.h` 中的语义。

use shared_contract::{
    HypercallCode, HvHypercallIn, HvHypercallOut, HYPERCALL_KEY, HYPERVISOR_SIGNATURE,
    STATUS_INVALID_PARAMETER, STATUS_NOT_IMPLEMENTED, STATUS_SUCCESS,
};

use crate::logger;
use crate::vcpu::VmxCluster;

unsafe extern "C" {
    static __ImageBase: u8;
}

fn pack_key_code_ok(rax: u64) -> bool {
    (rax >> 8) == HYPERCALL_KEY
}

/// 对带长度/地址参数的超级调用做最小校验（US3）。
fn validate_args(inp: &HvHypercallIn) -> Option<HvHypercallOut> {
    let code = inp.rax as u8;
    let Ok(c) = HypercallCode::try_from(code) else {
        return None;
    };
    match c {
        HypercallCode::ReadPhysMem | HypercallCode::WritePhysMem => {
            if inp.args[1] == 0 || inp.args[1] > 0x1000 {
                return Some(HvHypercallOut {
                    status: STATUS_INVALID_PARAMETER,
                    rax: 0,
                    _reserved: 0,
                });
            }
            None
        }
        HypercallCode::ReadVirtMem | HypercallCode::WriteVirtMem => {
            if inp.args[0] == 0 || inp.args[2] == 0 || inp.args[2] > 0x1000 {
                return Some(HvHypercallOut {
                    status: STATUS_INVALID_PARAMETER,
                    rax: 0,
                    _reserved: 0,
                });
            }
            None
        }
        _ => None,
    }
}

fn bad_key_out() -> HvHypercallOut {
    HvHypercallOut {
        status: STATUS_INVALID_PARAMETER,
        rax: 0,
        _reserved: 0,
    }
}

/// IOCTL 与将来 VM-exit 路径共用的入口。
pub fn dispatch(cluster: &mut Option<VmxCluster>, inp: &HvHypercallIn) -> HvHypercallOut {
    if !pack_key_code_ok(inp.rax) {
        logger::log("hypercall: invalid key");
        return bad_key_out();
    }

    if let Some(e) = validate_args(inp) {
        return e;
    }

    let code = match HypercallCode::try_from(inp.rax as u8) {
        Ok(c) => c,
        Err(()) => {
            return HvHypercallOut {
                status: STATUS_INVALID_PARAMETER,
                rax: 0,
                _reserved: 0,
            };
        }
    };

    match code {
        HypercallCode::Ping => handle_ping(cluster.as_ref()),
        HypercallCode::Unload => handle_unload(cluster),
        HypercallCode::QueryProcessCr3 => not_impl("QUERY_PROCESS_CR3"),
        HypercallCode::GetHvBase => handle_get_hv_base(),
        HypercallCode::FlushLogs => handle_flush_logs(),
        HypercallCode::Test => HvHypercallOut {
            status: STATUS_SUCCESS,
            rax: 0,
            _reserved: 0,
        },
        _ => not_impl("unimplemented hypercall"),
    }
}

fn not_impl(_what: &'static str) -> HvHypercallOut {
    HvHypercallOut {
        status: STATUS_NOT_IMPLEMENTED,
        rax: 0,
        _reserved: 0,
    }
}

fn handle_ping(cluster: Option<&VmxCluster>) -> HvHypercallOut {
    let Some(c) = cluster else {
        logger::log("hypercall PING: VMX session not started");
        return HvHypercallOut {
            status: STATUS_NOT_IMPLEMENTED,
            rax: 0,
            _reserved: 0,
        };
    };
    if !c.is_active() {
        return HvHypercallOut {
            status: STATUS_NOT_IMPLEMENTED,
            rax: 0,
            _reserved: 0,
        };
    }
    HvHypercallOut {
        status: STATUS_SUCCESS,
        rax: HYPERVISOR_SIGNATURE,
        _reserved: 0,
    }
}

fn handle_unload(cluster: &mut Option<VmxCluster>) -> HvHypercallOut {
    if let Some(mut inner) = cluster.take() {
        unsafe {
            inner.stop();
        }
    }
    HvHypercallOut {
        status: STATUS_SUCCESS,
        rax: 1,
        _reserved: 0,
    }
}

fn handle_get_hv_base() -> HvHypercallOut {
    let base = unsafe { core::ptr::addr_of!(__ImageBase) as u64 };
    HvHypercallOut {
        status: STATUS_SUCCESS,
        rax: base,
        _reserved: 0,
    }
}

fn handle_flush_logs() -> HvHypercallOut {
    logger::log("FLUSH_LOGS (no buffered messages)");
    HvHypercallOut {
        status: STATUS_SUCCESS,
        rax: 0,
        _reserved: 0,
    }
}
