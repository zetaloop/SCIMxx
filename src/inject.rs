use std::ffi::{CStr, CString, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Duration;

const ARM_THREAD_STATE64: u32 = 6;
const ARM_THREAD_STATE64_COUNT: u32 = 68;
const RTLD_NOW: c_int = 2;
const VM_FLAGS_ANYWHERE: c_int = 1;
const VM_PROT_READ: c_int = 1;
const VM_PROT_EXECUTE: c_int = 4;
const KERN_SUCCESS: i32 = 0;
const STACK_SIZE: u64 = 0x10000;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ArmThreadState64 {
    x: [u64; 29],
    fp: u64,
    lr: u64,
    sp: u64,
    pc: u64,
    cpsr: u32,
    flags: u32,
}

unsafe extern "C" {
    fn mach_task_self() -> u32;
    fn mach_port_deallocate(task: u32, name: u32) -> i32;
    fn mach_vm_allocate(target: u32, address: *mut u64, size: u64, flags: c_int) -> i32;
    fn mach_vm_write(target: u32, address: u64, data: u64, count: u32) -> i32;
    fn mach_vm_read_overwrite(
        target: u32,
        address: u64,
        size: u64,
        data: u64,
        outsize: *mut u64,
    ) -> i32;
    fn mach_vm_protect(
        target: u32,
        address: u64,
        size: u64,
        set_maximum: c_int,
        new_protection: c_int,
    ) -> i32;
    fn mach_vm_deallocate(target: u32, address: u64, size: u64) -> i32;
    fn thread_create(target: u32, thread: *mut u32) -> i32;
    fn thread_terminate(thread: u32) -> i32;
    fn thread_create_running(
        target: u32,
        flavor: u32,
        state: *mut c_void,
        count: u32,
        thread: *mut u32,
    ) -> i32;
    fn thread_convert_thread_state(
        thread: u32,
        direction: c_int,
        flavor: i32,
        in_state: *mut c_void,
        in_count: u32,
        out_state: *mut c_void,
        out_count: *mut u32,
    ) -> i32;
}

core::arch::global_asm!(
    ".section __TEXT,__scimxx_stub",
    ".p2align 2",
    ".globl _scimxx_stub_start",
    "_scimxx_stub_start:",
    "sub sp, sp, #0x60",
    "str x0, [sp, #0x20]",
    "str w1, [sp, #0x28]",
    "str x3, [sp, #0x30]",
    "str x2, [sp, #0x38]",
    "add x3, sp, #0x20",
    "add x0, sp, #0x8",
    "mov x8, #0",
    "str x8, [sp, #0x8]",
    "mov x1, x8",
    "adr x2, Lscimxx_stub_worker",
    "paciza x2",
    "mov x9, x4",
    "paciza x9",
    "blraaz x9",
    "b .",
    "Lscimxx_stub_worker:",
    "pacibsp",
    "sub sp, sp, #0x30",
    "stp x29, x30, [sp, #0x20]",
    "add x29, sp, #0x20",
    "str x0, [sp, #0x10]",
    "ldr x8, [x0, #0x10]",
    "ldr x0, [x0]",
    "paciza x8",
    "ldr x1, [sp, #0x10]",
    "ldr w1, [x1, #0x8]",
    "blraaz x8",
    "ldr x1, [sp, #0x10]",
    "ldr x1, [x1, #0x18]",
    "str x0, [x1]",
    "mov w0, #0",
    "ldp x29, x30, [sp, #0x20]",
    "add sp, sp, #0x30",
    "retab",
    ".globl _scimxx_stub_end",
    "_scimxx_stub_end:",
);

unsafe extern "C" {
    static scimxx_stub_start: u8;
    static scimxx_stub_end: u8;
}

pub fn inject(pid: i32, path: &Path) -> Result<(), String> {
    unsafe {
        let path_c =
            CString::new(path.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
        let mut task = 0;
        let kr = libc::task_for_pid(mach_task_self(), pid, &mut task);
        if kr != KERN_SUCCESS {
            return Err(format!("task_for_pid({pid}) 失败：{}", mach_error(kr)));
        }

        let dlopen = libc::dlsym(libc::RTLD_DEFAULT, c"dlopen".as_ptr());
        let pcfmt = libc::dlsym(
            libc::RTLD_DEFAULT,
            c"pthread_create_from_mach_thread".as_ptr(),
        );
        if dlopen.is_null() || pcfmt.is_null() {
            mach_port_deallocate(mach_task_self(), task);
            return Err("符号未找到".to_string());
        }
        let dlopen = strip_pac(dlopen as usize) as *mut c_void;
        let pcfmt = strip_pac(pcfmt as usize) as *mut c_void;

        let path_len = path_c.as_bytes_with_nul().len();
        let off_slot = (path_len + 7) & !7;
        let page = libc::vm_page_size as u64;
        let total = off_slot as u64 + 8 + STACK_SIZE + 2 * page;
        let mut remote = 0u64;
        let kr = mach_vm_allocate(task, &mut remote, total, VM_FLAGS_ANYWHERE);
        if kr != KERN_SUCCESS {
            mach_port_deallocate(mach_task_self(), task);
            return Err(format!("mach_vm_allocate 失败：{}", mach_error(kr)));
        }

        let kr = mach_vm_write(task, remote, path_c.as_ptr() as u64, path_len as u32);
        let slot = remote + off_slot as u64;
        let sentinel = 0xdeadbeefdeadbeefu64;
        let kr = if kr == KERN_SUCCESS {
            mach_vm_write(task, slot, &sentinel as *const u64 as u64, 8)
        } else {
            kr
        };
        let stub = (remote + off_slot as u64 + 8 + STACK_SIZE + page) & !(page - 1);
        let stub_start = &raw const scimxx_stub_start;
        let stub_end = &raw const scimxx_stub_end;
        let stub_len = stub_end as usize - stub_start as usize;
        let mut stub_copy = vec![0u8; stub_len];
        std::ptr::copy_nonoverlapping(stub_start, stub_copy.as_mut_ptr(), stub_len);
        let kr = if kr == KERN_SUCCESS {
            mach_vm_write(task, stub, stub_copy.as_ptr() as u64, stub_len as u32)
        } else {
            kr
        };
        let kr = if kr == KERN_SUCCESS {
            mach_vm_protect(task, stub, page, 0, VM_PROT_READ | VM_PROT_EXECUTE)
        } else {
            kr
        };
        if kr != KERN_SUCCESS {
            mach_vm_deallocate(task, remote, total);
            mach_port_deallocate(mach_task_self(), task);
            return Err(format!("mach_vm 失败：{}", mach_error(kr)));
        }

        let mut state = ArmThreadState64::default();
        let mut pc = stub;
        core::arch::asm!("pacia {pc}, {discr}", pc = inout(reg) pc, discr = in(reg) 0x7481u64);
        state.pc = pc;
        let mut sp = stub - 0x10;
        core::arch::asm!("pacda {sp}, {discr}", sp = inout(reg) sp, discr = in(reg) 0xCBEDu64);
        state.sp = sp;
        state.x[0] = remote;
        state.x[1] = RTLD_NOW as u64;
        state.x[2] = slot;
        state.x[3] = dlopen as u64;
        state.x[4] = pcfmt as u64;

        let mut tmp = 0u32;
        let kr = thread_create(task, &mut tmp);
        if kr != KERN_SUCCESS {
            mach_vm_deallocate(task, remote, total);
            mach_port_deallocate(mach_task_self(), task);
            return Err(format!("thread_create 失败：{}", mach_error(kr)));
        }
        let mut machine = ArmThreadState64::default();
        let mut machine_count = ARM_THREAD_STATE64_COUNT;
        let kr = thread_convert_thread_state(
            tmp,
            2,
            ARM_THREAD_STATE64 as i32,
            &state as *const _ as *mut c_void,
            ARM_THREAD_STATE64_COUNT,
            &mut machine as *mut _ as *mut c_void,
            &mut machine_count,
        );
        thread_terminate(tmp);
        mach_port_deallocate(mach_task_self(), tmp);
        if kr != KERN_SUCCESS {
            mach_vm_deallocate(task, remote, total);
            mach_port_deallocate(mach_task_self(), task);
            return Err(format!("thread 准备失败：{}", mach_error(kr)));
        }

        let mut thread = 0u32;
        let kr = thread_create_running(
            task,
            ARM_THREAD_STATE64,
            &machine as *const _ as *mut c_void,
            machine_count,
            &mut thread,
        );
        if kr != KERN_SUCCESS {
            mach_vm_deallocate(task, remote, total);
            mach_port_deallocate(mach_task_self(), task);
            return Err(format!("thread_create_running 失败：{}", mach_error(kr)));
        }

        let mut handle = sentinel;
        for _ in 0..500 {
            let mut read_size = 8u64;
            if mach_vm_read_overwrite(
                task,
                slot,
                8,
                &mut handle as *mut u64 as u64,
                &mut read_size,
            ) == KERN_SUCCESS
                && handle != sentinel
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        thread_terminate(thread);
        mach_port_deallocate(mach_task_self(), thread);
        mach_vm_deallocate(task, remote, total);
        mach_port_deallocate(mach_task_self(), task);

        if handle == sentinel {
            return Err("dlopen 未返回".to_string());
        }
        if handle == 0 {
            return Err(format!("dlopen 失败：{}", path.display()));
        }
        Ok(())
    }
}

fn strip_pac(mut pointer: usize) -> usize {
    unsafe {
        core::arch::asm!("xpaci {pointer}", pointer = inout(reg) pointer);
    }
    pointer
}

fn mach_error(kr: i32) -> String {
    unsafe {
        CStr::from_ptr(libc::mach_error_string(kr))
            .to_string_lossy()
            .into_owned()
    }
}
