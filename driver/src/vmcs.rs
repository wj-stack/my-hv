//! VMCS 区域初始化与访问封装。对应 `hv/hv/vmcs.h`、`hv/hv/vmcs.cpp`。

use crate::{arch, ia32};
use crate::{gdt, idt};

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
    pub const GUEST_ACTIVITY_STATE: Self = Self(ia32::VMCS_GUEST_ACTIVITY_STATE);

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
    fn from_raw(raw: u32) -> Self {
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

/// 写入最小 guest/control 字段集合，为后续 VM-entry/VM-exit 完整编排做铺垫。
///
/// 该函数遵循“先镜像当前主机关键状态，再由后续阶段覆盖细节字段”的策略。
///
/// # Safety
/// 调用前必须已 `VMPTRLD` 当前 VMCS，且运行在 VMX root。
pub unsafe fn seed_minimal_guest_state() -> Result<(), VmcsAccessError> {
    unsafe { vmwrite(VmcsField::GUEST_CR3, arch::read_cr3())? };
    unsafe { vmwrite(VmcsField::GUEST_RIP, 0)? };
    unsafe { vmwrite(VmcsField::GUEST_RSP, 0)? };
    unsafe { vmwrite(VmcsField::GUEST_RFLAGS, 0x2)? };
    unsafe { vmwrite(VmcsField::CTRL_CR0_READ_SHADOW, arch::read_cr0())? };
    unsafe { vmwrite(VmcsField::CTRL_CR4_READ_SHADOW, arch::read_cr4() & !CR4_VMXE)? };
    Ok(())
}

fn control_msr(use_true: u64, true_msr: u32, fallback: u32) -> u32 {
    ia32::vmx_true_controls_msr_or_default(use_true, true_msr, fallback)
}

/// 根据 VMX capability MSR 约束计算控制值并写入 VMCS。
///
/// # Safety
/// 需要已 `VMPTRLD`。
pub unsafe fn configure_control_fields(maybe_msr_bitmap_pa: Option<u64>, maybe_eptp: Option<u64>) -> Result<(), VmcsAccessError> {
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

    let pin = ia32::adjust_vmx_control(ia32::PIN_BASED_NMI_EXITING, unsafe { arch::rdmsr(pin_msr) });
    let primary_requested =
        ia32::CPU_BASED_USE_MSR_BITMAPS | ia32::CPU_BASED_ACTIVATE_SECONDARY_CONTROLS;
    let primary = ia32::adjust_vmx_control(primary_requested, unsafe { arch::rdmsr(proc_msr) });
    let exit_ctrl = ia32::adjust_vmx_control(
        ia32::EXIT_CONTROL_HOST_ADDR_SPACE_SIZE
            | ia32::EXIT_CONTROL_SAVE_IA32_PAT
            | ia32::EXIT_CONTROL_LOAD_IA32_PAT,
        unsafe { arch::rdmsr(exit_msr) },
    );
    let entry_ctrl = ia32::adjust_vmx_control(
        ia32::ENTRY_CONTROL_IA32E_MODE_GUEST | ia32::ENTRY_CONTROL_LOAD_IA32_PAT,
        unsafe { arch::rdmsr(entry_msr) },
    );
    let mut secondary_req = ia32::SECONDARY_CONTROL_ENABLE_RDTSCP
        | ia32::SECONDARY_CONTROL_ENABLE_INVPCID
        | ia32::SECONDARY_CONTROL_ENABLE_XSAVES_XRSTORS;
    if maybe_eptp.is_some() {
        secondary_req |= ia32::SECONDARY_CONTROL_ENABLE_EPT;
    }
    let secondary = ia32::adjust_vmx_control(
        secondary_req,
        unsafe { arch::rdmsr(ia32::IA32_VMX_PROCBASED_CTLS2) },
    );

    unsafe {
        vmwrite(VmcsField::CTRL_PIN_BASED, pin as u64)?;
        vmwrite(VmcsField::CTRL_CPU_BASED, primary as u64)?;
        vmwrite(VmcsField::CTRL_SECONDARY_CPU_BASED, secondary as u64)?;
        vmwrite(VmcsField::CTRL_VMEXIT_CONTROLS, exit_ctrl as u64)?;
        vmwrite(VmcsField::CTRL_VMENTRY_CONTROLS, entry_ctrl as u64)?;
        vmwrite(VmcsField::CTRL_EXCEPTION_BITMAP, 0)?;
        vmwrite(VmcsField::CTRL_CR3_TARGET_COUNT, 0)?;
        vmwrite(VmcsField::CTRL_PAGEFAULT_ERROR_CODE_MASK, 0)?;
        vmwrite(VmcsField::CTRL_PAGEFAULT_ERROR_CODE_MATCH, 0)?;
        vmwrite(VmcsField::CTRL_TSC_OFFSET, 0)?;
        vmwrite(VmcsField::CTRL_VMEXIT_MSR_STORE_COUNT, 0)?;
        vmwrite(VmcsField::CTRL_VMEXIT_MSR_LOAD_COUNT, 0)?;
        vmwrite(VmcsField::CTRL_VMENTRY_MSR_LOAD_COUNT, 0)?;
    }

    if let Some(msr_bitmap) = maybe_msr_bitmap_pa {
        unsafe { vmwrite(VmcsField::CTRL_MSR_BITMAP_ADDRESS, msr_bitmap)? };
    }
    if let Some(eptp) = maybe_eptp {
        unsafe { vmwrite(VmcsField::CTRL_EPT_POINTER, eptp)? };
    }

    Ok(())
}

/// 写入最小 host 字段（仅用于后续 VM-entry 路径打底）。
///
/// # Safety
/// 需要已 `VMPTRLD`。
pub unsafe fn configure_host_state(host_rip: u64, host_rsp: u64) -> Result<(), VmcsAccessError> {
    let gdtr = gdt::read_gdtr();
    let idtr = idt::read_idtr();
    // SAFETY: GDTR/IDTR 是 packed 结构，使用 read_unaligned 读取字段。
    let gdtr_base = unsafe { core::ptr::addr_of!(gdtr.base).read_unaligned() };
    let idtr_base = unsafe { core::ptr::addr_of!(idtr.base).read_unaligned() };
    unsafe {
        vmwrite(VmcsField::HOST_CR0, arch::read_cr0())?;
        vmwrite(VmcsField::HOST_CR3, arch::read_cr3())?;
        vmwrite(VmcsField::HOST_CR4, arch::read_cr4())?;
        vmwrite(VmcsField::HOST_RIP, host_rip)?;
        vmwrite(VmcsField::HOST_RSP, host_rsp)?;
        vmwrite(VmcsField::HOST_GDTR_BASE, gdtr_base)?;
        vmwrite(VmcsField::HOST_IDTR_BASE, idtr_base)?;
    }
    Ok(())
}

/// 写入最小 guest 字段。
///
/// # Safety
/// 需要已 `VMPTRLD`。
pub unsafe fn configure_guest_state() -> Result<(), VmcsAccessError> {
    unsafe {
        vmwrite(VmcsField::GUEST_CR0, arch::read_cr0())?;
        vmwrite(VmcsField::GUEST_CR3, arch::read_cr3())?;
        vmwrite(VmcsField::GUEST_CR4, arch::read_cr4())?;
        vmwrite(VmcsField::GUEST_DR7, 0x400)?;
        vmwrite(VmcsField::GUEST_VMCS_LINK_POINTER, ia32::MAXULONG64)?;
        vmwrite(VmcsField::GUEST_ACTIVITY_STATE, 0)?;
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
