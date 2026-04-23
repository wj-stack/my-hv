//! 每逻辑处理器的 VMXON/VMCS 生命周期。对应 `hv/hv/vcpu.cpp`、`hv/hv/vcpu.h` 中的 `virtualize_cpu` / `stop` 路径。

use alloc::format;

use core::mem::size_of;

use wdk_sys::ntddk::{
    ExAllocatePool2, ExFreePoolWithTag, KeGetCurrentProcessorNumberEx, KeRevertToUserAffinityThreadEx,
    KeSetSystemAffinityThreadEx,
};
use wdk_sys::PROCESSOR_NUMBER;
use wdk_sys::{NTSTATUS, POOL_FLAG_NON_PAGED, SIZE_T, ULONG};

unsafe extern "system" {
    /// 与 `hv/hv/hv.cpp` 中 `KeQueryActiveProcessorCount(nullptr)` 一致。
    fn KeQueryActiveProcessorCount(active_processors: *mut core::ffi::c_void) -> u16;
}

use crate::arch::{self, VmxFixedMsrs};
use crate::ept::EptState;
use crate::host_descriptor::{self, HostTaskState};
use crate::host_page_tables::{HostPageTables, HostPageTablesBox};
use crate::ia32;
use crate::logger;
use crate::msr_bitmap;
use crate::introspection;
use crate::mm::{self, free_contiguous, free_contiguous_page};
use crate::vmcs;
use crate::vmexit;
use crate::vm_launch;
use crate::vmx;
use shared_contract::{HypercallCode, HYPERCALL_KEY, HYPERVISOR_SIGNATURE};

const POOL_TAG: ULONG = u32::from_ne_bytes(*b"HvVc");

/// 与 `hv/hv/vcpu.h` 中 `guest_vpid` 一致。
const GUEST_VPID: u16 = 1;
/// `hv/vcpu.h::host_stack_size`（VM-exit 用 host 栈，6×4KiB）。
const HOST_STACK_SIZE: usize = 0x6000;
const HOST_STACK_PAGES: usize = HOST_STACK_SIZE / 4096;
/// `vcpu` 内 `host_gdt` / `host_tss` 均为 `alignas(0x1000)`，等价于连续两页（第一页仅 GDT，第二页仅 TSS）。
const HOST_GDT_TSS_PAGES: usize = 2;


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

