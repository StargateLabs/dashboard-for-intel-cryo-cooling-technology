//! Rilevamento della pompa ferma dalla firma termica, senza sensore RPM.
//!
//! **Il problema.** La protezione pompa legge l'RPM dai sensori HWiNFO. Se
//! HWiNFO non e' installato l'RPM non c'e', `pump_readable` resta `false` e le
//! protezioni legate alla pompa restano sospese: la potenza non viene mai
//! dimezzata per un loop fermo. Su una macchina senza HWiNFO quel ramo di
//! sicurezza semplicemente non esiste.
//!
//! **La soluzione non e' inventare un RPM, e' usare la fisica.** Se il loop
//! e' fermo il calore non viene asportato, e questo si vede: con potenza
//! alta e la piastra che **sale**, il TEC sta lavorando senza circolazione.
//! Non serve misurare i giri per accorgersene, basta guardare se scaldare.
//!
//! **Perche' non puo' dare falsi positivi in condizioni normali.**
//!  - Serve potenza alta: a potenza bassa il TEC non scalderebbe comunque.
//!  - Serve che la piastra **salga**: con il loop funzionante e potenza alta la
//!    temperatura scende o sta ferma.
//!  - Serve che il TEC stia provando a raffreddare, non a scaldare.
//!  - Serve persistenza: una finestra di campioni, non un singolo picco.
//!
//! Il giudizio lo dà solo `poll()`, e solo se l'RPM non è disponibile: se il
//! sensore c'è, l'RPM vale più di qualsiasi inferenza.

/// Configurazione della inferenza.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Potenza applicata sotto la quale non si conclude niente.
    pub potenza_minima: u8,
    /// Variazione minima di temperatura sulla finestra, in °C, da considerare
    /// "in salita". Sotto questa il rumore termico non conta.
    pub salita_minima: f32,
    /// Campioni consecutivi prima di dichiarare il guasto.
    pub campioni: u32,
    /// Variazione minima **al minuto** che vale come guasto.
    ///
    /// Serve perche' il criterio breve non vede i fermi lenti: 1,5 °C in 6 s
    /// chiedono 16 °C/min, e un loop secco su un carico pesante sale molto
    /// piu' piano. La firma risultava cieca proprio dove serve di piu'.
    pub salita_lenta:   f32,
    /// Campioni della finestra lunga.
    pub campioni_lenti: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // 60%: sotto, il TEC non ha abbastanza carico da scaldare nulla.
            potenza_minima: 60,
            // 1,5 °C sulla finestra: il rumore di un sensore e' ~0,2 °C, quindi
            // 1,5 e' rumore piu' rumore senza essere un trend reale.
            salita_minima: 1.5,
            // 12 campioni a 2 Hz = 6 s. La firma termica non e' istantanea.
            campioni: 12,
            // 0,3 °C/min: ben sopra il rumore di un sensore (0,2 °C) e
            // ben sotto la salita che il TEC provoca da solo, che sta
            // provando a raffreddare e non ha motivo di scaldare.
            salita_lenta: 0.3,
            campioni_lenti: 60,
        }
    }
}

/// Osserva la piastra e conclude sulla pompa.
#[derive(Debug, Clone)]
pub struct PompaWatch {
    cfg:    Config,
    /// Temperature campionate nella finestra corrente.
    finestra: Vec<f32>,
    /// Numero di volte che il criterio e' stato soddisfatto di fila.
    consecutivi: u32,
    /// Ultimo giudizio, per non ripeterlo a ogni campione.
    guasto: bool,
    /// Il TEC e' stato acceso almeno una volta: sotto, non si conclude niente.
    mai_acceso: bool,
}

impl PompaWatch {
    pub fn new() -> Self {
        Self::with_config(Config::default())
    }

    pub fn with_config(cfg: Config) -> Self {
        Self { cfg, finestra: Vec::new(), consecutivi: 0, guasto: false, mai_acceso: false }
    }

    pub fn guasto(&self) -> bool {
        self.guasto
    }

    /// Azzera tutto. Va chiamata quando l'utente cambia il setpoint o
    /// riaccende il TEC: dopo un cambio la temperatura sale comunque, e
    /// quella salita non e' colpa della pompa.
    pub fn reset(&mut self) {
        self.finestra.clear();
        self.consecutivi = 0;
        self.guasto = false;
        self.mai_acceso = false;
    }

