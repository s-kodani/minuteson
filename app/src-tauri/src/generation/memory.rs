//! Admission is separate from the frozen logical plan. Estimates include the
//! complete prospective allocation; a warm resident context is never charged
//! against free memory for a second time.
use super::llama::LlamaConfig;
use crate::local_engine::resident::{self, Estimator, Snapshot};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;

const GIB: u64 = 1024 * 1024 * 1024;
const ALIGNMENT: u32 = 256;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Device {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    shared_memory: bool,
    total: u64,
    free: u64,
    model_bytes: u64,
    context_bytes: u64,
    compute_bytes: u64,
}
impl Device {
    fn required(&self) -> Result<u64, String> {
        self.model_bytes
            .checked_add(self.context_bytes)
            .and_then(|n| n.checked_add(self.compute_bytes))
            .ok_or_else(invalid_estimate)
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Estimate {
    schema_version: u32,
    context_tokens: u32,
    gpu_layers: u32,
    devices: Vec<Device>,
}
fn invalid_estimate() -> String {
    "議事録のメモリ見積もりが不正です。ランタイムを更新してください。（minutes_memory_unavailable）"
        .into()
}
fn unavailable() -> String {
    "議事録生成に利用できるメモリ量を取得できません。生成前に停止しました。（minutes_memory_unavailable）".into()
}

#[derive(Clone, Copy, Debug)]
struct Budget {
    available: u64,
    usable: u64,
}
impl Budget {
    fn new(available: u64) -> Result<Self, String> {
        if available == 0 {
            return Err(unavailable());
        }
        Ok(Self {
            available,
            usable: available.saturating_sub(GIB.max(available / 5)),
        })
    }
}

#[cfg(windows)]
fn available() -> Result<u64, String> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return Err(unavailable());
    }
    let bytes = status.ullAvailPhys.min(status.ullAvailPageFile);
    if bytes == 0
        || status.ullAvailPhys > status.ullTotalPhys
        || status.ullAvailPageFile > status.ullTotalPageFile
    {
        return Err(unavailable());
    }
    Ok(bytes)
}

fn page_availability(free: u64, inactive: u64, page: u64, total: u64) -> Result<u64, String> {
    if page == 0 || total == 0 {
        return Err(unavailable());
    }
    free.checked_add(inactive)
        .and_then(|n| n.checked_mul(page))
        .map(|n| n.min(total))
        .filter(|n| *n > 0)
        .ok_or_else(unavailable)
}

#[cfg(target_os = "macos")]
fn available() -> Result<u64, String> {
    // Speculative pages are already part of free_count. Purgeable/compressed
    // pages must not be added separately to these reclaimable page counts.
    unsafe extern "C" {
        fn mach_port_deallocate(
            task: libc::mach_port_t,
            name: libc::mach_port_t,
        ) -> libc::kern_return_t;
    }
    struct Host(libc::mach_port_t);
    impl Drop for Host {
        fn drop(&mut self) {
            unsafe {
                mach_port_deallocate(libc::mach_task_self(), self.0);
            }
        }
    }
    let host = Host(unsafe { libc::mach_host_self() });
    if host.0 == 0 {
        return Err(unavailable());
    }
    let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    if unsafe {
        libc::host_statistics64(
            host.0,
            libc::HOST_VM_INFO64,
            (&mut stats as *mut libc::vm_statistics64).cast(),
            &mut count,
        )
    } != 0
        || count < 4
    {
        return Err(unavailable());
    }
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let mut total = 0_u64;
    let mut size = std::mem::size_of_val(&total);
    if page <= 0
        || unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut total as *mut u64).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        || size != 8
    {
        return Err(unavailable());
    }
    page_availability(
        stats.free_count.into(),
        stats.inactive_count.into(),
        page as u64,
        total,
    )
}
#[cfg(not(any(windows, target_os = "macos")))]
fn available() -> Result<u64, String> {
    Err(unavailable())
}

