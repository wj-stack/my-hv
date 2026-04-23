//! IA-32 / Intel64 constants used by the hypervisor. Names mirror `hv/extern/ia32-doc` when possible.
//! (See also `vmcs_field` for VMCS encodings and `ia32` defines in the reference C++ project.)
#![allow(dead_code)]

// --- Key MSRs ---

pub const IA32_FEATURE_CONTROL: u32 = 0x0000_00_3A;
pub const IA32_VMX_BASIC: u32 = 0x0000_00_48;
pub const IA32_VMX_PINBASED_CTLS: u32 = 0x0000_00_48 + 1;
pub const IA32_VMX_PROCBASED_CTLS: u32 = 0x0000_00_48 + 2;
pub const IA32_VMX_EXIT_CTLS: u32 = 0x0000_00_48 + 3;
pub const IA32_VMX_ENTRY_CTLS: u32 = 0x0000_00_48 + 4;
pub const IA32_VMX_PROCBASED_CTLS2: u32 = 0x0000_00_48 + 5;
pub const IA32_VMX_TRUE_PINBASED_CTLS: u32 = 0x0000_00_48 + 6;
pub const IA32_VMX_TRUE_PROCBASED_CTLS: u32 = 0x0000_00_48 + 7;
pub const IA32_VMX_TRUE_EXIT_CTLS: u32 = 0x0000_00_48 + 8;
pub const IA32_VMX_TRUE_ENTRY_CTLS: u32 = 0x0000_00_48 + 9;
pub const IA32_VMX_CR0_FIXED0: u32 = 0x0000_00_48 + 0x0A;
pub const IA32_VMX_CR0_FIXED1: u32 = 0x0000_00_48 + 0x0B;
pub const IA32_VMX_CR4_FIXED0: u32 = 0x0000_00_48 + 0x0C;
pub const IA32_VMX_CR4_FIXED1: u32 = 0x0000_00_48 + 0x0D;
pub const IA32_VMX_EPT_VPID_CAP: u32 = 0x0000_00_48 + 0x10;

/// VMX basic exit: VMCALL (18).
pub const VMX_EXIT_REASON_EXECUTE_VMCALL: u32 = 18;
pub const VMX_EXIT_REASON_CPUID: u32 = 10;
pub const VMX_EXIT_REASON_RDMSR: u32 = 31;
pub const VMX_EXIT_REASON_WRMSR: u32 = 32;
pub const VMX_EXIT_REASON_MOV_CR: u32 = 28;
pub const VMX_EXIT_REASON_EXCEPTION_NMI: u32 = 0;
pub const VMX_EXIT_REASON_EPT_VIOLATION: u32 = 48;
pub const VMX_EXIT_REASON_PREEMPTION_TIMER: u32 = 52;
pub const MAXULONG64: u64 = 0xFFFF_FFFF_FFFF_FFFF;

// --- A few VMCS field full-width encodings (16-bit) used by bring-up. ---

