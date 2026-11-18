//! Select and verify the NSIS installer and detached signature from one release.

use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;

use super::install::InstallErrorKind;
use super::{parse_version, GitHubRelease};

const RELEASE_DOWNLOADS: &str = "https://github.com/ttsbard/TTSBard/releases/download";
const MAX_SIGNATURE_BYTES: usize = 16 * 1024;
const MAX_INSTALLER_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub(super) struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

pub(super) struct SignedInstaller<'a> {
    setup: &'a ReleaseAsset,
    signature: &'a ReleaseAsset,
}

impl GitHubRelease {
    pub(super) fn signed_installer(&self) -> Option<SignedInstaller<'_>> {
        let tag = self.tag_name.as_deref()?;
        let version = parse_version(tag)?;
        let setup = format!("TTSBard_{version}_x64-setup.exe");
        Some(SignedInstaller {
            setup: self.asset(tag, &setup)?,
            signature: self.asset(tag, &format!("{setup}.sig"))?,
        })
    }

    fn asset(&self, tag: &str, name: &str) -> Option<&ReleaseAsset> {
        let url = format!("{RELEASE_DOWNLOADS}/{tag}/{name}");
        self.assets
            .iter()
            .find(|asset| asset.name == name && asset.browser_download_url == url)
    }
}

impl SignedInstaller<'_> {
    pub(super) async fn download(
        &self,
        encoded_key: &str,
        progress: impl Fn(u64, Option<u64>),
    ) -> Result<Vec<u8>, InstallErrorKind> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(300))
            .user_agent(format!("TTSBard/{}", super::current_version()))
            .build()
            .map_err(|_| InstallErrorKind::Network)?;
        let signature = download(&client, self.signature, MAX_SIGNATURE_BYTES, |_, _| {}).await?;
        let signature =
            decode_text(std::str::from_utf8(&signature).map_err(|_| InstallErrorKind::Signature)?)?;
        let signature = Signature::decode(&signature).map_err(|_| InstallErrorKind::Signature)?;
        let key = PublicKey::decode(&decode_text(encoded_key)?)
            .map_err(|_| InstallErrorKind::Signature)?;
        let bytes = download(&client, self.setup, MAX_INSTALLER_BYTES, progress).await?;
        let expected_name = self.setup.name.clone();
        tokio::task::spawn_blocking(move || {
            key.verify(&bytes, &signature, true)
                .map_err(|_| InstallErrorKind::Signature)?;
            verify_signed_filename(signature.trusted_comment(), &expected_name)?;
            Ok(bytes)
        })
        .await
        .map_err(|_| InstallErrorKind::Signature)?
    }
}

fn decode_text(encoded: &str) -> Result<String, InstallErrorKind> {
    let decoded = STANDARD
        .decode(encoded.trim())
        .map_err(|_| InstallErrorKind::Signature)?;
    String::from_utf8(decoded).map_err(|_| InstallErrorKind::Signature)
}

fn verify_signed_filename(comment: &str, expected: &str) -> Result<(), InstallErrorKind> {
    // The Tauri signer authenticates the filename, including the version.
    // Renaming a valid older installer and signature must still fail.
    let signed_name = comment
        .split('\t')
        .find_map(|field| field.strip_prefix("file:"));
    if signed_name != Some(expected) {
        return Err(InstallErrorKind::Signature);
    }
    Ok(())
}

async fn download(
    client: &reqwest::Client,
    asset: &ReleaseAsset,
    limit: usize,
    progress: impl Fn(u64, Option<u64>),
) -> Result<Vec<u8>, InstallErrorKind> {
    let mut response = client
        .get(&asset.browser_download_url)
        .send()
        .await
        .map_err(|_| InstallErrorKind::Network)?;
    match response.status() {
        reqwest::StatusCode::NOT_FOUND => return Err(InstallErrorKind::NotFound),
        status if !status.is_success() => return Err(InstallErrorKind::Network),
        _ => {}
    }
    let total = response.content_length();
    if total.is_some_and(|size| size > limit as u64) {
        return Err(InstallErrorKind::Network);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| InstallErrorKind::Network)?
    {
        if chunk.len() > limit - bytes.len() {
            return Err(InstallErrorKind::Network);
        }
        bytes.extend_from_slice(&chunk);
        progress(bytes.len() as u64, total);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_signature_and_setup_from_the_same_release() {
        let mut release: GitHubRelease = serde_json::from_value(serde_json::json!({
            "tag_name": "v1.8.1",
            "assets": [
                {"name": "TTSBard_1.8.1_x64-setup.exe", "browser_download_url":
                    "https://github.com/ttsbard/TTSBard/releases/download/v1.8.1/TTSBard_1.8.1_x64-setup.exe"},
                {"name": "TTSBard_1.8.1_x64-setup.exe.sig", "browser_download_url":
                    "https://github.com/ttsbard/TTSBard/releases/download/v1.8.1/TTSBard_1.8.1_x64-setup.exe.sig"}
            ]
        }))
        .unwrap();
        assert!(release.signed_installer().is_some());
        release.assets[1].browser_download_url = release.assets[1]
            .browser_download_url
            .replace("v1.8.1", "v1.8.0");
        assert!(release.signed_installer().is_none());
        release.assets.pop();
        assert!(release.signed_installer().is_none());
    }

    #[test]
    fn signed_filename_must_match_the_announced_asset() {
        let comment = "timestamp:1794946505\tfile:TTSBard_1.8.1_x64-setup.exe";
        assert!(verify_signed_filename(comment, "TTSBard_1.8.1_x64-setup.exe").is_ok());
        assert!(verify_signed_filename(comment, "TTSBard_1.8.2_x64-setup.exe").is_err());
        assert!(
            verify_signed_filename("timestamp:1794946505", "TTSBard_1.8.1_x64-setup.exe").is_err()
        );
    }
}
