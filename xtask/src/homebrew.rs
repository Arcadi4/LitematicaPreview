use std::{
    ffi::OsString,
    fs::OpenOptions,
    io::Write,
    process::{Command, Output, Stdio},
};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use semver::Version;
use serde_json::Value;

const SOURCE_REPO: &str = "Arcadi4/LitematicaPreview";
const TAP_REPO: &str = "Arcadi4/homebrew-tap";
const CASK_NAME: &str = "litematica-preview";
const CASK_PATH: &str = "Casks/litematica-preview.rb";
const CASK_DIR: &str = "Casks";

const HELP: &str = "usage: cargo xtask publish-homebrew vX.Y.Z [--repair]";

#[derive(Clone, Copy)]
enum Action {
    Create,
    Update,
    Unchanged,
    Skip,
}

impl Action {
    fn result(self) -> &'static str {
        match self {
            Self::Create => "created",
            Self::Update => "updated",
            Self::Unchanged => "unchanged",
            Self::Skip => "skipped",
        }
    }
}

struct Remote {
    exists: bool,
    sha: Option<String>,
    content: String,
}

pub fn run(args: &[OsString]) -> Result<()> {
    let mut tag = None;
    let mut repair = false;

    for arg in args {
        let arg = arg.to_str().context("arguments must be valid UTF-8")?;
        match arg {
            "--repair" => repair = true,
            "-h" | "--help" => {
                println!("{HELP}");
                return Ok(());
            }
            other if other.starts_with('-') => bail!("unknown option {other}\n{HELP}"),
            other => {
                if tag.replace(other.to_owned()).is_some() {
                    bail!("expected exactly one tag\n{HELP}");
                }
            }
        }
    }

    let tag = tag.context(HELP)?;
    let version = stable_version(&tag)?;
    publish(&tag, &version, repair)
}

fn stable_version(tag: &str) -> Result<Version> {
    let raw = tag
        .strip_prefix('v')
        .with_context(|| format!("tag {tag} must start with v"))?;
    let version =
        Version::parse(raw).with_context(|| format!("tag {tag} is not a semantic version"))?;
    if version.to_string() != raw {
        bail!("tag {tag} must be a stable MAJOR.MINOR.PATCH release");
    }
    Ok(version)
}

fn publish(tag: &str, version: &Version, repair: bool) -> Result<()> {
    let source_token = source_token()?;
    let tap_token = tap_token()?;
    let (arm_sha, intel_sha) = release_digests(&source_token, tag, version)?;
    let branch = tap_branch(&tap_token)?;
    let new_cask = render_cask(version, &arm_sha, &intel_sha);

    let mut remote = read_remote_cask(&tap_token)?;
    let action = plan(&remote, &new_cask, version, repair)?;

    let result = match action {
        Action::Unchanged | Action::Skip => action.result(),
        Action::Create | Action::Update => {
            match put_cask(&tap_token, &branch, &remote, action, version, &new_cask) {
                Ok(()) => action.result(),
                Err(first_error) => {
                    eprintln!("tap write failed ({first_error:#}); re-reading once");
                    remote = read_remote_cask(&tap_token)?;
                    match plan(&remote, &new_cask, version, repair)? {
                        retry @ (Action::Unchanged | Action::Skip) => retry.result(),
                        retry => {
                            put_cask(&tap_token, &branch, &remote, retry, version, &new_cask)?;
                            retry.result()
                        }
                    }
                }
            }
        }
    };

    emit(result, version)
}

fn release_digests(token: &str, tag: &str, version: &Version) -> Result<(String, String)> {
    let endpoint = format!("repos/{SOURCE_REPO}/releases/tags/{tag}");
    let release = gh_json(token, &["api", &endpoint])?;

    if release["tag_name"].as_str() != Some(tag) {
        bail!("GitHub returned a different tag than {tag}");
    }
    if release["draft"].as_bool().unwrap_or(false) {
        bail!("release {tag} is still a draft");
    }
    if release["prerelease"].as_bool().unwrap_or(false) {
        bail!("release {tag} is a prerelease; the Homebrew Cask tracks stable releases only");
    }

    let assets = release["assets"]
        .as_array()
        .context("release has no assets array")?;
    let arm = asset_digest(assets, tag, version, "aarch64")?;
    let intel = asset_digest(assets, tag, version, "x86_64")?;
    Ok((arm, intel))
}

