# backr

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
# binary will be at ./target/release/backr
```

CI (GitHub Actions) builds for both Linux and Windows automatically on each version tag push.

## Configuration

Run `backr` once with no `config.json` present and it will create a `config.example.json` template in the current directory. Copy it to `config.json` and fill in your values:

```bash
cp config.example.json config.json
```

```json
{
  "ssh_host": "hostname.local",
  "ssh_port": 22,
  "ssh_user": "username",
  "ssh_private_key_path": "~/.ssh/id_ed25519",
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
| `ssh_host` | Yes | SSH hostname or IP of the remote machine |
| `ssh_port` | No | SSH port (default: `22`) |
| `ssh_user` | Yes | SSH username |
| `ssh_private_key_path` | No* | Path to private key file (`~` is expanded) |
| `ssh_password` | No* | SSH password (used if no private key is set) |
| `target` | Yes | Destination directory for the archive |
| `compression` | No | `pixz` (default) or `pigz` |
| `include` | Yes | List of paths to archive |
| `exclude` | No | List of paths/patterns to exclude from the archive |

\* At least one of `ssh_private_key_path` or `ssh_password` is required when not using `-l`.

## Usage

```
backr [OPTIONS]

Options:
  -h, --help                       Print help
  -l, --local-target               Write backup to a local path instead of uploading via SSH/SFTP
  -c, --compression <PROGRAM>      Compression program: pixz or pigz
  -t, --target <DIR>               Destination directory
  -i, --include <PATH>             Path to include (repeatable)
  -e, --exclude <PATH>             Path to exclude (repeatable)
```

CLI flags override the corresponding `config.json` values. The `ssh_*` connection fields can only be set in `config.json`.

Both long and short flags accept a value with a space or `=`:

```bash
backr --compression pigz --target /mnt/backups
backr --compression=pigz --target=/mnt/backups
backr -c pigz -t /mnt/backups
```

Short flags can be combined. Since `-l` takes no value it can be prepended to any other short flag:

```bash
# -l combined with -c
backr -lc pigz

# -l combined with -t (space-separated value)
backr -lt /mnt/backups

# -l combined with -t (= value)
backr -lt=/mnt/backups

# -l combined with multiple flags
backr -lc pigz -t /mnt/backups
```

## Examples

```bash
# Normal remote backup using config.json defaults
backr

# Use pigz instead of pixz
backr --compression=pigz

# Write locally instead of uploading, override target directory
backr -l --target=/mnt/external/backups

# Same thing with combined short flags
backr -lt /mnt/external/backups

# Back up only /home, exclude caches, write locally
backr -l -i /home -e /home/*/.cache

# Back up only /home locally, combining -l with -i
backr -li /home -e /home/*/.cache

# Override everything from the command line
backr --compression=pigz --target=/backups --include=/ --exclude=/proc/* --exclude=/sys/*
```

## Output

Archives are named `{hostname}_backup_{timestamp}.tar.xz` (or `.tar.gz` with pigz), for example:

```
mymachine_backup_2025-06-01T14-30-00.tar.xz
```
