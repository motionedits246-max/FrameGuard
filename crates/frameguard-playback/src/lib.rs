//! Local HTTP playback for WebKitGTK, with access scoped to selected files.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use axum::{
    extract::{Path as RoutePath, Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tokio::{net::TcpListener, task::JoinHandle};
use tower_http::services::ServeFile;
use uuid::Uuid;

#[derive(Clone)]
struct Registry {
    host: String,
    files: Arc<Mutex<HashMap<String, PathBuf>>>,
}

pub struct Server {
    registry: Registry,
    task: JoinHandle<()>,
}

/// Keeping this value alive grants access to exactly one selected MP4.
pub struct Media {
    registry: Registry,
    token: String,
    url: String,
}

impl Server {
    pub async fn bind() -> io::Result<Self> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let registry = Registry {
            host: listener.local_addr()?.to_string(),
            files: Arc::default(),
        };
        let router = Router::new()
            .route("/media/{token}", get(serve))
            .with_state(registry.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Ok(Self { registry, task })
    }

    pub fn open(&self, path: &Path) -> io::Result<Media> {
        let path = path.canonicalize()?;
        if !path.is_file()
            || !path
                .extension()
                .and_then(|part| part.to_str())
                .is_some_and(|part| part.eq_ignore_ascii_case("mp4"))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Choose an MP4 file.",
            ));
        }
        let token = Uuid::new_v4().simple().to_string();
        self.registry
            .files
            .lock()
            .map_err(|_| io::Error::other("The media registry is unavailable."))?
            .insert(token.clone(), path);
        Ok(Media {
            url: format!("http://{}/media/{token}", self.registry.host),
            registry: self.registry.clone(),
            token,
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        if let Ok(mut files) = self.registry.files.lock() {
            files.clear();
        }
    }
}

impl Media {
    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        if let Ok(mut files) = self.registry.files.lock() {
            files.remove(&self.token);
        }
    }
}

async fn serve(
    State(registry): State<Registry>,
    RoutePath(token): RoutePath<String>,
    request: Request,
) -> Response {
    if request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        != Some(registry.host.as_str())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if request.headers().get(header::ORIGIN).is_some_and(|origin| {
        ![
            "tauri://localhost",
            "http://tauri.localhost",
            "https://tauri.localhost",
            "null",
        ]
        .iter()
        .any(|allowed| origin == *allowed)
    }) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let path = match registry.files.lock() {
        Ok(files) => files.get(&token).cloned(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let Some(path) = path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = match ServeFile::new(path).try_call(request).await {
        Ok(response) => response.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn streams_ranges_and_revokes_only_the_selected_file() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("sample.mp4");
        std::fs::write(&file, b"0123456789").unwrap();
        std::fs::write(directory.path().join("private.mp4"), b"private").unwrap();
        let server = Server::bind().await.unwrap();
        let media = server.open(&file).unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = media.url().to_owned();
        assert!(url.starts_with("http://127.0.0.1:"));

        let response = client
            .get(&url)
            .header(header::RANGE, "bytes=3-5")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 3-5/10");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.bytes().await.unwrap().as_ref(), b"345");
        let response = client
            .get(&url)
            .header(header::RANGE, "bytes=-2")
            .send()
            .await
            .unwrap();
        assert_eq!(response.bytes().await.unwrap().as_ref(), b"89");
        let response = client.head(&url).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
        assert!(response.bytes().await.unwrap().is_empty());
        assert_eq!(
            client
                .get(&url)
                .header(header::RANGE, "bytes=99-")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::RANGE_NOT_SATISFIABLE
        );
        assert_eq!(
            client
                .get(&url)
                .header(header::ORIGIN, "https://example.com")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .get(&url)
                .header(header::HOST, "example.com")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            client
                .get(format!("http://{}/media/private.mp4", server.registry.host))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        drop(media);
        assert_eq!(
            client.get(&url).send().await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
}
