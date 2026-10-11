//! Windows update check for the user's GitHub Releases channel.

const RELEASE_API: &str = "https://api.github.com/repos/Bilalarham/gridcraft/releases/latest";
const RELEASE_DOWNLOAD_PREFIX: &str = "https://github.com/Bilalarham/gridcraft/releases/download/";
const MAX_INSTALLER_BYTES: usize = 512 * 1024 * 1024;

fn background_command(program: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt as _;
    let mut command = std::process::Command::new(program);
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
}

pub fn check_for_updates() {
    start_check(false);
}

pub fn check_for_updates_on_startup() {
    start_check(true);
}

fn start_check(silent_when_current_or_unavailable: bool) {
    let result = std::thread::Builder::new().name("gridcraft-update-check".into()).spawn(move || {
        if let Err(error) = check_latest_release(silent_when_current_or_unavailable) {
            if silent_when_current_or_unavailable {
                log::debug!("background update check: {error}");
            } else {
                show_message("GridCraft update", &error);
            }
        }
    });
    if let Err(error) = result {
        show_message("GridCraft update", &format!("Could not start the update check: {error}"));
    }
}

fn check_latest_release(silent_when_current: bool) -> Result<(), String> {
    let api_script = format!(
        "$ErrorActionPreference='Stop'; try {{ $r=Invoke-RestMethod -Uri '{RELEASE_API}' -Headers @{{'User-Agent'='GridCraft'}} -TimeoutSec 30; [pscustomobject]@{{tag_name=$r.tag_name; assets=@($r.assets | ForEach-Object {{ [pscustomobject]@{{name=$_.name; browser_download_url=$_.browser_download_url; size=$_.size; digest=$_.digest}} }})}} | ConvertTo-Json -Depth 5 -Compress }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}"
    );
    let response = background_command("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &api_script])
        .output()
        .map_err(|error| format!("Could not start the built-in Windows update check: {error}"))?;
    if !response.status.success() {
        let detail = String::from_utf8_lossy(&response.stderr);
        return Err(format!("Could not check GitHub for updates. Check your internet connection and try again.\n\n{}", detail.trim()));
    }
    let release: serde_json::Value = serde_json::from_slice(&response.stdout)
        .map_err(|error| format!("GitHub returned an unreadable release record: {error}"))?;
    let tag = release.get("tag_name").and_then(serde_json::Value::as_str).ok_or("The GitHub release has no version tag.")?;
    let latest_text = tag.strip_prefix('v').unwrap_or(tag);
    let latest = semver::Version::parse(latest_text).map_err(|error| format!("GitHub release has an invalid version tag '{tag}': {error}"))?;
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|error| format!("This app has an invalid version: {error}"))?;
    if latest <= current {
        if !silent_when_current {
            show_message("GridCraft update", &format!("You already have the latest version ({current})."));
        }
        return Ok(());
    }

    let arch = if cfg!(target_arch = "x86_64") { "x64" } else if cfg!(target_arch = "aarch64") { "arm64" } else { "x86" };
    let asset_name = format!("gridcraft-{latest}-windows-{arch}.msi");
    let assets = release.get("assets").and_then(serde_json::Value::as_array).ok_or("The GitHub release has no installer files.")?;
    let asset = assets.iter().find(|asset| asset.get("name").and_then(serde_json::Value::as_str) == Some(&asset_name)).ok_or_else(|| {
        format!("GridCraft {latest} is available, but its Windows {arch} installer is missing from the GitHub release. Please try again later.")
    })?;
    let download_url = asset.get("browser_download_url").and_then(serde_json::Value::as_str).ok_or("The installer link in the GitHub release is missing.")?;
    let asset_size = asset.get("size").and_then(serde_json::Value::as_u64).ok_or("The installer size in the GitHub release is invalid.")?;
    let digest = asset.get("digest").and_then(serde_json::Value::as_str);
    if !download_url.starts_with(RELEASE_DOWNLOAD_PREFIX) || asset_size == 0 || asset_size > MAX_INSTALLER_BYTES as u64 {
        return Err("The installer link or file size in the GitHub release is not valid.".into());
    }

    let answer = rfd::MessageDialog::new()
        .set_title("GridCraft update available")
        .set_description(format!(
            "GridCraft {latest} is available. You have {current}.\n\nDownload the installer and start the update? Save your workbook first. Windows may ask for permission, and the installer may ask you to close GridCraft."
        ))
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    if answer != rfd::MessageDialogResult::Yes {
        return Ok(());
    }

    let installer = download_installer(download_url, asset_size, digest)?;
    let process = std::process::Command::new("msiexec.exe").arg("/i").arg(&installer).spawn();
    match process {
        Ok(mut child) => match child.wait() {
            Ok(status) if status.code() == Some(0) => {
                let _ = std::fs::remove_file(&installer);
                show_message(
                    "GridCraft update complete",
                    &format!("GridCraft {latest} was installed. Close and reopen GridCraft to use the updated version."),
                );
                Ok(())
            }
            Ok(status) if status.code() == Some(3010) => {
                let _ = std::fs::remove_file(&installer);
                show_message(
                    "GridCraft update complete",
                    &format!("GridCraft {latest} was installed. Windows needs to restart before the update is fully applied."),
                );
                Ok(())
            }
            Ok(status) => {
                let code = status.code().map_or_else(|| "unknown".to_string(), |code| code.to_string());
                Err(format!(
                    "Windows Installer could not complete the GridCraft {latest} update (exit code {code}). The downloaded installer is still available here:\n\n{}\n\nSave your work, close GridCraft, and open that installer to try again.",
                    installer.display()
                ))
            }
            Err(error) => Err(format!(
                "Windows Installer started but GridCraft could not read its result: {error}. The downloaded installer is still available here:\n\n{}",
                installer.display()
            )),
        },
        Err(error) => {
            Err(format!(
                "The installer downloaded, but Windows could not start it: {error}. The file is still available here:\n\n{}",
                installer.display()
            ))
        }
    }
}