fn asset_digest(assets: &[Value], tag: &str, version: &Version, arch: &str) -> Result<String> {
    let name = format!("LitematicaPreview-{version}-{arch}.dmg");
    let asset = assets
        .iter()
        .find(|asset| asset["name"].as_str() == Some(name.as_str()))
        .with_context(|| format!("release {tag} has no {name}"))?;

    if asset["state"].as_str() != Some("uploaded") {
        bail!("{name} is not fully uploaded");
    }
    if asset["size"].as_u64().unwrap_or(0) == 0 {
        bail!("{name} is empty");
    }

    let canonical = format!("https://github.com/{SOURCE_REPO}/releases/download/{tag}/{name}");
    if asset["browser_download_url"].as_str() != Some(canonical.as_str()) {
        bail!("{name} does not point at the canonical release URL");
    }

    let digest = asset["digest"].as_str().with_context(|| {
        format!("{name} has no GitHub digest; refusing to publish an unverified asset")
    })?;
    let hex = digest
        .strip_prefix("sha256:")
        .with_context(|| format!("{name} has an unsupported digest {digest}"))?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{name} has an invalid SHA-256 digest");
    }
    Ok(hex.to_ascii_lowercase())
}

fn tap_branch(token: &str) -> Result<String> {
    let endpoint = format!("repos/{TAP_REPO}");
    let repository = gh_json(token, &["api", &endpoint])?;
    repository["default_branch"]
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("cannot determine the default branch of {TAP_REPO}"))
}

fn read_remote_cask(token: &str) -> Result<Remote> {
    let endpoint = format!("repos/{TAP_REPO}/contents/{CASK_PATH}");
    if let Some(value) = gh_json_optional(token, &["api", &endpoint])? {
        let sha = value["sha"]
            .as_str()
            .context("GitHub returned no blob SHA for the Cask")?
            .to_owned();
        let encoded = value["content"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .filter(|character| !character.is_ascii_whitespace())
            .collect::<String>();
        let content = if encoded.is_empty() {
            String::new()
        } else {
            let bytes = STANDARD
                .decode(encoded)
                .context("the tap returned invalid base64 for the Cask")?;
            String::from_utf8(bytes).context("the tap Cask is not UTF-8")?
        };
        return Ok(Remote {
            exists: true,
            sha: Some(sha),
            content,
        });
    }

    let endpoint = format!("repos/{TAP_REPO}/contents/{CASK_DIR}");
    let listing = gh_json(token, &["api", &endpoint]).with_context(|| {
        format!("cannot read {TAP_REPO}/{CASK_DIR}; check HOMEBREW_TAP_TOKEN scope")
    })?;
    let entries = listing
        .as_array()
        .with_context(|| format!("{TAP_REPO}/{CASK_DIR} did not return a directory listing"))?;
    if entries
        .iter()
        .any(|entry| entry["name"].as_str() == Some("litematica-preview.rb"))
    {
        bail!("{CASK_PATH} exists but could not be read");
    }

    Ok(Remote {
        exists: false,
        sha: None,
        content: String::new(),
    })
}

fn plan(remote: &Remote, new_cask: &str, version: &Version, repair: bool) -> Result<Action> {
    if !remote.exists {
        return Ok(Action::Create);
    }

    let declared = cask_name(&remote.content).context("cannot find cask name in the tap file")?;
    if declared != CASK_NAME {
        bail!("{CASK_PATH} declares cask '{declared}'; refusing to modify it");
    }

    if remote.content == new_cask {
        return Ok(Action::Unchanged);
    }

    let raw = cask_version(&remote.content).context("cannot find version in the tap Cask")?;
    let current = Version::parse(raw)
        .with_context(|| format!("cannot parse the tap Cask version '{raw}'"))?;

    if current == *version {
        if repair {
            return Ok(Action::Update);
        }
        bail!(
            "{CASK_PATH} already has {version} with different contents; rerun via workflow_dispatch with repair enabled"
        );
    }
    if version > &current {
        return Ok(Action::Update);
    }

    println!(
        "the tap already tracks {current}, which is newer than {version}; leaving it unchanged"
    );
    Ok(Action::Skip)
}

fn cask_name(content: &str) -> Option<&str> {
    content.lines().find_map(|line| {
        let rest = line.strip_prefix("cask \"")?;
        rest.split_once('"').map(|(name, _)| name)
    })
}

fn cask_version(content: &str) -> Option<&str> {
    content.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix("version \"")?;
        rest.split_once('"').map(|(version, _)| version)
    })
}

