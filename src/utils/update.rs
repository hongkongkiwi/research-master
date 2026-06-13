//! Update utilities for self-updating the CLI.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Check if an external binary is available in PATH
fn is_command_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Information about the current installation method
#[derive(Debug, Clone)]
pub enum InstallationMethod {
    /// Installed via Homebrew
    Homebrew {
        /// Path to the Homebrew binary
        path: PathBuf,
    },
    /// Installed via cargo install
    Cargo {
        /// Installation directory
        path: PathBuf,
    },
    /// Directly installed binary
    Direct {
        /// Path to the binary
        path: PathBuf,
    },
    /// Unknown installation method
    Unknown,
}

/// Detect how the tool was installed
pub fn detect_installation() -> InstallationMethod {
    #[cfg(test)]
    {
        // Unit tests run from Cargo's test harness rather than an installed
        // research-master binary, so there is no meaningful installation method.
        InstallationMethod::Unknown
    }

    #[cfg(not(test))]
    {
        let exe_path = match std::env::current_exe() {
            Ok(path) => path,
            Err(_) => return InstallationMethod::Unknown,
        };

        // Check if running from Homebrew prefix
        if let Ok(homebrew_prefix) = std::env::var("HOMEBREW_PREFIX") {
            let homebrew_bin = PathBuf::from(homebrew_prefix)
                .join("bin")
                .join("research-master");
            if exe_path == homebrew_bin
                || exe_path.starts_with(homebrew_bin.parent().unwrap_or(&homebrew_bin))
            {
                return InstallationMethod::Homebrew { path: exe_path };
            }
        }

        // Check if installed via cargo (check if ~/.cargo/bin is in the path)
        if let Ok(cargo_home) = std::env::var("CARGO_HOME") {
            let cargo_bin = PathBuf::from(cargo_home)
                .join("bin")
                .join("research-master");
            if exe_path == cargo_bin {
                return InstallationMethod::Cargo { path: exe_path };
            }
        }

        // Check common Homebrew locations
        let homebrew_paths = [
            PathBuf::from("/opt/homebrew/bin/research-master"),
            PathBuf::from("/usr/local/bin/research-master"),
            PathBuf::from("/home/linuxbrew/.linuxbrew/bin/research-master"),
        ];

        for hb_path in &homebrew_paths {
            if exe_path == *hb_path {
                return InstallationMethod::Homebrew { path: exe_path };
            }
        }

        InstallationMethod::Direct { path: exe_path }
    }
}

/// Get installation-specific update instructions
pub fn get_update_instructions(method: &InstallationMethod) -> String {
    match method {
        InstallationMethod::Homebrew { .. } => {
            "You seem to have installed via Homebrew. Run:\n  brew upgrade research-master".to_string()
        }
        InstallationMethod::Cargo { .. } => {
            "You seem to have installed via cargo. Run:\n  cargo install research-master".to_string()
        }
        InstallationMethod::Direct { .. } => {
            "I'll download and install the latest version for you.".to_string()
        }
        InstallationMethod::Unknown => {
            "Unable to detect installation method.\n\nIf you installed via:\n  - Homebrew: run 'brew upgrade research-master'\n  - cargo: run 'cargo install research-master'\n  - Direct download: I'll download the latest binary".to_string()
        }
    }
}

/// GitHub release information
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    /// Tag name (e.g., "v0.1.5")
    pub tag_name: String,
    /// Version number without 'v' prefix
    pub version: String,
    /// Release notes/body
    pub body: String,
    /// Published date
    pub published_at: String,
    /// Array of assets with download URLs
    pub assets: Vec<ReleaseAsset>,
}

/// A single release asset
#[derive(Debug, Clone)]
pub struct ReleaseAsset {
    /// Asset name (e.g., "research-master-x86_64-apple-darwin.tar.gz")
    pub name: String,
    /// Download URL
    pub download_url: String,
}

