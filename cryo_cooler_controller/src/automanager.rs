//! Modalita' automatica: la dashboard gestisce il TEC da sola.
//!
//! **Perche' esiste.** L'auto-profiler gia' presente cambiava i campi
//! `inputs` in memoria e non mandava **niente** al controller: accendere
//! l'interruttore non cambiava nuno, ne' in UI ne' sull'hardware. Qui la
//! decisione e' separata dal comando: `AutoManager` decide il regime e dice
//! *quando* cambiare, `RunningState` applica e spinge all'hardware.
//!
//! **La sicurezza vince sempre.** Il regime automatico puo' solo chiedere:
//! il cap non supera quello consentito al modulo e l'offset passa dal
//! pavimento anticondensa. Se il regime dice "raffredda al massimo" ma il
//! controller e' a 70 °C, la guardia termica vince e scende. Nessuna
//! combinazione di regime puo' disattivare una protezione.
//!
//! **Perche' l'isteresi.** Senza, un carico che oscilla intorno alla soglia
//! farebbe commutare il regime ogni mezzo secondo: il TEC salirebbe e
//! scenderebbe di continuo, e ogni commutazione e' una scrittura seriale.
//! Un regime deve durare `isteresi` campioni prima di valere.

/// Regime di lavoro deciso dal gestore automatico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    /// Riposo: minima potenza, minimo rumore.
    Idle,
    /// Carico moderato.
    Leggero,
    /// Carico alto.
    Carico,
    /// Temperatura CPU oltre soglia: raffreddamento massimo.
    Picco,
    /// Nessun dato su cui decidere.
    ///
    /// Non e' un regime: e' l'assenza di una decisione. Va tenuta separata da
    /// `Idle` perche' `Idle` significa "posso affermare che non serve niente",
    /// mentre questa significa "non lo so".
    ///
    /// Restituire `Idle` quando mancano entrambi gli ingressi non era una scelta
    /// neutra: `Idle` e' il regime MENO aggressivo, quindi perdere un sensore
    /// **riduceva** il raffreddamento. E' la direzione sbagliata per un
    /// controller il cui compito e' tenere la CPU sotto soglia.
    ///
    /// Il firmware ha gia' `PID_INVALID` per questo caso: non e' una
    /// situazione inventata.
    Invalido,
}

impl Regime {
    /// Ordine di intervento: serve per non tornare a un regime piu' leggero
    /// mentre la situazione non e' ancora rientrata.
    pub fn livello(self) -> u8 {
        match self {
            Regime::Idle    => 0,
            Regime::Leggero => 1,
            Regime::Carico  => 2,
            Regime::Picco   => 3,
            // Nessun intervento: non e' un gradino della scala. Numerarlo 0
            // farebbe sembrare "il piu' leggero di tutti" invece che
            // "non lo so".
            Regime::Invalido => u8::MAX,
        }
    }

    pub fn etichetta(self) -> &'static str {
        match self {
            Regime::Idle    => "Idle",
            Regime::Leggero => "Leggero",
            Regime::Carico  => "Carico",
            Regime::Picco   => "Picco",
            Regime::Invalido => "sensori assenti",
        }
    }
}

/// Cosa deve guardare il gestore.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Considera il carico CPU (percentuale).
    pub usa_carico: bool,
    /// Considera la temperatura CPU.
    pub usa_temp: bool,
    /// Soglia di temperatura CPU per il regime Picco.
    pub soglia_temp: f32,
    /// Soglia di carico per il regime Carico (%).
    pub soglia_carico: f32,
    /// Soglia di carico per il regime Leggero (%).
    pub soglia_leggero: f32,
    /// Campioni consecutivi prima che un regime diventi valido.
    pub isteresi: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            usa_carico: true,
            usa_temp: true,
            // 70 °C: sopra i 65 il clock della CPU inizia a scendere da
            // solo, quindi e' il punto in cui il raffreddamento serve davvero.
            soglia_temp: 70.0,
            soglia_carico: 70.0,
            soglia_leggero: 30.0,
            // 2 Hz: 15 campioni = 7,5 s. Abbastanza per non oscillare, non
            // cosi' tanto da sembrare che non reagisca.
            isteresi: 15,
        }
    }
}