pub const VMCS_EXIT_REASON: u32 = 0x0000_4402;
pub const VMCS_VM_INSTRUCTION_ERROR: u32 = 0x0000_4400;
pub const VMCS_GUEST_RIP: u32 = 0x0000_681E;
pub const VMCS_GUEST_RSP: u32 = 0x0000_681C;
pub const VMCS_GUEST_RFLAGS: u32 = 0x0000_6820;
pub const VMCS_CTRL_EPT_POINTER: u32 = 0x0000_201A;
pub const VMCS_GUEST_CR3: u32 = 0x0000_6802;
pub const VMCS_CTRL_CR0_READ_SHADOW: u32 = 0x0000_6004;
pub const VMCS_CTRL_CR4_READ_SHADOW: u32 = 0x0000_6006;
pub const VMCS_CTRL_PIN_BASED: u32 = 0x0000_4000;
pub const VMCS_CTRL_CPU_BASED: u32 = 0x0000_4002;
pub const VMCS_CTRL_EXCEPTION_BITMAP: u32 = 0x0000_4004;
pub const VMCS_CTRL_VMEXIT_CONTROLS: u32 = 0x0000_400C;
pub const VMCS_CTRL_VMENTRY_CONTROLS: u32 = 0x0000_4012;
pub const VMCS_CTRL_SECONDARY_CPU_BASED: u32 = 0x0000_401E;
pub const VMCS_CTRL_CR3_TARGET_COUNT: u32 = 0x0000_400A;
pub const VMCS_CTRL_MSR_BITMAP_ADDRESS: u32 = 0x0000_2004;
pub const VMCS_CTRL_VMEXIT_MSR_STORE_COUNT: u32 = 0x0000_400E;
pub const VMCS_CTRL_VMEXIT_MSR_LOAD_COUNT: u32 = 0x0000_4010;
pub const VMCS_CTRL_VMENTRY_MSR_LOAD_COUNT: u32 = 0x0000_4014;
pub const VMCS_CTRL_PAGEFAULT_ERROR_CODE_MASK: u32 = 0x0000_4006;
pub const VMCS_CTRL_PAGEFAULT_ERROR_CODE_MATCH: u32 = 0x0000_4008;
pub const VMCS_CTRL_TSC_OFFSET: u32 = 0x0000_2010;
pub const VMCS_HOST_CR0: u32 = 0x0000_6C00;
pub const VMCS_HOST_CR3: u32 = 0x0000_6C02;
pub const VMCS_HOST_CR4: u32 = 0x0000_6C04;
pub const VMCS_HOST_RSP: u32 = 0x0000_6C14;
pub const VMCS_HOST_RIP: u32 = 0x0000_6C16;
pub const VMCS_HOST_GDTR_BASE: u32 = 0x0000_6C0C;
pub const VMCS_HOST_IDTR_BASE: u32 = 0x0000_6C0E;
pub const VMCS_GUEST_CR0: u32 = 0x0000_6800;
pub const VMCS_GUEST_CR4: u32 = 0x0000_6804;
pub const VMCS_GUEST_DR7: u32 = 0x0000_681A;
pub const VMCS_GUEST_VMCS_LINK_POINTER: u32 = 0x0000_2800;
pub const VMCS_GUEST_ACTIVITY_STATE: u32 = 0x0000_4826;

pub const VMX_BASIC_TRUE_CTLS: u64 = 1 << 55;

pub const PIN_BASED_NMI_EXITING: u32 = 1 << 3;
pub const CPU_BASED_USE_MSR_BITMAPS: u32 = 1 << 28;
pub const CPU_BASED_ACTIVATE_SECONDARY_CONTROLS: u32 = 1 << 31;
pub const CPU_BASED_CR3_LOAD_EXITING: u32 = 1 << 15;
pub const CPU_BASED_CR3_STORE_EXITING: u32 = 1 << 16;
pub const EXIT_CONTROL_HOST_ADDR_SPACE_SIZE: u32 = 1 << 9;
pub const EXIT_CONTROL_SAVE_IA32_PAT: u32 = 1 << 18;
pub const EXIT_CONTROL_LOAD_IA32_PAT: u32 = 1 << 19;
pub const ENTRY_CONTROL_IA32E_MODE_GUEST: u32 = 1 << 9;
pub const ENTRY_CONTROL_LOAD_IA32_PAT: u32 = 1 << 14;
pub const SECONDARY_CONTROL_ENABLE_EPT: u32 = 1 << 1;
pub const SECONDARY_CONTROL_ENABLE_RDTSCP: u32 = 1 << 3;
pub const SECONDARY_CONTROL_ENABLE_INVPCID: u32 = 1 << 12;
pub const SECONDARY_CONTROL_ENABLE_XSAVES_XRSTORS: u32 = 1 << 20;

#[inline]
pub const fn vmx_true_controls_msr_or_default(basic: u64, true_msr: u32, legacy_msr: u32) -> u32 {
    if (basic & VMX_BASIC_TRUE_CTLS) != 0 {
        true_msr
    } else {
        legacy_msr
    }
}

#[inline]
pub const fn adjust_vmx_control(requested: u32, msr_value: u64) -> u32 {
    let allowed0 = msr_value as u32;
    let allowed1 = (msr_value >> 32) as u32;
    (requested | allowed0) & allowed1
}
