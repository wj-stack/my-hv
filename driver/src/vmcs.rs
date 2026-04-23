//! VMCS 区域初始化与访问封装。对应 `hv/hv/vmcs.h`、`hv/hv/vmcs.cpp`。

use crate::{arch, ia32};
use crate::{gdt, idt, segment};

/// 与 `hv/hv/vmx.h` 中 `vmx_msr_entry` 一致：VM-exit MSR 存区 / VM-entry MSR 装区条目（16 字节）。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VmxMsrEntry {
    pub msr_index: u32,
    pub reserved: u32,
    pub data: u64,
}

/// `write_vmcs_ctrl_fields` 中 VM-exit MSR store 条目数（TSC、PERF_GLOBAL、APERF、MPERF）。
pub const VMX_MSR_AUTOSTORE_COUNT: u32 = 4;
/// VM-entry MSR load 条目数（APERF、MPERF）。
pub const VMX_MSR_AUTOLOAD_COUNT: u32 = 2;

/// 供 `configure_control_fields` 使用，与 `hv/hv/vmcs.cpp::write_vmcs_ctrl_fields` 同序字段。
pub struct VmcsControlParams {
    pub msr_bitmap_pa: Option<u64>,
    pub eptp: Option<u64>,
    pub vpid: u16,
    /// 与参考工程 `ghv.system_cr3` 一致：通常为 System 进程目录表；`None` 则不使用 CR3-target。
    pub cr3_target: Option<u64>,
    pub msr_exit_store_pa: u64,
    pub msr_entry_load_pa: u64,
}

/// VMCS 字段编码（Intel SDM full encoding）。
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmcsField(pub u32);

