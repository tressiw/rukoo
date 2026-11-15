//! `devices`: the microphones and output devices a track can record, as JSON.

use serde_json::Value;

/// `{ "inputs": [{id, name, default}], "outputs": [...] }`. An output's `id` goes into an
/// `output:<id>` source, which records it through loopback. A Mac without a microphone lists no
/// default input, although cpal 0.18.2 hands out a device for it.
#[cfg(any(windows, target_os = "macos"))]
pub fn list() -> Result<Value, String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    use serde_json::json;

    let host = cpal::default_host();
    let id_of = |d: &cpal::Device| d.id().ok().map(|id| id.to_string());
    let default_input = if crate::source::no_microphone() {
        None
    } else {
        host.default_input_device().as_ref().and_then(id_of)
    };
    let default_output = host.default_output_device().as_ref().and_then(id_of);
    let describe = |devices: Vec<cpal::Device>, default: &Option<String>| -> Vec<Value> {
        devices
            .iter()
            .filter_map(|d| {
                let id = id_of(d)?;
                // `to_string` can panic on a device whose name can't be read.
                let name = d
                    .description()
                    .map(|desc| desc.name().to_string())
                    .unwrap_or_else(|_| id.clone());
                Some(json!({ "id": id, "name": name, "default": default.as_ref() == Some(&id) }))
            })
            .collect()
    };
    let inputs = host.input_devices().map_err(|e| e.to_string())?.collect();
    let outputs = host.output_devices().map_err(|e| e.to_string())?.collect();
    Ok(json!({
        "inputs": describe(inputs, &default_input),
        "outputs": describe(outputs, &default_output),
    }))
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn list() -> Result<Value, String> {
    Err(crate::source::UNSUPPORTED.into())
}
