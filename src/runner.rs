use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        process::{CommandExt, ExitStatusExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use tempfile::{TempDir, tempdir};

use crate::problem::Problem;

const MAX_SOURCE_BYTES: u64 = 1_048_576;
const MAX_CAPTURED_STREAM_BYTES: usize = 1_048_576;
const COMPILE_TIME_LIMIT_MS: u64 = 120_000;
const COMPILE_MEMORY_LIMIT_MB: u64 = 2_048;
const COMPILE_FILE_LIMIT_MB: u64 = 512;
const COMPILE_WORKSPACE_LIMIT_MB: u64 = 512;
const COMPILE_TMP_LIMIT_MB: u64 = 64;
const RUN_FILE_LIMIT_MB: u64 = 16;
const RUN_WORKSPACE_LIMIT_MB: u64 = 64;
const RUN_TMP_LIMIT_MB: u64 = 16;
const COMPILE_SECCOMP_POLICY: &str = "POLICY restricted { DENY { mount, umount, pivot_root, chroot, ptrace, setns, unshare, bpf, perf_event_open, init_module, finit_module, delete_module, kexec_load, open_by_handle_at, userfaultfd, keyctl } } USE restricted DEFAULT ALLOW";
const RUN_SECCOMP_POLICY: &str = "POLICY restricted { DENY { mount, umount, pivot_root, chroot, ptrace, setns, unshare, bpf, perf_event_open, init_module, finit_module, delete_module, kexec_load, open_by_handle_at, userfaultfd, keyctl, clone, clone3, fork, vfork } } USE restricted DEFAULT ALLOW";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Python,
    C,
    Rust,
}

impl Language {
    fn from_source(path: &Path) -> Result<Self> {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("py") => Ok(Self::Python),
            Some("c") => Ok(Self::C),
            Some("rs") => Ok(Self::Rust),
            _ => bail!(
                "unsupported source file extension for {}; expected .py, .c, or .rs",
                path.display()
            ),
        }
    }
}

#[derive(Debug)]
struct Mount {
    source: PathBuf,
    destination: String,
    read_only: bool,
}

struct ExecutionConfig<'a> {
    time_limit_ms: u64,
    memory_limit_mb: u64,
    file_limit_mb: u64,
    workspace_limit_mb: u64,
    tmp_limit_mb: u64,
    seccomp_policy: &'a str,
    mounts: &'a [Mount],
    extra_env: &'a [String],
}

struct Sandbox {
    nsjail: PathBuf,
}

struct JailDirectory {
    root: PathBuf,
    _storage: TempDir,
}

struct PreparedSolution {
    _storage: TempDir,
    launch: Launch,
    runtime_mounts: Vec<Mount>,
}

enum Launch {
    Python { executable: String, script: String },
    Binary { path: String },
}

struct ProcessOutcome {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    elapsed: Duration,
    timed_out: bool,
    output_limit_exceeded: bool,
    stdout_truncated: bool,
    stderr_truncated: bool,
}

struct CapturedStream {
    bytes: Vec<u8>,
    truncated: bool,
}

impl JailDirectory {
    fn new() -> Result<Self> {
        let storage = tempdir().context("failed to create temporary jail directory")?;
        let root = storage.path().join("root");
        fs::create_dir(&root).context("failed to create jail root directory")?;
        Ok(Self {
            root,
            _storage: storage,
        })
    }
}

impl Drop for JailDirectory {
    fn drop(&mut self) {
        // The root is deliberately unwritable to UID 65534 while jailed. Restore
        // owner permissions so TempDir can recursively remove the temporary tree.
        let _ = set_directory_permissions(&self.root, 0o700);
    }
}

impl Sandbox {
    fn new() -> Result<Self> {
        if !cfg!(target_os = "linux") {
            bail!("code-lab execution is supported on Linux only");
        }

        let nsjail = find_executable("nsjail")
            .context("nsjail is required; refusing to run submitted code without isolation")?;
        if !Path::new("/usr").is_dir() {
            bail!("the Linux /usr directory is required by the nsjail runtime image");
        }
        Ok(Self { nsjail })
    }

