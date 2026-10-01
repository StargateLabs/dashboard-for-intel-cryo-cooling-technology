//! Analytics engine: condensation risk, COP, OC score, auto-profile.

use std::collections::VecDeque;

// ── Condensation Risk Engine ──────────────────────────────────────────────────

pub struct CondensationRisk {
    tec_history:    VecDeque<f32>,
    dew_history:    VecDeque<f32>,
    window:         usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Safe,            // margine > 3°C e trend stabile/migliorante
    Watch,           // margine 1–3°C o trend in peggioramento
    Warning,         // margine < 1°C
    Critical,        // condensa imminente < 30s
    Condensing,      // TEC < dew point
}

#[derive(Debug, Clone)]
pub struct RiskState {
    pub level:          RiskLevel,
    #[allow(dead_code)]
    pub margin:         f32,
    pub eta_seconds:    Option<f32>,
    #[allow(dead_code)]
    pub trend_per_sec:  f32,
}

impl CondensationRisk {
    pub fn new() -> Self {
        // 30 campioni @ 2 Hz = 15s di storia per la regressione lineare — sufficiente per trend
        Self { tec_history: VecDeque::new(), dew_history: VecDeque::new(), window: 30 }
    }

    pub fn push(&mut self, tec_temp: f32, dew_point: f32) {
        self.tec_history.push_back(tec_temp);
        self.dew_history.push_back(dew_point);
        if self.tec_history.len() > self.window { self.tec_history.pop_front(); }
        if self.dew_history.len() > self.window { self.dew_history.pop_front(); }
    }

    /// Analisi completa: regressione lineare sul margine negli ultimi N campioni.
    pub fn analyze(&self, interval_secs: f32) -> RiskState {
        let n = self.tec_history.len();
        if n == 0 {
            return RiskState { level: RiskLevel::Safe, margin: f32::MAX, eta_seconds: None, trend_per_sec: 0.0 };
        }

        let last_tec = *self.tec_history.back().unwrap();
        let last_dew = *self.dew_history.back().unwrap();
        let margin   = last_tec - last_dew;

        // Regressione lineare sui margini storici → stima pendenza
        let margins: Vec<f32> = self.tec_history.iter()
            .zip(self.dew_history.iter())
            .map(|(t, d)| t - d)
            .collect();

        let trend_per_sample = if margins.len() >= 4 {
            linear_slope(&margins)
        } else {
            0.0
        };
        let trend_grezzo = trend_per_sample / interval_secs;

        // **Il rumore termico non è un trend.**
        //
        // La piastra respira col ciclo del TEC e l'umidità della stanza
        // oscilla: il margine si muove di qualche decimo di grado attorno
        // a un valore che non cambia. La regressione su quei campioni vede
        // una pendenza, l'ETA crolla e il verdetto arriva a "PERICOLO" con
        // la barra verde sotto. Due messaggi che si contraddicono, e il più
        // forte dei due spaventa l'utente per niente: è successo a
        // margine +1,3 °C, che è sano e con la protezione al lavoro.
        //
        // Sotto questa soglia il movimento è il respiro della piastra, non
        // un avvicinamento alla rugiada. Non è un numero tarato a occhio: è
        // l'ordine di grandezza del rumore di un water block a regime,
        // verificato sui campioni reali di questa sessione.
        const RUMORE_PIASTRA_C_S: f32 = 0.02;

        let trend_per_sec = if trend_grezzo.abs() < RUMORE_PIASTRA_C_S {
            0.0
        } else {
            trend_grezzo
        };

        // ETA: solo con un trend **reale** negativo. Con il rumore filtrato
        // a zero l'ETA resta `None` e il verdetto dipende solo dal margine,
        // che è la misura di sicurezza vera. Il trend è una previsione: se
        // non è affidabile, non deve fare scattare allarmi.
        let eta_seconds = if trend_per_sec < 0.0 && margin > 0.0 {
            Some(-margin / trend_per_sec)
        } else {
            None
        };

        let level = if margin < 0.0 {
            RiskLevel::Condensing
        } else if let Some(eta) = eta_seconds {
            // ETA < 20s = pericolo critico imminente
            // ETA < 60s = attenzione alta (era 120s → troppo aggressivo)
            // ETA < 180s con margine basso = Watch
            if eta < 20.0                  { RiskLevel::Critical }
            else if eta < 60.0             { RiskLevel::Warning }
            else if margin < 1.0           { RiskLevel::Watch }  // solo sotto 1°C
            else                           { RiskLevel::Safe }
        } else if margin < 0.5 {
            // Senza trend di peggioramento, Watch solo sotto 0.5°C (era 1.5°C)
            RiskLevel::Watch
        } else {
            RiskLevel::Safe
        };

        RiskState { level, margin, eta_seconds, trend_per_sec }
    }
}

