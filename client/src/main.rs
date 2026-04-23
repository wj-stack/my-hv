//! 用户态 CLI：打开设备并发送 IOCTL（ping/echo + HV START/STOP + 超级调用）。

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use shared_contract::{
    ECHO_MAX_LEN, HypercallCode, HvHypercallIn, HvHypercallOut, IOCTL_ECHO, IOCTL_HV_HYPERCALL,
    IOCTL_HV_START, IOCTL_HV_STOP, IOCTL_PING, HYPERCALL_KEY, USER_DEVICE_PATH,
};
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

fn hypercall_rax(code: HypercallCode) -> u64 {
    (HYPERCALL_KEY << 8) | (code as u8 as u64)
}

#[derive(Parser)]
#[command(name = "my-hv-client", version, about = "IOCTL client for my-hv VT-x driver")]
struct Cli {
    #[arg(long, short = 'd')]
    device: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 模板 IOCTL_PING（返回 u32）
    Ping,
    /// IOCTL_ECHO
    Echo {
        #[arg(short, long, default_value = "echo-from-client")]
        message: String,
    },
    /// 启动 VMX root（IOCTL_HV_START）
    Start,
    /// 停止 VMX root（IOCTL_HV_STOP）
    Stop,
    /// 超级调用 PING（需先 Start）
    HvPing,
    /// 超级调用 UNLOAD（等价于 Stop + 会话清理）
    HvUnload,
    /// 查询 HV 映像基址（超级调用 GET_HV_BASE）
    HvBase,
    /// 刷新日志占位（超级调用 FLUSH_LOGS）
    HvFlushLogs,
    Smoke,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let device = cli.device.as_deref().unwrap_or(USER_DEVICE_PATH);

    println!("contract version: {}", shared_contract::CONTRACT_VERSION);
    println!("opening: {device}");

    let h = open_device(device)?;
    match cli.command.unwrap_or(Commands::Smoke) {
        Commands::Ping => ping(&h)?,
        Commands::Echo { message } => echo(&h, message.as_bytes())?,
        Commands::Start => hv_start(&h)?,
        Commands::Stop => hv_stop(&h)?,
        Commands::HvPing => hv_hypercall(&h, HypercallCode::Ping, [0; 6])?,
        Commands::HvUnload => hv_hypercall(&h, HypercallCode::Unload, [0; 6])?,
        Commands::HvBase => hv_hypercall(&h, HypercallCode::GetHvBase, [0; 6])?,
        Commands::HvFlushLogs => hv_hypercall(&h, HypercallCode::FlushLogs, [0; 6])?,
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

fn hv_start(h: &HANDLE) -> anyhow::Result<()> {
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            *h,
            IOCTL_HV_START,
            None,
            0,
            None,
            0,
            Some(std::ptr::from_mut(&mut returned)),
            None,
        )?;
    }
    println!("HV START ok (returned bytes={returned})");
    Ok(())
}

fn hv_stop(h: &HANDLE) -> anyhow::Result<()> {
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            *h,
            IOCTL_HV_STOP,
            None,
            0,
            None,
            0,
            Some(std::ptr::from_mut(&mut returned)),
            None,
        )?;
    }
    println!("HV STOP ok (returned bytes={returned})");
    Ok(())
}

fn hv_hypercall(h: &HANDLE, code: HypercallCode, args: [u64; 6]) -> anyhow::Result<()> {
    let inp = HvHypercallIn {
        rax: hypercall_rax(code),
        args,
    };
    let in_bytes = core::mem::size_of::<HvHypercallIn>() as u32;
    let out_bytes = core::mem::size_of::<HvHypercallOut>() as u32;
    let buf_len = (in_bytes as usize).max(out_bytes as usize).max(64);
    let mut buf = vec![0u8; buf_len];
    unsafe {
        core::ptr::copy_nonoverlapping(
            (&raw const inp).cast::<u8>(),
            buf.as_mut_ptr(),
            in_bytes as usize,
        );
    }
    let mut returned = 0u32;
    unsafe {
        DeviceIoControl(
            *h,
            IOCTL_HV_HYPERCALL,
            Some(buf.as_ptr().cast::<c_void>()),
            in_bytes,
            Some(buf.as_mut_ptr().cast::<c_void>()),
            out_bytes,
            Some(std::ptr::from_mut(&mut returned)),
            None,
        )?;
    }
    let out = unsafe { buf.as_ptr().cast::<HvHypercallOut>().read_unaligned() };
    println!(
        "hypercall {:?}: ioctl ok, out.status={:#x}, out.rax={:#x}, bytes={returned}",
        code, out.status, out.rax
    );
    Ok(())
}