    /// Registra un campione. Ritorna `true` solo al passaggio a guasto.
    ///
    /// `rpm_noto` e' l'RPM letto da un sensore. Se c'e', la inferenza non viene
    /// nemmeno calcolata: un dato misurato vale piu' di uno dedotto.
    pub fn poll(&mut self, temp: f32, potenza: u8, raffredda_attivo: bool, rpm_noto: Option<f32>) -> bool {
        let era_guasto = self.guasto;

        // Con l'RPMAvailable non si deduce niente: si usa il numero.
        if let Some(rpm) = rpm_noto {
            self.mai_acceso = true;
            self.consecutivi = 0;
            self.finestra.clear();
            self.guasto = rpm < 200.0;
            return !era_guasto && self.guasto;
        }

        if !temp.is_finite() {
            return false;
        }
        if potenza > 0 {
            self.mai_acceso = true;
        }

        // Finestra troppo corta per trarne una conclusione.
        self.finestra.push(temp);
        let n = self.cfg.campioni as usize;
        if self.finestra.len() > n {
            self.finestra.remove(0);
        }
        if self.finestra.len() < n || !self.mai_acceso {
            return false;
        }

        let salita = self.finestra[n - 1] - self.finestra[0];
        let sospetto = potenza >= self.cfg.potenza_minima
            && raffredda_attivo
            && salita >= self.cfg.salita_minima;

        // Criterio lento, sulla stessa finestra ma con orizzonte piu'
        // lungo. Serve quando il guasto non e' rapido: il solo criterio
        // breve richiede 16 °C/min, che un carico ben isolato non raggiunge
        // mai quando il loop si secca.
        let lento = self.finestra.len() as f32 >= self.cfg.campioni_lenti as f32
            && self.finestra.len() >= 2
            && potenza >= self.cfg.potenza_minima
            && raffredda_attivo;
        let sospetto_lento = lento
            && (self.finestra[0] - self.finestra[self.finestra.len() - 1]) * 2.0
                >= self.cfg.salita_lenta;

        if sospetto || sospetto_lento {
            self.consecutivi += 1;
        } else {
            self.consecutivi = 0;
        }

        // Il guasto si dichiara quando il criterio e' stato vero per tutta la
        // finestra, e si rientra al primo campione buono: un guasto che si
        // auto-guarisce non deve tenere la potenza dimezzata per sempre.
        let nuovo = self.consecutivi >= self.cfg.campioni;
        if nuovo != era_guasto {
            self.guasto = nuovo;
            return nuovo;
        }
        self.guasto = nuovo;
        false
    }
}

