//! Optional card recognition: proxy to local service and fetch card details from Pokemon TCG API.

use serde::Deserialize;

/// Response from local recognition service (expected JSON: { "cardId": "setX-nnn" }).
#[derive(Debug, Deserialize)]
pub struct RecognitionResponse {
    pub card_id: Option<String>,
    #[serde(alias = "cardId")]
    pub card_id_alt: Option<String>,
}

impl RecognitionResponse {
    pub fn card_id(&self) -> Option<&str> {
        self.card_id
            .as_deref()
            .or(self.card_id_alt.as_deref())
    }
}

/// Call local recognition service with image bytes. Returns card id on success.
pub async fn recognize_card(
    service_url: &str,
    image_bytes: &[u8],
) -> Result<String, RecognitionError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| RecognitionError::Client(e.to_string()))?;
    let part = reqwest::multipart::Part::bytes(image_bytes.to_vec())
        .file_name("card.jpg")
        .mime_str("image/jpeg")
        .map_err(|e| RecognitionError::Client(e.to_string()))?;
    let form = reqwest::multipart::Form::new().part("image", part);
    let resp = client
        .post(service_url.trim_end_matches('/').to_owned() + "/recognize")
        .multipart(form)
        .send()
        .await
        .map_err(|e| RecognitionError::Service(e.to_string()))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(RecognitionError::ServiceUnavailable(status, body));
    }
    let body: RecognitionResponse = resp
        .json()
        .await
        .map_err(|e| RecognitionError::Service(e.to_string()))?;
    body.card_id()
        .map(str::to_string)
        .ok_or(RecognitionError::NoCardId)
}

#[derive(Debug)]
pub enum RecognitionError {
    NotConfigured,
    Client(String),
    Service(String),
    /// Recognition service returned non-2xx (status, response body).
    ServiceUnavailable(u16, String),
    NoCardId,
}

/// Pokemon TCG API v2 card response (subset we need).
#[derive(Debug, Deserialize)]
pub struct TcgCardResponse {
    pub data: TcgCardData,
}

#[derive(Debug, Deserialize)]
pub struct TcgCardData {
    pub id: String,
    pub name: String,
    pub set: TcgSet,
    pub images: TcgCardImages,
}

#[derive(Debug, Deserialize)]
pub struct TcgSet {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct TcgCardImages {
    pub large: Option<String>,
    pub small: Option<String>,
}

/// Fetch card by id from api.pokemontcg.io.
pub async fn fetch_card_details(card_id: &str) -> Result<CardDetails, String> {
    let url = format!("https://api.pokemontcg.io/v2/cards/{}", card_id);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("API returned {}", resp.status()));
    }
    let body: TcgCardResponse = resp.json().await.map_err(|e| e.to_string())?;
    Ok(CardDetails {
        id: body.data.id.clone(),
        name: body.data.name.clone(),
        set_name: body.data.set.name.clone(),
        image_url: body
            .data
            .images
            .large
            .or(body.data.images.small)
            .unwrap_or_default(),
    })
}

#[derive(Debug, Clone)]
pub struct CardDetails {
    pub id: String,
    pub name: String,
    pub set_name: String,
    pub image_url: String,
}