/// Gestore del regime automatico.
#[derive(Debug, Clone)]
pub struct AutoManager {
    pub cfg: Config,
    regime:      Regime,
    /// Regime appena valutato: deve durare `isteresi` campioni.
    candidato:   Regime,
    campioni:    u32,
    /// Il regime e' salito e non e' ancora rientrato: serve per l'isteresi
    /// sul rientro, che deve essere piu' lento della salita.
    in_risalita: bool,
}

impl AutoManager {
    pub fn new(cfg: Config) -> Self {
        let r = Regime::Idle;
        Self { cfg, regime: r, candidato: r, campioni: 0, in_risalita: false }
    }

    pub fn regime(&self) -> Regime {
        self.regime
    }

    /// Ricalcola il regime da zero. Chiamata quando l'utente cambia una soglia.
    pub fn reset(&mut self) {
        self.regime = Regime::Idle;
        self.candidato = Regime::Idle;
        self.campioni = 0;
        self.in_risalita = false;
    }

    /// Regime che corrisponde ai valori letti adesso.
    ///
    /// La temperatura ha la precedenza sul carico: se la CPU e' calda,
    /// sapere quanto carico c'e' non serve, serve raffreddare. E il regime
    /// `Picco` non si può superare in nessuna combinazione.
    pub fn classifica(&self, carico: Option<f32>, temp: Option<f32>) -> Regime {
        // Serve sapere se almeno un ingresso **abilitato** ha prodotto un
        // valore utilizzabile. Senza questo tracciamento non si distinguono
        // due casi che sembrano uguali ma non lo sono:
        //
        //  - il carico c'e' ed e' basso  -> `Idle` e' la risposta GIUSTA,
        //    l'abbiamo misurata;
        //  - nessun sensore ha parlato    -> qui non si sa niente, e la
        //    risposta non puo' essere `Idle`, perche' `Idle` e' il regime
        //    meno aggressivo e "non lo so" diventerebbe "raffredda meno".
        let mut ha_dato_un_dato = false;

        // Temperatura: prima di tutto. Se il sensore manca, si prosegue con
        // il carico: un regime senza temperatura e' meglio di nessun regime.
        //
        // **Solo valori plausibili.** Un sensore bloccato a 100 °C o che
        // sputa 1000 °C non e' "la CPU scotta": e' un sensore rotto, e un
        // sensore rotto non deve poter chiedere Picco. Fuori da
        // -50..125 °C il dato si tratta come assente.
        if self.cfg.usa_temp {
            if let Some(t) = temp {
                if t.is_finite() && (-50.0..=125.0).contains(&t) {
                    ha_dato_un_dato = true;
                    if t >= self.cfg.soglia_temp {
                        return Regime::Picco;
                    }
                }
            }
        }
        if self.cfg.usa_carico {
            if let Some(c) = carico {
                // Stessa regola per il carico: 0..100, il resto e' spazzatura.
                if c.is_finite() && (0.0..=100.0).contains(&c) {
                    ha_dato_un_dato = true;
                    if c >= self.cfg.soglia_carico {
                        return Regime::Carico;
                    }
                    if c >= self.cfg.soglia_leggero {
                        return Regime::Leggero;
                    }
                }
            }
        }

        if ha_dato_un_dato {
            // C'è un dato, e quel dato dice "basta cosi'".
            return Regime::Idle;
        }

        // Nessun ingresso abilitato ha prodotto niente. NON si sceglie Idle:
        // si dice che non si sa, e chi chiama mantiene l'ultimo regime
        // valido invece di abbassare la potenza.
        Regime::Invalido
    }

