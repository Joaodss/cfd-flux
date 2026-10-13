//! The engines a long-running process (the server) opens once at start-up: what is available,
//! on which physical device, with how much memory, and which engine runs a given domain.

use cfd_core::scene::BackendPreference;
use serde::Serialize;

use crate::{open, Engine, EngineKind, EngineOptions};

/// Bytes of device memory per cell of the LBM solvers: two copies of the D2Q9 populations
/// (72), ρ/ux/uy/θ (16), flags (2) and the boundary slot (4); the thermal lattice adds two copies
/// of the D2Q5 populations (40). The same layout on every backend (ADR-019).
pub fn bytes_per_cell(thermal: bool) -> u64 {
    2 + 4 + 72 + 16 + if thermal { 40 } else { 0 }
}

/// The share of a GPU's memory a job may use (the rest is left to the display, the driver and
/// staging buffers).
const USABLE_MEMORY: f64 = 0.9;

/// One engine as `/api/capabilities` reports it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub kind: EngineKind,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Why the engine is unavailable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Device memory (CUDA, or wgpu on the same GPU as a CUDA device).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    /// Largest storage buffer (wgpu).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_buffer_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compute_capability: Option<String>,
    /// CPU threads (CPU engine).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threads: Option<usize>,
    /// A software adapter (wgpu): never chosen by `auto`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub software: bool,
    /// Engines with the same key run on the same hardware and share its job slot:
    /// `pci:<domain:bus:device.function>`, `gpu:<name>` when the PCI location is unknown, or `cpu`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_device: Option<String>,
}

impl EngineInfo {
    fn unavailable(kind: EngineKind, reason: String) -> Self {
        Self {
            kind,
            available: false,
            device: None,
            reason: Some(reason),
            memory_bytes: None,
            max_buffer_bytes: None,
            compute_capability: None,
            threads: None,
            software: false,
            physical_device: None,
        }
    }

    fn describe(engine: &Engine) -> Self {
        let mut info = Self {
            kind: engine.kind(),
            available: true,
            device: engine.device(),
            reason: None,
            memory_bytes: None,
            max_buffer_bytes: None,
            compute_capability: None,
            threads: None,
            software: false,
            physical_device: None,
        };
        match engine {
            Engine::Cpu => {
                info.threads = Some(rayon::current_num_threads());
                info.physical_device = Some("cpu".into());
            }
            Engine::Wgpu(ctx) => {
                let limits = ctx.limits();
                info.max_buffer_bytes = Some(
                    limits
                        .max_storage_buffer_binding_size
                        .min(limits.max_buffer_size),
                );
                info.software = ctx.is_software();
                let a = ctx.info();
                info.physical_device = Some(if a.device_pci_bus_id.is_empty() {
                    format!("gpu:{}", a.name)
                } else {
                    format!("pci:{}", a.device_pci_bus_id)
                });
            }
            #[cfg(feature = "cuda")]
            Engine::Cuda(dev) => {
                info.memory_bytes = Some(dev.total_memory());
                let (major, minor) = dev.compute_capability();
                info.compute_capability = Some(format!("{major}.{minor}"));
                info.physical_device = Some(match dev.pci_bus_id() {
                    Some(id) => format!("pci:{id}"),
                    None => format!("gpu:{}", dev.name()),
                });
            }
        }
        info
    }
}

/// Why no engine can run a domain.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ChooseError {
    #[error("the {kind} engine is not available: {reason}")]
    Unavailable { kind: EngineKind, reason: String },
    #[error("{0}")]
    TooLarge(String),
}

/// Every engine, opened once.
#[derive(Debug, Clone)]
pub struct Inventory {
    entries: Vec<(EngineInfo, Option<Engine>)>,
}

impl Inventory {
    /// Opens CUDA, wgpu and the CPU (in `auto`'s order). wgpu takes a software adapter only when
    /// `opts.allow_software` is set (and `auto` still skips it).
    pub fn probe(opts: &EngineOptions) -> Self {
        Self::probe_only(&EngineKind::BY_PREFERENCE, opts)
    }

