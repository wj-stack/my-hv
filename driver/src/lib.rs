//! Minimal WDM driver template: device object, symbolic link, ping/echo IOCTLs only.
//! Pairing constants come from `shared-contract`; device strings MUST stay aligned with
//! `shared_contract::DEVICE_BASENAME` / `USER_DEVICE_PATH` (see project `README.md`).

#![no_std]
extern crate alloc;

#[cfg(not(test))]
extern crate wdk_panic;

use core::mem::size_of;

#[cfg(not(test))]
use wdk_alloc::WdkAllocator;
use wdk::println;
use wdk_sys::{
    CCHAR, DEVICE_OBJECT, DRIVER_OBJECT, IO_NO_INCREMENT, IRP, NTSTATUS, PCUNICODE_STRING,
    STATUS_BUFFER_TOO_SMALL, STATUS_INVALID_DEVICE_REQUEST, STATUS_INVALID_PARAMETER,
    STATUS_SUCCESS, STATUS_UNSUCCESSFUL, UNICODE_STRING,
};

#[cfg(not(test))]
#[global_allocator]
static GLOBAL_ALLOCATOR: WdkAllocator = WdkAllocator;

use shared_contract::{ECHO_MAX_LEN, IOCTL_ECHO, IOCTL_PING, PING_RESPONSE_U32};

const IRP_MJ_CREATE_INDEX: usize = 0x00;
const IRP_MJ_CLOSE_INDEX: usize = 0x02;
const IRP_MJ_DEVICE_CONTROL_INDEX: usize = 0x0e;

const FILE_DEVICE_UNKNOWN: u32 = 0x0000_0022;
const FILE_DEVICE_SECURE_OPEN: u32 = 0x0000_0100;
const DO_BUFFERED_IO: u32 = 0x0000_0004;

/// Must match `shared_contract::DEVICE_BASENAME` (`\Device\{basename}`).
const DEVICE_NAME: &str = "\\Device\\MyHvTpl";
/// Must match user-mode `\\.\{basename}`.
const SYMLINK_NAME: &str = "\\DosDevices\\MyHvTpl";

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
            // METHOD_BUFFERED: I/O manager already placed input in `SystemBuffer`; echo is identity.
            unsafe { complete_request(irp, STATUS_SUCCESS, input_len) }
        }
        _ => unsafe { complete_request(irp, STATUS_INVALID_DEVICE_REQUEST, 0) },
    }
}

extern "C" fn driver_unload(driver: *mut DRIVER_OBJECT) {
    let mut symlink_buf = [0u16; 96];
    let symlink_used = match encode_utf16z(SYMLINK_NAME, &mut symlink_buf) {
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
    driver.DriverUnload = Some(driver_unload);
    driver.MajorFunction[IRP_MJ_CREATE_INDEX] = Some(dispatch_create_close);
    driver.MajorFunction[IRP_MJ_CLOSE_INDEX] = Some(dispatch_create_close);
    driver.MajorFunction[IRP_MJ_DEVICE_CONTROL_INDEX] = Some(dispatch_device_control);

    let mut device_name_buf = [0u16; 96];
    let device_used = match encode_utf16z(DEVICE_NAME, &mut device_name_buf) {
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
    let symlink_used = match encode_utf16z(SYMLINK_NAME, &mut symlink_buf) {
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

    println!(
        "{} loaded ({} -> {})",
        "my-hv-driver",
        DEVICE_NAME,
        SYMLINK_NAME
    );
    STATUS_SUCCESS
}
