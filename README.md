# stream-backup

A lightweight Rust tool that creates a compressed tar archive of one or more local paths and streams it directly to a remote host over SFTP — or writes it to a local directory — without buffering to a temporary file.

## Features

- Streams tar output directly to the destination (no local temp file required)
- Multi-threaded compression via **pixz** (`.tar.xz`) or **pigz** (`.tar.gz`)
- Remote delivery over SSH/SFTP with public-key or password authentication
- Local delivery mode (`-l`) that bypasses SSH entirely
- All non-SSH settings can be overridden at runtime via CLI flags
- Archive filenames are automatically stamped with the hostname and timestamp

## Prerequisites

- Rust toolchain (to build)
- [`pixz`](https://github.com/vasi/pixz) and/or [`pigz`](https://zlib.net/pigz/) installed and on `$PATH`
- SSH key or password access to the remote host (for remote mode)

## Building

```bash
cargo build --release
# binary will be at ./target/release/stream-backup
```

## Configuration

Copy and edit `config.json` in the working directory you run the binary from:

```json
{
  "pi_host": "hostname.local",
  "pi_port": 22,
  "pi_user": "username",
  "pi_private_key_path": "~/.ssh/id_ed25519",
  "target": "/media/user/backups/",
  "compression": "pixz",
  "include": [
    "/"
  ],
  "exclude": [
    "/dev/*",
    "/proc/*",
    "/sys/*",
    "/tmp/*",
    "/run/*",
    "/mnt/*",
    "/media/*",
    "/swapfile"
  ]
}
```

| Field | Required | Description |
|---|---|---|
| `pi_host` | Yes | SSH hostname or IP of the remote machine |
| `pi_port` | No | SSH port (default: `22`) |
| `pi_user` | Yes | SSH username |
| `pi_private_key_path` | No* | Path to private key file (`~` is expanded) |
| `pi_password` | No* | SSH password (used if no private key is set) |
| `target` | Yes | Destination directory for the archive |
| `compression` | No | `pixz` (default) or `pigz` |
| `include` | Yes | List of paths to archive |
| `exclude` | No | List of paths/patterns to exclude from the archive |

\* At least one of `pi_private_key_path` or `pi_password` is required when not using `-l`.

## Usage

```
stream-backup [OPTIONS]

Options:
  -h, --help                       Print help
  -l, --local-target               Write backup to a local path instead of uploading via SSH/SFTP
  -c, --compression=<PROGRAM>      Compression program: pixz or pigz
  -t, --target=<DIR>               Destination directory
  -i, --include=<PATH>             Path to include (repeatable)
  -e, --exclude=<PATH>             Path to exclude (repeatable)
```

CLI flags override the corresponding `config.json` values. The `pi_*` connection fields can only be set in `config.json`.

**Long flags require `=`** between the flag and its value:

```bash
# Correct
stream-backup --compression=pigz --target=/mnt/backups

# Wrong — will print help and exit
stream-backup --compression pigz
```

Short flags use space-separated values and can be combined:

```bash
# These are all equivalent
stream-backup -c pigz -t /mnt/backups
stream-backup -c pigz -t=/mnt/backups

# -l takes no value so it can be merged with -c
stream-backup -lc pigz
```

## Examples

```bash
# Normal remote backup using config.json defaults
stream-backup

# Use pigz instead of pixz
stream-backup --compression=pigz

# Write locally instead of uploading, override target directory
stream-backup -l --target=/mnt/external/backups

# Same thing with combined short flags
stream-backup -lt=/mnt/external/backups

# Back up only /home, exclude caches, write locally
stream-backup -l -i /home -e /home/*/.cache

# Override everything from the command line
stream-backup --compression=pigz --target=/backups --include=/ --exclude=/proc/* --exclude=/sys/*
```

## Output

Archives are named `{hostname}_backup_{timestamp}.tar.xz` (or `.tar.gz` with pigz), for example:

```
mymachine_backup_2025-06-01T14-30-00.tar.xz
```