/// Fetch the latest release information from GitHub
pub async fn fetch_latest_release() -> Result<ReleaseInfo> {
    let client = reqwest::Client::new();
    let response = client
        .get("https://api.github.com/repos/hongkongkiwi/research-master/releases/latest")
        .header("User-Agent", "research-master")
        .send()
        .await
        .context("Failed to fetch latest release")?;

    if !response.status().is_success() {
        bail!(
            "GitHub API request failed with status: {}",
            response.status()
        );
    }

    let json: serde_json::Value = response
        .json()
        .await
        .context("Failed to parse release info")?;

    let tag_name = json["tag_name"]
        .as_str()
        .context("Missing tag_name")?
        .to_string();

    let version = tag_name.trim_start_matches('v').to_string();

    let body = json["body"].as_str().unwrap_or("").to_string();
    let published_at = json["published_at"].as_str().unwrap_or("").to_string();

    let mut assets = Vec::new();
    if let Some(assets_array) = json["assets"].as_array() {
        for asset in assets_array {
            if let (Some(name), Some(download_url)) = (
                asset["name"].as_str(),
                asset["browser_download_url"].as_str(),
            ) {
                assets.push(ReleaseAsset {
                    name: name.to_string(),
                    download_url: download_url.to_string(),
                });
            }
        }
    }

    Ok(ReleaseInfo {
        tag_name,
        version,
        body,
        published_at,
        assets,
    })
}

/// Get the target triple for the current platform
pub fn get_current_target() -> &'static str {
    // Get target from std
    let target = std::env::consts::ARCH;

    // Determine OS
    let os = if cfg!(target_os = "linux") {
        if cfg!(target_env = "musl") {
            "unknown-linux-musl"
        } else {
            "unknown-linux-gnu"
        }
    } else if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(target_os = "windows") {
        "pc-windows-msvc"
    } else {
        return "";
    };

    match target {
        "x86_64" => {
            if os == "apple-darwin" {
                "x86_64-apple-darwin"
            } else if os == "unknown-linux-musl" {
                "x86_64-unknown-linux-musl"
            } else if os == "unknown-linux-gnu" {
                "x86_64-unknown-linux-gnu"
            } else if os == "pc-windows-msvc" {
                "x86_64-pc-windows-msvc"
            } else {
                ""
            }
        }
        "aarch64" => {
            if os == "apple-darwin" {
                "aarch64-apple-darwin"
            } else {
                ""
            }
        }
        _ => "",
    }
}

/// Find the appropriate release asset for the current platform
pub fn find_asset_for_platform(release: &ReleaseInfo) -> Option<&ReleaseAsset> {
    let target = get_current_target();
    if target.is_empty() {
        return None;
    }

    // Look for tar.gz first (Linux/macOS), then zip (Windows)
    let preferred_ext = if cfg!(target_os = "windows") {
        ".zip"
    } else {
        ".tar.gz"
    };

    // Try to find exact match
    if let Some(asset) = release
        .assets
        .iter()
        .find(|asset| asset.name.contains(target) && asset.name.ends_with(preferred_ext))
    {
        return Some(asset);
    }

    // Fallback: just find any asset with the target
    release
        .assets
        .iter()
        .find(|asset| asset.name.contains(target))
}

/// Download and extract a release asset
pub async fn download_and_extract_asset(asset: &ReleaseAsset, temp_dir: &Path) -> Result<PathBuf> {
    let client = reqwest::Client::new();

    // Download the archive
    eprintln!("Downloading {}...", asset.name);
    let response = client
        .get(&asset.download_url)
        .send()
        .await
        .context("Failed to download asset")?;

    if !response.status().is_success() {
        bail!("Download failed with status: {}", response.status());
    }

    let bytes = response
        .bytes()
        .await
        .context("Failed to read response body")?;

    // Save to temp file
    let archive_path = temp_dir.join(&asset.name);
    fs::write(&archive_path, &bytes).context("Failed to save archive")?;

    // Extract based on file type
    let binary_path = if asset.name.ends_with(".tar.gz") {
        extract_tar_gz(&archive_path, temp_dir)?
    } else if asset.name.ends_with(".zip") {
        extract_zip(&archive_path, temp_dir)?
    } else {
        bail!("Unsupported archive format: {}", asset.name);
    };

    Ok(binary_path)
}

