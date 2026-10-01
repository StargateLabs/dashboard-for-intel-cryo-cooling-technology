//! PID Auto-Tuning Wizard — step-response method.
//!
//! Applica uno step di potenza al TEC e misura la risposta termica.
//! Calcola automaticamente P, I, D ottimali con il metodo
//! Ziegler-Nichols modificato per sistemi cryo.
//!
//! FASI:
//! 1. IDLE     — attesa condizioni iniziali stabili
//! 2. STEP     — applica 80% potenza per N secondi, registra risposta
//! 3. ANALYZE  — calcola guadagno K, tempo morto L, costante tempo T
//! 4. COMPUTE  — applica formula Z-N → P, I, D
//! 5. DONE / FAILED

use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq)]
pub enum WizardPhase {
    Idle,
    WaitingStable,      // attende temperatura stabile (< 0.1°C/s)
    ApplyingStep,       // 80% potenza, registra risposta
    Analyzing,          // elaborazione dati
    Done {              // risultati pronti
        p: f32, i: f32, d: f32,
        k_gain: f32, dead_time: f32, time_const: f32,
    },
    Failed(String),
}

pub struct PidWizard {
    pub phase:          WizardPhase,
    pub progress_pct:   u8,          // 0–100 per barra progresso
    pub log:            Vec<String>,
    temp_history:       VecDeque<f32>,
    step_start_temp:    f32,
    step_start_tick:    u32,
    current_tick:       u32,
    // Parametri
    step_power_pct:     u8,
    stable_threshold:   f32,   // °C/campione per considerare stabile (2 Hz → 0.05 = 0.1°C/s)
    step_duration_ticks: u32,  // ticks a 2 Hz: 60 ticks × 500ms = 30s
}

impl PidWizard {
    /// Frequenza reale di `tick()`.
    ///
    /// `tick()` gira dentro il ramo `Ok` di `tec.monitor()`, che viene
    /// chiamato solo quando `should_update()` lascia passare un tick: 500 ms,
    /// cioe' 2 Hz. Tutte le conversioni da campioni a secondi devono usare
    /// questo valore, non il frequenza nominale del loop UI (20 Hz) ne'
    /// tanto meno 100 Hz.
    const SAMPLE_HZ: f32 = 2.0;

    pub fn new() -> Self {
        PidWizard {
            phase:               WizardPhase::Idle,
            progress_pct:        0,
            log:                 Vec::new(),
            temp_history:        VecDeque::new(),
            step_start_temp:     0.0,
            step_start_tick:     0,
            current_tick:        0,
            step_power_pct:      80,
            // tick() viene chiamato a 2 Hz (Tick gate da should_update @ 500ms).
            // step_duration_ticks: tick × 500ms/tick = secondi
            // → 60 tick × 500ms = 30s per lo step test
            stable_threshold:    0.05,  // °C/campione (ogni 500ms)
            step_duration_ticks: 60,    // 60 × 500ms = 30s
        }
    }

    /// Avvia il wizard. Ritorna true se avviato correttamente.
    pub fn start(&mut self) {
        self.phase         = WizardPhase::WaitingStable;
        self.progress_pct  = 0;
        self.current_tick  = 0;
        self.temp_history.clear();
        self.log.clear();
        self.log.push("Wizard avviato — attendo temperatura stabile...".to_owned());
    }

    pub fn cancel(&mut self) {
        self.phase        = WizardPhase::Idle;
        self.progress_pct = 0;
        self.log.push("Wizard annullato.".to_owned());
    }

    /// Aggiorna il wizard con la temperatura corrente.
    /// Ritorna Some(power_pct) se il wizard vuole impostare una potenza specifica,
    /// None se deve usare il controllo normale.
    pub fn tick(&mut self, tec_temp: f32) -> Option<u8> {
        self.current_tick += 1;
        self.temp_history.push_back(tec_temp);
        if self.temp_history.len() > 500 { self.temp_history.pop_front(); }

        match &self.phase.clone() {
            WizardPhase::WaitingStable => {
                self.progress_pct = 10;
                if self.temp_history.len() < 100 { return None; }
                // Calcola slope degli ultimi 5s (500 ticks a 100fps = 5s)
                let recent: Vec<f32> = self.temp_history.iter().rev().take(500).cloned().collect();
                let slope = linear_slope_per_sample(&recent).abs();
                // `tick()` gira a 2 Hz (500 ms per campione, gate di
                // `should_update`), NON a 100 Hz.
                //
                // Con il vecchio fattore 100 la soglia di stabilita' valeva
                // 0,001 °C/s: il solo rumore del sensore (sigma 0,05 °C
                // sulla regressione di 100 campioni) vale ~0,005 °C/campione,
                // quindi il wizard restava in "attendo temperatura stabile"
                // per sempre, anche a regime perfetto.
                let slope_per_sec = slope * Self::SAMPLE_HZ;

                if slope_per_sec < self.stable_threshold {
                    self.step_start_temp = tec_temp;
                    self.step_start_tick = self.current_tick;
                    self.phase = WizardPhase::ApplyingStep;
                    self.log.push(format!("Temperatura stabile a {:.1}°C — applico step 80%", tec_temp));
                    return Some(self.step_power_pct);
                }
                None
            }

            WizardPhase::ApplyingStep => {
                let elapsed = self.current_tick - self.step_start_tick;
                self.progress_pct = 20 + (elapsed * 60 / self.step_duration_ticks) as u8;

                if elapsed >= self.step_duration_ticks {
                    self.phase = WizardPhase::Analyzing;
                    self.log.push("Step completato — analisi in corso...".to_owned());
                    self.analyze();
                    return Some(50); // torna a potenza media
                }
                Some(self.step_power_pct)
            }

            WizardPhase::Analyzing => {
                // analyze() è sincrono, la fase viene aggiornata lì
                None
            }

            _ => None
        }
    }

