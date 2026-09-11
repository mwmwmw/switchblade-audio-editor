//! Minimal CLAP host built directly on `clap-sys`.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread::ThreadId;

use anyhow::{anyhow, bail, Result};
use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::entry::clap_plugin_entry;
use clap_sys::events::{
    clap_event_header, clap_event_param_value, clap_input_events, clap_output_events,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE,
};
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS,
};
use clap_sys::ext::gui::{clap_host_gui, clap_plugin_gui, CLAP_EXT_GUI};
#[cfg(target_os = "macos")]
use clap_sys::ext::gui::CLAP_WINDOW_API_COCOA;
#[cfg(target_os = "windows")]
use clap_sys::ext::gui::CLAP_WINDOW_API_WIN32;
#[cfg(all(unix, not(target_os = "macos")))]
use clap_sys::ext::gui::CLAP_WINDOW_API_X11;
use clap_sys::ext::thread_check::{clap_host_thread_check, CLAP_EXT_THREAD_CHECK};
use clap_sys::ext::params::{
    clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS, CLAP_PARAM_IS_HIDDEN,
    CLAP_PARAM_IS_STEPPED,
};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::process::clap_process;
use clap_sys::version::CLAP_VERSION;
use libloading::Library;

use super::scan::binary_path;
use super::stack::STACK_CHANNELS;
use super::{ParamInfo, PluginDescriptor, PluginFormat, PluginInstance};

/// Window API this platform speaks, as CLAP names them.
#[cfg(target_os = "windows")]
const WINDOW_API: &CStr = CLAP_WINDOW_API_WIN32;
#[cfg(target_os = "macos")]
const WINDOW_API: &CStr = CLAP_WINDOW_API_COCOA;
#[cfg(all(unix, not(target_os = "macos")))]
const WINDOW_API: &CStr = CLAP_WINDOW_API_X11;

const ENTRY_SYMBOL: &[u8] = b"clap_entry\0";
const HOST_NAME: &CStr = c"Switchblade";
const HOST_VENDOR: &CStr = c"Switchblade";
const HOST_URL: &CStr = c"https://github.com/switchblade";
const HOST_VERSION: &CStr = c"0.1.0";
const PARAM_TEXT_CAPACITY: usize = 128;
const MIN_BLOCK_FRAMES: u32 = 1;
const UNUSED_NOTE_FIELD: i16 = -1;

struct ClapLibrary {
    _library: Library,
    entry: *const clap_plugin_entry,
    factory: *const clap_plugin_factory,
}

impl ClapLibrary {
    unsafe fn open(path: &Path) -> Result<Self> {
        let binary = binary_path(path);
        let library = Library::new(&binary)
            .map_err(|error| anyhow!("loading {}: {error}", binary.display()))?;
        let entry = *library
            .get::<*const clap_plugin_entry>(ENTRY_SYMBOL)
            .map_err(|_| anyhow!("{} has no clap_entry symbol", binary.display()))?;
        if entry.is_null() {
            bail!("null clap_entry");
        }
        let path_c = CString::new(path.to_string_lossy().as_bytes())?;
        if let Some(init) = (*entry).init {
            if !init(path_c.as_ptr()) {
                bail!("clap_entry.init failed");
            }
        }
        let get_factory = (*entry)
            .get_factory
            .ok_or_else(|| anyhow!("clap_entry.get_factory missing"))?;
        let factory = get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()) as *const clap_plugin_factory;
        if factory.is_null() {
            bail!("plugin has no plugin factory");
        }
        Ok(Self {
            _library: library,
            entry,
            factory,
        })
    }

    unsafe fn descriptors(&self) -> Vec<*const clap_plugin_descriptor> {
        let factory = &*self.factory;
        let (Some(count), Some(get)) = (factory.get_plugin_count, factory.get_plugin_descriptor)
        else {
            return Vec::new();
        };
        (0..count(self.factory))
            .map(|index| get(self.factory, index))
            .filter(|d| !d.is_null())
            .collect()
    }
}

