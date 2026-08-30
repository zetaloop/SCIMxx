#![no_main]

mod inject;

use std::ffi::{OsStr, OsString, c_void};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const LABEL: &str = "build.loop.scimxx";
const HOOK_NAME: &str = "libscimxx_hook.dylib";
const PLIST_PATH: &str = "/Library/LaunchDaemons/build.loop.scimxx.plist";
const PROC_ALL_PIDS: u32 = 1;
const ADMIN_SCRIPT: &str = r#"on run argv
set command to ""
repeat with argument in argv
set command to command & quoted form of (contents of argument) & " "
end repeat
do shell script command with administrator privileges
end run"#;

fn write_line(fd: libc::c_int, message: &str) {
    unsafe {
        libc::write(fd, message.as_ptr().cast(), message.len());
        libc::write(fd, c"\n".as_ptr().cast(), 1);
    }
}

fn output(message: &str) {
    write_line(libc::STDOUT_FILENO, message);
}

fn report(message: &str) {
    write_line(libc::STDERR_FILENO, message);
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

fn watch_exit(pid: i32) -> Result<(), String> {
    unsafe {
        let kq = libc::kqueue();
        if kq < 0 {
            return Err(format!("kqueue 失败：{}", std::io::Error::last_os_error()));
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
        let result = libc::kevent(kq, &change, 1, &mut event, 1, ptr::null());
        let error = std::io::Error::last_os_error();
        libc::close(kq);
        if result == 1 {
            Ok(())
        } else {
            Err(format!("kevent 失败：{error}"))
        }
    }
}

fn run_daemon(path: &Path) -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("daemon 需要管理员权限".to_string());
    }
    if !path.is_file() {
        return Err(format!("未找到 hook dylib：{}", path.display()));
    }
    loop {
        let Some(pid) = find_scim_extension() else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        if let Err(message) = inject::inject(pid, path) {
            report(&format!("注入失败 pid={pid}：{message}"));
        }
        watch_exit(pid)?;
    }
}

fn hook_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or_else(|| "HOME 未设置".to_string())?;
    Ok(PathBuf::from(home)
        .join("Library/Dictionaries")
        .join(HOOK_NAME))
}

fn invoked_executable() -> Result<PathBuf, String> {
    let invoked = PathBuf::from(
        std::env::args_os()
            .next()
            .ok_or_else(|| "无法取得自身路径".to_string())?,
    );
    if invoked.components().count() > 1 {
        return std::path::absolute(invoked).map_err(|error| error.to_string());
    }
    let path = std::env::var_os("PATH").ok_or_else(|| "PATH 未设置".to_string())?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(&invoked))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| "无法取得自身路径".to_string())
        .and_then(|path| std::path::absolute(path).map_err(|error| error.to_string()))
}

fn elevate(action: &str, arguments: &[&OsStr]) -> Result<(), String> {
    if unsafe { libc::geteuid() } == 0 {
        return Err("请以当前用户运行 SCIMxx".to_string());
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let result = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(ADMIN_SCRIPT)
        .arg(executable)
        .arg("service")
        .arg(action)
        .args(arguments)
        .output()
        .map_err(|error| format!("无法请求管理员权限：{error}"))?;
    if result.status.success() {
        Ok(())
    } else {
        let message = String::from_utf8_lossy(&result.stderr).trim().to_string();
        if message.is_empty() {
            Err(format!("管理员操作失败：{}", result.status))
        } else {
            Err(message)
        }
    }
}

fn install() -> Result<(), String> {
    let source = fs::canonicalize(std::env::current_exe().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?
        .with_file_name(HOOK_NAME);
    if !source.is_file() {
        return Err(format!("未找到 companion dylib：{}", source.display()));
    }
    let executable = invoked_executable()?;
    let hook = hook_path()?;
    fs::create_dir_all(hook.parent().unwrap()).map_err(|error| error.to_string())?;
    fs::copy(&source, &hook).map_err(|error| format!("安装 hook 失败：{error}"))?;
    elevate("install", &[executable.as_os_str(), hook.as_os_str()])
}

fn uninstall() -> Result<(), String> {
    elevate("uninstall", &[])?;
    match fs::remove_file(hook_path()?) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("删除 hook 失败：{error}")),
    }
}

fn escape_xml(path: &Path) -> Result<String, String> {
    let path = path
        .to_str()
        .ok_or_else(|| format!("路径不是有效的 Unicode：{}", path.display()))?;
    Ok(path
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;"))
}

fn service_plist(executable: &Path, hook: &Path) -> Result<String, String> {
    let executable = escape_xml(executable)?;
    let hook = escape_xml(hook)?;
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>KeepAlive</key>
    <true/>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{executable}</string>
        <string>daemon</string>
        <string>{hook}</string>
    </array>
</dict>
</plist>
"#
    ))
}