    fn execute(
        &self,
        command: &[String],
        input: &[u8],
        config: ExecutionConfig<'_>,
    ) -> Result<ProcessOutcome> {
        if command.is_empty() {
            bail!("internal error: jailed command is empty");
        }

        let sandbox_dir = JailDirectory::new()?;
        let jail_root = sandbox_dir.root.clone();
        for directory in [
            "usr",
            "lib",
            "lib64",
            "tmp",
            "workspace",
            "opt/rust",
            "input",
            "solution",
            "artifact",
        ] {
            fs::create_dir_all(jail_root.join(directory))
                .with_context(|| format!("failed to prepare jail directory /{directory}"))?;
        }
        std::os::unix::fs::symlink("usr/bin", jail_root.join("bin"))
            .context("failed to create /bin link in jail")?;
        for mount in config.mounts {
            prepare_mount_target(&jail_root, mount)?;
        }
        set_directory_permissions(&jail_root, 0o555)?;

        let jail_time_limit_seconds = config.time_limit_ms.div_ceil(1_000).saturating_add(1);
        let mut process = Command::new(&self.nsjail);
        process
            .arg("--mode")
            .arg("o")
            .arg("--chroot")
            .arg(&jail_root)
            // The temporary root is writable at the mount layer for rootless nsjail,
            // but is mode 0555 and the jailed process runs as an unprivileged UID.
            .arg("--rw")
            .arg("--quiet")
            .arg("--user")
            .arg("65534")
            .arg("--group")
            .arg("65534")
            // Keep nsjail's child in our process group so the watchdog can terminate
            // the entire jail without leaving a runaway process behind.
            .arg("--skip_setsid")
            .arg("--time_limit")
            .arg(jail_time_limit_seconds.to_string())
            .arg("--rlimit_cpu")
            .arg(jail_time_limit_seconds.to_string())
            .arg("--rlimit_as")
            .arg(config.memory_limit_mb.to_string())
            .arg("--rlimit_core")
            .arg("0")
            .arg("--rlimit_fsize")
            .arg(config.file_limit_mb.to_string())
            .arg("--rlimit_nofile")
            .arg("64")
            .arg("--rlimit_nproc")
            .arg("32")
            .arg("--max_cpus")
            .arg("1")
            .arg("--seccomp_string")
            .arg(config.seccomp_policy)
            .arg("--bindmount_ro")
            .arg("/usr:/usr");

        for system_path in ["/lib", "/lib64"] {
            if Path::new(system_path).exists() {
                process
                    .arg("--bindmount_ro")
                    .arg(format!("{system_path}:{system_path}"));
            }
        }
        let workspace_size = config.workspace_limit_mb * 1024 * 1024;
        let tmp_size = config.tmp_limit_mb * 1024 * 1024;
        process
            .arg("--mount")
            .arg(format!(
                "none:/workspace:tmpfs:size={workspace_size},mode=1777"
            ))
            .arg("--mount")
            .arg(format!("none:/tmp:tmpfs:size={tmp_size},mode=1777"))
            .arg("--mount")
            .arg(format!(
                "none:/artifact:tmpfs:size={workspace_size},mode=1777"
            ));
        for mount in config.mounts {
            let option = if mount.read_only {
                "--bindmount_ro"
            } else {
                "--bindmount"
            };
            process
                .arg(option)
                .arg(format!("{}:{}", mount.source.display(), mount.destination));
        }

        process
            .arg("--cwd")
            .arg("/workspace")
            .arg("--env")
            .arg("PATH=/usr/bin:/bin")
            .arg("--env")
            .arg("HOME=/workspace")
            .arg("--env")
            .arg("TMPDIR=/workspace/tmp")
            .arg("--env")
            .arg("LC_ALL=C");
        for environment_variable in config.extra_env {
            process.arg("--env").arg(environment_variable);
        }
        process
            .arg("--")
            .arg(&command[0])
            .args(&command[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        process.process_group(0);

        let start = Instant::now();
        let mut child = process
            .spawn()
            .with_context(|| format!("failed to start nsjail at {}", self.nsjail.display()))?;
        let output_limit = Arc::new(AtomicBool::new(false));

        let stdout_thread = spawn_reader(
            child
                .stdout
                .take()
                .context("nsjail stdout pipe was not created")?,
            Arc::clone(&output_limit),
        );
        let stderr_thread = spawn_reader(
            child
                .stderr
                .take()
                .context("nsjail stderr pipe was not created")?,
            Arc::clone(&output_limit),
        );
        let stdin_thread = child.stdin.take().map(|mut stdin| {
            let input = input.to_vec();
            thread::spawn(move || {
                let _ = stdin.write_all(&input);
            })
        });

        let deadline = start + Duration::from_millis(config.time_limit_ms);
        let (status, timed_out, output_limit_exceeded) = loop {
            if let Some(status) = child.try_wait().context("failed to wait for nsjail")? {
                break (status, false, output_limit.load(Ordering::Relaxed));
            }
            if output_limit.load(Ordering::Relaxed) {
                terminate_process_group(&mut child);
                let status = child
                    .wait()
                    .context("failed to reap nsjail after output limit")?;
                break (status, false, true);
            }
            if Instant::now() >= deadline {
                terminate_process_group(&mut child);
                let status = child
                    .wait()
                    .context("failed to reap nsjail after time limit")?;
                break (status, true, false);
            }
            thread::sleep(Duration::from_millis(2));
        };

        if let Some(stdin_thread) = stdin_thread {
            let _ = stdin_thread.join();
        }
        let stdout = stdout_thread
            .join()
            .map_err(|_| anyhow::anyhow!("stdout reader thread panicked"))??;
        let stderr = stderr_thread
            .join()
            .map_err(|_| anyhow::anyhow!("stderr reader thread panicked"))??;

        let outcome = ProcessOutcome {
            status,
            stdout: stdout.bytes,
            stderr: stderr.bytes,
            elapsed: start.elapsed(),
            timed_out,
            output_limit_exceeded,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
        };
        if looks_like_sandbox_failure(&outcome) {
            bail!(
                "nsjail could not establish the sandbox; submitted code was not run. {}",
                display_stderr(&outcome)
            );
        }
        Ok(outcome)
    }
}

pub fn run_solution(problem: &Problem, source: &Path) -> Result<()> {
    let language = Language::from_source(source)?;
    let source = fs::canonicalize(source)
        .with_context(|| format!("source file {} does not exist", source.display()))?;
    let metadata = fs::metadata(&source)
        .with_context(|| format!("failed to inspect source file {}", source.display()))?;
    if !metadata.is_file() {
        bail!("source path {} is not a file", source.display());
    }
    if metadata.len() > MAX_SOURCE_BYTES {
        bail!("source file exceeds the 1 MiB source size limit");
    }

    let sandbox = Sandbox::new()?;
    let prepared = prepare_solution(&sandbox, language, &source)?;

    let mut passed = 0usize;
    for (index, test_case) in problem.test_cases.iter().enumerate() {
        let command = match &prepared.launch {
            Launch::Python { executable, script } => vec![
                executable.clone(),
                "-I".to_owned(),
                format!("/solution/{script}"),
            ],
            Launch::Binary { path } => vec![format!("/solution/{path}")],
        };

        let outcome = sandbox.execute(
            &command,
            test_case.input.as_bytes(),
            ExecutionConfig {
                time_limit_ms: problem.time_limit_ms,
                memory_limit_mb: problem.memory_limit_mb,
                file_limit_mb: RUN_FILE_LIMIT_MB,
                workspace_limit_mb: RUN_WORKSPACE_LIMIT_MB,
                tmp_limit_mb: RUN_TMP_LIMIT_MB,
                seccomp_policy: RUN_SECCOMP_POLICY,
                mounts: &prepared.runtime_mounts,
                extra_env: &[],
            },
        )?;
        let label = format!(
            "[{}/{}] {}",
            index + 1,
            problem.test_cases.len(),
            test_case.name
        );

        if outcome.timed_out {
            println!(
                "{label}: Time Limit Exceeded ({} ms)",
                outcome.elapsed.as_millis()
            );
        } else if outcome.output_limit_exceeded {
            println!("{label}: Output Limit Exceeded");
        } else if !outcome.status.success() {
            if is_memory_limit_error(&outcome.stderr) {
                println!("{label}: Memory Limit Exceeded");
            } else {
                println!(
                    "{label}: Runtime Error ({})",
                    describe_status(&outcome.status)
                );
            }
            print_diagnostics(&outcome);
        } else if outputs_match(&outcome.stdout, test_case.expected_output.as_bytes()) {
            passed += 1;
            println!("{label}: Passed ({} ms)", outcome.elapsed.as_millis());
        } else {
            println!("{label}: Wrong Answer");
            println!(
                "  expected: {:?}",
                String::from_utf8_lossy(trim_final_newline(test_case.expected_output.as_bytes()))
            );
            println!(
                "  actual:   {:?}",
                String::from_utf8_lossy(trim_final_newline(&outcome.stdout))
            );
            print_diagnostics(&outcome);
        }
    }

    if passed == problem.test_cases.len() {
        println!("Accepted: {passed}/{} test cases", problem.test_cases.len());
        Ok(())
    } else {
        bail!(
            "{} of {} test cases passed",
            passed,
            problem.test_cases.len()
        );
    }
}

fn prepare_solution(
    sandbox: &Sandbox,
    language: Language,
    source: &Path,
) -> Result<PreparedSolution> {
    let storage = tempdir().context("failed to create solution workspace")?;
    match language {
        Language::Python => {
            let script = "submission.py";
            let staged_source = storage.path().join(script);
            fs::copy(source, &staged_source).context("failed to stage Python source")?;
            set_file_permissions(&staged_source, 0o444)?;
            let executable = find_tool_in_usr("python3")?;
            Ok(PreparedSolution {
                _storage: storage,
                launch: Launch::Python {
                    executable: jail_path(&executable)?,
                    script: script.to_owned(),
                },
                runtime_mounts: vec![Mount {
                    source: staged_source,
                    destination: format!("/solution/{script}"),
                    read_only: true,
                }],
            })
        }
        Language::C => {
            let source_name = "submission.c";
            let staged_source = storage.path().join(source_name);
            fs::copy(source, &staged_source).context("failed to stage C source")?;
            set_file_permissions(&staged_source, 0o444)?;
            let artifact = storage.path().join("program");
            fs::File::create(&artifact).context("failed to prepare C output file")?;
            set_file_permissions(&artifact, 0o666)?;

            let compiler = find_tool_in_usr("gcc")?;
            let command = vec![
                jail_path(&compiler)?,
                "-std=c17".to_owned(),
                "-O2".to_owned(),
                "/input/submission.c".to_owned(),
                "-o".to_owned(),
                "/artifact/program".to_owned(),
            ];
            let mounts = [
                Mount {
                    source: staged_source,
                    destination: "/input/submission.c".to_owned(),
                    read_only: true,
                },
                Mount {
                    source: artifact.clone(),
                    destination: "/artifact/program".to_owned(),
                    read_only: false,
                },
            ];
            compile(sandbox, &command, &mounts, &[])?;
            set_file_permissions(&artifact, 0o555)?;
            Ok(PreparedSolution {
                _storage: storage,
                launch: Launch::Binary {
                    path: "program".to_owned(),
                },
                runtime_mounts: vec![Mount {
                    source: artifact,
                    destination: "/solution/program".to_owned(),
                    read_only: true,
                }],
            })
        }
        Language::Rust => {
            let source_name = "submission.rs";
            let staged_source = storage.path().join(source_name);
            fs::copy(source, &staged_source).context("failed to stage Rust source")?;
            set_file_permissions(&staged_source, 0o444)?;
            let artifact = storage.path().join("program");
            fs::File::create(&artifact).context("failed to prepare Rust output file")?;
            set_file_permissions(&artifact, 0o666)?;

            let (rustc, sysroot) = rust_toolchain()?;
            let command = vec![
                "/opt/rust/bin/rustc".to_owned(),
                "--edition=2021".to_owned(),
                "-O".to_owned(),
                "/input/submission.rs".to_owned(),
                "-o".to_owned(),
                "/artifact/program".to_owned(),
            ];
            let mounts = [
                Mount {
                    source: staged_source,
                    destination: "/input/submission.rs".to_owned(),
                    read_only: true,
                },
                Mount {
                    source: artifact.clone(),
                    destination: "/artifact/program".to_owned(),
                    read_only: false,
                },
                Mount {
                    source: sysroot,
                    destination: "/opt/rust".to_owned(),
                    read_only: true,
                },
            ];
            compile(
                sandbox,
                &command,
                &mounts,
                &["LD_LIBRARY_PATH=/opt/rust/lib".to_owned()],
            )
            .with_context(|| {
                format!(
                    "Rust compiler {} could not build the submission",
                    rustc.display()
                )
            })?;
            set_file_permissions(&artifact, 0o555)?;
            Ok(PreparedSolution {
                _storage: storage,
                launch: Launch::Binary {
                    path: "program".to_owned(),
                },
                runtime_mounts: vec![Mount {
                    source: artifact,
                    destination: "/solution/program".to_owned(),
                    read_only: true,
                }],
            })
        }
    }
}

fn compile(
    sandbox: &Sandbox,
    command: &[String],
    mounts: &[Mount],
    extra_env: &[String],
) -> Result<()> {
    let outcome = sandbox.execute(
        command,
        &[],
        ExecutionConfig {
            time_limit_ms: COMPILE_TIME_LIMIT_MS,
            memory_limit_mb: COMPILE_MEMORY_LIMIT_MB,
            file_limit_mb: COMPILE_FILE_LIMIT_MB,
            workspace_limit_mb: COMPILE_WORKSPACE_LIMIT_MB,
            tmp_limit_mb: COMPILE_TMP_LIMIT_MB,
            seccomp_policy: COMPILE_SECCOMP_POLICY,
            mounts,
            extra_env,
        },
    )?;
    if outcome.timed_out {
        bail!("Compilation Error: compilation exceeded the 120 second time limit");
    }
    if outcome.output_limit_exceeded {
        bail!("Compilation Error: compiler output exceeded the 1 MiB limit");
    }
    if !outcome.status.success() {
        eprintln!("Compilation Error");
        print_diagnostics(&outcome);
        bail!("submission did not compile");
    }
    Ok(())
}

fn rust_toolchain() -> Result<(PathBuf, PathBuf)> {
    let rustc = find_executable("rustc")?;
    // Invoke through PATH, not the canonicalized rustup proxy path: rustup
    // selects its command from argv[0], which must remain "rustc".
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .with_context(|| format!("failed to query Rust sysroot using {}", rustc.display()))?;
    if !output.status.success() {
        bail!(
            "failed to query Rust sysroot: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let sysroot = fs::canonicalize(&sysroot)
        .with_context(|| format!("Rust sysroot {} does not exist", sysroot.display()))?;
    if !sysroot.join("bin/rustc").is_file() {
        bail!("Rust compiler was not found under {}", sysroot.display());
    }
    Ok((rustc, sysroot))
}

fn find_tool_in_usr(name: &str) -> Result<PathBuf> {
    let executable = find_executable(name)?;
    if !executable.starts_with("/usr") {
        bail!("{name} must be installed under /usr to be available inside the nsjail filesystem");
    }
    Ok(executable)
}

fn find_executable(name: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return fs::canonicalize(&candidate)
                .with_context(|| format!("failed to resolve executable {}", candidate.display()));
        }
    }
    bail!("required executable '{name}' was not found in PATH")
}

fn jail_path(host_path: &Path) -> Result<String> {
    let path = host_path
        .to_str()
        .with_context(|| format!("tool path {} is not valid UTF-8", host_path.display()))?;
    if !path.starts_with("/usr/") {
        bail!("tool path {path} is outside the mounted /usr tree");
    }
    Ok(path.to_owned())
}

fn prepare_mount_target(root: &Path, mount: &Mount) -> Result<()> {
    let relative_destination = mount
        .destination
        .strip_prefix('/')
        .with_context(|| format!("mount destination {} must be absolute", mount.destination))?;
    let destination = root.join(relative_destination);
    if mount.source.is_dir() {
        fs::create_dir_all(&destination)
            .with_context(|| format!("failed to prepare mount point {}", mount.destination))?;
    } else {
        let parent = destination
            .parent()
            .context("mount destination has no parent")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to prepare mount point {}", mount.destination))?;
        fs::File::create(&destination)
            .with_context(|| format!("failed to prepare mount file {}", mount.destination))?;
    }
    Ok(())
}

fn set_directory_permissions(root: &Path, mode: u32) -> Result<()> {
    for entry in fs::read_dir(root).with_context(|| format!("failed to read {}", root.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            set_directory_permissions(&entry.path(), mode)?;
        }
    }
    let mut permissions = fs::metadata(root)?.permissions();
    permissions.set_mode(mode);
    fs::set_permissions(root, permissions)
        .with_context(|| format!("failed to set permissions on {}", root.display()))?;
    Ok(())
}

fn set_file_permissions(path: &Path, mode: u32) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("failed to set permissions on {}", path.display()))?;
    Ok(())
}