impl Drop for ClapLibrary {
    fn drop(&mut self) {
        unsafe {
            if let Some(deinit) = (*self.entry).deinit {
                deinit();
            }
        }
    }
}

pub fn describe(path: &Path) -> Result<Vec<PluginDescriptor>> {
    unsafe {
        let library = ClapLibrary::open(path)?;
        Ok(library
            .descriptors()
            .into_iter()
            .map(|d| to_descriptor(path, &*d))
            .collect())
    }
}

unsafe fn to_descriptor(path: &Path, descriptor: &clap_plugin_descriptor) -> PluginDescriptor {
    PluginDescriptor {
        format: PluginFormat::Clap,
        path: path.to_path_buf(),
        id: c_string(descriptor.id),
        name: c_string(descriptor.name),
        vendor: c_string(descriptor.vendor),
    }
}

unsafe fn c_string(pointer: *const c_char) -> String {
    if pointer.is_null() {
        String::new()
    } else {
        CStr::from_ptr(pointer).to_string_lossy().into_owned()
    }
}

pub fn load(descriptor: &PluginDescriptor) -> Result<Box<dyn PluginInstance>> {
    unsafe {
        ClapInstance::create(descriptor)
            .map(|instance| Box::new(instance) as Box<dyn PluginInstance>)
    }
}

pub struct ClapInstance {
    descriptor: PluginDescriptor,
    plugin: *const clap_plugin,
    host: Box<clap_host>,
    /// Kept alive for as long as the plugin can call back into it; `host.host_data` points here.
    host_data: Box<HostData>,
    library: Option<ClapLibrary>,
    gui_ext: *const clap_plugin_gui,
    editor_open: bool,
    params_ext: *const clap_plugin_params,
    params: Vec<ParamInfo>,
    cookies: Vec<*mut c_void>,
    input_channels: usize,
    output_channels: usize,
    active: bool,
    pending_events: Vec<clap_event_param_value>,
    extra_inputs: Vec<Vec<f32>>,
    extra_outputs: Vec<Vec<f32>>,
    steady_time: i64,
}

unsafe impl Send for ClapInstance {}

impl ClapInstance {
    unsafe fn create(descriptor: &PluginDescriptor) -> Result<Self> {
        let library = ClapLibrary::open(&descriptor.path)?;
        let mut host_data = Box::new(HostData::default());
        let mut host = new_host();
        // Callbacks arrive with only the host pointer, so they find their way back here.
        host.host_data = (&mut *host_data as *mut HostData).cast();
        let id = CString::new(descriptor.id.as_bytes())?;
        let create = (*library.factory)
            .create_plugin
            .ok_or_else(|| anyhow!("factory.create_plugin missing"))?;
        let plugin = create(library.factory, &*host, id.as_ptr());
        if plugin.is_null() {
            bail!("plugin {} refused to instantiate", descriptor.name);
        }
        if !(*plugin).init.is_some_and(|init| init(plugin)) {
            bail!("plugin {} failed to initialise", descriptor.name);
        }
        let (input_channels, output_channels) = main_port_channels(plugin);
        let mut instance = Self {
            descriptor: descriptor.clone(),
            plugin,
            host,
            host_data,
            library: Some(library),
            gui_ext: extension(plugin, CLAP_EXT_GUI) as *const clap_plugin_gui,
            editor_open: false,
            params_ext: extension(plugin, CLAP_EXT_PARAMS) as *const clap_plugin_params,
            params: Vec::new(),
            cookies: Vec::new(),
            input_channels,
            output_channels,
            active: false,
            pending_events: Vec::new(),
            extra_inputs: Vec::new(),
            extra_outputs: Vec::new(),
            steady_time: 0,
        };
        instance.read_params();
        Ok(instance)
    }

