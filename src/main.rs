#![no_main]

mod inject;

use std::ffi::{OsStr, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::Duration;

const DEFAULT_DYLIB: &str = "Library/Dictionaries/libscimxx_hook.dylib";
const PROC_ALL_PIDS: u32 = 1;
const ADMIN_SCRIPT: &str = r#"on run argv
do shell script ((quoted form of item 1 of argv) & " inject " & (quoted form of item 2 of argv) & " " & (quoted form of item 3 of argv)) with administrator privileges
end run"#;

fn report(message: &str) {
    unsafe {
        libc::write(libc::STDERR_FILENO, message.as_ptr().cast(), message.len());
        libc::write(libc::STDERR_FILENO, c"\n".as_ptr().cast(), 1);
    }
}

#[link(name = "proc")]
unsafe extern "C" {
    fn proc_listpids(proc_type: u32, typeinfo: u32, buffer: *mut c_void, buffersize: i32) -> i32;
    fn proc_name(pid: i32, buffer: *mut c_void, buffersize: u32) -> i32;
}

fn find_scim_extension() -> Option<i32> {
    unsafe {
        let needed = proc_listpids(PROC_ALL_PIDS, 0, ptr::null_mut(), 0);
        if needed <= 0 {
            return None;
        }
        let mut pids = vec![0i32; needed as usize / 4];
        let count = proc_listpids(PROC_ALL_PIDS, 0, pids.as_mut_ptr() as *mut c_void, needed);
        if count <= 0 {
            return None;
        }
        let mut name = [0u8; 64];
        pids[..count as usize / 4].iter().copied().find(|&pid| {
            let len = proc_name(pid, name.as_mut_ptr() as *mut c_void, name.len() as u32);
            len > 0 && &name[..len as usize] == b"SCIM_Extension"
        })
    }
}

fn inject(pid: i32, dylib_path: &Path) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| format!("无法取得自身路径：{error}"))?;
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(ADMIN_SCRIPT)
        .arg(exe)
        .arg(pid.to_string())
        .arg(dylib_path)
        .output()
        .map_err(|error| format!("osascript 执行失败：{error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn watch_exit(pid: i32) {
    unsafe {
        let kq = libc::kqueue();
        if kq < 0 {
            return;
        }
        let change = libc::kevent {
            ident: pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: ptr::null_mut(),
        };
        let mut event: libc::kevent = std::mem::zeroed();
        if libc::kevent(kq, &change, 1, &mut event, 1, ptr::null()) == 1 {
            report(&format!("SCIM_Extension (pid {pid}) 已退出"));
        }
        libc::close(kq);
    }
}

#[unsafe(no_mangle)]
extern "C" fn main() -> i32 {
    run()
}

fn run() -> i32 {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(OsStr::new("inject")) {
        let pid = args.next().and_then(|value| value.to_str()?.parse().ok());
        let path = args.next().map(PathBuf::from);
        return match (pid, path) {
            (Some(pid), Some(path)) => match inject::inject(pid, &path) {
                Ok(()) => 0,
                Err(message) => {
                    report(&message);
                    1
                }
            },
            _ => {
                report("usage: scimxx inject <pid> <dylib-path>");
                2
            }
        };
    }

    let dylib_path = if let Some(path) = first {
        PathBuf::from(path)
    } else {
        let Some(home) = std::env::var_os("HOME") else {
            report("HOME 未设置");
            return 2;
        };
        PathBuf::from(home).join(DEFAULT_DYLIB)
    };
    if !dylib_path.exists() {
        report(&format!("未找到 hook dylib：{}", dylib_path.display()));
        return 2;
    }
    report("scimxx 等待 SCIM_Extension 出现");
    loop {
        let Some(pid) = find_scim_extension() else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        match inject(pid, &dylib_path) {
            Ok(()) => {
                report(&format!("注入成功 pid={pid}"));
                watch_exit(pid);
            }
            Err(message) => {
                report(&format!("注入失败 pid={pid}：{message}"));
                watch_exit(pid);
            }
        }
    }
}