impl Estimate {
    fn parse(bytes: &[u8], config: &LlamaConfig, device: Option<&str>) -> Result<Self, String> {
        let e: Self = serde_json::from_slice(bytes).map_err(|_| invalid_estimate())?;
        if e.schema_version != 1
            || e.context_tokens != config.context_tokens
            || e.gpu_layers > config.gpu_layers
            || (device.is_some() && e.gpu_layers == 0)
            || e.devices.is_empty()
            || e.devices.len() > 65
            || (device.is_none() && (e.gpu_layers != 0 || e.devices.len() != 1))
        {
            return Err(invalid_estimate());
        }
        for (index, d) in e.devices.iter().enumerate() {
            let host = index + 1 == e.devices.len();
            if d.name.is_empty()
                || d.name.len() > 256
                || d.name.contains('\0')
                || d.free > d.total
                || (host && (d.kind != "host" || d.shared_memory))
                || (!host
                    && (!matches!(d.kind.as_str(), "gpu" | "integratedGpu")
                        || Some(d.name.as_str()) != device))
                || (d.kind == "integratedGpu" && !d.shared_memory)
            {
                return Err(invalid_estimate());
            }
            d.required()?;
        }
        if e.devices.last().unwrap().required()? == 0 && device.is_none() {
            return Err(invalid_estimate());
        }
        Ok(e)
    }
    fn check(&self, budget: Budget) -> Result<(), String> {
        let mut host_required = self
            .devices
            .last()
            .ok_or_else(invalid_estimate)?
            .required()?;
        for d in &self.devices[..self.devices.len() - 1] {
            let required = d.required()?;
            // Device memory is an additional constraint, never extra host RAM.
            let usable = Budget::new(d.free)?.usable;
            if required > usable {
                return Err(insufficient(&d.name, required, d.free, usable));
            }
            if d.shared_memory {
                host_required = host_required
                    .checked_add(required)
                    .ok_or_else(invalid_estimate)?;
            }
        }
        if host_required > budget.usable {
            return Err(insufficient(
                "CPU/共有メモリ",
                host_required,
                budget.available,
                budget.usable,
            ));
        }
        Ok(())
    }
}
fn insufficient(device: &str, required: u64, available: u64, usable: u64) -> String {
    format!("議事録生成のメモリが不足しています（{device}：必要 {:.2} GiB、利用可能 {:.2} GiB、安全余裕を除く上限 {:.2} GiB）。他の処理を終了して再試行するか、新しく生成して分割サイズを調整してください。保存済みの結果は保持しています。（minutes_memory_insufficient）",
        required as f64 / GIB as f64, available as f64 / GIB as f64, usable as f64 / GIB as f64)
}
fn arguments(config: &LlamaConfig, device: Option<&str>) -> Result<Vec<String>, String> {
    let model = config.model.path.to_str().ok_or_else(invalid_estimate)?;
    let mut args = vec![
        "--model".into(),
        model.into(),
        "--ctx-size".into(),
        config.context_tokens.to_string(),
        "--gpu-layers".into(),
        config.gpu_layers.to_string(),
        "--threads".into(),
        config.threads.to_string(),
        "--threads-batch".into(),
        config.threads.to_string(),
        "--offline".into(),
        "--fit".into(),
        "off".into(),
        "--no-warmup".into(),
        "--log-verbosity".into(),
        "1".into(),
    ];
    if let Some(device) = device {
        args.extend(["--device".into(), device.into()]);
    } else {
        args.extend([
            "--device".into(),
            "none".into(),
            "--no-kv-offload".into(),
            "--no-op-offload".into(),
        ]);
    }
    Ok(args)
}
fn estimate(
    session: &mut Estimator,
    config: &LlamaConfig,
    device: Option<&str>,
    cancelled: &impl Fn() -> bool,
) -> Result<Estimate, String> {
    let bytes = session.run(arguments(config, device)?, device.is_some(), cancelled)
        .map_err(|e| format!("議事録のメモリを見積もれません。ランタイムを確認してください: {e}（minutes_memory_unavailable）"))?;
    Estimate::parse(&bytes, config, device)
}
fn diagnostic(config: &LlamaConfig, device: Option<&str>, e: &Estimate, b: Budget, outcome: &str) {
    crate::internal_logs::detail(crate::internal_logs::current_context("minutes_generation"), "memory", "info", Duration::ZERO,
        &json!({"event":"議事録メモリ確認","contextTokens":config.context_tokens,"gpuLayers":config.gpu_layers,
            "device":device.unwrap_or("CPU"),"availableBytes":b.available,"usableBytes":b.usable,
            "estimatedBytes":e.devices.iter().map(|d| d.required().unwrap_or(u64::MAX)).collect::<Vec<_>>(),
            "outcome":outcome}).to_string());
}

