use anyhow::{Context, Result};
use serde::Deserialize;
use ssh2::Session;
use std::cmp::max;
use std::env;
use std::fs;
use std::io;
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};

// Load Configuration Struct
#[derive(Deserialize, Debug)]
struct Config {
    pi_host: String,
    pi_port: Option<u16>,
    pi_user: String,
    pi_private_key_path: Option<String>,
    pi_password: Option<String>,
    remote_directory: String,
    backup_paths: Vec<String>,
    exclude_paths: Vec<String>,
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

// Helper: Get formatted timestamp matching Node.js ISOString logic
fn get_timestamp() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string()
}

// Main Backup Logic
fn run_backup(config: &Config) -> Result<()> {
    let hostname = hostname::get()?
        .to_string_lossy()
        .replace(|c: char| c.is_whitespace(), "_");
    let timestamp = get_timestamp();
    let archive_name = format!("{}_backup_{}.tar.xz", hostname, timestamp);

    // 1. Prepare Paths
    let mut valid_paths = Vec::new();
    for p in &config.backup_paths {
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

    let remote_path = format!(
        "{}/{}",
        config.remote_directory.trim_end_matches('/'),
        archive_name
    );

    println!("\n📦 Starting Streamed Backup for host: {}", hostname);
    println!("🔥 Compression: pixz (Multi-threaded)");
    println!("📡 Destination: {}:{}", config.pi_host, remote_path);

    // 2. Initialize SSH Connection
    let port = config.pi_port.unwrap_or(22);
    let tcp = TcpStream::connect(format!("{}:{}", config.pi_host, port))
        .with_context(|| format!("Failed to connect to {}:{}", config.pi_host, port))?;

    let mut session = Session::new()?;
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .with_context(|| "Failed SSH handshake")?;

    // Auth Configuration
    if let Some(key_path) = &config.pi_private_key_path {
        let resolved_key_path = resolve_path(key_path);
        session
            .userauth_pubkey_file(&config.pi_user, None, Path::new(&resolved_key_path), None)
            .with_context(|| {
                format!(
                    "Failed to authenticate with private key: {}",
                    resolved_key_path
                )
            })?;
    } else if let Some(pass) = &config.pi_password {
        session
            .userauth_password(&config.pi_user, pass)
            .with_context(|| "Failed to authenticate with password")?;
    } else {
        anyhow::bail!("No authentication method provided in config.json");
    }

    if !session.authenticated() {
        anyhow::bail!("SSH Authentication failed");
    }

    println!("✅ SSH Connection established. Starting stream...");

    // 3. Create Remote Write Stream via SFTP
    let sftp = session
        .sftp()
        .with_context(|| "Failed to initialize SFTP session")?;
    let mut remote_stream = sftp
        .create(Path::new(&remote_path))
        .with_context(|| format!("Failed to create remote file at {}", remote_path))?;

    // 4. Spawn Local Tar Process
    let is_gnu_tar = std::env::consts::OS == "linux";
    let compress_flag = if is_gnu_tar {
        "-I"
    } else {
        "--use-compress-program"
    };
    let cpus = max(1, num_cpus::get() / 2);
    let compress_cmd = format!("pixz -p {}", cpus);

    let mut cmd = Command::new("tar");
    cmd.arg(compress_flag).arg(compress_cmd);

    for p in &config.exclude_paths {
        cmd.arg("--exclude").arg(p);
    }

    cmd.arg("-cvf").arg("-"); // Create, write to stdout

    for p in &valid_paths {
        cmd.arg(p);
    }

    // Capture stdout for piping, inherit stderr to show tar logs natively
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut tar_process = cmd.spawn().with_context(|| "Failed to spawn tar process")?;

    // 5. Pipe Data: Tar STDOUT -> SFTP Remote Stream
    if let Some(mut stdout) = tar_process.stdout.take() {
        io::copy(&mut stdout, &mut remote_stream)
            .with_context(|| "Failed while streaming data to remote file")?;
    }

    // Handle Tar Process Exit
    let status = tar_process.wait()?;
    if !status.success() {
        eprintln!(
            "❌ Tar exited with {}. Check if 'pixz' is installed.",
            status
        );
        anyhow::bail!("Tar process failed");
    } else {
        println!("✅ Local compression finished.");
    }

    println!("✅ Upload stream closed successfully.");

    Ok(())
}

fn main() {
    // Determine config path relative to execution directory
    let config_path = env::current_dir().unwrap_or_default().join("config.json");

    // Load config manually here to match the JS global exit exactly
    let config_content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("❌ Error loading config.json: {}", e);
            std::process::exit(1);
        }
    };

    let config: Config = match serde_json::from_str(&config_content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("❌ Error loading config.json: {}", e);
            std::process::exit(1);
        }
    };

    // Run the backup and catch bubbling errors
    if let Err(e) = run_backup(&config) {
        eprintln!("\n💥 Backup failed: {:#}", e);
        std::process::exit(1);
    } else {
        println!("\n🎉 Backup completed successfully!");
    }
}