    /// Registra un campione. Ritorna il nuovo regime **solo quando cambia**,
    /// cosi' chi chiama scrive all'hardware una volta per transizione e non a
    /// ogni campione.
    ///
    /// `carico` e `temp` sono `None` quando il sensore non e' disponibile:
    /// in quel caso non si inventa un valore, si prosegue con quello che c'e'.
    pub fn push(&mut self, carico: Option<f32>, temp: Option<f32>) -> Option<Regime> {
        let valutato = self.classifica(carico, temp);
        let in_salita = valutato.livello() > self.regime.livello();

        // Campioni da aspettare: il doppio quando si rientra, cosi' la
        // discesa e' piu' lenta della salita e non si oscilla.
        let richiesti = if !in_salita && self.in_risalita {
            self.cfg.isteresi * 2
        } else {
            self.cfg.isteresi
        };

        // Niente da fare: siamo gia' dove dovremmo.azzera il conto, cosi'
        // un regime stabile non accumula isteresi per il prossimo cambio.
        if valutato == self.regime {
            self.candidato = self.regime;
            self.campioni = 0;
            self.in_risalita = false;
            return None;
        }

        // Regime diverso da quello in valutazione: si riparte da zero. Il
        // PRIMO campione di un regime nuovo conta come 1, non come 0.
        //
        // Prima questo controllo veniva DOPO l'attesa, e insieme al conto
        // faceva la cosa due volte: servivano 7 campioni per commutare invece
        // di 3, e l'isteresi risultava effectively triplicata.
        if valutato != self.candidato {
            self.candidato = valutato;
            self.campioni = 1;
            return None;
        }

        self.campioni += 1;
        if self.campioni < richiesti {
            return None;
        }

        // Confermato: il regime cambia davvero, e si scrive all'hardware una
        // volta per transizione.
        self.regime = valutato;
        self.campioni = 0;
        self.in_risalita = in_salita;
        Some(valutato)
    }
}

