//! VST 2.4 host on top of the `vst` crate's host module.
//! The crate is unmaintained (the VST2 SDK itself is retired) but remains the only Rust host implementation.
#![allow(deprecated)]

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use vst::host::{Host, HostBuffer, PluginLoader};
use vst::plugin::{Plugin, PluginParameters};

use super::scan::{binary_path, display_name};
use super::stack::STACK_CHANNELS;
use super::{ParamInfo, PluginDescriptor, PluginFormat, PluginInstance};

struct SilentHost;

impl Host for SilentHost {}

pub fn describe(path: &Path) -> PluginDescriptor {
    PluginDescriptor {
        format: PluginFormat::Vst2,
        path: path.to_path_buf(),
        id: String::new(),
        name: display_name(path),
        vendor: String::new(),
    }
}

pub fn load(descriptor: &PluginDescriptor) -> Result<Box<dyn PluginInstance>> {
    let binary = binary_path(&descriptor.path);
    let host = Arc::new(Mutex::new(SilentHost));
    let mut loader = PluginLoader::load(&binary, host)
        .map_err(|error| anyhow!("loading {}: {error}", binary.display()))?;
    let mut instance = loader
        .instance()
        .map_err(|error| anyhow!("instantiating {}: {error}", descriptor.name))?;
    instance.init();
    let info = instance.get_info();
    let param_object = instance.get_parameter_object();
    let params = read_params(&*param_object, info.parameters);
    let mut descriptor = descriptor.clone();
    if !info.name.is_empty() {
        descriptor.name = info.name.clone();
    }
    descriptor.vendor = info.vendor.clone();
    Ok(Box::new(Vst2Instance {
        descriptor,
        instance,
        param_object,
        params,
        input_count: info.inputs.max(0) as usize,
        output_count: info.outputs.max(0) as usize,
        host_buffer: HostBuffer::new(info.inputs.max(0) as usize, info.outputs.max(0) as usize),
        inputs: Vec::new(),
        outputs: Vec::new(),
        active: false,
    }))
}

fn read_params(params: &dyn PluginParameters, count: i32) -> Vec<ParamInfo> {
    (0..count.max(0))
        .map(|index| ParamInfo {
            id: index as u32,
            name: params.get_parameter_name(index),
            min: 0.0,
            max: 1.0,
            default: params.get_parameter(index) as f64,
            stepped: false,
        })
        .collect()
}

pub struct Vst2Instance {
    descriptor: PluginDescriptor,
    instance: vst::host::PluginInstance,
    param_object: Arc<dyn PluginParameters>,
    params: Vec<ParamInfo>,
    input_count: usize,
    output_count: usize,
    host_buffer: HostBuffer<f32>,
    inputs: Vec<Vec<f32>>,
    outputs: Vec<Vec<f32>>,
    active: bool,
}

unsafe impl Send for Vst2Instance {}

impl Vst2Instance {
    fn fill_inputs(&mut self, input: &[Vec<f32>], frames: usize) {
        for (channel, plane) in self.inputs.iter_mut().enumerate() {
            match input
                .get(channel)
                .or(input.first())
                .filter(|_| channel < STACK_CHANNELS)
            {
                Some(source) => plane[..frames].copy_from_slice(&source[..frames]),
                None => plane[..frames].fill(0.0),
            }
        }
    }

    fn drain_outputs(&self, output: &mut [Vec<f32>], frames: usize) {
        for (channel, plane) in output.iter_mut().enumerate() {
            let source = self.outputs.get(channel).or(self.outputs.first());
            if let Some(source) = source {
                plane[..frames].copy_from_slice(&source[..frames]);
            }
        }
    }
}

impl PluginInstance for Vst2Instance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn params(&self) -> &[ParamInfo] {
        &self.params
    }

    fn param_value(&self, id: u32) -> f64 {
        self.param_object.get_parameter(id as i32) as f64
    }

    fn set_param(&mut self, id: u32, value: f64) {
        self.param_object.set_parameter(id as i32, value as f32);
    }

    fn param_text(&self, id: u32, _value: f64) -> String {
        let text = self.param_object.get_parameter_text(id as i32);
        let label = self.param_object.get_parameter_label(id as i32);
        format!("{text} {label}").trim().to_string()
    }

    fn activate(&mut self, sample_rate: f64, max_block_frames: usize) -> Result<()> {
        self.deactivate();
        self.inputs = vec![vec![0.0; max_block_frames]; self.input_count];
        self.outputs = vec![vec![0.0; max_block_frames]; self.output_count];
        self.instance.set_sample_rate(sample_rate as f32);
        self.instance.set_block_size(max_block_frames as i64);
        self.instance.resume();
        self.active = true;
        Ok(())
    }

    fn deactivate(&mut self) {
        if self.active {
            self.instance.suspend();
        }
        self.active = false;
    }

    fn process(&mut self, input: &[Vec<f32>], output: &mut [Vec<f32>], frames: usize) {
        if !self.active || self.output_count == 0 {
            for (channel, plane) in output.iter_mut().enumerate() {
                if let Some(source) = input.get(channel) {
                    plane[..frames].copy_from_slice(&source[..frames]);
                }
            }
            return;
        }
        self.fill_inputs(input, frames);
        {
            let ins: Vec<&[f32]> = self.inputs.iter().map(|p| &p[..frames]).collect();
            let mut outs: Vec<&mut [f32]> =
                self.outputs.iter_mut().map(|p| &mut p[..frames]).collect();
            let mut buffer = self.host_buffer.bind(&ins, &mut outs);
            self.instance.process(&mut buffer);
        }
        self.drain_outputs(output, frames);
    }
}