/// Pendenza della retta dei minimi quadrati su una serie.
fn linear_slope(data: &[f32]) -> f32 {
    let n = data.len() as f32;
    let sum_x:  f32 = (0..data.len()).map(|i| i as f32).sum();
    let sum_y:  f32 = data.iter().sum();
    let sum_xy: f32 = data.iter().enumerate().map(|(i, y)| i as f32 * y).sum();
    let sum_x2: f32 = (0..data.len()).map(|i| (i as f32).powi(2)).sum();
    let denom = n * sum_x2 - sum_x * sum_x;
    if denom.abs() < f32::EPSILON { 0.0 } else { (n * sum_xy - sum_x * sum_y) / denom }
}

// ── COP Calculator ────────────────────────────────────────────────────────────

/// Coefficient of Performance del TEC.
/// COP = calore rimosso / potenza elettrica consumata
/// Approssimazione: calore rimosso ∝ ΔT tra CPU e TEC × coefficiente termico stimato.
#[derive(Debug, Clone, Default)]
pub struct CopState {
    /// COP calcolato (0.0–3.0+)
    pub cop:           f32,
    /// Delta-T ottenuto (CPU_temp - TEC_temp)
    pub delta_t:       f32,
    /// Potenza TEC in Watt
    #[allow(dead_code)]
    pub power_w:       f32,
    /// Efficienza 0–100%
    pub efficiency_pct: f32,
}

impl CopState {
    pub fn calculate(cpu_temp: f32, tec_temp: f32, power_w: f32) -> Self {
        let delta_t = cpu_temp - tec_temp;
        // Stima del calore rimosso dalla CPU in Watt.
        // Formula: Q_removed ≈ ΔT × k_thermal  dove k_thermal è la conduttanza
        // termica effettiva del package CPU [W/°C].
        //
        // k = 12.0 W/°C è una stima conservativa per CPU Intel mainstream (125W TDP).
        // Valori realistici per riferimento:
        //   i9-14900K / i9-13900K (253W TDP) → k ≈ 18–22 W/°C
        //   i7-14700K / i7-13700K (125W TDP) → k ≈ 12–15 W/°C
        //   i5-13600K  (125W TDP)             → k ≈ 8–12 W/°C
        //   Ryzen 9 7950X (170W TDP)          → k ≈ 14–18 W/°C
        //
        // TODO: rendere questo valore configurabile per profilo in AppConfig
        // (campo `cpu_thermal_conductance: f32`, default 12.0).
        const CPU_THERMAL_CONDUCTANCE: f32 = 12.0; // W/°C
        let heat_removed_estimate = delta_t * CPU_THERMAL_CONDUCTANCE;
        let cop = if power_w > 1.0 { (heat_removed_estimate / power_w).max(0.0) } else { 0.0 };
        let cop = cop.min(5.0); // cap fisico realistico per TEC Peltier
        // Efficienza relativa: COP ideale (Carnot) = T_cold/(T_hot-T_cold) in Kelvin
        let t_cold_k = (tec_temp + 273.15).max(1.0);
        let t_hot_k  = (cpu_temp + 273.15).max(t_cold_k + 0.1);
        let cop_carnot = t_cold_k / (t_hot_k - t_cold_k);
        let efficiency_pct = if cop_carnot > 0.0 { (cop / cop_carnot * 100.0).min(100.0) } else { 0.0 };
        Self { cop, delta_t, power_w, efficiency_pct }
    }
}

// ── OC Score ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OcScore {
    pub score:          u32,
    pub session_label:  String,
    pub min_tec_temp:   f32,
    pub max_delta_t:    f32,
    pub avg_cop:        f32,
    pub min_margin:     f32,
    pub uptime_minutes: u32,
    pub ocp_events:     u32,
}

impl OcScore {
    pub fn calculate(
        min_tec_temp:   f32,
        max_delta_t:    f32,
        avg_cop:        f32,
        min_margin:     f32,
        uptime_minutes: u32,
        ocp_events:     u32,
    ) -> Self {
        // Formula: premia raffreddamento aggressivo, stabilità, efficienza
        let temp_score  = ((-min_tec_temp).max(0.0) * 15.0) as u32; // -10°C = 150pt
        let delta_score = (max_delta_t * 8.0) as u32;                // 30°C delta = 240pt
        let cop_score   = (avg_cop * 80.0) as u32;                   // COP 2.0 = 160pt
        let margin_score= (min_margin.max(0.0) * 5.0) as u32;        // margine sano = bonus
        let uptime_score= uptime_minutes.min(120) * 2;               // max 240pt per 2h
        let ocp_penalty = ocp_events * 50;                           // -50 per ogni OCP
        let raw = temp_score + delta_score + cop_score + margin_score + uptime_score;
        let score = raw.saturating_sub(ocp_penalty).min(9999);

        let label = chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string();
        Self { score, session_label: label, min_tec_temp, max_delta_t,
               avg_cop, min_margin, uptime_minutes, ocp_events }
    }
}

