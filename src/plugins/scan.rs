use std::path::{Path, PathBuf};

use super::{clap, vst2, vst3, PluginDescriptor, PluginFormat};

const CLAP_PATH_ENV: &str = "CLAP_PATH";
const MACOS_BUNDLE_BINARY_DIR: &str = "Contents/MacOS";

pub fn scan_all() -> Vec<PluginDescriptor> {
    let mut found = Vec::new();
    for format in [PluginFormat::Clap, PluginFormat::Vst2, PluginFormat::Vst3] {
        for dir in search_dirs(format) {
            scan_dir(&dir, format, &mut found);
        }
    }
    found.sort_by_key(|a| a.name.to_lowercase());
    found
}

fn scan_dir(dir: &Path, format: PluginFormat, found: &mut Vec<PluginDescriptor>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if has_extension(&path, extension_for(format)) {
            found.extend(describe(&path, format));
        } else if path.is_dir() {
            scan_dir(&path, format, found);
        }
    }
}

fn describe(path: &Path, format: PluginFormat) -> Vec<PluginDescriptor> {
    match format {
        PluginFormat::Clap => clap::describe(path).unwrap_or_else(|error| {
            log::warn!("skipping {}: {error}", path.display());
            Vec::new()
        }),
        PluginFormat::Vst2 => vec![vst2::describe(path)],
        PluginFormat::Vst3 => vec![vst3::describe(path)],
    }
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(extension))
}

fn extension_for(format: PluginFormat) -> &'static str {
    match format {
        PluginFormat::Clap => "clap",
        PluginFormat::Vst2 => "vst",
        PluginFormat::Vst3 => "vst3",
    }
}

pub fn search_dirs(format: PluginFormat) -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut dirs = platform_dirs(format, &home);
    if format == PluginFormat::Clap {
        dirs.extend(
            std::env::var_os(CLAP_PATH_ENV)
                .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
                .unwrap_or_default(),
        );
    }
    dirs
}

#[cfg(target_os = "macos")]
fn platform_dirs(format: PluginFormat, home: &Path) -> Vec<PathBuf> {
    let sub = match format {
        PluginFormat::Clap => "CLAP",
        PluginFormat::Vst2 => "VST",
        PluginFormat::Vst3 => "VST3",
    };
    vec![
        PathBuf::from("/Library/Audio/Plug-Ins").join(sub),
        home.join("Library/Audio/Plug-Ins").join(sub),
    ]
}

#[cfg(target_os = "windows")]
fn platform_dirs(format: PluginFormat, home: &Path) -> Vec<PathBuf> {
    let common = PathBuf::from(r"C:\Program Files\Common Files");
    match format {
        PluginFormat::Clap => vec![
            common.join("CLAP"),
            home.join(r"AppData\Local\Programs\Common\CLAP"),
        ],
        PluginFormat::Vst2 => vec![
            common.join("VST2"),
            PathBuf::from(r"C:\Program Files\VstPlugins"),
            PathBuf::from(r"C:\Program Files\Steinberg\VstPlugins"),
        ],
        PluginFormat::Vst3 => vec![common.join("VST3")],
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_dirs(format: PluginFormat, home: &Path) -> Vec<PathBuf> {
    let (system, user) = match format {
        PluginFormat::Clap => ("clap", ".clap"),
        PluginFormat::Vst2 => ("vst", ".vst"),
        PluginFormat::Vst3 => ("vst3", ".vst3"),
    };
    vec![
        PathBuf::from("/usr/lib").join(system),
        PathBuf::from("/usr/local/lib").join(system),
        home.join(user),
    ]
}

/// Resolves a plugin path to the shared library to load (bundles on macOS wrap the binary).
pub fn binary_path(path: &Path) -> PathBuf {
    if !path.is_dir() {
        return path.to_path_buf();
    }
    let binary_dir = path.join(MACOS_BUNDLE_BINARY_DIR);
    let named = path.file_stem().map(|stem| binary_dir.join(stem));
    if let Some(named) = named.filter(|p| p.is_file()) {
        return named;
    }
    first_file_in(&binary_dir).unwrap_or_else(|| path.to_path_buf())
}

fn first_file_in(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file())
}

pub fn display_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown plugin")
        .to_string()
}
