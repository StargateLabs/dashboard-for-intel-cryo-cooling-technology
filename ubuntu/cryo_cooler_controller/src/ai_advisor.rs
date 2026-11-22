//! AI Advisor — chiama l'API Anthropic con i dati live del sistema
//! e ritorna raccomandazioni sui parametri PID ottimali.
//!
//! Modello: claude-sonnet-4-6 (corrente marzo 2026)
//!
//! La API key viene letta da:
//!   1. Variabile d'ambiente ANTHROPIC_API_KEY
//!   2. File %APPDATA%\StargateLabsCryo\api_key.txt  ← unificato con config.json e sessions.db
//!
//! Usa reqwest::blocking per semplicità (chiamato da thread separato via Task::perform).

use serde::{Deserialize, Serialize};

// ── Strutture richiesta/risposta API ────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ApiRequest {
    model:      String,
    max_tokens: u32,
    system:     String,
    messages:   Vec<ApiMessage>,
}

#[derive(Debug, Serialize)]
struct ApiMessage {
    role:    String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

// ── Contesto dati di sessione da mandare all'AI ───────────────────────────

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub tec_temp:      f32,
    pub dew_point:     f32,
    pub cpu_temp:      Option<f32>,
    pub gpu_temp:      Option<f32>,   // FEAT #3: temp GPU per AI advisor in workload render/AI
    pub power_watts:   f32,
    pub humidity:      f32,
    pub cop:           f32,
    pub cop_eff_pct:   f32,
    pub margin:        f32,
    pub oc_score:      u32,
    pub p_coef:        f32,
    pub i_coef:        f32,
    pub d_coef:        f32,
    pub set_point:     f32,
    pub max_power:     u8,
    pub ocp_events:    u32,
    pub uptime_min:    u32,
    pub tec_min:       f32,
    pub tec_avg:       f32,
    pub tec_max:       f32,
    pub workload_mode: String,
}

// ── Risultato del consiglio AI ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AiAdvice {
    /// Testo completo della raccomandazione
    pub text:          String,
    /// Parametri suggeriti estratti dalla risposta (se presenti)
    pub suggested_p:   Option<f32>,
    pub suggested_i:   Option<f32>,
    pub suggested_d:   Option<f32>,
    pub suggested_sp:  Option<f32>,
    pub suggested_pwr: Option<u8>,
}

// ── Leggi API key ────────────────────────────────────────────────────────

pub fn get_api_key() -> Option<String> {
    // 1. Variabile d'ambiente
    if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
        if !k.trim().is_empty() { return Some(k.trim().to_owned()); }
    }
    // 2. File locale
    let path = api_key_path();
    if let Ok(k) = std::fs::read_to_string(&path) {
        let k = k.trim().to_owned();
        if !k.is_empty() { return Some(k); }
    }
    None
}

pub fn save_api_key(key: &str) {
    let path = api_key_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, key.trim());
}

fn api_key_path() -> std::path::PathBuf {
    let base = std::env::var("APPDATA")
        .unwrap_or_else(|_| ".".to_owned());
    // Stessa directory di config.json e sessions.db — StargateLabsCryo, non StargateCryo
    std::path::PathBuf::from(base)
        .join("StargateLabsCryo")
        .join("api_key.txt")
}

// ── Chiamata API sincrona (da thread) ────────────────────────────────────

