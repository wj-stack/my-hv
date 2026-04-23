//! Shared contract for the my-hv VT-x–style driver and user-mode client.
//! Must stay `no_std` so the kernel driver can depend on this crate.
//!
//! IOCTL layout follows the historical template; hypercalls mirror `hv/um/hv.h` and `hv/hv/hypercalls.h`.
#![no_std]

/// Keep in sync with `specs/001-refactor-vtx-driver/contracts/driver-interface.md` and the driver.
pub const CONTRACT_VERSION: &str = "0.1.0";

/// 56-bit key in the hypercall RAX pack (low 8 = code, upper 56 = key). See `hypercalls.cpp`.
pub const HYPERCALL_KEY: u64 = 69_420;

// --- IOCTL basics (align with `CTL_CODE` macro) ---

pub const FILE_DEVICE_UNKNOWN: u32 = 0x0000_0022;
pub const FILE_ANY_ACCESS: u32 = 0;
pub const METHOD_BUFFERED: u32 = 0;

/// `CTL_CODE` equivalent.
pub const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

/// Template ping (returns `PING_RESPONSE_U32` in 4 output bytes).
pub const IOCTL_PING: u32 = ctl_code(
    FILE_DEVICE_UNKNOWN,
    0x900,
    METHOD_BUFFERED,
    FILE_ANY_ACCESS,
);

pub const IOCTL_ECHO: u32 = ctl_code(
    FILE_DEVICE_UNKNOWN,
    0x901,
    METHOD_BUFFERED,
    FILE_ANY_ACCESS,
);

/// User-mode and kernel dispatch the same `HvHypercallIn` / `HvHypercallOut` over buffered IOCTL.
pub const IOCTL_HV_HYPERCALL: u32 = ctl_code(
    FILE_DEVICE_UNKNOWN,
    0x902,
    METHOD_BUFFERED,
    FILE_ANY_ACCESS,
);

/// 进入 VMX root（每逻辑处理器 VMXON + VMCLEAR/VMPTRLD）。输入/输出可为空；可选输出 `u32` 状态码。
pub const IOCTL_HV_START: u32 = ctl_code(
    FILE_DEVICE_UNKNOWN,
    0x903,
    METHOD_BUFFERED,
    FILE_ANY_ACCESS,
);

/// 离开 VMX 操作（VMCLEAR + VMXOFF），释放每 CPU 资源。
pub const IOCTL_HV_STOP: u32 = ctl_code(
    FILE_DEVICE_UNKNOWN,
    0x904,
    METHOD_BUFFERED,
    FILE_ANY_ACCESS,
);

pub const ECHO_MAX_LEN: usize = 1024;

pub const PING_RESPONSE_U32: u32 = 0x0047_4E50; // "PNG\0" LE (template marker)

/// MSVC `hv::hypervisor_signature = 'fr0g'`：多字符常量按首字符在最高字节（`0x66723067`），与 `hv/um/hv.h` 一致。
pub const HYPERVISOR_SIGNATURE: u64 = 0x6672_3067;

/// Device & symlink basenames: `\\Device\\{B}` and `\\DosDevices\\{B}` and `\\.\{B}`.
pub const DEVICE_BASENAME: &str = "MyHvTpl";
pub const USER_DEVICE_PATH: &str = r"\\.\MyHvTpl";

// --- Hypercall opcodes: order must match the legacy C++ `hv::hypercall_code` table ---

/// Hypercall operation code (low 8 bits of the packed RAX / `HvHypercallIn.rax` low byte).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HypercallCode {
    Ping = 0,
    Test = 1,
    Unload = 2,
    ReadPhysMem = 3,
    WritePhysMem = 4,
    ReadVirtMem = 5,
    WriteVirtMem = 6,
    QueryProcessCr3 = 7,
    InstallEptHook = 8,
    RemoveEptHook = 9,
    FlushLogs = 10,
    GetPhysicalAddress = 11,
    HidePhysicalPage = 12,
    UnhidePhysicalPage = 13,
    GetHvBase = 14,
    InstallMmr = 15,
    RemoveMmr = 16,
    RemoveAllMmrs = 17,
}

impl TryFrom<u8> for HypercallCode {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        let code = match value {
            0 => Self::Ping,
            1 => Self::Test,
            2 => Self::Unload,
            3 => Self::ReadPhysMem,
            4 => Self::WritePhysMem,
            5 => Self::ReadVirtMem,
            6 => Self::WriteVirtMem,
            7 => Self::QueryProcessCr3,
            8 => Self::InstallEptHook,
            9 => Self::RemoveEptHook,
            10 => Self::FlushLogs,
            11 => Self::GetPhysicalAddress,
            12 => Self::HidePhysicalPage,
            13 => Self::UnhidePhysicalPage,
            14 => Self::GetHvBase,
            15 => Self::InstallMmr,
            16 => Self::RemoveMmr,
            17 => Self::RemoveAllMmrs,
            _ => return Err(()),
        };
        Ok(code)
    }
}

/// `hv::hypercall_input` (packed RAX) — 64-bit RAX in ioctl buffer is enough for code+key.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct HvHypercallIn {
    /// Full guest RAX image: `code` in low byte, `key` in the upper 56 bits.
    pub rax: u64,
    /// Guest arguments `rcx, rdx, r8, r9, r10, r11` as in the old union `args[6]`.
    pub args: [u64; 6],
}

/// Buffered output from a hypercall. `rax` is the result (bytes read, signature, error detail, etc.).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct HvHypercallOut {
    pub status: u32,
    pub rax: u64,
    pub _reserved: u64,
}

/// `STATUS_SUCCESS` in shared layer (matches Windows success).
pub const STATUS_SUCCESS: u32 = 0x0000_0000;
/// `STATUS_INVALID_PARAMETER` — used for key/code validation at the ioctl bridge.
pub const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
/// `STATUS_NOT_SUPPORTED` for operations not yet implemented in the Rust driver.
pub const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0022;
