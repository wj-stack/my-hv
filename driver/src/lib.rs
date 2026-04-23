//! WDM 驱动：设备对象、符号链接、IOCTL（模板 ping/echo + HV STOP/超级调用；与 C++ `hv` 一致在 DriverEntry 自动 START）。
//! 设备名字符串必须与 `shared_contract::DEVICE_BASENAME` 一致。

#![no_std]
extern crate alloc;

#[cfg(not(test))]
extern crate wdk_panic;

use alloc::string::String;
use core::cell::UnsafeCell;
use core::mem::size_of;

#[cfg(not(test))]
use wdk_alloc::WdkAllocator;
use wdk::println;
use wdk_sys::ntddk::{
    KeAcquireGuardedMutex, KeInitializeGuardedMutex, KeReleaseGuardedMutex,
};
use wdk_sys::{
    CCHAR, DEVICE_OBJECT, DRIVER_OBJECT, IO_NO_INCREMENT, IRP, KGUARDED_MUTEX, NTSTATUS,
    PCUNICODE_STRING, STATUS_ALREADY_INITIALIZED, STATUS_BUFFER_TOO_SMALL, STATUS_HV_OPERATION_FAILED,
    STATUS_INVALID_DEVICE_REQUEST, STATUS_INVALID_PARAMETER, STATUS_SUCCESS, STATUS_UNSUCCESSFUL,
    UNICODE_STRING,
};

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOCATOR: WdkAllocator = WdkAllocator;

mod arch;
mod ept;
mod exception_inject;
mod guest_context;
mod exit_handlers;
mod mtrr;
mod msr_bitmap;
mod gdt;
mod hypercalls;
mod ia32;
mod idt;
mod introspection;
mod logger;
mod mm;
mod segment;
mod timing;
mod vmcs;
mod vmexit;
mod vcpu;
mod vmx;

use shared_contract::{
    DEVICE_BASENAME, ECHO_MAX_LEN, HvHypercallIn, HvHypercallOut, IOCTL_ECHO, IOCTL_HV_HYPERCALL,
    IOCTL_HV_START, IOCTL_HV_STOP, IOCTL_PING, PING_RESPONSE_U32,
};

use crate::vcpu::VmxCluster;

const IRP_MJ_CREATE_INDEX: usize = 0x00;
const IRP_MJ_CLOSE_INDEX: usize = 0x02;
const IRP_MJ_DEVICE_CONTROL_INDEX: usize = 0x0e;

const FILE_DEVICE_UNKNOWN: u32 = 0x0000_0022;
const FILE_DEVICE_SECURE_OPEN: u32 = 0x0000_0100;
const DO_BUFFERED_IO: u32 = 0x0000_0004;

struct SyncSession(UnsafeCell<Option<VmxCluster>>);
// SAFETY: 仅通过 `SESSION_MUTEX` 下的 `with_session` 访问。
unsafe impl Sync for SyncSession {}

static SESSION: SyncSession = SyncSession(UnsafeCell::new(None));
static mut SESSION_MUTEX: KGUARDED_MUTEX = unsafe { core::mem::zeroed() };

fn encode_utf16z(input: &str, out: &mut [u16]) -> Option<usize> {
    let mut idx = 0;
    for code_unit in input.encode_utf16() {
        if idx + 1 >= out.len() {
            return None;
        }
        out[idx] = code_unit;
        idx += 1;
    }
    out[idx] = 0;
    Some(idx + 1)
}

fn to_unicode_string(buffer: &mut [u16], used_with_nul: usize) -> UNICODE_STRING {
    UNICODE_STRING {
        Length: ((used_with_nul - 1) * core::mem::size_of::<u16>()) as u16,
        MaximumLength: (used_with_nul * core::mem::size_of::<u16>()) as u16,
        Buffer: buffer.as_mut_ptr(),
    }
}

unsafe fn complete_request(irp: *mut IRP, status: NTSTATUS, info: usize) -> NTSTATUS {
    unsafe {
        (*irp).IoStatus.__bindgen_anon_1.Status = status;
        (*irp).IoStatus.Information = info as u64;
        wdk_sys::ntddk::IofCompleteRequest(irp, IO_NO_INCREMENT as CCHAR);
    }
    status
}

unsafe fn with_session<R>(f: impl FnOnce(&mut Option<VmxCluster>) -> R) -> R {
    unsafe {
        KeAcquireGuardedMutex(&raw mut SESSION_MUTEX);
        let r = f(&mut *SESSION.0.get());
        KeReleaseGuardedMutex(&raw mut SESSION_MUTEX);
        r
    }
}