#[allow(dead_code)]
impl VmcsField {
    pub const VM_INSTRUCTION_ERROR: Self = Self(ia32::VMCS_VM_INSTRUCTION_ERROR);
    pub const EXIT_REASON: Self = Self(ia32::VMCS_EXIT_REASON);
    pub const GUEST_RIP: Self = Self(ia32::VMCS_GUEST_RIP);
    pub const GUEST_RSP: Self = Self(ia32::VMCS_GUEST_RSP);
    pub const GUEST_RFLAGS: Self = Self(ia32::VMCS_GUEST_RFLAGS);
    pub const GUEST_CR3: Self = Self(ia32::VMCS_GUEST_CR3);
    pub const CTRL_EPT_POINTER: Self = Self(ia32::VMCS_CTRL_EPT_POINTER);
    pub const CTRL_CR0_READ_SHADOW: Self = Self(ia32::VMCS_CTRL_CR0_READ_SHADOW);
    pub const CTRL_CR4_READ_SHADOW: Self = Self(ia32::VMCS_CTRL_CR4_READ_SHADOW);
    pub const CTRL_PIN_BASED: Self = Self(ia32::VMCS_CTRL_PIN_BASED);
    pub const CTRL_CPU_BASED: Self = Self(ia32::VMCS_CTRL_CPU_BASED);
    pub const CTRL_SECONDARY_CPU_BASED: Self = Self(ia32::VMCS_CTRL_SECONDARY_CPU_BASED);
    pub const CTRL_EXCEPTION_BITMAP: Self = Self(ia32::VMCS_CTRL_EXCEPTION_BITMAP);
    pub const CTRL_VMEXIT_CONTROLS: Self = Self(ia32::VMCS_CTRL_VMEXIT_CONTROLS);
    pub const CTRL_VMENTRY_CONTROLS: Self = Self(ia32::VMCS_CTRL_VMENTRY_CONTROLS);
    pub const CTRL_MSR_BITMAP_ADDRESS: Self = Self(ia32::VMCS_CTRL_MSR_BITMAP_ADDRESS);
    pub const CTRL_CR3_TARGET_COUNT: Self = Self(ia32::VMCS_CTRL_CR3_TARGET_COUNT);
    pub const CTRL_TSC_OFFSET: Self = Self(ia32::VMCS_CTRL_TSC_OFFSET);
    pub const CTRL_PAGEFAULT_ERROR_CODE_MASK: Self = Self(ia32::VMCS_CTRL_PAGEFAULT_ERROR_CODE_MASK);
    pub const CTRL_PAGEFAULT_ERROR_CODE_MATCH: Self = Self(ia32::VMCS_CTRL_PAGEFAULT_ERROR_CODE_MATCH);
    pub const CTRL_VMEXIT_MSR_STORE_COUNT: Self = Self(ia32::VMCS_CTRL_VMEXIT_MSR_STORE_COUNT);
    pub const CTRL_VMEXIT_MSR_LOAD_COUNT: Self = Self(ia32::VMCS_CTRL_VMEXIT_MSR_LOAD_COUNT);
    pub const CTRL_VMENTRY_MSR_LOAD_COUNT: Self = Self(ia32::VMCS_CTRL_VMENTRY_MSR_LOAD_COUNT);
    pub const CTRL_CR0_GUEST_HOST_MASK: Self = Self(ia32::VMCS_CTRL_CR0_GUEST_HOST_MASK);
    pub const CTRL_CR4_GUEST_HOST_MASK: Self = Self(ia32::VMCS_CTRL_CR4_GUEST_HOST_MASK);
    pub const CTRL_CR3_TARGET_VALUE0: Self = Self(ia32::VMCS_CTRL_CR3_TARGET_VALUE0);
    pub const CTRL_VIRTUAL_PROCESSOR_ID: Self = Self(ia32::VMCS_CTRL_VIRTUAL_PROCESSOR_ID);
    pub const CTRL_VMEXIT_MSR_STORE_ADDRESS: Self = Self(ia32::VMCS_CTRL_VMEXIT_MSR_STORE_ADDRESS);
    pub const CTRL_VMENTRY_MSR_LOAD_ADDRESS: Self = Self(ia32::VMCS_CTRL_VMENTRY_MSR_LOAD_ADDRESS);
    pub const HOST_CR0: Self = Self(ia32::VMCS_HOST_CR0);
    pub const HOST_CR3: Self = Self(ia32::VMCS_HOST_CR3);
    pub const HOST_CR4: Self = Self(ia32::VMCS_HOST_CR4);
    pub const HOST_RSP: Self = Self(ia32::VMCS_HOST_RSP);
    pub const HOST_RIP: Self = Self(ia32::VMCS_HOST_RIP);
    pub const HOST_GDTR_BASE: Self = Self(ia32::VMCS_HOST_GDTR_BASE);
    pub const HOST_IDTR_BASE: Self = Self(ia32::VMCS_HOST_IDTR_BASE);
    pub const GUEST_CR0: Self = Self(ia32::VMCS_GUEST_CR0);
    pub const GUEST_CR4: Self = Self(ia32::VMCS_GUEST_CR4);
    pub const GUEST_DR7: Self = Self(ia32::VMCS_GUEST_DR7);
    pub const GUEST_VMCS_LINK_POINTER: Self = Self(ia32::VMCS_GUEST_VMCS_LINK_POINTER);
    pub const GUEST_DEBUGCTL: Self = Self(ia32::VMCS_GUEST_DEBUGCTL);
    pub const GUEST_PAT: Self = Self(ia32::VMCS_GUEST_PAT);
    pub const GUEST_EFER: Self = Self(ia32::VMCS_GUEST_EFER);
    pub const GUEST_IA32_PERF_GLOBAL_CTRL: Self = Self(ia32::VMCS_GUEST_IA32_PERF_GLOBAL_CTRL);
    pub const GUEST_PENDING_DEBUG_EXCEPTIONS: Self = Self(ia32::VMCS_GUEST_PENDING_DEBUG_EXCEPTIONS);
    pub const GUEST_ES_SELECTOR: Self = Self(ia32::VMCS_GUEST_ES_SELECTOR);
    pub const GUEST_CS_SELECTOR: Self = Self(ia32::VMCS_GUEST_CS_SELECTOR);
    pub const GUEST_SS_SELECTOR: Self = Self(ia32::VMCS_GUEST_SS_SELECTOR);
    pub const GUEST_DS_SELECTOR: Self = Self(ia32::VMCS_GUEST_DS_SELECTOR);
    pub const GUEST_FS_SELECTOR: Self = Self(ia32::VMCS_GUEST_FS_SELECTOR);
    pub const GUEST_GS_SELECTOR: Self = Self(ia32::VMCS_GUEST_GS_SELECTOR);
    pub const GUEST_LDTR_SELECTOR: Self = Self(ia32::VMCS_GUEST_LDTR_SELECTOR);
    pub const GUEST_TR_SELECTOR: Self = Self(ia32::VMCS_GUEST_TR_SELECTOR);
    pub const GUEST_ES_LIMIT: Self = Self(ia32::VMCS_GUEST_ES_LIMIT);
    pub const GUEST_CS_LIMIT: Self = Self(ia32::VMCS_GUEST_CS_LIMIT);
    pub const GUEST_SS_LIMIT: Self = Self(ia32::VMCS_GUEST_SS_LIMIT);
    pub const GUEST_DS_LIMIT: Self = Self(ia32::VMCS_GUEST_DS_LIMIT);
    pub const GUEST_FS_LIMIT: Self = Self(ia32::VMCS_GUEST_FS_LIMIT);
    pub const GUEST_GS_LIMIT: Self = Self(ia32::VMCS_GUEST_GS_LIMIT);
    pub const GUEST_LDTR_LIMIT: Self = Self(ia32::VMCS_GUEST_LDTR_LIMIT);
    pub const GUEST_TR_LIMIT: Self = Self(ia32::VMCS_GUEST_TR_LIMIT);
    pub const GUEST_GDTR_LIMIT: Self = Self(ia32::VMCS_GUEST_GDTR_LIMIT);
    pub const GUEST_IDTR_LIMIT: Self = Self(ia32::VMCS_GUEST_IDTR_LIMIT);
    pub const GUEST_ES_ACCESS_RIGHTS: Self = Self(ia32::VMCS_GUEST_ES_ACCESS_RIGHTS);
    pub const GUEST_CS_ACCESS_RIGHTS: Self = Self(ia32::VMCS_GUEST_CS_ACCESS_RIGHTS);
    pub const GUEST_SS_ACCESS_RIGHTS: Self = Self(ia32::VMCS_GUEST_SS_ACCESS_RIGHTS);
    pub const GUEST_DS_ACCESS_RIGHTS: Self = Self(ia32::VMCS_GUEST_DS_ACCESS_RIGHTS);
    pub const GUEST_FS_ACCESS_RIGHTS: Self = Self(ia32::VMCS_GUEST_FS_ACCESS_RIGHTS);
    pub const GUEST_GS_ACCESS_RIGHTS: Self = Self(ia32::VMCS_GUEST_GS_ACCESS_RIGHTS);
    pub const GUEST_LDTR_ACCESS_RIGHTS: Self = Self(ia32::VMCS_GUEST_LDTR_ACCESS_RIGHTS);
    pub const GUEST_TR_ACCESS_RIGHTS: Self = Self(ia32::VMCS_GUEST_TR_ACCESS_RIGHTS);
    pub const GUEST_INTERRUPTIBILITY_STATE: Self = Self(ia32::VMCS_GUEST_INTERRUPTIBILITY_STATE);
    pub const GUEST_ACTIVITY_STATE: Self = Self(ia32::VMCS_GUEST_ACTIVITY_STATE);
    pub const GUEST_IA32_SYSENTER_CS: Self = Self(ia32::VMCS_GUEST_IA32_SYSENTER_CS);
    pub const GUEST_ES_BASE: Self = Self(ia32::VMCS_GUEST_ES_BASE);
    pub const GUEST_CS_BASE: Self = Self(ia32::VMCS_GUEST_CS_BASE);
    pub const GUEST_SS_BASE: Self = Self(ia32::VMCS_GUEST_SS_BASE);
    pub const GUEST_DS_BASE: Self = Self(ia32::VMCS_GUEST_DS_BASE);
    pub const GUEST_FS_BASE: Self = Self(ia32::VMCS_GUEST_FS_BASE);
    pub const GUEST_GS_BASE: Self = Self(ia32::VMCS_GUEST_GS_BASE);
    pub const GUEST_LDTR_BASE: Self = Self(ia32::VMCS_GUEST_LDTR_BASE);
    pub const GUEST_TR_BASE: Self = Self(ia32::VMCS_GUEST_TR_BASE);
    pub const GUEST_GDTR_BASE: Self = Self(ia32::VMCS_GUEST_GDTR_BASE);
    pub const GUEST_IDTR_BASE: Self = Self(ia32::VMCS_GUEST_IDTR_BASE);
    pub const GUEST_IA32_SYSENTER_ESP: Self = Self(ia32::VMCS_GUEST_IA32_SYSENTER_ESP);
    pub const GUEST_IA32_SYSENTER_EIP: Self = Self(ia32::VMCS_GUEST_IA32_SYSENTER_EIP);
    pub const HOST_ES_SELECTOR: Self = Self(ia32::VMCS_HOST_ES_SELECTOR);
    pub const HOST_CS_SELECTOR: Self = Self(ia32::VMCS_HOST_CS_SELECTOR);
    pub const HOST_SS_SELECTOR: Self = Self(ia32::VMCS_HOST_SS_SELECTOR);
    pub const HOST_DS_SELECTOR: Self = Self(ia32::VMCS_HOST_DS_SELECTOR);
    pub const HOST_FS_SELECTOR: Self = Self(ia32::VMCS_HOST_FS_SELECTOR);
    pub const HOST_GS_SELECTOR: Self = Self(ia32::VMCS_HOST_GS_SELECTOR);
    pub const HOST_TR_SELECTOR: Self = Self(ia32::VMCS_HOST_TR_SELECTOR);
    pub const HOST_FS_BASE: Self = Self(ia32::VMCS_HOST_FS_BASE);
    pub const HOST_GS_BASE: Self = Self(ia32::VMCS_HOST_GS_BASE);
    pub const HOST_TR_BASE: Self = Self(ia32::VMCS_HOST_TR_BASE);
    pub const HOST_IA32_SYSENTER_CS: Self = Self(ia32::VMCS_HOST_IA32_SYSENTER_CS);
    pub const HOST_IA32_SYSENTER_ESP: Self = Self(ia32::VMCS_HOST_IA32_SYSENTER_ESP);
    pub const HOST_IA32_SYSENTER_EIP: Self = Self(ia32::VMCS_HOST_IA32_SYSENTER_EIP);
    pub const HOST_PAT: Self = Self(ia32::VMCS_HOST_PAT);
    pub const HOST_EFER: Self = Self(ia32::VMCS_HOST_EFER);
    pub const HOST_IA32_PERF_GLOBAL_CTRL: Self = Self(ia32::VMCS_HOST_IA32_PERF_GLOBAL_CTRL);
    pub const EXIT_QUALIFICATION: Self = Self(ia32::VMCS_EXIT_QUALIFICATION);
    pub const VMEXIT_INSTRUCTION_LEN: Self = Self(ia32::VMCS_VM_EXIT_INSTRUCTION_LEN);
    pub const VMEXIT_INTERRUPTION_INFO: Self = Self(ia32::VMCS_VMEXIT_INTERRUPTION_INFO);
    pub const VMENTRY_INTERRUPTION_INFO: Self = Self(ia32::VMCS_VMENTRY_INTERRUPTION_INFO);
    pub const VMENTRY_EXCEPTION_ERROR_CODE: Self = Self(ia32::VMCS_VMENTRY_EXCEPTION_ERROR_CODE);
    pub const VMENTRY_INSTRUCTION_LEN: Self = Self(ia32::VMCS_VMENTRY_INSTRUCTION_LEN);
    pub const GUEST_PHYSICAL_ADDRESS: Self = Self(ia32::VMCS_GUEST_PHYSICAL_ADDRESS);
    pub const GUEST_LINEAR_ADDRESS: Self = Self(ia32::VMCS_GUEST_LINEAR_ADDRESS);
    pub const GUEST_VMX_PREEMPTION_TIMER: Self = Self(ia32::VMCS_GUEST_VMX_PREEMPTION_TIMER);

    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

const CR4_VMXE: u64 = 1 << 13;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VmxInstructionStatus {
    Success,
    VmFailValid,
    VmFailInvalid,
}

/// VMCS 访问失败信息。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmcsAccessError {
    VmFailInvalid {
        field: VmcsField,
    },
    VmFailValid {
        field: VmcsField,
        /// 若可读取，给出 `VM_INSTRUCTION_ERROR`。
        instruction_error: Option<u32>,
    },
}