#[cfg(unix)]
fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    // Use tar to extract
    let output = Command::new("tar")
        .args([
            "xzf",
            archive_path.to_string_lossy().as_ref(),
            "-C",
            dest_dir.to_string_lossy().as_ref(),
        ])
        .output()
        .context("Failed to extract tar.gz")?;

    if !output.status.success() {
        bail!(
            "tar extraction failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Find the extracted binary
    for entry in fs::read_dir(dest_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .map(|n| n.to_string_lossy().starts_with("research-master"))
                .unwrap_or(false)
        {
            // Make executable
            let mut perms = fs::metadata(&path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms)?;
            return Ok(path);
        }
    }

    bail!("Could not find binary in archive")
}

#[cfg(windows)]
fn extract_tar_gz(_archive_path: &Path, _dest_dir: &Path) -> Result<PathBuf> {
    bail!("tar.gz extraction on Windows requires additional dependencies")
}

#[cfg(windows)]
fn extract_zip(archive_path: &Path, dest_dir: &Path) -> Result<PathBuf> {
    use zip::ZipArchive;

    let file = fs::File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let out_path = dest_dir.join(entry.name());

        if entry.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out_file = fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;
        }
    }

    // Find the binary
    for entry in fs::read_dir(dest_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .map(|n| n.to_string_lossy().starts_with("research-master"))
                .unwrap_or(false)
        {
            return Ok(path);
        }
    }

    bail!("Could not find binary in archive")
}

#[cfg(unix)]
fn extract_zip(_archive_path: &Path, _dest_dir: &Path) -> Result<PathBuf> {
    bail!("zip extraction on Unix requires additional dependencies")
}

/// Replace the current binary with a new one
pub fn replace_binary(current: &Path, new: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        // On Unix, we need to copy to a temp location first, then rename
        // because the current binary is still running
        let temp_path = current.with_file_name(format!(
            "{}.new",
            current
                .file_name()
                .unwrap_or(std::ffi::OsStr::new("binary"))
                .to_string_lossy()
        ));

        // Copy new binary to temp location
        fs::copy(new, &temp_path)?;
        std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755))?;

        // Rename temp to current (atomic on POSIX)
        // First, rename current to backup
        let backup_path = current.with_file_name(format!(
            "{}.backup",
            current
                .file_name()
                .unwrap_or(std::ffi::OsStr::new("binary"))
                .to_string_lossy()
        ));
        if current.exists() {
            fs::rename(current, &backup_path)?;
        }

        // Rename temp to current
        fs::rename(&temp_path, current)?;

        // Remove backup
        if backup_path.exists() {
            fs::remove_file(&backup_path)?;
        }

        Ok(())
    }

    #[cfg(windows)]
    {
        // On Windows, we can't replace a running executable
        // So we copy to a .new file and tell the user to restart
        let new_path = current.with_extension(".exe.new");
        fs::copy(new, &new_path)?;
        eprintln!(
            "New binary downloaded to: {}. Please restart your terminal to use the new version.",
            new_path.display()
        );
        Ok(())
    }
}

/// Clean up temporary files
pub fn cleanup_temp_files(files: Vec<PathBuf>) {
    for file in files {
        if file.exists() {
            let _ = fs::remove_file(file);
        }
    }
}