/// Pin a CPU-safe cap before constructing any immutable extraction/merge plan.
pub(super) fn select_context(
    engine: &LlamaConfig,
    snapshot: Option<&Snapshot>,
    desired_context: u32,
    min_context: u32,
    model_max: u32,
) -> Result<u32, String> {
    let upper = desired_context.min(model_max).min(262144) / ALIGNMENT * ALIGNMENT;
    let lower = min_context
        .max(2048)
        .checked_add(ALIGNMENT - 1)
        .ok_or_else(invalid_estimate)?
        / ALIGNMENT
        * ALIGNMENT;
    if upper < lower {
        return Err("議事録の入力と出力に必要な処理枠がありません".into());
    }
    #[cfg(test)]
    if !resident::enabled() || resident::synthetic_enabled() {
        return Ok(upper);
    }
    let mut session = Estimator::open(snapshot)?;
    let budget = Budget::new(available()?)?;
    let mut config = engine.clone();
    config.gpu_layers = 0;
    config.prediction_tokens = 64;
    let started = std::time::Instant::now();
    let mut probe = |context| -> Result<bool, String> {
        config.context_tokens = context;
        let e = estimate(&mut session, &config, None, &|| {
            started.elapsed() > Duration::from_secs(60)
        })?;
        let result = e.check(budget);
        diagnostic(
            &config,
            None,
            &e,
            budget,
            if result.is_ok() {
                "fits"
            } else {
                "insufficient"
            },
        );
        match result {
            Ok(()) => Ok(true),
            Err(error) if error.contains("minutes_memory_insufficient") => Ok(false),
            Err(error) => Err(error),
        }
    };
    let selected = largest_fitting(lower, upper, &mut probe)?;
    match selected {
        Some(context) => Ok(context),
        None => {
            config.context_tokens = lower;
            let e = estimate(&mut session, &config, None, &|| {
                started.elapsed() > Duration::from_secs(60)
            })?;
            e.check(budget)?;
            Err(invalid_estimate())
        }
    }
}

