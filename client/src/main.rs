//! User-mode CLI: open device and run ping / echo IOCTLs (no domain logic).

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use shared_contract::{ECHO_MAX_LEN, IOCTL_ECHO, IOCTL_PING, USER_DEVICE_PATH};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;

fn to_wide(path: &str) -> Vec<u16> {
    OsStr::new(path).encode_wide().chain(Some(0)).collect()
}

fn open_device(path: &str) -> anyhow::Result<HANDLE> {
    let wide = to_wide(path);
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .with_context(|| format!("CreateFileW failed for {path}"))?;
    Ok(handle)
}

#[derive(Parser)]
#[command(name = "my-hv-client", version, about = "IOCTL client for the minimal NT driver template")]
struct Cli {
    /// Device path for CreateFileW (e.g. \\.\YourDevice)
    #[arg(long, short = 'd')]
    device: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Send IOCTL_PING and print the returned u32
    Ping,
    /// Send IOCTL_ECHO with the given payload (UTF-8 bytes, max ECHO_MAX_LEN)
    Echo {
        #[arg(short, long, default_value = "echo-from-client")]
        message: String,
    },
    /// Run ping then echo (smoke test)
    Smoke,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let device = cli
        .device
        .as_deref()
        .unwrap_or(USER_DEVICE_PATH);

    println!("contract version: {}", shared_contract::CONTRACT_VERSION);
    println!("opening: {device}");

    let h = open_device(device)?;
    match cli.command.unwrap_or(Commands::Smoke) {
        Commands::Ping => ping(&h)?,
        Commands::Echo { message } => echo(&h, message.as_bytes())?,
        Commands::Smoke => {
            ping(&h)?;
            echo(&h, b"echo-from-client")?;
        }
    }
    unsafe {
        CloseHandle(h)?;
    }
    Ok(())
}

fn ping(h: &HANDLE) -> anyhow::Result<()> {
    let mut out = [0u8; 4];
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            *h,
            IOCTL_PING,
            None,
            0,
            Some(out.as_mut_ptr().cast::<c_void>()),
            4,
            Some(std::ptr::from_mut(&mut returned)),
            None,
        )?;
    }
    if returned != 4 {
        bail!("IOCTL_PING expected 4 bytes, got {returned}");
    }
    let v = u32::from_le_bytes(out);
    let exp = shared_contract::PING_RESPONSE_U32;
    println!("ping ok: output u32 = {v:#010x} (expect {exp:#010x})");
    Ok(())
}

fn echo(h: &HANDLE, msg: &[u8]) -> anyhow::Result<()> {
    if msg.len() > ECHO_MAX_LEN {
        bail!("message length {} exceeds ECHO_MAX_LEN ({ECHO_MAX_LEN})", msg.len());
    }
    let mut buffer = [0u8; ECHO_MAX_LEN];
    buffer[..msg.len()].copy_from_slice(msg);
    let mut returned = 0u32;
    let in_len = msg.len() as u32;
    unsafe {
        DeviceIoControl(
            *h,
            IOCTL_ECHO,
            Some(buffer.as_ptr().cast::<c_void>()),
            in_len,
            Some(buffer.as_mut_ptr().cast::<c_void>()),
            ECHO_MAX_LEN as u32,
            Some(std::ptr::from_mut(&mut returned)),
            None,
        )?;
    }
    let n = returned as usize;
    if n != msg.len() {
        bail!("IOCTL_ECHO length mismatch");
    }
    println!("echo ok: {:?}", std::str::from_utf8(&buffer[..n]));
    Ok(())
}