    fn analyze(&mut self) {
        self.progress_pct = 90;

        // Raccoglie i sample dal momento dello step
        let step_start_idx = self.temp_history.len()
            .saturating_sub(self.step_duration_ticks as usize);
        let step_data: Vec<f32> = self.temp_history.iter()
            .skip(step_start_idx).cloned().collect();

        if step_data.len() < self.step_duration_ticks as usize {
            self.phase = WizardPhase::Failed("Dati insufficienti per l'analisi".to_owned());
            return;
        }

        let t0 = self.step_start_temp;
        let t_final = *step_data.iter().rev().take(50)
            .collect::<Vec<_>>().iter().copied()
            .reduce(|a, b| if a > b { a } else { b })
            .unwrap_or(&t0);

        let delta_t = (t_final - t0).abs();
        if delta_t < 0.5 {
            self.phase = WizardPhase::Failed(
                format!("Delta-T insufficiente ({:.2}°C) — sistema già stabile", delta_t)
            );
            return;
        }

        // Metodo Ziegler-Nichols process reaction:
        // K = ΔY / ΔU  (guadagno)
        // L = dead time (tempo prima che la temperatura inizi a cambiare)
        // T = time constant (63.2% del delta finale)

        let k_gain = delta_t / self.step_power_pct as f32; // °C per %
        let target_63 = t0 + delta_t * 0.632;
        let t63_idx = step_data.iter().position(|&v| (t0 > t_final && v <= target_63) || (t0 < t_final && v >= target_63))
            .unwrap_or(step_data.len() / 2);
        let dead_samples = step_data.iter().position(|&v| (t0 > t_final && v < t0 - 0.1) || (t0 < t_final && v > t0 + 0.1))
            .unwrap_or(20);

        // Converti da campioni a secondi.
        //
        // Stesso errore di scala del gate di stabilita': i campioni sono a
        // 2 Hz, non a 100 Hz. Con /100.0 la costante di tempo risultava 50
        // volte piu' piccola, e da I = 2*dead_time e D = 0.5*dead_time
        // seguivano valori quasi nulli: il wizard avrebbe proposto un
        // controller quasi solo-P, senza azione derivativa e senza
        // smorzamento, che su un loop criogenico oscilla.
        let hz = Self::SAMPLE_HZ;
        let time_const  = (t63_idx as f32 - dead_samples as f32).max(1.0) / hz;
        let dead_time   = dead_samples as f32 / hz;

        // Formule Z-N per PID (modificate per sistemi lenti/cryo)
        let p = 1.2 * time_const / (k_gain * dead_time.max(0.01));
        let i = 2.0 * dead_time;
        let d = 0.5 * dead_time;

        // Clamp a range ragionevoli
        let p = p.clamp(5.0, 300.0);
        let i = i.clamp(0.01, 10.0);
        let d = d.clamp(0.0, 5.0);

        self.log.push(format!("K={:.3} L={:.1}s T={:.1}s", k_gain, dead_time, time_const));
        self.log.push(format!("Risultato → P={:.2}  I={:.3}  D={:.3}", p, i, d));

        self.phase = WizardPhase::Done { p, i, d, k_gain, dead_time, time_const };
        self.progress_pct = 100;
    }

    pub fn is_running(&self) -> bool {
        !matches!(self.phase, WizardPhase::Idle | WizardPhase::Done {..} | WizardPhase::Failed(_))
    }
}

impl Default for PidWizard {
    fn default() -> Self { Self::new() }
}

fn linear_slope_per_sample(data: &[f32]) -> f32 {
    let n = data.len() as f32;
    if n < 2.0 { return 0.0; }
    let sum_x:  f32 = (0..data.len()).map(|i| i as f32).sum();
    let sum_y:  f32 = data.iter().sum();
    let sum_xy: f32 = data.iter().enumerate().map(|(i, y)| i as f32 * y).sum();
    let sum_x2: f32 = (0..data.len()).map(|i| (i as f32).powi(2)).sum();
    let denom = n * sum_x2 - sum_x * sum_x;
    if denom.abs() < f32::EPSILON { 0.0 } else { (n * sum_xy - sum_x * sum_y) / denom }
}
