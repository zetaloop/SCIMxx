use core::ffi::{CStr, c_void};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use ctor::ctor;
use objc2::ffi::{
    class_getInstanceMethod, class_replaceMethod, method_getImplementation, method_getTypeEncoding,
    objc_getClass,
};
use objc2::runtime::{AnyObject, Imp, Sel};
use objc2::{Encode, Encoding, msg_send, sel};

static ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

const SHIFT: u64 = 1 << 17;
const COMMAND: u64 = 1 << 20;
const NSKEYDOWN: u64 = 10;
const DEVICE_INDEPENDENT_FLAGS: u64 = 0xffff0000;

core::arch::global_asm!(
    ".text",
    ".p2align 2",
    ".globl _scimxx_call_imp",
    "_scimxx_call_imp:",
    "braaz x4",
);

unsafe extern "C-unwind" {
    fn scimxx_call_imp(
        this: *mut AnyObject,
        cmd: Sel,
        event: *mut AnyObject,
        client: *mut AnyObject,
        imp: Imp,
    ) -> bool;
}

unsafe extern "C" {
    fn _dyld_register_func_for_add_image(
        func: Option<unsafe extern "C" fn(*const libc::c_void, isize)>,
    );
    fn dprintf(fd: libc::c_int, format: *const libc::c_char, ...) -> libc::c_int;
}

struct Target {
    key_code: u16,
    modifiers: u64,
    characters: &'static CStr,
    ignoring: &'static CStr,
}