/// VMCS 中的基本 guest 状态快照（用于 bring-up 期诊断）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuestStateSnapshot {
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
}

/// `VMCS_EXIT_REASON` 的解码结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmExitReason {
    pub raw: u32,
    pub basic: u16,
    pub failed_vmentry: bool,
    pub from_vmx_root: bool,
}

impl VmExitReason {
    #[inline]
    pub fn from_raw(raw: u32) -> Self {
        Self {
            raw,
            basic: (raw & 0xFFFF) as u16,
            failed_vmentry: (raw & (1 << 31)) != 0,
            from_vmx_root: (raw & (1 << 29)) != 0,
        }
    }
}

#[inline]
fn decode_vmx_status(rflags: u64) -> VmxInstructionStatus {
    let cf = (rflags & 0x1) != 0;
    let zf = (rflags & (1 << 6)) != 0;
    if cf {
        VmxInstructionStatus::VmFailInvalid
    } else if zf {
        VmxInstructionStatus::VmFailValid
    } else {
        VmxInstructionStatus::Success
    }
}

#[inline]
unsafe fn vmwrite_raw(field: VmcsField, value: u64) -> VmxInstructionStatus {
    let rflags: u64;
    unsafe {
        core::arch::asm!(
            "vmwrite {value}, {field}",
            "pushfq",
            "pop {rflags}",
            value = in(reg) value,
            field = in(reg) field.raw() as u64,
            rflags = lateout(reg) rflags,
        );
    }
    decode_vmx_status(rflags)
}