/// Fetch and verify SHA256 checksum for a file
pub async fn fetch_and_verify_sha256(asset_name: &str, _temp_dir: &Path) -> Result<String> {
    let client = reqwest::Client::new();
    let checksums_url =
        "https://github.com/hongkongkiwi/research-master/releases/download/latest/SHA256SUMS.txt";

    eprintln!("Downloading SHA256 checksums...");
    let response = client
        .get(checksums_url)
        .header("User-Agent", "research-master")
        .send()
        .await
        .context("Failed to download checksums file")?;

    if !response.status().is_success() {
        bail!("Failed to download checksums (HTTP {})", response.status());
    }

    let checksums_text = response.text().await.context("Failed to read checksums")?;

    // Parse the checksums file and find the matching entry
    for line in checksums_text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let hash = parts[0];
            let filename = parts.last().unwrap_or(&"");

            // Handle both formats:
            // e.g., "abc123...  research-master-x86_64-unknown-linux-musl.tar.gz"
            // or "abc123...  ./research-master-x86_64-unknown-linux-musl.tar.gz"
            let normalized_filename = filename.trim_start_matches("./");

            if normalized_filename == asset_name || filename.contains(asset_name) {
                return Ok(hash.to_string());
            }
        }
    }

    bail!("Checksum not found for {}", asset_name)
}

/// Compute SHA256 hash of a file
pub fn compute_sha256(file_path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};

    let data = fs::read(file_path).context("Failed to read file for checksum")?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

/// Verify downloaded file against expected SHA256 hash
pub fn verify_sha256(file_path: &Path, expected_hash: &str) -> Result<bool> {
    let actual_hash = compute_sha256(file_path)?;

    if actual_hash == expected_hash {
        Ok(true)
    } else {
        eprintln!("SHA256 mismatch!");
        eprintln!("Expected: {}", expected_hash);
        eprintln!("Actual:   {}", actual_hash);
        Ok(false)
    }
}

/// Fetch the GPG signature for SHA256SUMS.txt
pub async fn fetch_sha256_signature() -> Result<String> {
    let client = reqwest::Client::new();
    let signature_url = "https://github.com/hongkongkiwi/research-master/releases/download/latest/SHA256SUMS.txt.asc";

    eprintln!("Downloading GPG signature...");
    let response = client
        .get(signature_url)
        .header("User-Agent", "research-master")
        .send()
        .await
        .context("Failed to download GPG signature")?;

    if !response.status().is_success() {
        bail!(
            "Failed to download GPG signature (HTTP {})",
            response.status()
        );
    }

    let signature = response.text().await.context("Failed to read signature")?;
    Ok(signature)
}