fn target_for(key_code: u16, modifiers: u64) -> Option<Target> {
    match (key_code, modifiers & DEVICE_INDEPENDENT_FLAGS) {
        (0x2b, 0) => Some(Target {
            key_code: 0x21,
            modifiers: 0,
            characters: c"[",
            ignoring: c"[",
        }),
        (0x2f, 0) => Some(Target {
            key_code: 0x1e,
            modifiers: 0,
            characters: c"]",
            ignoring: c"]",
        }),
        (0x21, 0) => Some(Target {
            key_code: 0x21,
            modifiers: SHIFT | COMMAND,
            characters: c"{",
            ignoring: c"【",
        }),
        (0x1e, 0) => Some(Target {
            key_code: 0x1e,
            modifiers: SHIFT | COMMAND,
            characters: c"}",
            ignoring: c"】",
        }),
        _ => None,
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}

unsafe impl Encode for NSPoint {
    const ENCODING: Encoding = Encoding::Struct("NSPoint", &[f64::ENCODING, f64::ENCODING]);
}

fn rewritten_event(event: *const AnyObject, target: &Target) -> Option<*const AnyObject> {
    unsafe {
        let location: NSPoint = msg_send![event, locationInWindow];
        let timestamp: f64 = msg_send![event, timestamp];
        let window_number: isize = msg_send![event, windowNumber];
        let context: *const AnyObject = msg_send![event, context];
        let is_repeat: bool = msg_send![event, isARepeat];
        let ns_string = objc_getClass(c"NSString".as_ptr());
        let ns_event = objc_getClass(c"NSEvent".as_ptr());
        if ns_string.is_null() || ns_event.is_null() {
            return None;
        }
        let characters: *const AnyObject =
            msg_send![ns_string, stringWithUTF8String: target.characters.as_ptr()];
        let ignoring: *const AnyObject =
            msg_send![ns_string, stringWithUTF8String: target.ignoring.as_ptr()];
        if characters.is_null() || ignoring.is_null() {
            return None;
        }
        let new_event: *const AnyObject = msg_send![
            ns_event,
            keyEventWithType: NSKEYDOWN,
            location: location,
            modifierFlags: target.modifiers,
            timestamp: timestamp,
            windowNumber: window_number,
            context: context,
            characters: characters,
            charactersIgnoringModifiers: ignoring,
            isARepeat: is_repeat,
            keyCode: target.key_code,
        ];
        (!new_event.is_null()).then_some(new_event)
    }
}

unsafe extern "C-unwind" fn handle_event_replacement(
    this: *mut AnyObject,
    cmd: Sel,
    event: *mut AnyObject,
    client: *mut AnyObject,
) -> bool {
    unsafe {
        let original = ORIGINAL.load(Ordering::Acquire);
        if original.is_null() {
            return false;
        }
        let original = core::mem::transmute::<*mut c_void, Imp>(original);
        let mut forwarded = event;
        let candidate_controller: *const AnyObject = msg_send![this, candidateController];
        let showing = if candidate_controller.is_null() {
            false
        } else {
            msg_send![candidate_controller, isVisible]
        };
        let event_type: u64 = msg_send![event, type];
        if showing && event_type == NSKEYDOWN {
            let key_code: u16 = msg_send![event, keyCode];
            let modifiers: u64 = msg_send![event, modifierFlags];
            if let Some(target) = target_for(key_code, modifiers)
                && let Some(rewritten) = rewritten_event(event, &target)
            {
                forwarded = rewritten as *mut AnyObject;
            }
        }
        scimxx_call_imp(this, cmd, forwarded, client, original)
    }
}

#[ctor]
fn install() {
    unsafe {
        if !install_inner() {
            let callback = core::mem::transmute::<
                usize,
                unsafe extern "C" fn(*const libc::c_void, isize),
            >(sign_function(on_image_added as *const () as usize));
            _dyld_register_func_for_add_image(Some(callback));
        }
    }
}

unsafe extern "C" fn on_image_added(_header: *const libc::c_void, _slide: isize) {
    unsafe {
        install_inner();
    }
}

unsafe fn sign_function(mut function: usize) -> usize {
    unsafe {
        core::arch::asm!("paciza {function}", function = inout(reg) function);
    }
    function
}

unsafe fn install_inner() -> bool {
    if !ORIGINAL.load(Ordering::Acquire).is_null() {
        return true;
    }
    let engine_class = unsafe { objc_getClass(c"CIMPinyinEngine".as_ptr()) };
    if engine_class.is_null() {
        return false;
    }
    let sel = sel!(handleEvent:client:);
    let method = unsafe { class_getInstanceMethod(engine_class, sel) };
    if method.is_null() {
        log(c"handleEvent:client: not found");
        return true;
    }
    let encoding = unsafe { CStr::from_ptr(method_getTypeEncoding(method)) };
    if encoding.to_bytes() != b"B32@0:8@16@24" {
        log(c"unexpected handleEvent:client: signature");
        return true;
    }
    let Some(original) = (unsafe { method_getImplementation(method) }) else {
        log(c"handleEvent:client: implementation missing");
        return true;
    };
    let original = original as *mut c_void;
    if ORIGINAL
        .compare_exchange(
            ptr::null_mut(),
            original,
            Ordering::Release,
            Ordering::Acquire,
        )
        .is_err()
    {
        return true;
    }
    let replacement: Imp = unsafe {
        core::mem::transmute::<usize, Imp>(sign_function(
            handle_event_replacement as *const () as usize,
        ))
    };
    unsafe {
        class_replaceMethod(
            engine_class.cast_mut(),
            sel,
            replacement,
            method_getTypeEncoding(method),
        );
    }
    log(c"handleEvent:client: hooked");
    true
}

fn log(message: &CStr) {
    unsafe {
        let home = libc::getenv(c"HOME".as_ptr());
        if home.is_null() {
            return;
        }
        let mut path = [0 as libc::c_char; libc::PATH_MAX as usize];
        let length = libc::snprintf(
            path.as_mut_ptr(),
            path.len(),
            c"%s/Library/Dictionaries/scimxx-hook.log".as_ptr(),
            home,
        );
        if length < 0 || length as usize >= path.len() {
            return;
        }
        let fd = libc::open(
            path.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND,
            0o644,
        );
        if fd < 0 {
            return;
        }
        dprintf(fd, c"[%d] %s\n".as_ptr(), libc::getpid(), message.as_ptr());
        libc::close(fd);
    }
}