#[inline]
unsafe fn vmread_raw(field: VmcsField) -> (VmxInstructionStatus, u64) {
    let value: u64;
    let rflags: u64;
    unsafe {
        core::arch::asm!(
            "vmread {value}, {field}",
            "pushfq",
            "pop {rflags}",
            value = lateout(reg) value,
            field = in(reg) field.raw() as u64,
            rflags = lateout(reg) rflags,
        );
    }
    (decode_vmx_status(rflags), value)
}

#[inline]
unsafe fn read_vm_instruction_error() -> Option<u32> {
    let (status, value) = unsafe { vmread_raw(VmcsField::VM_INSTRUCTION_ERROR) };
    if matches!(status, VmxInstructionStatus::Success) {
        Some(value as u32)
    } else {
        None
    }
}

#[inline]
fn access_error_from_status(field: VmcsField, status: VmxInstructionStatus) -> VmcsAccessError {
    match status {
        VmxInstructionStatus::VmFailInvalid => VmcsAccessError::VmFailInvalid { field },
        VmxInstructionStatus::VmFailValid => VmcsAccessError::VmFailValid {
            field,
            instruction_error: unsafe { read_vm_instruction_error() },
        },
        VmxInstructionStatus::Success => unreachable!("success should not be converted to error"),
    }
}

/// 对当前 VMCS 执行 `VMWRITE`。
///
/// # Safety
/// 调用前必须已 `VMPTRLD` 当前 VMCS，且运行在 VMX root。
pub unsafe fn vmwrite(field: VmcsField, value: u64) -> Result<(), VmcsAccessError> {
    let status = unsafe { vmwrite_raw(field, value) };
    if matches!(status, VmxInstructionStatus::Success) {
        Ok(())
    } else {
        Err(access_error_from_status(field, status))
    }
}

/// 对当前 VMCS 执行 `VMREAD`。
///
/// # Safety
/// 调用前必须已 `VMPTRLD` 当前 VMCS，且运行在 VMX root。
pub unsafe fn vmread(field: VmcsField) -> Result<u64, VmcsAccessError> {
    let (status, value) = unsafe { vmread_raw(field) };
    if matches!(status, VmxInstructionStatus::Success) {
        Ok(value)
    } else {
        Err(access_error_from_status(field, status))
    }
}

#[inline]
unsafe fn vmwrite_segment(
    selector: VmcsField,
    limit: VmcsField,
    ar: VmcsField,
    base: VmcsField,
    ps: segment::ParsedSegment,
) -> Result<(), VmcsAccessError> {
    unsafe {
        vmwrite(selector, ps.selector as u64)?;
        vmwrite(limit, ps.limit as u64)?;
        vmwrite(ar, ps.access_rights as u64)?;
        vmwrite(base, ps.base)?;
    }
    Ok(())
}