    /// As [`probe`](Self::probe), but only the engines of `kinds` are opened; the others are
    /// reported as disabled.
    pub fn probe_only(kinds: &[EngineKind], opts: &EngineOptions) -> Self {
        let entries = EngineKind::BY_PREFERENCE
            .iter()
            .map(|&kind| {
                if !kinds.contains(&kind) {
                    return (EngineInfo::unavailable(kind, "disabled".into()), None);
                }
                match open(kind, opts) {
                    Ok(engine) => (EngineInfo::describe(&engine), Some(engine)),
                    Err(e) => (EngineInfo::unavailable(kind, format!("{e:#}")), None),
                }
            })
            .collect();
        Self::from_entries(entries)
    }

    /// An inventory from given entries (tests use fake devices).
    pub fn from_entries(mut entries: Vec<(EngineInfo, Option<Engine>)>) -> Self {
        // wgpu on the same GPU as CUDA: report (and check) the memory CUDA knows about.
        let cuda_memory: Vec<(String, u64)> = entries
            .iter()
            .filter_map(|(i, _)| Some((i.physical_device.clone()?, i.memory_bytes?)))
            .collect();
        for (info, _) in &mut entries {
            if info.memory_bytes.is_none() {
                if let Some(pd) = &info.physical_device {
                    info.memory_bytes = cuda_memory.iter().find(|(p, _)| p == pd).map(|m| m.1);
                }
            }
        }
        Self { entries }
    }

    pub fn engines(&self) -> impl Iterator<Item = &EngineInfo> {
        self.entries.iter().map(|(i, _)| i)
    }

    pub fn info(&self, kind: EngineKind) -> Option<&EngineInfo> {
        self.engines().find(|i| i.kind == kind)
    }

    /// The opened engine of `kind`, if available.
    pub fn engine(&self, kind: EngineKind) -> Option<&Engine> {
        self.entries
            .iter()
            .find(|(i, _)| i.kind == kind)
            .and_then(|(_, e)| e.as_ref())
    }

    /// The engine `auto` takes when memory is not a concern.
    pub fn auto(&self) -> Option<EngineKind> {
        self.engines()
            .find(|i| i.available && !i.software)
            .map(|i| i.kind)
    }

    /// Whether `kind` can hold a domain of `cells` cells.
    pub fn fits(&self, kind: EngineKind, cells: u64, thermal: bool) -> Result<(), ChooseError> {
        let info = self.info(kind).ok_or_else(|| ChooseError::Unavailable {
            kind,
            reason: "unknown engine".into(),
        })?;
        if !info.available {
            return Err(ChooseError::Unavailable {
                kind,
                reason: info.reason.clone().unwrap_or_default(),
            });
        }
        let needed = cells * bytes_per_cell(thermal);
        if let Some(mem) = info.memory_bytes {
            let usable = (mem as f64 * USABLE_MEMORY) as u64;
            if needed > usable {
                return Err(ChooseError::TooLarge(format!(
                    "{cells} cells need {} MiB on {kind}, but {} MiB of its {} MiB are usable",
                    needed >> 20,
                    usable >> 20,
                    mem >> 20
                )));
            }
        }
        if let Some(max) = info.max_buffer_bytes {
            // The largest buffer holds 9 populations per cell.
            let buffer = cells * 36;
            if buffer > max {
                return Err(ChooseError::TooLarge(format!(
                    "{cells} cells need a {} MiB buffer on {kind}, larger than its {} MiB limit",
                    buffer >> 20,
                    max >> 20
                )));
            }
        }
        Ok(())
    }

