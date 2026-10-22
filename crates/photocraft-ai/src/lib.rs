pub mod firefly_bar;
pub use firefly_bar::FireflyContextBar;

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct ContentPart {
    text: String,
}

#[derive(Serialize)]
struct Content {
    parts: Vec<ContentPart>,
}

#[derive(Serialize)]
struct GenerateRequest {
    contents: Vec<Content>,
}

#[derive(Deserialize, Debug)]
pub struct CandidatePart {
    pub text: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct CandidateContent {
    pub parts: Option<Vec<CandidatePart>>,
}

#[derive(Deserialize, Debug)]
pub struct Candidate {
    pub content: Option<CandidateContent>,
}

#[derive(Deserialize, Debug)]
pub struct GenerateResponse {
    pub candidates: Option<Vec<Candidate>>,
}

pub struct GeminiClient {
    api_key: String,
    model: String,
    http: reqwest::Client,
}

impl GeminiClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "gemini-2.5-flash".to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub async fn ask(&self, prompt: &str) -> Result<String, String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let body = GenerateRequest {
            contents: vec![Content {
                parts: vec![ContentPart { text: prompt.to_string() }],
            }],
        };

        let response = self.http.post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            return Err(format!("API Error {status}: {err_text}"));
        }

        let parsed: GenerateResponse = response.json()
            .await
            .map_err(|e| format!("JSON parse error: {e}"))?;

        let reply = parsed.candidates
            .and_then(|mut c| c.pop())
            .and_then(|c| c.content)
            .and_then(|cnt| cnt.parts.and_then(|mut p| p.pop()))
            .and_then(|p| p.text)
            .unwrap_or_else(|| "No text in reply".to_string());

        Ok(reply)
    }
}