    unsafe fn read_params(&mut self) {
        if self.params_ext.is_null() {
            return;
        }
        let ext = &*self.params_ext;
        let (Some(count), Some(get_info)) = (ext.count, ext.get_info) else {
            return;
        };
        for index in 0..count(self.plugin) {
            let mut info: clap_param_info = std::mem::zeroed();
            if !get_info(self.plugin, index, &mut info) || info.flags & CLAP_PARAM_IS_HIDDEN != 0 {
                continue;
            }
            self.params.push(ParamInfo {
                id: info.id,
                name: c_string(info.name.as_ptr()),
                min: info.min_value,
                max: info.max_value,
                default: info.default_value,
                stepped: info.flags & CLAP_PARAM_IS_STEPPED != 0,
            });
            self.cookies.push(info.cookie);
        }
    }

    fn cookie_for(&self, id: u32) -> *mut c_void {
        self.params
            .iter()
            .position(|p| p.id == id)
            .and_then(|index| self.cookies.get(index).copied())
            .unwrap_or(ptr::null_mut())
    }

    fn param_event(&self, id: u32, value: f64) -> clap_event_param_value {
        clap_event_param_value {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_param_value>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_PARAM_VALUE,
                flags: 0,
            },
            param_id: id,
            cookie: self.cookie_for(id),
            note_id: -1,
            port_index: UNUSED_NOTE_FIELD,
            channel: UNUSED_NOTE_FIELD,
            key: UNUSED_NOTE_FIELD,
            value,
        }
    }

    unsafe fn flush_params(&mut self) {
        if self.params_ext.is_null() || self.pending_events.is_empty() {
            return;
        }
        if let Some(flush) = (*self.params_ext).flush {
            let in_events = input_events(&mut self.pending_events);
            let out_events = output_events();
            flush(self.plugin, &in_events, &out_events);
        }
        self.pending_events.clear();
    }

    fn resize_scratch(&mut self, frames: usize) {
        let extra_in = self.input_channels.saturating_sub(STACK_CHANNELS);
        let extra_out = self.output_channels.saturating_sub(STACK_CHANNELS);
        self.extra_inputs = vec![vec![0.0; frames]; extra_in];
        self.extra_outputs = vec![vec![0.0; frames]; extra_out];
    }

    fn input_pointers(&mut self, input: &[Vec<f32>]) -> Vec<*mut f32> {
        (0..self.input_channels)
            .map(|channel| match input.get(channel).or(input.first()) {
                Some(plane) if channel < STACK_CHANNELS => plane.as_ptr() as *mut f32,
                _ => self.extra_inputs[channel - STACK_CHANNELS].as_mut_ptr(),
            })
            .collect()
    }

    fn output_pointers(&mut self, output: &mut [Vec<f32>]) -> Vec<*mut f32> {
        (0..self.output_channels)
            .map(|channel| {
                if channel < STACK_CHANNELS {
                    output[channel].as_mut_ptr()
                } else {
                    self.extra_outputs[channel - STACK_CHANNELS].as_mut_ptr()
                }
            })
            .collect()
    }

    unsafe fn run_process(&mut self, input: &[Vec<f32>], output: &mut [Vec<f32>], frames: usize) {
        let mut in_ptrs = self.input_pointers(input);
        let mut out_ptrs = self.output_pointers(output);
        let in_buffer = audio_buffer(&mut in_ptrs);
        let mut out_buffer = audio_buffer(&mut out_ptrs);
        let in_events = input_events(&mut self.pending_events);
        let out_events = output_events();
        let process = clap_process {
            steady_time: self.steady_time,
            frames_count: frames as u32,
            transport: ptr::null(),
            audio_inputs: &in_buffer,
            audio_outputs: &mut out_buffer,
            audio_inputs_count: u32::from(self.input_channels > 0),
            audio_outputs_count: u32::from(self.output_channels > 0),
            in_events: &in_events,
            out_events: &out_events,
        };
        if let Some(process_fn) = (*self.plugin).process {
            process_fn(self.plugin, &process);
        }
        self.pending_events.clear();
        self.steady_time += frames as i64;
    }
}

