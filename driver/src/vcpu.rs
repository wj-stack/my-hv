//! 每逻辑处理器的 VMXON/VMCS 生命周期。对应 `hv/hv/vcpu.cpp`、`hv/hv/vcpu.h` 中的 `virtualize_cpu` / `stop` 路径。

use core::mem::size_of;

use wdk_sys::ntddk::{
    ExAllocatePool2, ExFreePoolWithTag, KeQueryActiveProcessorCountEx, KeRevertToUserAffinityThreadEx,
    KeSetSystemAffinityThreadEx,
};
use wdk_sys::{NTSTATUS, POOL_FLAG_NON_PAGED, SIZE_T, ULONG};

use crate::arch::{self, VmxFixedMsrs};
use crate::ept::EptState;
use crate::exit_handlers;
use crate::logger;
use crate::mm::{self, free_contiguous_page};
use crate::vmcs;
use crate::vmexit;
use crate::vmx;

const POOL_TAG: ULONG = u32::from_ne_bytes(*b"HvVc");

/// 最小 64 位 guest：`xor rax,rax` → `vmcall` → `jmp $`。
const GUEST_STUB_CODE: [u8; 8] = [
    0x48, 0x31, 0xc0, // xor rax, rax
    0x0f, 0x01, 0xc1, // vmcall
    0xeb, 0xfe,       // jmp short -2
];

/// 单 CPU 上的 VMX root 资源。
pub struct PerCpuState {
    pub vmxon_page: *mut u8,
    pub vmcs_page: *mut u8,
    pub vmxon_phys: u64,
    pub vmcs_phys: u64,
    pub msr_bitmap_page: *mut u8,
    pub msr_bitmap_phys: u64,
    pub host_stack_page: *mut u8,
    pub guest_stub_page: *mut u8,
    pub ept: Option<EptState>,
    /// 已成功执行 `VMXON`。
    pub vmxon_done: bool,
}

impl PerCpuState {
    const fn empty() -> Self {
        Self {
            vmxon_page: core::ptr::null_mut(),
            vmcs_page: core::ptr::null_mut(),
            vmxon_phys: 0,
            vmcs_phys: 0,
            msr_bitmap_page: core::ptr::null_mut(),
            msr_bitmap_phys: 0,
            host_stack_page: core::ptr::null_mut(),
            guest_stub_page: core::ptr::null_mut(),
            ept: None,
            vmxon_done: false,
        }
    }

    unsafe fn free_pages(&mut self) {
        unsafe {
            free_contiguous_page(self.vmxon_page);
            free_contiguous_page(self.vmcs_page);
            free_contiguous_page(self.msr_bitmap_page);
            free_contiguous_page(self.host_stack_page);
            free_contiguous_page(self.guest_stub_page);
        }
        if let Some(mut ept) = self.ept.take() {
            unsafe { ept.release() };
        }
        self.vmxon_page = core::ptr::null_mut();
        self.vmcs_page = core::ptr::null_mut();
        self.msr_bitmap_page = core::ptr::null_mut();
        self.host_stack_page = core::ptr::null_mut();
        self.guest_stub_page = core::ptr::null_mut();
        self.msr_bitmap_phys = 0;
        self.vmxon_done = false;
    }
}

/// 全系统 VMX root 会话（组 0 内全部活动逻辑处理器）。
pub struct VmxCluster {
    cpus: *mut PerCpuState,
    count: u32,
    /// 所有 CPU 均完成 `VMXON`。
    active: bool,
}