fn put_cask(
    token: &str,
    branch: &str,
    remote: &Remote,
    action: Action,
    version: &Version,
    cask: &str,
) -> Result<()> {
    let message = match action {
        Action::Create => format!("Add Litematica Preview {version}"),
        _ => format!("Update Litematica Preview to {version}"),
    };
    let mut payload = serde_json::json!({
        "message": message,
        "content": STANDARD.encode(cask.as_bytes()),
        "branch": branch,
    });
    if let Some(sha) = &remote.sha {
        payload["sha"] = Value::String(sha.clone());
    }

    let endpoint = format!("repos/{TAP_REPO}/contents/{CASK_PATH}");
    let mut child = gh_command(token)
        .args(["api", "--method", "PUT", "--input", "-", endpoint.as_str()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to run gh; install the GitHub CLI")?;

    child
        .stdin
        .as_mut()
        .context("gh stdin was not captured")?
        .write_all(payload.to_string().as_bytes())?;

    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "GitHub rejected the Cask update: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn render_cask(version: &Version, arm_sha: &str, intel_sha: &str) -> String {
    format!(
        r#"cask "{CASK_NAME}" do
  arch arm: "aarch64", intel: "x86_64"

  version "{version}"
  sha256 arm:   "{arm_sha}",
         intel: "{intel_sha}"

  url "https://github.com/{SOURCE_REPO}/releases/download/v#{{version}}/LitematicaPreview-#{{version}}-#{{arch}}.dmg"
  name "Litematica Preview"
  desc "View Minecraft schematic and structure files"
  homepage "https://github.com/{SOURCE_REPO}"

  livecheck do
    url "https://github.com/{SOURCE_REPO}"
    strategy :github_latest
  end

  depends_on macos: :big_sur

  auto_updates true

  app "Litematica Preview.app"
end
"#
    )
}

fn gh_json(token: &str, args: &[&str]) -> Result<Value> {
    let output = gh_output(token, args)?;
    if !output.status.success() {
        bail!(
            "gh {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout)
        .with_context(|| format!("gh {} returned invalid JSON", args.join(" ")))
}

fn gh_json_optional(token: &str, args: &[&str]) -> Result<Option<Value>> {
    let output = gh_output(token, args)?;
    if !output.status.success() {
        return Ok(None);
    }
    serde_json::from_slice(&output.stdout)
        .map(Some)
        .with_context(|| format!("gh {} returned invalid JSON", args.join(" ")))
}

fn gh_output(token: &str, args: &[&str]) -> Result<Output> {
    gh_command(token)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("failed to run gh; install the GitHub CLI")
}

fn gh_command(token: &str) -> Command {
    let mut command = Command::new("gh");
    command
        .env("GH_TOKEN", token)
        .env("GH_PAGER", "cat")
        .env("NO_COLOR", "1")
        .env_remove("GH_DEBUG")
        .env_remove("DEBUG")
        .env_remove("GH_HOST");
    command
}

fn source_token() -> Result<String> {
    for name in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(value) = std::env::var(name)
            && !value.trim().is_empty()
        {
            return Ok(value);
        }
    }
    bail!("GITHUB_TOKEN or GH_TOKEN is required to read the release")
}

fn tap_token() -> Result<String> {
    let value = std::env::var("HOMEBREW_TAP_TOKEN").context(
        "HOMEBREW_TAP_TOKEN with Contents read/write on Arcadi4/homebrew-tap is required",
    )?;
    if value.trim().is_empty() {
        bail!("HOMEBREW_TAP_TOKEN is empty");
    }
    Ok(value)
}

fn emit(result: &str, version: &Version) -> Result<()> {
    println!("result={result}");
    if let Some(path) = std::env::var_os("GITHUB_OUTPUT").filter(|path| !path.is_empty()) {
        let path = std::path::PathBuf::from(path);
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("cannot open GITHUB_OUTPUT at {}", path.display()))?;
        writeln!(file, "result={result}")?;
        writeln!(file, "version={version}")?;
    }
    Ok(())
}
