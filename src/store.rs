//! Per-user persistent data (one JSON file per user).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::config::Button;

#[derive(Default, Serialize, Deserialize)]
pub struct UserData {
    #[serde(default)]
    pub buttons: Vec<Button>,
}

pub struct Store {
    dir: PathBuf,
    /// Serializes read-modify-write cycles.
    lock: Mutex<()>,
}

impl Store {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let dir = data_dir.join("users");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self {
            dir,
            lock: Mutex::new(()),
        })
    }

    fn path(&self, email: &str) -> PathBuf {
        self.dir.join(format!("{}.json", file_key(email)))
    }

    pub async fn load(&self, email: &str) -> Result<UserData> {
        let _guard = self.lock.lock().await;
        self.read(email)
    }

    fn read(&self, email: &str) -> Result<UserData> {
        let path = self.path(email);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(UserData::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub async fn set_buttons(&self, email: &str, buttons: Vec<Button>) -> Result<Vec<Button>> {
        let mut buttons = buttons;
        for (i, button) in buttons.iter_mut().enumerate() {
            button.name = button.name.trim().to_string();
            if button.name.is_empty() || button.command.trim().is_empty() {
                bail!("button {}: name and command are required", i + 1);
            }
            if button.id.is_empty() {
                button.id = format!("b{}", rand::random::<u32>());
            }
        }
        let _guard = self.lock.lock().await;
        let mut data = self.read(email)?;
        data.buttons = buttons;
        let path = self.path(email);
        let temp_path = path.with_extension("json.tmp");
        std::fs::write(&temp_path, serde_json::to_vec_pretty(&data)?)
            .with_context(|| format!("writing {}", temp_path.display()))?;
        std::fs::rename(&temp_path, &path)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(data.buttons)
    }
}

/// Encodes an email as a filename without collisions or path separators.
fn file_key(email: &str) -> String {
    let mut encoded = String::with_capacity(email.len());
    for byte in email.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-' || byte == b'@' {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("_{byte:02x}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_key() {
        assert_eq!(super::file_key("a.b@x.io"), "a.b@x.io");
        assert_eq!(super::file_key("a_b@x.io"), "a_5fb@x.io");
        assert_eq!(super::file_key("../x"), ".._2fx");
    }
}