impl VmxCluster {
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// 在组 0 的每个逻辑处理器上执行 `VMXON` + `VMCLEAR`/`VMPTRLD`。
    ///
    /// # Safety
    /// 调用方必须处于 `IRQL <= APC_LEVEL`（与 `KeSetSystemAffinityThreadEx` 要求一致）。
    pub unsafe fn start() -> Result<Self, NTSTATUS> {
        if !vmx::host_supports_vmx_root() {
            logger::log("VMX root not supported by platform");
            return Err(wdk_sys::STATUS_NOT_SUPPORTED);
        }

        let count = unsafe { KeQueryActiveProcessorCountEx(0) };
        if count == 0 {
            logger::log("no active processors in group 0");
            return Err(wdk_sys::STATUS_UNSUCCESSFUL);
        }

        let bytes = (count as usize).saturating_mul(size_of::<PerCpuState>());
        let cpus = unsafe {
            ExAllocatePool2(
                POOL_FLAG_NON_PAGED,
                bytes as SIZE_T,
                POOL_TAG,
            )
        } as *mut PerCpuState;
        if cpus.is_null() {
            logger::log("failed to allocate per-cpu VMX state array");
            return Err(wdk_sys::STATUS_INSUFFICIENT_RESOURCES);
        }
        unsafe {
            core::ptr::write_bytes(cpus, 0, count as usize);
        }

        let mut cluster = VmxCluster {
            cpus,
            count,
            active: false,
        };

        for i in 0..count {
            let affinity: u64 = 1u64 << i;
            let prev = unsafe { KeSetSystemAffinityThreadEx(affinity) };
            let st = unsafe { cluster.init_cpu(i) };
            unsafe { KeRevertToUserAffinityThreadEx(prev) };
            if !wdk::nt_success(st) {
                logger::log("VMX init failed on a CPU, starting rollback");
                unsafe { cluster.rollback_partial(i) };
                unsafe { ExFreePoolWithTag(cpus.cast(), POOL_TAG) };
                return Err(st);
            }
        }

        cluster.active = true;
        Ok(cluster)
    }

