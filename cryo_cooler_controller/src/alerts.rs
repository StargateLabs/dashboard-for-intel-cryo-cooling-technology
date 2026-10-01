//! Alert system: soglie configurabili, notifiche Windows toast, audio sintetizzato.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AlertChannel {
    TecTemp,
    CpuTemp,
    DewPointMargin,
    PowerWatts,
    HumidityPct,
    /// FEAT #5: RPM pompa — trigger quando la pompa del loop scende sotto soglia.
    /// Abbassa automaticamente la potenza TEC al 50% per prevenire surriscaldamento
    /// prima che il loop smetta completamente di circolare.
    PumpRpm,
}

impl std::fmt::Display for AlertChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertChannel::TecTemp         => write!(f, "Temp. TEC"),
            AlertChannel::CpuTemp         => write!(f, "Temp. CPU"),
            AlertChannel::DewPointMargin  => write!(f, "Margine Condensa"),
            AlertChannel::PowerWatts      => write!(f, "Potenza TEC"),
            AlertChannel::HumidityPct     => write!(f, "Umidità"),
            AlertChannel::PumpRpm         => write!(f, "RPM Pompa"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertCondition {
    Above(f32),
    Below(f32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub channel:     AlertChannel,
    pub condition:   AlertCondition,
    pub label:       String,
    pub enabled:     bool,
    pub cooldown_s:  u32,
}

impl AlertRule {
    pub fn default_rules() -> Vec<AlertRule> {
        vec![
            AlertRule {
                channel:    AlertChannel::DewPointMargin,
                condition:  AlertCondition::Below(0.0),
                label:      "⚠ CONDENSA ATTIVA — TEC sotto punto di rugiada!".to_owned(),
                enabled:    true,
                cooldown_s: 600,
            },
            AlertRule {
                // **Scarto, non temperatura assoluta.** Il valore di questo
                // canale e' `piastra - setpoint`: 6 significa "la piastra e'
                // 6 °C piu' calda di quanto glie' ho chiesto".
                //
                // Con una soglia assoluta di 20 °C la regola era sempre
                // vera: il TEC qui lavora a 26-30 °C perche' il pavimento
                // anticondensa glielo impedisce, e l'utente riceveva
                // "raffreddamento anomalo" ogni 10 minuti con tutto
                // regolare. Una soglia assoluta non puo' funzionare: quanto
                // freddo si puo' fare lo decide l'umidita' della stanza, e
                // quel numero cambia di continuo.
                channel:    AlertChannel::TecTemp,
                condition:  AlertCondition::Above(6.0),
                label:      "⚠ TEC calda — raffreddamento anomalo".to_owned(),
                enabled:    true,
                cooldown_s: 600,
            },
            AlertRule {
                channel:    AlertChannel::PowerWatts,
                condition:  AlertCondition::Above(200.0),
                // **Nessuna soglia nell'etichetta.** La soglia la scrive
                // `etichetta_con_valore`, che la prende dalla condizione.
                // Averla anche qui dentro faceva stampare due volte lo stesso
                // numero: "Potenza TEC > 200W > 200W (94W)".
                label:      "Potenza TEC".to_owned(),
                enabled:    false,
                cooldown_s: 600,
            },
            AlertRule {
                channel:    AlertChannel::HumidityPct,
                condition:  AlertCondition::Above(80.0),
                // Come la potenza: la soglia la stampa la condizione, non
                // l'etichetta. "Umidità > 80%" compariva due volte.
                label:      "Umidità — rischio condensa elevato".to_owned(),
                enabled:    false,
                cooldown_s: 1800,
            },
            AlertRule {
                // FEAT #5: pompa ferma o quasi — azione di emergenza
                channel:    AlertChannel::PumpRpm,
                condition:  AlertCondition::Below(200.0),
                // Il "50%" qui dentro era un'altra soglia scritta a mano,
                // e non quella della condizione: se l'utente cambia la
                // soglia pompa, l'etichetta continuava a promettere il 50%.
                // Ora dice solo cosa succede, e il numero lo stampa la
                // condizione reale.
                label:      "🚨 POMPA FERMA — riduzione TEC automatica!".to_owned(),
                enabled:    true,
                cooldown_s: 300, // max 1 alert ogni 5 minuti
            },
        ]
    }

    /// Valore numerico della soglia, qualunque sia la direzione.
    ///
    /// Serve per far commutare la condizione (sopra/sotto) mantenendo il
    /// valore: senza questo, cambiare da "supera" a "scende sotto"
    /// azzererebbe la soglia.
    pub fn threshold(&self) -> f32 {
        match self.condition {
            AlertCondition::Above(t) | AlertCondition::Below(t) => t,
        }
    }

    pub fn triggered(&self, value: f32) -> bool {
        if !self.enabled { return false; }
        match self.condition {
            AlertCondition::Above(thresh) => value > thresh,
            AlertCondition::Below(thresh) => value < thresh,
        }
    }
}

/// Etichetta di un avviso scattato, con il valore che l'ha fatto scattare.
///
/// Ogni canale mostra la **sua** unita': prima tutti gli avvisi uscivano con
/// la temperatura di piastra davanti, quindi "Potenza TEC > 200W" mostrava
/// gradi invece di watt. Il valore e' quello valutato dalla regola, non una
/// rilettura: resta quello del momento dello scatto.
fn etichetta_con_valore(
    channel: &AlertChannel,
    value: f32,
    label: &str,
    condizione: &AlertCondition,
) -> String {
    let misura = match channel {
        AlertChannel::TecTemp => format!("{value:+.1}°C scarto"),
        AlertChannel::CpuTemp => format!("{value:.1}°C"),
        AlertChannel::DewPointMargin => format!("{value:+.1}°C margine"),
        AlertChannel::PowerWatts => format!("{value:.0}W"),
        AlertChannel::HumidityPct => format!("{value:.0}%"),
        AlertChannel::PumpRpm => format!("{value:.0} RPM"),
    };
    // La soglia viene dalla **condizione**, non dalla stringa dell'etichetta.
    //
    // Prima l'etichetta era fissa ("Potenza TEC > 200W") e l'utente poteva
    // cambiare la soglia dall'interfaccia: risultato un avviso che
    // dichiarava 200 W mentre il valore che l'aveva fatto scattare era
    // 113 W. Un avviso che si contraddice fa perdere fiducia nell'intera
    // pagina di allarmi, quindi la soglia la determina sempre la condizione.
    let soglia = match condizione {
        AlertCondition::Above(s) => format!("> {s}{}", unita_canale(channel)),
        AlertCondition::Below(s) => format!("< {s}{}", unita_canale(channel)),
    };
    format!("{label} {soglia} ({misura})")
}

/// L'unita' del canale, per la soglia. Ogni canale ha la sua: una soglia
/// in watt scritta senza unita' accanto a una temperatura e' ambigua.
fn unita_canale(channel: &AlertChannel) -> &'static str {
    match channel {
        AlertChannel::TecTemp | AlertChannel::DewPointMargin | AlertChannel::CpuTemp => "°C",
        AlertChannel::PowerWatts => "W",
        AlertChannel::HumidityPct => "%",
        AlertChannel::PumpRpm => " RPM",
    }
}

/// Migra le regole scritte con la vecchia semantica.
///
/// Le config precedenti valutavano `TecTemp` come temperatura assoluta
/// (default `Above(20.0)`); ora il canale vale lo scarto dal setpoint e il
/// default e' `Above(6.0)`. Una soglia >= 10 su questo canale non puo' che
/// essere un resto assoluto: valutarla sullo scarto la rende quasi muta
/// (o, con setpoint bassi, rumorosa). Viene riportata al default
/// preservando on/off, cooldown ed etichetta dell'utente.
pub fn migra_regole_obsolete(rules: Vec<AlertRule>) -> Vec<AlertRule> {
    let default_tec = AlertRule::default_rules()
        .into_iter()
        .find(|r| r.channel == AlertChannel::TecTemp);
    // Coppia canale/etichetta dei default, per la migrazione sotto.
    let default_power = AlertRule::default_rules()
        .into_iter()
        .find(|r| r.channel == AlertChannel::PowerWatts);
    let default_pump = AlertRule::default_rules()
        .into_iter()
        .find(|r| r.channel == AlertChannel::PumpRpm);
    let default_humidity = AlertRule::default_rules()
        .into_iter()
        .find(|r| r.channel == AlertChannel::HumidityPct);
    rules
        .into_iter()
        .map(|mut r| {
            if r.channel == AlertChannel::TecTemp && r.threshold() >= 10.0 {
                if let Some(d) = &default_tec {
                    r.condition = d.condition.clone();
                }
            }
            // **Etichette dei default che avevano la soglia scritta dentro.**
            //
            // L'utente le aveva nel suo file di configurazione, salvate da
            // una versione precedente. Senza questa migrazione continuerebbero a
            // stampare "Potenza TEC > 200W > 200W (94W)": la correzione
            // sui default non le tocca, perche' quelle regole sono gia' su
            // disco con la soglia dentro l'etichetta.
            //
            // Si sostituisce l'etichetta **solo se combacia esattamente** con
            // la vecchia: un'etichetta scritta a mano dall'utente e' sua e
            // non va toccata. La condizione, che e' il dato vero, resta
            // sempre quella che e' gia' su disco.
            for (canale, vecchia, nuovo) in [
                (AlertChannel::PowerWatts, "Potenza TEC > 200W", &default_power),
                (AlertChannel::PumpRpm, "🚨 POMPA FERMA — riduzione TEC automatica al 50%!", &default_pump),
                (AlertChannel::HumidityPct, "Umidità > 80% — rischio condensa elevato", &default_humidity),
            ] {
                if r.channel == canale && r.label == vecchia {
                    if let Some(d) = nuovo {
                        r.label = d.label.clone();
                    }
                }
            }
            r
        })
        .collect()
}

/// Testo di un avviso con il valore live, se disponibile.
///
/// Quando il valore manca (sensore assente al momento del rendering) mostra
/// solo l'etichetta: non si inventa niente, perche' un numero inventato
/// letto come misura e' peggio di un'etichetta senza numero.
pub fn formatta_avviso(
    channel: &AlertChannel,
    valore: Option<f32>,
    label: &str,
    condizione: Option<&AlertCondition>,
) -> String {
    match (valore, condizione) {
        (Some(v), Some(c)) => etichetta_con_valore(channel, v, label, c),
        // Senza soglia non si stima: l'etichetta nuda e' meglio di una
        // soglia inventata, che l'utente leggerebbe come misura.
        (Some(v), None) => format!("{label} ({})", solo_misura(channel, v)),
        (None, _) => label.to_owned(),
    }
}

/// Il valore formattato con l'unita' del suo canale, senza soglia.
fn solo_misura(channel: &AlertChannel, value: f32) -> String {
    match channel {
        AlertChannel::TecTemp => format!("{value:+.1}°C scarto"),
        AlertChannel::CpuTemp => format!("{value:.1}°C"),
        AlertChannel::DewPointMargin => format!("{value:+.1}°C margine"),
        AlertChannel::PowerWatts => format!("{value:.0}W"),
        AlertChannel::HumidityPct => format!("{value:.0}%"),
        AlertChannel::PumpRpm => format!("{value:.0} RPM"),
    }
}

/// Risultato di check(): avvisi scattati + flag azioni di emergenza.
///
/// `labels` trasporta `(canale, etichetta)`: il canale serve a rileggere il
/// valore **live** al momento del rendering. Prima viaggiava solo la
/// stringa con il valore congelato allo scatto, e il banner mostrava un
/// numero fermo che sembrava inventato.
#[derive(Debug, Default)]
pub struct AlertCheckResult {
    pub labels:          Vec<(AlertChannel, String)>,
    /// Se true, l'alert PumpRpm è scattato → il chiamante deve ridurre TEC al 50%
    pub pump_emergency:  bool,
}

#[derive(Debug, Default)]
pub struct AlertManager {
    rules:     Vec<AlertRule>,
    cooldowns: Vec<u32>, // tick rimanenti prima di poter rilanciare
}

impl AlertManager {
    pub fn new(rules: Vec<AlertRule>) -> Self {
        let n = rules.len();
        Self { rules, cooldowns: vec![0; n] }
    }

    /// Controlla tutte le regole. Chiamare ogni ~2s (4 tick @ 2 Hz).
    /// `pump_rpm`: RPM pompa primaria (0.0 se non disponibile/sconosciuta).
    /// Canali e come si giudicano.
    ///
    /// `TecTemp` e' l'unico che dipende dallo stato del TEC, e per un motivo
    /// che vale la pena tenere a mente prima di toccare la soglia: **una
    /// piastra calda non e' un guasto**. Se il TEC e' spento, o sta solo
    /// tenendo il setpoint, la piastra e' semplicemente a temperatura
    /// ambiente e deve stare li'. Quello che indica un problema e' una
    /// piastra calda **mentre il TEC sta raffreddando**: il freddo non arriva
    /// dove dovrebbe.
    ///
    /// Prima la regola veniva valutata sempre, e su una macchina a 22 °C
    /// ambiente diceva "raffreddamento anomalo" ogni 10 minuti con tutto
    /// regolare. La soglia di 20 °C resta giusta per il caso in cui il TEC
    /// sta lavorando: e' il contesto che mancava, non il numero.
    pub fn check(
        &mut self,
        tec_temp:    Option<f32>,
        tec_attivo:  bool,
        auto_regola: bool,
        cpu_temp:    Option<f32>,
        dew_margin:  f32,
        power_w:     f32,
        humidity:    f32,
        pump_rpm:    f32,
        ticks_per_s: f32,
    ) -> AlertCheckResult {
        let mut result = AlertCheckResult::default();
        for (i, rule) in self.rules.iter().enumerate() {
            // Il valore del canale, se esiste.
            //
            // `None` significa che la misurazione **non c'**: non che sia
            // zero. La distinzione e' tutta qui, ed e' la ragione per cui non
            // si usa piu' `unwrap_or(0.0)`: uno zero inventato, valutato da
            // una regola "sotto", fa scattare un allarme per un sensore che
            // semplicemente non c'e'. Una regola puo' valutare solo una
            // grandezza realmente misurata.
            let valore: Option<f32> = match rule.channel {
                AlertChannel::TecTemp        => tec_temp,
                AlertChannel::CpuTemp        => cpu_temp,
                AlertChannel::DewPointMargin => Some(dew_margin),
                AlertChannel::PowerWatts     => Some(power_w),
                AlertChannel::HumidityPct    => Some(humidity),
                AlertChannel::PumpRpm        => Some(pump_rpm),
            };

            // Una regola non applicabile non deve consumare il cooldown: la si
            // salta interamente, cosi' al ripristino del contesto (per esempio
            // riaccendendo il TEC, o tornando a leggere la CPU) il giudizio
            // parte pulito invece di trovarsi gia' scaduto.
            if valore.is_none() { continue; }
            if rule.channel == AlertChannel::TecTemp && !tec_attivo { continue; }
            // Lo scarto dal setpoint ha senso solo se un setpoint viene
            // inseguito: in manuale la potenza e' fissa e la piastra sta
            // dove la mette la fisica, quindi confrontarla col setpoint
            // produce un falso allarme permanente. Come per il TEC fermo,
            // saltare non consuma il cooldown.
            if rule.channel == AlertChannel::TecTemp && !auto_regola { continue; }
            if self.cooldowns[i] > 0 {
                self.cooldowns[i] -= 1;
                continue;
            }
            let value = valore.expect("appena verificato: nessun None arriva qui");
            if rule.triggered(value) {
                // Etichetta pulita: il valore live lo aggiunge il rendering
                // con `formatta_avviso`, cosi' il numero si aggiorna invece
                // di restare congelato al momento dello scatto.
                result.labels.push((rule.channel.clone(), rule.label.clone()));
                if rule.channel == AlertChannel::PumpRpm {
                    result.pump_emergency = true;
                }
                self.cooldowns[i] = (rule.cooldown_s as f32 * ticks_per_s) as u32;
            }
        }
        result
    }

    pub fn rules(&self) -> &[AlertRule] { &self.rules }
    pub fn rules_mut(&mut self) -> &mut Vec<AlertRule> { &mut self.rules }
}

/// Invia una notifica Windows balloon.
///
/// **Perche' l'interruttore globale e' un `AtomicBool` e non un semplice
/// controllo nel chiamante:** la notifica viene prodotta in un thread
/// separato, quindi un `if` valutato prima dello spawn non basta. Se
/// l'utente spegne le notifiche mentre un toast e' gia' in coda, quel
/// PowerShell partirebbe comunque. Il thread lo rilegge subito prima di
/// lanciare PowerShell, quindi l'hashtag arriva davvero.
static TOASTS_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// Imposta l'interruttore globale delle notifiche Windows.
/// Le protezioni termiche non sono interessate: cambia solo l'avviso.
pub fn set_toasts_enabled(enabled: bool) {
    TOASTS_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Le notifiche Windows sono abilitate?
///
/// Esposta per la UI: mostra lo stato reale del modulo, non quello salvato
/// in config. Se i due divergono, la UI mostrerebbe uno stato falso.
pub fn toasts_enabled() -> bool {
    TOASTS_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Invia una notifica Windows balloon — no-op su non-Windows o se disattivate.
///
/// Un singolo toast per evento: raggruppare le etichette in un messaggio
/// solo e' importante perche' ogni notifica costa un processo PowerShell
/// (~100 MB di working set). In precedenza un evento con 3 alert ne
/// lanciava 3, sprecando ~300 MB per notificare la stessa cosa.
#[cfg(target_os = "windows")]
pub fn send_toast(title: &str, message: &str) {
    use std::sync::atomic::Ordering;

    if !TOASTS_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    // Il thread richiede dati 'static: convertiamo subito in String owned.
    let title:   String = title.to_owned();
    let message: String = message.to_owned();
    std::thread::spawn(move || {
        // Rileggiamo il flag: l'utente puo' aver spento le notifiche
        // mentre questo thread era in coda.
        if !TOASTS_ENABLED.load(Ordering::Relaxed) {
            return;
        }
        let script = format!(
            r#"[void][System.Reflection.Assembly]::LoadWithPartialName('System.Windows.Forms');
$n=New-Object System.Windows.Forms.NotifyIcon;
$n.Icon=[System.Drawing.SystemIcons]::Information;
$n.Visible=$true;
$n.ShowBalloonTip(4000,'{title}','{message}','Info');
Start-Sleep -s 3;$n.Dispose()"#,
            title   = title.replace('\'', "").replace('\r', " ").replace('\n', " "),
            message = message.replace('\'', "").replace('\r', " ").replace('\n', " "),
        );
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("powershell")
            .args(["-WindowStyle", "Hidden", "-Command", &script])
            // CREATE_NO_WINDOW: senza questo si apre una finestra di console
            // che lampeggia a ogni notifica.
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    });
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(not(target_os = "windows"))]
pub fn send_toast(_title: &str, _message: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una piastra calda **non** e' un guasto: se il TEC non sta
    /// raffreddando, la soglia viene raggiunta senza che sia successo
    /// niente. Era la causa dell'allarme a raffica che l'utente ha
    /// segnalato con "le temperature sono sotto controllo".
    #[test]
    fn tec_calda_non_scatta_senza_tec_attivo() {
        let mut m = AlertManager::new(vec![AlertRule {
            channel:    AlertChannel::TecTemp,
            condition:  AlertCondition::Above(20.0),
            label:      "tec calda".to_owned(),
            enabled:    true,
            cooldown_s: 600,
        }]);

        // 24 °C: sopra la soglia, ma il TEC e' fermo -> niente allarme.
        let r = m.check(Some(24.0), false, true, None, 10.0, 100.0, 50.0, 3000.0, 0.5);
        assert!(
            r.labels.is_empty(),
            "piastra calda con TEC fermo non e' un guasto, ha scattato: {:?}",
            r.labels
        );

        // Stessa temperatura, TEC attivo: adesso il guasto e' reale.
        let r = m.check(Some(24.0), true, true, None, 10.0, 100.0, 50.0, 3000.0, 0.5);
        assert_eq!(r.labels.len(), 1, "col TEC attivo il guasto deve scattare");
    }

    /// Il guardia non deve mangiare il cooldown: saltata la regola, al
    /// ripristino del contesto il giudizio deve avvenire subito e non
    /// trovarsi gia' scaduto.
    #[test]
    fn guardia_non_consuma_il_cooldown() {
        let mut m = AlertManager::new(vec![AlertRule {
            channel:    AlertChannel::TecTemp,
            condition:  AlertCondition::Above(20.0),
            label:      "tec calda".to_owned(),
            enabled:    true,
            cooldown_s: 600,
        }]);

        for _ in 0..50 {
            let r = m.check(Some(24.0), false, true, None, 10.0, 100.0, 50.0, 3000.0, 0.5);
            assert!(r.labels.is_empty());
        }
        let r = m.check(Some(24.0), true, true, None, 10.0, 100.0, 50.0, 3000.0, 0.5);
        assert_eq!(
            r.labels.len(), 1,
            "dopo 50 campioni saltati col TEC fermo, accendendo deve scattare"
        );
    }

    /// Una regola "sotto" su un canale senza sensore non deve scattare: lo
    /// zero che si metteva al posto della misurazione mancante la faceva
    /// scattare subito e a ogni cooldown, per un dato che non esiste.
    #[test]
    fn regola_senza_sensore_non_scatta() {
        let regola = AlertRule {
            channel:    AlertChannel::CpuTemp,
            condition:  AlertCondition::Below(40.0),
            label:      "cpu fredda".to_owned(),
            enabled:    true,
            cooldown_s: 600,
        };
        let mut m = AlertManager::new(vec![regola]);

        // Nessuna lettura della CPU: la regola non puo' valutare.
        for _ in 0..10 {
            let r = m.check(Some(20.0), true, true, None, 5.0, 100.0, 50.0, 3000.0, 0.5);
            assert!(
                r.labels.is_empty(),
                "regola su canale assente ha scattato: {:?}",
                r.labels
            );
        }

        // Con la lettura reale, 30 °C e' sotto 40 e la regola scatta.
        let r = m.check(Some(20.0), true, true, Some(30.0), 5.0, 100.0, 50.0, 3000.0, 0.5);
        assert_eq!(r.labels.len(), 1, "con la misura reale deve scattare");
    }
}

#[cfg(test)]
mod test_pompa {
    /// La V1 non ha il sensore RPM. "Non leggibile" NON vuol dire "ferma".
    ///
    /// Se si limitasse la potenza, il TEC resterebbe al 50% per tutta la
    /// sessione, e la firma termica — che vuole potenza >= 60% per poter
    /// giudicare — non potrebbe mai concludere niente. La protezione si
    /// sarebbe autosabotata: bloccando la potenza, impediva a se stessa di
    /// vedere il guasto che voleva prevenire.
    ///
    /// Questa funzione replica la decisione del caller, che va tenuta
    /// separata dal resto della logica per essere testabile.
    const PUMP_MIN_RPM: f32 = 200.0;

    fn pompa_in_fallo(pump_readable: bool, rpm: f32, stalled: bool) -> bool {
        (pump_readable && rpm < PUMP_MIN_RPM) || stalled
    }

    #[test]
    fn senza_sensore_la_protezione_e_sospesa() {
        assert!(
            !pompa_in_fallo(false, 0.0, false),
            "assenza di sensore trattata come pompa ferma: la V1 resterebbe \
             bloccata al 50% e la firma termica non potrebbe giudicare"
        );
    }

    #[test]
    fn rpm_basso_e_guasto_reale() {
        assert!(pompa_in_fallo(true, 0.0, false));
        assert!(pompa_in_fallo(true, 199.0, false));
    }

    #[test]
    fn rpm_sano_non_e_guasto() {
        assert!(!pompa_in_fallo(true, 2500.0, false));
    }

    /// La firma termica vale anche senza sensore: e' l'unico giudizio
    /// disponibile sulla V1, quindi DEVE poter azionare la protezione.
    #[test]
    fn firma_termica_aziona_la_protezione_senza_sensore() {
        assert!(
            pompa_in_fallo(false, 0.0, true),
            "firma termica ignorata: la dashboard scriveva FERMA in rosso e \
             la potenza non cambiava"
        );
    }
}

#[cfg(test)]
mod test_potenza {
    /// Il nuovo `applied_power` dipende **solo** dall'esito della scrittura
    /// hardware, mai dall'intenzione.
    ///
    /// Prima l'invariante valeva in 4 dei 9 siti che scrivono la potenza. Il
    /// re-push periodico scriveva in hardware `applied_power`, cioe' il valore
    /// *creduto*: se la guardia era ferma nella banda morta e l'emergenza
    /// aveva abbassato il tetto, il re-push rimetteva il valore vecchio e
    /// annullava la protezione, ogni 10 secondi, all'infinito.
    ///
    /// Fallo come funzione pura: per testarlo serve sapere cosa diventa lo
    /// stato, non serve un controller.
    fn applica_potenza(
        scrittura: Result<(), ()>,
        next: u8,
        precedente: u8,
    ) -> u8 {
        match scrittura {
            Ok(()) => next,
            Err(_) => precedente,
        }
    }

    #[test]
    fn scrittura_riuscita_aggiorna_lo_stato() {
        assert_eq!(applica_potenza(Ok(()), 40, 100), 40);
    }

    #[test]
    fn scrittura_fallita_mantiene_lo_stato_precedente() {
        assert_eq!(
            applica_potenza(Err(()), 40, 100), 100,
            "stato aggiornato senza successo hardware: il re-push avrebbe \
             scritto in modulo un valore mai applicato"
        );
    }

    /// Il tetto d'emergenza non e' uno snapshot, e' una condizione.
    ///
    /// `max_power_before_emergency` veniva salvato col cap PRIMA dell'evento
    /// (quindi il cap piu' alto) e poi usato con `.min()` per "limitare": il
    /// min non limitava niente. Il tetto va ricalcolato ogni volta.
    #[test]
    fn tetto_e_ricalcolato_non_ricordato() {
        let tetto = |emergenza: bool, tetto_emergenza: u8, cap: u8| -> u8 {
            let t = if emergenza { tetto_emergenza } else { cap };
            cap.min(t)
        };
        // Emergenza attiva con tetto 50 e cap 100: deve limitare a 50.
        assert_eq!(tetto(true, 50, 100), 50);
        // Emergenza disattivata: il cap torna valido.
        assert_eq!(tetto(false, 50, 100), 100);
        // Cap gia' basso: la guardia non deve MAI alzare il tetto.
        assert_eq!(tetto(true, 50, 30), 30);
        assert_eq!(tetto(false, 50, 30), 30);
    }
}

#[cfg(test)]
mod test_scarto_tec {
    use super::*;

    fn regola_tec() -> AlertRule {
        AlertRule {
            channel:    AlertChannel::TecTemp,
            condition:  AlertCondition::Above(6.0),
            label:      "TEC calda".to_owned(),
            enabled:    true,
            cooldown_s: 600,
        }
    }

    /// Il caso che riguardava l'utente: l'allarme restava sempre acceso.
    ///
    /// Il canale riceve lo scarto dal setpoint, e senza il dato di rugiada
    /// il pavimento anti-condensa restituisce -1000. Se quel numero fosse
    /// entrato nello scarto, il valore sarebbe ~1000 e la regola
    /// scatterebbe ogni volta con un numero che non descriveva nulla.
    ///
    /// La difesa e' che uno scarto non credibile non e' uno scarto: si
    /// passa `None` e la regola non viene valutata.
    #[test]
    fn setpoint_non_credibile_non_valuta_la_regola() {
        let mut m = AlertManager::new(vec![regola_tec()]);
        for _ in 0..5 {
            let r = m.check(None, true, true, None, 5.0, 100.0, 50.0, 0.5, 0.5);
            assert!(
                r.labels.is_empty(),
                "regola scattata senza misura: {:?}",
                r.labels
            );
        }
    }

    /// Con scarto credibile la regola deve ancora funzionare: 9 °C sopra
    /// quanto chiesto e' un guasto vero.
    #[test]
    fn scarto_credile_fa_scattare() {
        let mut m = AlertManager::new(vec![regola_tec()]);
        let r = m.check(Some(9.0), true, true, None, 5.0, 100.0, 50.0, 0.5, 0.5);
        assert_eq!(r.labels.len(), 1, "scarto oltre soglia deve scattare");
    }

    /// E uno scarto piccolo, col TEC al lavoro, non deve: e' il caso normale.
    #[test]
    fn scarto_piccolo_non_scattare() {
        let mut m = AlertManager::new(vec![regola_tec()]);
        for _ in 0..5 {
            let r = m.check(Some(1.5), true, true, None, 5.0, 100.0, 50.0, 0.5, 0.5);
            assert!(r.labels.is_empty(), "falso positivo: {:?}", r.labels);
        }
    }
}

#[cfg(test)]
mod test_etichetta_valore {
    use super::*;

    #[test]
    fn potenza_mostra_watt_non_gradi() {
        let s = etichetta_con_valore(
            &AlertChannel::PowerWatts,
            239.4,
            "Potenza TEC",
            &AlertCondition::Above(200.0),
        );
        assert!(s.contains('W'), "manca l'unita' watt: {s}");
        assert!(!s.contains('°'), "i watt non sono gradi: {s}");
        assert!(s.contains("239"), "manca il valore: {s}");
    }

    #[test]
    fn temperatura_mostra_gradi() {
        let s = etichetta_con_valore(
            &AlertChannel::TecTemp,
            14.0,
            "TEC calda",
            &AlertCondition::Above(6.0),
        );
        assert!(s.contains('°'), "manca l'unita' gradi: {s}");
    }

    #[test]
    fn pompa_mostra_rpm() {
        let s = etichetta_con_valore(
            &AlertChannel::PumpRpm,
            150.0,
            "POMPA FERMA",
            &AlertCondition::Below(200.0),
        );
        assert!(s.contains("RPM"), "manca l'unita' RPM: {s}");
    }

    /// Il difetto che ha motivato il task: l'etichetta diceva 200 W
    /// mentre il valore che l'aveva fatta scattare era 113 W, perche'
    /// l'utente aveva abbassato la soglia e l'etichetta era rimasta quella
    /// di default. La soglia deve venire dalla condizione.
    #[test]
    fn l_etichetta_segue_la_soglia_reale() {
        let s = etichetta_con_valore(
            &AlertChannel::PowerWatts,
            113.0,
            "Potenza TEC",
            &AlertCondition::Above(100.0),
        );
        assert!(s.contains("100"), "la soglia reale non compare: {s}");
        assert!(!s.contains("200W"), "compare ancora la soglia di default: {s}");
    }

    /// Ogni canale ha la sua unita' nella soglia: watt e gradi non si
    /// confondono.
    #[test]
    fn la_soglia_ha_l_unita_del_canale() {
        let w = etichetta_con_valore(
            &AlertChannel::PowerWatts, 113.0, "Potenza TEC", &AlertCondition::Above(100.0),
        );
        assert!(w.contains('W'), "manca l'unita' watt: {w}");
        let t = etichetta_con_valore(
            &AlertChannel::CpuTemp, 75.0, "CPU", &AlertCondition::Above(70.0),
        );
        assert!(t.contains('°'), "manca l'unita' gradi: {t}");
    }

    /// **Nessuna regola predefinita scrive la soglia dentro l'etichetta.**
    ///
    /// Il difetto che l'utente ha visto: "Potenza TEC > 200W > 200W (94W)".
    /// La soglia la stampa `etichetta_con_valore` a partire dalla
    /// condizione, quindi averla anche nell'etichetta la faceva comparire
    /// due volte. E non e' solo questione di aspetto: con l'utente che
    /// cambia la soglia, l'etichetta direbbe un numero e la condizione
    /// un altro, e non si sa quale dei due sia quello vero.
    #[test]
    fn nessuna_regola_default_scrive_la_soglia_nell_etichetta() {
        for r in AlertRule::default_rules() {
            let testo = r.label.to_lowercase();
            for numero in ["200", "80", "50", "6", "1", "25"] {
                assert!(
                    !testo.contains(&format!(">{numero}"))
                        && !testo.contains(&format!("<{numero}"))
                        && !testo.contains(&format!("{numero}w"))
                        && !testo.contains(&format!("{numero}%")),
                    "la regola {:?} ha la soglia {numero} dentro l'etichetta: {:?}",
                    r.channel,
                    r.label
                );
            }
        }
    }

    /// La potenza non deve piu' scrivere il tetto come se fosse un limite.
    ///
    /// Il controller Gen 1 misurato regge 220-237 W: 200 W e' l'etichetta
    /// sulla scatola del kit, non un tetto erogabile. Un avviso che lo
    /// chiama "tetto" dice al lettore una cosa falsa sulla sua macchina.
    #[test]
    fn la_potenza_non_parla_di_tetto() {
        let regola = AlertRule::default_rules()
            .into_iter()
            .find(|r| r.channel == AlertChannel::PowerWatts)
            .expect("regola potenza presente");
        let testo = regola.label.to_lowercase();
        for parola in ["tetto", "limite", "massimo consentito", "200w"] {
            assert!(
                !testo.contains(parola),
                "l'etichetta di potenza parla di '{parola}': {:?}",
                regola.label
            );
        }
    }

    /// Anche una condizione "sotto" mostra la soglia, col verso giusto:
    /// sotto 10 non e' la stessa cosa che sopra 10.
    #[test]
    fn anche_la_condizione_sotto_mostra_la_soglia() {
        let s = etichetta_con_valore(
            &AlertChannel::HumidityPct, 30.0, "Umidita'", &AlertCondition::Below(40.0),
        );
        assert!(s.contains('<'), "manca il verso 'sotto': {s}");
        assert!(s.contains("40"), "manca la soglia: {s}");
    }
}

#[cfg(test)]
mod test_migrazione_regole {
    use super::*;

    #[test]
    fn tectemp_assoluta_obsoleta_viene_migrata() {
        // Le config scritte prima del cambio di semantica hanno
        // `Above(20.0)` con significato assoluto; valutata sullo scarto
        // non significa piu' niente. Va riportata al default.
        let vecchie = vec![AlertRule {
            channel: AlertChannel::TecTemp,
            condition: AlertCondition::Above(20.0),
            label: "vecchia".to_owned(),
            enabled: true,
            cooldown_s: 600,
        }];
        let nuove = migra_regole_obsolete(vecchie);
        assert_eq!(nuove.len(), 1);
        assert_eq!(nuove[0].threshold(), 6.0, "soglia migrata al default: {:?}", nuove[0]);
        assert!(nuove[0].enabled, "lo stato on/off dell'utente va preservato");
    }

    /// **L'utente ha gia' la vecchia etichetta su disco.** Senza questa
    /// migrazione la correzione sui default non cambierebbe nulla: la
    /// regola su disco continua a stampare "Potenza TEC > 200W > 200W
    /// (94W)" perche' il default nuovo non la tocca.
    #[test]
    fn l_etichetta_vecchia_su_disco_viene_corretta() {
        let regole = vec![AlertRule {
            channel: AlertChannel::PowerWatts,
            condition: AlertCondition::Above(200.0),
            label: "Potenza TEC > 200W".to_owned(),
            enabled: true,
            cooldown_s: 600,
        }];
        let nuove = migra_regole_obsolete(regole);
        assert_eq!(nuove[0].label, "Potenza TEC", "etichetta vecchia rimasta: {:?}", nuove[0].label);
        assert!(
            !nuove[0].label.contains("200W"),
            "la soglia deve sparire dall'etichetta: {:?}", nuove[0].label
        );
    }

    /// La condizione non si tocca: e' il dato vero, e la soglia scelta
    /// dall'utente va rispettata.
    #[test]
    fn la_migrazione_etichetta_non_tocca_la_soglia_scelta_dall_utente() {
        let regole = vec![AlertRule {
            channel: AlertChannel::PowerWatts,
            condition: AlertCondition::Above(100.0),
            label: "Potenza TEC > 200W".to_owned(),
            enabled: true,
            cooldown_s: 600,
        }];
        let nuove = migra_regole_obsolete(regole);
        assert_eq!(nuove[0].threshold(), 100.0, "soglia dell'utente sovrascritta");
    }

    /// Un'etichetta scritta a mano non si tocca: sovrascriverla sarebbe
    /// perdere una scelta dell'utente.
    #[test]
    fn un_etichetta_personale_non_viene_tocata() {
        let regole = vec![AlertRule {
            channel: AlertChannel::PowerWatts,
            condition: AlertCondition::Above(200.0),
            label: "potenza alta, controlla il radiatore".to_owned(),
            enabled: true,
            cooldown_s: 600,
        }];
        let nuove = migra_regole_obsolete(regole);
        assert_eq!(nuove[0].label, "potenza alta, controlla il radiatore");
    }

    #[test]
    fn regola_deviazione_recente_non_si_tocca() {
        let regole = vec![AlertRule {
            channel: AlertChannel::TecTemp,
            condition: AlertCondition::Above(6.0),
            label: "mia".to_owned(),
            enabled: false,
            cooldown_s: 120,
        }];
        let nuove = migra_regole_obsolete(regole);
        assert_eq!(nuove[0].threshold(), 6.0);
        assert!(!nuove[0].enabled);
        assert_eq!(nuove[0].cooldown_s, 120);
    }

    #[test]
    fn altri_canali_non_si_toccano() {
        let regole = vec![AlertRule {
            channel: AlertChannel::PowerWatts,
            condition: AlertCondition::Above(200.0),
            label: "potenza".to_owned(),
            enabled: true,
            cooldown_s: 600,
        }];
        let nuove = migra_regole_obsolete(regole);
        assert_eq!(nuove[0].threshold(), 200.0);
    }

    #[test]
    fn in_manuale_lo_scarto_non_valuta() {
        // In manuale non c'e' un setpoint inseguito: lo scarto non e'
        // un'anomalia, anche se enorme. E' il falso positivo segnalato.
        let mut m = AlertManager::new(vec![AlertRule {
            channel: AlertChannel::TecTemp,
            condition: AlertCondition::Above(6.0),
            label: "tec calda".to_owned(),
            enabled: true,
            cooldown_s: 600,
        }]);
        let r = m.check(Some(30.0), true, false, None, 5.0, 100.0, 50.0, 3000.0, 0.5);
        assert!(r.labels.is_empty(), "in manuale non deve scattare: {:?}", r.labels);
        let r = m.check(Some(30.0), true, true, None, 5.0, 100.0, 50.0, 3000.0, 0.5);
        assert_eq!(r.labels.len(), 1, "in automatico deve scattare");
    }
}

#[cfg(test)]
mod test_formatta_avviso {
    use super::*;

    #[test]
    fn con_valore_mostra_valore() {
        let s = formatta_avviso(
            &AlertChannel::PowerWatts,
            Some(239.4),
            "Potenza TEC",
            Some(&AlertCondition::Above(200.0)),
        );
        assert!(s.contains("239") && s.contains('W'), "deve mostrare i watt: {s}");
    }

    #[test]
    fn senza_valore_mostra_solo_etichetta() {
        // Niente trattini ne' zeri inventati: solo l'etichetta.
        let s = formatta_avviso(&AlertChannel::CpuTemp, None, "CPU?", None);
        assert_eq!(s, "CPU?");
    }

    /// Senza la regola non c'e' soglia da mostrare, e va detto in modo
    /// leggibile: stampare "> NaN" o "> 0W" farebbe credere all'utente che
    /// esista una soglia reale.
    #[test]
    fn senza_regola_non_inventa_una_soglia() {
        let s = formatta_avviso(
            &AlertChannel::PowerWatts,
            Some(113.0),
            "Potenza TEC",
            None,
        );
        assert!(!s.contains("NaN"), "NaN a schermo: {s}");
        assert!(!s.contains('>'), "soglia inventata: {s}");
        assert!(s.contains("113"), "il valore c'e' comunque: {s}");
    }
}