/// Guest 段、GDTR/IDTR、SYSENTER、DEBUGCTL/PAT 镜像当前 CPU（与 `hv/vmcs.cpp::write_vmcs_guest_fields` 一致，不单独写 `GUEST_EFER`）。
///
/// # Safety
/// 需要已 `VMPTRLD`。
pub unsafe fn configure_guest_segment_state() -> Result<(), VmcsAccessError> {
    let gdtr = gdt::read_gdtr();
    let idtr = idt::read_idtr();
    let gdtr_limit = unsafe { core::ptr::addr_of!(gdtr.limit).read_unaligned() } as u64;
    let gdtr_base = unsafe { core::ptr::addr_of!(gdtr.base).read_unaligned() };
    let idtr_limit = unsafe { core::ptr::addr_of!(idtr.limit).read_unaligned() } as u64;
    let idtr_base = unsafe { core::ptr::addr_of!(idtr.base).read_unaligned() };

    let sel = segment::read_segment_selectors();
    let es = segment::parse_segment(&gdtr, sel.es);
    let cs = segment::parse_segment(&gdtr, sel.cs);
    let ss = segment::parse_segment(&gdtr, sel.ss);
    let ds = segment::parse_segment(&gdtr, sel.ds);
    let mut fs = segment::parse_segment(&gdtr, sel.fs);
    let mut gs = segment::parse_segment(&gdtr, sel.gs);
    fs.base = unsafe { arch::rdmsr(ia32::IA32_FS_BASE) };
    gs.base = unsafe { arch::rdmsr(ia32::IA32_GS_BASE) };
    let tr = segment::parse_segment(&gdtr, sel.tr);
    let ldtr = segment::parse_ldtr(&gdtr);

    unsafe {
        vmwrite_segment(
            VmcsField::GUEST_ES_SELECTOR,
            VmcsField::GUEST_ES_LIMIT,
            VmcsField::GUEST_ES_ACCESS_RIGHTS,
            VmcsField::GUEST_ES_BASE,
            es,
        )?;
        vmwrite_segment(
            VmcsField::GUEST_CS_SELECTOR,
            VmcsField::GUEST_CS_LIMIT,
            VmcsField::GUEST_CS_ACCESS_RIGHTS,
            VmcsField::GUEST_CS_BASE,
            cs,
        )?;
        vmwrite_segment(
            VmcsField::GUEST_SS_SELECTOR,
            VmcsField::GUEST_SS_LIMIT,
            VmcsField::GUEST_SS_ACCESS_RIGHTS,
            VmcsField::GUEST_SS_BASE,
            ss,
        )?;
        vmwrite_segment(
            VmcsField::GUEST_DS_SELECTOR,
            VmcsField::GUEST_DS_LIMIT,
            VmcsField::GUEST_DS_ACCESS_RIGHTS,
            VmcsField::GUEST_DS_BASE,
            ds,
        )?;
        vmwrite_segment(
            VmcsField::GUEST_FS_SELECTOR,
            VmcsField::GUEST_FS_LIMIT,
            VmcsField::GUEST_FS_ACCESS_RIGHTS,
            VmcsField::GUEST_FS_BASE,
            fs,
        )?;
        vmwrite_segment(
            VmcsField::GUEST_GS_SELECTOR,
            VmcsField::GUEST_GS_LIMIT,
            VmcsField::GUEST_GS_ACCESS_RIGHTS,
            VmcsField::GUEST_GS_BASE,
            gs,
        )?;
        vmwrite_segment(
            VmcsField::GUEST_LDTR_SELECTOR,
            VmcsField::GUEST_LDTR_LIMIT,
            VmcsField::GUEST_LDTR_ACCESS_RIGHTS,
            VmcsField::GUEST_LDTR_BASE,
            ldtr,
        )?;
        vmwrite_segment(
            VmcsField::GUEST_TR_SELECTOR,
            VmcsField::GUEST_TR_LIMIT,
            VmcsField::GUEST_TR_ACCESS_RIGHTS,
            VmcsField::GUEST_TR_BASE,
            tr,
        )?;

        vmwrite(VmcsField::GUEST_GDTR_LIMIT, gdtr_limit)?;
        vmwrite(VmcsField::GUEST_GDTR_BASE, gdtr_base)?;
        vmwrite(VmcsField::GUEST_IDTR_LIMIT, idtr_limit)?;
        vmwrite(VmcsField::GUEST_IDTR_BASE, idtr_base)?;

        vmwrite(
            VmcsField::GUEST_IA32_SYSENTER_CS,
            unsafe { arch::rdmsr(ia32::IA32_SYSENTER_CS) } & 0xFFFF,
        )?;
        vmwrite(
            VmcsField::GUEST_IA32_SYSENTER_ESP,
            unsafe { arch::rdmsr(ia32::IA32_SYSENTER_ESP) },
        )?;
        vmwrite(
            VmcsField::GUEST_IA32_SYSENTER_EIP,
            unsafe { arch::rdmsr(ia32::IA32_SYSENTER_EIP) },
        )?;

        vmwrite(VmcsField::GUEST_DEBUGCTL, unsafe { arch::rdmsr(ia32::IA32_DEBUGCTL) })?;
        vmwrite(VmcsField::GUEST_PAT, unsafe { arch::rdmsr(ia32::IA32_PAT) })?;

        vmwrite(VmcsField::GUEST_INTERRUPTIBILITY_STATE, 0)?;
        vmwrite(VmcsField::GUEST_ACTIVITY_STATE, 0)?;
    }
    Ok(())
}

fn control_msr(use_true: u64, true_msr: u32, fallback: u32) -> u32 {
    ia32::vmx_true_controls_msr_or_default(use_true, true_msr, fallback)
}

