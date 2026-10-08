//! Facts about other processes: name, working directory, parent, command line.

use std::path::PathBuf;

#[cfg(target_os = "linux")]
pub fn name(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_string())
}

#[cfg(target_os = "linux")]
pub fn cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(target_os = "macos")]
pub fn name(pid: u32) -> Option<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: buf is valid for writes of its length.
    let bytes_read =
        unsafe { libc::proc_pidpath(pid as i32, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if bytes_read <= 0 {
        return None;
    }
    let path = String::from_utf8_lossy(&buf[..bytes_read as usize]).into_owned();
    Some(path.rsplit('/').next().unwrap_or(&path).to_string())
}

#[cfg(target_os = "macos")]
pub fn cwd(pid: u32) -> Option<PathBuf> {
    // SAFETY: zeroed POD struct filled by the kernel.
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    let bytes_read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&mut info as *mut libc::proc_vnodepathinfo).cast(),
            size,
        )
    };
    if bytes_read != size {
        return None;
    }
    let path = &info.pvi_cdir.vip_path;
    // libc declares the MAXPATHLEN buffer as [[c_char; 32]; 32].
    let bytes: Vec<u8> = path
        .iter()
        .flatten()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn name(_: u32) -> Option<String> {
    None
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn cwd(_: u32) -> Option<PathBuf> {
    None
}

#[cfg(target_os = "linux")]
pub fn ppid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // "pid (comm) state ppid …": comm may contain spaces and parentheses.
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(target_os = "linux")]
pub fn cmdline(pid: u32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let args: Vec<String> = raw
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    (!args.is_empty()).then(|| args.join(" "))
}

#[cfg(target_os = "macos")]
pub fn ppid(pid: u32) -> Option<u32> {
    // SAFETY: zeroed POD struct filled by the kernel.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    let bytes_read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    (bytes_read == size).then_some(info.pbi_ppid)
}

#[cfg(target_os = "macos")]
pub fn cmdline(pid: u32) -> Option<String> {
    // KERN_PROCARGS2 parsing is intricate; ps does it (development only).
    let output = std::process::Command::new("ps")
        .args(["-o", "args=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let command_line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!command_line.is_empty()).then_some(command_line)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn ppid(_: u32) -> Option<u32> {
    None
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn cmdline(_: u32) -> Option<String> {
    None
}

/// Whether `ancestor` is `pid` or one of its ancestors.
pub fn descends_from(pid: u32, ancestor: u32) -> bool {
    let mut current_pid = pid;
    for _ in 0..64 {
        if current_pid == ancestor {
            return true;
        }
        match ppid(current_pid) {
            Some(parent_pid) if parent_pid > 1 && parent_pid != current_pid => {
                current_pid = parent_pid
            }
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn own_process() {
        let pid = std::process::id();
        assert!(super::ppid(pid).is_some());
        assert!(super::descends_from(pid, pid));
        assert!(super::descends_from(pid, super::ppid(pid).unwrap()));
        assert!(super::cmdline(pid).is_some());
        assert!(super::cwd(pid).is_some());
    }
}
