//! 每逻辑处理器的 VMXON/VMCS 生命周期。对应 `hv/hv/vcpu.cpp`、`hv/hv/vcpu.h` 中的 `virtualize_cpu` / `stop` 路径。

use core::mem::size_of;

use wdk_sys::ntddk::{
    ExAllocatePool2, ExFreePoolWithTag, KeGetCurrentProcessorNumberEx, KeQueryActiveProcessorCountEx,
    KeRevertToUserAffinityThreadEx, KeSetSystemAffinityThreadEx,
};
use wdk_sys::PROCESSOR_NUMBER;
use wdk_sys::{NTSTATUS, POOL_FLAG_NON_PAGED, SIZE_T, ULONG};

use crate::arch::{self, VmxFixedMsrs};
use crate::ept::EptState;
use crate::ia32;
use crate::logger;
use crate::msr_bitmap;
use crate::introspection;
use crate::mm::{self, free_contiguous_page};
use crate::vmcs;
use crate::vmexit;
use crate::vm_launch;
use crate::vmx;
use shared_contract::{HypercallCode, HYPERCALL_KEY, HYPERVISOR_SIGNATURE};

const POOL_TAG: ULONG = u32::from_ne_bytes(*b"HvVc");


fn fill_vcpu_cache(cpu: &mut PerCpuState) {
    let feature_control = unsafe { arch::rdmsr(ia32::IA32_FEATURE_CONTROL) };
    let mut g = feature_control;
    g |= 1;
    g &= !(1u64 << 1);
    g &= !(1u64 << 2);
    let vmx_misc = unsafe { arch::rdmsr(ia32::IA32_VMX_MISC) };
    let d = arch::cpuid(0x0D, 0x00);
    let xmask = !(((d.edx as u64) << 32) | (d.eax as u64));
    cpu.cache.feature_control = feature_control;
    cpu.cache.guest_feature_control = g;
    cpu.cache.xcr0_unsupported_mask = xmask;
    cpu.cache.vmx_misc = vmx_misc;
}

/// 最小 64 位 guest：`xor rax,rax` → `vmcall` → `jmp $`。
/// 与 `hv/hv/vcpu.cpp` 中 `cache_cpu_data` / `vcpu_cached_data` 对齐的可见字段子集。
#[derive(Clone, Copy, Debug)]
pub struct VcpuCache {
    pub feature_control: u64,
    pub guest_feature_control: u64,
    pub xcr0_unsupported_mask: u64,
    pub vmx_misc: u64,
    pub tsc_offset: u64,
    pub preemption_timer: u64,
    pub hide_vm_exit_overhead: bool,
    pub vm_exit_tsc_overhead: u64,
}