/// 根据 VMX capability MSR 约束计算控制值并写入 VMCS（对齐 `hv/hv/vmcs.cpp::write_vmcs_ctrl_fields`）。
///
/// # Safety
/// 需要已 `VMPTRLD`。
pub unsafe fn configure_control_fields(params: &VmcsControlParams) -> Result<(), VmcsAccessError> {
    let basic = unsafe { arch::rdmsr(ia32::IA32_VMX_BASIC) };

    let pin_msr = control_msr(
        basic,
        ia32::IA32_VMX_TRUE_PINBASED_CTLS,
        ia32::IA32_VMX_PINBASED_CTLS,
    );
    let proc_msr = control_msr(
        basic,
        ia32::IA32_VMX_TRUE_PROCBASED_CTLS,
        ia32::IA32_VMX_PROCBASED_CTLS,
    );
    let exit_msr = control_msr(
        basic,
        ia32::IA32_VMX_TRUE_EXIT_CTLS,
        ia32::IA32_VMX_EXIT_CTLS,
    );
    let entry_msr = control_msr(
        basic,
        ia32::IA32_VMX_TRUE_ENTRY_CTLS,
        ia32::IA32_VMX_ENTRY_CTLS,
    );

    let pin_req = ia32::PIN_BASED_NMI_EXITING
        | ia32::PIN_BASED_VIRTUAL_NMI
        | ia32::PIN_BASED_ACTIVATE_VMX_PREEMPTION_TIMER;
    let pin = ia32::adjust_vmx_control(pin_req, unsafe { arch::rdmsr(pin_msr) });

    // 与 `hv/vmcs.cpp` 一致：primary 仅 CR3-load exiting + MSR 位图 + TSC offset + secondary。
    let primary_requested = ia32::CPU_BASED_USE_MSR_BITMAPS
        | ia32::CPU_BASED_ACTIVATE_SECONDARY_CONTROLS
        | ia32::CPU_BASED_USE_TSC_OFFSETTING
        | ia32::CPU_BASED_CR3_LOAD_EXITING;
    let primary = ia32::adjust_vmx_control(primary_requested, unsafe { arch::rdmsr(proc_msr) });

    let exit_ctrl = ia32::adjust_vmx_control(
        ia32::EXIT_CONTROL_HOST_ADDR_SPACE_SIZE
            | ia32::EXIT_CONTROL_SAVE_DEBUG_CONTROLS
            | ia32::EXIT_CONTROL_SAVE_IA32_PAT
            | ia32::EXIT_CONTROL_LOAD_IA32_PAT
            | ia32::EXIT_CONTROL_LOAD_IA32_PERF_GLOBAL_CTRL
            | ia32::EXIT_CONTROL_CONCEAL_VMX_FROM_PT,
        unsafe { arch::rdmsr(exit_msr) },
    );
    let entry_ctrl = ia32::adjust_vmx_control(
        ia32::ENTRY_CONTROL_IA32E_MODE_GUEST
            | ia32::ENTRY_CONTROL_LOAD_DEBUG_CONTROLS
            | ia32::ENTRY_CONTROL_LOAD_IA32_PAT
            | ia32::ENTRY_CONTROL_LOAD_IA32_PERF_GLOBAL_CTRL
            | ia32::ENTRY_CONTROL_CONCEAL_VMX_FROM_PT,
        unsafe { arch::rdmsr(entry_msr) },
    );

    let mut secondary_req = ia32::SECONDARY_CONTROL_ENABLE_RDTSCP
        | ia32::SECONDARY_CONTROL_ENABLE_INVPCID
        | ia32::SECONDARY_CONTROL_ENABLE_XSAVES_XRSTORS
        | ia32::SECONDARY_CONTROL_ENABLE_USER_WAIT_PAUSE
        | ia32::SECONDARY_CONTROL_CONCEAL_VMX_FROM_PT;
    if params.eptp.is_some() {
        secondary_req |= ia32::SECONDARY_CONTROL_ENABLE_EPT | ia32::SECONDARY_CONTROL_ENABLE_VPID;
    }
    let secondary = ia32::adjust_vmx_control(
        secondary_req,
        unsafe { arch::rdmsr(ia32::IA32_VMX_PROCBASED_CTLS2) },
    );

    let (cr0_mask, cr4_mask) = if cfg!(debug_assertions) {
        (!0u64, !0u64)
    } else {
        let cr0_fixed0 = unsafe { arch::rdmsr(ia32::IA32_VMX_CR0_FIXED0) };
        let cr0_fixed1 = unsafe { arch::rdmsr(ia32::IA32_VMX_CR0_FIXED1) };
        let cr4_fixed0 = unsafe { arch::rdmsr(ia32::IA32_VMX_CR4_FIXED0) };
        let cr4_fixed1 = unsafe { arch::rdmsr(ia32::IA32_VMX_CR4_FIXED1) };
        (
            cr0_fixed0 | !cr0_fixed1 | ia32::CR0_CACHE_DISABLE_FLAG | ia32::CR0_WRITE_PROTECT_FLAG,
            cr4_fixed0 | !cr4_fixed1,
        )
    };

    unsafe {
        vmwrite(VmcsField::CTRL_PIN_BASED, pin as u64)?;
        vmwrite(VmcsField::CTRL_CPU_BASED, primary as u64)?;
        vmwrite(VmcsField::CTRL_SECONDARY_CPU_BASED, secondary as u64)?;
        vmwrite(VmcsField::CTRL_VMEXIT_CONTROLS, exit_ctrl as u64)?;
        vmwrite(VmcsField::CTRL_VMENTRY_CONTROLS, entry_ctrl as u64)?;
        vmwrite(VmcsField::CTRL_EXCEPTION_BITMAP, 0)?;
        vmwrite(VmcsField::CTRL_PAGEFAULT_ERROR_CODE_MASK, 0)?;
        vmwrite(VmcsField::CTRL_PAGEFAULT_ERROR_CODE_MATCH, 0)?;
        vmwrite(VmcsField::CTRL_TSC_OFFSET, 0)?;

        vmwrite(VmcsField::CTRL_CR0_GUEST_HOST_MASK, cr0_mask)?;
        vmwrite(VmcsField::CTRL_CR4_GUEST_HOST_MASK, cr4_mask)?;
        vmwrite(VmcsField::CTRL_CR0_READ_SHADOW, arch::read_cr0())?;
        vmwrite(VmcsField::CTRL_CR4_READ_SHADOW, arch::read_cr4() & !CR4_VMXE)?;

        if let Some(cr3) = params.cr3_target {
            vmwrite(VmcsField::CTRL_CR3_TARGET_COUNT, 1)?;
            vmwrite(VmcsField::CTRL_CR3_TARGET_VALUE0, cr3)?;
        } else {
            vmwrite(VmcsField::CTRL_CR3_TARGET_COUNT, 0)?;
        }

        if let Some(msr_bitmap) = params.msr_bitmap_pa {
            vmwrite(VmcsField::CTRL_MSR_BITMAP_ADDRESS, msr_bitmap)?;
        }
        if let Some(eptp) = params.eptp {
            vmwrite(VmcsField::CTRL_EPT_POINTER, eptp)?;
        }

        if params.eptp.is_some() {
            vmwrite(VmcsField::CTRL_VIRTUAL_PROCESSOR_ID, u64::from(params.vpid))?;
        }

        vmwrite(
            VmcsField::CTRL_VMEXIT_MSR_STORE_COUNT,
            u64::from(VMX_MSR_AUTOSTORE_COUNT),
        )?;
        vmwrite(VmcsField::CTRL_VMEXIT_MSR_STORE_ADDRESS, params.msr_exit_store_pa)?;
        vmwrite(VmcsField::CTRL_VMEXIT_MSR_LOAD_COUNT, 0)?;
        vmwrite(
            VmcsField::CTRL_VMENTRY_MSR_LOAD_COUNT,
            u64::from(VMX_MSR_AUTOLOAD_COUNT),
        )?;
        vmwrite(VmcsField::CTRL_VMENTRY_MSR_LOAD_ADDRESS, params.msr_entry_load_pa)?;

        vmwrite(VmcsField::VMENTRY_INTERRUPTION_INFO, 0)?;
        vmwrite(VmcsField::VMENTRY_EXCEPTION_ERROR_CODE, 0)?;
        vmwrite(VmcsField::VMENTRY_INSTRUCTION_LEN, 0)?;
    }

    Ok(())
}

