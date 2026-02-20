# JIRA Downloader

A Windows desktop app for downloading attachments from JIRA Cloud, built with Rust + egui.

## Features

- Lists your open cases automatically
- Downloads attachments organized by date into `<DownloadDir>\<ISSUE-KEY>\<YYYY-MM-DD>\`
- Detects already-downloaded files and marks them as "On disk ✓"
- Tracks issue status — flags closed/resolved cases for cleanup
- API token encrypted with AES-256-GCM; key stored in Windows Registry

## Download

Pre-built Windows x64 binary available on the [Releases](../../releases) page.

## Build from Source

### Prerequisites

- [Rust](https://rustup.rs/) (stable, 1.70+)
- Windows 10 / 11

### Steps

```powershell
# 1. Clone the repository
git clone https://github.com/merol77a/jira-downloader.git
cd jira-downloader

# 2. Build release binary
cargo build --release

# 3. Run
.\target\release\jira-downloader.exe
```

The compiled binary will be at `target\release\jira-downloader.exe`.

### Dependencies

All dependencies are managed by Cargo and downloaded automatically on first build:

| Crate | Purpose |
|---|---|
| `eframe` / `egui` | GUI framework |
| `reqwest` | HTTP client for JIRA API |
| `tokio` | Async runtime |
| `serde` / `serde_json` | Config serialization |
| `aes-gcm` | Token encryption |
| `winreg` | Encryption key storage (Windows Registry) |
| `keyring` | — |
| `chrono` | Date handling |
| `rfd` | Native folder picker dialog |

## Configuration

On first launch, go to the **Settings** tab and enter:

- **JIRA URL** — e.g. `https://yourcompany.atlassian.net`
- **Email** — your Atlassian account email
- **API Token** — generate one at [id.atlassian.com/manage-profile/security/api-tokens](https://id.atlassian.com/manage-profile/security/api-tokens)

Settings are saved to `%APPDATA%\jira-downloader\config.json`. The API token is stored encrypted; the encryption key lives in `HKCU\Software\jira-downloader` in the Windows Registry.
