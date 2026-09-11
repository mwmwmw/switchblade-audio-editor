pub mod clap;
pub mod scan;
pub mod stack;
pub mod vst2;
pub mod vst3;

use std::path::PathBuf;

use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluginFormat {
    Clap,
    Vst2,
    Vst3,
}

impl PluginFormat {
    pub fn label(self) -> &'static str {
        match self {
            PluginFormat::Clap => "CLAP",
            PluginFormat::Vst2 => "VST2",
            PluginFormat::Vst3 => "VST3",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PluginDescriptor {
    pub format: PluginFormat,
    /// Bundle directory on macOS, shared library elsewhere.
    pub path: PathBuf,
    /// Format-specific identifier (CLAP id; empty for VST2/VST3).
    pub id: String,
    pub name: String,
    pub vendor: String,
}

#[derive(Clone, Debug)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub stepped: bool,
}

/// A loaded plugin. Implementations wrap raw pointers, so all access is serialised by the stack's mutex.
pub trait PluginInstance: Send {
    fn descriptor(&self) -> &PluginDescriptor;
    fn params(&self) -> &[ParamInfo];
    fn param_value(&self, id: u32) -> f64;
    fn set_param(&mut self, id: u32, value: f64);
    fn param_text(&self, id: u32, value: f64) -> String;
    fn activate(&mut self, sample_rate: f64, max_block_frames: usize) -> Result<()>;
    fn deactivate(&mut self);
    /// Processes `frames` samples from `input` into `output`; both hold `stack::STACK_CHANNELS` planes.
    fn process(&mut self, input: &[Vec<f32>], output: &mut [Vec<f32>], frames: usize);

    /// Whether this plugin can show its own editor window.
    ///
    /// Only plugins that provide a *floating* editor qualify today: those create and own
    /// their window themselves. An embedded editor has to be parented into a window the host
    /// supplies, which means native windowing code Switchblade does not have yet.
    fn has_editor(&self) -> bool {
        false
    }

    fn editor_is_open(&self) -> bool {
        false
    }

    /// Opens the editor. Must be called from the main thread; CLAP requires it.
    fn open_editor(&mut self) -> Result<()> {
        anyhow::bail!("{} has no editor Switchblade can show", self.descriptor().name)
    }

    fn close_editor(&mut self) {}

    /// Called once per UI frame while an editor is open, for hosts duties that need a pulse.
    fn tick_editor(&mut self) {}
}

pub fn load(descriptor: &PluginDescriptor) -> Result<Box<dyn PluginInstance>> {
    match descriptor.format {
        PluginFormat::Clap => clap::load(descriptor),
        PluginFormat::Vst2 => vst2::load(descriptor),
        PluginFormat::Vst3 => vst3::load(descriptor),
    }
}
