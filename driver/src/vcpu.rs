//! 每逻辑处理器的 VMXON/VMCS 生命周期。对应 `hv/hv/vcpu.cpp`、`hv/hv/vcpu.h` 中的 `virtualize_cpu` / `stop` 路径（此处不执行 `vmlaunch`）。

use core::mem::size_of;

use wdk_sys::ntddk::{
    ExAllocatePool2, ExFreePoolWithTag, KeQueryActiveProcessorCountEx, KeRevertToUserAffinityThreadEx,
    KeSetSystemAffinityThreadEx,
};
use wdk_sys::{NTSTATUS, POOL_FLAG_NON_PAGED, SIZE_T, ULONG};

use crate::arch::{self, VmxFixedMsrs};
use crate::mm::{self, free_contiguous_page};
use crate::vmcs;
use crate::vmx;

const POOL_TAG: ULONG = u32::from_ne_bytes(*b"HvVc");

/// 单 CPU 上的 VMX root 资源。
pub struct PerCpuState {
    pub vmxon_page: *mut u8,
    pub vmcs_page: *mut u8,
    pub vmxon_phys: u64,
    pub vmcs_phys: u64,
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
            vmxon_done: false,
        }
    }

    unsafe fn free_pages(&mut self) {
        unsafe {
            free_contiguous_page(self.vmxon_page);
            free_contiguous_page(self.vmcs_page);
        }
        self.vmxon_page = core::ptr::null_mut();
        self.vmcs_page = core::ptr::null_mut();
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
            return Err(wdk_sys::STATUS_NOT_SUPPORTED);
        }

        let count = unsafe { KeQueryActiveProcessorCountEx(0) };
        if count == 0 {
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
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };
        let Some(vmcs) = (unsafe { mm::alloc_contiguous_page() }) else {
            unsafe { free_contiguous_page(vmxon) };
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };

        unsafe {
            vmcs::prepare_vmxon_region(vmxon);
            vmcs::prepare_vmcs_region(vmcs);
        }

        let Some(fixed) = VmxFixedMsrs::read() else {
            unsafe {
                free_contiguous_page(vmxon);
                free_contiguous_page(vmcs);
            }
            return wdk_sys::STATUS_NOT_SUPPORTED;
        };

        if !unsafe { arch::enable_vmx_in_hardware(&fixed) } {
            unsafe {
                free_contiguous_page(vmxon);
                free_contiguous_page(vmcs);
            }
            return wdk_sys::STATUS_NOT_SUPPORTED;
        }

        let vmxon_phys = unsafe { mm::physical_address(vmxon) };
        let vmcs_phys = unsafe { mm::physical_address(vmcs) };

        cpu.vmxon_page = vmxon;
        cpu.vmcs_page = vmcs;
        cpu.vmxon_phys = vmxon_phys;
        cpu.vmcs_phys = vmcs_phys;

        if !unsafe { vmx::vmxon(&raw const cpu.vmxon_phys) } {
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if !unsafe { vmx::vmclear(&raw const cpu.vmcs_phys) } {
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if !unsafe { vmx::vmptrld(&raw const cpu.vmcs_phys) } {
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
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