impl Default for PompaWatch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(short: bool) -> PompaWatch {
        let base = Config::default();
        PompaWatch::with_config(if short {
            Config { campioni: 4, ..base }
        } else {
            base
        })
    }

    /// Il caso normale: potenza alta ma la piastra SCENDE, loop che funziona.
    /// Non deve mai dichiarare guasto.
    #[test]
    fn loop_funzionante_non_allarma() {
        let mut p = m(true);
        for i in 0..40 {
            let t = 20.0 - i as f32 * 0.2; // scende
            assert!(!p.poll(t, 90, true, None), "falso allarme al campione {i}");
        }
        assert!(!p.guasto());
    }

    /// Il caso da intercettare: potenza alta e la piastra SALE. Il loop non
    /// asporta calore e il guasto deve essere dichiarato.
    #[test]
    fn loop_fermo_allarma() {
        let mut p = m(true);
        for i in 0..40 {
            // 0,5 °C a passo: sulla finestra di 4 campioni fa 1,5 °C, che e'
            // esattamente la soglia. Con 0,2 la salita sarebbe 0,6 e il
            // criterio non potrebbe mai valere.
            let t = 20.0 + i as f32 * 0.5;
            p.poll(t, 90, true, None);
        }
        assert!(p.guasto(), "il guasto non e' stato dichiarato");
    }

    /// La salita deve essere vera, non rumore. 0,05 °C per campione non e' un
    /// trend: non deve allarmare.
    #[test]
    fn rumore_non_allarma() {
        let mut p = m(true);
        for i in 0..40 {
            let t = 20.0 + (i as f32 * 0.05);
            p.poll(t, 90, true, None);
        }
        assert!(!p.guasto(), "il rumore non deve fare scattare la protezione");
    }

    /// Potenza bassa: il TEC non scalderebbe nulla, quindi salire non dice niente
    /// sulla pompa. Non deve allarmare.
    #[test]
    fn potenza_bassa_non_allarma() {
        let mut p = m(true);
        for i in 0..40 {
            p.poll(20.0 + i as f32 * 0.2, 20, true, None);
        }
        assert!(!p.guasto(), "a potenza bassa non si giudica la pompa");
    }

    /// Se il TEC sta riscaldando invece di raffreddare non c'e' niente da
    /// dedurre: non e' un guasto del loop.
    #[test]
    fn non_sta_raffreddando_non_allarma() {
        let mut p = m(true);
        for i in 0..40 {
            p.poll(20.0 + i as f32 * 0.5, 90, false, None);
        }
        assert!(!p.guasto());
    }

    /// Con l'RPM disponibile vale il dato reale, non l'inferenza: qui
    /// l'inferenza direbbe "guasto" (la temperatura sale) ma la pompa gira.
    #[test]
    fn rpm_disponibile_vince_sull_inferenza() {
        let mut p = m(true);
        for i in 0..40 {
            p.poll(20.0 + i as f32 * 0.5, 90, true, Some(1800.0));
        }
        assert!(!p.guasto(), "l'RPM reale deve avere la precedenza");

        // Con l'RPM basso invece deve allarmare, anche se la temperatura
        // scende: il dato reale dice che la pompa e' ferma.
        let mut q = m(true);
        for i in 0..40 {
            q.poll(20.0 - i as f32 * 0.2, 90, true, Some(50.0));
        }
        assert!(q.guasto(), "RPM basso = pompa ferma, con temperature che scendono");
    }

    /// Il guasto non deve restare agganciato: dopo che la piastra torna a
    /// scendere, la protezione si rientra da sola.
    #[test]
    fn il_guasto_si_rientra() {
        let mut p = m(true);
        for i in 0..40 { p.poll(20.0 + i as f32 * 0.5, 90, true, None); }
        assert!(p.guasto());
        for _ in 0..10 { p.poll(10.0, 90, true, None); }
        assert!(!p.guasto(), "il guasto deve rientrare quando la situazione torna");
    }

    /// Dopo un reset non deve restare memoria del guasto: l'utente ha cambiato
    /// qualcosa e la salita successiva non e' colpa della pompa.
    #[test]
    fn reset_cancella_il_guasto() {
        let mut p = m(true);
        for i in 0..40 { p.poll(20.0 + i as f32 * 0.5, 90, true, None); }
        assert!(p.guasto());
        p.reset();
        assert!(!p.guasto(), "il reset deve azzerare il giudizio");

        // Con temperature che scendono il reset tiene: la finestra e' vuota e
        // non si deve re-inferire un guasto che non c'e' piu'.
        for i in 0..40 { p.poll(10.0 - i as f32 * 0.5, 90, true, None); }
        assert!(!p.guasto(), "il reset deve azzerare anche la finestra");

        // Se pero' il loop e' ANCORA fermo dopo il reset, il guasto deve
        // tornare: azzerare la memoria non significa ignorare il presente.
        for i in 0..40 { p.poll(20.0 + i as f32 * 0.5, 90, true, None); }
        assert!(p.guasto(), "un guasto ancora presente deve essere rilevato di nuovo");
    }
}

#[cfg(test)]
mod test_calo_lento {
    use super::*;

    /// Il caso che il test esistente definiva come corretto senza esserlo.
    ///
    /// Con la soglia di 1,5 °C in 6 s servono 16 °C/min. Un fermo pompa su
    /// un carico con molta massa termica sale molto piu' lentamente di
    /// cosi': la firma era cieco proprio dove la protezione serve di piu',
    /// e il test precedente asseriva che una salita sotto soglia non deve
    /// mai allarmare, fissando il difetto.
    #[test]
    fn rileva_salita_lenta_su_carico_pesante() {
        let mut p = PompaWatch::with_config(Config {
            potenza_minima: 60,
            salita_minima: 1.5,
            campioni: 12,
            salita_lenta: 0.3,
            campioni_lenti: 60,
            ..Default::default()
        });
        p.reset();
        // 60 campioni a +0,2 °C: in 30 s sale di 12 °C mentre il TEC prova a
        // raffreddare. Nessun tratto di 12 campioni arriva a 1,5 °C, ma il
        // trend e' inequivocabile e il solo criterio breve non lo vede.
        for i in 0..60 {
            p.poll(i as f32 * 0.2, 100, true, None);
        }
        assert!(p.guasto(), "salita lenta non rilevata");
    }

    /// Il controtesto: con il loop sano la piastra scende, e nessuno dei due
    /// criteri deve scattare.
    #[test]
    fn non_allarma_con_loop_sano() {
        let mut p = PompaWatch::with_config(Config {
            salita_lenta: 0.3,
            campioni_lenti: 60,
            ..Default::default()
        });
        p.reset();
        for i in 0..80 {
            p.poll(40.0 - i as f32 * 0.1, 100, true, None);
        }
        assert!(!p.guasto(), "falso positivo con loop in salute");
    }
}