/// Verify GPG signature of SHA256SUMS.txt
/// This requires the project maintainer's public key to be in the system keyring.
/// For CI/CD, set GPG_FINGERPRINT to the expected signer's fingerprint.
pub fn verify_gpg_signature(sha256sums_path: &Path, signature: &str) -> Result<bool> {
    use std::io::Write as _;

    // Check if GPG is available first
    if !is_command_available("gpg") {
        #[cfg(windows)]
        {
            eprintln!("WARNING: GPG is not installed or not in PATH.");
            eprintln!("On Windows, install GPG from https://www.gpg4win.org/");
        }
        #[cfg(not(windows))]
        {
            eprintln!("WARNING: GPG is not installed or not in PATH.");
            eprintln!("Install GPG with your package manager (e.g., brew install gnupg)");
        }
        eprintln!("Skipping GPG signature verification.");
        return Ok(false);
    }

    // Write signature to a temp file
    let sig_path = sha256sums_path.with_extension("txt.asc");
    let mut sig_file = std::fs::File::create(&sig_path)?;
    sig_file.write_all(signature.as_bytes())?;
    sig_file.flush()?;

    // Verify using gpg
    let output = Command::new("gpg")
        .args([
            "--verify",
            sig_path.to_string_lossy().as_ref(),
            sha256sums_path.to_string_lossy().as_ref(),
        ])
        .output()
        .context("Failed to run gpg")?;

    // Clean up signature file
    let _ = std::fs::remove_file(&sig_path);

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Check for "Good signature" in output
    if stderr.contains("Good signature") || stderr.contains("gpg: Good signature") {
        // Optionally verify the signer if fingerprint is set
        if let Ok(fingerprint) = std::env::var("GPG_FINGERPRINT") {
            if stderr.contains(&fingerprint) || output.status.success() {
                eprintln!("GPG signature verified successfully!");
                return Ok(true);
            } else {
                eprintln!("WARNING: Signature is good but from unexpected signer!");
                eprintln!("Expected fingerprint: {}", fingerprint);
                return Ok(false);
            }
        }
        eprintln!("GPG signature verified successfully!");
        return Ok(true);
    }

    if stderr.contains("BAD signature") || stderr.contains("gpg: BAD signature") {
        eprintln!("ERROR: GPG signature verification FAILED!");
        eprintln!("{}", stderr);
        return Ok(false);
    }

    // If gpg is not available or key not found
    if stderr.contains("no public key") || stderr.contains("gpg: Can't check signature") {
        eprintln!("WARNING: GPG is not configured properly.");
        eprintln!("To enable GPG verification, either:");
        eprintln!("  1. Install GPG and import the maintainer's public key");
        eprintln!("  2. Set GPG_FINGERPRINT to skip signer verification");
        return Ok(false);
    }

    eprintln!("GPG verification result: {}", stderr);
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    const HELLO_WORLD_SHA256: &str =
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    fn release_with_assets(assets: Vec<ReleaseAsset>) -> ReleaseInfo {
        ReleaseInfo {
            tag_name: "v1.2.3".to_string(),
            version: "1.2.3".to_string(),
            body: "Release notes".to_string(),
            published_at: "2026-01-01T00:00:00Z".to_string(),
            assets,
        }
    }

    fn asset(name: impl Into<String>) -> ReleaseAsset {
        let name = name.into();
        ReleaseAsset {
            download_url: format!("https://example.com/{name}"),
            name,
        }
    }

    #[test]
    fn get_current_target_returns_non_empty() {
        assert!(!get_current_target().is_empty());
    }

    #[test]
    fn find_asset_for_platform_prefers_exact_archive_match() {
        let target = get_current_target();
        let preferred_ext = if cfg!(target_os = "windows") {
            ".zip"
        } else {
            ".tar.gz"
        };
        let fallback_name = format!("research-master-{target}.bin");
        let exact_name = format!("research-master-{target}{preferred_ext}");
        let release = release_with_assets(vec![asset(&fallback_name), asset(&exact_name)]);

        let found = find_asset_for_platform(&release).expect("expected matching asset");

        assert_eq!(found.name, exact_name);
    }

    #[test]
    fn find_asset_for_platform_falls_back_to_any_target_match() {
        let target = get_current_target();
        let non_preferred_ext = if cfg!(target_os = "windows") {
            ".tar.gz"
        } else {
            ".zip"
        };
        let fallback_name = format!("research-master-{target}{non_preferred_ext}");
        let release = release_with_assets(vec![asset(&fallback_name)]);

        let found = find_asset_for_platform(&release).expect("expected fallback asset");

        assert_eq!(found.name, fallback_name);
    }

    #[test]
    fn find_asset_for_platform_returns_none_without_target_match() {
        let release = release_with_assets(vec![
            asset("research-master-x86_64-unsupported-os.tar.gz"),
            asset("SHA256SUMS.txt"),
        ]);

        assert!(find_asset_for_platform(&release).is_none());
    }

    #[test]
    fn compute_sha256_returns_known_hash_for_file() {
        let temp_dir = tempdir().expect("tempdir");
        let file_path = temp_dir.path().join("hello.txt");
        fs::write(&file_path, b"hello world").expect("write test file");

        let hash = compute_sha256(&file_path).expect("compute sha256");

        assert_eq!(hash, HELLO_WORLD_SHA256);
    }

    #[test]
    fn compute_sha256_errors_for_nonexistent_file() {
        let temp_dir = tempdir().expect("tempdir");
        let missing_path = temp_dir.path().join("missing.txt");

        assert!(compute_sha256(&missing_path).is_err());
    }

    #[test]
    fn verify_sha256_returns_true_for_matching_hash() {
        let temp_dir = tempdir().expect("tempdir");
        let file_path = temp_dir.path().join("hello.txt");
        fs::write(&file_path, b"hello world").expect("write test file");

        let verified = verify_sha256(&file_path, HELLO_WORLD_SHA256).expect("verify sha256");

        assert!(verified);
    }

    #[test]
    fn verify_sha256_returns_false_for_mismatched_hash() {
        let temp_dir = tempdir().expect("tempdir");
        let file_path = temp_dir.path().join("hello.txt");
        fs::write(&file_path, b"hello world").expect("write test file");
        let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";

        let verified = verify_sha256(&file_path, wrong_hash).expect("verify sha256");

        assert!(!verified);
    }

    #[test]
    fn get_update_instructions_handles_all_installation_methods() {
        let path = PathBuf::from("/tmp/research-master");

        let homebrew =
            get_update_instructions(&InstallationMethod::Homebrew { path: path.clone() });
        assert!(homebrew.contains("Homebrew"));
        assert!(homebrew.contains("brew upgrade research-master"));

        let cargo = get_update_instructions(&InstallationMethod::Cargo { path: path.clone() });
        assert!(cargo.contains("cargo"));
        assert!(cargo.contains("cargo install research-master"));

        let direct = get_update_instructions(&InstallationMethod::Direct { path });
        assert!(direct.contains("download and install"));

        let unknown = get_update_instructions(&InstallationMethod::Unknown);
        assert!(unknown.contains("Unable to detect installation method"));
        assert!(unknown.contains("brew upgrade research-master"));
        assert!(unknown.contains("cargo install research-master"));
        assert!(unknown.contains("Direct download"));
    }

    #[test]
    fn detect_installation_returns_unknown_in_unit_tests() {
        assert!(matches!(detect_installation(), InstallationMethod::Unknown));
    }

    #[test]
    fn release_asset_can_be_constructed() {
        let asset = ReleaseAsset {
            name: "research-master-x86_64-apple-darwin.tar.gz".to_string(),
            download_url: "https://example.com/download".to_string(),
        };

        assert_eq!(asset.name, "research-master-x86_64-apple-darwin.tar.gz");
        assert_eq!(asset.download_url, "https://example.com/download");
    }

    #[test]
    fn release_info_can_be_constructed_with_assets() {
        let release = release_with_assets(vec![
            asset("research-master-x86_64-apple-darwin.tar.gz"),
            asset("SHA256SUMS.txt"),
        ]);

        assert_eq!(release.tag_name, "v1.2.3");
        assert_eq!(release.version, "1.2.3");
        assert_eq!(release.body, "Release notes");
        assert_eq!(release.published_at, "2026-01-01T00:00:00Z");
        assert_eq!(release.assets.len(), 2);
        assert_eq!(
            release.assets[0].name,
            "research-master-x86_64-apple-darwin.tar.gz"
        );
        assert_eq!(
            release.assets[1].download_url,
            "https://example.com/SHA256SUMS.txt"
        );
    }

    #[test]
    fn cleanup_temp_files_removes_existing_files_and_ignores_missing_files() {
        let temp_dir = tempdir().expect("tempdir");
        let file_one = temp_dir.path().join("one.tmp");
        let file_two = temp_dir.path().join("two.tmp");
        let missing_file = temp_dir.path().join("missing.tmp");
        fs::write(&file_one, b"one").expect("write file one");
        fs::write(&file_two, b"two").expect("write file two");

        cleanup_temp_files(vec![file_one.clone(), missing_file, file_two.clone()]);

        assert!(!file_one.exists());
        assert!(!file_two.exists());
        assert!(temp_dir.path().exists());
    }

    #[test]
    fn is_command_available_detects_known_and_missing_commands() {
        assert!(is_command_available("cargo"));
        assert!(is_command_available("rustc"));
        assert!(!is_command_available(
            "research-master-definitely-not-a-real-command"
        ));
    }
}
