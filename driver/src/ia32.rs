//! IA-32 / Intel64 constants used by the hypervisor. Names mirror `hv/extern/ia32-doc` when possible.
//! (See also `vmcs_field` for VMCS encodings and `ia32` defines in the reference C++ project.)

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