/// 单 CPU 上的 VMX root 资源。
pub struct PerCpuState {
    pub vmxon_page: *mut u8,
    pub vmcs_page: *mut u8,
    pub vmxon_phys: u64,
    pub vmcs_phys: u64,
    pub msr_bitmap_page: *mut u8,
    pub msr_bitmap_phys: u64,
    pub host_stack_page: *mut u8,
    /// MSR 自动存/装列表（`hv` 的 `msr_exit_store` + `msr_entry_load`），单页。
    pub msr_auto_page: *mut u8,
    /// `prepare_host_idt`：256 项 × 16 字节。
    pub host_idt_page: *mut u8,
    /// GDT（前 32 字节）+ 对齐后的 `HostTaskState`。
    pub host_gdt_tss_page: *mut u8,
    pub ept: Option<EptState>,
    /// 与参考 `cache_cpu_data` 对齐的只读/伪造 MSR 缓存。
    pub cache: VcpuCache,
    /// 参考 `vcpu::queued_nmis`：host NMI 在 `dispatch_host_interrupt` 中递增。
    pub queued_nmis: u32,
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
            msr_auto_page: core::ptr::null_mut(),
            host_idt_page: core::ptr::null_mut(),
            host_gdt_tss_page: core::ptr::null_mut(),
            ept: None,
            cache: VcpuCache::empty(),
            queued_nmis: 0,
            vmxon_done: false,
        }
    }

    unsafe fn free_pages(&mut self) {
        unsafe {
            free_contiguous_page(self.vmxon_page);
            free_contiguous_page(self.vmcs_page);
            free_contiguous_page(self.msr_bitmap_page);
            free_contiguous(self.host_stack_page);
            free_contiguous_page(self.msr_auto_page);
            free_contiguous_page(self.host_idt_page);
            free_contiguous(self.host_gdt_tss_page);
        }
        if let Some(mut ept) = self.ept.take() {
            unsafe { ept.release() };
        }
        self.vmxon_page = core::ptr::null_mut();
        self.vmcs_page = core::ptr::null_mut();
        self.msr_bitmap_page = core::ptr::null_mut();
        self.host_stack_page = core::ptr::null_mut();
        self.msr_auto_page = core::ptr::null_mut();
        self.host_idt_page = core::ptr::null_mut();
        self.host_gdt_tss_page = core::ptr::null_mut();
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
    /// 与 `ghv.host_page_tables` 一致：全 VCPU 共享 host CR3 页表。
    host_page_tables: Option<HostPageTablesBox>,
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

    /// 在每个活动逻辑处理器上执行 `VMXON` + `VMCLEAR`/`VMPTRLD`（与 `hv::start` 使用 `KeQueryActiveProcessorCount(nullptr)` 一致）。
    ///
    /// # Safety
    /// 调用方必须处于 `IRQL <= APC_LEVEL`（与 `KeSetSystemAffinityThreadEx` 要求一致）。
    pub unsafe fn start() -> Result<Self, NTSTATUS> {
        logger::log("VmxCluster::start: Checking if VMX root is supported by platform...");
        if !vmx::host_supports_vmx_root() {
            logger::log("VmxCluster::start: VMX root not supported by platform. Aborting setup.");
            return Err(wdk_sys::STATUS_NOT_SUPPORTED);
        }

        logger::log("VmxCluster::start: Probing kernel structure offsets (calling introspection::find_offsets)...");
        if !unsafe { introspection::find_offsets() } {
            logger::log("VmxCluster::start: Failed to determine offsets via introspection::find_offsets (Ps* opcode probe). Aborting setup.");
            return Err(wdk_sys::STATUS_UNSUCCESSFUL);
        }

        logger::log("VmxCluster::start: Querying active processor count...");
        let count = unsafe { KeQueryActiveProcessorCount(core::ptr::null_mut()) } as u32;
        logger::log(&format!("VmxCluster::start: Active processor count detected: {}", count));
        if count == 0 {
            logger::log("VmxCluster::start: No active processors found. Aborting setup.");
            return Err(wdk_sys::STATUS_UNSUCCESSFUL);
        }

        let bytes = (count as usize).saturating_mul(size_of::<PerCpuState>());
        logger::log(&format!("VmxCluster::start: Allocating {} bytes for {} PerCpuState(s)...", bytes, count));
        let cpus = unsafe {
            ExAllocatePool2(
                POOL_FLAG_NON_PAGED,
                bytes as SIZE_T,
                POOL_TAG,
            )
        } as *mut PerCpuState;
        if cpus.is_null() {
            logger::log("VmxCluster::start: Failed to allocate per-CPU VMX state array (ExAllocatePool2 returned null).");
            return Err(wdk_sys::STATUS_INSUFFICIENT_RESOURCES);
        }
        unsafe {
            core::ptr::write_bytes(cpus, 0, count as usize);
        }
        logger::log("VmxCluster::start: Per-CPU VMX state array allocated and zeroed.");

        let mut cluster = VmxCluster {
            cpus,
            count,
            active: false,
            host_page_tables: None,
        };

        let system_cr3 = introspection::system_cr3().unwrap_or_else(arch::read_cr3);
        logger::log(&format!(
            "VmxCluster::start: Detected system CR3 value: 0x{:x}. Preparing host page tables...",
            system_cr3
        ));


        let Some(host_pt) = HostPageTables::prepare(system_cr3) else {
            logger::log("VmxCluster::start: Failed to prepare HostPageTables. Freeing per-CPU VMX state array.");
            unsafe { ExFreePoolWithTag(cpus.cast(), POOL_TAG) };
            return Err(wdk_sys::STATUS_INSUFFICIENT_RESOURCES);
        };

        logger::log("VmxCluster::start: HostPageTables successfully prepared.");
        
        logger::log(&format!(
            "VmxCluster::start: host VMCS CR3 candidate=0x{:x}; pml4[255]=0x{:x} pml4[256]=0x{:x} pml4[511]=0x{:x}",
            host_pt.cr3_value(),
            host_pt.pml4[255],
            host_pt.pml4[256],
            host_pt.pml4[511],
        ));

        for i in 0..count {
            let affinity: u64 = 1u64 << i;
            logger::log(&format!("VmxCluster::start: Setting affinity to logical processor {}...", i));
            let prev = unsafe { KeSetSystemAffinityThreadEx(affinity) };
            logger::log(&format!("VmxCluster::start: Initializing VMX state on processor {}...", i));
            let st = unsafe { cluster.init_cpu(i, &host_pt) };
            unsafe { KeRevertToUserAffinityThreadEx(prev) };
            if !wdk::nt_success(st) {
                logger::log(&format!("VmxCluster::start: VMX init failed on logical processor {} with NTSTATUS=0x{:x}. Starting rollback.", i, st));
                unsafe { cluster.rollback_partial(i) };
                unsafe { ExFreePoolWithTag(cpus.cast(), POOL_TAG) };
                drop(host_pt);
                return Err(st);
            }
            logger::log(&format!("VmxCluster::start: Successfully initialized VMX state on logical processor {}.", i));
        }

        cluster.host_page_tables = Some(host_pt);
        cluster.active = true;
        logger::log("VmxCluster::start: All logical processors initialized and VMXON complete. VmxCluster is now active.");
        Ok(cluster)
    }

    /// `virtualize_cpu` 失败时尚未写入 `PerCpuState` 的局部分配释放（与 `hv` 多页 host 栈 / 分离 GDT+TSS 页一致）。
    unsafe fn free_init_cpu_locals(
        vmxon: *mut u8,
        vmcs: *mut u8,
        msr_bitmap: *mut u8,
        host_stack: *mut u8,
        msr_auto: *mut u8,
        host_idt: *mut u8,
        host_gdt_tss: *mut u8,
    ) {
        unsafe {
            free_contiguous_page(vmxon);
            free_contiguous_page(vmcs);
            free_contiguous_page(msr_bitmap);
            free_contiguous(host_stack);
            free_contiguous_page(msr_auto);
            free_contiguous_page(host_idt);
            free_contiguous(host_gdt_tss);
        }
    }

    /// 与 `hv/vcpu.cpp::virtualize_cpu` 同序：`cache_cpu_data` → `enable_vmx_operation` → `enter_vmx_operation` →
    /// `load_vmcs_pointer` → `prepare_external_structures` → 写 VMCS → `vm_launch`。
    unsafe fn init_cpu(&mut self, index: u32, host_pt: &HostPageTables) -> NTSTATUS {
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
        let Some(host_stack) = (unsafe { mm::alloc_contiguous_pages(HOST_STACK_PAGES) }) else {
            logger::log("failed to allocate host VM-exit stack");
            unsafe {
                free_contiguous_page(vmxon);
                free_contiguous_page(vmcs);
                free_contiguous_page(msr_bitmap);
            }
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };
        let Some(msr_auto) = (unsafe { mm::alloc_contiguous_page() }) else {
            logger::log("failed to allocate MSR auto-load/store page");
            unsafe {
                Self::free_init_cpu_locals(vmxon, vmcs, msr_bitmap, host_stack, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
            }
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };
        let Some(host_idt) = (unsafe { mm::alloc_contiguous_page() }) else {
            logger::log("failed to allocate host IDT page");
            unsafe {
                Self::free_init_cpu_locals(vmxon, vmcs, msr_bitmap, host_stack, msr_auto, core::ptr::null_mut(), core::ptr::null_mut());
            }
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };
        let Some(host_gdt_tss) = (unsafe { mm::alloc_contiguous_pages(HOST_GDT_TSS_PAGES) }) else {
            logger::log("failed to allocate host GDT + TSS pages");
            unsafe {
                Self::free_init_cpu_locals(vmxon, vmcs, msr_bitmap, host_stack, msr_auto, host_idt, core::ptr::null_mut());
            }
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };

        let host_gdt = host_gdt_tss;
        let tss = unsafe { host_gdt_tss.add(4096).cast::<HostTaskState>() };

        // `enter_vmx_operation` / `load_vmcs_pointer`：仅填 revision（与 C++ 在 VMXON/VMPTRLD 前写头一致）。
        unsafe {
            vmcs::prepare_vmxon_region(vmxon);
            vmcs::prepare_vmcs_region(vmcs);
        }

        // `cache_cpu_data`
        fill_vcpu_cache(cpu);

        let Some(fixed) = VmxFixedMsrs::read() else {
            logger::log("failed to read VMX fixed MSRs");
            unsafe {
                Self::free_init_cpu_locals(vmxon, vmcs, msr_bitmap, host_stack, msr_auto, host_idt, host_gdt_tss);
            }
            return wdk_sys::STATUS_NOT_SUPPORTED;
        };

        if !unsafe { arch::enable_vmx_in_hardware(&fixed) } {
            logger::log("enable_vmx_in_hardware returned false");
            unsafe {
                Self::free_init_cpu_locals(vmxon, vmcs, msr_bitmap, host_stack, msr_auto, host_idt, host_gdt_tss);
            }
            return wdk_sys::STATUS_NOT_SUPPORTED;
        }

        let vmxon_phys = unsafe { mm::physical_address(vmxon) };
        let vmcs_phys = unsafe { mm::physical_address(vmcs) };

        if !unsafe { vmx::vmxon(&raw const vmxon_phys) } {
            logger::log("VMXON failed");
            unsafe {
                arch::disable_vmx_hardware();
                Self::free_init_cpu_locals(vmxon, vmcs, msr_bitmap, host_stack, msr_auto, host_idt, host_gdt_tss);
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if !unsafe { vmx::invept_all_contexts() } {
            logger::log("INVEPT(all-context) after VMXON failed");
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                Self::free_init_cpu_locals(vmxon, vmcs, msr_bitmap, host_stack, msr_auto, host_idt, host_gdt_tss);
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if !unsafe { vmx::vmclear(&raw const vmcs_phys) } {
            logger::log("VMCLEAR failed");
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                Self::free_init_cpu_locals(vmxon, vmcs, msr_bitmap, host_stack, msr_auto, host_idt, host_gdt_tss);
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        if !unsafe { vmx::vmptrld(&raw const vmcs_phys) } {
            logger::log("VMPTRLD failed");
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                Self::free_init_cpu_locals(vmxon, vmcs, msr_bitmap, host_stack, msr_auto, host_idt, host_gdt_tss);
            }
            return wdk_sys::STATUS_UNSUCCESSFUL;
        }

        // `prepare_external_structures`：须在 `load_vmcs_pointer` 之后（与 `vcpu.cpp` 顺序一致）。
        unsafe {
            msr_bitmap::prepare_msr_bitmap_page(msr_bitmap);
            core::ptr::write_bytes(host_stack, 0, HOST_STACK_SIZE);
            vmcs::prepare_msr_auto_lists(msr_auto);
            core::ptr::write_bytes(tss.cast::<u8>(), 0, size_of::<HostTaskState>());
            host_descriptor::prepare_host_idt(host_idt);
            host_descriptor::prepare_host_gdt(host_gdt, tss.cast_const());
        }

        let Some(ept) = (unsafe { EptState::build_identity_skeleton() }) else {
            logger::log("EPT build_identity_skeleton failed");
            let _ = unsafe { vmx::vmxoff() };
            unsafe {
                arch::disable_vmx_hardware();
                Self::free_init_cpu_locals(vmxon, vmcs, msr_bitmap, host_stack, msr_auto, host_idt, host_gdt_tss);
            }
            return wdk_sys::STATUS_INSUFFICIENT_RESOURCES;
        };

        let msr_bitmap_phys = unsafe { mm::physical_address(msr_bitmap) };
        cpu.vmxon_page = vmxon;
        cpu.vmcs_page = vmcs;
        cpu.vmxon_phys = vmxon_phys;
        cpu.vmcs_phys = vmcs_phys;
        cpu.msr_bitmap_page = msr_bitmap;
        cpu.msr_bitmap_phys = msr_bitmap_phys;
        cpu.host_stack_page = host_stack;
        cpu.msr_auto_page = msr_auto;
        cpu.host_idt_page = host_idt;
        cpu.host_gdt_tss_page = host_gdt_tss;
        cpu.ept = Some(ept);

        let msr_auto_phys = unsafe { mm::physical_address(msr_auto) };
        let system_cr3 = introspection::system_cr3().unwrap_or_else(arch::read_cr3);
        let ctrl_params = vmcs::VmcsControlParams {
            msr_bitmap_pa: Some(cpu.msr_bitmap_phys),
            eptp: cpu.ept.as_ref().map(|v| v.eptp.0),
            vpid: GUEST_VPID,
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

        // `hv/vmcs.cpp::write_vmcs_host_fields`：`((host_stack + host_stack_size) & ~0xF) - 8`
        let host_rsp_top = host_stack as u64 + HOST_STACK_SIZE as u64;
        let host_rsp = (host_rsp_top & !0xFu64).saturating_sub(8);
        let host_entry = vmexit::vmexit_host_stub as *const () as usize as u64;
        let cpu_self_va = unsafe { self.cpus.add(index as usize) } as u64;
        let tss_va = tss as u64;
        let host_layout = vmcs::HostVmcsLayout {
            rip: host_entry,
            rsp: host_rsp,
            cr3: host_pt.cr3_value(),
            gdtr_base: host_gdt as u64,
            idtr_base: host_idt as u64,
            tr_base: tss_va,
            fs_base: cpu_self_va,
        };
        if let Err(err) = unsafe { vmcs::configure_host_state(&host_layout) } {
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

        self.host_page_tables.take();

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

