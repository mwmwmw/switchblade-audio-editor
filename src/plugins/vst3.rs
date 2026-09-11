//! VST3 bundles are discovered so they show up in the browser, but hosting is not implemented yet.

use std::path::Path;

use anyhow::{bail, Result};

use super::scan::display_name;
use super::{PluginDescriptor, PluginFormat, PluginInstance};

pub const UNSUPPORTED_MESSAGE: &str =
    "VST3 hosting is not implemented in this version; use the CLAP or VST2 build of the plugin";

pub fn describe(path: &Path) -> PluginDescriptor {
    PluginDescriptor {
        format: PluginFormat::Vst3,
        path: path.to_path_buf(),
        id: String::new(),
        name: display_name(path),
        vendor: String::new(),
    }
}

pub fn load(descriptor: &PluginDescriptor) -> Result<Box<dyn PluginInstance>> {
    bail!("{}: {UNSUPPORTED_MESSAGE}", descriptor.name)
}
