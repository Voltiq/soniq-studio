//! Built-in device definitions, shared with the UI through one JSON file.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::{Device, DeviceParam, DeviceRole};

const CATALOG_JSON: &str = include_str!("../../../shared/device-catalog.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSpec {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub role: DeviceRole,
    /// "instrument", "audio-fx" or "midi-fx".
    #[serde(default)]
    pub category: String,
    pub params: Vec<ParamSpec>,
    /// Drum pads, each with its own copy of `pads.params`.
    #[serde(default)]
    pub pads: Option<PadSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamSpec {
    pub id: String,
    pub label: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub unit: String,
    #[serde(default)]
    pub log: Option<bool>,
    #[serde(default)]
    pub bipolar: Option<bool>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PadSpec {
    pub count: u8,
    /// Note of the first pad.
    pub first: u8,
    pub params: Vec<ParamSpec>,
    /// Per-pad default overrides, by note and parameter id.
    #[serde(default)]
    pub defaults: HashMap<String, HashMap<String, f64>>,
}

impl DeviceSpec {
    /// Every parameter, with pad parameters expanded to `p36.tune`, `p37.tune`, …
    /// and grouped as `pad:36`, `pad:37`, …
    pub fn all_params(&self) -> Vec<ParamSpec> {
        let mut all = self.params.clone();
        if let Some(pads) = &self.pads {
            for note in pads.first..pads.first.saturating_add(pads.count) {
                let overrides = pads.defaults.get(&note.to_string());
                all.extend(pads.params.iter().map(|p| ParamSpec {
                    id: format!("p{note}.{}", p.id),
                    group: Some(format!("pad:{note}")),
                    default: overrides.and_then(|o| o.get(&p.id)).copied().unwrap_or(p.default),
                    ..p.clone()
                }));
            }
        }
        all
    }
}

pub fn specs() -> &'static [DeviceSpec] {
    static SPECS: OnceLock<Vec<DeviceSpec>> = OnceLock::new();
    SPECS.get_or_init(|| serde_json::from_str(CATALOG_JSON).expect("shared/device-catalog.json is valid"))
}

pub fn spec(kind: &str) -> Option<&'static DeviceSpec> {
    specs().iter().find(|s| s.kind == kind)
}

/// MIDI effects change notes, so they sit before the instrument.
pub fn is_midi_fx(kind: &str) -> bool {
    spec(kind).is_some_and(|s| s.category == "midi-fx")
}

fn param(p: &ParamSpec, value: f64) -> DeviceParam {
    DeviceParam {
        id: p.id.clone(),
        label: p.label.clone(),
        value,
        min: p.min,
        max: p.max,
        default: p.default,
        unit: p.unit.clone(),
        log: p.log,
        bipolar: p.bipolar,
        step: p.step,
        options: p.options.clone(),
        group: p.group.clone(),
    }
}

/// A fresh device of the given type, with optional parameter overrides.
pub fn device(kind: &str, overrides: &[(&str, f64)]) -> Option<Device> {
    let spec = spec(kind)?;
    Some(Device {
        id: crate::new_id("dev"),
        kind: spec.kind.clone(),
        name: spec.name.clone(),
        role: spec.role,
        on: true,
        params: spec
            .all_params()
            .iter()
            .map(|p| param(p, overrides.iter().find(|(id, _)| *id == p.id).map_or(p.default, |(_, v)| *v)))
            .collect(),
        samples: Vec::new(),
        chains: Vec::new(),
        macros: Vec::new(),
        sidechain: None,
        modulates: None,
        plugin: None,
        state: None,
    })
}

/// Brings a device saved by an older version up to date: parameters added
/// since then appear at their defaults, removed ones go, values are kept.
pub fn refresh(device: &mut Device) {
    let Some(spec) = spec(&device.kind) else { return };
    let old = std::mem::take(&mut device.params);
    device.params = spec
        .all_params()
        .iter()
        .map(|p| {
            let value = old.iter().find(|o| o.id == p.id).map_or(p.default, |o| o.value.clamp(p.min, p.max));
            param(p, value)
        })
        .collect();
    device.role = spec.role;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drum_pads_get_their_own_parameters() {
        let rack = device("drum-rack", &[]).unwrap();
        let open_hat_choke = rack.params.iter().find(|p| p.id == "p46.choke").unwrap();
        assert_eq!(open_hat_choke.value, 1.0);
        assert_eq!(open_hat_choke.group.as_deref(), Some("pad:46"));
        assert_eq!(rack.params.iter().filter(|p| p.group.as_deref() == Some("pad:36")).count(), 5);
    }

    #[test]
    fn refresh_adds_new_parameters_and_keeps_values() {
        let mut synth = device("synth", &[]).unwrap();
        synth.params.retain(|p| p.id == "cutoff");
        synth.params[0].value = 900.0;
        synth.params.push(DeviceParam { id: "gone".into(), ..synth.params[0].clone() });
        refresh(&mut synth);
        assert_eq!(synth.params.iter().find(|p| p.id == "cutoff").unwrap().value, 900.0);
        assert!(synth.params.iter().any(|p| p.id == "unison"));
        assert!(!synth.params.iter().any(|p| p.id == "gone"));
    }
}