    /// The engine for a domain: the one named by `pref` (an error if it is unavailable or too
    /// small), or for `auto` the first available hardware engine (CUDA > wgpu > CPU) that can hold
    /// it. `notes` gets why engines were skipped.
    pub fn choose(
        &self,
        pref: BackendPreference,
        cells: u64,
        thermal: bool,
        notes: &mut Vec<String>,
    ) -> Result<EngineKind, ChooseError> {
        if let Some(kind) = EngineKind::from_preference(pref) {
            self.fits(kind, cells, thermal)?;
            return Ok(kind);
        }
        let mut last = None;
        for info in self.engines() {
            if info.software || !info.available {
                continue;
            }
            match self.fits(info.kind, cells, thermal) {
                Ok(()) => return Ok(info.kind),
                Err(e) => {
                    notes.push(e.to_string());
                    last = Some(e);
                }
            }
        }
        Err(last.unwrap_or_else(|| ChooseError::TooLarge("no engine is available".into())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(kind: EngineKind, pd: &str) -> EngineInfo {
        EngineInfo {
            physical_device: Some(pd.into()),
            ..EngineInfo::unavailable(kind, String::new())
        }
    }

    /// An RTX 4060-like GPU through CUDA and wgpu, and the CPU (no device handles: only the
    /// choice is tested).
    fn inventory(cuda_memory: u64) -> Inventory {
        let available = |mut i: EngineInfo| {
            i.available = true;
            i.reason = None;
            (i, None)
        };
        Inventory::from_entries(vec![
            available(EngineInfo {
                memory_bytes: Some(cuda_memory),
                ..info(EngineKind::Cuda, "pci:0000:01:00.0")
            }),
            available(EngineInfo {
                max_buffer_bytes: Some(2 << 30),
                ..info(EngineKind::Wgpu, "pci:0000:01:00.0")
            }),
            available(info(EngineKind::Cpu, "cpu")),
        ])
    }

    #[test]
    fn wgpu_on_the_cuda_gpu_reports_its_memory() {
        let inv = inventory(8 << 30);
        assert_eq!(
            inv.info(EngineKind::Wgpu).unwrap().memory_bytes,
            Some(8 << 30)
        );
        assert_eq!(inv.info(EngineKind::Cpu).unwrap().memory_bytes, None);
    }

    #[test]
    fn auto_takes_the_best_engine_that_holds_the_domain() {
        let inv = inventory(8 << 30);
        let mut notes = Vec::new();
        let auto = BackendPreference::Auto;
        assert_eq!(
            inv.choose(auto, 1024 * 1024, false, &mut notes),
            Ok(EngineKind::Cuda)
        );
        assert!(notes.is_empty());
        // 8192² × 94 B = 6 GiB: fits in 90% of 8 GiB on CUDA.
        assert_eq!(
            inv.choose(auto, 8192 * 8192, false, &mut notes),
            Ok(EngineKind::Cuda)
        );
        // With a 4 GiB card it falls through wgpu (same memory) to the CPU.
        let small = inventory(4 << 30);
        assert_eq!(
            small.choose(auto, 8192 * 8192, false, &mut notes),
            Ok(EngineKind::Cpu)
        );
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].contains("cuda"), "{notes:?}");
    }

    #[test]
    fn an_explicit_engine_must_be_available_and_large_enough() {
        let mut inv = inventory(4 << 30);
        let mut notes = Vec::new();
        assert!(matches!(
            inv.choose(BackendPreference::Cuda, 8192 * 8192, false, &mut notes),
            Err(ChooseError::TooLarge(_))
        ));
        inv.entries[0].0.available = false;
        inv.entries[0].0.reason = Some("no NVIDIA driver".into());
        assert_eq!(
            inv.choose(BackendPreference::Cuda, 16, false, &mut notes),
            Err(ChooseError::Unavailable {
                kind: EngineKind::Cuda,
                reason: "no NVIDIA driver".into()
            })
        );
        assert_eq!(inv.auto(), Some(EngineKind::Wgpu));
    }

    #[test]
    fn software_adapters_are_never_automatic() {
        let mut inv = inventory(8 << 30);
        inv.entries[0].0.available = false;
        inv.entries[1].0.software = true;
        assert_eq!(inv.auto(), Some(EngineKind::Cpu));
        let mut notes = Vec::new();
        assert_eq!(
            inv.choose(BackendPreference::Auto, 64, false, &mut notes),
            Ok(EngineKind::Cpu)
        );
        assert_eq!(
            inv.choose(BackendPreference::Wgpu, 64, false, &mut notes),
            Ok(EngineKind::Wgpu)
        );
    }
}
