//! Where a game's save lives, its backups, and cloud saves through a synced folder - the app's SavesLocation,
//! SaveBackups and SyncedSaves (app/.../SyncedSaves.kt), in Rust. Keep the rules in step.
//!
//! Cloud saves: the saves folder sits inside OneDrive, Dropbox, Google Drive or iCloud Drive and their own
//! client carries the files between devices. On top of a plain folder this adds a `<save>.playing` marker
//! (another device opening the same game warns first) and the pick between the copies a sync client makes
//! when two devices saved anyway.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The Android app's name for it (SaveBackups.kt): one folder of backups for a save both devices play.
pub const BACKUP_DIR: &str = "pokedaisy-backups";
pub const BACKUPS_KEPT: usize = 10;
/// A save bigger than this isn't a save (the biggest GBA chip is 128 KB; mGBA adds a 16-byte RTC footer).
const MAX_SAVE_BYTES: u64 = 1 << 20;

/// `rom`'s save in `dir`: an existing `.sav`, else an existing `.srm` (RetroArch's), else a new `.sav`.
pub fn save_for(dir: &Path, rom: &Path) -> PathBuf {
    let base = rom
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sav = dir.join(format!("{base}.sav"));
    if sav.exists() {
        return sav;
    }
    let srm = dir.join(format!("{base}.srm"));
    if srm.exists() {
        return srm;
    }
    sav
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Same save data, ignoring mGBA's 16-byte RTC footer (every save chip's size is a multiple of 512).
fn same_save(a: &[u8], b: &[u8]) -> bool {
    let n = a.len() - a.len() % 512;
    if n == 0 || n != b.len() - b.len() % 512 {
        return a == b;
    }
    a[..n] == b[..n]
}

/// `save`'s backups, newest first.
pub fn backups(save: &Path) -> Vec<PathBuf> {
    let Some(dir) = save.parent().map(|p| p.join(BACKUP_DIR)) else {
        return Vec::new();
    };
    let prefix = format!(
        "{}.backup-",
        save.file_stem().unwrap_or_default().to_string_lossy()
    );
    let mut list: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
        })
        .collect();
    list.sort();
    list.reverse();
    list
}

/// Backs `save` up beside it (`pokedaisy-backups/<game>.backup-<stamp>.<ext>`) if it has data the newest
/// backup doesn't, keeping the newest [`BACKUPS_KEPT`]. Never fails the launch: errors are only logged.
pub fn backup(save: &Path) -> Option<PathBuf> {
    let meta = fs::metadata(save).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_SAVE_BYTES {
        return None;
    }
    let bytes = fs::read(save).ok()?;
    let existing = backups(save);
    if existing
        .first()
        .and_then(|b| fs::read(b).ok())
        .is_some_and(|b| same_save(&b, &bytes))
    {
        return None;
    }
    let dir = save.parent()?.join(BACKUP_DIR);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let stem = save.file_stem()?.to_string_lossy();
    let ext = save
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sav".into());
    let out = dir.join(format!("{stem}.backup-{stamp}.{ext}"));
    let result = (|| -> std::io::Result<()> {
        fs::create_dir_all(&dir)?;
        write_atomically(&out, &bytes)
    })();
    if let Err(e) = result {
        eprintln!("save backup failed: {e}");
        return None;
    }
    for old in existing.into_iter().skip(BACKUPS_KEPT - 1) {
        let _ = fs::remove_file(old);
    }
    Some(out)
}

/// Writes `bytes` to `path` through a temporary file beside it, so a crash never leaves half a file.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).or_else(|_| {
        let _ = fs::remove_file(path);
        fs::rename(&tmp, path)
    })
}

// --- synced folders -----------------------------------------------------------------------------------

/// A sync client's folder: `name` for the player ("OneDrive"), `root` the folder it syncs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudFolder {
    pub name: &'static str,
    pub root: PathBuf,
}

impl CloudFolder {
    /// Where Adventure Pad keeps its saves inside it.
    pub fn saves_dir(&self) -> PathBuf {
        self.root.join("AdventurePad").join("saves")
    }
}

