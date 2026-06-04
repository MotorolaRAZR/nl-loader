use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use rmpv::Value;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

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
struct CloudState {
    #[serde(default)]
    username: String,
    #[serde(default)]
    type7_blob: Option<String>,
    #[serde(default)]
    last_loaded_config_id: Option<i32>,
    last_loaded_style_id: Option<i32>,
    log: Vec<LogEntry>,
}

#[derive(Debug, Deserialize)]
struct LogEntry {
    entry_id: i32,
    entry_type: String,
    name: String,
    #[serde(default)]
    deleted_at: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
struct ThemeColor {
    css: String,
    hex: String,
    alpha: f64,
}

#[derive(Debug, Serialize)]
struct LauncherTheme {
    source: String,
    variables: HashMap<String, String>,
}

#[derive(Debug, Serialize)]
struct LauncherSettings {
    username: String,
    selected_config_id: Option<i32>,
    selected_config_name: Option<String>,
    configs: Vec<ConfigEntry>,
}

#[derive(Debug, Serialize)]
struct ConfigEntry {
    entry_id: i32,
    name: String,
}

#[tauri::command]
fn load_launcher_theme() -> Result<LauncherTheme, String> {
    let cloud = nl_cloud_path()?;
    let state_path = cloud.join("state.json");
    let state_text = fs::read_to_string(&state_path)
        .map_err(|error| format!("failed to read {}: {error}", state_path.display()))?;
    let state: CloudState = serde_json::from_str(&state_text)
        .map_err(|error| format!("failed to parse {}: {error}", state_path.display()))?;

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

    let style_path = cloud
        .join("styles")
        .join(format!("{}_{}.style", entry.entry_id, sanitize_filename(&entry.name)));
    let style_text = fs::read_to_string(&style_path)
        .map_err(|error| format!("failed to read {}: {error}", style_path.display()))?;
    let colors = decode_style_colors(&style_text)?;

    Ok(LauncherTheme {
        source: style_path.display().to_string(),
        variables: build_launcher_variables(&colors),
    })
}

#[tauri::command]
fn minimize_main_window(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window was not found".to_string())?;
    window
        .minimize()
        .map_err(|error| format!("failed to minimize main window: {error}"))
}

#[tauri::command]
fn close_main_window(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window was not found".to_string())?;
    window
        .close()
        .map_err(|error| format!("failed to close main window: {error}"))
}

#[tauri::command]
fn load_launcher_settings() -> Result<LauncherSettings, String> {
    let cloud = nl_cloud_path()?;
    let state_path = cloud.join("state.json");
    let state_text = fs::read_to_string(&state_path)
        .map_err(|error| format!("failed to read {}: {error}", state_path.display()))?;
    let state: CloudState = serde_json::from_str(&state_text)
        .map_err(|error| format!("failed to parse {}: {error}", state_path.display()))?;

    let configs = state
        .log
        .iter()
        .filter(|entry| entry.entry_type == "Config" && entry.deleted_at.is_none())
        .map(|entry| ConfigEntry {
            entry_id: entry.entry_id,
            name: entry.name.clone(),
        })
        .collect::<Vec<_>>();

    let selected_config_name = state
        .last_loaded_config_id
        .and_then(|id| configs.iter().find(|config| config.entry_id == id))
        .map(|config| config.name.clone())
        .or_else(|| configs.first().map(|config| config.name.clone()));

    Ok(LauncherSettings {
        username: state.username,
        selected_config_id: state.last_loaded_config_id,
        selected_config_name,
        configs,
    })
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

    selected.as_i64().and_then(|value| i32::try_from(value).ok())
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

fn nl_cloud_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("NL_CLOUD_PATH") {
        return Ok(PathBuf::from(path));
    }

    let user_profile = std::env::var("USERPROFILE")
        .map_err(|error| format!("USERPROFILE is not available: {error}"))?;
    Ok(Path::new(&user_profile)
        .join("Documents")
        .join("neverlose")
        .join("nl_cloud"))
}

fn decode_style_colors(style_text: &str) -> Result<Vec<ThemeColor>, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(style_text.trim())
        .map_err(|error| format!("style base64 decode failed: {error}"))?;
    let value = rmpv::decode::read_value(&mut &bytes[..])
        .map_err(|error| format!("style MessagePack decode failed: {error}"))?;
    let Value::Array(items) = value else {
        return Err("style payload was not an array".to_string());
    };