impl VcpuCache {
    pub const fn empty() -> Self {
        Self {
            feature_control: 0,
            guest_feature_control: 0,
            xcr0_unsupported_mask: !0u64,
            vmx_misc: 0,
            tsc_offset: 0,
            preemption_timer: !0u64,
            hide_vm_exit_overhead: false,
            vm_exit_tsc_overhead: 0,
        }
    }
}

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
    /// MSR 自动存/装列表（`hv` 的 `msr_exit_store` + `msr_entry_load`），单页。
    pub msr_auto_page: *mut u8,
    pub ept: Option<EptState>,
    /// 与参考 `cache_cpu_data` 对齐的只读/伪造 MSR 缓存。
    pub cache: VcpuCache,
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
            msr_auto_page: core::ptr::null_mut(),
            ept: None,
            cache: VcpuCache::empty(),
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
            free_contiguous_page(self.msr_auto_page);
        }
        if let Some(mut ept) = self.ept.take() {
            unsafe { ept.release() };
        }
        self.vmxon_page = core::ptr::null_mut();
        self.vmcs_page = core::ptr::null_mut();
        self.msr_bitmap_page = core::ptr::null_mut();
        self.host_stack_page = core::ptr::null_mut();
        self.guest_stub_page = core::ptr::null_mut();
        self.msr_auto_page = core::ptr::null_mut();
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

    /// 当前已亲和到的逻辑处理器的 `PerCpuState`（VM-exit 处理路径用于读 `VcpuCache`、EPT）。
    pub fn current_cpu_mut(&mut self) -> Option<&mut PerCpuState> {
        let mut pn: PROCESSOR_NUMBER = unsafe { core::mem::zeroed() };
        let i = unsafe { KeGetCurrentProcessorNumberEx(&mut pn) } as u32;
        if i < self.count {
            Some(unsafe { &mut *self.cpus.add(i as usize) })
        } else {
            None
        }
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
        let Some(msr_auto) = (unsafe { mm::alloc_contiguous_page() }) else {
            logger::log("failed to allocate MSR auto-load/store page");
            unsafe {
                free_contiguous_page(vmxon);
                free_contiguous_page(vmcs);
                free_contiguous_page(msr_bitmap);
                free_contiguous_page(host_stack);
                free_contiguous_page(guest_stub);
            }
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };

        // 在 `VMXON` / `VMCLEAR` 之前初始化各 4KiB 控制结构。对照 `hv/hv/vcpu.cpp` 的 `virtualize_cpu`：
        // - `prepare_vmxon_region` ≈ `enter_vmx_operation()` 里对 `vmxon_region.revision_id` / `must_be_zero` 的填写；
        // - `prepare_vmcs_region` ≈ `load_vmcs_pointer()` 里对 `vmcs_region.revision_id` / `shadow_vmcs_indicator` 的填写；
        // - `prepare_msr_bitmap_page` ≈ `prepare_external_structures()` 里 `memset(msr_bitmap)` + FEATURE_CONTROL 读退出 + `enable_mtrr_exiting`。
        unsafe {
            // 写入 IA32_VMX_BASIC 中的 VMCS revision id，满足 Intel SDM 对 VMXON 区域的要求。
            vmcs::prepare_vmxon_region(vmxon);
            // 同上，供后续 VMCLEAR/VMPTRLD 使用的 VMCS 区域头。
            vmcs::prepare_vmcs_region(vmcs);
            // 整页 MSR 位图：默认不拦截；对 FEATURE_CONTROL 读与 MTRR 相关写置位以在 guest 访问时 VM-exit。
            msr_bitmap::prepare_msr_bitmap_page(msr_bitmap);
            // Host VM-exit 处理栈。参考工程在 `virtualize_cpu` 开头 `memset(cpu,0,...)` 把 `host_stack` 清零；
            // 这里用 0xCC（INT3）填满一页，便于栈下溢或未初始化使用时立刻断在调试器里。
            core::ptr::write_bytes(host_stack, 0xCC, 4096);
            // 参考 HV 的 guest 初始 RIP 来自 `write_vmcs_guest_fields()`（当前内核上下文）；本驱动改为映射到
            // 独立物理页上的 `GUEST_STUB_CODE`（xor rax,rax; vmcall; 死循环），作为最小 guest 入口。
            core::ptr::copy_nonoverlapping(GUEST_STUB_CODE.as_ptr(), guest_stub, GUEST_STUB_CODE.len());
            vmcs::prepare_msr_auto_lists(msr_auto);
        }

        let Some(fixed) = VmxFixedMsrs::read() else {
            logger::log("failed to read VMX fixed MSRs");
            unsafe {
                free_contiguous_page(vmxon);
                free_contiguous_page(vmcs);
                free_contiguous_page(msr_bitmap);
                free_contiguous_page(host_stack);
                free_contiguous_page(guest_stub);
                free_contiguous_page(msr_auto);
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
                free_contiguous_page(msr_auto);
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
        cpu.msr_auto_page = msr_auto;
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

        let msr_auto_phys = unsafe { mm::physical_address(msr_auto) };
        let system_cr3 = introspection::query_process_cr3(4).unwrap_or_else(arch::read_cr3);
        let ctrl_params = vmcs::VmcsControlParams {
            msr_bitmap_pa: Some(cpu.msr_bitmap_phys),
            eptp: cpu.ept.as_ref().map(|v| v.eptp.0),
            vpid: (index + 1) as u16,
            cr3_target: Some(system_cr3),
            msr_exit_store_pa: msr_auto_phys,
            msr_entry_load_pa: msr_auto_phys + 64,
        };
        if let Err(err) = unsafe { vmcs::configure_control_fields(&ctrl_params) } {
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

        let host_rsp_top = host_stack as u64 + 4096;
        let host_rsp = (host_rsp_top & !0xFu64).saturating_sub(8);
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

        fill_vcpu_cache(cpu);

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

        if !unsafe { vm_launch::vmlaunch_enter_guest() } {
            let inst_err = unsafe { vmcs::vmread(vmcs::VmcsField::VM_INSTRUCTION_ERROR) };
            match inst_err {
                Ok(c) => {
                    wdk::println!(
                        "[my-hv-driver] VMLAUNCH failed, VM_INSTRUCTION_ERROR=0x{:x}",
                        c
                    );
                }
                Err(e) => {
                    logger::log_vmcs_error("vmread(VM_INSTRUCTION_ERROR)", vmcs::VmcsField::VM_INSTRUCTION_ERROR, e);
                }
            }
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                cpu.free_pages();
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        let rax_ping = (HYPERCALL_KEY << 8) | (HypercallCode::Ping as u64);
        let (ok, rax_out) = unsafe { vmx::vmcall(rax_ping, 0, 0) };
        if ok && rax_out == HYPERVISOR_SIGNATURE {
            wdk::println!("[my-hv-driver] post-VMLAUNCH PING returned hypervisor signature");
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

