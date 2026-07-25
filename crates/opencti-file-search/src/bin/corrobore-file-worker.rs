// Copyright (c) 2026 AreDee-Bangs
// SPDX-License-Identifier: MIT

use std::{
    env,
    path::PathBuf,
    process::{Child, Command, ExitCode},
    thread,
    time::{Duration, Instant, SystemTime},
};

use opencti_file_search::{
    ExtractionLimits, FileExtractionWorker, FileJobStore, FilesystemBlobSource,
};

const DEFAULT_MAX_ATTEMPTS: u32 = 3;
const DEFAULT_LEASE_MS: u64 = 60_000;
const DEFAULT_MAX_RUNTIME_MS: u64 = 30_000;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;

#[derive(Clone, Debug)]
struct WorkerConfig {
    metadata_dir: PathBuf,
    blob_root: PathBuf,
    max_attempts: u32,
    lease_ms: u64,
    max_runtime_ms: u64,
    poll_interval_ms: u64,
    limits: ExtractionLimits,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("corrobore file worker failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let config = WorkerConfig::from_environment()?;
    match env::args().nth(1).as_deref() {
        Some("--extract-once") => extract_once(&config),
        Some("--supervise-once") => supervise_child(&config),
        Some(argument) => Err(format!("unsupported argument: {argument}")),
        None => loop {
            supervise_child(&config)?;
            thread::sleep(Duration::from_millis(config.poll_interval_ms));
        },
    }
}

impl WorkerConfig {
    fn from_environment() -> Result<Self, String> {
        let mut limits = ExtractionLimits::default();
        limits.max_input_bytes =
            optional_number("CORROBORE_FILE_MAX_INPUT_BYTES", limits.max_input_bytes)?;
        limits.max_extracted_bytes = optional_number(
            "CORROBORE_FILE_MAX_EXTRACTED_BYTES",
            limits.max_extracted_bytes,
        )?;
        limits.max_pages = optional_number("CORROBORE_FILE_MAX_PAGES", limits.max_pages)?;
        limits.max_sheets = optional_number("CORROBORE_FILE_MAX_SHEETS", limits.max_sheets)?;
        limits.max_rows_per_sheet = optional_number(
            "CORROBORE_FILE_MAX_ROWS_PER_SHEET",
            limits.max_rows_per_sheet,
        )?;
        limits.max_cells = optional_number("CORROBORE_FILE_MAX_CELLS", limits.max_cells)?;
        limits.max_chunks = optional_number("CORROBORE_FILE_MAX_CHUNKS", limits.max_chunks)?;
        limits.max_chunk_chars =
            optional_number("CORROBORE_FILE_MAX_CHUNK_CHARS", limits.max_chunk_chars)?;
        let config = Self {
            metadata_dir: required_path("CORROBORE_FILE_METADATA_DIR")?,
            blob_root: required_path("CORROBORE_FILE_BLOB_ROOT")?,
            max_attempts: optional_number("CORROBORE_FILE_MAX_ATTEMPTS", DEFAULT_MAX_ATTEMPTS)?,
            lease_ms: optional_number("CORROBORE_FILE_LEASE_MS", DEFAULT_LEASE_MS)?,
            max_runtime_ms: optional_number(
                "CORROBORE_FILE_MAX_RUNTIME_MS",
                DEFAULT_MAX_RUNTIME_MS,
            )?,
            poll_interval_ms: optional_number(
                "CORROBORE_FILE_POLL_INTERVAL_MS",
                DEFAULT_POLL_INTERVAL_MS,
            )?,
            limits,
        };
        if config.max_runtime_ms == 0
            || config.poll_interval_ms == 0
            || config.max_runtime_ms >= config.lease_ms
        {
            return Err(
                "worker runtime and poll interval must be non-zero, and runtime must be shorter than its lease"
                    .to_owned(),
            );
        }
        Ok(config)
    }
}

fn extract_once(config: &WorkerConfig) -> Result<(), String> {
    let mut store = FileJobStore::open(
        config.metadata_dir.clone(),
        config.max_attempts,
        config.lease_ms,
    )
    .map_err(|error| error.to_string())?;
    let source = FilesystemBlobSource::new(config.blob_root.clone());
    let mut worker =
        FileExtractionWorker::new(source, config.limits.clone(), config.max_runtime_ms)
            .map_err(|error| error.to_string())?;
    let outcome = worker
        .run_once(&mut store, now_ms()?)
        .map_err(|error| error.to_string())?;
    eprintln!("corrobore file worker outcome: {outcome:?}");
    Ok(())
}

fn supervise_child(config: &WorkerConfig) -> Result<(), String> {
    let executable = env::current_exe().map_err(|error| error.to_string())?;
    let mut child = Command::new(executable)
        .arg("--extract-once")
        .spawn()
        .map_err(|error| format!("could not start isolated extractor: {error}"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("isolated extractor exited with {status}"))
            };
        }
        if u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX) >= config.max_runtime_ms
        {
            terminate(&mut child)?;
            eprintln!("corrobore file worker killed an extractor after its deadline");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn terminate(child: &mut Child) -> Result<(), String> {
    child
        .kill()
        .map_err(|error| format!("could not kill timed-out extractor: {error}"))?;
    child
        .wait()
        .map_err(|error| format!("could not reap timed-out extractor: {error}"))?;
    Ok(())
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|value| !value.as_os_str().is_empty())
        .ok_or_else(|| format!("{name} must be set"))
}

fn optional_number<T>(name: &str, default: T) -> Result<T, String>
where
    T: std::str::FromStr,
{
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|_| format!("{name} must be a valid positive integer")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(format!("could not read {name}: {error}")),
    }
}

fn now_ms() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    u64::try_from(duration.as_millis()).map_err(|_| "system clock is out of range".to_owned())
}