/// 与 `hv::main.cpp` 中 `hv::start()` 同序：`install_vmexit_session` 后 `VmxCluster::start()`。
unsafe fn hv_try_start() -> NTSTATUS {
    crate::vmexit::install_vmexit_session(SESSION.0.get());
    let status = unsafe {
        with_session(|session| {
            if session.is_some() {
                return STATUS_ALREADY_INITIALIZED;
            }
            match VmxCluster::start() {
                Ok(c) => {
                    *session = Some(c);
                    STATUS_SUCCESS
                }
                Err(e) => e,
            }
        })
    };
    if status != STATUS_SUCCESS && status != STATUS_ALREADY_INITIALIZED {
        crate::vmexit::clear_vmexit_session();
    }
    status
}

unsafe extern "C" fn dispatch_create_close(
    _device_object: *mut DEVICE_OBJECT,
    irp: *mut IRP,
) -> NTSTATUS {
    unsafe { complete_request(irp, STATUS_SUCCESS, 0) }
}

unsafe extern "C" fn dispatch_device_control(
    _device_object: *mut DEVICE_OBJECT,
    irp: *mut IRP,
) -> NTSTATUS {
    let stack_location = unsafe {
        (*irp)
            .Tail
            .Overlay
            .__bindgen_anon_2
            .__bindgen_anon_1
            .CurrentStackLocation
    };
    if stack_location.is_null() {
        return unsafe { complete_request(irp, STATUS_UNSUCCESSFUL, 0) };
    }

    let device_io_control = unsafe { (*stack_location).Parameters.DeviceIoControl };
    let ioctl_code = device_io_control.IoControlCode;
    let input_len = device_io_control.InputBufferLength as usize;
    let output_len = device_io_control.OutputBufferLength as usize;
    let system_buffer = unsafe { (*irp).AssociatedIrp.SystemBuffer.cast::<u8>() };

    match ioctl_code {
        IOCTL_PING => {
            if output_len < size_of::<u32>() {
                return unsafe { complete_request(irp, STATUS_BUFFER_TOO_SMALL, 0) };
            }
            if system_buffer.is_null() {
                return unsafe { complete_request(irp, STATUS_UNSUCCESSFUL, 0) };
            }
            unsafe {
                system_buffer
                    .cast::<u32>()
                    .write_unaligned(PING_RESPONSE_U32);
            }
            unsafe { complete_request(irp, STATUS_SUCCESS, size_of::<u32>()) }
        }
        IOCTL_ECHO => {
            if input_len == 0 || input_len > ECHO_MAX_LEN {
                return unsafe { complete_request(irp, STATUS_INVALID_PARAMETER, 0) };
            }
            if output_len < input_len {
                return unsafe { complete_request(irp, STATUS_BUFFER_TOO_SMALL, 0) };
            }
            if system_buffer.is_null() {
                return unsafe { complete_request(irp, STATUS_UNSUCCESSFUL, 0) };
            }
            unsafe { complete_request(irp, STATUS_SUCCESS, input_len) }
        }
        IOCTL_HV_START => {
            let status = unsafe { hv_try_start() };
            unsafe { complete_request(irp, status, 0) }
        }
        IOCTL_HV_STOP => {
            let status = unsafe {
                with_session(|session| {
                    crate::vmexit::clear_vmexit_session();
                    if let Some(mut c) = session.take() {
                        c.stop();
                    }
                    STATUS_SUCCESS
                })
            };
            unsafe { complete_request(irp, status, 0) }
        }
        IOCTL_HV_HYPERCALL => {
            if input_len < size_of::<HvHypercallIn>() || output_len < size_of::<HvHypercallOut>() {
                return unsafe { complete_request(irp, STATUS_BUFFER_TOO_SMALL, 0) };
            }
            if system_buffer.is_null() {
                return unsafe { complete_request(irp, STATUS_UNSUCCESSFUL, 0) };
            }
            let inp = unsafe { system_buffer.cast::<HvHypercallIn>().read_unaligned() };
            let out = unsafe {
                with_session(|session| hypercalls::dispatch(session, &inp))
            };
            unsafe {
                system_buffer
                    .cast::<HvHypercallOut>()
                    .write_unaligned(out);
            }
            unsafe {
                complete_request(irp, STATUS_SUCCESS, size_of::<HvHypercallOut>())
            }
        }
        _ => unsafe { complete_request(irp, STATUS_INVALID_DEVICE_REQUEST, 0) },
    }
}

