//! Generate immutable asset URLs at startup. Source files stay editable and
//! `cargo run` needs no frontend build. Old versions remain available across
//! deployments for tabs which already loaded an older HTML document.
use axum::{extract::Request, http::header, middleware::Next, response::Response};
use sha2::{Digest, Sha256};
use std::{fs, io, path::Path};

fn publish(root: &Path, name: &str, bytes: &[u8]) -> io::Result<String> {
    let digest = format!("{:x}", Sha256::digest(bytes));
    let filename = format!("{}.{}", &digest[..20], name);
    let destination = root.join("assets").join(&filename);
    if !destination.exists() {
        // A second development server can prepare the same assets concurrently.
        let mut temporary = tempfile::NamedTempFile::new_in(root.join("assets"))?;
        use std::io::Write;
        temporary.write_all(bytes)?;
        temporary
            .persist(&destination)
            .map_err(|error| error.error)?;
    }
    Ok(format!("/assets/{filename}"))
}

pub fn prepare(root: &Path) -> io::Result<String> {
    fs::create_dir_all(root.join("assets"))?;
    let mut css = fs::read_to_string(root.join("app.css"))?;
    for entry in fs::read_dir(root.join("fonts"))? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("woff2") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy();
        let url = publish(root, &name, &fs::read(&path)?)?;
        css = css.replace(&format!("/fonts/{name}"), &url);
    }
    let mut html = fs::read_to_string(root.join("index.html"))?;
    let stylesheet = publish(root, "app.css", css.as_bytes())?;
    html = html.replace("\"/app.css\"", &format!("\"{stylesheet}\""));
    for name in ["terminal.js", "hosts.js", "app.js", "session-model.js"] {
        let url = publish(root, name, &fs::read(root.join(name))?)?;
        html = html.replace(&format!("\"/{name}\""), &format!("\"{url}\""));
    }
    Ok(html)
}

fn cache_policy(path: &str, successful: bool) -> &'static str {
    let versioned = path
        .strip_prefix("/assets/")
        .and_then(|name| name.split_once('.'))
        .is_some_and(|(hash, name)| {
            hash.len() == 20
                && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                && !name.contains('/')
        });
    if successful && versioned {
        "public, max-age=31536000, immutable"
    } else if path == "/" || path == "/index.html" {
        "no-cache, must-revalidate"
    } else {
        // Terminal snapshots, uploads, previews and all other API data remain
        // private. Mutable source URLs must never inherit immutable caching.
        "no-store"
    }
}

pub async fn cache_control(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    let policy = cache_policy(&path, response.status().is_success());
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(policy),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_publish_new_urls_and_keep_old_bytes() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("assets")).unwrap();
        let first = publish(root.path(), "app.js", b"old").unwrap();
        assert_eq!(publish(root.path(), "app.js", b"old").unwrap(), first);
        let second = publish(root.path(), "app.js", b"new").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            fs::read(root.path().join(first.trim_start_matches('/'))).unwrap(),
            b"old"
        );
    }

    #[test]
    fn only_successful_versioned_assets_are_immutable() {
        let asset = "/assets/0123456789abcdef0123.app.js";
        assert!(cache_policy(asset, true).contains("immutable"));
        for path in [
            "/api/capture",
            "/api/serve-file",
            "/app.js",
            "/assets/app.js",
        ] {
            assert_eq!(cache_policy(path, true), "no-store");
        }
        assert_eq!(cache_policy(asset, false), "no-store");
        assert_eq!(cache_policy("/", true), "no-cache, must-revalidate");
    }
}