impl PluginInstance for ClapInstance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn has_editor(&self) -> bool {
        unsafe { self.supports_floating_gui() }
    }

    fn editor_is_open(&self) -> bool {
        self.editor_open
    }

    fn open_editor(&mut self) -> Result<()> {
        if self.editor_open {
            return Ok(());
        }
        unsafe {
            if !self.supports_floating_gui() {
                bail!(
                    "{} only offers an embedded editor, which needs host windowing \
                     Switchblade does not have yet",
                    self.descriptor.name
                );
            }
            let gui = &*self.gui_ext;
            let create = gui.create.ok_or_else(|| anyhow!("gui.create missing"))?;
            if !create(self.plugin, WINDOW_API.as_ptr(), true) {
                bail!("{} refused to create its editor", self.descriptor.name);
            }
            if let Some(suggest_title) = gui.suggest_title {
                let title = CString::new(format!("{} — Switchblade", self.descriptor.name))
                    .unwrap_or_default();
                suggest_title(self.plugin, title.as_ptr());
            }
            if let Some(show) = gui.show {
                if !show(self.plugin) {
                    if let Some(destroy) = gui.destroy {
                        destroy(self.plugin);
                    }
                    bail!("{} would not show its editor", self.descriptor.name);
                }
            }
        }
        self.host_data.editor_closed.store(false, Ordering::Relaxed);
        self.editor_open = true;
        Ok(())
    }

    fn close_editor(&mut self) {
        if !self.editor_open {
            return;
        }
        unsafe {
            if let Some(destroy) = self.gui_ext.as_ref().and_then(|gui| gui.destroy) {
                destroy(self.plugin);
            }
        }
        self.editor_open = false;
    }

    /// Notices a window the plugin closed on its own, so the panel's button goes back in step.
    fn tick_editor(&mut self) {
        if self.editor_open && self.host_data.editor_closed.swap(false, Ordering::Relaxed) {
            self.close_editor();
        }
    }

    fn params(&self) -> &[ParamInfo] {
        &self.params
    }

    fn param_value(&self, id: u32) -> f64 {
        unsafe {
            let mut value = 0.0;
            if !self.params_ext.is_null() {
                if let Some(get_value) = (*self.params_ext).get_value {
                    get_value(self.plugin, id, &mut value);
                }
            }
            value
        }
    }

    fn set_param(&mut self, id: u32, value: f64) {
        let event = self.param_event(id, value);
        self.pending_events.push(event);
        if !self.active {
            unsafe { self.flush_params() };
        }
    }

    fn param_text(&self, id: u32, value: f64) -> String {
        unsafe {
            let Some(value_to_text) = self.params_ext.as_ref().and_then(|ext| ext.value_to_text)
            else {
                return format!("{value:.3}");
            };
            let mut buffer = [0 as c_char; PARAM_TEXT_CAPACITY];
            if value_to_text(
                self.plugin,
                id,
                value,
                buffer.as_mut_ptr(),
                PARAM_TEXT_CAPACITY as u32,
            ) {
                c_string(buffer.as_ptr())
            } else {
                format!("{value:.3}")
            }
        }
    }

    fn activate(&mut self, sample_rate: f64, max_block_frames: usize) -> Result<()> {
        self.deactivate();
        self.resize_scratch(max_block_frames);
        unsafe {
            let plugin = &*self.plugin;
            let activated = plugin.activate.is_some_and(|f| {
                f(
                    self.plugin,
                    sample_rate,
                    MIN_BLOCK_FRAMES,
                    max_block_frames as u32,
                )
            });
            if !activated {
                bail!(
                    "{} refused to activate at {sample_rate} Hz",
                    self.descriptor.name
                );
            }
            if !plugin.start_processing.is_none_or(|f| f(self.plugin)) {
                if let Some(f) = plugin.deactivate {
                    f(self.plugin)
                }
                bail!("{} refused to start processing", self.descriptor.name);
            }
        }
        self.active = true;
        Ok(())
    }

    fn deactivate(&mut self) {
        if !self.active {
            return;
        }
        unsafe {
            let plugin = &*self.plugin;
            if let Some(f) = plugin.stop_processing {
                f(self.plugin)
            }
            if let Some(f) = plugin.deactivate {
                f(self.plugin)
            }
        }
        self.active = false;
    }

    fn process(&mut self, input: &[Vec<f32>], output: &mut [Vec<f32>], frames: usize) {
        if !self.active || self.output_channels == 0 {
            pass_through(input, output, frames);
            return;
        }
        unsafe { self.run_process(input, output, frames) };
        if self.output_channels == 1 {
            let (first, rest) = output.split_at_mut(1);
            for plane in rest {
                plane[..frames].copy_from_slice(&first[0][..frames]);
            }
        }
    }
}

