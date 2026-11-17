#[tauri::command]
pub async fn profile_keywords(role: String, topics: Vec<String>) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        if !matches!(
            role.as_str(),
            "editor" | "designer" | "educator" | "business" | "developer"
        ) {
            return Ok(Vec::new());
        }
        tauri::async_runtime::spawn_blocking(move || {
            download_keywords(&role, &topics).map_err(super::failure)
        })
        .await
        .map_err(super::failure)?
    }
    #[cfg(not(windows))]
    {
        let _ = (role, topics);
        Ok(Vec::new())
    }
}

#[cfg(windows)]
fn download_keywords(role: &str, topics: &[String]) -> anyhow::Result<Vec<String>> {
    use std::{collections::HashMap, io::Read, time::Duration};

    #[derive(serde::Deserialize)]
    struct Catalog {
        version: u32,
        roles: HashMap<String, Vec<String>>,
        topics: HashMap<String, Vec<String>>,
    }

    const CATALOG_URL: &str = "https://raw.githubusercontent.com/motionedits246-max/FrameGuard/refs/heads/feature/local-ocr/desktop/keyword-catalog.json";
    const MAX_BYTES: u64 = 32 * 1024;
    let response = reqwest::blocking::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(7))
        .build()?
        .get(CATALOG_URL)
        .send()?
        .error_for_status()?;
    let mut data = Vec::new();
    response.take(MAX_BYTES + 1).read_to_end(&mut data)?;
    anyhow::ensure!(
        data.len() <= MAX_BYTES as usize,
        "The keyword catalog is too large."
    );
    let catalog: Catalog = serde_json::from_slice(&data)?;
    anyhow::ensure!(
        catalog.version == 1,
        "The keyword catalog version is unavailable."
    );

    let Some(role_words) = catalog.roles.get(role) else {
        return Ok(Vec::new());
    };
    let mut words = Vec::new();
    for topic in ["work", "personal", "access"] {
        if topics.iter().any(|picked| picked == topic) {
            let list = if topic == "work" {
                Some(role_words)
            } else {
                catalog.topics.get(topic)
            };
            for term in list.into_iter().flatten().map(|term| term.trim()) {
                if !term.is_empty()
                    && term.chars().count() <= 90
                    && !term.chars().any(char::is_control)
                    && !words
                        .iter()
                        .any(|word: &String| word.eq_ignore_ascii_case(term))
                {
                    words.push(term.to_owned());
                }
            }
        }
    }
    words.truncate(12);
    Ok(words)
}
