use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use base64::Engine as _;
use rmpv::Value;
use crate::error::LauncherError;
use crate::steam;

const STYLE_ALPHA: &str = "177670";
const STYLE_BLUE: &str = "177671";
const STYLE_GREEN: &str = "177676";
const STYLE_RED: &str = "177687";
const STYLE_SELECTION_KEY_1: &str = "2090499946";
const STYLE_SELECTION_KEY_2: &str = "993947594";
const STYLE_SELECTION_KEY_3: &str = "277698370";

const BUILTIN_STYLE_BLUE: &str = include_str!("../builtin-styles/Blue.style");
const BUILTIN_STYLE_BLACK: &str = include_str!("../builtin-styles/Black.style");
const BUILTIN_STYLE_LIGHT: &str = include_str!("../builtin-styles/Light.style");

#[derive(Debug, Deserialize)]
pub struct CloudState {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub type7_blob: Option<String>,
    #[serde(default)]
    pub last_loaded_config_id: Option<i32>,
    pub last_loaded_style_id: Option<i32>,
    pub log: Vec<LogEntry>,
}

#[derive(Debug, Deserialize)]
pub struct LogEntry {
    pub entry_id: i32,
    pub entry_type: String,
    pub name: String,
    #[serde(default)]
    pub deleted_at: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThemeColor {
    pub css: String,
    pub hex: String,
    pub alpha: f64,
}

#[derive(Debug, Serialize)]
pub struct LauncherTheme {
    pub source: String,
    pub variables: HashMap<String, String>,
}

pub async fn load_launcher_theme() -> Result<LauncherTheme, LauncherError> {
    let cloud = nl_cloud_path()?;
    let state_path = cloud.join("state.json");
    let state_text = tokio::fs::read_to_string(&state_path)
        .await
        .map_err(|error| LauncherError::Io(format!("failed to read {}: {error}", state_path.display())))?;
    let state: CloudState = serde_json::from_str(&state_text)
        .map_err(|error| LauncherError::SerdeJson(format!("failed to parse {}: {error}", state_path.display())))?;

    let style_id = state
        .type7_blob
        .as_deref()
        .and_then(extract_style_id_from_type7)
        .or(state.last_loaded_style_id);

    let Some(style_id) = style_id else {
        return Ok(default_theme("built-in style"));
    };

    if let Some((name, style_text)) = builtin_style(style_id) {
        let colors = decode_style_colors(style_text)?;
        return Ok(LauncherTheme {
            source: format!("built-in {name} style"),
            variables: build_launcher_variables(&colors),
        });
    }

    let Some(entry) = state.log.iter().find(|entry| {
        entry.entry_id == style_id && entry.entry_type == "Style" && entry.deleted_at.is_none()
    }) else {
        return Ok(default_theme("style not found"));
    };

    let style_path = cloud.join("styles").join(format!(
        "{}_{}.style",
        entry.entry_id,
        sanitize_filename(&entry.name)
    ));
    let style_text = tokio::fs::read_to_string(&style_path)
        .await
        .map_err(|error| LauncherError::Io(format!("failed to read {}: {error}", style_path.display())))?;
    let colors = decode_style_colors(&style_text)?;

    Ok(LauncherTheme {
        source: style_path.display().to_string(),
        variables: build_launcher_variables(&colors),
    })
}

pub fn default_theme(source: &str) -> LauncherTheme {
    LauncherTheme {
        source: source.to_string(),
        variables: HashMap::new(),
    }
}

pub fn nl_cloud_path() -> Result<PathBuf, LauncherError> {
    if let Ok(path) = std::env::var("NL_CLOUD_PATH") {
        return Ok(PathBuf::from(path));
    }

    launcher_cloud_dir()
}

pub fn launcher_cloud_dir() -> Result<PathBuf, LauncherError> {
    if let Some(game_dir) = steam::find_game_install_path("Counter-Strike Global Offensive") {
        let cloud_path = game_dir.join("nl_cloud");
        if cloud_path.exists() {
            return Ok(cloud_path);
        }
    }
    if let Some(game_dir) = steam::find_game_install_path("csgo legacy") {
        let cloud_path = game_dir.join("nl_cloud");
        if cloud_path.exists() {
            return Ok(cloud_path);
        }
    }
    if let Some(game_dir) = steam::find_game_install_path("Counter-Strike Global Offensive") {
        return Ok(game_dir.join("nl_cloud"));
    }
    if let Some(game_dir) = steam::find_game_install_path("csgo legacy") {
        return Ok(game_dir.join("nl_cloud"));
    }

    let appdata =
        std::env::var("APPDATA").map_err(|error| LauncherError::System(format!("APPDATA is not available: {error}")))?;
    Ok(Path::new(&appdata).join("neverlose").join("nl_cloud"))
}

fn builtin_style(style_id: i32) -> Option<(&'static str, &'static str)> {
    match style_id {
        0 => Some(("Blue", BUILTIN_STYLE_BLUE)),
        1 => Some(("Black", BUILTIN_STYLE_BLACK)),
        2 => Some(("Light", BUILTIN_STYLE_LIGHT)),
        _ => None,
    }
}

fn extract_style_id_from_type7(blob_b64: &str) -> Option<i32> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob_b64.trim())
        .ok()?;
    let value = rmpv::decode::read_value(&mut &bytes[..]).ok()?;
    let selected = map_get(&value, STYLE_SELECTION_KEY_1)
        .and_then(|value| map_get(value, STYLE_SELECTION_KEY_2))
        .and_then(|value| map_get(value, STYLE_SELECTION_KEY_3))?;

    selected
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
}