fn spawn_reader<R>(
    mut reader: R,
    output_limit: Arc<AtomicBool>,
) -> thread::JoinHandle<std::io::Result<CapturedStream>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut bytes = Vec::with_capacity(8_192);
        let mut truncated = false;
        let mut buffer = [0_u8; 8_192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            let available = MAX_CAPTURED_STREAM_BYTES.saturating_sub(bytes.len());
            let keep = count.min(available);
            bytes.extend_from_slice(&buffer[..keep]);
            if keep < count {
                truncated = true;
                output_limit.store(true, Ordering::Relaxed);
            }
        }
        Ok(CapturedStream { bytes, truncated })
    })
}

fn terminate_process_group(child: &mut Child) {
    let process_group = -(child.id() as libc::pid_t);
    // The process group includes nsjail and its supervised process (--skip_setsid).
    unsafe {
        libc::kill(process_group, libc::SIGKILL);
    }
    let _ = child.kill();
}

fn looks_like_sandbox_failure(outcome: &ProcessOutcome) -> bool {
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    [
        "Failed to build mount tree",
        "Couldn't launch the child process",
        "Couldn't prepare sandboxing policy",
        "Could not compile policy",
        "Operation not permitted",
    ]
    .iter()
    .any(|marker| stderr.contains(marker))
}

