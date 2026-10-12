//! Adapter discovery and device creation.
//!
//! wgpu's object model, with the closest .NET/Direct3D equivalents:
//! - [`wgpu::Instance`]: the entry point (≈ a DXGI factory); it lists the adapters of every
//!   enabled backend (Vulkan, DX12, Metal).
//! - [`wgpu::Adapter`]: one physical GPU or a software rasteriser (WARP, lavapipe).
//! - [`wgpu::Device`]: the logical connection to an adapter; it creates buffers, shaders and
//!   pipelines (≈ `ID3D12Device`).
//! - [`wgpu::Queue`]: where recorded command buffers are submitted; work runs in submission
//!   order, asynchronously with respect to the CPU.
//!
//! One [`GpuContext`] is meant to be shared (`Arc`) by every solver of a process: opening a device
//! costs ~100 ms and the validation suite creates dozens of solvers.

use std::sync::{Arc, Mutex};

use wgpu::{Backend, DeviceType};

use crate::GpuError;

/// How to choose an adapter.
#[derive(Debug, Clone, Default)]
pub struct GpuOptions {
    /// Accept software adapters (WARP, lavapipe/llvmpipe): fine for tests, but usually slower
    /// than the CPU backend.
    pub allow_software: bool,
    /// Only adapters whose name contains this text (case-insensitive). Defaults to the
    /// `CFD_WGPU_ADAPTER` environment variable.
    pub adapter_name: Option<String>,
}

/// Environment variable that restricts the adapter by name.
pub const ADAPTER_ENV: &str = "CFD_WGPU_ADAPTER";

/// A device ready to run solvers.
pub struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    limits: wgpu::Limits,
    /// First validation/out-of-memory error not caught by an error scope (wgpu's default is to
    /// panic; solvers turn it into a `SolverError` instead).
    uncaptured: Arc<Mutex<Option<String>>>,
}

impl std::fmt::Debug for GpuContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.description())
    }
}

/// Adapters in order of preference: discrete > integrated > virtual > software, and Vulkan >
/// DX12 > Metal among equals. The GL backend is never used (limited compute support).
fn rank(info: &wgpu::AdapterInfo) -> Option<(u8, u8)> {
    let kind = match info.device_type {
        DeviceType::DiscreteGpu => 0,
        DeviceType::IntegratedGpu => 1,
        DeviceType::VirtualGpu | DeviceType::Other => 2,
        DeviceType::Cpu => 3,
    };
    let backend = match info.backend {
        Backend::Vulkan => 0,
        Backend::Dx12 => 1,
        Backend::Metal => 2,
        _ => return None,
    };
    Some((kind, backend))
}

pub fn is_software(info: &wgpu::AdapterInfo) -> bool {
    info.device_type == DeviceType::Cpu
}

/// Every usable adapter, best first.
pub fn adapters() -> Vec<wgpu::AdapterInfo> {
    let mut list: Vec<_> = enumerate()
        .into_iter()
        .map(|a| a.get_info())
        .filter(|i| rank(i).is_some())
        .collect();
    list.sort_by_key(rank);
    list
}

fn enumerate() -> Vec<wgpu::Adapter> {
    // `WGPU_BACKEND=vulkan|dx12|…` restricts the backends (wgpu's own convention).
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
}

/// One-line description of an adapter, e.g. "NVIDIA GeForce RTX 4060 (Vulkan, DiscreteGpu)".
pub fn describe(info: &wgpu::AdapterInfo) -> String {
    format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type)
}

impl GpuContext {
    /// Picks the best adapter allowed by `opts` and opens a device with the adapter's own limits
    /// (the WebGPU defaults, e.g. 128 MiB per storage buffer, are too small for large grids).
    pub fn new(opts: &GpuOptions) -> Result<Arc<Self>, GpuError> {
        let filter = opts
            .adapter_name
            .clone()
            .or_else(|| std::env::var(ADAPTER_ENV).ok())
            .map(|s| s.to_lowercase());
        let mut candidates: Vec<wgpu::Adapter> = enumerate()
            .into_iter()
            .filter(|a| {
                let info = a.get_info();
                rank(&info).is_some()
                    && (opts.allow_software || !is_software(&info))
                    && filter
                        .as_ref()
                        .is_none_or(|f| info.name.to_lowercase().contains(f))
            })
            .collect();
        candidates.sort_by_key(|a| rank(&a.get_info()));
        let Some(adapter) = candidates.into_iter().next() else {
            return Err(GpuError::NoAdapter {
                available: adapters().iter().map(describe).collect(),
                software_allowed: opts.allow_software,
            });
        };
        let info = adapter.get_info();
        let limits = adapter.limits();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("cfd-gpu"),
            required_features: wgpu::Features::empty(),
            required_limits: limits.clone(),
            ..Default::default()
        }))
        .map_err(|e| GpuError::Device(format!("{}: {e}", describe(&info))))?;
        let uncaptured = Arc::new(Mutex::new(None));
        let sink = uncaptured.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            let mut slot = sink.lock().unwrap_or_else(|p| p.into_inner());
            slot.get_or_insert_with(|| e.to_string());
        }));
        Ok(Arc::new(Self {
            device,
            queue,
            info,
            limits,
            uncaptured,
        }))
    }

    pub fn info(&self) -> &wgpu::AdapterInfo {
        &self.info
    }

    pub fn limits(&self) -> &wgpu::Limits {
        &self.limits
    }

    pub fn is_software(&self) -> bool {
        is_software(&self.info)
    }

    pub fn description(&self) -> String {
        describe(&self.info)
    }

    /// The first uncaptured device error, if any (it stays set: the device is unusable after it).
    pub fn take_error(&self) -> Option<String> {
        self.uncaptured
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Fails with a clear message when a buffer of `bytes` cannot be bound as a storage buffer.
    pub fn check_storage_size(&self, what: &'static str, bytes: u64) -> Result<(), GpuError> {
        let limit = self
            .limits
            .max_storage_buffer_binding_size
            .min(self.limits.max_buffer_size);
        if bytes > limit {
            return Err(GpuError::TooLarge {
                what,
                bytes,
                limit,
                adapter: self.description(),
            });
        }
        Ok(())
    }

    /// Runs `f`, turning wgpu validation and out-of-memory errors raised inside it into a
    /// `Result` (wgpu reports them asynchronously through error scopes, like a WebGPU page).
    pub fn scoped<T>(&self, f: impl FnOnce() -> T) -> Result<T, GpuError> {
        let oom = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let value = f();
        let v = pollster::block_on(validation.pop());
        let o = pollster::block_on(oom.pop());
        match (v, o) {
            (Some(e), _) => Err(GpuError::Validation(e.to_string())),
            (None, Some(e)) => Err(GpuError::OutOfMemory(e.to_string())),
            (None, None) => Ok(value),
        }
    }
}