/// 填充 MSR 自动列表（`hv/hv/vmcs.cpp` 中 `msr_exit_store` / `msr_entry_load`）。
///
/// # Safety
/// `page` 至少 96 字节可写；调用方可置于独立 4K 页首。
pub unsafe fn prepare_msr_auto_lists(page: *mut u8) {
    let store = page.cast::<VmxMsrEntry>();
    unsafe {
        store.write(VmxMsrEntry {
            msr_index: ia32::IA32_TIME_STAMP_COUNTER,
            reserved: 0,
            data: 0,
        });
        store.add(1).write(VmxMsrEntry {
            msr_index: ia32::IA32_PERF_GLOBAL_CTRL,
            reserved: 0,
            data: 0,
        });
        store.add(2).write(VmxMsrEntry {
            msr_index: ia32::IA32_APERF,
            reserved: 0,
            data: 0,
        });
        store.add(3).write(VmxMsrEntry {
            msr_index: ia32::IA32_MPERF,
            reserved: 0,
            data: 0,
        });

        let load = page.add(64).cast::<VmxMsrEntry>();
        load.write(VmxMsrEntry {
            msr_index: ia32::IA32_APERF,
            reserved: 0,
            data: arch::rdmsr(ia32::IA32_APERF),
        });
        load.add(1).write(VmxMsrEntry {
            msr_index: ia32::IA32_MPERF,
            reserved: 0,
            data: arch::rdmsr(ia32::IA32_MPERF),
        });
    }
}

/// 与 `hv/hv/vmcs.cpp::write_vmcs_host_fields` 一致的 host 视图（专用 CR3 / GDT / IDT / TSS）。
#[derive(Clone, Copy, Debug)]
pub struct HostVmcsLayout {
    pub rip: u64,
    pub rsp: u64,
    pub cr3: u64,
    pub gdtr_base: u64,
    pub idtr_base: u64,
    pub tr_base: u64,
    /// 与参考工程一致：`HOST_FS_BASE` 指向当前 `PerCpuState`。
    pub fs_base: u64,
}

/// 复位风格 `IA32_PAT`（与 Linux 默认 / `hv` `write_vmcs_host_fields` 构造一致）。
const HOST_PAT_RESET: u64 = 0x00070406_00070406;