fn download_installer(download_url: &str, expected_size: u64, expected_digest: Option<&str>) -> Result<std::path::PathBuf, String> {
    if !download_url.starts_with(RELEASE_DOWNLOAD_PREFIX) {
        return Err("The installer URL is not from the configured GitHub Releases page.".into());
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("System clock error: {error}"))?
        .as_millis();
    let path = std::env::temp_dir().join(format!("gridcraft-update-{}-{stamp}.msi", std::process::id()));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("Could not prepare a temporary file for the installer: {error}"))?;
    drop(file);

    let response = background_command("curl.exe")
        .args(["--fail", "--location", "--silent", "--show-error", "--max-time", "600", "--max-filesize", "536870912", "--output"])
        .arg(&path)
        .arg(download_url)
        .output();
    match response {
        Ok(response) if response.status.success() => {}
        Ok(response) => {
            let _ = std::fs::remove_file(&path);
            return Err(format!("Could not download the GridCraft installer.\n\n{}", String::from_utf8_lossy(&response.stderr).trim()));
        }
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            return Err(format!("Could not start Windows' built-in download tool: {error}"));
        }
    }
    let actual_size = match std::fs::metadata(&path) {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            return Err(format!("Could not verify the downloaded installer: {error}"));
        }
    };
    if actual_size == 0 || actual_size > MAX_INSTALLER_BYTES as u64 || actual_size != expected_size {
        let _ = std::fs::remove_file(&path);
        return Err("The installer download was incomplete or larger than expected. Please try again.".into());
    }
    if let Some(digest) = expected_digest {
        let Some(expected) = digest.strip_prefix("sha256:") else {
            let _ = std::fs::remove_file(&path);
            return Err("GitHub provided an unsupported installer checksum.".into());
        };
        let hash = match background_command("certutil.exe").args(["-hashfile"]).arg(&path).arg("SHA256").output() {
            Ok(hash) => hash,
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                return Err(format!("Windows could not verify the installer checksum: {error}"));
            }
        };
        if !hash.status.success() {
            let _ = std::fs::remove_file(&path);
            return Err("Windows could not verify the installer checksum. The installer was not opened.".into());
        }
        let hash_output = String::from_utf8_lossy(&hash.stdout);
        let Some(actual) = hash_output.lines().map(str::trim).find(|line| line.len() == 64 && line.bytes().all(|b| b.is_ascii_hexdigit())) else {
            let _ = std::fs::remove_file(&path);
            return Err("Windows returned an unreadable installer checksum.".into());
        };
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(&path);
            return Err("The installer checksum did not match GitHub's record. The file was not opened.".into());
        }
    }
    Ok(path)
}

fn show_message(title: &str, message: &str) {
    let _ = rfd::MessageDialog::new().set_title(title).set_description(message).show();
}
