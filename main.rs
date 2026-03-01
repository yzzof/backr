use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use serde::Deserialize;
use ssh2::Session;
use std::cmp::max;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};

// CLI Arguments (override corresponding config.json fields; pi_* fields cannot be overridden)
// Flags accept a value with space or "=": -c pixz / --compression=pixz, -t /path / --target=/path
#[derive(Parser, Debug)]
#[command(about = "Streaming backup tool using tar + SSH/SFTP", disable_help_flag = true)]
struct Cli {
    /// Print help
    #[arg(short = 'h', short_aliases = ['?'], long = "help", action = clap::ArgAction::Help)]
    help: Option<bool>,

    /// Write backup to a local path instead of uploading via SSH/SFTP.
    /// Can be combined with other short flags, e.g. -lc pixz
    #[arg(short = 'l', long)]
    local_target: bool,

    /// Compression program to use: pixz (xz) or pigz (gzip). Overrides config.json.
    #[arg(short = 'c', long)]
    compression: Option<String>,

    /// Target directory for backup storage. Overrides config.json.
    #[arg(short = 't', long)]
    target: Option<String>,

    /// Path to include in backup; may be specified multiple times. Overrides config.json include.
    #[arg(short = 'i', long)]
    include: Vec<String>,

    /// Path to exclude from backup; may be specified multiple times. Overrides config.json exclude.
    #[arg(short = 'e', long)]
    exclude: Vec<String>,
}

// Load Configuration Struct
#[derive(Deserialize, Debug)]
struct Config {
    ssh_host: String,
    ssh_port: Option<u16>,
    ssh_user: String,
    ssh_private_key_path: Option<String>,
    ssh_password: Option<String>,
    target: String,
    compression: Option<String>,
    include: Vec<String>,
    exclude: Vec<String>,
}

// Helper: Resolve `~` to the home directory for paths
fn resolve_path(p: &str) -> String {
    if p.starts_with('~') {
        if let Some(mut home) = dirs::home_dir() {
            let without_tilde = p
                .strip_prefix("~/")
                .unwrap_or_else(|| p.strip_prefix('~').unwrap_or(p));
            if !without_tilde.is_empty() {
                home.push(without_tilde);
            }
            return home.to_string_lossy().to_string();
        }
    }
    p.to_string()
}

// Helper: Check if a program exists on PATH (handles .exe suffix on Windows)
fn is_installed(program: &str) -> bool {
    let exe = format!("{}{}", program, std::env::consts::EXE_SUFFIX);
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(&exe).is_file()))
        .unwrap_or(false)
}

// Helper: Detect a supported package manager and install a package
fn try_install(package: &str) -> Result<()> {
    // (manager binary, install subcommand args, needs sudo)
    let managers: &[(&str, &[&str], bool)] = &[
        ("apt-get", &["install", "-y"], true),
        ("dnf",     &["install", "-y"], true),
        ("pacman",  &["-S", "--noconfirm"], true),
        ("brew",    &["install"],        false),
    ];

    let (manager, args, needs_sudo) = managers
        .iter()
        .find(|(bin, _, _)| is_installed(bin))
        .ok_or_else(|| anyhow::anyhow!(
            "No supported package manager found (tried apt-get, dnf, pacman, brew)"
        ))?;

    let status = if *needs_sudo {
        Command::new("sudo").arg(manager).args(*args).arg(package).status()
    } else {
        Command::new(manager).args(*args).arg(package).status()
    }
    .with_context(|| format!("Failed to run {} install", manager))?;

    if !status.success() {
        anyhow::bail!("Installation of '{}' failed", package);
    }
    Ok(())
}

// Helper: Get formatted timestamp matching Node.js ISOString logic
fn get_timestamp() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string()
}

