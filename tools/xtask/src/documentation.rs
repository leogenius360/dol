use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use pulldown_cmark::{Event, Parser, Tag};

#[derive(Debug, Eq, PartialEq)]
struct BrokenLink {
    source: PathBuf,
    line: usize,
    target: String,
    resolved: Option<PathBuf>,
    reason: &'static str,
}

pub(crate) fn validate(root: &Path) -> Result<(), String> {
    let files = tracked_markdown_files(root)?;
    let broken = validate_files(root, &files)?;

    if broken.is_empty() {
        println!("Markdown links: {} files checked", files.len());
        return Ok(());
    }

    let mut message = format!(
        "Markdown link validation failed with {} broken link{}:",
        broken.len(),
        if broken.len() == 1 { "" } else { "s" }
    );
    for link in broken {
        let source = display_relative(root, &link.source);
        message.push_str(&format!(
            "\n  {}:{}: `{}`: {}",
            source.display(),
            link.line,
            link.target,
            link.reason
        ));
        if let Some(resolved) = link.resolved {
            message.push_str(&format!(
                " (resolved as `{}`)",
                display_relative(root, &resolved).display()
            ));
        }
    }

    Err(message)
}

fn tracked_markdown_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "-z", "--", "*.md"])
        .output()
        .map_err(|error| format!("failed to list tracked Markdown files with `git`: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`git ls-files` failed while listing Markdown files: {}",
            stderr.trim()
        ));
    }

    let mut files = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = std::str::from_utf8(path)
                .map_err(|error| format!("tracked Markdown path is not valid UTF-8: {error}"))?;
            Ok(root.join(path))
        })
        .collect::<Result<Vec<_>, String>>()?;
    files.sort();
    Ok(files)
}

fn validate_files(root: &Path, files: &[PathBuf]) -> Result<Vec<BrokenLink>, String> {
    let root = normalize_path(root);
    let mut broken = Vec::new();

    for source in files {
        let content = fs::read_to_string(source)
            .map_err(|error| format!("failed to read `{}`: {error}", source.display()))?;
        let source_directory = source.parent().unwrap_or(&root);

        for (event, range) in Parser::new(&content).into_offset_iter() {
            let target = match event {
                Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                    dest_url.into_string()
                }
                _ => continue,
            };

            let Some(local_target) = local_target(&target) else {
                continue;
            };
            let line = 1 + content.as_bytes()[..range.start]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count();

            let decoded = match percent_decode(&local_target) {
                Ok(decoded) => decoded,
                Err(reason) => {
                    broken.push(BrokenLink {
                        source: source.clone(),
                        line,
                        target,
                        resolved: None,
                        reason,
                    });
                    continue;
                }
            };
            let resolved = normalize_path(&source_directory.join(decoded));

            let (missing, reason) = if !resolved.starts_with(&root) {
                (true, "target escapes the repository")
            } else {
                match resolved.try_exists() {
                    Ok(true) => (false, ""),
                    Ok(false) => (true, "target does not exist"),
                    Err(_) => (true, "target could not be inspected"),
                }
            };

            if missing {
                broken.push(BrokenLink {
                    source: source.clone(),
                    line,
                    target,
                    resolved: Some(resolved),
                    reason,
                });
            }
        }
    }

    Ok(broken)
}

fn local_target(target: &str) -> Option<String> {
    let target = target.trim();
    if target.is_empty()
        || target.starts_with('#')
        || target.starts_with('?')
        || target.starts_with("//")
        || Path::new(target).is_absolute()
        || has_uri_scheme(target)
    {
        return None;
    }

    let path = target
        .split(['#', '?'])
        .next()
        .expect("split always produces an item");
    (!path.is_empty()).then(|| path.to_owned())
}

fn has_uri_scheme(target: &str) -> bool {
    let Some(colon) = target.find(':') else {
        return false;
    };
    let scheme = &target[..colon];
    scheme
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
        && scheme
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

fn percent_decode(target: &str) -> Result<String, &'static str> {
    let bytes = target.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }

        let Some(hex) = bytes.get(index + 1..index + 3) else {
            return Err("target contains an incomplete percent escape");
        };
        let high = hex_value(hex[0]).ok_or("target contains an invalid percent escape")?;
        let low = hex_value(hex[1]).ok_or("target contains an invalid percent escape")?;
        decoded.push((high << 4) | low);
        index += 3;
    }

    String::from_utf8(decoded).map_err(|_| "percent-decoded target is not valid UTF-8")
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn display_relative<'a>(root: &'a Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(root).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn accepts_supported_local_links_and_ignores_non_local_targets() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let source = root.join("docs/guide.md");
        write(&root.join("README.md"), "# Repository\n");
        write(&root.join("docs/an image.png"), "not really a png");
        write(
            &source,
            "[root](../README.md#top)\n\
             ![image](<an image.png?raw=1>)\n\
             [encoded](an%20image.png)\n\
             [reference][readme]\n\
             [readme]: ../README.md\n\
             [web](https://example.com/nope)\n\
             [mail](mailto:docs@example.com)\n\
             [anchor](#section)\n\
             `[code](missing.md)`\n\
             ```text\n[example](also-missing.md)\n```\n",
        );

        assert_eq!(validate_files(root, &[source]).unwrap(), Vec::new());
    }

    #[test]
    fn reports_every_missing_or_invalid_target_with_its_line() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let source = root.join("docs/guide.md");
        write(
            &source,
            "# Guide\n\n[missing](missing.md#section)\n![bad](bad%ZZ.png)\n[escape](../../outside.md)\n",
        );

        let broken = validate_files(root, std::slice::from_ref(&source)).unwrap();
        assert_eq!(broken.len(), 3);
        assert_eq!(broken[0].source, source);
        assert_eq!(broken[0].line, 3);
        assert_eq!(broken[0].target, "missing.md#section");
        assert_eq!(broken[0].reason, "target does not exist");
        assert_eq!(broken[1].line, 4);
        assert_eq!(
            broken[1].reason,
            "target contains an invalid percent escape"
        );
        assert_eq!(broken[2].line, 5);
        assert_eq!(broken[2].reason, "target escapes the repository");
    }

    #[test]
    fn identifies_uri_schemes_without_misclassifying_relative_paths() {
        assert!(has_uri_scheme("https://example.com"));
        assert!(has_uri_scheme("git+ssh://example.com/repo"));
        assert!(!has_uri_scheme("docs/chapter:one.md"));
        assert_eq!(
            local_target("chapter.md?plain=1#heading").as_deref(),
            Some("chapter.md")
        );
    }

    #[test]
    fn decodes_utf8_and_path_separators() {
        assert_eq!(percent_decode("caf%C3%A9.md"), Ok("café.md".to_owned()));
        assert_eq!(
            percent_decode("docs%2Fguide.md"),
            Ok("docs/guide.md".to_owned())
        );
    }
}