fn norm(p: &Path) -> String {
    p.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// The sync folders found on this machine, most likely first.
pub fn cloud_folders() -> Vec<CloudFolder> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let home = env("USERPROFILE")
        .or_else(|| env("HOME"))
        .unwrap_or_default();
    let mut out = Vec::new();
    // OneDrive sets these for the signed-in accounts (personal, then work / school).
    for k in ["OneDriveConsumer", "OneDrive", "OneDriveCommercial"] {
        if let Some(p) = env(k) {
            out.push(CloudFolder {
                name: "OneDrive",
                root: p,
            });
        }
    }
    out.push(CloudFolder {
        name: "OneDrive",
        root: home.join("OneDrive"),
    });
    for info in [env("LOCALAPPDATA"), env("APPDATA")]
        .into_iter()
        .flatten()
        .map(|d| d.join("Dropbox/info.json"))
    {
        if let Some(root) = dropbox_root(&info) {
            out.push(CloudFolder {
                name: "Dropbox",
                root,
            });
        }
    }
    out.push(CloudFolder {
        name: "Dropbox",
        root: home.join("Dropbox"),
    });
    // Google Drive for desktop mounts a drive letter with "My Drive" in it.
    for letter in b'D'..=b'Z' {
        out.push(CloudFolder {
            name: "Google Drive",
            root: PathBuf::from(format!("{}:\\My Drive", letter as char)),
        });
    }
    out.push(CloudFolder {
        name: "Google Drive",
        root: home.join("Google Drive").join("My Drive"),
    });
    out.push(CloudFolder {
        name: "iCloud Drive",
        root: home.join("iCloudDrive"),
    });
    let mut seen = std::collections::HashSet::new();
    out.retain(|c| c.root.is_dir() && seen.insert(norm(&c.root)));
    out
}

/// Dropbox writes where its folder is to info.json ({"personal": {"path": ...}}).
fn dropbox_root(info: &Path) -> Option<PathBuf> {
    let json: serde_json::Value = serde_json::from_slice(&fs::read(info).ok()?).ok()?;
    ["personal", "business"].iter().find_map(|k| {
        json.get(k)?
            .get("path")?
            .as_str()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    })
}

/// The sync client `dir` is in, or None for a folder that isn't synced (as far as we can tell).
pub fn cloud_for<'a>(dir: &Path, folders: &'a [CloudFolder]) -> Option<&'a CloudFolder> {
    let d = norm(dir);
    folders.iter().find(|c| {
        let r = norm(&c.root);
        d == r || d.starts_with(&format!("{r}\\"))
    })
}

/// Copies a sync client made of `save` when two devices wrote it: the save's name plus OneDrive's
/// `-<COMPUTERNAME>` (upper case: a lower-case suffix is another game's save, Emerald / Emerald-Legacy),
/// Dropbox's ` (... conflicted copy ...)` or Google Drive's ` (1)`, same extension. Newest first.
pub fn conflict_copies(save: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem), Some(ext)) = (save.parent(), save.file_stem(), save.extension())
    else {
        return Vec::new();
    };
    let stem = stem.to_string_lossy();
    let ext = format!(".{}", ext.to_string_lossy());
    let is_copy = |name: &str| -> bool {
        let Some(rest) = name
            .strip_prefix(stem.as_ref())
            .and_then(|r| r.strip_suffix(ext.as_str()))
        else {
            return false;
        };
        if let Some(dev) = rest.strip_prefix('-') {
            let mut chars = dev.chars();
            return dev.len() >= 3
                && chars
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                && dev
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-');
        }
        if let Some(inner) = rest.strip_prefix(" (").and_then(|r| r.strip_suffix(')')) {
            return inner.contains("conflicted copy")
                || (!inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()));
        }
        false
    };
    let mut copies: Vec<(SystemTime, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.path() != save && e.path().is_file() && is_copy(&e.file_name().to_string_lossy())
        })
        .map(|e| {
            (
                e.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(UNIX_EPOCH),
                e.path(),
            )
        })
        .collect();
    copies.sort_by_key(|c| std::cmp::Reverse(c.0));
    copies.into_iter().map(|(_, p)| p).collect()
}

/// Puts `copy` in `save`'s place, backing the current save up first, and removes the copy.
pub fn use_copy(save: &Path, copy: &Path) -> std::io::Result<()> {
    if save.is_file() {
        backup(save);
    }
    let bytes = fs::read(copy)?;
    write_atomically(save, &bytes)?;
    fs::remove_file(copy)
}

/// How often an open game says it's still open, and when a marker counts as left behind (a crash, a lost
/// connection).
pub const HEARTBEAT: Duration = Duration::from_secs(60);
pub const STALE: Duration = Duration::from_secs(180);

/// Another device holding a save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub device: String,
    /// Milliseconds since 1970 of its last heartbeat.
    pub heartbeat: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct MarkerJson {
    device: String,
    id: String,
    heartbeat: u64,
}

/// `<save>.playing`: which device has this save open, and when it last said so (the app's PlayMarker, the
/// same JSON, so Android and Windows read each other's).
pub struct PlayMarker {
    file: PathBuf,
    device: String,
    device_id: String,
}