fn display_stderr(outcome: &ProcessOutcome) -> String {
    let stderr = String::from_utf8_lossy(&outcome.stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        "nsjail produced no diagnostic output".to_owned()
    } else if outcome.stderr_truncated {
        format!("{} [stderr truncated]", stderr)
    } else {
        stderr.to_owned()
    }
}

fn print_diagnostics(outcome: &ProcessOutcome) {
    if !outcome.stderr.is_empty() {
        eprintln!("{}", display_stderr(outcome));
    }
    if outcome.stdout_truncated || outcome.stderr_truncated {
        eprintln!("Diagnostic output was truncated.");
    }
}

fn is_memory_limit_error(stderr: &[u8]) -> bool {
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    [
        "memoryerror",
        "cannot allocate memory",
        "out of memory",
        "memory allocation of",
        "std::bad_alloc",
        "virtual memory exhausted",
    ]
    .iter()
    .any(|marker| stderr.contains(marker))
}

fn describe_status(status: &ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exit code {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        _ => "unknown process status".to_owned(),
    }
}

fn trim_final_newline(output: &[u8]) -> &[u8] {
    if let Some(output) = output.strip_suffix(b"\r\n") {
        output
    } else if let Some(output) = output.strip_suffix(b"\n") {
        output
    } else {
        output
    }
}

