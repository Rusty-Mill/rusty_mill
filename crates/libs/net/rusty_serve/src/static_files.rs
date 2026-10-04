//! Static files for the web UI: a directory of built assets served read-only.
//!
//! Path handling is the whole security story here, so it is small and
//! tested: a request path is split into segments, and any segment that is
//! empty, `.`, `..`, hidden, or contains a separator or NUL is refused before
//! the filesystem is touched.

use std::path::{Path, PathBuf};

/// A file ready to send.
pub struct Asset {
    pub body: Vec<u8>,
    pub content_type: &'static str,
}

/// Look up `request_path` (no query string) under `root`. `/` and any path
/// that is not a file serve `index.html`, so client-side routes survive a
/// reload. `None` when `root` has no index or the path is refused.
pub fn load(root: &Path, request_path: &str) -> Option<Asset> {
    let target = resolve(root, request_path);
    let (file, content_type) = match target {
        Some(file) if file.is_file() => {
            let ct = content_type(&file);
            (file, ct)
        }
        _ => (root.join("index.html"), "text/html; charset=utf-8"),
    };
    let body = std::fs::read(file).ok()?;
    Some(Asset { body, content_type })
}

/// The file `request_path` names under `root`, if the path is acceptable.
fn resolve(root: &Path, request_path: &str) -> Option<PathBuf> {
    let mut file = root.to_path_buf();
    let mut any = false;
    for segment in request_path.split('/').filter(|s| !s.is_empty()) {
        if !safe_segment(segment) {
            return None;
        }
        file.push(segment);
        any = true;
    }
    any.then_some(file)
}

fn safe_segment(segment: &str) -> bool {
    !segment.starts_with('.') && !segment.contains(['\\', '\0', ':']) && !segment.contains('%')
}

fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<html>app</html>").unwrap();
        std::fs::create_dir(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets/app.js"), "x()").unwrap();
        std::fs::write(dir.path().join(".secret"), "s").unwrap();
        dir
    }

    #[test]
    fn serves_files_with_their_type_and_falls_back_to_index() {
        let dir = site();
        let js = load(dir.path(), "/assets/app.js").unwrap();
        assert_eq!(
            (js.body.as_slice(), js.content_type),
            (&b"x()"[..], "text/javascript; charset=utf-8")
        );
        for path in ["/", "", "/missing", "/some/deep/route"] {
            let page = load(dir.path(), path).unwrap();
            assert_eq!(page.body, b"<html>app</html>", "{path}");
        }
    }

    #[test]
    fn refuses_traversal_hidden_files_and_odd_segments() {
        let dir = site();
        for path in [
            "/../secret",
            "/assets/../../x",
            "/.secret",
            "/a\\b",
            "/%2e%2e/x",
            "/c:/x",
        ] {
            let served = load(dir.path(), path).unwrap();
            assert_eq!(
                served.body, b"<html>app</html>",
                "{path} must not reach a file"
            );
        }
    }

    #[test]
    fn no_index_means_nothing_to_serve() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path(), "/").is_none());
    }
}