/// Decide se applicare un regime all'hardware in questo giro.
///
/// `esito` e' il risultato di `push`: `Some` solo quando il regime e'
/// appena cambiato. `allineare` e' vero subito dopo che l'utente accende
/// l'automatico: serve ad allineare l'hardware al regime corrente anche se
/// non c'e' stata una transizione, altrimenti accendere non cambia niente
/// di visibile e sembra rotto.
///
/// `dati_validi` e' falso quando non c'e' nessun sensore: in quel caso non
/// si applica mai, nemmeno per allineare. Applicare `Idle` senza dati
/// abbasserebbe il raffreddamento proprio quando non si sa cosa serva.
pub fn regime_da_applicare(
    auto_on: bool,
    allineare: bool,
    esito: Option<Regime>,
    corrente: Regime,
    dati_validi: bool,
) -> Option<Regime> {
    if !auto_on || !dati_validi {
        return None;
    }
    if let Some(r) = esito {
        return Some(r);
    }
    if allineare {
        return Some(corrente);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> AutoManager {
        AutoManager::new(Config { isteresi: 3, ..Default::default() })
    }

    /// Il caso che l'utente ha segnalato: accendere l'interruttore non deve
    /// cambiare niente, quindi il regime DEVE cambiare quando il carico
    /// sale, non prima e non dopo.
    #[test]
    fn commuta_solo_al_cambio() {
        let mut m = mgr();
        // 3 campioni a zero: resta Idle, e non restituisce nulla.
        for _ in 0..5 {
            assert_eq!(m.push(Some(5.0), Some(40.0)), None);
        }
        assert_eq!(m.regime(), Regime::Idle);

        // Carico alto: al terzo campione il regime deve cambiare, ed essere
        // l'unico ritorno non nullo.
        let mut cambi = 0;
        for _ in 0..3 {
            if m.push(Some(85.0), Some(45.0)).is_some() {
                cambi += 1;
            }
        }
        assert_eq!(cambi, 1, "il regime deve commutare una volta sola");
        assert_eq!(m.regime(), Regime::Carico);

        // Poi piu' campioni allo stesso regime: piu' nessun ritorno.
        for _ in 0..10 {
            assert_eq!(m.push(Some(85.0), Some(45.0)), None);
        }
    }

    /// La temperatura deve vincere sul carico: se la CPU scotta, serve
    /// raffreddare comunque, anche a carico basso.
    #[test]
    fn temperatura_vince_sul_carico() {
        let m = mgr();
        assert_eq!(m.classifica(Some(5.0), Some(85.0)), Regime::Picco);
        assert_eq!(m.classifica(Some(5.0), Some(40.0)), Regime::Idle);
        // Il regime massimo e' Picco: nessuna combinazione lo supera.
        assert_eq!(m.classifica(Some(100.0), Some(100.0)), Regime::Picco);
    }

    /// Sensore assente: non si inventa, si prosegue con l'altro. Un valore
    /// `None` non deve mai far cadere il regime a Idle di colpo.
    ///
    /// **L'ultima riga di questo test e' stata cambiata, e va spiegato
    /// perche'.** Prima asseriva che `classifica(None, None)` valesse `Idle`,
    /// cioe' che l'assenza di ogni sensore equivalesse a "il regime meno
    /// aggressivo". Non era una scelta, era il difetto: `Idle` significa
    /// "ho misurato e basta cosi'", mentre qui non e' stato misurato
    /// niente. La conseguenza era che perdere un sensore **costava potenza
    /// di raffreddamento**, la direzione opposta a quella giusta.
    ///
    /// Sulla V1 la temperatura CPU non e' quasi mai disponibile e l'unico
    /// segnale e' il carico Windows: se `GetSystemTimes` falliva una volta,
    /// il regime scivolava a Idle e lo scriveva in hardware.
    #[test]
    fn sensore_assente_non_inventa() {
        let m = mgr();
        assert_eq!(m.classifica(None, Some(85.0)), Regime::Picco);
        assert_eq!(m.classifica(Some(85.0), None), Regime::Carico);
        // Nessun sensore: si dichiara che non si sa, e `RunningState` non
        // tocca l'hardware.
        assert_eq!(m.classifica(None, None), Regime::Invalido);
        // NaN non e' un valore: non deve valere come "massimo", e non deve
        // nemmeno valere come "bassa", per lo stesso motivo.
        assert_eq!(m.classifica(Some(f32::NAN), Some(f32::NAN)), Regime::Invalido);
    }

    /// Un carico basso **misurato** resta Idle: e' una lettura, non
    /// un'assenza. E' il caso che distingue `Idle` legittimo da
    /// `Invalido`, ed e' la ragione per cui `classifica` deve tracciare se
    /// un ingresso ha prodotto un dato, invece di restituire `Invalido` a
    /// fine percorso.
    #[test]
    fn carico_basso_misurato_e_idle() {
        let m = mgr();
        assert_eq!(m.classifica(Some(3.0), Some(40.0)), Regime::Idle);
    }

    /// Il rientro e' piu' lento della salita: evita l'oscillazione.
    #[test]
    fn rientro_piu_lento_della_salita() {
        let mut m = mgr();
        // Salita a Carico: 3 campioni.
        for _ in 0..3 { m.push(Some(85.0), Some(45.0)); }
        assert_eq!(m.regime(), Regime::Carico);

        // Rientro immediato a Idle: NON deve commutare subito.
        assert_eq!(m.push(Some(5.0), Some(40.0)), None);
        assert_eq!(m.regime(), Regime::Carico, "non deve rientrare subito");
        // Serve il doppio dell'isteresi.
        for _ in 0..3 { m.push(Some(5.0), Some(40.0)); }
        assert_eq!(m.regime(), Regime::Carico, "ancora troppo presto");
        for _ in 0..4 { m.push(Some(5.0), Some(40.0)); }
        assert_eq!(m.regime(), Regime::Idle, "ora deve essere rientrato");
    }

    /// Disattivando un sensore il regime deve cambiare di conseguenza.
    #[test]
    fn disattivare_un_sensore_cambia_il_regime() {
        let m = AutoManager::new(Config { usa_temp: false, isteresi: 3, ..Default::default() });
        // Con la temperatura spenta, 85 °C non devono piu' fare Picco.
        assert_eq!(m.classifica(Some(5.0), Some(85.0)), Regime::Idle);
    }

    /// **Un sensore rotto non chiede Picco.** 1000 °C non esistono su una
    /// CPU: e' un sensore bloccato o spazzatura, e va trattato come
    /// assente invece di mandare la cella a palla.
    #[test]
    fn temperatura_assurda_non_fa_picco() {
        let m = mgr();
        assert_eq!(m.classifica(Some(5.0), Some(1000.0)), Regime::Idle);
        assert_eq!(m.classifica(None, Some(1000.0)), Regime::Invalido);
    }

    /// Stessa regola per il carico: fuori 0..100 e' spazzatura, non un
    /// carico alto.
    #[test]
    fn carico_assurdo_non_fa_carico() {
        let m = mgr();
        assert_eq!(m.classifica(Some(500.0), None), Regime::Invalido);
        assert_eq!(m.classifica(Some(-20.0), None), Regime::Invalido);
    }

    /// **Senza rugiada valida non si va in aggressivo.** E' il guasto
    /// segnalato: il pavimento anticondensa vale -1000 quando manca il
    /// dato, e un setpoint di -18 passava il clamp. Picco e Carico
    /// scendono a Leggero, gli altri restano.
    #[test]
    fn senza_pavimento_niente_regimi_aggressivi() {
        assert_eq!(regime_sicuro(Regime::Picco, false), Regime::Leggero);
        assert_eq!(regime_sicuro(Regime::Carico, false), Regime::Leggero);
        assert_eq!(regime_sicuro(Regime::Leggero, false), Regime::Leggero);
        assert_eq!(regime_sicuro(Regime::Idle, false), Regime::Idle);
    }

    /// Con il pavimento valido non si tocca niente: la protezione c'e' e
    /// il regime resta quello deciso.
    #[test]
    fn con_pavimento_il_regime_resta() {
        for r in [Regime::Idle, Regime::Leggero, Regime::Carico, Regime::Picco] {
            assert_eq!(regime_sicuro(r, true), r);
        }
    }

    /// **Accendere l'automatico allinea subito l'hardware.** Senza questo,
    /// se il regime corrente e' gia' quello giusto non c'e' transizione,
    /// `push` restituisce `None` e non si scrive niente: l'interruttore
    /// sembra rotto.
    #[test]
    fn accensione_allinea_anche_senza_transizione() {
        assert_eq!(
            regime_da_applicare(true, true, None, Regime::Idle, true),
            Some(Regime::Idle)
        );
    }

    /// La transizione vince sempre sull'allineamento: se il regime e'
    /// appena cambiato, si applica quello nuovo, non quello vecchio.
    #[test]
    fn la_transizione_vince_sull_allineamento() {
        assert_eq!(
            regime_da_applicare(true, true, Some(Regime::Carico), Regime::Idle, true),
            Some(Regime::Carico)
        );
    }

    /// Senza dati non si applica mai, nemmeno per allineare: applicare
    /// `Idle` al buio abbasserebbe il raffreddamento quando non si sa cosa
    /// serva.
    #[test]
    fn senza_dati_non_si_applica_mai() {
        assert_eq!(
            regime_da_applicare(true, true, None, Regime::Idle, false),
            None
        );
        assert_eq!(
            regime_da_applicare(true, false, Some(Regime::Carico), Regime::Idle, false),
            None
        );
    }

    /// A interruttore spento non si applica niente, nemmeno se il gestore
    /// avesse un esito pendente.
    #[test]
    fn spento_non_applica() {
        assert_eq!(
            regime_da_applicare(false, true, Some(Regime::Carico), Regime::Idle, true),
            None
        );
    }
}

/// Regime effettivamente applicabile, date le protezioni disponibili.
///
/// **Senza punto di rugiada valido non si va in aggressivo.** Il pavimento
/// anticondensa vale `-1000` quando rugiada o ambiente non sono validi: in
/// quel caso un setpoint di `-18` passerebbe il clamp e la cella andrebbe a
/// palla senza nessuna regolazione. E' il guasto segnalato: l'automatico
/// sembrava impazzito, invece stava solo eseguendo un ordine che nessuno
/// aveva controllato.
///
/// Con `pavimento_valido == false`, Picco e Carico scendono a Leggero: si
/// continua a raffreddare, ma senza spingere al massimo al buio.
pub fn regime_sicuro(richiesto: Regime, pavimento_valido: bool) -> Regime {
    if pavimento_valido {
        return richiesto;
    }
    match richiesto {
        Regime::Picco | Regime::Carico => Regime::Leggero,
        altro => altro,
    }
}