pub fn ask_advisor(snap: &SessionSnapshot, api_key: &str) -> Result<AiAdvice, String> {
    let system = "Sei un esperto di extreme overclocking con sistemi di raffreddamento \
        TEC (Peltier) su CPU Intel. Analizzi i dati live di una dashboard cryo cooling \
        e dai consigli precisi e tecnici sui parametri PID ottimali. \
        Rispondi SEMPRE in italiano. Sii conciso e diretto — max 200 parole. \
        Alla fine includi SEMPRE una sezione 'PARAMETRI SUGGERITI:' con i valori numerici \
        su righe separate nel formato esatto: P=X.XX I=X.XX D=X.XX OFFSET=X.X MAX_POT=XX\n\
        Dove X sono numeri. Esempio: P=1.20 I=0.05 D=0.80 OFFSET=-3.0 MAX_POT=85".to_owned();

    let user_msg = format!(
        "DATI LIVE SISTEMA:\n\
         Temp TEC: {:.1}°C (min {:.1} / med {:.1} / max {:.1})\n\
         Punto rugiada: {:.1}°C\n\
         Margine condensa: {:.1}°C\n\
         CPU temp: {}\n\
         GPU temp: {}\n\
         Potenza TEC: {:.1} W\n\
         Umidità: {:.0}%\n\
         COP efficienza: {:.2} ({:.0}%)\n\
         OC Score: {}\n\
         Eventi OCP: {}\n\
         Uptime: {} min\n\
         Workload: {}\n\
         \n\
         PARAMETRI PID ATTUALI:\n\
         P={:.3}  I={:.3}  D={:.3}\n\
         Offset set-point: {:.1}°C\n\
         Potenza massima: {}%\n\
         \n\
         Analizza questi dati e dimmi:\n\
         1. Cosa sta andando bene e cosa si può migliorare\n\
         2. I parametri PID ottimali per la situazione attuale\n\
         3. Eventuali rischi o avvertenze",
        snap.tec_temp, snap.tec_min, snap.tec_avg, snap.tec_max,
        snap.dew_point,
        snap.margin,
        snap.cpu_temp.map(|t| format!("{:.1}°C", t)).unwrap_or("N/D".to_owned()),
        snap.gpu_temp.map(|t| format!("{:.1}°C", t)).unwrap_or("N/D".to_owned()),
        snap.power_watts,
        snap.humidity,
        snap.cop, snap.cop_eff_pct,
        snap.oc_score,
        snap.ocp_events,
        snap.uptime_min,
        snap.workload_mode,
        snap.p_coef, snap.i_coef, snap.d_coef,
        snap.set_point,
        snap.max_power,
    );

    let req_body = ApiRequest {
        model:      "claude-sonnet-4-6".to_owned(),  // modello corrente marzo 2026
        max_tokens: 600,
        system,
        messages:   vec![ApiMessage { role: "user".to_owned(), content: user_msg }],
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Errore client HTTP: {}", e))?;

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&req_body)
        .send()
        .map_err(|e| format!("Errore connessione API: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("API error {}: {}", status, body));
    }

    let api_resp: ApiResponse = resp.json()
        .map_err(|e| format!("Errore parsing risposta: {}", e))?;

    let text = api_resp.content.iter()
        .filter(|b| b.kind == "text")
        .filter_map(|b| b.text.as_deref())
        .collect::<Vec<_>>()
        .join("\n");

    // Estrai parametri suggeriti dalla sezione "PARAMETRI SUGGERITI:"
    let (sp, si, sd, ssp, spwr) = extract_params(&text);

    Ok(AiAdvice { text, suggested_p: sp, suggested_i: si, suggested_d: sd,
                   suggested_sp: ssp, suggested_pwr: spwr })
}

/// Estrae P, I, D, OFFSET, MAX_POT dalla risposta testuale.
///
/// Parsing robusto: cerca token `KEY=VALUE` su ogni riga della sezione
/// "PARAMETRI SUGGERITI:". Gestisce spazi attorno a `=` e più valori per riga.
/// Non usa contains() generico per evitare falsi match su altre parti del testo.
fn extract_params(text: &str) -> (Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<u8>) {
    let mut p   = None::<f32>;
    let mut i   = None::<f32>;
    let mut d   = None::<f32>;
    let mut sp  = None::<f32>;
    let mut pwr = None::<u8>;

    // Cerca solo nella sezione "PARAMETRI SUGGERITI:" per evitare falsi match
    // nel testo descrittivo (es. "aumenta P=" in mezzo a una spiegazione).
    // Se la sezione non esiste, usa tutto il testo come fallback.
    let search_text = if let Some(idx) = text.to_uppercase().find("PARAMETRI SUGGERITI") {
        &text[idx..]
    } else {
        text
    };

    for line in search_text.lines() {
        // Normalizza: rimuovi spazi attorno a = e uppercase
        let l: String = line.chars()
            .map(|c| if c == ' ' && false { c } else { c.to_ascii_uppercase() })
            .collect();
        let l = l.trim();

        // Estrae coppie KEY=VALUE dai token della riga
        for token in l.split_whitespace() {
            if let Some((key, val_str)) = token.split_once('=') {
                let val_str = val_str.trim_matches(|c: char| !c.is_ascii_digit() && c != '-' && c != '.');
                match key.trim() {
                    "P"       => { p   = val_str.parse().ok(); }
                    "I"       => { i   = val_str.parse().ok(); }
                    "D"       => { d   = val_str.parse().ok(); }
                    "OFFSET"  => { sp  = val_str.parse().ok(); }
                    "MAX_POT" => { pwr = val_str.parse::<f32>().ok().map(|v| v.clamp(0.0, 100.0) as u8); }
                    _ => {}
                }
            }
        }
    }
    (p, i, d, sp, pwr)
}

