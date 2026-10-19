//! Preset fluids and solid materials (design proposal for Phase 4, §6): the values the examples,
//! the CLI and the web editor use. The editor reads them from `apps/web/src/generated/library.json`
//! (`cfd-cli library`), which a test keeps in step with this file.
//!
//! Values at 20 °C (293 K) and 1 atm unless noted. Sources: Incropera, DeWitt, Bergman & Lavine,
//! *Fundamentals of Heat and Mass Transfer*, 6th ed., Tables A.1 (solids, 300 K), A.3
//! (non-metallic solids), A.4 (air) and A.5–A.6 (liquids), interpolated to 293 K where needed.

use serde::Serialize;

use crate::scene::{Fluid, SolidMaterial};

/// A fluid preset; `key` is what `Fluid::preset` records.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FluidPreset {
    pub key: &'static str,
    pub name: &'static str,
    /// kg/m³
    pub density: f64,
    /// m²/s
    pub kinematic_viscosity: f64,
    /// W/(m·K)
    pub thermal_conductivity: f64,
    /// J/(kg·K)
    pub specific_heat: f64,
    /// 1/K
    pub thermal_expansion: f64,
    pub source: &'static str,
}

/// A solid material preset.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SolidPreset {
    pub key: &'static str,
    pub name: &'static str,
    /// W/(m·K)
    pub thermal_conductivity: f64,
    /// kg/m³
    pub density: f64,
    /// J/(kg·K)
    pub specific_heat: f64,
    pub source: &'static str,
}

pub const FLUIDS: &[FluidPreset] = &[
    FluidPreset {
        key: "air",
        name: "Air",
        density: 1.204,
        kinematic_viscosity: 1.516e-5,
        thermal_conductivity: 0.0257,
        specific_heat: 1005.0,
        thermal_expansion: 3.43e-3, // ideal gas: 1/T
        source: "Incropera Table A.4, 293 K",
    },
    FluidPreset {
        key: "water",
        name: "Water",
        density: 998.2,
        kinematic_viscosity: 1.004e-6,
        thermal_conductivity: 0.598,
        specific_heat: 4182.0,
        thermal_expansion: 2.07e-4,
        source: "Incropera Table A.6, 293 K",
    },
    FluidPreset {
        key: "engineOil",
        name: "Engine oil",
        density: 888.0,
        kinematic_viscosity: 9.0e-4,
        thermal_conductivity: 0.145,
        specific_heat: 1880.0,
        thermal_expansion: 7.0e-4,
        source: "Incropera Table A.5 (unused engine oil), 293 K",
    },
    FluidPreset {
        key: "glycerol",
        name: "Glycerol",
        density: 1264.0,
        kinematic_viscosity: 1.18e-3,
        thermal_conductivity: 0.286,
        specific_heat: 2390.0,
        thermal_expansion: 4.8e-4,
        source: "Incropera Table A.5, 293 K",
    },
    FluidPreset {
        key: "ethanol",
        name: "Ethanol",
        density: 789.0,
        kinematic_viscosity: 1.52e-6,
        thermal_conductivity: 0.167,
        specific_heat: 2440.0,
        thermal_expansion: 1.09e-3,
        source: "CRC Handbook of Chemistry and Physics, 293 K",
    },
];

pub const SOLIDS: &[SolidPreset] = &[
    SolidPreset {
        key: "aluminium",
        name: "Aluminium",
        thermal_conductivity: 237.0,
        density: 2702.0,
        specific_heat: 903.0,
        source: "Incropera Table A.1 (pure), 300 K",
    },
    SolidPreset {
        key: "copper",
        name: "Copper",
        thermal_conductivity: 401.0,
        density: 8933.0,
        specific_heat: 385.0,
        source: "Incropera Table A.1 (pure), 300 K",
    },
    SolidPreset {
        key: "steel",
        name: "Steel (carbon)",
        thermal_conductivity: 63.9,
        density: 7832.0,
        specific_heat: 434.0,
        source: "Incropera Table A.1 (AISI 1010), 300 K",
    },
    SolidPreset {
        key: "glass",
        name: "Glass",
        thermal_conductivity: 1.4,
        density: 2500.0,
        specific_heat: 750.0,
        source: "Incropera Table A.3 (plate glass), 300 K",
    },
    SolidPreset {
        key: "pine",
        name: "Wood (pine)",
        thermal_conductivity: 0.12,
        density: 510.0,
        specific_heat: 1380.0,
        source: "Incropera Table A.3 (softwood, across the grain), 300 K",
    },
    SolidPreset {
        key: "concrete",
        name: "Concrete",
        thermal_conductivity: 1.4,
        density: 2300.0,
        specific_heat: 880.0,
        source: "Incropera Table A.3 (stone mix), 300 K",
    },
];

impl FluidPreset {
    /// The preset as a scene fluid with id `id`.
    pub fn fluid(&self, id: u8) -> Fluid {
        Fluid {
            id,
            name: self.name.into(),
            preset: Some(self.key.into()),
            density: self.density,
            kinematic_viscosity: self.kinematic_viscosity,
            thermal_conductivity: Some(self.thermal_conductivity),
            specific_heat: Some(self.specific_heat),
            thermal_expansion: Some(self.thermal_expansion),
        }
    }
}

impl SolidPreset {
    /// The preset as a scene solid material with id `id`.
    pub fn material(&self, id: u8) -> SolidMaterial {
        SolidMaterial {
            id,
            name: self.name.into(),
            thermal_conductivity: self.thermal_conductivity,
            density: self.density,
            specific_heat: self.specific_heat,
        }
    }
}

/// The fluid preset with this key.
pub fn fluid(key: &str) -> Option<&'static FluidPreset> {
    FLUIDS.iter().find(|f| f.key == key)
}

/// The solid preset with this key.
pub fn solid(key: &str) -> Option<&'static SolidPreset> {
    SOLIDS.iter().find(|s| s.key == key)
}

/// The library as written to `apps/web/src/generated/library.json`.
pub fn to_json_pretty() -> String {
    #[derive(Serialize)]
    struct Library {
        fluids: &'static [FluidPreset],
        solids: &'static [SolidPreset],
    }
    serde_json::to_string_pretty(&Library {
        fluids: FLUIDS,
        solids: SOLIDS,
    })
    .expect("library serialization cannot fail")
        + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_values_physical() {
        for (i, f) in FLUIDS.iter().enumerate() {
            assert!(FLUIDS[..i].iter().all(|g| g.key != f.key), "{}", f.key);
            assert!(f.density > 0.0 && f.kinematic_viscosity > 0.0 && f.specific_heat > 0.0);
            // Prandtl number in a sane range for gases and liquids.
            let pr = f.kinematic_viscosity * f.density * f.specific_heat / f.thermal_conductivity;
            assert!((0.5..2e4).contains(&pr), "{}: Pr = {pr}", f.key);
        }
        for (i, s) in SOLIDS.iter().enumerate() {
            assert!(SOLIDS[..i].iter().all(|t| t.key != s.key), "{}", s.key);
            assert!(s.density > 0.0 && s.thermal_conductivity > 0.0 && s.specific_heat > 0.0);
        }
    }

    #[test]
    fn presets_become_scene_values() {
        let air = fluid("air").unwrap().fluid(3);
        assert_eq!((air.id, air.preset.as_deref()), (3, Some("air")));
        assert_eq!(
            solid("copper").unwrap().material(1).thermal_conductivity,
            401.0
        );
        assert!(fluid("mercury").is_none());
    }
}