impl ClapInstance {
    /// True when the plugin can put its editor in a window it owns.
    ///
    /// Embedded editors are the common case and are deliberately not reported here: they
    /// need a native parent window, so claiming support would only produce a dead button.
    unsafe fn supports_floating_gui(&self) -> bool {
        let Some(gui) = self.gui_ext.as_ref() else {
            return false;
        };
        let (Some(supported), Some(_)) = (gui.is_api_supported, gui.create) else {
            return false;
        };
        supported(self.plugin, WINDOW_API.as_ptr(), true)
    }
}

impl Drop for ClapInstance {
    fn drop(&mut self) {
        self.close_editor();
        self.deactivate();
        unsafe {
            if let Some(destroy) = (*self.plugin).destroy {
                destroy(self.plugin);
            }
        }
        self.library.take();
        // Both outlive every call the plugin could still make through them.
        let _ = &self.host;
        let _ = &self.host_data;
    }
}

fn pass_through(input: &[Vec<f32>], output: &mut [Vec<f32>], frames: usize) {
    for (channel, plane) in output.iter_mut().enumerate() {
        if let Some(source) = input.get(channel) {
            plane[..frames].copy_from_slice(&source[..frames]);
        }
    }
}

unsafe fn extension(plugin: *const clap_plugin, id: &CStr) -> *const c_void {
    (*plugin)
        .get_extension
        .map_or(ptr::null(), |get| get(plugin, id.as_ptr()))
}

unsafe fn main_port_channels(plugin: *const clap_plugin) -> (usize, usize) {
    let ports = extension(plugin, CLAP_EXT_AUDIO_PORTS) as *const clap_plugin_audio_ports;
    if ports.is_null() {
        return (STACK_CHANNELS, STACK_CHANNELS);
    }
    (
        port_channels(plugin, &*ports, true),
        port_channels(plugin, &*ports, false),
    )
}

unsafe fn port_channels(
    plugin: *const clap_plugin,
    ports: &clap_plugin_audio_ports,
    is_input: bool,
) -> usize {
    let (Some(count), Some(get)) = (ports.count, ports.get) else {
        return 0;
    };
    if count(plugin, is_input) == 0 {
        return 0;
    }
    let mut info: clap_audio_port_info = std::mem::zeroed();
    if get(plugin, 0, is_input, &mut info) {
        info.channel_count as usize
    } else {
        0
    }
}

fn audio_buffer(pointers: &mut [*mut f32]) -> clap_audio_buffer {
    clap_audio_buffer {
        data32: pointers.as_mut_ptr(),
        data64: ptr::null_mut(),
        channel_count: pointers.len() as u32,
        latency: 0,
        constant_mask: 0,
    }
}

fn input_events(queue: &mut Vec<clap_event_param_value>) -> clap_input_events {
    clap_input_events {
        ctx: queue as *mut Vec<clap_event_param_value> as *mut c_void,
        size: Some(events_size),
        get: Some(events_get),
    }
}

fn output_events() -> clap_output_events {
    clap_output_events {
        ctx: ptr::null_mut(),
        try_push: Some(events_try_push),
    }
}

