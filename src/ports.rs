//! Discovery of local TCP listeners that can be forwarded.

use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Listener {
    pub port: u16,
    /// Process name, when it belongs to a process we can inspect.
    pub process: String,
    pub pid: u32,
}

/// Listeners reachable through `localhost` (bound to a wildcard or loopback
/// address), sorted by port.
pub fn listeners() -> Vec<Listener> {
    platform::listeners().into_values().collect()
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::collections::HashMap;

    const TCP_LISTEN: &str = "0A";

    pub fn listeners() -> BTreeMap<u16, Listener> {
        // inode → port, from /proc/net/tcp{,6}
        let mut by_inode: HashMap<u64, u16> = HashMap::new();
        let mut by_port = BTreeMap::new();
        for (file, ipv6) in [("/proc/net/tcp", false), ("/proc/net/tcp6", true)] {
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            for line in text.lines().skip(1) {
                let fields: Vec<&str> = line.split_whitespace().collect();
                if fields.len() < 10 || fields[3] != TCP_LISTEN {
                    continue;
                }
                let Some((address, port)) = fields[1].split_once(':') else {
                    continue;
                };
                let Ok(port) = u16::from_str_radix(port, 16) else {
                    continue;
                };
                if !reachable_via_localhost(address, ipv6) {
                    continue;
                }
                let inode: u64 = fields[9].parse().unwrap_or(0);
                if inode != 0 {
                    by_inode.insert(inode, port);
                }
                by_port.entry(port).or_insert(Listener {
                    port,
                    process: String::new(),
                    pid: 0,
                });
            }
        }
        // Map socket inodes to processes we are allowed to inspect.
        let Ok(processes) = std::fs::read_dir("/proc") else {
            return by_port;
        };
        for process in processes.flatten() {
            let Some(pid) = process
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(fds) = std::fs::read_dir(process.path().join("fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                let Ok(target) = std::fs::read_link(fd.path()) else {
                    continue;
                };
                let Some(inode) = target
                    .to_str()
                    .and_then(|t| t.strip_prefix("socket:["))
                    .and_then(|t| t.strip_suffix(']'))
                    .and_then(|t| t.parse::<u64>().ok())
                else {
                    continue;
                };
                if let Some(port) = by_inode.get(&inode)
                    && let Some(listener) =
                        by_port.get_mut(port).filter(|listener| listener.pid == 0)
                {
                    listener.pid = pid;
                    listener.process = std::fs::read_to_string(process.path().join("comm"))
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                }
            }
        }
        by_port
    }

    /// /proc/net addresses are hex, in host byte order per 32-bit word.
    fn reachable_via_localhost(address: &str, ipv6: bool) -> bool {
        if !ipv6 {
            // 0.0.0.0 or 127.0.0.0/8
            return address == "00000000" || address.ends_with("7F") && address.len() == 8;
        }
        matches!(
            address,
            // ::
            "00000000000000000000000000000000"
            // ::1
            | "00000000000000000000000001000000"
            // ::ffff:127.0.0.1 (v4-mapped loopback)
            | "0000000000000000FFFF00000100007F"
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn addresses() {
            assert!(reachable_via_localhost("0100007F", false)); // 127.0.0.1
            assert!(reachable_via_localhost("00000000", false));
            assert!(!reachable_via_localhost("0500000A", false)); // 10.0.0.5
            assert!(reachable_via_localhost(
                "00000000000000000000000001000000",
                true
            ));
            assert!(!reachable_via_localhost(
                "000080FE00000000FF005450B1F6FFFE",
                true
            ));
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    /// macOS has no /proc; lsof is part of the base system.
    pub fn listeners() -> BTreeMap<u16, Listener> {
        let mut by_port = BTreeMap::new();
        let Ok(output) = std::process::Command::new("lsof")
            .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "pcn"])
            .output()
        else {
            return by_port;
        };
        let (mut pid, mut command) = (0u32, String::new());
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let (tag, value) = line.split_at(1.min(line.len()));
            match tag {
                "p" => pid = value.parse().unwrap_or(0),
                "c" => command = value.to_string(),
                "n" => {
                    let Some((address, port)) = value.rsplit_once(':') else {
                        continue;
                    };
                    let Ok(port) = port.parse::<u16>() else {
                        continue;
                    };
                    let local =
                        matches!(address, "*" | "127.0.0.1" | "[::1]" | "[::]" | "localhost")
                            || address.starts_with("127.");
                    if local {
                        by_port.entry(port).or_insert(Listener {
                            port,
                            process: command.clone(),
                            pid,
                        });
                    }
                }
                _ => {}
            }
        }
        by_port
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    use super::*;
    pub fn listeners() -> BTreeMap<u16, Listener> {
        BTreeMap::new()
    }
}