fn map_get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let Value::Map(entries) = value else {
        return None;
    };

    entries.iter().find_map(|(entry_key, entry_value)| {
        if entry_key.as_str()? == key {
            Some(entry_value)
        } else {
            None
        }
    })
}

fn decode_style_colors(style_text: &str) -> Result<Vec<ThemeColor>, LauncherError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(style_text.trim())?;
    let value = rmpv::decode::read_value(&mut &bytes[..])?;
    let Value::Array(items) = value else {
        return Err(LauncherError::Validation("style payload was not an array".to_string()));
    };

    items
        .iter()
        .map(decode_style_color)
        .collect::<Result<Vec<_>, _>>()
}

fn decode_style_color(value: &Value) -> Result<ThemeColor, LauncherError> {
    let Value::Map(entries) = value else {
        return Err(LauncherError::Validation("style color entry was not a map".to_string()));
    };

    let alpha = style_channel(entries, STYLE_ALPHA)?;
    let red = style_channel(entries, STYLE_RED)?;
    let green = style_channel(entries, STYLE_GREEN)?;
    let blue = style_channel(entries, STYLE_BLUE)?;
    let red_u8 = channel_to_u8(red);
    let green_u8 = channel_to_u8(green);
    let blue_u8 = channel_to_u8(blue);
    let alpha_u8 = channel_to_u8(alpha);

    Ok(ThemeColor {
        css: format!(
            "rgba({}, {}, {}, {:.3})",
            red_u8,
            green_u8,
            blue_u8,
            alpha.clamp(0.0, 1.0)
        ),
        hex: format!("#{red_u8:02X}{green_u8:02X}{blue_u8:02X}{alpha_u8:02X}"),
        alpha,
    })
}

fn style_channel(entries: &[(Value, Value)], key: &str) -> Result<f64, LauncherError> {
    entries
        .iter()
        .find_map(|(entry_key, value)| {
            let entry_key = entry_key.as_str()?;
            if entry_key == key {
                value.as_f64()
            } else {
                None
            }
        })
        .ok_or_else(|| LauncherError::Validation(format!("style color entry was missing channel {key}")))
}

