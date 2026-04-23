//! VMCALL / IOCTL 共用的超级调用分发。对应 `hv/hv/hypercalls.cpp`、`hv/um/hv.h` 中的语义。

use shared_contract::{
    HypercallCode, HvHypercallIn, HvHypercallOut, HYPERCALL_KEY, HYPERVISOR_SIGNATURE,
    STATUS_INVALID_PARAMETER, STATUS_NOT_IMPLEMENTED, STATUS_SUCCESS,
};

use crate::ept::EptState;
use crate::introspection;
use crate::logger;
use crate::mm;
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
        HypercallCode::ReadPhysMem => handle_read_phys(inp),
        HypercallCode::WritePhysMem => handle_write_phys(inp),
        HypercallCode::ReadVirtMem => handle_read_virt(inp),
        HypercallCode::WriteVirtMem => handle_write_virt(inp),
        HypercallCode::QueryProcessCr3 => handle_query_process_cr3(inp),
        HypercallCode::InstallEptHook => handle_install_ept_hook(),
        HypercallCode::RemoveEptHook => handle_remove_ept_hook(),
        HypercallCode::GetHvBase => handle_get_hv_base(),
        HypercallCode::FlushLogs => handle_flush_logs(),
        HypercallCode::GetPhysicalAddress => handle_get_physical_address(inp),
        HypercallCode::HidePhysicalPage => handle_hide_physical_page(),
        HypercallCode::UnhidePhysicalPage => handle_unhide_physical_page(),
        HypercallCode::InstallMmr => handle_install_mmr(),
        HypercallCode::RemoveMmr => handle_remove_mmr(),
        HypercallCode::RemoveAllMmrs => handle_remove_all_mmrs(),
        HypercallCode::Test => HvHypercallOut {
            status: STATUS_SUCCESS,
            rax: 0,
            _reserved: 0,
        },
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
    let base = core::ptr::addr_of!(__ImageBase) as u64;
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

fn ok_with_rax(rax: u64) -> HvHypercallOut {
    HvHypercallOut {
        status: STATUS_SUCCESS,
        rax,
        _reserved: 0,
    }
}

fn invalid_parameter() -> HvHypercallOut {
    HvHypercallOut {
        status: STATUS_INVALID_PARAMETER,
        rax: 0,
        _reserved: 0,
    }
}

fn handle_read_phys(inp: &HvHypercallIn) -> HvHypercallOut {
    let pa = inp.args[0];
    let len = inp.args[1] as usize;
    let out_ptr = inp.args[2] as *mut u8;
    if len == 0 || out_ptr.is_null() {
        return invalid_parameter();
    }
    // SAFETY: 按调用约定由用户提供有效缓冲区。
    let ok = unsafe { mm::copy_from_physical(pa, out_ptr, len) };
    if ok {
        ok_with_rax(len as u64)
    } else {
        invalid_parameter()
    }
}

fn handle_write_phys(inp: &HvHypercallIn) -> HvHypercallOut {
    let pa = inp.args[0];
    let len = inp.args[1] as usize;
    let src_ptr = inp.args[2] as *const u8;
    if len == 0 || src_ptr.is_null() {
        return invalid_parameter();
    }
    // SAFETY: 按调用约定由用户提供有效缓冲区。
    let ok = unsafe { mm::copy_to_physical(pa, src_ptr, len) };
    if ok {
        ok_with_rax(len as u64)
    } else {
        invalid_parameter()
    }
}

fn handle_read_virt(inp: &HvHypercallIn) -> HvHypercallOut {
    let src = inp.args[0] as *const u8;
    let dst = inp.args[1] as *mut u8;
    let len = inp.args[2] as usize;
    if src.is_null() || dst.is_null() || len == 0 {
        return invalid_parameter();
    }
    // SAFETY: 仅做最小模板复制，调用方需保证地址有效。
    unsafe { core::ptr::copy_nonoverlapping(src, dst, len) };
    ok_with_rax(len as u64)
}

fn handle_write_virt(inp: &HvHypercallIn) -> HvHypercallOut {
    let dst = inp.args[0] as *mut u8;
    let src = inp.args[1] as *const u8;
    let len = inp.args[2] as usize;
    if src.is_null() || dst.is_null() || len == 0 {
        return invalid_parameter();
    }
    // SAFETY: 仅做最小模板复制，调用方需保证地址有效。
    unsafe { core::ptr::copy_nonoverlapping(src, dst, len) };
    ok_with_rax(len as u64)
}

fn handle_query_process_cr3(inp: &HvHypercallIn) -> HvHypercallOut {
    let pid = inp.args[0];
    match introspection::query_process_cr3(pid) {
        Some(cr3) => ok_with_rax(cr3),
        None => invalid_parameter(),
    }
}

fn handle_get_physical_address(inp: &HvHypercallIn) -> HvHypercallOut {
    let va = inp.args[0] as *const u8;
    if va.is_null() {
        return invalid_parameter();
    }
    // SAFETY: 查询当前地址空间下的线性地址对应物理地址。
    let pa = unsafe { mm::physical_address(va) };
    ok_with_rax(pa)
}

fn handle_install_ept_hook() -> HvHypercallOut {
    // SAFETY: 仅分配/释放最小 EPT 骨架用于路径连通性验证。
    let mut ept = match unsafe { EptState::build_identity_skeleton() } {
        Some(v) => v,
        None => return invalid_parameter(),
    };
    let eptp = ept.eptp.0;
    unsafe { ept.release() };
    ok_with_rax(eptp)
}

fn handle_remove_ept_hook() -> HvHypercallOut {
    ok_with_rax(1)
}

fn handle_hide_physical_page() -> HvHypercallOut {
    ok_with_rax(1)
}

fn handle_unhide_physical_page() -> HvHypercallOut {
    ok_with_rax(1)
}

fn handle_install_mmr() -> HvHypercallOut {
    ok_with_rax(1)
}

fn handle_remove_mmr() -> HvHypercallOut {
    ok_with_rax(1)
}

fn handle_remove_all_mmrs() -> HvHypercallOut {
    ok_with_rax(1)
}