fn largest_fitting(
    lower: u32,
    upper: u32,
    fits: &mut impl FnMut(u32) -> Result<bool, String>,
) -> Result<Option<u32>, String> {
    if fits(upper)? {
        return Ok(Some(upper));
    }
    if lower == upper || !fits(lower)? {
        return Ok(None);
    }
    let mut low = lower / ALIGNMENT;
    let mut high = upper / ALIGNMENT - 1;
    while low < high {
        let mid = low + (high - low + 1) / 2;
        if fits(mid * ALIGNMENT)? {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    Ok(Some(low * ALIGNMENT))
}

/// Called immediately before launching a cold generation worker, including
/// compatibility checks and frozen legacy jobs. Never changes their context.
pub(super) fn guard(
    config: &LlamaConfig,
    device: Option<&str>,
    cancelled: &impl Fn() -> bool,
) -> Result<(), String> {
    #[cfg(test)]
    if !resident::enabled() || resident::synthetic_enabled() {
        return Ok(());
    }
    if resident::generation_ready_for(&config.model.path, config.context_tokens, config.gpu_layers)
    {
        return Ok(());
    }
    let mut session = Estimator::open(None)?;
    let e = estimate(&mut session, config, device, cancelled)?;
    // Refresh after GPU teardown/probing; never reuse the enqueue-time value.
    let budget = Budget::new(available()?)?;
    let result = e.check(budget);
    diagnostic(
        config,
        device,
        &e,
        budget,
        if result.is_ok() {
            "admitted"
        } else {
            "insufficient"
        },
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn host(bytes: u64) -> Device {
        Device {
            kind: "host".into(),
            name: "CPU".into(),
            shared_memory: false,
            total: 32 * GIB,
            free: 16 * GIB,
            model_bytes: bytes,
            context_bytes: 0,
            compute_bytes: 0,
        }
    }
    fn e(devices: Vec<Device>) -> Estimate {
        Estimate {
            schema_version: 1,
            context_tokens: 131072,
            gpu_layers: 0,
            devices,
        }
    }
    #[test]
    fn reserve_and_commit_limit_leave_real_headroom() {
        assert_eq!(Budget::new(GIB).unwrap().usable, 0);
        assert_eq!(Budget::new(4 * GIB).unwrap().usable, 3 * GIB);
        assert_eq!(Budget::new(10 * GIB).unwrap().usable, 8 * GIB);
        assert!(Budget::new(0).is_err());
        let physical = 20 * GIB;
        let commit = 4 * GIB;
        assert!(e(vec![host(4 * GIB)])
            .check(Budget::new(physical.min(commit)).unwrap())
            .is_err());
    }
    #[test]
    fn shared_gpu_allocations_do_not_create_extra_host_memory() {
        let mut gpu = host(3 * GIB);
        gpu.kind = "integratedGpu".into();
        gpu.name = "Vulkan0".into();
        gpu.shared_memory = true;
        assert!(e(vec![gpu.clone(), host(3 * GIB)])
            .check(Budget::new(6 * GIB).unwrap())
            .is_err());
        gpu.shared_memory = false;
        gpu.kind = "gpu".into();
        assert!(e(vec![gpu, host(3 * GIB)])
            .check(Budget::new(6 * GIB).unwrap())
            .is_ok());
    }
    #[test]
    fn memory_drop_between_planning_and_fallback_is_rejected_before_allocation() {
        let estimate = e(vec![host(6 * GIB)]);
        assert!(estimate.check(Budget::new(10 * GIB).unwrap()).is_ok());
        let error = estimate.check(Budget::new(5 * GIB).unwrap()).unwrap_err();
        assert!(error.contains("minutes_memory_insufficient") && error.contains("6.00"));
    }
    #[test]
    fn pages_do_not_include_speculative_twice_and_overflow_is_rejected() {
        assert_eq!(page_availability(2, 3, 4096, GIB).unwrap(), 5 * 4096);
        assert_eq!(page_availability(1000, 1000, 4096, 4096).unwrap(), 4096);
        for (free, inactive, page, total) in [
            (u64::MAX, 1, 4096, GIB),
            (1, 1, u64::MAX, GIB),
            (1, 1, 0, GIB),
            (0, 0, 4096, GIB),
        ] {
            assert!(page_availability(free, inactive, page, total).is_err());
        }
    }
    #[test]
    fn cap_search_is_bounded_aligned_and_never_tries_real_allocations() {
        let mut calls = 0;
        let cap = largest_fitting(10752, 131072, &mut |ctx| {
            calls += 1;
            Ok(ctx <= 30000)
        })
        .unwrap()
        .unwrap();
        assert_eq!(cap, 29952);
        assert!(calls <= 12);
        assert_eq!(
            largest_fitting(10752, 131072, &mut |_| Ok(false)).unwrap(),
            None
        );
        assert_eq!(
            largest_fitting(10752, 131072, &mut |_| Ok(true)).unwrap(),
            Some(131072)
        );
    }
    #[test]
    fn native_estimate_validation_rejects_overflow_and_false_cpu_placement() {
        let config = LlamaConfig {
            executable: crate::local_engine::Fingerprint {
                path: "/not-used".into(),
                size: 1,
                sha256: "a".repeat(64),
            },
            model: crate::local_engine::Fingerprint {
                path: "/model".into(),
                size: 1,
                sha256: "b".repeat(64),
            },
            threads: 1,
            context_tokens: 131072,
            prediction_tokens: 64,
            gpu_layers: 0,
            seed: 1,
        };
        let valid = json!({"schemaVersion":1,"contextTokens":131072,"gpuLayers":0,"devices":[{"type":"host","name":"CPU","sharedMemory":false,"total":32*GIB,"free":16*GIB,"modelBytes":2*GIB,"contextBytes":16*GIB,"computeBytes":GIB}]});
        let estimate =
            Estimate::parse(&serde_json::to_vec(&valid).unwrap(), &config, None).unwrap();
        assert_eq!(estimate.devices[0].context_bytes, 16 * GIB);
        for change in ["overflow", "context", "version", "device", "shared", "free"] {
            let mut bad = valid.clone();
            match change {
                "overflow" => bad["devices"][0]["modelBytes"] = json!(u64::MAX),
                "context" => bad["contextTokens"] = json!(2048),
                "version" => bad["schemaVersion"] = json!(2),
                "device" => bad["devices"][0]["type"] = json!("gpu"),
                "shared" => bad["devices"][0]["sharedMemory"] = json!(true),
                _ => bad["devices"][0]["free"] = json!(64 * GIB),
            }
            assert!(
                Estimate::parse(&serde_json::to_vec(&bad).unwrap(), &config, None).is_err(),
                "{change}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires MON_MINUTES_PHI_MODEL and rebuilt bundled CPU/Vulkan workers; isolated synthetic input only"]
    fn windows_phi_selects_small_context_and_generates_after_gpu_fallback() {
        use crate::local_engine::Fingerprint;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let mut snapshot = resident::prepare(&path, "llama").unwrap().unwrap();
        // Force GPU unavailability in this isolated job, without changing any
        // installed runtime, saved meeting, global setting or environment.
        snapshot.gpu = None;
        let model = Fingerprint::capture(
            &std::path::PathBuf::from(std::env::var_os("MON_MINUTES_PHI_MODEL").unwrap()),
            &|| false,
        )
        .unwrap();
        assert_eq!(
            model.sha256,
            "01999f17c39cc3074afae5e9c539bc82d45f2dd7faa3917c66cbef76fce8c0c2"
        );
        let mut config = LlamaConfig {
            executable: Fingerprint::capture(
                &crate::windows_runtime::resources()
                    .unwrap()
                    .join("llama/llama-completion.exe"),
                &|| false,
            )
            .unwrap(),
            model,
            threads: 2,
            context_tokens: 131072,
            prediction_tokens: 64,
            gpu_layers: 0,
            seed: 42,
        };
        let _scope = resident::Scope::enter(&path, Some(&snapshot)).unwrap();
        let mut estimator = Estimator::open(Some(&snapshot)).unwrap();
        let maximum = estimate(&mut estimator, &config, None, &|| false).unwrap();
        assert_eq!(maximum.devices.last().unwrap().context_bytes, 16 * GIB);
        drop(estimator);
        let capacity = super::super::model_budget::Capacity::for_new_job(&config.model).unwrap();
        let policy =
            super::super::optimization::memory_aware_policy(&config, Some(&snapshot)).unwrap();
        let token_scope = super::super::optimization::Scope::enter(
            &capacity.token_configuration(&config).unwrap(),
            Some(&policy),
        )
        .unwrap();
        let input = super::super::optimization::count(
            super::super::acceleration::SYSTEM,
            super::super::acceleration::PROMPT,
        )
        .unwrap()
        .unwrap();
        let desired = capacity.desired_context(input).unwrap();
        let selected = select_context(
            &config,
            Some(&snapshot),
            desired,
            capacity.minimum_context(128).unwrap(),
            capacity.maximum_context(),
        )
        .unwrap();
        assert!(selected < 131072 && selected <= desired);
        config.context_tokens = selected;
        drop(token_scope);
        resident::discard_trial();
        let gpu_directory = path.join("gpu-trial");
        std::fs::create_dir(&gpu_directory).unwrap();
        let resolved = super::super::acceleration::resolve(
            super::super::acceleration::Policy::AutoVulkanV1,
            &config,
            selected,
            &gpu_directory,
            &|| false,
        )
        .unwrap()
        .unwrap();
        assert!(matches!(
            resolved,
            super::super::acceleration::Resolved::Cpu { .. }
        ));
        let directory = path.join("cpu-generation");
        std::fs::create_dir(&directory).unwrap();
        let schema = json!({"type":"object","properties":{"ok":{"type":"boolean","const":true}},"required":["ok"],"additionalProperties":false});
        let value = super::super::llama::run_unit_observed_on_device(
            &config,
            &directory,
            super::super::acceleration::SYSTEM,
            super::super::acceleration::PROMPT,
            &schema,
            Duration::from_secs(120),
            &|| false,
            &|_| {},
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(value, json!({"ok":true}));
        // Reusing the admitted full context must not charge it to free RAM twice.
        assert!(resident::generation_ready_for(
            &config.model.path,
            selected,
            0
        ));
        guard(&config, None, &|| false).unwrap();
        eprintln!("Phi guarded CPU fallback: inputTokens={input} contextTokens={selected} maximumKvBytes={} selectedEstimateBudget={:?}",16*GIB,Budget::new(available().unwrap()).unwrap());
    }
}