// Main Backup Logic
fn run_backup(config: &Config, local_target: bool) -> Result<()> {
    let hostname = hostname::get()?
        .to_string_lossy()
        .replace(|c: char| c.is_whitespace(), "_");
    let timestamp = get_timestamp();

    let compression = config.compression.as_deref().unwrap_or("pixz");

    // Ensure the compression program is available, offering to install it if not
    if !is_installed(compression) {
        eprint!("⚠️  '{}' is not installed. Install it now? [y/N] ", compression);
        io::stderr().flush().ok();
        let mut response = String::new();
        io::stdin().read_line(&mut response).with_context(|| "Failed to read input")?;
        if response.trim().eq_ignore_ascii_case("y") {
            try_install(compression)?;
            if !is_installed(compression) {
                anyhow::bail!("'{}' still not found after installation", compression);
            }
            eprintln!("✅ '{}' installed successfully.", compression);
        } else {
            anyhow::bail!("'{}' is required but not installed", compression);
        }
    }

    // Choose archive extension based on compression program
    let ext = match compression {
        "pigz" => "gz",
        _ => "xz",
    };
    let archive_name = format!("{}_backup_{}.tar.{}", hostname, timestamp, ext);

    // 1. Prepare include paths
    let mut valid_paths = Vec::new();
    for p in &config.include {
        let resolved = resolve_path(p);
        if Path::new(&resolved).exists() {
            valid_paths.push(resolved);
        } else {
            eprintln!("⚠️  Warning: Path not found, skipping: {}", p);
        }
    }

    if valid_paths.is_empty() {
        anyhow::bail!("No valid backup paths found.");
    }

    let output_path = format!(
        "{}/{}",
        config.target.trim_end_matches('/'),
        archive_name
    );

    println!("\n📦 Starting Streamed Backup for host: {}", hostname);
    println!("🔥 Compression: {} (Multi-threaded)", compression);

    // 2. Build tar command (common to both local and remote paths)
    let is_gnu_tar = std::env::consts::OS == "linux";
    let compress_flag = if is_gnu_tar { "-I" } else { "--use-compress-program" };
    let cpus = max(1, num_cpus::get() / 2);
    let compress_cmd = format!("{} -p {}", compression, cpus);

    let mut cmd = Command::new("tar");
    cmd.arg(compress_flag).arg(compress_cmd);
    for p in &config.exclude {
        cmd.arg("--exclude").arg(p);
    }
    cmd.arg("-cvf").arg("-"); // Create, write to stdout
    for p in &valid_paths {
        cmd.arg(p);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    if local_target {
        // 3a. Local path: create the output file, then stream tar into it
        println!("💾 Destination: {}", output_path);

        let mut file = fs::File::create(&output_path)
            .with_context(|| format!("Failed to create local file: {}", output_path))?;

        let mut tar_process = cmd.spawn().with_context(|| "Failed to spawn tar process")?;

        if let Some(mut stdout) = tar_process.stdout.take() {
            io::copy(&mut stdout, &mut file)
                .with_context(|| "Failed while writing to local file")?;
        }

        let status = tar_process.wait()?;
        if !status.success() {
            eprintln!(
                "❌ Tar exited with {}. Check if '{}' is installed.",
                status, compression
            );
            anyhow::bail!("Tar process failed");
        }

        println!("✅ Local backup written successfully.");
    } else {
        // 3b. Remote path: establish SSH/SFTP, then stream tar into the remote file
        println!("📡 Destination: {}:{}", config.ssh_host, output_path);

        let port = config.ssh_port.unwrap_or(22);
        let tcp = TcpStream::connect(format!("{}:{}", config.ssh_host, port))
            .with_context(|| format!("Failed to connect to {}:{}", config.ssh_host, port))?;

        let mut session = Session::new()?;
        session.set_tcp_stream(tcp);
        session
            .handshake()
            .with_context(|| "Failed SSH handshake")?;

        if let Some(key_path) = &config.ssh_private_key_path {
            let resolved_key_path = resolve_path(key_path);
            session
                .userauth_pubkey_file(&config.ssh_user, None, Path::new(&resolved_key_path), None)
                .with_context(|| {
                    format!(
                        "Failed to authenticate with private key: {}",
                        resolved_key_path
                    )
                })?;
        } else if let Some(pass) = &config.ssh_password {
            session
                .userauth_password(&config.ssh_user, pass)
                .with_context(|| "Failed to authenticate with password")?;
        } else {
            anyhow::bail!("No authentication method provided in config.json");
        }

        if !session.authenticated() {
            anyhow::bail!("SSH Authentication failed");
        }

        println!("✅ SSH Connection established. Starting stream...");

        let sftp = session
            .sftp()
            .with_context(|| "Failed to initialize SFTP session")?;
        let mut remote_stream = sftp
            .create(Path::new(&output_path))
            .with_context(|| format!("Failed to create remote file at {}", output_path))?;

        let mut tar_process = cmd.spawn().with_context(|| "Failed to spawn tar process")?;

        if let Some(mut stdout) = tar_process.stdout.take() {
            io::copy(&mut stdout, &mut remote_stream)
                .with_context(|| "Failed while streaming data to remote file")?;
        }

        let status = tar_process.wait()?;
        if !status.success() {
            eprintln!(
                "❌ Tar exited with {}. Check if '{}' is installed.",
                status, compression
            );
            anyhow::bail!("Tar process failed");
        }

        println!("✅ Local compression finished.");
        println!("✅ Upload stream closed successfully.");
    }

    Ok(())
}

fn main() {
    // Parse CLI args; print full help and exit on any error (unknown flags, missing =, etc.)
    let cli = Cli::try_parse().unwrap_or_else(|e| {
        match e.kind() {
            // --help / -h / -? handled by clap directly (exits 0)
            clap::error::ErrorKind::DisplayHelp => e.exit(),
            _ => {
                let _ = Cli::command().print_help();
                eprintln!("\n\nerror: {}", e.kind());
                std::process::exit(2);
            }
        }
    });

    // Determine config path relative to execution directory
    let config_path = env::current_dir().unwrap_or_default().join("config.json");

    let config_content = if config_path.exists() {
        match fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("❌ Error loading config.json: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        let example_path = env::current_dir().unwrap_or_default().join("config.example.json");
        match fs::write(&example_path, include_str!("config.example.json")) {
            Ok(_) => {
                eprintln!("❌ config.json not found.");
                eprintln!("   A template has been written to: {}", example_path.display());
                eprintln!("   Copy it to config.json, fill in your values, and run backr again.");
            }
            Err(e) => {
                eprintln!("❌ config.json not found and could not write config.example.json: {}", e);
            }
        }
        std::process::exit(1);
    };

    let mut config: Config = match serde_json::from_str(&config_content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("❌ Error parsing config.json: {}", e);
            std::process::exit(1);
        }
    };

    // Apply CLI overrides (pi_* fields are intentionally not overridable via CLI)
    if let Some(compression) = cli.compression {
        config.compression = Some(compression);
    }
    if let Some(target) = cli.target {
        config.target = target;
    }
    if !cli.include.is_empty() {
        config.include = cli.include;
    }
    if !cli.exclude.is_empty() {
        config.exclude = cli.exclude;
    }

    // Run the backup and catch bubbling errors
    if let Err(e) = run_backup(&config, cli.local_target) {
        eprintln!("\n💥 Backup failed: {:#}", e);
        std::process::exit(1);
    } else {
        println!("\n🎉 Backup completed successfully!");
    }
}