/// 写入 host 字段。对应 `write_vmcs_host_fields`（`host_cs_selector=0x08`, `host_tr_selector=0x10`）。
///
/// # Safety
/// 需要已 `VMPTRLD`。
pub unsafe fn configure_host_state(layout: &HostVmcsLayout) -> Result<(), VmcsAccessError> {
    use crate::host_descriptor::{HOST_CS_SELECTOR, HOST_TR_SELECTOR};

    let mut cr4 = arch::read_cr4();
    cr4 |= 1 << 16;
    cr4 |= 1 << 18;
    cr4 &= !(1 << 20);
    cr4 &= !(1 << 21);
    unsafe {
        vmwrite(VmcsField::HOST_CR0, arch::read_cr0())?;
        vmwrite(VmcsField::HOST_CR3, layout.cr3)?;
        vmwrite(VmcsField::HOST_CR4, cr4)?;
        vmwrite(VmcsField::HOST_RIP, layout.rip)?;
        vmwrite(VmcsField::HOST_RSP, layout.rsp)?;
        vmwrite(VmcsField::HOST_GDTR_BASE, layout.gdtr_base)?;
        vmwrite(VmcsField::HOST_IDTR_BASE, layout.idtr_base)?;

        vmwrite(VmcsField::HOST_ES_SELECTOR, 0)?;
        vmwrite(VmcsField::HOST_CS_SELECTOR, u64::from(HOST_CS_SELECTOR))?;
        vmwrite(VmcsField::HOST_SS_SELECTOR, 0)?;
        vmwrite(VmcsField::HOST_DS_SELECTOR, 0)?;
        vmwrite(VmcsField::HOST_FS_SELECTOR, 0)?;
        vmwrite(VmcsField::HOST_GS_SELECTOR, 0)?;
        vmwrite(VmcsField::HOST_TR_SELECTOR, u64::from(HOST_TR_SELECTOR))?;

        vmwrite(VmcsField::HOST_FS_BASE, layout.fs_base)?;
        vmwrite(VmcsField::HOST_GS_BASE, 0)?;
        vmwrite(VmcsField::HOST_TR_BASE, layout.tr_base)?;

        vmwrite(VmcsField::HOST_IA32_SYSENTER_CS, 0)?;
        vmwrite(VmcsField::HOST_IA32_SYSENTER_ESP, 0)?;
        vmwrite(VmcsField::HOST_IA32_SYSENTER_EIP, 0)?;

        vmwrite(VmcsField::HOST_PAT, HOST_PAT_RESET)?;
        vmwrite(VmcsField::HOST_IA32_PERF_GLOBAL_CTRL, 0)?;
    }
    Ok(())
}

/// 写入最小 guest 字段。
///
/// `GUEST_RIP`/`GUEST_RSP` 先置 0，由 `hv_vm_launch` 在 `VMLAUNCH` 前写入（与 `hv/vmcs.cpp` + `vm-launch.asm` 一致）。
///
/// # Safety
/// 需要已 `VMPTRLD`。
pub unsafe fn configure_guest_state() -> Result<(), VmcsAccessError> {
    unsafe {
        vmwrite(VmcsField::GUEST_CR0, arch::read_cr0())?;
        vmwrite(VmcsField::GUEST_CR3, arch::read_cr3())?;
        vmwrite(VmcsField::GUEST_CR4, arch::read_cr4())?;
        vmwrite(VmcsField::GUEST_DR7, arch::read_dr7())?;
        vmwrite(VmcsField::GUEST_RSP, 0)?;
        vmwrite(VmcsField::GUEST_RIP, 0)?;
        vmwrite(VmcsField::GUEST_RFLAGS, arch::read_rflags())?;
        vmwrite(
            VmcsField::GUEST_IA32_PERF_GLOBAL_CTRL,
            arch::rdmsr(ia32::IA32_PERF_GLOBAL_CTRL),
        )?;
        vmwrite(VmcsField::GUEST_VMCS_LINK_POINTER, ia32::MAXULONG64)?;
        vmwrite(VmcsField::GUEST_ACTIVITY_STATE, 0)?;
        vmwrite(VmcsField::GUEST_INTERRUPTIBILITY_STATE, 0)?;
        vmwrite(VmcsField::GUEST_PENDING_DEBUG_EXCEPTIONS, 0)?;
        vmwrite(VmcsField::GUEST_VMX_PREEMPTION_TIMER, ia32::MAXULONG64)?;
    }
    Ok(())
}

/// 读取当前 VMCS 的 guest RIP/RSP/RFLAGS 快照。
///
/// # Safety
/// 调用前必须已 `VMPTRLD` 当前 VMCS，且运行在 VMX root。
pub unsafe fn read_guest_state_snapshot() -> Result<GuestStateSnapshot, VmcsAccessError> {
    let rip = unsafe { vmread(VmcsField::GUEST_RIP)? };
    let rsp = unsafe { vmread(VmcsField::GUEST_RSP)? };
    let rflags = unsafe { vmread(VmcsField::GUEST_RFLAGS)? };
    Ok(GuestStateSnapshot { rip, rsp, rflags })
}

/// 读取并解码 `VMCS_EXIT_REASON`。
///
/// # Safety
/// 调用前必须已 `VMPTRLD` 当前 VMCS，且运行在 VMX root。
pub unsafe fn read_exit_reason() -> Result<VmExitReason, VmcsAccessError> {
    let raw = unsafe { vmread(VmcsField::EXIT_REASON)? } as u32;
    Ok(VmExitReason::from_raw(raw))
}

/// 将 IA32_VMX_BASIC 中的 VMCS revision 写入区域首 dword，其余清零。
pub unsafe fn prepare_vmcs_region(page: *mut u8) {
    let basic = unsafe { arch::rdmsr(ia32::IA32_VMX_BASIC) };
    let revision = (basic & 0x7FFF_FFFF) as u32;
    unsafe {
        core::ptr::write_bytes(page, 0, 4096);
        page.cast::<u32>().write_unaligned(revision);
    }
}

/// 与 `prepare_vmcs_region` 相同布局要求；VMXON 区域也使用 revision id。
pub unsafe fn prepare_vmxon_region(page: *mut u8) {
    unsafe { prepare_vmcs_region(page) };
}