fn command_error(name: &str, result: &std::process::Output) -> String {
    let message = String::from_utf8_lossy(&result.stderr).trim().to_string();
    if message.is_empty() {
        format!("{name} 失败：{}", result.status)
    } else {
        message
    }
}

fn bootout_service() -> Result<(), String> {
    let target = format!("system/{LABEL}");
    let status = Command::new("/bin/launchctl")
        .args(["print", &target])
        .output()
        .map_err(|error| format!("launchctl 执行失败：{error}"))?;
    if status.status.code() == Some(113) {
        return Ok(());
    }
    if !status.status.success() {
        return Err(command_error("读取服务状态", &status));
    }
    let result = Command::new("/bin/launchctl")
        .args(["bootout", &target])
        .output()
        .map_err(|error| format!("launchctl 执行失败：{error}"))?;
    if result.status.success() {
        Ok(())
    } else {
        Err(command_error("停止服务", &result))
    }
}

fn bootstrap_service() -> Result<(), String> {
    let result = Command::new("/bin/launchctl")
        .args(["bootstrap", "system", PLIST_PATH])
        .output()
        .map_err(|error| format!("launchctl 执行失败：{error}"))?;
    if result.status.success() {
        Ok(())
    } else {
        Err(command_error("启动服务", &result))
    }
}

fn terminate_scim_extension() -> Result<(), String> {
    let result = Command::new("/usr/bin/killall")
        .args(["-9", "SCIM_Extension"])
        .output()
        .map_err(|error| format!("killall 执行失败：{error}"))?;
    if result.status.success() || result.status.code() == Some(1) {
        Ok(())
    } else {
        Err(command_error("停止 SCIM_Extension", &result))
    }
}

fn remove_plist() -> Result<(), String> {
    match fs::remove_file(PLIST_PATH) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("删除服务配置失败：{error}")),
    }
}

fn service_install(executable: &Path, hook: &Path) -> Result<(), String> {
    if !executable.is_absolute() || !executable.is_file() {
        return Err(format!("无效的可执行文件：{}", executable.display()));
    }
    if !hook.is_absolute() || !hook.is_file() {
        return Err(format!("无效的 hook：{}", hook.display()));
    }
    let plist = service_plist(executable, hook)?;
    bootout_service()?;
    terminate_scim_extension()?;
    remove_plist()?;
    fs::write(PLIST_PATH, plist).map_err(|error| format!("写入服务配置失败：{error}"))?;
    fs::set_permissions(PLIST_PATH, fs::Permissions::from_mode(0o644))
        .map_err(|error| format!("设置服务配置权限失败：{error}"))?;
    bootstrap_service()
}

fn service_start() -> Result<(), String> {
    if !Path::new(PLIST_PATH).is_file() {
        return Err("SCIMxx 尚未安装".to_string());
    }
    bootout_service()?;
    bootstrap_service()
}

fn service_stop() -> Result<(), String> {
    bootout_service()?;
    terminate_scim_extension()
}

fn service_uninstall() -> Result<(), String> {
    service_stop()?;
    remove_plist()
}

fn run_service(action: &OsStr, arguments: &[OsString]) -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("service 需要管理员权限".to_string());
    }
    match (action.to_str(), arguments) {
        (Some("install"), [executable, hook]) => {
            service_install(Path::new(executable), Path::new(hook))
        }
        (Some("start"), []) => service_start(),
        (Some("stop"), []) => service_stop(),
        (Some("uninstall"), []) => service_uninstall(),
        _ => Err("无效的 service 命令".to_string()),
    }
}

fn run() -> Result<Option<String>, String> {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    match arguments.as_slice() {
        [] => elevate("start", &[]).map(|()| Some("已开启".to_string())),
        [command] if command == "start" || command == "--start" => {
            elevate("start", &[]).map(|()| Some("已开启".to_string()))
        }
        [command] if command == "stop" || command == "--stop" => {
            elevate("stop", &[]).map(|()| Some("已关闭".to_string()))
        }
        [command] if command == "install" => install().map(|()| Some("已安装".to_string())),
        [command] if command == "uninstall" => uninstall().map(|()| Some("已卸载".to_string())),
        [command] if command == "version" || command == "--version" => {
            Ok(Some(format!("v{VERSION}")))
        }
        [command, path] if command == "daemon" => run_daemon(Path::new(path)).map(|()| None),
        [command, action, rest @ ..] if command == "service" => {
            run_service(action, rest).map(|()| None)
        }
        _ => Err("参数无法识别".to_string()),
    }
}

#[unsafe(no_mangle)]
extern "C" fn main() -> i32 {
    match run() {
        Ok(Some(message)) => {
            output(&message);
            0
        }
        Ok(None) => 0,
        Err(message) => {
            report(&message);
            1
        }
    }
}
