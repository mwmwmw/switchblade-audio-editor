use cpal::traits::HostTrait;

#[derive(Clone, Debug)]
pub struct HostInfo {
    pub id: cpal::HostId,
    pub name: String,
    pub outputs: Vec<String>,
    pub inputs: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceCatalog {
    pub hosts: Vec<HostInfo>,
}

impl DeviceCatalog {
    pub fn host(&self, id: cpal::HostId) -> Option<&HostInfo> {
        self.hosts.iter().find(|h| h.id == id)
    }
}

/// Device names are used as stable identifiers so the selection can outlive a device handle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceSelection {
    pub host: Option<cpal::HostId>,
    pub output: Option<String>,
    pub input: Option<String>,
}

pub fn enumerate() -> DeviceCatalog {
    let hosts = cpal::available_hosts()
        .into_iter()
        .filter_map(|id| {
            cpal::host_from_id(id)
                .ok()
                .map(|host| describe_host(id, &host))
        })
        .collect();
    DeviceCatalog { hosts }
}

fn describe_host(id: cpal::HostId, host: &cpal::Host) -> HostInfo {
    HostInfo {
        id,
        name: id.name().to_string(),
        outputs: host.output_devices().map(device_names).unwrap_or_default(),
        inputs: host.input_devices().map(device_names).unwrap_or_default(),
    }
}

fn device_names(devices: impl Iterator<Item = cpal::Device>) -> Vec<String> {
    devices.map(|device| device.to_string()).collect()
}

pub fn resolve_host(selection: &DeviceSelection) -> cpal::Host {
    selection
        .host
        .and_then(|id| cpal::host_from_id(id).ok())
        .unwrap_or_else(cpal::default_host)
}

pub fn resolve_output(host: &cpal::Host, selection: &DeviceSelection) -> Option<cpal::Device> {
    find_named(host.output_devices().ok(), selection.output.as_deref())
        .or_else(|| host.default_output_device())
}

pub fn resolve_input(host: &cpal::Host, selection: &DeviceSelection) -> Option<cpal::Device> {
    find_named(host.input_devices().ok(), selection.input.as_deref())
        .or_else(|| host.default_input_device())
}

fn find_named(
    devices: Option<impl Iterator<Item = cpal::Device>>,
    name: Option<&str>,
) -> Option<cpal::Device> {
    let name = name?;
    devices?.find(|device| device.to_string() == name)
}
