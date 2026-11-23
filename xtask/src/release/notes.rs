use anyhow::{Context, Result};
use semver::Version;

pub fn compose(tag: &str, version: &Version) -> Result<String> {
    let changelog = std::fs::read_to_string(crate::root().join("CHANGELOG.md"))
        .context("could not read release notes from CHANGELOG.md")?;
    let mut section = changelog_section(&changelog, tag);
    if section.is_empty() {
        section = changelog_section(&changelog, "Unreleased");
    }
    let install = install(version);

    Ok(if section.trim().is_empty() {
        format!("{install}\n")
    } else {
        format!("{}\n\n{install}\n", section.trim())
    })
}

fn changelog_section(changelog: &str, tag: &str) -> String {
    let changelog = changelog.strip_prefix('\u{feff}').unwrap_or(changelog);
    let lines: Vec<&str> = changelog.lines().collect();
    let heading = format!("# {tag}");
    let Some(start) = lines
        .iter()
        .position(|line| line.trim_end() == heading.as_str())
    else {
        return String::new();
    };

    let mut section = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in &lines[start + 1..] {
        if let Some((marker, length)) = fence {
            section.push((*line).to_owned());
            if is_closing_fence(line, marker, length) {
                fence = None;
            }
            continue;
        }
        if let Some((marker, length)) = opening_fence(line) {
            fence = Some((marker, length));
            section.push((*line).to_owned());
            continue;
        }
        if is_heading(line) {
            break;
        }
        section.push(demote_heading(line).to_owned());
    }

    section.join("\n").trim_matches('\n').to_owned()
}

fn opening_fence(line: &str) -> Option<(char, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let marker = rest.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let length = rest
        .chars()
        .take_while(|character| *character == marker)
        .count();
    (length >= 3).then_some((marker, length))
}

fn is_closing_fence(line: &str, marker: char, opening_length: usize) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let length = rest
        .chars()
        .take_while(|character| *character == marker)
        .count();
    length >= opening_length && rest[length..].trim().is_empty()
}

fn is_heading(line: &str) -> bool {
    line.starts_with('#')
        && line
            .as_bytes()
            .get(1)
            .is_none_or(|byte| byte.is_ascii_whitespace())
}

fn demote_heading(line: &str) -> &str {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if (2..=6).contains(&hashes)
        && line
            .as_bytes()
            .get(hashes)
            .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        &line[1..]
    } else {
        line
    }
}

fn install(version: &Version) -> String {
    format!(
        "# How to install

## Windows x64

- A one-click installer (`LitematicaPreview-{version}-x86_64-setup.exe`): installs per-user without requiring administrator privileges.
- A portable .zip package (`LitematicaPreview-{version}-x86_64-portable.zip`): download and extract the archive, then run LitematicaPreview.exe.

## macOS

- Apple silicon (`LitematicaPreview-{version}-aarch64.dmg`) and Intel (`LitematicaPreview-{version}-x86_64.dmg`): open the disk image and drag Litematica Preview to Applications. Release builds are signed and notarized.

## Linux x64

- A Debian package (`LitematicaPreview-{version}-x86_64.deb`): install with `sudo apt install ./LitematicaPreview-{version}-x86_64.deb`.
- An AppImage (`LitematicaPreview-{version}-x86_64.AppImage`): run `chmod +x` on the file, then launch it. Its `.zsync` companion supports AppImageUpdate.

Linux releases target Ubuntu 22.04 and later x64 systems with Vulkan and a compatible driver."
    )
}