extern "C" fn driver_unload(driver: *mut DRIVER_OBJECT) {
    unsafe {
        with_session(|session| {
            crate::vmexit::clear_vmexit_session();
            if let Some(mut c) = session.take() {
                c.stop();
            }
        });
    }

    let mut symlink_buf = [0u16; 96];
    let mut devname = String::from("\\DosDevices\\");
    devname.push_str(DEVICE_BASENAME);
    let symlink_used = match encode_utf16z(&devname, &mut symlink_buf) {
        Some(v) => v,
        None => return,
    };
    let mut symlink = to_unicode_string(&mut symlink_buf, symlink_used);

    unsafe {
        let _ = wdk_sys::ntddk::IoDeleteSymbolicLink(&raw mut symlink);
    }

    unsafe {
        if !(*driver).DeviceObject.is_null() {
            wdk_sys::ntddk::IoDeleteDevice((*driver).DeviceObject);
        }
    }
    println!("{} unloaded", "my-hv-driver");
}

// SAFETY: Exported kernel entry point.
#[unsafe(export_name = "DriverEntry")]
pub unsafe extern "system" fn driver_entry(
    driver: &mut DRIVER_OBJECT,
    _registry_path: PCUNICODE_STRING,
) -> NTSTATUS {
    unsafe {
        KeInitializeGuardedMutex(&raw mut SESSION_MUTEX);
    }

    driver.DriverUnload = Some(driver_unload);
    driver.MajorFunction[IRP_MJ_CREATE_INDEX] = Some(dispatch_create_close);
    driver.MajorFunction[IRP_MJ_CLOSE_INDEX] = Some(dispatch_create_close);
    driver.MajorFunction[IRP_MJ_DEVICE_CONTROL_INDEX] = Some(dispatch_device_control);

    let mut dev_kernel = String::from("\\Device\\");
    dev_kernel.push_str(DEVICE_BASENAME);

    let mut device_name_buf = [0u16; 96];
    let device_used = match encode_utf16z(&dev_kernel, &mut device_name_buf) {
        Some(v) => v,
        None => return STATUS_UNSUCCESSFUL,
    };
    let mut device_name = to_unicode_string(&mut device_name_buf, device_used);

    let mut device_object: *mut DEVICE_OBJECT = core::ptr::null_mut();
    let status = unsafe {
        wdk_sys::ntddk::IoCreateDevice(
            driver,
            0,
            &raw mut device_name,
            FILE_DEVICE_UNKNOWN,
            FILE_DEVICE_SECURE_OPEN,
            0,
            &raw mut device_object,
        )
    };
    if !wdk::nt_success(status) {
        return status;
    }

    unsafe {
        (*device_object).Flags |= DO_BUFFERED_IO;
    }

    let mut symlink_buf = [0u16; 96];
    let mut symlink_str = String::from("\\DosDevices\\");
    symlink_str.push_str(DEVICE_BASENAME);
    let symlink_used = match encode_utf16z(&symlink_str, &mut symlink_buf) {
        Some(v) => v,
        None => {
            unsafe {
                wdk_sys::ntddk::IoDeleteDevice(device_object);
            }
            return STATUS_UNSUCCESSFUL;
        }
    };
    let mut symlink_name = to_unicode_string(&mut symlink_buf, symlink_used);

    let status = unsafe { wdk_sys::ntddk::IoCreateSymbolicLink(&raw mut symlink_name, &raw mut device_name) };
    if !wdk::nt_success(status) {
        unsafe {
            wdk_sys::ntddk::IoDeleteDevice(device_object);
        }
        return status;
    }

    let hv_status = unsafe { hv_try_start() };
    if !wdk::nt_success(hv_status) {
        unsafe {
            let _ = wdk_sys::ntddk::IoDeleteSymbolicLink(&raw mut symlink_name);
            wdk_sys::ntddk::IoDeleteDevice(device_object);
        }
        println!(
            "{}: virtualization failed (status {hv_status:#x}), driver not loaded",
            "my-hv-driver"
        );
        return STATUS_HV_OPERATION_FAILED;
    }

    println!(
        "{} loaded (contract {}, device \\Device\\{})",
        "my-hv-driver",
        shared_contract::CONTRACT_VERSION,
        DEVICE_BASENAME
    );
    STATUS_SUCCESS
}