impl PlayMarker {
    pub fn new(save: &Path, device: &str, device_id: &str) -> PlayMarker {
        let mut name = save.file_name().unwrap_or_default().to_os_string();
        name.push(".playing");
        PlayMarker {
            file: save.with_file_name(name),
            device: device.into(),
            device_id: device_id.into(),
        }
    }

    fn read(&self) -> Option<MarkerJson> {
        serde_json::from_slice(&fs::read(&self.file).ok()?).ok()
    }

    /// Another device's fresh marker on this save, or None (none, our own, or stale).
    pub fn other_device(&self) -> Option<Holder> {
        let m = self.read()?;
        if m.id == self.device_id
            || now_millis().saturating_sub(m.heartbeat) > STALE.as_millis() as u64
        {
            return None;
        }
        Some(Holder {
            device: m.device,
            heartbeat: m.heartbeat,
        })
    }

    pub fn touch(&self) {
        let m = MarkerJson {
            device: self.device.clone(),
            id: self.device_id.clone(),
            heartbeat: now_millis(),
        };
        if let Ok(json) = serde_json::to_vec(&m) {
            let _ = write_atomically(&self.file, &json);
        }
    }

    /// Removes the marker if it's still ours (another device may have taken over meanwhile).
    pub fn clear(&self) {
        if self.read().is_none_or(|m| m.id == self.device_id) {
            let _ = fs::remove_file(&self.file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_an_srm_but_prefers_a_sav() {
        let d = tempfile::tempdir().unwrap();
        let rom = Path::new("C:/roms/Emerald.gba");
        assert_eq!(save_for(d.path(), rom), d.path().join("Emerald.sav"));
        fs::write(d.path().join("Emerald.srm"), b"x").unwrap();
        assert_eq!(save_for(d.path(), rom), d.path().join("Emerald.srm"));
        fs::write(d.path().join("Emerald.sav"), b"x").unwrap();
        assert_eq!(save_for(d.path(), rom), d.path().join("Emerald.sav"));
    }

    #[test]
    fn backs_up_only_new_data() {
        let d = tempfile::tempdir().unwrap();
        let save = d.path().join("FireRed.sav");
        fs::write(&save, vec![1u8; 1024]).unwrap();
        assert!(backup(&save).is_some());
        assert!(backup(&save).is_none(), "the same data twice");
        // A changed RTC footer alone isn't new data.
        let mut with_clock = vec![1u8; 1024];
        with_clock.extend_from_slice(&[9u8; 16]);
        fs::write(&save, &with_clock).unwrap();
        assert!(backup(&save).is_none());
        assert_eq!(backups(&save).len(), 1);
    }

    #[test]
    fn tells_conflict_copies_from_other_games() {
        let d = tempfile::tempdir().unwrap();
        let save = d.path().join("Emerald.sav");
        for n in [
            "Emerald.sav",
            "Emerald-DESKTOP-4F2K9QX.sav",
            "Emerald (Wolf's conflicted copy 2026-10-21).sav",
            "Emerald (1).sav",
            "Emerald-Legacy.sav",
            "Emerald Rogue.sav",
            "Emerald-DESKTOP-4F2K9QX.srm",
        ] {
            fs::write(d.path().join(n), b"x").unwrap();
        }
        let mut names: Vec<String> = conflict_copies(&save)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "Emerald (1).sav",
                "Emerald (Wolf's conflicted copy 2026-10-21).sav",
                "Emerald-DESKTOP-4F2K9QX.sav"
            ]
        );
    }

    #[test]
    fn markers_warn_only_about_other_fresh_devices() {
        let d = tempfile::tempdir().unwrap();
        let save = d.path().join("FireRed.sav");
        let here = PlayMarker::new(&save, "PC", "a");
        let phone = PlayMarker::new(&save, "Phone", "b");
        assert_eq!(here.other_device(), None);
        phone.touch();
        assert_eq!(
            here.other_device().map(|h| h.device),
            Some("Phone".to_owned())
        );
        assert_eq!(phone.other_device(), None, "its own marker");
        here.clear();
        assert!(here.other_device().is_some(), "not ours to clear");
        phone.clear();
        assert_eq!(here.other_device(), None);
    }

    #[test]
    fn a_folder_inside_a_sync_root_is_synced() {
        let folders = [CloudFolder {
            name: "OneDrive",
            root: PathBuf::from("C:\\Users\\me\\OneDrive"),
        }];
        assert!(cloud_for(
            Path::new("C:/Users/me/OneDrive/AdventurePad/saves"),
            &folders
        )
        .is_some());
        assert!(cloud_for(Path::new("C:\\Users\\me\\OneDriveOther"), &folders).is_none());
        assert!(cloud_for(Path::new("C:\\Users\\me\\Documents"), &folders).is_none());
    }
}