    unsafe fn init_cpu(&mut self, index: u32) -> NTSTATUS {
        let cpu = unsafe { &mut *self.cpus.add(index as usize) };
        *cpu = PerCpuState::empty();

        let Some(vmxon) = (unsafe { mm::alloc_contiguous_page() }) else {
            logger::log("failed to allocate VMXON page");
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };
        let Some(vmcs) = (unsafe { mm::alloc_contiguous_page() }) else {
            logger::log("failed to allocate VMCS page");
            unsafe { free_contiguous_page(vmxon) };
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };
        let Some(msr_bitmap) = (unsafe { mm::alloc_contiguous_page() }) else {
            logger::log("failed to allocate MSR bitmap page");
            unsafe {
                free_contiguous_page(vmxon);
                free_contiguous_page(vmcs);
            }
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };
        let Some(host_stack) = (unsafe { mm::alloc_contiguous_page() }) else {
            logger::log("failed to allocate host VM-exit stack page");
            unsafe {
                free_contiguous_page(vmxon);
                free_contiguous_page(vmcs);
                free_contiguous_page(msr_bitmap);
            }
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };
        let Some(guest_stub) = (unsafe { mm::alloc_contiguous_page() }) else {
            logger::log("failed to allocate guest stub page");
            unsafe {
                free_contiguous_page(vmxon);
                free_contiguous_page(vmcs);
                free_contiguous_page(msr_bitmap);
                free_contiguous_page(host_stack);
            }
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };

        unsafe {
            vmcs::prepare_vmxon_region(vmxon);
            vmcs::prepare_vmcs_region(vmcs);
            core::ptr::write_bytes(msr_bitmap, 0xFF, 4096);
            core::ptr::write_bytes(host_stack, 0xCC, 4096);
            core::ptr::copy_nonoverlapping(GUEST_STUB_CODE.as_ptr(), guest_stub, GUEST_STUB_CODE.len());
        }

        let Some(fixed) = VmxFixedMsrs::read() else {
            logger::log("failed to read VMX fixed MSRs");
            unsafe {
                free_contiguous_page(vmxon);
                free_contiguous_page(vmcs);
                free_contiguous_page(msr_bitmap);
                free_contiguous_page(host_stack);
                free_contiguous_page(guest_stub);
            }
            return wdk_sys::STATUS_NOT_SUPPORTED;
        };

        if !unsafe { arch::enable_vmx_in_hardware(&fixed) } {
            logger::log("enable_vmx_in_hardware returned false");
            unsafe {
                free_contiguous_page(vmxon);
                free_contiguous_page(vmcs);
                free_contiguous_page(msr_bitmap);
                free_contiguous_page(host_stack);
                free_contiguous_page(guest_stub);
            }
            return wdk_sys::STATUS_NOT_SUPPORTED;
        }

        let vmxon_phys = unsafe { mm::physical_address(vmxon) };
        let vmcs_phys = unsafe { mm::physical_address(vmcs) };
        let msr_bitmap_phys = unsafe { mm::physical_address(msr_bitmap) };
        let ept = unsafe { EptState::build_identity_skeleton() };

        cpu.vmxon_page = vmxon;
        cpu.vmcs_page = vmcs;
        cpu.vmxon_phys = vmxon_phys;
        cpu.vmcs_phys = vmcs_phys;
        cpu.msr_bitmap_page = msr_bitmap;
        cpu.msr_bitmap_phys = msr_bitmap_phys;
        cpu.host_stack_page = host_stack;
        cpu.guest_stub_page = guest_stub;
        cpu.ept = ept;

        if !unsafe { vmx::vmxon(&raw const cpu.vmxon_phys) } {
            logger::log("VMXON failed");
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if !unsafe { vmx::vmclear(&raw const cpu.vmcs_phys) } {
            logger::log("VMCLEAR failed");
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if !unsafe { vmx::vmptrld(&raw const cpu.vmcs_phys) } {
            logger::log("VMPTRLD failed");
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if let Err(err) = unsafe {
            vmcs::configure_control_fields(
                Some(cpu.msr_bitmap_phys),
                cpu.ept.as_ref().map(|v| v.eptp.0),
            )
        } {
            logger::log_vmcs_error("configure_control_fields", vmcs::VmcsField::CTRL_CPU_BASED, err);
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if let Some(ref ept) = cpu.ept {
            if !unsafe { ept.invept_single_context() } {
                logger::log("INVEPT(single-context) failed");
                let _ = unsafe { vmx::vmxoff() };
                unsafe {
                    arch::disable_vmx_hardware();
                    cpu.free_pages();
                }
                return wdk_sys::STATUS_UNSUCCESSFUL;
            }
        }

        let host_rsp = host_stack as u64 + 4096 - 0x20;
        let host_entry = vmexit::vmexit_host_stub as *const () as usize as u64;
        if let Err(err) = unsafe { vmcs::configure_host_state(host_entry, host_rsp) } {
            logger::log_vmcs_error("configure_host_state", vmcs::VmcsField::HOST_RIP, err);
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if let Err(err) = unsafe { vmcs::configure_guest_state() } {
            logger::log_vmcs_error("configure_guest_state", vmcs::VmcsField::GUEST_CR3, err);
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if let Err(err) = unsafe { vmcs::configure_guest_segment_state() } {
            logger::log_vmcs_error(
                "configure_guest_segment_state",
                vmcs::VmcsField::GUEST_CS_SELECTOR,
                err,
            );
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        let guest_rip = guest_stub as u64;
        let guest_rsp = guest_stub as u64 + 0xFF0u64;
        if let Err(err) = unsafe { vmcs::seed_guest_entry_state(guest_rip, guest_rsp, 0x2) } {
            logger::log_vmcs_error("seed_guest_entry_state", vmcs::VmcsField::GUEST_RFLAGS, err);
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        let guest_rflags = match unsafe { vmcs::vmread(vmcs::VmcsField::GUEST_RFLAGS) } {
            Ok(v) => v,
            Err(err) => {
                logger::log_vmcs_error("vmread", vmcs::VmcsField::GUEST_RFLAGS, err);
                let _ = unsafe { vmx::vmxoff() };
                unsafe {
                    arch::disable_vmx_hardware();
                    cpu.free_pages();
                }
                return wdk_sys::STATUS_UNSUCCESSFUL;
            }
        };
        if guest_rflags != 0x2 {
            logger::log("VMCS self-check mismatch on GUEST_RFLAGS");
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        match unsafe { vmcs::read_guest_state_snapshot() } {
            Ok(snapshot) => {
                logger::log_vmcs_guest_state(snapshot.rip, snapshot.rsp, snapshot.rflags);
            }
            Err(err) => {
                logger::log_vmcs_error("vmread(guest_state_snapshot)", vmcs::VmcsField::GUEST_RIP, err);
            }
        }

        match unsafe { vmcs::read_exit_reason() } {
            Ok(reason) => {
                logger::log_vm_exit_reason(reason.basic, reason.raw);
                exit_handlers::log_basic_exit(reason.basic as u32);
                let ctx = exit_handlers::VmExitContext {
                    reason,
                    guest_rip: 0,
                    guest_rsp: 0,
                    guest_rflags,
                };
                if let Some(opt) = peek_session_for_dispatch() {
                    let _ = exit_handlers::dispatch_vm_exit(opt, &ctx, 0, [0; 6]);
                }
            }
            Err(err) => {
                logger::log_vmcs_error("vmread(EXIT_REASON)", vmcs::VmcsField::EXIT_REASON, err);
            }
        }

        cpu.vmxon_done = true;
        wdk_sys::STATUS_SUCCESS
    }

    unsafe fn rollback_partial(&mut self, failed_index: u32) {
        for j in 0..failed_index {
            let prev = unsafe { KeSetSystemAffinityThreadEx(1u64 << j) };
            unsafe {
                self.teardown_cpu(j);
                KeRevertToUserAffinityThreadEx(prev);
            }
        }
    }

    /// `VMCLEAR` + `VMXOFF` 并释放页面；可在部分失败路径或 `STOP` 中调用。
    ///
    /// # Safety
    /// `IRQL <= APC_LEVEL`。
    pub unsafe fn stop(&mut self) {
        if self.cpus.is_null() || self.count == 0 {
            return;
        }

        for i in 0..self.count {
            let prev = unsafe { KeSetSystemAffinityThreadEx(1u64 << i) };
            unsafe {
                self.teardown_cpu(i);
                KeRevertToUserAffinityThreadEx(prev);
            }
        }

        unsafe {
            ExFreePoolWithTag(self.cpus.cast(), POOL_TAG);
        }
        self.cpus = core::ptr::null_mut();
        self.count = 0;
        self.active = false;
    }

    unsafe fn teardown_cpu(&mut self, index: u32) {
        let cpu = unsafe { &mut *self.cpus.add(index as usize) };
        if !cpu.vmxon_page.is_null() && cpu.vmxon_done {
            let _ = unsafe { vmx::vmclear(&raw const cpu.vmcs_phys) };
            let _ = unsafe { vmx::vmxoff() };
            unsafe { arch::disable_vmx_hardware() };
        }
        unsafe { cpu.free_pages() };
        *cpu = PerCpuState::empty();
    }
}

/// 供 bring-up 诊断路径在无 `VMEXIT_SESSION` 时读取 `Option<VmxCluster>`（与 `install_vmexit_session` 指向同一块内存）。
pub fn peek_session_for_dispatch() -> Option<&'static mut Option<VmxCluster>> {
    vmexit::peek_session_option_mut()
}

/// 在当前逻辑处理器上尝试 `VMLAUNCH`（成功则进入 guest，通常**不会**返回到此函数）。
///
/// 需已加载本驱动并完成 `IOCTL_HV_START`，且当前线程亲和到目标 CPU。仅 `--features experimental_vmentry` 可用。
#[cfg(feature = "experimental_vmentry")]
#[allow(dead_code)]
pub unsafe fn debug_vmlaunch_on_this_cpu() -> bool {
    unsafe { vmx::vmlaunch() }
}
