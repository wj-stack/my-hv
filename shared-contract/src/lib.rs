//! Shared IOCTL codes and buffer limits for the minimal driver/client template.
//! Must stay `no_std` so the kernel driver can depend on this crate.
//!
//! See `specs/004-nt-driver-template/contracts/README.md` for semantics.

#![no_std]

/// Logical contract version (bump when IOCTL shapes change).
pub const CONTRACT_VERSION: &str = "0.1.0";

pub const FILE_DEVICE_UNKNOWN: u32 = 0x0000_0022;
pub const FILE_ANY_ACCESS: u32 = 0;
pub const METHOD_BUFFERED: u32 = 0;

/// `CTL_CODE` equivalent (matches Windows `CTL_CODE` macro layout).
pub const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

/// Ping: minimal buffer; driver returns 4-byte magic in output buffer.
pub const IOCTL_PING: u32 = ctl_code(
    FILE_DEVICE_UNKNOWN,
    0x900,
    METHOD_BUFFERED,
    FILE_ANY_ACCESS,
);

/// Echo: copy up to `ECHO_MAX_LEN` bytes input to output (METHOD_BUFFERED).
pub const IOCTL_ECHO: u32 = ctl_code(
    FILE_DEVICE_UNKNOWN,
    0x901,
    METHOD_BUFFERED,
    FILE_ANY_ACCESS,
);

pub const ECHO_MAX_LEN: usize = 1024;

/// Fixed response for ping (ASCII "PNG\x00" style marker as LE u32).
pub const PING_RESPONSE_U32: u32 = 0x0047_4E50; // 'PNG\0' little-endian-ish marker

/// Default basename for `\\Device\\{basename}` / `\\DosDevices\\{basename}` / `\\.\{basename}`.
pub const DEVICE_BASENAME: &str = "MyHvTpl";

/// User-mode path (UTF-8) — pass to `CreateFileW` after UTF-16 conversion.
pub const USER_DEVICE_PATH: &str = r"\\.\MyHvTpl";