fn outputs_match(actual: &[u8], expected: &[u8]) -> bool {
    trim_final_newline(actual) == trim_final_newline(expected)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Language, is_memory_limit_error, outputs_match, trim_final_newline};

    #[test]
    fn detects_languages_by_case_insensitive_extension() {
        assert_eq!(
            Language::from_source(Path::new("solution.PY")).unwrap(),
            Language::Python
        );
        assert_eq!(
            Language::from_source(Path::new("solution.c")).unwrap(),
            Language::C
        );
        assert_eq!(
            Language::from_source(Path::new("solution.rs")).unwrap(),
            Language::Rust
        );
        assert!(Language::from_source(Path::new("solution.cpp")).is_err());
    }

    #[test]
    fn ignores_only_one_final_line_ending() {
        assert!(outputs_match(b"answer\n", b"answer"));
        assert!(outputs_match(b"answer\r\n", b"answer\n"));
        assert!(!outputs_match(b"answer  \n", b"answer\n"));
        assert!(!outputs_match(b"answer\n\n", b"answer\n"));
    }

    #[test]
    fn removes_crlf_as_a_single_line_ending() {
        assert_eq!(trim_final_newline(b"answer\r\n"), b"answer");
    }

    #[test]
    fn recognizes_common_memory_exhaustion_diagnostics() {
        assert!(is_memory_limit_error(b"MemoryError: unable to allocate"));
        assert!(!is_memory_limit_error(b"invalid pointer access"));
    }
}
