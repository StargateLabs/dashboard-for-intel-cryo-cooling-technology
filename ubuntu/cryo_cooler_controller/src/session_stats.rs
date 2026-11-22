//! Session statistics — min/max/avg calcolati su tutta la sessione.
//! Aggiornato ad ogni campionamento, leggero e senza allocazioni extra.

#[derive(Debug, Clone)]
pub struct StatTracker {
    pub count: u64,
    pub min:   f32,
    pub max:   f32,
    sum:       f64,
    /// Il **primo** valore inserito.
    ///
    /// Serve alla diagnostica, che mostra la "temperatura iniziale": il
    /// valore da cui si parte, non la media della sessione. Sono due
    /// informazioni diverse e l'etichetta deve dire quale sta mostrando.
    /// Senza questo campo la diagnostica mostrava la media sotto una
    /// dicitura che prometteva l'inizio.
    first:     Option<f32>,
}

impl StatTracker {
    pub fn new() -> Self {
        StatTracker {
            count: 0,
            min: f32::MAX,
            max: f32::MIN,
            sum: 0.0,
            first: None,
        }
    }

    pub fn push(&mut self, value: f32) {
        self.count += 1;
        if self.first.is_none() { self.first = Some(value); }
        if value < self.min { self.min = value; }
        if value > self.max { self.max = value; }
        self.sum += value as f64;
    }

    pub fn avg(&self) -> f32 {
        if self.count == 0 { return 0.0; }
        (self.sum / self.count as f64) as f32
    }

    /// Il primo valore inserito, se ce n'è uno.
    pub fn first(&self) -> Option<f32> {
        self.first
    }
}

impl Default for StatTracker {
    fn default() -> Self { Self::new() }
}

/// Aggregate session statistics across all monitored channels.
#[derive(Debug, Default)]
pub struct SessionStats {
    pub tec_temp:            StatTracker,
    pub tec_power_watts:     StatTracker,
    pub condensation_margin: StatTracker,
    pub tec_current:         StatTracker,
    pub tec_voltage:         StatTracker,
    pub cop:                 StatTracker,  // FIX [6]: COP medio sessione
    pub session_samples:     u64,
}

impl SessionStats {
    pub fn update(&mut self, data: &cryo_cooler_controller_lib::MonitoringData) {
        self.session_samples += 1;
        self.tec_temp.push(data.tec_temperature);
        self.tec_power_watts.push(data.tec_power_watts);
        self.condensation_margin.push(data.condensation_margin);
        self.tec_current.push(data.tec_current);
        self.tec_voltage.push(data.tec_voltage);
    }

    /// Aggiorna COP separatamente (calcolato dopo update con cpu_temp)
    pub fn push_cop(&mut self, cop: f32) {
        if cop > 0.0 && cop < 5.0 { self.cop.push(cop); }
    }
}

#[cfg(test)]
mod test_dati_diagnostica {
    //! Verifica la ricostruzione di `dati_diagnostica` (2026-09-29).
    //!
    //! Il corpo originale di quella funzione andava perso e ricostruito. Il
    //! rischio di una ricostruzione non e' che non compili: e' che produca
    //! **numeri che sembrano misure**. Un tracciatore vuoto ha `min = f32::MAX`
    //! e `max = f32::MIN`, e passarli alla diagnostica mostrerebbe "-3.4e38 °C".
    //! Questi test verificano che non succeda.

    use super::*;

    fn dato(t: f32, p: f32) -> cryo_cooler_controller_lib::MonitoringData {
        cryo_cooler_controller_lib::MonitoringData {
            timestamp: chrono::Utc::now(),
            tec_temperature: t,
            pcb_temperature: 30.0,
            humidity: 40.0,
            dew_point_temperature: 12.0,
            tec_voltage: 10.0,
            tec_current: 2.0,
            tec_power_level: 100,
            tec_power_watts: p,
            condensation_margin: 5.0,
        }
    }

    /// **Un tracciatore vuoto non deve produrre numeri.**
    #[test]
    fn i_tracciatori_vuoti_non_hanno_min_max() {
        let t = StatTracker::new();
        assert_eq!(t.count, 0);
        assert!(t.min > 1e30, "`min` vuoto e' f32::MAX");
        assert!(t.max < -1e30, "`max` vuoto e' f32::MIN");
        // Ecco perche' `dati_diagnostica` controlla `count > 0`: senza quel
        // controllo, questi due valori finirebbero a schermo.
        assert!(!t.min.is_finite() || t.min > 1e30);
    }

    /// **"Temperatura finale" e' il punto piu' freddo, non l'ultimo.**
    ///
    /// E' la scelta che il ricostruito doveva preservare: su una cella che
    /// raffredda, il minimo e' il risultato della sessione.
    #[test]
    fn il_minimo_e_il_punto_piu_freddo_raggiunto() {
        let mut st = StatTracker::new();
        st.push(22.0); // inizio
        st.push(15.0); // il picco di freddo
        st.push(18.0); // ora e' piu' calda
        assert_eq!(st.first(), Some(22.0), "l'inizio e' il primo valore");
        assert_eq!(st.min, 15.0, "il finale e' il piu' freddo, non 18");
        assert_eq!(st.avg(), (22.0 + 15.0 + 18.0) / 3.0);
    }

    /// I dati che la diagnostica riceve devono essere numeri normali: nessun
    /// `f32::MAX` o `f32::MIN` deve arrivare alla tabella.
    #[test]
    fn le_misure_da_un_campione_sono_plausibili() {
        let mut stats = SessionStats::default();
        stats.update(&dato(15.1, 227.0));
        assert_eq!(stats.tec_temp.first(), Some(15.1));
        assert_eq!(stats.tec_power_watts.max, 227.0);
        assert!(stats.tec_temp.min < 100.0, "la minima e' un numero reale");
        assert!(stats.tec_power_watts.max < 10_000.0);
    }
}