fn channel_to_u8(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn color(colors: &[ThemeColor], index: usize, fallback: &str) -> String {
    colors
        .get(index)
        .map(|color| color.css.clone())
        .unwrap_or_else(|| fallback.to_string())
}

fn alpha_color(colors: &[ThemeColor], index: usize, fallback: &str, alpha: f64) -> String {
    let Some(color) = colors.get(index) else {
        return fallback.to_string();
    };

    let hex = color.hex.trim_start_matches('#');
    if hex.len() < 6 {
        return fallback.to_string();
    }

    let Ok(red) = u8::from_str_radix(&hex[0..2], 16) else {
        return fallback.to_string();
    };
    let Ok(green) = u8::from_str_radix(&hex[2..4], 16) else {
        return fallback.to_string();
    };
    let Ok(blue) = u8::from_str_radix(&hex[4..6], 16) else {
        return fallback.to_string();
    };

    format!("rgba({red}, {green}, {blue}, {:.3})", alpha.clamp(0.0, 1.0))
}

fn opaque_color(colors: &[ThemeColor], index: usize, fallback: &str) -> String {
    alpha_color(colors, index, fallback, 1.0)
}

pub fn sanitize_filename(name: &str) -> String {
    let value: String = name
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect();
    let value = value.trim_end_matches(|character: char| {
        character == '.' || character == ' ' || character.is_control()
    });
    if value.is_empty() {
        "unnamed".to_string()
    } else {
        value.to_string()
    }
}

pub fn sanitize_path_segment(name: &str) -> String {
    let value = sanitize_filename(name);
    if value == "." || value == ".." {
        "version".to_string()
    } else {
        value
    }
}

pub fn build_launcher_variables(colors: &[ThemeColor]) -> HashMap<String, String> {
    let mut variables = HashMap::new();

    variables.insert(
        "--nl-text".to_string(),
        color(colors, 0, "rgba(255, 255, 255, 0.88)"),
    );
    variables.insert(
        "--nl-disabled-text".to_string(),
        color(colors, 1, "rgba(255, 255, 255, 0.38)"),
    );
    variables.insert("--nl-active-text".to_string(), color(colors, 2, "#FFFFFF"));
    variables.insert(
        "--nl-small-text".to_string(),
        color(colors, 3, "rgba(255, 255, 255, 0.575)"),
    );
    variables.insert(
        "--nl-sidebar-text".to_string(),
        color(colors, 4, "rgba(255, 255, 255, 0.589)"),
    );
    variables.insert("--nl-logo".to_string(), color(colors, 5, "white"));
    variables.insert("--nl-sidebar-bg".to_string(), color(colors, 6, "#010306"));
    variables.insert(
        "--nl-popup-bg".to_string(),
        color(colors, 8, "rgba(7, 12, 19, 0.65)"),
    );
    variables.insert("--nl-main-bg".to_string(), color(colors, 9, "#010306"));
    variables.insert(
        "--nl-main-bg-opaque".to_string(),
        opaque_color(colors, 9, "#010306"),
    );
    variables.insert("--nl-preview-bg".to_string(), color(colors, 10, "#03080f"));
    variables.insert(
        "--nl-border".to_string(),
        color(colors, 11, "rgba(255, 255, 255, 0.075)"),
    );
    variables.insert("--nl-frame-bg".to_string(), color(colors, 12, "#03080f"));
    variables.insert(
        "--nl-frame-active-bg".to_string(),
        color(colors, 13, "rgba(7, 12, 19, 0.72)"),
    );
    variables.insert(
        "--nl-text-preview".to_string(),
        color(colors, 14, "rgba(247, 245, 255, 0.8)"),
    );
    variables.insert(
        "--nl-window-title-bg".to_string(),
        color(colors, 15, "rgba(4, 8, 13, 0.96)"),
    );
    variables.insert(
        "--nl-active-window-title".to_string(),
        color(colors, 16, "rgba(255, 255, 255, 0.88)"),
    );
    variables.insert("--nl-spinner".to_string(), color(colors, 40, "#aab6ff"));
    variables.insert("--nl-block-bg".to_string(), color(colors, 41, "#03080f"));
    variables.insert(
        "--nl-block-bg-opaque".to_string(),
        opaque_color(colors, 41, "#03080f"),
    );
    variables.insert(
        "--nl-sidebar-selection".to_string(),
        color(colors, 42, "rgba(255, 255, 255, 0.08)"),
    );
    variables.insert(
        "--nl-logo-back".to_string(),
        color(colors, 44, "rgba(255, 255, 255, 0.18)"),
    );
    variables.insert("--nl-button".to_string(), color(colors, 28, "#3f66f5"));
    variables.insert(
        "--nl-button-active".to_string(),
        color(colors, 29, "#486cee"),
    );
    variables.insert(
        "--nl-button-active-text".to_string(),
        color(colors, 30, "rgba(255, 255, 255, 0.9)"),
    );
    variables.insert("--nl-link".to_string(), color(colors, 31, "#626be6"));
    variables.insert("--nl-link-active".to_string(), color(colors, 32, "white"));
    variables.insert(
        "--nl-selection".to_string(),
        color(colors, 34, "rgba(16, 31, 49, 0.78)"),
    );
    variables.insert(
        "--nl-separator".to_string(),
        color(colors, 35, "rgba(255, 255, 255, 0.045)"),
    );
    variables.insert(
        "--nl-shadow".to_string(),
        color(colors, 39, "rgba(0, 0, 0, 0.52)"),
    );
    variables.insert(
        "--nl-shadow-soft".to_string(),
        alpha_color(colors, 39, "rgba(0, 0, 0, 0.48)", 0.48),
    );

    variables
}

#[derive(Debug, Serialize)]
pub struct LauncherSettings {
    pub username: String,
    pub avatar_data_url: Option<String>,
    pub selected_config_id: Option<i32>,
    pub configs: Vec<ConfigEntry>,
}

#[derive(Debug, Serialize)]
pub struct ConfigEntry {
    pub entry_id: i32,
    pub name: String,
}

pub async fn read_launcher_settings() -> Result<LauncherSettings, LauncherError> {
    let cloud = nl_cloud_path()?;
    let state_path = cloud.join("state.json");
    let state_text = tokio::fs::read_to_string(&state_path)
        .await
        .map_err(|error| LauncherError::Io(format!("failed to read {}: {error}", state_path.display())))?;
    let state: CloudState = serde_json::from_str(&state_text)
        .map_err(|error| LauncherError::SerdeJson(format!("failed to parse {}: {error}", state_path.display())))?;

    let configs = state
        .log
        .iter()
        .filter(|entry| entry.entry_type == "Config" && entry.deleted_at.is_none())
        .map(|entry| ConfigEntry {
            entry_id: entry.entry_id,
            name: entry.name.clone(),
        })
        .collect::<Vec<_>>();

    Ok(LauncherSettings {
        username: state.username,
        avatar_data_url: load_avatar_data_url(&cloud).await?,
        selected_config_id: state.last_loaded_config_id,
        configs,
    })
}

async fn load_avatar_data_url(cloud: &Path) -> Result<Option<String>, LauncherError> {
    let avatar_path = cloud.join("avatar.png");
    if !avatar_path.exists() {
        return Ok(None);
    }

    let bytes = tokio::fs::read(&avatar_path)
        .await
        .map_err(|error| LauncherError::Io(format!("failed to read {}: {error}", avatar_path.display())))?;
    let mime = image_mime_type(&bytes).unwrap_or("image/png");
    Ok(Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
}

fn image_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

pub async fn save_launcher_profile(
    username: String,
    avatar_bytes: Option<Vec<u8>>,
) -> Result<LauncherSettings, LauncherError> {
    let username = username.trim();
    if username.is_empty() {
        return Err(LauncherError::Validation("profile name cannot be empty".to_string()));
    }

    let cloud = nl_cloud_path()?;
    tokio::fs::create_dir_all(&cloud)
        .await
        .map_err(|error| LauncherError::Io(format!("failed to create {}: {error}", cloud.display())))?;

    let state_path = cloud.join("state.json");
    let state_text = tokio::fs::read_to_string(&state_path)
        .await
        .map_err(|error| LauncherError::Io(format!("failed to read {}: {error}", state_path.display())))?;
    let mut state_value: serde_json::Value = serde_json::from_str(&state_text)
        .map_err(|error| LauncherError::SerdeJson(format!("failed to parse {}: {error}", state_path.display())))?;
    let Some(state_object) = state_value.as_object_mut() else {
        return Err(LauncherError::Validation("state.json root was not an object".to_string()));
    };
    state_object.insert(
        "username".to_string(),
        serde_json::Value::String(username.to_string()),
    );
    let next_state = serde_json::to_string_pretty(&state_value)
        .map_err(|error| LauncherError::SerdeJson(format!("failed to serialize {}: {error}", state_path.display())))?;
    tokio::fs::write(&state_path, next_state)
        .await
        .map_err(|error| LauncherError::Io(format!("failed to write {}: {error}", state_path.display())))?;

    if let Some(bytes) = avatar_bytes {
        if bytes.is_empty() {
            return Err(LauncherError::Validation("profile image was empty".to_string()));
        }
        let avatar_path = cloud.join("avatar.png");
        tokio::fs::write(&avatar_path, bytes)
            .await
            .map_err(|error| LauncherError::Io(format!("failed to write {}: {error}", avatar_path.display())))?;
    }

    read_launcher_settings().await
}