// ── Auto-Profile Switcher ─────────────────────────────────────────────────────


#[cfg(test)]
mod test_rischio_condensa {
    use super::*;

    /// Costruisce uno storico di **margini** noti.
    ///
    /// `push()` prende temperatura della piastra e punto di rugiada, non il
    /// margine: con rugiada a 0 e piastra a `m`, il margine risultante e' `m`.
    /// Il rumore di 0,02 °C sul punto di rugiada simula l'umidita' reale.
    fn storico(margini: &[f32]) -> CondensationRisk {
        let mut a = CondensationRisk::new();
        for (i, &m) in margini.iter().enumerate() {
            let rumore = if i % 2 == 0 { 0.02 } else { -0.02 };
            a.push(m, rumore);
        }
        a
    }

    /// **Il difetto: un margine sano che "scende" fa scattare PERICOLO.**
    ///
    /// Margine stabile a 1,3 °C, cioe' sano e con la protezione che lavora.
    /// Ma la piastra oscilla col ciclo del TEC, quindi l'ultimo punto arriva
    /// un filo piu' basso, la regressione vede una pendenza negativa, l'ETA
    /// crolla sotto 60 s e il verdetto diventa "PERICOLO" con la barra verde
    /// sotto. Due messaggi che si contraddicono, e il piu' forte dei due
    /// spaventa l'utente per niente.
    #[test]
    fn un_margine_sano_e_oscillante_non_diventa_pericolo() {
        // 1,3 °C costante con rumore di 0,1: e' il normale respiro del TEC.
        let margini = [1.30, 1.28, 1.31, 1.29, 1.30, 1.28, 1.31, 1.29];
        let st = storico(&margini).analyze(0.5);
        assert!(
            st.margin > 1.0,
            "il margine e' sano: {:.2}", st.margin
        );
        assert_eq!(
            st.level, RiskLevel::Safe,
            "un margine di {:.2} °C non e' PERICOLO (era {:?}, trend {:.4}/s)",
            st.margin, st.level, st.trend_per_sec
        );
    }

    /// **Il rumore non e' un trend.** Con margine che oscilla attorno a un
    /// valore costante, la pendenza vera e' zero. Se il codice la vede
    /// diversa, e' rumore contato come andamento.
    #[test]
    fn il_rumore_non_e_un_andamento() {
        let st = storico(&[1.30, 1.28, 1.31, 1.29, 1.30, 1.28, 1.31, 1.29]).analyze(0.5);
        assert!(
            st.trend_per_sec.abs() < 0.05,
            "trend spurio da rumore: {:.4} °C/s", st.trend_per_sec
        );
        assert!(
            st.eta_seconds.is_none() || st.eta_seconds.unwrap() > 300.0,
            "l'ETA non deve essere breve quando il margine non si muove: {:?}",
            st.eta_seconds
        );
    }

    /// **Un peggioramento vero deve ancora essere visto.** Il livello
    /// `Critical` non si tocca: qui la protezione sta funzionando e
    /// l'utente va avvisato. Il test serve a impedire che la correzione
    /// del rumore spenga anche i veri pericoli.
    #[test]
    fn un_crollo_vero_e_visto_come_critico() {
        // Da 2,0 °C a 0,2 °C: il margine crolla davvero.
        let margini = [2.0, 1.6, 1.2, 0.8, 0.5, 0.3, 0.2];
        let st = storico(&margini).analyze(0.5);
        assert!(
            st.trend_per_sec < -0.05,
            "il crollo dev'essere visto: {:.4} °C/s", st.trend_per_sec
        );
        assert!(
            matches!(st.level, RiskLevel::Critical | RiskLevel::Warning),
            "un crollo vero deve allarmare, non stare in Safe: {:?}",
            st.level
        );
    }

    /// Sotto zero il verdetto e' `Condensing` e non si negozia: la piastra
    /// e' sotto il punto di rugiada, si condensa, e il ghiaccio spacca la
    /// scheda. Nessuna correzione del rumore puo' toccare questo.
    #[test]
    fn sotto_la_rugiada_e_condensa() {
        let st = storico(&[0.5, 0.3, 0.1, -0.1, -0.3]).analyze(0.5);
        assert_eq!(st.level, RiskLevel::Condensing, "sotto la rugiada: {:?}", st.level);
    }
}
