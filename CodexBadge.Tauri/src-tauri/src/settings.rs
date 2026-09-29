use crate::domain::Settings;
use std::fs;
use std::path::PathBuf;
use winreg::{enums::HKEY_CURRENT_USER, RegKey};

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("CodexBadge/tauri-settings.json"))
}

pub fn load() -> Settings {
    path()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
        .filter(|settings| settings.offset_x.is_finite() && settings.offset_y.is_finite())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let path = path().ok_or("LOCALAPPDATA missing")?;
    fs::create_dir_all(path.parent().ok_or("settings directory missing")?)
        .map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    fs::write(
        &temp,
        serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::rename(temp, path).map_err(|e| e.to_string())
}

pub fn set_startup(enabled: bool) -> Result<(), String> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")
        .map_err(|e| e.to_string())?
        .0;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        key.set_value("CodexBadgeTauri", &format!("\"{}\"", exe.display()))
            .map_err(|e| e.to_string())
    } else {
        match key.delete_value("CodexBadgeTauri") {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_round_trip_keeps_glass_default_theme() {
        let source = Settings {
            offset_x: 18.5,
            offset_y: -7.0,
            ..Settings::default()
        };
        let value = serde_json::to_vec(&source).unwrap();
        let decoded: Settings = serde_json::from_slice(&value).unwrap();
        assert_eq!(decoded.theme, crate::domain::Theme::Glass);
        assert_eq!((decoded.offset_x, decoded.offset_y), (18.5, -7.0));
        assert!(!decoded.always_on_top);
        let legacy = br#"{"theme":"system","offsetX":0.0,"offsetY":0.0,"startWithWindows":false}"#;
        let migrated: Settings = serde_json::from_slice(legacy).unwrap();
        assert_eq!(migrated.theme, crate::domain::Theme::System);
        assert!(!migrated.always_on_top);
    }
}