unsafe extern "C" fn events_size(list: *const clap_input_events) -> u32 {
    let queue = &*((*list).ctx as *const Vec<clap_event_param_value>);
    queue.len() as u32
}

unsafe extern "C" fn events_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    let queue = &*((*list).ctx as *const Vec<clap_event_param_value>);
    queue
        .get(index as usize)
        .map_or(ptr::null(), |event| &event.header)
}

unsafe extern "C" fn events_try_push(
    _list: *const clap_output_events,
    _event: *const clap_event_header,
) -> bool {
    true
}

/// Per-instance state the plugin's callbacks reach through `clap_host::host_data`.
#[derive(Default)]
struct HostData {
    /// Set when the plugin tells us its editor went away, e.g. the user closed the window.
    editor_closed: AtomicBool,
}

/// Wrapper that lets a `static` hold the C vtables, which are fn pointers and so are Sync.
struct SyncVtable<T>(T);
unsafe impl<T> Sync for SyncVtable<T> {}

static HOST_GUI: SyncVtable<clap_host_gui> = SyncVtable(clap_host_gui {
    resize_hints_changed: Some(host_noop),
    // A floating window sizes itself, so the host has nothing to resize or reveal.
    request_resize: Some(host_request_resize),
    request_show: Some(host_request_show_or_hide),
    request_hide: Some(host_request_show_or_hide),
    closed: Some(host_gui_closed),
});

static HOST_THREAD_CHECK: SyncVtable<clap_host_thread_check> = SyncVtable(clap_host_thread_check {
    is_main_thread: Some(host_is_main_thread),
    is_audio_thread: Some(host_is_audio_thread),
});

/// The thread that built the UI. Plugins ask before touching anything thread-restricted, and
/// several refuse to create an editor at all when the host cannot answer.
static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();

/// Records the calling thread as the main one. Called once while the app starts up.
pub fn set_main_thread() {
    let _ = MAIN_THREAD.set(std::thread::current().id());
}

fn new_host() -> Box<clap_host> {
    Box::new(clap_host {
        clap_version: CLAP_VERSION,
        host_data: ptr::null_mut(),
        name: HOST_NAME.as_ptr(),
        vendor: HOST_VENDOR.as_ptr(),
        url: HOST_URL.as_ptr(),
        version: HOST_VERSION.as_ptr(),
        get_extension: Some(host_get_extension),
        request_restart: Some(host_noop),
        request_process: Some(host_noop),
        request_callback: Some(host_noop),
    })
}

unsafe fn host_data(host: *const clap_host) -> Option<&'static HostData> {
    if host.is_null() {
        return None;
    }
    ((*host).host_data as *const HostData).as_ref()
}

unsafe extern "C" fn host_get_extension(
    _host: *const clap_host,
    id: *const c_char,
) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_GUI {
        return (&HOST_GUI.0 as *const clap_host_gui).cast();
    }
    if id == CLAP_EXT_THREAD_CHECK {
        return (&HOST_THREAD_CHECK.0 as *const clap_host_thread_check).cast();
    }
    ptr::null()
}

unsafe extern "C" fn host_noop(_host: *const clap_host) {}

unsafe extern "C" fn host_request_resize(_host: *const clap_host, _w: u32, _h: u32) -> bool {
    false
}

unsafe extern "C" fn host_request_show_or_hide(_host: *const clap_host) -> bool {
    false
}

unsafe extern "C" fn host_gui_closed(host: *const clap_host, _was_destroyed: bool) {
    if let Some(data) = host_data(host) {
        data.editor_closed.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_is_main_thread(_host: *const clap_host) -> bool {
    MAIN_THREAD
        .get()
        .is_some_and(|id| *id == std::thread::current().id())
}

unsafe extern "C" fn host_is_audio_thread(_host: *const clap_host) -> bool {
    // Everything that is not the UI thread reaches plugins from the audio callback.
    !host_is_main_thread(_host)
}