    items
        .iter()
        .map(decode_style_color)
        .collect::<Result<Vec<_>, _>>()
}

fn decode_style_color(value: &Value) -> Result<ThemeColor, String> {
    let Value::Map(entries) = value else {
        return Err("style color entry was not a map".to_string());
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

fn style_channel(entries: &[(Value, Value)], key: &str) -> Result<f64, String> {
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
        .ok_or_else(|| format!("style color entry was missing channel {key}"))
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

fn build_launcher_variables(colors: &[ThemeColor]) -> HashMap<String, String> {
    let mut variables = HashMap::new();

    variables.insert("--nl-text".to_string(), color(colors, 0, "rgba(255, 255, 255, 0.88)"));
    variables.insert("--nl-disabled-text".to_string(), color(colors, 1, "rgba(255, 255, 255, 0.38)"));
    variables.insert("--nl-active-text".to_string(), color(colors, 2, "#FFFFFF"));
    variables.insert("--nl-small-text".to_string(), color(colors, 3, "rgba(255, 255, 255, 0.575)"));
    variables.insert("--nl-sidebar-text".to_string(), color(colors, 4, "rgba(255, 255, 255, 0.589)"));
    variables.insert("--nl-logo".to_string(), color(colors, 5, "white"));
    variables.insert("--nl-sidebar-bg".to_string(), color(colors, 6, "#010306"));
    variables.insert("--nl-popup-bg".to_string(), color(colors, 8, "rgba(7, 12, 19, 0.65)"));
    variables.insert("--nl-main-bg".to_string(), color(colors, 9, "#010306"));
    variables.insert("--nl-main-bg-opaque".to_string(), opaque_color(colors, 9, "#010306"));
    variables.insert("--nl-preview-bg".to_string(), color(colors, 10, "#03080f"));
    variables.insert("--nl-border".to_string(), color(colors, 11, "rgba(255, 255, 255, 0.075)"));
    variables.insert("--nl-frame-bg".to_string(), color(colors, 12, "#03080f"));
    variables.insert("--nl-frame-active-bg".to_string(), color(colors, 13, "rgba(7, 12, 19, 0.72)"));
    variables.insert("--nl-text-preview".to_string(), color(colors, 14, "rgba(247, 245, 255, 0.8)"));
    variables.insert("--nl-window-title-bg".to_string(), color(colors, 15, "rgba(4, 8, 13, 0.96)"));
    variables.insert("--nl-active-window-title".to_string(), color(colors, 16, "rgba(255, 255, 255, 0.88)"));
    variables.insert("--nl-spinner".to_string(), color(colors, 40, "#aab6ff"));
    variables.insert("--nl-block-bg".to_string(), color(colors, 41, "#03080f"));
    variables.insert("--nl-block-bg-opaque".to_string(), opaque_color(colors, 41, "#03080f"));
    variables.insert("--nl-sidebar-selection".to_string(), color(colors, 42, "rgba(255, 255, 255, 0.08)"));
    variables.insert("--nl-logo-back".to_string(), color(colors, 44, "rgba(255, 255, 255, 0.18)"));
    variables.insert("--nl-button".to_string(), color(colors, 28, "#3f66f5"));
    variables.insert("--nl-button-active".to_string(), color(colors, 29, "#486cee"));
    variables.insert("--nl-button-active-text".to_string(), color(colors, 30, "rgba(255, 255, 255, 0.9)"));
    variables.insert("--nl-link".to_string(), color(colors, 31, "#626be6"));
    variables.insert("--nl-link-active".to_string(), color(colors, 32, "white"));
    variables.insert("--nl-selection".to_string(), color(colors, 34, "rgba(16, 31, 49, 0.78)"));
    variables.insert("--nl-separator".to_string(), color(colors, 35, "rgba(255, 255, 255, 0.045)"));
    variables.insert("--nl-shadow".to_string(), color(colors, 39, "rgba(0, 0, 0, 0.52)"));
    variables.insert("--nl-shadow-soft".to_string(), alpha_color(colors, 39, "rgba(0, 0, 0, 0.48)", 0.48));

    variables
}

fn default_theme(source: &str) -> LauncherTheme {
    LauncherTheme {
        source: source.to_string(),
        variables: HashMap::new(),
    }
}

fn sanitize_filename(name: &str) -> String {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            load_launcher_theme,
            load_launcher_settings,
            minimize_main_window,
            close_main_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
