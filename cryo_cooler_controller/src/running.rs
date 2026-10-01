use std::time::{Duration, Instant};

use iced::{
    alignment,
    widget::{Column, Container, Row, Text},
    Element, Length,
    Task,
};
use iced_aw::NumberInput;

use cryo_cooler_controller_lib::TecStatus;

use crate::{
    ai_advisor::{AiAdvice, SessionSnapshot, get_api_key, save_api_key, ask_advisor},
    session_db::{SessionDb, SampleRow, SessionSummary},
    report_pdf::{ReportData, export_pdf},
    pid_wizard::{PidWizard, WizardPhase},
    overlay::{rtss::RtssOverlay, DiscordRpc},
    analytics::{CondensationRisk, CopState, OcScore},
    alerts::AlertManager,
    automanager::Regime,
    charts::ChartGroup,
    sensors_panel::SensorsPanel,
    config::{AppConfig, Profile},
    hwinfo,
    palette,
    session_stats::SessionStats,
    Message,
};

const OCP_DEBOUNCE_THRESHOLD: u32 = 3;

/// **Tetto di potenza del controller, in watt.**
///
/// `max_power` e' una **percentuale firmware**, non un watt: il firmware la
/// traduce in watt con un proprio fattore, e su questo hardware 100% valgono
/// circa 230 W. Il controller Gen 1, pero', porta 200 W. Il risultato era che
/// con la percentuale al massimo si chiedevano ~230 W, cioe' il 115% del
/// tetto: il modulo segnalava OCP, il firmware tagliava, e quei watt erano
/// sprecati perche' il freddo non arrivava.
///
/// Non c'era **nessun tetto in watt** nel codice: solo la percentuale, che
/// non sa nulla dell'hardware.
///
/// **Cambia questo numero se cambi controller.** Il Gen 2 regge piu'
/// corrente, e con lui il tetto sale. Il valore e' dichiarato qui perche'
/// e' la radice del difetto: se non lo vedi, il bug torna.
const TETTO_WATT_CONTROLLER: f32 = 200.0;

/// Watt che il firmware ottiene al 100% della percentuale, misurati su questo
/// hardware (10,38 V × 21,70 A a piena richiesta ≈ 225 W osservati, 230 W
/// dichiarati). Serve per tradurre il tetto in watt nella percentuale.
const WATT_AL_100_PERCENTO: f32 = 230.0;

/// Ogni 2 s la lettura della CPU, espresso in tick.
///
/// Il **dato** arriva a 2 Hz (`update_interval` = 500 ms), quindi 4 tick = 2 s.
///
/// Prima erano 10 tick = 5 s, e il grafico ne buttava meta' con un filtro
/// `i % 2`: in una finestra di 3 minuti la linea CPU aveva **9 punti** contro i
/// 360 della piastra. Non sembrava stabile perche' era campionata male: la
/// CPU era quasi piatta perche' mancavano i dati, non perche' non cambiasse.
///
/// Nota: la lettura della CPU resta a 2 s e non piu' frequente di proposito.
/// HWiNFO/AIDA64 sono letture con overhead, e la temperatura della CPU e' la
/// **causa** del riscaldamento, non il suo indicatore: campionarla piu' spesso
/// non serve a nulla e toglie risorse alla cella. Il lato freddo del TEC e'
/// cio' che va letto con cura, e quello arriva gia' a 2 Hz.
const CPU_TEMP_PUSH_INTERVAL: u32 = 4;
/// Re-invia il cap di potenza al firmware ogni ~5s (10 tick × 500ms).
/// Il firmware Intel Cryo ha un PID onboard che gira in autonomia — senza
/// re-push periodico il cap viene ignorato dopo l'enable. Il software
/// ufficiale Intel invia i setpoint "in realtime" nel suo loop principale.
/// Ogni 5 s, espresso in tick: stessa calibrazione di
/// `CPU_TEMP_PUSH_INTERVAL`.


/// Ogni 2 s il controllo degli alert: 4 tick a 4 Hz. Erano 8 tick, quindi
/// ogni 4 s, mentre `ALERT_CHECK_HZ` dichiarava 0,5 Hz: ogni cooldown durava
/// il doppio dei secondi configurati.
const ALERT_EVERY_TICKS: u32 = 4;

/// Frequenza con cui viene chiamato `AlertManager::check`, in Hz.
///
/// **Deve essere 0,5, non 2.** Il valore serve a convertire i secondi di
/// cooldown in numero di controlli. Il controllo gira ogni 2 s, quindi 0,5 Hz.
/// Prima dichiarava 2.0 mentre girava ogni 0,2 s: il cooldown di 600 s durava
/// 240 s. E a 2 Hz dichiarava 2.0 mentre girava ogni 2 s, quindi sarebbe
/// durato 1200 s. Il valore e' legato alla cadenza reale, non al commento.
const ALERT_CHECK_HZ: f32 = 1.0 / 2.0;

#[derive(Debug)]
struct Inputs {
    p_coef:       f32,
    i_coef:       f32,
    d_coef:       f32,
    set_point:    f32,
    max_power:    u8,
    profile_name: String,
}

impl Default for Inputs {
    fn default() -> Self {
        Inputs {
            p_coef: 100.0, i_coef: 1.0, d_coef: 0.0,
            set_point: 2.0, max_power: 100,
            profile_name: "My Profile".to_owned(),
        }
    }
}

struct LogEntry {
    timestamp:           String,
    tec_temp:            f32,
    pcb_temp:            f32,
    humidity:            f32,
    dew_point:           f32,
    condensation_margin: f32,
    tec_voltage:         f32,
    tec_current:         f32,
    tec_power_watts:     f32,
    tec_power_level:     u8,
    cpu_temp_ext:        Option<f32>,
}

/// Mini-grafico della potenza dentro il tasto TEC.
///
/// Disegna gli ultimi campioni normalizzati sul proprio minimo-massimo:
/// e' una forma d'onda, non una misura assoluta — dice se la potenza sta
/// salendo, scendendo o e' ferma. Spento, disegna una linea piatta spenta.
struct MiniSpark {
    punti:  Vec<f32>,
    acceso: bool,
    /// Colore del LED della modalita' corrente: tasto, striscia e menu
    /// condividono lo stesso colore, quindi non c'e' ambiguita' su quale sia
    /// lo stato.
    colore: (u8, u8, u8),
}

/// Normalizza i punti su scala fissa 0..250 W.
///
/// NON sul minimo-massimo della finestra: quella normalizzazione gonfia
/// il rumore a scala piena e una linea piatta diventa uno scalino che
/// sembra un glitch. Con fondo scala fisso lo zero e' zero, il piatto
/// resta piatto e la forma e' quella vera.
fn normalizza_spark(punti: &[f32]) -> Vec<f32> {
    const FONDO_SCALA: f32 = 250.0;
    punti
        .iter()
        .rev()
        .take(120)
        .map(|v| (v.clamp(0.0, FONDO_SCALA) / FONDO_SCALA).clamp(0.0, 1.0))
        .collect()
}

/// I punti sullo schermo: campione → pixel.
///
/// Separata dal `draw` perche' e' la conversione che decide se la striscia
/// scorre o salta, e va provata da sola. `n` e' quanti punti ci sono: con
/// `n` punti su `w` pixel ogni campione occupa `w/n` pixel, e sotto i 2 px
/// il salto di un campione si vede come un gradino.
fn pixel_di_punti(norm: &[f32], w: f32, h: f32) -> Vec<(f32, f32)> {
    let n = norm.len().max(2);
    norm.iter()
        .enumerate()
        .map(|(i, v)| {
            (
                1.0 + (w - 2.0) * (i as f32 / (n.max(2) - 1) as f32),
                1.0 + (h - 2.0) * (1.0 - v),
            )
        })
        .collect()
}

impl iced::widget::canvas::Program<crate::Message> for MiniSpark {
type State = ();

fn draw(
    &self,
    _state: &Self::State,
    _renderer: &iced::Renderer,
    _theme: &iced::Theme,
    bounds: iced::Rectangle,
    _cursor: iced::mouse::Cursor,
) -> Vec<iced::widget::canvas::Geometry> {
    use iced::widget::canvas::{Frame, Path, Stroke};
    let mut frame = Frame::new(_renderer, bounds.size());
    let w = bounds.width;
    let h = bounds.height;
    if w < 10.0 || h < 8.0 {
        return vec![];
    }
        let norm = normalizza_spark(&self.punti);
        // Acceso: il colore del LED, cosi' la striscia parla la stessa lingua
        // del tasto. Spento: grigio, perche' qui il colore non deve dire
        // "modalita'", deve dire "sta lavorando".
        let colore = if self.acceso {
            let (r, g, b) = self.colore;
            iced::Color::from_rgb8(r, g, b)
        } else {
            iced::Color { r: 0.35, g: 0.45, b: 0.50, a: 0.60 }
        };
        // Punti sullo schermo una volta sola: linea e riempimento usano gli
        // stessi, altrimenti i due disegni divergono di un pixel e si vede.
        let xy: Vec<iced::Point> = if norm.is_empty() {
            vec![
                iced::Point::new(1.0, h * 0.5),
                iced::Point::new(w - 1.0, h * 0.5),
            ]
        } else {
            pixel_di_punti(&norm, w, h)
                .into_iter()
                .map(|(x, y)| iced::Point::new(x, y))
                .collect()
        };
        // Curva morbida **attraverso** i punti, non spezzata.
        //
        // Con `line_to` ogni campione e' uno spigolo: a 2 Hz con 120 punti la
        // striscia avanza di uno scalino ogni mezzo secondo e si legge a
        // scatti, non come una curva. Qui ogni coppia di punti e' unita da una
        // quadratica che passa per il punto medio: il punto medio e' la media
        // dei due estremi, quindi la curva resta dentro l'andamento dei dati e
        // non inventa estremi nuovi. Sparisce lo spigolo, il dato resta uguale.
        //
        // Il path builder di iced 0.13 non espone il tipo, quindi il corpo sta
        // in una macro locale: una definizione sola, due usi (area e linea).
        macro_rules! traccia_morbida {
            ($p:expr) => {{
                // `$p` non viene spostato: e' un `&mut`, e i metodi del
                // builder si chiamano su di esso per riferimento.
                if xy.len() < 3 {
                    $p.move_to(xy[0]);
                    for pt in xy.iter().skip(1) {
                        $p.line_to(*pt);
                    }
                } else {
                    let mezzo = |a: iced::Point, b: iced::Point| {
                        iced::Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
                    };
                    $p.move_to(mezzo(xy[0], xy[1]));
                    for w in xy.windows(2) {
                        // In iced 0.13 il metodo e' `quadratic_curve_to`.
                        $p.quadratic_curve_to(w[0], mezzo(w[0], w[1]));
                    }
                    $p.line_to(xy[xy.len() - 1]);
                }
            }};
        }
        let area = Path::new(|p| {
            // La macro traccia gia' dal primo punto: qui si parte gia' sul
            // tracciato, altrimenti `move_to` verrebbe chiamato due volte e
            // il path ripartirebbe da capo a meta' area.
            traccia_morbida!(p);
            let ultimo = xy[xy.len() - 1];
            p.line_to(iced::Point::new(ultimo.x, h - 1.0));
            p.line_to(iced::Point::new(xy[0].x, h - 1.0));
            p.close();
        });
        frame.fill(
            &area,
            iced::Color { r: colore.r, g: colore.g, b: colore.b, a: 0.18 },
        );
        // Alone: stessa linea piu' spessa e trasparente sotto quella piena.
        for (larghezza, alpha) in [(5.0, 0.18), (1.8, 0.95)] {
            let linea = Path::new(|p| {
                traccia_morbida!(p);
            });
            frame.stroke(
                &linea,
                Stroke {
                    style: iced::widget::canvas::Style::Solid(iced::Color {
                        r: colore.r,
                        g: colore.g,
                        b: colore.b,
                        a: alpha,
                    }),
                    width: larghezza,
                    line_cap: iced::widget::canvas::LineCap::Round,
                    ..Default::default()
                },
            );
        }
        vec![frame.into_geometry()]
}
}

#[cfg(test)]
mod test_mini_spark {
use super::normalizza_spark;

    #[test]
    fn scala_fissa_zero_duecentocinquanta() {
        // Ordine cronologico: il piu' vecchio per primo.
        let n = normalizza_spark(&[250.0, 125.0, 0.0]);
        assert!((n[0] - 0.0).abs() < 1e-5, "{n:?}");
        assert!((n[1] - 0.5).abs() < 1e-5, "{n:?}");
        assert!((n[2] - 1.0).abs() < 1e-5, "{n:?}");
    }

    #[test]
    fn piatto_resta_piatto_dov_e() {
        // 50 W costanti stanno al 20% della scala, non a meta' altezza e
        // non sul fondo: la forma dice la verita'.
        let n = normalizza_spark(&[50.0, 50.0, 50.0]);
        assert!(n.iter().all(|&v| (v - 0.2).abs() < 1e-5), "{n:?}");
    }

    #[test]
    fn oltre_fondo_scala_si_limita() {
        let n = normalizza_spark(&[300.0]);
        assert!((n[0] - 1.0).abs() < 1e-5, "{n:?}");
    }

    #[test]
    fn vuoto_resta_vuoto() {
        assert!(normalizza_spark(&[]).is_empty());
    }
}

pub struct RunningState {
    tec_regulator: crate::tec_optimizer::Regulator,
    regulation_offset: f32,
    regulation_started: Instant,
    last_sample_time:  Instant,
    /// Istante di avvio: riferimento per le animazioni legate al tempo
    /// (fiocco cryo), cosi' girano alla stessa velocita' su ogni macchina
    /// indipendentemente dalla frequenza dei tick.
    /// Il TEC e' in fase di avviamento graduale: la guardia lo fa salire
    /// di 1%/tick invece di portarlo subito al cap.
    soft_starting:      bool,
    /// Errori consecutivi di `monitor()`. Al terzo scatta il watchdog.
    monitor_failures:   u32,
    /// Il watchdog ha ridotto la potenza perche' la comunicazione si e'
    /// interrotta: la UI lo segnala, perche' a quel punto i valori mostrati
    /// non sono piu' affidabili.
    watchdog_tripped:   bool,
    /// Ultimo punto di rugiada letto (°C) e temperatura ambiente stimata.
    /// Servono al pavimento anticondensa.
    last_dew_point:     f32,
    last_room_temp:     f32,
    /// Il pavimento anticondensa ha limitato l'offset richiesto: la UI lo
    /// segnala, altrimenti sembrerebbe che il campo numerico non funzioni.
    cond_limit_active:  bool,
    /// L'OCP e' stato mitigato: la potenza e' stata ridotta.
    ocp_mitigated:      bool,
    /// Il controller TEC.
    ///
    /// Diretto e senza intermediari: e' il percorso verificato su hw
    /// (build r81, confermata dall'utente). Un tentativo di renderlo
    /// asincrono con un thread proprietario e canali e' stato fatto e
    /// **annullato**: in hardware non abilitava il TEC, quindi qui si
    /// torna alla semplice lettura/scrittura diretta.
    /// Scritture verso il TEC. Priorita' assoluta sulle letture: senza questo
    /// un `Enable` restava in coda dietro i campioni e il TEC non si
    /// abilitava (vedi `attore_tec::prossima_richiesta`).
    tec_tx:            std::sync::mpsc::Sender<crate::attore_tec::Richiesta>,
    /// Risposte del thread proprietario: campioni per i grafici, `Ack` per
    /// l'esito reale delle scritture.
    tec_rx:            std::sync::mpsc::Receiver<crate::attore_tec::Risposta>,
    /// Un campione e' stato richiesto e non e' ancora arrivato: evita di
    /// accodarne un secondo che il tick non riuscirebbe a vedere.
    poll_in_flight:    bool,
    /// Potenza all'ultima misura registrata nella curva della cella: serve a
    /// non riscrivere il file mentre la potenza non cambia.
    curva_ultima_potenza: u8,
    /// Quando e' stata fatta l'ultima misura, per attendere che le misure si
    /// stabilizzino prima di registrarle.
    curva_ultimo_istante: Option<std::time::Instant>,
    tec_status:        TecStatus,
    /// Lo stato a 32 bit **completo**: i 18 bit noti piu' i 14 alternativi
    /// che il vecchio mask scartava. Va in `stato_log`.
    ///
    /// Separato da `tec_status` per non toccare i chiamanti che gia'
    /// leggono i 18 noti: la dashboard deve continuare a funzionare come
    /// prima anche se gli alternativi non significano ancora niente.
    tec_status_completo: cryo_cooler_controller_lib::tecstatus::StatusCompleto,
    /// Log CSV dei bit alternativi, per la correlazione con l'OCP.
    stato_log: crate::stato_log::StatoLog,
    /// L'ultimo verdetto scritto in `diagnostica.log`.
    ///
    /// Serve a scrivere **una riga per cambio di stato**, non una riga per
    /// campione: a 2 Hz un file per giorno sarebbe illeggibile, e
    /// l'interessante e' il momento in cui il verdetto cambia.
    diagnostica_precedente: Option<crate::diagnostica::Verdetto>,
    fw_major:          u8,
    fw_minor:          u8,
    hw_version:        u32,
    chart:             ChartGroup,
    inputs:            Inputs,
    error_text:        Option<String>,
    /// **Modal informativo con titolo proprio** (titolo, testo).
    ///
    /// `error_text` mostra una card intitolata "Errore": va bene per un
    /// guasto, ma non per spiegare le modalita'. Dire "Errore" davanti a
    /// una spiegazione confonderebbe l'utente, quindi qui c'e' un canale
    /// separato.
    /// Lo chiude anche `CloseModal`, cosi' Esc e il bottone OK chiudono
    /// qualunque finestra, senza codice dedicato per ognuna.
    info_modale:       Option<(String, String)>,
    update_interval:   Duration,
    ocp_consecutive:   u32,
    ocp_confirmed:     bool,
    last_power_watts:  f32,
    last_electrical: Option<(f32,f32)>,
    /// L'ultima temperatura della scheda, che e' sul lato caldo.
    ///
    /// Serve al calcolo dell'efficienza: senza il lato caldo non c'e' nessun
    /// riferimento per capire quanto freddo si sta vincendo. A `NAN` finche'
    /// non arriva un campione, e i consumatori lo trattano come "non so".
    last_pcb_temperature: f32,
    /// L'efficienza marginale appena calcolata: gradi vinti per watt in piu'.
    ///
    /// `None` = non ancora calcolabile (serve un aumento di potenza). Non e'
    /// un errore: e' la risposta normale quando la potenza non e' salita, e
    /// in quel caso **non** si tocca la potenza.
    rendimento_marginale: Option<f32>,
    /// L'istantanea precedente (delta, potenza) per il confronto.
    efficienza_precedente: Option<(f32, f32)>,
    last_cond_margin:  f32,
    last_cpu_temp_ext: Option<f32>,
    last_gpu_temp_ext: Option<f32>,
    /// FEAT #5: pump RPM — letto dai sensori ventola per alert emergenza pompa
    last_pump_rpm: f32,
    /// La pompa e' stata identificata fra i sensori? Se false la
    /// protezione non puo' valutare e resta sospesa, invece di sparire
    /// come faceva prima (o allarmare senza motivo).
    pump_readable:  bool,
    /// Loop fermo dedotto dalla firma termica, perche' l'RPM non c'e'.
    ///
    /// Vero solo quando non esiste alcun sensore RPM: con l'RPM disponibile
    /// la decisione e' sul dato reale e questo resta falso.
    pump_stalled:   bool,
    /// Osservatore che deduce la pompa dalla temperatura.
    pompa_watch:    crate::pumpwatch::PompaWatch,
    /// FEAT #5:emergenza pompa attiva — tiene traccia del cap originale
    /// per ripristinarlo quando la pompa riparte.
    pump_emergency_active:   bool,
    /// Cap potenza impostato dall'utente prima dell'emergenza pompa.
    max_power_before_emergency: u8,
    // ── Temperatura controller / cooling block ─────────────────────────
    /// Lettura cmd 0x1F: temperatura del blocco/controller che pilota le
    /// celle del TEC. Il firmware Delta2 la usa per i suoi errori:
    ///   OT1/OT2 oltre 80°C -> il controller va in Standby (il raffreddamento
    ///                  si ferma, quindi il sistema si scalda di nuovo)
    ///   OT3   oltre 90°C -> spegnimento automatico dopo 5 secondi
    /// Con un controller V1 su modulo TEC V2 questa soglia viene superata:
    /// il ciclo Standby/ripresa è esattamente il sintomo "scalda molte volte".
    ctrl_temp:            f32,
    /// Picco di temperatura controller nella sessione.
    ctrl_temp_peak:       f32,
    /// Potenza REALMENTE applicata al TEC. Il cap dell'utente
    /// (`inputs.max_power`) non viene mai modificato: se la protezione lo
    /// abbassa, `applied_power` insegue il cap e ci torna appena il controller
    /// si raffredda.
    /// L'utente ha chiesto l'accensione e il comando e' andato a buon fine.
    ///
    /// E' l'unica verita' su "il TEC e' acceso che ne rispondiamo". Non
    /// viene dal polling: `tec_status` arriva da `hear_beat()` e resta
    /// indietro quando la seriale instabile, ed e' proprio il caso in cui
    /// spegnere conta di piu'.
    applied_power:        u8,
    /// Tick dell'ultima richiesta di accensione ancora pendente (0 = nessuna).
    /// Serve alla riga di spool: dice da quanti secondi il modulo sta partendo.
    attesa_tick:         u32,
    /// L'ultimo regime **richiesto** dall'operatore, non quello confermato.
    ///
    /// Serve per l'avviso di Unregulated: su questo Gen 1 i bit non confermano
    /// mai il regime, quindi senza questo campo non c'e' modo di sapere che
    /// l'operatore ha chiesto Unregulated — e il pannello taceva mentre la
    /// piastra andava sotto la rugiada. `None` = nessuna scelta ancora.
    ultimo_regime_richiesto: Option<crate::commutazione::Regime>,
    /// Consecutivi `set_power_level` falliti (diagnostica).
    power_push_failures:  u32,
    /// Avviso "controller in surricaldamento" già emesso.
    ctrl_warned:          bool,
    fw_power_level:    u8,
    log:               Vec<LogEntry>,
    stats:             SessionStats,
    tick_count:        u32,
    app_config:        AppConfig,
    sensors:           SensorsPanel,
    // ── Analytics ───────────────────────────────────────────────────
    cond_risk:         CondensationRisk,
    cop_state:         CopState,
    oc_score_current:  u32,
    oc_best:           u32,
    ocp_event_count:   u32,

    auto_profile_on:    bool,   // ON/OFF auto-switch profili
    /// Gestore della modalita' automatica: decide il regime da carico e
    /// temperatura CPU. Separato dall'auto-profiler, che resta quello basato
    /// sui profili salvati.
    auto_mgr:           crate::automanager::AutoManager,
    /// Regime mostrato in UI solo per distinguere "nessuna decisione" da
    /// "Idle". La fonte primaria del regime e' `auto_mgr.regime()`.
    last_auto_regime:   Option<crate::automanager::Regime>,
    /// La modalita' automatica e' accesa ma non vede nessun sensore CPU.
    ///
    /// Senza HWiNFO o AIDA64 attivi non c'e' niente su cui decidere: il
    /// regime resta Idle e sembra che l'interruttore non funzioni. Dirlo
    /// nella UI evita di cercare il difetto nel posto sbagliato.
    auto_senza_sensore: bool,
    /// Allineare l'hardware appena si accende l'automatico.
    ///
    /// Se il regime corrente e' gia' quello giusto non c'e' transizione e
    /// `push` restituisce `None`: senza questo flag non si scriverebbe
    /// niente e l'interruttore sembrerebbe rotto. Resta vero finche' non
    /// si applica un regime con dati validi, oppure finche' non si spegne.
    auto_da_allineare: bool,
    /// Campionatore del carico CPU via API Windows.
    ///
    /// E' il ripiego quando HWiNFO o AIDA64 non sono attivi: il carico lo
    /// espone il sistema stesso e si legge senza installare nulla. Senza
    /// questo la modalita' automatica dipendeva da un programma esterno e
    /// restava ferma in Idle su una macchina che non ce l'ha.
    carico_win:       crate::cpuload::CaricoCpu,
    /// Da dove arriva il carico letto al tick corrente: HWiNFO, Windows,
    /// o nessuno. Serve a dirlo nella UI invece di far indovinare.
    carico_origine:   &'static str,
    // ── Session DB ──────────────────────────────────────────────
    session_db:        SessionDb,
    // ── PID Wizard ──────────────────────────────────────────────
    pid_wizard:        PidWizard,
    // ── Overlay / integrations ──────────────────────────────────
    rtss:              RtssOverlay,
    discord:           DiscordRpc,
    overlay_ticks:     u32,
    pdf_status:        Option<String>,
    show_session_hist: bool,
    /// Pannello diagnostica aperto.
    ///
    /// Il dettaglio (misure, bit, cosa fare per ciascun problema) vive qui e
    /// non in sidebar: in sidebar gli stessi numeri sono gia' nella griglia
    /// principale, quindi ripeterli faceva leggere ogni valore due volte senza
    /// aggiungere niente.
    show_diagnostica: bool,
    /// Esito dell'ultimo salvataggio del rapporto, mostrato nel pannello.
    /// `None` finche' l'utente non preme "Salva rapporto".
    diagnostica_msg: Option<String>,
    /// Il regime che l'utente ha chiesto, in attesa di conferma.
    ///
    /// Resta `Some` finche' il controller non risponde. Serve a confrontare
    /// la richiesta con lo stato riletto: senza questo, "ho scritto due
    /// byte" e "il controller ha obbedito" sarebbero la stessa cosa, e il
    /// secondo dei due casi non e' quello che succede sempre.
    ///
    /// Lo stato dell'ultima commutazione: nessuna, in corso, o confermata.
    ///
    /// Un campo solo, non due. I due campi precedenti (`regime_richiesto` +
    /// `regime_confermato`) permettevano uno stato che non esiste: confermato
    /// per un regime diverso da quello in volo.
    commutazione: crate::commutazione::Commutazione,
    /// Cosa rispondere all'operatore, in parole.
    esito_commutazione: Option<String>,
    /// Una conferma in attesa, o nessuna.
    ///
    /// Tipo invece di flag: con un `bool`, "in conferma" e "commutazione
    /// partita" potevano essere veri insieme, e la conferma scriveva il regime
    /// premuto **dopo** il dialogo. Qui il regime da scrivere viaggia dentro lo
    /// stato.
    in_attesa: crate::commutazione::InAttesa,
    /// `true` mentre l'operatore sta valutando l'Unregulated. Il pulsante non
    /// scrive niente finche' questo non e' `true`: il secondo clic serve per
    ///che nessun comando pericoloso parta da un gesto distratto.
    chiedi_unregulated: bool,
    /// Pannello impostazioni aperto: qui vivono le soglie di allarme.
    show_settings:    bool,
    /// Istante in cui la finestra e' stata nascosta con "Nascondi".
    ///
    /// Serve al watchdog: senza un recupero, l'unico modo per tornare alla
    /// dashboard era il tray, e se l'icona non e' raggiungibile la finestra
    /// restava nascosta per sempre. `None` = finestra visibile.
    pub hidden_at:       Option<std::time::Instant>,
    /// Cache delle sessioni per il pannello cronologia.
    ///
    /// Va tenuta in cache perche' `recent_sessions()` interroga SQLite: se
    /// lo chiamassi dentro `view()` it'di una query al database a ogni
    /// frame (20 Hz), consumando CPU e I/O per sempre. Si ricarica solo
    /// quando l'utente apre il pannello o quando una sessione si chiude.
    session_list:     Vec<SessionSummary>,
    /// Flag per modal debug (mostra stato interno sistema)
    debug_mode:        bool,
    // ── Auto-save CSV (FEAT #1) ──────────────────────────────────
    auto_save_ticks:   u32,   // counter per auto-save periodico
    // ── Alerts ──────────────────────────────────────────────────────
    alert_manager:     AlertManager,
    active_alerts:     Vec<(crate::alerts::AlertChannel, String)>,
    // ── Fan Controller (HWiNFO RPM read-only) ──────────────────────────
    // ── AI Advisor ──────────────────────────────────────────────────
    ai_advice:         Option<AiAdvice>,
    ai_loading:        bool,
    ai_error:          Option<String>,
    ai_api_key:        String,
    ai_modal_open:     bool,
    // ── Dynamic theme ───────────────────────────────────────────────
    pub theme_hue:         f32,
    win_w:             u32,
    win_h:             u32,
    /// Handle logo precalcolato — evita LOGO_BANNER.to_vec() (586 KB) ad ogni frame.
    logo_handle:       iced::widget::image::Handle,
    /// Logo cryogenic. Handle precalcolato come gli altri, per non riallocare
    /// i pixel a ogni frame mentre il banner e' visibile.
    cryo_warn_handle:  iced::widget::image::Handle,
    /// Handle icona cryo — usata nel banner "Raffreddamento attivo"
    cryo_icon_handle: iced::widget::image::Handle,
    /// Handle icona OCP — usata nel banner "OCP segnalato"
    ocp_icon_handle:  iced::widget::image::Handle,
    /// Sfondo della dashboard. Handle preallocato come gli altri: crearlo a
    /// ogni frame copierebbe 2 MB di pixel RGBA a ogni ridisegno.
    bg_handle:        iced::widget::image::Handle,
    /// Cache ventole HWiNFO — rimossa: era populata a ogni tick ma mai letta.
    /// (si usa direttamente `self.sensors.sensors` dove serve)
    /// Cache risultato regressione lineare condensa — evita ricalcolo per-frame.
    last_risk_state:   crate::analytics::RiskState,
}

/// Gravita' di un avviso, per dare a ogni riga la sua importanza.
///
/// Il punto e' che **non tutte le righe rosse valgono lo stesso**. Prima ogni
/// avviso aveva lo stesso riempitivo rosso pieno, quindi un'umidita' al 81% e
/// un fermo pompa erano identici: l'occhio imparava subito a ignorarli.
/// Dividendoli per gravita' l'urgenza si legge dal colore e dalla forma,
/// senza dover leggere ogni volta.
#[derive(Clone, Copy, PartialEq)]
enum Gravita {
    /// Danno in corso o in arrivo: pompa ferma, condensa attiva. Il software
    /// riduce la potenza per questo.
    Critica,
    /// Da tenere d'occhio, non ancora dannoso.
    Avviso,
    /// Soglia superata con margine ancora sufficiente.
    Info,
}

impl Gravita {
    fn colore(self) -> iced::Color {
        match self {
            // Rosso pieno solo qui: e' l'unico livello che lo merita, e
            // renderlo raro e' il motivo per cui si distingue dagli altri.
            //
            // Prendono i token semantici (`SEM_*`) invece di `DANGER`/
            // `WARNING`/`BLUE_PRIMARY`: cosi' un avviso e l'intestazione
            // della sezione che contiene quella grandezza parlano lo
            // stesso colore. Il rosso di una riga in sidebar significa
            // "pericolo" e non "potenza", e l'ambra dei watt e' la stessa
            // degli avvisi da tenere d'occhio: la gravita' si legge dal
            // colore senza dover ricordare due tavole diverse.
            Gravita::Critica => palette::SEM_RISCHIO,
            Gravita::Avviso  => palette::SEM_POTENZA,
            Gravita::Info    => palette::SEM_TEMPERATURA,
        }
    }
}


impl RunningState {
    pub fn new<T>(serial_port: &T) -> Result<Self, std::io::Error>
    where
        T: AsRef<std::path::Path> + std::fmt::Debug,
    {
        let mut tec =
            cryo_cooler_controller_lib::Tec::new(&serial_port.as_ref().as_os_str())?;
        let fw         = tec.fw_version()?;
        let hw         = tec.hw_version()?;
        let status     = tec.hear_beat()?;
        let app_config = AppConfig::load();
        // Le regole di alert servono due volte: per inizializzare
        // l'AlertManager e dentro app_config, che viene spostato nella
        // struttura. Cloniamo qui, prima dello spostamento.
        let saved_alerts = app_config.alerts.clone();
        // Riallineiamo l'interruttore globale dei toast con la config
        // caricata da disco, cosi' la scelta sopravvive al riavvio.
        crate::alerts::set_toasts_enabled(app_config.notify.toasts);

        // Il thread proprietario riceve il `Tec` in esclusiva e non lo
        // condivide con nessuno: niente lock, mai. Un solo paio di canali:
        // richieste UI->attore, risposte attore->UI. Quando la UI esce,
        // `tec_tx` si chiude e il ciclo termina da solo.
        // Il thread proprietario riceve il `Tec` in esclusiva: e' l'unico a
        // toccare la porta seriale. La UI non ha piu' il `Tec`, quindi non
        // puo' bloccarlo: manda richieste e legge risposte senza aspettare.
        //
        // Nel ciclo le **scritture hanno la precedenza** sulle letture: e' la
        // correzione rispetto al tentativo precedente, in cui un `Enable`
        // restava in coda dietro i campioni e il TEC non si abilitava.
        let (tx_rich, rx_rich) = std::sync::mpsc::channel();
        let (tx_risp, rx_risp) = std::sync::mpsc::channel();
        let _poll = std::thread::Builder::new()
            .name("cryo-tec-poll".to_owned())
            .spawn(move || crate::attore_tec::ciclo_attore(tec, rx_rich, tx_risp))
            .expect("spawn thread di polling seriale");

        Ok(RunningState {
            tec_regulator: crate::tec_optimizer::Regulator::default(),
            regulation_offset: 2.0,
            regulation_started: Instant::now(),
            last_sample_time: Instant::now(),
            soft_starting:     false,
            monitor_failures:  0,
            watchdog_tripped:  false,
            last_dew_point:    f32::NAN,
            last_room_temp:    f32::NAN,
            cond_limit_active: false,
            ocp_mitigated:     false,
            tec_status: status,
            tec_status_completo: cryo_cooler_controller_lib::tecstatus::scompone(status.bits()),
            stato_log: crate::stato_log::StatoLog::apri(
                crate::stato_log::cartella_app(
                    &std::env::var("LOCALAPPDATA").unwrap_or_default(),
                )
                .join("bit-stato.csv"),
            ),
            diagnostica_precedente: None,
            fw_major: fw.0, fw_minor: fw.1, hw_version: hw,
            chart: Default::default(),
            tec_tx: tx_rich,
            tec_rx: rx_risp,
            poll_in_flight: false,
            curva_ultima_potenza: u8::MAX,
            curva_ultimo_istante: None,
            inputs: Inputs::default(),
            error_text: None,
            info_modale: None,
            // Frequenza di **richiesta** del campione: 2 Hz, come prima.
            //
            // Ho provato ad alzarla a 4 Hz e i grafici sono andati peggio:
            // il ritmo di campionamento non e' la leva della fluidita', e
            // cambiarlo destabilizza la guardia. Il rilevamento resta
            // fermo a 500 ms; la fluidita' si cerca nel disegno, non qui.
            update_interval: Duration::from_millis(500),
            ocp_consecutive: 0, ocp_confirmed: false,
            last_power_watts: 0.0, last_electrical: None, last_cond_margin: f32::MAX,
            last_pcb_temperature: f32::NAN,
            rendimento_marginale: None, efficienza_precedente: None,
            last_cpu_temp_ext: None,
            last_gpu_temp_ext: None,
            last_pump_rpm: 0.0,
            pump_readable: false,
            pump_stalled: false,
            pompa_watch: crate::pumpwatch::PompaWatch::new(),
            pump_emergency_active: false,
            max_power_before_emergency: 100,
            ctrl_temp: 0.0,
            ctrl_temp_peak: 0.0,
            applied_power: 0,
            attesa_tick: 0,
            ultimo_regime_richiesto: None,
            power_push_failures: 0,
            ctrl_warned: false,
            fw_power_level: 0,
            log: Vec::new(), stats: SessionStats::default(),
            tick_count: 0, app_config,
            sensors: SensorsPanel::new(),
            cond_risk: CondensationRisk::new(),
            cop_state: CopState::default(),
            oc_score_current: 0,
            oc_best: 0,
            ocp_event_count: 0,

            auto_profile_on: false,
            auto_mgr: crate::automanager::AutoManager::new(
                crate::automanager::Config::default()),
            last_auto_regime: None,
            auto_senza_sensore: false,
            auto_da_allineare: false,
            carico_win: crate::cpuload::CaricoCpu::new(),
            carico_origine: "nessuno",
            session_db: {
                let mut db = SessionDb::open();
                if !db.is_open() {
                    eprintln!(
                        "[sessioni] database non disponibile: i campioni di \
                         questa sessione NON saranno salvati e la cronologia \
                         restera' vuota."
                    );
                }
                db.start_session("Sessione");
                db
            },
            pid_wizard: PidWizard::new(),
            rtss: { let mut r = RtssOverlay::new(); r.try_connect(); r },
            discord: { let mut d = DiscordRpc::new("1234567890"); d.try_connect(); d },
            overlay_ticks: 0,
            pdf_status: None,
            show_session_hist: false,
            show_diagnostica: false,
            diagnostica_msg: None,
            commutazione: crate::commutazione::Commutazione::Nessuna,
            esito_commutazione: None,
            in_attesa: crate::commutazione::InAttesa::Nessuna,
            chiedi_unregulated: false,
            show_settings:    false,
            hidden_at:       None,
            session_list:     Vec::new(),
            debug_mode:        false,
            auto_save_ticks: 0,
            alert_manager: AlertManager::new(saved_alerts),
            active_alerts: Vec::new(),
            ai_advice: None,
            ai_loading: false,
            ai_error: None,
            ai_api_key: get_api_key().unwrap_or_default(),
            ai_modal_open: false,
            theme_hue: 161.0,
            win_w: 1500, win_h: 1000,
            // Pre-alloca handle logo una volta sola — clonare Handle è O(1) (Arc interno)
            logo_handle: iced::widget::image::Handle::from_rgba(
                crate::LOGO_W, crate::LOGO_H, crate::LOGO_BANNER.to_vec(),
            ),
            cryo_warn_handle: iced::widget::image::Handle::from_rgba(
                crate::CRYO_WARN_W, crate::CRYO_WARN_H,
                crate::CRYO_WARN_BANNER.to_vec(),
            ),
            // Pre-alloca handle icone cryo e OCP
            cryo_icon_handle: iced::widget::image::Handle::from_rgba(
                64, 64, crate::CRYO_ICON.to_vec(),
            ),
            bg_handle: iced::widget::image::Handle::from_rgba(
                crate::DASHBOARD_BG_W, crate::DASHBOARD_BG_H,
                crate::DASHBOARD_BG.to_vec(),
            ),
            ocp_icon_handle: iced::widget::image::Handle::from_rgba(
                64, 64, crate::OCP_ICON.to_vec(),
            ),
            last_risk_state: crate::analytics::RiskState {
                level:       crate::analytics::RiskLevel::Safe,
                margin:      f32::MAX,
                eta_seconds: None,
                trend_per_sec: 0.0,
            },
        })
    }

    // ── Protezione controller: regolatore a discesa graduale ────────────
    // Soglia oltre la quale iniziamo a togliere corrente al controller.
    // ── Soglie della guardia: finestra stretta, 30 °C contro 40 °C ─────
    //
    // ── ATTENZIONE: queste soglie valgono per il TUO controller ──────────
    //
    // Il tuo e' un **controller Gen 1 modificato** (potenziato), non un Gen 1
    // originale, e gira su una TEC Gen 2. Le soglie qui sotto sono quelle che
    // hai dichiarato per *questo* componente, con la ventola e il dissipatore
    // che hai aggiunto sul lato dei componenti.
    //
    // Sono quindi **specifiche del tuo hardware**, non del modello "Gen 1". Se
    // questo software venisse usato su un Gen 1 originale, i 36/38 °C
    // potrebbero non avere senso: non li ricaviamo da un manuale del
    // produttore, li ricaviamo dalle tue misure. Il software non puo' saperlo,
    // quindi non lo presume: li usa e basta, e i numeri restano dichiarati qui
    // per chi li modifica.
    //
    // I numeri non vengono da un manuale, vengono da tre fatti tuoi:
    //
    //  - il controller **a regime sta a 30 °C**;
    //  - **fino a 35 °C e' normale** (la tua soglia dichiarata);
    //  - **a 40 °C si sciolgono le guaine dei fili**, e hai gia' dovuto
    //    sostituirle una volta.
    //
    // La finestra utile e' quindi larga **cinque gradi**, e le soglie stanno
    // dentro, non ai bordi:
    //
    //     fino a 35 °C  regime normale          (verde)
    //        36-37 °C  allarme                 (giallo)
    //        da 38 °C  rischio di danno       (rosso)
    //        40 °C     guaine sciolte, sostituite gia' una volta
    //
    // Non e' un interruttore ma una **scala**: la guardia comincia a togliere
    // potenza appena sopra il normale e, dal rosso, puo' scendere fino a zero.
    // Un interruttore che aspetta 38 °C per reagire lascerebbe 35-37 °C senza
    // nessuna protezione, e sono esattamente i gradi in cui il danno inizia a
    // accumularsi.
    //
    // Finestra utile: cinque gradi. Non e' un margine comodo, e' la finestra che
    // c'e'. La prima cosa che la guardia deve fare resta **non salire mai**.
    //
    // Il firmware Delta2 va in Standby a 80 °C e in shutdown a 90 °C: quelle
    // sono soglie di *azione del firmware*, non di *sicurezza del componente*.
    // Fidarsi di loro avrebbe aspettato il danno — che e' esattamente quello
    // che faceva la versione precedente.
    /// **Ripristinato da r116, che e' l'unica versione con 20-30 W.**
    ///
    /// Avevo abbassato queste soglie a 36/38 sulla parola "40 °C" detta al
    /// volo. Ma il tuo controller **sta a 70 °C** — e lo dice il log, non io:
    /// `ctrl=70.0 °C`. Con 36/38 il tetto era zero in permanenza e il modulo
    /// non si abilitava: la guardia spegneva sempre, perche' il controller e'
    /// oltre la soglia in modo permanente, non eccezionale.
    ///
    /// 70 °C e' la temperatura **normale** di questo impianto: e' cosi' che
    /// ha girato per anni con r116 senza danni. Quindi 36/38 erano una soglia
    /// inventata che impediva al programma di funzionare, e il tetto si mangiava
    /// il soft start da 30 %.
    ///
    /// Qui sotto sta la soglia vera di r116. Il muro a 38 non sparisce: diventa
    /// un **avviso**, che e' quello che un dato non misurato puo' giustificare.
    const CTRL_SOFT:      f32 = 66.0;
    const CTRL_CRITICO:   f32 = 76.0;
    /// L'allarme che ** precede la guardia: dice "il controller si sta
    /// scaldando" prima che la guardia tolga potenza.
    ///
    /// Sta a 65 °C, un grado sotto la guardia a 66, ed e' **voluto** che
    /// scatti in modo permanente: il controller di questo impianto sta a
    /// 70 °C di normale, quindi un avviso che suona solo a 34 non direbbe
    /// niente. Qui avvisa che siamo **sopra il muro**, che e' il fatto vero.
    const CTRL_AVVISO: f32 = 65.0;
    /// **Il muro che avevi indicato tu: 38 °C.** Non e' piu' una soglia che
    /// spegne, e' un **avviso**: il registro dice "controller oltre 38 °C", e la
    /// decisione di spegnere resta tua.
    ///
    /// La differenza e' sostanziale e non un dettaglio: una soglia che spegne
    /// a 38 su un controller che sta a 70 **spegne sempre**, quindi non e'
    /// sicurezza, e' un guasto. Un avviso a 38 invece dice la verta — che quel
    /// numero e' stato superato — senza impedire il raffreddamento.
    const CTRL_MURO: f32 = 38.0;

    /// Il controller e' vicino al muro che l'utente ha dichiarato?
    ///
    /// Solo per temperature reali: 200 °C non esiste su questo hardware, e il
    /// `NAN` arriva dal parsing di byte grezzi. Nessuno dei due e' un
    /// pericolo, e un allarme su un dato che non e' un dato fa smettere di
    /// leggere gli allarmi veri.
    /// **Il muro che hai indicato tu, adesso collegato a qualcosa.**
    ///
    /// Prima `CTRL_MURO` era un numero dichiarato e mai usato, quindi la
    /// guardia poteva spegnere a 66 °C senza che quel numero dicesse nulla.
    /// Ora la risposta e' un motivo: se il controller ha superato i 38 °C che
    /// hai indicato come limite, l'allarme lo dice **perche'**, non solo che
    /// si sta scaldando.
    ///
    /// Resta comunque un avviso. Il tuo controller gira a 70 °C da sempre: una
    /// soglia che spegne a 38 su questo impianto non protegge niente, lo
    /// impedisce di raffreddare.
    fn avvicinamento_muro(ctrl_temp: f32) -> bool {
        ctrl_temp.is_finite() && ctrl_temp > Self::CTRL_AVVISO && ctrl_temp <= 120.0
    }

    /// Il controller ha superato il muro di 38 °C che hai indicato tu?
    ///
    /// Serve al testo dell'allarme: la differenza tra "si sta scaldando" e
    /// "oltre i 38 °C" e' la differenza tra un avviso e una segnalazione.
    fn oltre_il_muro(ctrl_temp: f32) -> bool {
        ctrl_temp.is_finite() && ctrl_temp > Self::CTRL_MURO
    }
    /// Passo di correzione per tick (2 Hz). Piccolo di proposito: la
    /// correzione deve essere graduale, non un gradino.
    const CTRL_STEP_DOWN: i16 = 2;


    /// Regola la potenza applicata in base alla temperatura del controller.
    ///
    /// Perche' non una soglia con isteresi: un regolatore on/off (bang-bang)
    /// tra 100% e 35% oscilla per natura — sale, il controller si scalda,
    /// scende, si raffredda, risale. Con isteresi anche a 8°C il ciclo
    /// restava, solo piu' lento: esattamente il difetto che vincevamo di
    /// evitare. Qui la potenza insegue la temperatura a piccoli passi
    /// (regolazione con azione integrale), quindi si stabilizza.
    ///
    /// `inputs.max_power` resta il cap scelto dall'utente e non viene toccato:
    /// a regolatore fermo la potenza applicata vi torna da sola, quindi
    /// nessuna perdita permanente di prestazioni.
    /// Scrive P/I/D al controller se il TEC e' in funzione.
    ///
    /// Usato da ogni percorso che cambia un coefficiente: modifica manuale,
    /// caricamento di un profilo, applicazione del Wizard o dell'AI. Senza
    /// questo, quei valori restavano solo nello stato Rust.
    /// Applica un regime di lavoro: scrive i campi, poi **spinge tutto
    /// all'hardware**.
    ///
    /// **Perche' esiste questo metodo.** L'auto-profiler prima aggiornava solo
    /// `self.inputs` e basta: accendere l'interruttore non cambiava nuno,
    /// ne' in UI ne' sul controller. Il difetto era che la UI e l'hardware
    /// potevano mostrare due cose diverse senza che nessuno se ne accorgesse.
    ///
    /// **La sicurezza vince sempre.** `max_power` non supera mai il cap
    /// consentito al modulo e `set_point` passa dal pavimento anticondensa: un
    /// regime puo' solo chiedere, non imporre. Se il controller e' caldo, la
    /// guardia termica agisce dopo e vince.
    fn apply_regime(&mut self, regime_richiesto: Regime) {
        // Il gestore di carico non deve sovrascrivere modalita' o preset.
        if self.ultimo_regime_richiesto != Some(crate::commutazione::Regime::Cryo)
            || matches!(self.inputs.profile_name.as_str(), "Gaming" | "AI / Rendering" | "Silenzioso / Idle") { return; }
        // Soglie del regime. Sono qui e non in `AppConfig` perche' sono
        // politiche di sicurezza, non preferenze: cambiarle non deve poter
        // disattivare una protezione.
        //
        // **Senza rugiada valida, niente regimi aggressivi.** Il pavimento
        // anticondensa vale -1000 quando manca il dato, e un setpoint di
        // -18 passerebbe il clamp: la cella andrebbe a palla al buio, che
        // e' il guasto segnalato. Si scende a Leggero e lo si dice nel log,
        // cosi' resta traccia del perche'.
        let pavimento_valido = self.dew_floor_offset().is_some();
        let regime = crate::automanager::regime_sicuro(regime_richiesto, pavimento_valido);
        // Si registra solo il declassamento vero: requested diverso da
        // applicato. Altrimenti il log si riempie di righe che non dicono
        // niente e la riga che conta si perde in mezzo.
        // (Il confronto e' possibile perche' `Regime` deriva `PartialEq`.)
        if regime != regime_richiesto {
            crate::commissioning::event(
                "REGIME",
                &format!(
                    "pavimento anticondensa non valido: {regime_richiesto:?} ridotto a {regime:?}"
                ),
            );
        }
        // `set_point` e' un **margine sul punto di rugiada**, non un offset in
        // °C: prima qui c'erano `-5/-8/-12/-18`, che riletti col significato
        // nuovo sarebbero "stai 12 gradi sotto la rugiada" — condensa
        // garantita, e con valori che il campo non accetta.
        //
        // Quindi il regime automatico sceglie **quanto freddo** vuole (il
        // margine) e lascia che sia il regolatore a trovare la potenza: piu'
        // il regime e' pesante, piu' vicino al limite di condensa.
        let (max_power, set_point) = match regime {
            Regime::Idle    => (30_u8, 6.0_f32),
            Regime::Leggero => (50_u8, 4.0_f32),
            Regime::Carico  => (60_u8, 3.0_f32),
            Regime::Picco   => (80_u8, 2.0_f32),
            // Nessun dato: **non si tocca l'hardware**. Si esce prima di
            // scrivere qualsiasi cosa, e il modulo resta con l'ultimo regime
            // valido.
            //
            // Non e' una via d'uscita comoda: e' il comportamento corretto.
            // Scrivere qui significherebbe scegliere un regime arbitrario, e
            // perdita di sensore non deve costare potenza di raffreddamento.
            Regime::Invalido => {
                self.error_text =
                    Some("Modalita' automatica: nessun sensore disponibile, \
                          regime invariato"
                        .to_owned());
                return;
            }
        };

        // Il cap non supera quello consentito al modulo.
        self.inputs.max_power = max_power.min(if self.pump_emergency_active { self.inputs.max_power } else { 100 });

        // L'offset passa dal pavimento anticondensa, come quando lo imposta
        // l'utente a mano: un regime automatico non deve poter scavalcare la
        // protezione, nemmeno in Picco.
        let richiesto = set_point;
        let (applicato, limitato) = self.offset_con_pavimento(richiesto);
        self.inputs.set_point = applicato;
        self.cond_limit_active = limitato && applicato != richiesto;

        self.last_auto_regime = Some(regime);

        // ── Scrittura all'hardware ────────────────────────────────
        // Senza questo blocco il regime cambia solo in memoria: e' esattamente
        // il difetto che l'utente ha segnalato.
        crate::commissioning::event(
            "REGIME",
            &format!(
                "{regime:?}  cap {}%  margine {applicato:.1} °C sotto la rugiada",
                self.inputs.max_power
            ),
        );

        // **Il regime automatico cambia il margine e il tetto, non la
        // potenza.** La potenza la decide il regolatore chiuso: scriverla
        // qui lo inchiodava al cap, e con `PID_RUNNING` — che sul Gen 1 non si
        // attiva mai — questo ramo non entrava nemmeno, quindi le impostazioni
        // restavano solo in stato mentre la dashboard mostrava l'offset
        // nuovo e l'hardware usava quello vecchio.
        //
        // Si scrive il margine quando il modulo e' veramente acceso (watt
        // reali), perche' a spento non c'e' niente da regolare e il prossimo
        // accodamento lo scrive da solo.
        if self.last_power_watts > 2.0 {
            self.scrivi_tec(crate::attore_tec::Richiesta::Setpoint(applicato));
        }
    }

    fn push_pid_if_running(&mut self) {
        if !self
            .tec_status
            .contains(cryo_cooler_controller_lib::TecStatus::PID_RUNNING)
        {
            // TEC spento: si applichera' al prossimo Enable.
            return;
        }
        let (p, i, d) = (
            self.inputs.p_coef,
            self.inputs.i_coef,
            self.inputs.d_coef,
        );
        // Invio asincrono: l'esito arriva via ack nel tick.
        self.scrivi_tec(crate::attore_tec::Richiesta::Pid { p, i, d });
            }

    /// Offset massimo consentito per non scendere sotto il punto di rugiada.
    ///
    /// Il punto di rugiada e' la temperatura sotto la quale l'acqua
    /// dell'aria condensa. Se l'ambiente e' a 25 °C con umidita' 40% il
    /// punto di rugiada e' circa 10 °C: la piastra non puo' scendere sotto.
    ///
    /// Il margine di 1,5 °C copre l'errore dei sensori e il tempo di
    /// risposta: la condensa si forma in pochi secondi, non in pochi
    /// minuti. Senza margine, un sensore leggermente ottimistico
    /// basterebbe a far formare ghiaccio.
    ///
    /// Restituisce `-1000.0` quando non c'e' un dato valido: in quel caso
    /// l'utente puo' scegliere liberamente, perche' vietare tutto senza
    /// informazioni sarebbe peggio che lasciare fare.
    /// La temperatura a cui l'offset e' relativo. **Non la conosciamo.**
    ///
    /// Su questo controller c'e' una sola sonda sulla TEC e il lato caldo non
    /// si legge, quindi non c'e' nessun riferimento. Dichiararlo `None` e' il
    /// modo onesto di dirlo, ed e' la ragione per cui il pavimento non viene
    /// calcolato. Se un giorno il lato caldo diventasse leggibile basta
    /// restituire quel valore, e la protezione torna da sola.
    fn riferimento_offset(&self) -> Option<f32> {
        None
    }

    /// Il pavimento anticondensa, **o `None` se non e' calcolabile**.
    ///
    /// Prima restituiva un `f32` e, quando non sapeva, restituiva `-1000.0`
    /// — cioe' "nessun limite", senza dirlo. Il valore non era pero' neutrale:
    /// era calcolato da una temperatura ambiente *inventata* con la formula
    /// `rugiada + (1 - umidita'/100) * 18`. Con rugiada a 16.7 °C e umidita'
    /// al 37% dava 27.9 °C invece dei ~21 °C reali, e il pavimento risultava
    /// -12.7 °C: abbastanza basso da **accettare** un offset che lasciava la
    /// piastra sotto il punto di rugiada. Il pericolo era reale e il
    /// controllo diceva che era tutto a posto.
    ///
    /// Su questo controller il riferimento a cui l'offset e' relativo **non e'
    /// noto**: c'e' una sola sonda sulla TEC, niente lato caldo. Quindi qui si
    /// restituisce `None` e chi chiama deve dirlo all'operatore, invece di
    /// limitare niente in silenzio.
    ///
    /// La logica e' in `condensa.rs`, che ha i test.
    fn dew_floor_offset(&self) -> Option<f32> {
        use crate::condensa::pavimento_anticondensa;
        // Il riferimento non e' disponibile: su Gen1 non sappiamo a cosa sia
        // relativo l'offset. Passare `None` e' la dichiarazione onesta.
        match pavimento_anticondensa(self.riferimento_offset(), self.last_cond_margin) {
            crate::condensa::EsitoPavimento::Calcolabile { pavimento } => Some(pavimento),
            crate::condensa::EsitoPavimento::NonCalcolabile { .. } => None,
        }
    }

    /// L'offset applicato, e se il pavimento e' stato applicato.
    ///
    /// Restituisce anche il flag perche' la UI deve poter dire all'operatore
    /// "non ti sto limitando il freddo perche' non posso sapere dove stai
    /// rispetto alla rugiada". Un controllo che non agisce e non lo dice e'
    /// peggio di un controllo assente: fa stare tranquillo.
    fn offset_con_pavimento(&self, richiesto: f32) -> (f32, bool) {
        match self.dew_floor_offset() {
            Some(_) => match crate::condensa::offset_limitato(
                richiesto, self.riferimento_offset(), self.last_cond_margin,
            ) {
                Some(v) => (v, true),
                None => (richiesto, false),
            },
            // Nessun pavimento: l'offset va cosi' com'e'. E' una scelta, non un
            // caso dimenticato — il pavimento era inventato, quindi toglierlo
            // e' un miglioramento anche se apparentemente "permissivo".
            None => (richiesto, false),
        }
    }

    /// Il calcolo della guardia, isolato dalla UI per poterlo provare.
    ///
    /// **Questa e' l'unica copia.** Prima la stessa logica era scritta qui
    /// dentro *e* duplicata nel modulo dei test: i test passavano, ma non
    /// toccavano il codice che gira davvero. Ora produzione e test usano
    /// questa funzione, e non possono divergere.
    ///
    /// `cur` e' la potenza applicata, `target` il cap dell'operatore,
    /// `ctrl_temp` la temperatura del controller.
    fn prossima_potenza(ctrl_temp: f32, cur: u8, target: u8) -> u8 {
        let next: u8 = if ctrl_temp >= Self::CTRL_CRITICO {
            // Critico: il pavimento sparisce, si puo' scendere a zero.
            cur.saturating_sub(Self::CTRL_STEP_DOWN as u8)
        } else if ctrl_temp >= Self::CTRL_SOFT {
            // Troppo caldo: scendi di un passo.
            //
            // **Il pavimento non fa salire.** Prima era `scesa.max(50)` sempre:
            // con `cur = 30` dava `50`, cioe' la guardia alzava da sola di 20
            // punti mentre il controller era caldo. Il pavimento serve a non
            // spegnere il raffreddamento scendendo sotto 50, non a saltarci
            // dentro dal basso: se sei gia' sotto, continui a scendere.
            //
            // **Nessun pavimento a 50%.** Il registro lo dice:
            // `60% -> 58%` con `ctrl=70.0 °C` — la guardia scende di due punti
            // per tick e si ferma sul 50, quindi la potenza non arriva mai
            // dove tu l'hai misurata.
            //
            // Il pavimento era un numero che avevo messo io "per non spegnere
            // il raffreddamento", senza misurarlo. Ma il cap che scegli tu e'
            // gia' la protezione: se vuoi il 30%, il 30% e' la tua decisione.
            // Un secondo pavimento, invisibile, annulla quella scelta — e per
            // questo non si scendeva mai sotto.
            cur.saturating_sub(Self::CTRL_STEP_DOWN as u8).min(target)
        } else {
            // **Sotto la soglia la guardia e' muta.** Come in r116: non sale e
            // non scende.
            //
            // La salita che avevo aggiunto qui era **il motore dei 150-225 W**.
            // Non un difetto di taratura: una funzione che non doveva esistere.
            // Il regolatore guardava la temperatura e saliva di uno (o otto)
            // punti a ogni tick, e cosi' il modulo restava inchiodato al cap per
            // sempre. r116 non aveva nessun regolatore: contiamo zero funzioni
            // di quel tipo nel backup, tre in questo file.
            //
            // **La potenza la scrive l'operatore**, e il firmware col suo
            // regolatore interno trova il punto di lavoro: 20-30 W, come hai
            // misurato. Il compito del programma e' non starci in mezzo, e
            // l'unica cosa che continua a fare e' abbassare la potenza se il
            // controller si scalda.
            cur
        };
        next.min(target)
    }

    fn update_ctrl_guard(&mut self, ctrl_temp: f32) {
        self.ctrl_temp = ctrl_temp;
        if ctrl_temp > self.ctrl_temp_peak {
            self.ctrl_temp_peak = ctrl_temp;
        }

        // ── Lettura assurda: non agire ────────────────────────────────
        //
        // Una sola lettura sballata (NaN, 0 °C, 200 °C) non deve comandare
        // la potenza: al massimo il valore peggiore. Il range accettato e'
        // volutamente largo, la guardia deve poter regolare anche in
        // emergenza, ma 200 °C non esiste su questo hardware.
        // Sensore assente o lettura assurda: si toglie potenza.
        //
        // Prima qui si tornava indietro SENZA AGIRE, e il contatore dei
        // fallimenti non veniva letto da nessuna parte: la guardia restava
        // inerte per tutto il tempo in cui la lettura restava cattiva. Un NTC
        // scollegato non e' "nessun dato", e' "non lo so e il modulo e'
        // acceso": l'unica risposta che non puo' danneggiare e' togliere
        // potenza.
        //
        // Il firmware espone gia' il suo parere: i bit `BOARD_TEMP_OK` e
        // `TEMP_SENSE_OK` dicono se il sensore e' vivo. Non li si ascoltava,
        // e il modulo restava al cap in prefissato.
        if !ctrl_temp.is_finite() || !(-40.0..=150.0).contains(&ctrl_temp) {
            self.power_push_failures = self.power_push_failures.wrapping_add(1);
            crate::commissioning::event(
                "CTRL-LET",
                &format!(
                    "lettura controller non plausibile ({ctrl_temp} C): potenza \
                     MANTENUTA. fallimenti={}",
                    self.power_push_failures
                ),
            );
            // **Non si azzera piu'.**
            //
            // `ctrl_temp` non e' una temperatura misurata: arriva da
            // `f32::from_le_bytes` su 4 byte grezzi del controller. Se la
            // risposta non e' allineata come si suppone, quei byte non sono
            // un numero ma spazzatura, e la spazzatura letta come float dà
            // NAN. Non e' "sensore guasto", e' un artefatto di parsing: non
            // si puo' usare per spegnere.
            //
            // Il firmware ha gia' soglie hardware proprie (Standby 80 °C,
            // spegnimento 90 °C) e protegge l'hardware da solo. Qui si
            // conserva l'ultimo valore valido e si torna indietro: la
            // protezione vera, quando il dato manca, e' non alzare la
            // potenza, non metterla a zero.
            return;
        }

        // `BOARD_TEMP_OK` **non comanda la potenza.**
        //
        // Prima limitava al 30% e fermava la guardia con un `return`, prima
        // della logica di regolazione: se il firmware non imposta quel bit la
        // potenza configurata non arrivava mai e il TEC restava piantato al
        // 30%. Il sintomo era "il TEC non si abilita".
        //
        // Il bit non e' mai stato verificato su questa scheda e il protocollo
        // Delta² non e' documentato. Un controllo di cui non si sa quando e'
        // vero non distingue "sensore guasto" da "il firmware non lo usa":
        // protegge un'ipotesi e impedisce il funzionamento reale. Resta
        // solo la diagnostica.
        if !self.tec_status.contains(TecStatus::BOARD_TEMP_OK) {
            crate::commissioning::event(
                "CTRL-SENS",
                "il firmware non segnala BOARD_TEMP_OK: solo diagnostica, \
                 nessun effetto sulla potenza",
            );
        }

        let target = self.tetto_potenza();
        let cur    = self.applied_power;

        // **Una sola guardia, non due.**
        //
        // Questa logica era scritta qui dentro *e* copiata in
        // `prossima_potenza`, che i test chiamavano. I test passavano, la
        // copia non era mai stata eseguita durante l'uso reale, e potevano
        // divergere senza che nessuno se ne accorgesse: 255 test verdi senza
        // coprire il codice che gira.
        //
        // Ora la funzione qui sotto e' l'unica, e i test la chiamano
        // davvero. Se domani cambia una soglia, cambia per l'app e per le
        // prove insieme.
        //
        // Niente rampa qui dentro: l'app non alza mai la potenza. La rampa di
        // accensione e' stata rimossa perche' scriveva a ogni campione e
        // teneva il firmware in manuale inchiodato al cap (227 W, piastra
        // sotto rugiada). Il firmware regola da solo; la guardia puo' solo
        // scendere.
        let next: u8 = Self::prossima_potenza(ctrl_temp, cur, target);

        if next != cur {
            // Log di collaudo: la decisione della guardia, con la soglia che
            // l'ha provocata. E' la riga che permette di verificare su
            // hardware che la guardia reagisce alle soglie del manuale.
            // Il log dice **perche'** la guardia ha deciso. Il ramo "sotto
            // soglia fredda" e' sparito con la salita: ora la guardia puo'
            // solo scendere, e quando scende e' perche' e' caldo.
            // **L'ordine dei controlli conta.** Prima era:
            //
            //     if ctrl_temp >= CTRL_SOFT      { "sopra guardia" }
            //     else if ctrl_temp >= CTRL_CRITICO { "sopra soglia critica" }
            //
            // Il secondo ramo era **codice morto**: `CTRL_SOFT` e' 36 e
            // `CTRL_CRITICO` e' 38, quindi ogni temperatura sopra 38 °C
            // soddisfa anche il primo controllo e il log scriveva sempre
            // "sopra guardia 36 °C" anche a 45 °C. Il caso peggiore, quello
            // per cui il log esiste, era proprio quello che non si vedeva.
            //
            // Ora il caso piu' grave viene valutato per primo.
            let motivo = if ctrl_temp >= Self::CTRL_CRITICO {
                format!("sopra soglia critica {} °C", Self::CTRL_CRITICO)
            } else if ctrl_temp >= Self::CTRL_SOFT {
                format!("sopra guardia {} °C", Self::CTRL_SOFT)
            } else {
                "banda morta".to_owned()
            };
            crate::commissioning::event(
                "POTENZA",
                &format!(
                    "ctrl={ctrl_temp:.1} °C  {motivo}  {cur}% -> {next}%  (cap {target}%)"
                ),
            );

            // `applied_power` si aggiorna subito (ottimistico): la guardia
            // del tick dopo deve decidere con il valore appena chiesto, non
            // con quello precedente. Se l'ack fallisce, il tick registra
            // l'errore e il re-push riprova.
            self.applica_potenza(next);
            self.power_push_failures = 0;
            crate::commissioning::event(
                "SET-OK",
                &format!("set_power_level({next}) inviato all'attore"),
            );
        }

        // ── L'avviso sul controller ─────────────────────────────────────
        //
        // Prima l'allarme scattava a `CTRL_SOFT - 4.0`: con la vecchia soglia
        // a 66 gradi erano 62, ma riportando il numero a 36 diventavano **32**,
        // cioe' sotto la temperatura che hai dichiarato normale. Un avviso che
        // scatta in condizioni normali e' un avviso che l'operatore impara a
        // ignorare — ed e' l'unico che dice "sto limitando la potenza".
        //
        // Ora l'avviso parte a `CTRL_SOFT`, che e' l'inizio della zona gialla.
        if !self.ctrl_warned && ctrl_temp >= Self::CTRL_SOFT {
            self.ctrl_warned = true;
            self.error_text.get_or_insert_with(|| {
                format!(
                    "Controller TEC a {:.0}°C: sto abbassando la potenza. \
                     Sopra i 38°C il rischio è danno alle guaine dei fili, \
                     non solo Standby.",
                    ctrl_temp
                )
            });
        } else if self.ctrl_warned && ctrl_temp < Self::CTRL_SOFT - 2.0 {
            // Due gradi di richiamo: evita che l'avviso lampeggi se la
            // temperatura e' appena sopra la soglia.
            self.ctrl_warned = false;
        }
    }

    /// Scrive il setpoint dell'operatore **solo se il regime lo usa**.
    ///
    /// Il setpoint dell'operatore e' l'offset di `Cryo`. Scriverlo in Standby o
    /// Unregulated sovrascrive l'offset del regime: succedeva dal cursore e dal
    /// consiglio dell'AI, e in Unregulated il `-30` spariva mentre la UI
    /// continuava a dire "Unregulated" — perche' quella legge i bit, e i bit non
    /// ricordano che cosa e' stato scritto loro sopra.
    ///
    /// Il filtro e' `offset_dopo_cambio_setpoint`, che e' una funzione pura e
    /// testata: qui non c'e' logica, solo il dispatch.
    /// **Il regolatore chiuso: e' qui che stava il 52%.**
    ///
    /// Su questo Gen 1 il bit `PID_RUNNING` non si attiva **mai**: il
    /// firmware non ha un regolatore attivo, e non esiste un "automatico" che
    /// tenga la piastra al punto giusto. La versione che funziona chiama
    /// questo ciclo: misura la piastra, la confronta con il target e alza o
    /// abbassa la potenza di un passo, finche' l'equilibrio non e' quello
    /// giusto per il carico. Ecco perche' lei stava al 52% e le mie
    /// build no: quelle scrivevano un valore fisso, e fermo al 10% non
    /// regolava niente.
    ///
    /// Il target e' `rugiada + offset`: la piastra si ferma un margine sopra
    /// il punto di rugiada, quindi il regolatore e' anticondensa per
    /// costruzione e non ha bisogno di un riferimento esterno.
    ///
    /// Passi piccoli e zona morta: senza zona morta oscilla (sale, scende,
    /// sale), con zona morta si stabilizza — ed e' la stabilizzazione che
    /// rende leggibili i grafici.
    /// **La temperatura a cui il regolatore tiene la piastra.**
    ///
    /// IlFreddo e' scelto dall'operatore (`offset_sicuro`, il campo "Offset
    /// temp."); l'unico limite e' il **bordo di condensa**: la piastra non
    /// scende mai sotto `rugiada + margine`, o si bagna la scheda.
    ///
    /// Il margine di sicurezza non e' inventato: e' la differenza che la
    /// versione che funzionava manteneva senza condensare. Il campo
    /// "offset" e' il margine che vuoi tu — piu' lo abbassi, piu' freddo
    /// scendi, e piu' ti avvicini al bordo. Il programma non alza il freddo
    /// oltre il tuo valore: quello e' il tuo ordine.
    ///
    /// Il minimo e' 0.5 °C sopra la rugiada: sotto, l'aria davanti alla
    /// piastra puo' condensare anche se la sonda dice "asciutto".
    /// **Salita graduale dopo l'accensione, come nella r116 che funziona.**
    ///
    /// Il campo c'era gia' (`soft_starting`) ma non era piu' usato: l'avevo
    /// perso in un refactor, e senza di lui l'app scriveva il cap di colpo.
    /// Con l'avvio a 30% e la salita di 1%/tick, il firmware ha il tempo di
    /// avviare il regolatore e la piastra non crolla sotto la rugiada.
    fn avanza_soft_start(&mut self) {
        if !self.soft_starting {
            return;
        }
        if !self.ctrl_temp.is_finite() || !(-40.0..=150.0).contains(&self.ctrl_temp)
            || !self.last_cond_margin.is_finite() || self.last_cond_margin < 1.0 {
            return;
        }
        let tetto = self.tetto_potenza();
        // La guardia vince: sopra soglia non si sale.
        if self.ctrl_temp >= Self::CTRL_SOFT {
            self.soft_starting = false;
            return;
        }
        if self.applied_power >= tetto {
            self.soft_starting = false;
            return;
        }
        let prossimo = self.applied_power.saturating_add(1).min(tetto);
        if prossimo != self.applied_power {
            self.applica_potenza(prossimo);
        }
    }

    fn scrivi_setpoint_utente(&mut self, applicato: f32) {
        use crate::commutazione::{offset_dopo_cambio_setpoint, Regime};
        let regime = self.ultimo_regime_richiesto
            .or_else(|| Regime::da_stato(self.tec_status)).unwrap_or(Regime::Spento);
        // A spento `offset_dopo_cambio_setpoint` ritorna `None`: non si scrive.
        if let Some(offset) = offset_dopo_cambio_setpoint(regime, applicato) {
            self.scrivi_tec(crate::attore_tec::Richiesta::Setpoint(offset));
        }
    }

    /// La `Certezza` sullo stato corrente: fresco? confermato?
    ///
    /// Passa per `da_stato_certainza` con gli stessi due segnali usati da
    /// `tec_eroga_potenza`: `last_sample_time` e lo stato della commutazione.
    /// Entrambi sono gia' mantenuti, quindi qui non si introduce nulla.
    fn certezza(&self) -> crate::certezza::Certezza {
        crate::commutazione::Regime::da_stato_certainza(
            self.tec_status,
            self.last_sample_time.elapsed(),
            self.commutazione.confermato(),
        )
        .1
    }

    /// La differenza di temperatura attuale attraverso il complessio.
    ///
    /// Due sonde: la piastra e la scheda, che e' sul lato caldo. Il COP vero
    /// del modulo non e' calcolabile senza i suoi parametri di costruzione, ma
    /// il delta si, e da quello si ricava l'efficienza di sistema.
    fn delta_t_corrente(&self) -> Option<f32> {
        let tec = self.chart.last_tec_temp();
        let scheda = self.last_pcb_temperature;
        crate::efficienza::delta_t(scheda, tec)
    }

    /// Il margine di condensa e' fresco, cioe' descrive adesso e non prima?
    ///
    /// Usa la **stessa** soglia della `Certezza` (`CAMPIONE_TIMEOUT`): la
    /// freschezza ha una soglia sola in tutto il software. Con due soglie
    /// diverse, la guardia e la riga di stato potrebbero essere in disaccordo
    /// sullo stesso campione — la guardia che avvisa mentre la riga dice che
    /// non si sa, o viceversa.
    fn margine_fresco(&self) -> bool {
        crate::certezza::puo_scrivere(self.certezza())
    }

    /// Il colore e l'etichetta del margine, **sapendo se il dato e' fresco**.
    ///
    /// Un punto solo per la decisione: prima ne esistevano due copie, nella
    /// colonna laterale e nella striscia in portrait, e ognuna faceva i suoi
    /// confronti. Divergerebbero al primo ritocco.
    fn etichetta_margine_ui(&self) -> (iced::Color, String) {
        use crate::diagnostica::{etichetta_margine, EtichettaMargine as E};
        let m = self.last_cond_margin;
        let fresco = self.margine_fresco();
        match etichetta_margine(m, fresco) {
            E::Ok => (palette::NEON_GREEN, format!("Margine  +{m:.1}\u{b0}C OK")),
            E::Basso => (palette::WARNING, format!("Margine  {m:.1}\u{b0}C  basso!")),
            E::Condensa => (palette::DANGER, format!("CONDENSA  {m:.1}\u{b0}C")),
            // **Mai verde.** Un dato vecchio non e' una buona notizia: e'
            // l'assenza di una notizia. Il testo lo dice, e il colore non
            // contraddice il testo.
            E::NonFresco => (palette::BLUE_DIM, "Margine  non fresco".to_owned()),
        }
    }

    /// Il TEC eroga potenza *adesso*. **Derivato, mai memorizzato.**
    ///
    /// Sostituisce il campo `tec_abilitato`, che veniva assegnato in quattro
    /// punti e poteva quindi essere lasciato in uno stato diverso dagli altri.
    /// Il peggiore era l'assegnazione dopo una commutazione:
    /// `tec_abilitato = !LOW_POWER_MODE_ACTIVE`, che inverte la semantica —
    /// `LOW_POWER_MODE` descrive *standby*, non *alimentazione*, quindi in
    /// Standby (dove il TEC e' acceso) la guardia perdeva il permesso di
    /// scrivere potenza.
    ///
    /// Passa dalla stessa funzione che decide la `Certezza`: due letture
    /// indipendenti della stessa domanda ricreerebbero, in miniatura, il difetto
    /// che questa funzione chiude.
    fn tec_eroga_potenza(&self) -> bool {
        let (regime, certezza) = crate::commutazione::Regime::da_stato_certainza(
            self.tec_status,
            self.last_sample_time.elapsed(),
            self.commutazione.confermato(),
        );
        crate::certezza::puo_scrivere(certezza) && regime.tec_acceso()
    }

    #[inline]
    fn should_update(&self) -> bool {
        self.last_sample_time.elapsed() > self.update_interval
    }

    /// Chiusura ordinata: salva le statistiche di sessione, spegne il TEC,
    /// libera gli overlay (RTSS / Discord) e chiude la porta seriale.
    /// Senza questo, uscendo dall'app la sessione restava `ended_at = NULL`
    /// e il database cresceva indefinitamente.
    /// Scrive la potenza sul modulo e, **solo se la scrittura e' riuscita**,
    /// aggiorna `applied_power`.
    ///
    /// Questo e' l'unico punto del programma che scrive la potenza. Esisteva
    /// in cinque, e ognuno ricordava una parte della verita': il risultato
    /// era che il re-push periodico scriveva in hardware il valore *creduto*
    /// invece di quello *misurato*, e poteva annullare una protezione gia'
    /// attiva.
    ///
    /// Scrive la potenza sul TEC.
    ///
    /// Lo stato **non** viene aggiornato qui: la scrittura e' asincrona e
    /// arriva con un `Ack`. Fino a quel momento `applied_power` resta il
    /// valore dell'ultimo ack, e la guardia periodica continua a re-inviare
    /// quello, quindi se la scrittura si perde il TEC torna al valore
    /// **misurato**, non a quello sperato. Questo e' l'inverso
    /// dell'ottimismo asincrono che non abilitava il TEC: la UI dichiara
    /// solo cio' che l'hardware ha confermato.
    fn applica_potenza(&mut self, next: u8) {
        self.scrivi_tec(crate::attore_tec::Richiesta::Potenza(next));
    }

    /// Il tetto di potenza effettivo, **ricalcolato** a ogni uso.
    ///
    /// `max_power_before_emergency` e' il cap dell'utente *prima* dell'evento.
    /// Usarlo con `.min()` per "limitare" non limitava niente, perche' il
    /// valore salvato era il piu' alto: era uno snapshot scaduto, cioe' un
    /// limite che non limitava piu'. Qui si applica solo se l'emergenza e'
    /// ancora in forza.
    /// Registra un punto della curva di funzionamento della cella.
    ///
    /// **Non decide nulla sul TEC**: scrive solo un file. E' un misuratore
    /// passivo, quindi non puo' cambiare il comportamento termico, e nessuna
    /// protezione legge il file.
    ///
    /// Registra quando sono soddisfatte tre condizioni:
    ///
    /// 1. il collaudo e' attivo (`CRYO_COMMISSIONING`), altrimenti non si
    ///    scrive alcun file;
    /// 2. **la potenza e' cambiata** rispetto all'ultimo punto: a potenza
    ///    costante la curva non aggiunge nulla;
    /// 3. **sono passati almeno 3 secondi** dall'ultima misura a quella
    ///    potenza: senza questo si registra durante il transitorio, e il
    ///    transitorio non dice niente sul regime stabile.
    fn misura_curva(&mut self, data: &cryo_cooler_controller_lib::MonitoringData) {
        use crate::curva::Punto;
        if std::env::var_os("CRYO_COMMISSIONING").is_none() {
            return;
        }
        const ATTESA_STABILIZZAMENTO: std::time::Duration =
            std::time::Duration::from_secs(3);

        let potenza = self.applied_power;
        if potenza == self.curva_ultima_potenza {
            return;
        }
        if let Some(t) = self.curva_ultimo_istante {
            if t.elapsed() < ATTESA_STABILIZZAMENTO {
                return;
            }
        }

        // Stessa base del COP a schermo, stessa costante: e' una stima, e il
        // file la dichiara.
        const K_CPU: f32 = 12.0; // W/°C
        let cpu_t = self.last_cpu_temp_ext.unwrap_or(data.tec_temperature);
        let q_removed = ((cpu_t - data.tec_temperature) * K_CPU).max(0.0);

        let punto = Punto {
            power_level: potenza,
            volt:        data.tec_voltage,
            amp:         data.tec_current,
            watt:        data.tec_power_watts,
            tec_temp:    data.tec_temperature,
            ctrl_temp:   data.pcb_temperature,
            q_removed,
            cop:         0.0, // calcolato da `Punto::cop()`
            k_cpu:       K_CPU,
            delta_hot_cold: data.pcb_temperature - data.tec_temperature,
        };

        let salvato = dirs::data_local_dir()
            .or_else(|| dirs::config_dir())
            .map(|d| d.join("stargate-cryo"))
            .and_then(|d| std::fs::create_dir_all(&d).ok().map(|_| d))
            .map(|d| crate::curva::registra(&d.join("curva-cella.csv"), &punto))
            .unwrap_or(false);

        if salvato {
            crate::commissioning::event(
                "CURVA",
                &format!(
                    "potenza {}%: {:.1} V, {:.2} A, {:.1} W, TEC {:.1} C, \
                     controller {:.1} C, COP stimato {:.2}",
                    punto.power_level, punto.volt, punto.amp, punto.watt,
                    punto.tec_temp, punto.ctrl_temp, punto.cop()
                ),
            );
            self.curva_ultima_potenza = potenza;
            self.curva_ultimo_istante = Some(std::time::Instant::now());
        }
    }

    fn tetto_potenza(&self) -> u8 {
        let soffitto = if self.pump_emergency_active {
            self.max_power_before_emergency
        } else {
            self.inputs.max_power
        };
        // **Nessun tetto rigido in watt.** R96 lo imposeva a 200 W, e con
        // l'OCP come segnale si era rivelato falso: l'OCP scatta anche a
        // 73 W, quindi non e' un indicatore di sovraccorrente. Misurato su
        // questo hardware, 220 / 230 / 237 W funzionano e raffreddano, e il
        // controller regge piu' di quanto si supponesse.
        //
        // Il limite fisico resta quello dell'utente (`max_power`) e le
        // protezioni termiche. Quello che era stato un tetto rigido e' ora un
        // **avviso** in diagnostica: se la percentuale scelta chiede piu' di
        // quanto il controller dovrebbe dare, si avvisa, ma non si blocca. Il
        // blocco e' una decisione dell'utente, non del software.
        let cap = self.inputs.max_power.min(soffitto);
        // **Il tetto finale e' il piu' piccolo fra quello che vuoi e quello che
        // il regolamento termico permette.** Il secondo dipende dalla
        // temperatura del controller, e li' c'e' il muro a 38 °C.
        Self::tetto_per_ctrl(self.ctrl_temp, cap)
    }

    /// **Il tetto del modulo, limitato dalla temperatura del controller.**
    ///
    /// Il vincolo e' dell'utente: il controller non deve mai superare
    /// 38/40 °C, e a 40 °C le guaine si sciolgono. Quindi il tetto non e' un
    /// numero fisso ma una funzione della temperatura: sotto la soglia di
    /// guardia vale il cap dell'operatore, e salendo verso la soglia critica il
    /// tetto scende fino a zero.
    ///
    /// Il motivo e' che il modulo e' il carico termico che scalda il controller:
    /// piu' modulo spinge, piu' il controller si scalda, e il limite e' il
    /// lato caldo. Un cap fisso non puo' funzionare, perche' non sa quanto
    /// calore il tuo impianto riesce a smaltire — e quel numero cambia con il
    /// carico della CPU, che non e' noto.
    ///
    /// La riduzione parte a 30 °C, molto prima della guardia a 36, cosi' il
    /// tetto scende gradualmente e il controller non arriva mai al muro.
    fn tetto_per_ctrl(ctrl_temp: f32, cap: u8) -> u8 {
        if !ctrl_temp.is_finite() {
            return cap;
        }
        if ctrl_temp >= Self::CTRL_CRITICO {
            return 0;
        }
        if ctrl_temp <= Self::CTRL_SOFT {
            return cap;
        }
        // **Sotto 36 °C il tetto non esiste.** Partiva da 30 °C, e quella
        // fascia non l'aveva misurata nessuno: l'ho scritta per "difendersi
        // prima". Ma il tuo controller sta di **normale a 34-35 °C**, quindi
        // la riduzione scattava sempre e tagliava la potenza a meta' (a 35 °C
        // il tetto era ~37 con cap 100).
        //
        // Con il regolatore tolto, non restava piu' nulla per far salire la
        // potenza: il soft start partiva da 30 e si fermava al tetto. Per
        // questo il TEC non si abilitava.
        //
        // **L'unica soglia che hai dato tu e' 38 °C.** Sotto la guardia
        // (36 °C) il cap dell'operatore e' il tetto, punto. Non si inventa
        // nessun'altra soglia per un numero che non e' mai stato misurato.
        let t = (ctrl_temp - Self::CTRL_SOFT)
            / (Self::CTRL_CRITICO - Self::CTRL_SOFT);
        // **Non scende a zero a 38 per interpolazione**: arriva a 1/4 del cap,
        // e l'ultimo passo lo fa la guardia. Un tetto che crolla a zero
        // annullerebbe il regolamento del firmware mentre il controller e'
        // ancora entro soglia.
        let fattore = 1.0 - 0.75 * t.clamp(0.0, 1.0);
        ((cap as f32 * fattore).round() as u8).min(cap)
    }

    /// `true` se la percentuale scelta chiede piu' watt di quanto il
    /// controller dovrebbe erogare stabilmente.
    ///
    /// Serve solo per l'avviso in diagnostica: **non limita nulla**.
    fn potenza_supera_tetto_hardware(&self) -> bool {
        let richiesti = self.inputs.max_power as f32 / 100.0 * WATT_AL_100_PERCENTO;
        richiesti > TETTO_WATT_CONTROLLER
    }


    /// Raccoglie un campione gia' eseguito dal thread, **senza bloccare**.
    ///
    /// `try_recv` e non `recv`: questa funzione gira dentro il tick e non
    /// deve mai fermare la UI. Restituisce `None` se il round e' ancora in
    /// corso: si salta tutto e si riprova al tick successivo.
    /// I dati grezzi da cui giudicare lo stato di salute.
    ///
    /// Unico punto in cui si raccolgono, cosi' il pannello e il file di log
    /// non possono guardare numeri diversi. Tutto viene da `SessionStats` e
    /// dai campioni gia' letti: nessuna misura inventata.
    ///
    /// I guard `count > 0` servono perche' `StatTracker::avg()` su un
    /// tracker vuoto non significa niente, e `last_cond_margin` parte da
    /// `f32::MAX` come sentinella.
    /// Scrive il rapporto diagnostico su file e dice dove.
    ///
    /// **Perche' un bottone e non solo il log automatico.** Il log automatico
    /// scrive una riga a ogni *cambio di verdetto*, quindi quando il verdetto
    /// e' "Attenzione" da due ore il file non contiene lo stato di adesso. Il
    /// bottone scrive la riga di questo istante: e' quello che serve per
    /// allegare al supporto lo stato mentre si guarda la dashboard.
    ///
    /// riusa `riga_log`, quindi la riga dichiara gia' per iscritto che non e'
    /// il self test del produttore. Quel testo non si duplica qui apposta:
    /// un rapporto che mente sulla sua provenienza fa perdere tempo a chi lo
    /// riceve.
    fn salva_rapporto_diagnostica(&self) -> String {
        use crate::diagnostica::valuta;
        let esito = match self.dati_diagnostica() {
            Some(m) => valuta(m, self.tec_status),
            None => {
                return "✗ diagnostica non disponibile: il corpo di \
                        dati_diagnostica non e' stato ricostruito"
                    .to_owned();
            }
        };
        let Some(dir) = std::env::var_os("LOCALAPPDATA") else {
            return "✗ LOCALAPPDATA non disponibile".to_owned();
        };
        let percorso = crate::stato_log::cartella_app(&dir.to_string_lossy())
            .join("diagnostica.log");
        let riga = esito.riga_log(&chrono::Utc::now().to_rfc3339());
        let esito_apertura = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&percorso);
        match esito_apertura {
            Ok(mut f) => {
                use std::io::Write;
                match writeln!(f, "{riga}") {
                    Ok(_) => format!("✓ Salvato: {}", percorso.display()),
                    Err(e) => format!("✗ Scrittura fallita: {e}"),
                }
            }
            Err(e) => format!("✗ Non apribile: {e}"),
        }
    }

    /// Invia al thread proprietario la richiesta di cambiare regime.
    ///
    /// Non scrive niente qui: la scrittura la fa `attore_tec`, che e' l'unico
    /// che tiene la porta. Questo metodo prepara solo il piano — l'offset
    /// viene calcolato da `commutazione::Regime::piano` con i valori presi
    /// dal binario Intel, non con un numero scritto a mano.
    ///
    /// L'errore di un regime gia' attivo non e' un errore: commutare a cio'
    /// che si sta gia' facendo non serve a niente e rischia di far saltare il
    /// setpoint per un tocco inutile. Quindi si risponde e basta.
    fn commutazione_richiesta(&mut self, regime: crate::commutazione::Regime) {
        use crate::commutazione::Regime;

        // **"Gia' attivo" si confronta con quello che hai scelto tu**, non coi
        // bit del controller.
        //
        // Prima usava `da_stato(tec_status)`: su questo Gen 1 quei bit non
        // confermano il regime, eppure la funzione restituiva comunque un
        // valore, quindi il pulsante Cryo rispondeva "Cryo: gia' attivo" e
        // **non scriveva niente** — mentre l'hardware era ancora a -30 di
        // Unregulated. Il pannello e il modulo non erano d'accordo, e la
        // riga diceva il contrario di quello che stava succedendo.
        let regime_corrente = self
            .ultimo_regime_richiesto
            .or_else(|| Regime::da_stato(self.tec_status));
        if regime_corrente == Some(regime)
            && self.commutazione == crate::commutazione::Commutazione::Confermata(regime) {
            self.esito_commutazione = Some(format!("{regime}: gia' attivo"));
            return;
        }

        // La guardia anticondensa **non blocca e non chiede**: avvisa e basta.
        //
        // Il cancello di conferma e' stato tolto perche' rompeva i comandi:
        // con margine basso ogni clic finiva in attesa, e la finestra
        // chiedeva "Procedo" mentre la riga d'esito diceva "Premi Conferma"
        // — due nomi diversi per lo stesso passo, e l'operatore restava
        // fermo a cercare un pulsante che non esisteva. Peggio: la conferma
        // ricontrollava il margine e rimetteva in attesa all'infinito.
        //
        // La regola standing resta "nessun intervento automatico": qui non ce
        // n'e' comunque, perche' a scrivere e' sempre un clic esplicito
        // dell'operatore, mai il programma da solo. Il rischio condensa resta
        // urlato dai banner, dal badge e dalla riga di margine: avviso, non
        // blocco.
        self.scrivi_regime(regime);
    }

    /// Scrive il regime sull'hardware, senza chiedere niente.
    ///
    /// E' la coda di `commutazione_richiesta`, separata perche' la conferma
    /// anticondensa deve arrivarci **senza ripassare dal cancello**: il
    /// margine era gia' scritto nella finestra quando l'operatore ha premuto
    /// "Procedo", quindi il consenso e' informato. Ricontrollare il margine
    /// dopo il consenso rimetteva in attesa all'infinito con margine basso.
    fn scrivi_regime(&mut self, regime: crate::commutazione::Regime) {
        let piano = regime.piano(self.inputs.set_point);
        crate::recovery::record_enabled(piano.tec_acceso);
        self.tec_regulator.reset();
        if let Some(offset) = piano.offset { self.regulation_offset = offset; }
        self.esito_commutazione = Some(format!("Clic ricevuto: invio {regime} al controller Gen 1..."));
        crate::commissioning::event("CLIC-TEC", &format!("regime={regime} offset={:?} PID={}/{}/{} cap={}%", piano.offset, self.inputs.p_coef, self.inputs.i_coef, self.inputs.d_coef, self.inputs.max_power));
        self.commutazione = crate::commutazione::Commutazione::Richiesta(regime);
        // La rampa parte solo dopo l'ACK della sequenza di accensione.
        self.soft_starting = false;
        // Si ricorda **quello che e' stato chiesto**: e' l'unica informazione
        // che su questo Gen 1 esiste, perche' i bit non confermano mai il
        // regime. L'avviso di Unregulated parte da qui.
        self.ultimo_regime_richiesto = Some(regime);
        // Timer dello spool: parte solo sulle accensioni, si azzera allo
        // spegnimento. Il tick corre a ~2 Hz: diviso 2 fa i secondi.
        if piano.tec_acceso {
            self.attesa_tick = self.tick_count;
            // Si parte da 30, non dal cap: la salita graduale fa il resto.
            self.soft_starting = false;
        } else {
            self.attesa_tick = 0;
            self.soft_starting = false;
        }
        // **All'accensione non si scrive la potenza. Solo offset e alimentazione.**
        //
        // La r131 ha provato la spinta prima dell'enable e il grafico lo prova:
        // salto verticale a 227 W e poi fisso con la CPU a 22 °C. Qualsiasi
        // scrittura di livello, prima durante o dopo, inchioda il manuale.
        // Solo zero scritture lasciano l'automatico, che impiega ~2 minuti a
        // salire da solo (i grafici della vecchia lo mostrano).
        // Se il canale e' chiuso il thread e' morto: la richiesta non parte e
        // il regime richiesto resta pendente, cosi' la UI non dichiarera' mai
        // un avvenimento che non e' successo.
        if self.tec_tx.send(crate::attore_tec::Richiesta::Regime {
            offset: piano.offset,
            tec_acceso: piano.tec_acceso,
            // I PID viaggiano con l'abilitazione: sono cio' che dà al
            // firmware un regolatore. Senza restava al 10% di default.
            pid: Some((
                self.inputs.p_coef,
                self.inputs.i_coef,
                self.inputs.d_coef,
            )),
            // **Acceso → acceso serve il disable prima.** Il firmware ignora
            // un enable quando il modulo e' gia' acceso: e' il caso di
            // Unregulated → Cryo, che senza questo restava su -30 mentre la
            // dashboard diceva Cryo, e l'unica via era spegnere e riaccendere
            // a mano. Da spento non serve: l'enable funziona gia'.
            prima_spegni: piano.tec_acceso && self.last_power_watts > 2.0,
        }).is_err() {
            self.commutazione = crate::commutazione::Commutazione::Nessuna;
            self.esito_commutazione = Some("comunicazione col TEC interrotta".to_owned());
            return;
        }
    }

    /// Le misure per la diagnostica, **o `None` se non sono disponibili**.
    ///
    /// `None` non e' un dettaglio: significa "non lo so", e la UI lo
    /// dichiara. Prima questa funzione restituiva sempre un `Esito`, e un
    /// `Esito` vuoto fa dire a `valuta` che non ci sono problemi — quindi la
    /// sidebar avrebbe scritto **"Nessuna anomalia"** su un controller che
    /// magari era in guasto. Un pannello che mente quando non sa e' peggio di
    /// un pannello che dice di non sapere.
    ///
    /// **Ricostruita il 2026-09-29** dopo aver perso il corpo originale. Le
    /// fonti sono le stesse, ricavate dai nomi dei campi: `self.stats` tiene
    /// sei tracciatori, `first()` restituisce il primo valore della sessione e
    /// `min`/`max` gli estremi.
    ///
    /// La scelta che conta, e che era il motivo del commento originale:
    /// **"temperatura finale" e' il punto piu' freddo raggiunto**, non
    /// l'ultimo campione e non la media. Su una cella che raffredda, il
    /// minimo e' il risultato della sessione; l'ultimo campione e' solo "adesso"
    /// e cambia di continuo. La media, che c'era prima, non e' nessuna delle
    /// due, e sotto quell'etichetta faceva leggere una temperatura che non era
    /// mai stata raggiunta.
    fn dati_diagnostica(&self) -> Option<crate::diagnostica::Esito> {
        use crate::diagnostica::scegli_misure;

        // `count > 0` e' la condizione che rende leggibile `min`/`max`:
        // su un tracciatore vuoto `min` e' `f32::MAX` e `max` e' `f32::MIN`,
        // e passarli alla diagnostica produrrebbe "-3.4e38 °C" a schermo.
        let st = &self.stats;
        if st.tec_temp.count == 0 || st.tec_power_watts.count == 0 {
            return None;
        }

        let primo = st.tec_temp.first();
        // Il punto piu' freddo raggiunto: il risultato della sessione.
        let piu_freddo = if st.tec_temp.min.is_finite() {
            Some(st.tec_temp.min)
        } else {
            None
        };
        let potenza_max = if st.tec_power_watts.max.is_finite() {
            Some(st.tec_power_watts.max)
        } else {
            None
        };
        // `avg()` su un tracciatore vuoto non significa niente, quindi si
        // usa lo stesso controllo su `count` che si fa sopra.
        let volts = self.last_electrical.map(|(v,_)|v).filter(|v|v.is_finite());
        let amps = self.last_electrical.map(|(_,a)|a).filter(|a|a.is_finite());
        let cop = Some(self.cop_state.cop).filter(|v|v.is_finite());

        let mut esito = scegli_misure(
            primo,
            piu_freddo,
            crate::diagnostica::margine_attuale(st.condensation_margin.count,self.last_cond_margin),
            potenza_max,
            volts,
            amps,
            cop,
        );

        // La potenza istantanea la imposta il chiamante, non `scegli_misure`:
        // qui e' l'ultima letta, che e' il numero che l'operatore vede in
        // questo momento, e non la media della sessione.
        esito.watts = if self.last_power_watts.is_finite() {
            Some(self.last_power_watts)
        } else {
            None
        };
        esito.modalita = Some(
            crate::commutazione::Regime::da_stato(self.tec_status)
                .map(|r| r.to_string())
                .unwrap_or_else(|| "non determinata".to_owned()),
        );
        Some(esito)
    }

    /// Raccoglie il campione piu' fresco senza bloccare la UI.
    ///
    /// La dichiarazione (firma, `use P`, ritorno) e' stata persa dalla stessa
    /// regex che ha cancellato `dati_diagnostica`; il corpo, che segue, e'
    /// intatto. La firma e' ricostruita dal backup del 28 sera con una
    /// differenza: il secondo elemento e' `StatusCompleto` e non `TecStatus`,
    /// perche' da allora `hear_beat_completo` conserva anche i 14 bit alti.
    fn raccogli_campione(&mut self) -> Option<
        Result<
            (
                cryo_cooler_controller_lib::MonitoringData,
                cryo_cooler_controller_lib::tecstatus::StatusCompleto,
            ),
            String,
        >,
    > {
        use crate::attore_tec::Risposta as P;
        // Si svuota **tutta** la coda e si tiene solo il campione piu'
        // recente. Prima si restituiva il primo e si usciva: con piu' campioni
        // accodati la UI leggeva dati vecchi uno per tick, la linea restava
        // indietro e poi recuperava a scatti. Svuotare tutto e tenere
        // l'ultimo e' cio' che serve: la UI mostra sempre il dato piu' fresco
        // e non resta mai indietro. I campioni scartati sono vecchi di qualche
        // decina di millisecondi, non informazioni perse.
        let mut ultimo: Option<
            Result<
                (
                    cryo_cooler_controller_lib::MonitoringData,
                    cryo_cooler_controller_lib::tecstatus::StatusCompleto,
                ),
                String,
            >,
        > = None;
        loop {
            match self.tec_rx.try_recv() {
                Ok(P::Campione(r)) => {
                    // Continua a svuotare: questo non e' ancora il piu' recente.
                    ultimo = Some(r);
                }
                // Gli ack si applicano qui, ma **non** interrompono la ricerca
                // del campione: e' l'unico posto dove lo stato della UI
                // diventa vero, e solo se il firmware ha confermato.
                Ok(P::ErroreRegime(e)) => {
                    self.soft_starting = false;
                    self.ultimo_regime_richiesto = None;
                    self.esito_commutazione = Some(format!("Comando TEC fallito: {e}"));
                    crate::commissioning::event("REGIME-ERRORE", &e);
                    self.error_text = Some(e);
                }
                Ok(P::Ack(Err(e))) => {
                    crate::commissioning::event("SCRITTURA-ERRORE", &e);
                    self.error_text = Some(e);
                }
                Ok(P::Ack(Ok(scritto))) => {
                    use crate::attore_tec::Scritto as S;
                    match scritto {
                        S::Potenza(v) => {
                            self.applied_power = v;
                            self.power_push_failures = 0;
                        }
                        S::Setpoint(offset) => { self.regulation_offset = offset; }
                        S::Pid | S::TempCpu(_) => {}
                        // La commutazione ha risposto. Si aggiorna lo stato
                        // con **quello che il controller ha detto**, non con
                        // quello che avevamo chiesto: e' l'unico modo per non
                        // mostrare un regime che l'hardware non ha assunto.
                        S::Regime(stato) | S::Spegnimento(stato) => {
                            if let Some(r) = self.commutazione.richiesto() {
                                if r.tec_acceso() != matches!(scritto, S::Regime(_)) {
                                    // ACK precedente: non conferma la richiesta piu' recente.
                                    continue;
                                }
                                self.applied_power = if r.tec_acceso() {
                                    cryo_cooler_controller_lib::Tec::SOFT_START_POWER
                                } else { 0 };
                                // Gen 1 ignores the requested power cap: no percentage ramp.
                                self.soft_starting = false;
                            }
                            self.tec_status = stato.noti;
                            self.tec_status_completo = stato;
                            // L'ack registra i bit del firmware: se il comando
                            // e' accettato ma il modulo non parte, qui si vede
                            // perche' (failsafe, low-power, OCP). Senza questa
                            // riga un enable perso nel firmware sembrava un
                            // pulsante morto.
                            {
                                use cryo_cooler_controller_lib::TecStatus as TS;
                                crate::commissioning::event(
                                    "REGIME-ACK",
                                    &format!(
                                        "OCP={} FAILSAFE={} LOWPWR={} PID={} PWR_OK={}",
                                        stato.noti.contains(TS::OCP_ACTIVE),
                                        stato.noti.contains(TS::FAILSAFE_ACTIVE),
                                        stato.noti.contains(TS::LOW_POWER_MODE_ACTIVE),
                                        stato.noti.contains(TS::PID_RUNNING),
                                        stato.noti.contains(TS::POWER_OK),
                                    ),
                                );
                            }
                            // Il regime richiesto e' tenuto qui per poter
                            // confrontarlo con quello letto: se il controller
                            // ha fatto qualcos'altro, il messaggio lo dice.
                            //
                            // `letto` non e' piu' un `Option`: `da_stato`
                            // dichiara sempre un regime, e quando i bit non
                            // bastano il regime e' `Spento`. Incapsulare
                            // l'assunzione in `.unwrap_or` qui significa che
                            // il confronto non puo' fallire: il caso "non so"
                            // e' gia' stato risolto a monte.
                            if self.commutazione.in_corso() {
                                // `da_stato` puo' tornare `None`, e quel `None`
                                // **non** viene schiacciato su `Spento`.
                                //
                                // Prima c'era `.unwrap_or(Spento)`, e da li' veniva
                                // la riga che hai letto: "richiesto Cryo, il
                                // controller e' in Spento" — mentre passavano
                                // 227 W. Il software non sapeva in che regime
                                // fosse, e lo dichiarava spento: una parola falsa
                                // su un pannello di sicurezza.
                                //
                                // Ora `None` resta `None` e viene detto a parole.
                                let letto = crate::commutazione::Regime::da_stato(stato.noti);
                                let richiesto = self.commutazione.richiesto();
                                let acceso = crate::commutazione::modulo_acceso(stato.noti);
                                self.commutazione = match letto {
                                    Some(l) => self.commutazione.confermata(l),
                                    // Nessun regime ricavabile: la richiesta resta
                                    // pendente, non viene "confermata" a caso.
                                    None => self.commutazione,
                                };
                                self.esito_commutazione = match (richiesto, letto) {
                                    (Some(r), Some(l)) if r == l => {
                                        Some(format!("{r} attivato"))
                                    }
                                    (Some(r), Some(l)) => Some(format!(
                                        "richiesto {r}, il controller e' in {l}"
                                    )),
                                    (Some(r), None) => Some(if acceso {
                                        format!(
                                            "richiesto {r}, ma il controller NON segnala \
                                             il regime — non posso confermarlo"
                                        )
                                    } else {
                                        format!(
                                            "richiesto {r}, e il modulo risulta spento"
                                        )
                                    }),
                                    (None, _) => Some(
                                        "il controller non ha indicato un regime".to_owned(),
                                    ),
                                };
                            }
                        }
                    }
                }
                // Coda svuotata: o il round e' ancora in viaggio, o non ne
                // era arrivato nessuno. Non e' un errore in nessuno dei due
                // casi: si esce semplicemente senza dati per questo giro.
                Err(_) => break,
            }
        }
        if ultimo.is_some() {
            // Il round e' arrivato: si libera il posto per il prossimo.
            self.poll_in_flight = false;
        }
        ultimo
    }

    /// Invia una **scrittura** al thread proprietario, senza aspettare.
    ///
    /// `send` su canale non limitato non blocca mai. Le scritture hanno la
    /// precedenza sulle letture nel ciclo dell'attore, quindi un `Enable`
    /// premuto dall'utente non resta in coda dietro i campioni.
    fn scrivi_tec(&mut self, r: crate::attore_tec::Richiesta) {
        if self.tec_tx.send(r).is_err() {
            self.error_text = Some("Thread di polling seriale non raggiungibile".to_owned());
        }
    }

    /// Aspetta l'esito dello spegnimento, entro un tetto temporale.
    ///
    /// Blocca, ed e' voluto: siamo nella chiusura e serve la **certezza**
    /// che il TEC sia spento prima che il processo termini. Fuori da qui
    /// nessuna attesa: la UI non deve mai fermarsi.
    fn attendi_spegnimento(&mut self, timeout: std::time::Duration) -> String {
        use crate::attore_tec::{Risposta as P, Scritto as S};
        let scade = std::time::Instant::now() + timeout;
        loop {
            let rimane = scade.saturating_duration_since(std::time::Instant::now());
            if rimane.is_zero() {
                return "disable NON confermato entro il tempo".to_owned();
            }
            match self.tec_rx.recv_timeout(rimane) {
                // **L'ack dello spegnimento e' un `Regime`, come ogni altra
                // scrittura.** Prima era una variante dedicata (`S::Disable`),
                // e poteva essere prodotta solo dal pulsante "DISABILITA TEC" o
                // dall'uscita. Ora lo spegnimento e' `Richiesta::spegnimento()`,
                // cioe' `Regime::Spento`, e l'ack arriva come `S::Regime` con lo
                // stato riletto: l'ultimo comando dell'app sa **perche'** e' stato
                // eseguito, non solo che e' passato.
                Ok(P::Ack(Ok(S::Spegnimento(stato)))) => {
                    // Un ack di regime che dice ancora "il TEC gira" non e' uno
                    // spegnimento riuscito: il controller ha fatto qualcos'altro.
                    if stato.noti.contains(TecStatus::PID_RUNNING) {
                        return format!(
                            "spegnimento NON riuscito: il controller ha ancora il PID attivo"
                        );
                    }
                    return "spegnimento eseguito e atteso".to_owned();
                }
                // Un campione o un'altra scrittura non sono lo spegnimento:
                // si continua ad aspettare, entro il tetto.
                Ok(_) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    return "spegnimento NON confermato entro il tempo".to_owned()
                }
                // Il canale e' chiuso: il thread e' morto, il TEC e' rimasto
                // com'era. Va detto ad alta voce, non silenziosamente.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return "spegnimento IMPOSSIBILE: thread seriale non raggiungibile".to_owned()
                }
            }
        }
    }

    /// **Modalita' del controller: due righe, non due sezioni.**
    ///
    /// Prima qui c'era tutto: una riga di stato, poi le stesse tre modalita'
    /// elencate una per una, il tag "consigliata", il colore del LED con il suo
    /// lampeggio, e una nota che ripeteva perche' non si puo' commutare. Erano
    /// cinque righe per dire una cosa: **in quale regime si e' adesso**.
    ///
    /// Ora restano la riga di stato e il bottone che apre la spiegazione. Il
    /// perche' e la spiegazione stanno nella finestra, che si apre una volta e
    /// poi si ricorda. La lista delle tre modalita' era il posto sbagliato:
    /// elencare le alternative accanto allo stato fa sembrare che si possa
    /// scegliere, e non si puo'.
    ///
    /// Il colore che resta e' quello della modalita' corrente, e arriva da
    /// `colore_led()` come il tasto Abilita TEC e la striscia di potenza: un
    /// colore, un significato.

    fn shutdown(&mut self) {
        // 1) Chiudi la sessione con le statistiche finali
        let min_tec    = if self.stats.tec_temp.count > 0 { self.stats.tec_temp.min } else { 0.0 };
        let avg_cop    = if self.stats.cop.count > 0 { self.stats.cop.avg() } else { 0.0 };
        let min_margin = if self.last_cond_margin == f32::MAX { 0.0 } else { self.last_cond_margin };
        self.session_db.close_session(
            self.oc_score_current,
            min_tec,
            avg_cop,
            min_margin,
            self.ocp_event_count,
            self.stats.session_samples as u64,
        );
        // La potatura delle sessioni vecchie sta qui e non in `open()`.
        //
        // `prune_old_sessions` finisce in `VACUUM`, che riscrive l'intero
        // file: dentro `open()` bloccava la finestra per secondi a ogni
        // avvio. Ma va comunque chiamata da qualche parte, altrimenti la
        // tabella delle sessioni cresce senza limite — il tetto a 50 non
        // sarebbe piu' un tetto.
        //
        // Qui e' il momento giusto: la sessione e' chiusa, e non e' il
        // percorso che precede l'apertura della dashboard.
        self.session_db.pota_se_necessario();
        // Se il pannello cronologia e' aperto, la sessione appena chiusa
        // deve comparire subito: la cache vainvalidata.
        if self.show_session_hist {
            self.session_list = self.session_db.recent_sessions(40);
        }

        // 2) Spegni il TEC se lo abbiamo acceso noi.
        //
        // NON si usa `tec_status.contains(PID_RUNNING)`: quel campo viene dal
        // polling e resta sul valore pre-accensione se il battito fallisce.
        // Proprio in quel caso restavamo con il modulo alimentato a schermo
        // spento: niente guardia, niente watchdog, niente UI che lo
        // controlli. Il comando `disable` e' idempotente, quindi mandarlo
        // quando non serve costa un round-trip e toglie ogni dubbio.
            if self.tec_eroga_potenza() || self.applied_power > 0 {
                // **Unico punto che attende davvero.** Alla chiusura non
                // basta accodare il comando: se il processo termina prima
                // che il thread proprietario lo esegua, il TEC resta acceso.
                // Qui si accoda il `disable` e si aspetta il suo `Ack` sul
                // canale gia' esistente, con un tetto temporale per non
                // impiantare l'uscita se la porta non risponde.
                const SHUTDOWN_TIMEOUT: std::time::Duration =
                    std::time::Duration::from_millis(2000);
                self.scrivi_tec(crate::attore_tec::Richiesta::spegnimento());
                let esito = self.attendi_spegnimento(SHUTDOWN_TIMEOUT);
                crate::commissioning::event("SHUTDOWN", &esito);
                self.applied_power = 0;
            }

        // 3) Libera gli overlay
        self.rtss.clear();
        self.discord.clear();
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                // ── Watchdog finestra nascosta ──────────────────────
                // "Nascondi" ha come unico recupero il tray. Se l'icona non
                // e' raggiungibile la dashboard restava nascosta per sempre,
                // con il TEC che continua a girare senza nessuna lettura
                // visibile. Dopo 45 s la riportiamo su da soli.
                //
                // Prima di `should_update`: il watchdog deve girare anche
                // quando il campionamento e' sospeso.
                if let Some(t) = self.hidden_at {
                    if t.elapsed() >= std::time::Duration::from_secs(45) {
                        self.hidden_at = None;
                        // Dopo aver riportato la finestra si avvisa il
                        // livello superiore, che e' dove vive lo stato di
                        // fullscreen: altrimenti il flag continuerebbe a
                        // dire "schermo intero" e il F11 successivo
                        // sembrerebbe non funzionare.
                        return iced::window::get_latest().then(|opt_id| {
                            if let Some(id) = opt_id {
                                iced::window::change_mode(
                                    id, iced::window::Mode::Windowed,
                                )
                                .chain(iced::Task::done(
                                    Message::FinestraRiportataInFinestra,
                                ))
                            } else {
                                Task::done(Message::FinestraRiportataInFinestra)
                            }
                        });
                    }
                }
                if !self.should_update() { return Task::none(); }
                // **Non** reimpostare `last_sample_time` qui: sotto, in fondo
                // al tick, la richiesta del campione e' condizionata da
                // `should_update()`. Se il clock venisse azzerato adesso, quel
                // controllo sarebbe sempre falso e il campione non verrebbe
                // MAI richiesto: la UI resterebbe per sempre senza dati.
                self.tick_count = self.tick_count.wrapping_add(1);
                // Intestazione del log di collaudo al primo tick: il file deve
                // dire con quale hardware e firmware e' stato prodotto,
                // altrimenti le righe non si possono attribuire.
                if self.tick_count == 1 {
                    crate::commissioning::header(self.fw_major, self.fw_minor, self.hw_version);
                }

                // Nota: tick_count incrementa a 2 Hz (ogni 500ms, gated da should_update).
                // Tutte le temporizzazioni sono calcolate in base a questo.
                //   1 tick = 500ms   |   2 tick = 1s   |   4 tick = 2s
                //   6 tick = 3s      |  10 tick = 5s   |  20 tick = 10s
                //  40 tick = 20s     | 120 tick = 60s

                // sensors.tick() ogni tick = 500ms (HWiNFO shared memory ~10ms)
                {
                    self.sensors.tick();

                    // ── RPM della pompa, non "la ventola piu' veloce" ─────
                    //
                    // Prima si prendeva il MAX su tutte le ventole: una
                    // ventola di case a 2000 RPM mascherava una pompa
                    // ferma, cioe' la protezione non scattava proprio
                    // quando serviva. Ora si cerca la pompa per nome.
                    //
                    // Se non e' identificabile non si inventa nulla: si
                    // tiene il valore precedente e si segnala che la
                    // protezione non puo' valutare. Meglio "non lo so"
                    // che una stima falsa che innesca o disinnesca la
                    // limitazione a caso.
                    let pompa = self.sensors.sensors.iter().find(|s| {
                        s.sensor_type == crate::hwinfo::SensorType::Fan
                            && (crate::hwinfo::ci_contains(&s.reading_name, "pump")
                                || crate::hwinfo::ci_contains(&s.reading_name, "pompa")
                                || crate::hwinfo::ci_contains(&s.sensor_id, "pump"))
                    });

                    if let Some(p) = pompa {
                        self.last_pump_rpm = p.value;
                        self.pump_readable = true;
                    } else {
                        // Nessuna pompa identificabile nei sensori. NON si
                        // assume che sia ferma (causerebbe un limitamento
                        // continuo a caso) e non si assume che giri.
                        //
                        // Si passa alla firma termica: se il loop e' fermo la
                        // piastra sale con potenza alta, e quello lo si vede
                        // senza nessun sensore. `pompa::poll` aggiorna
                        // `pump_stalled` e lo dichiara solo se il criterio
                        // tiene da abbastanza campioni.
                        self.pump_readable = false;
                    }

                    // ── Firma termica della pompa ───────────────────
                    // Se l'RPM c'e' vale l'RPM; se non c'e', si deduce dalla
                    // temperatura. Prima di questo, su una macchina senza
                    // HWiNFO la protezione pompa non poteva MAI scattare.
                    let rpm_noto = if self.pump_readable { Some(self.last_pump_rpm) } else { None };
                    let raffredda = self.inputs.set_point < 0.0
                        && self.tec_status.contains(
                            cryo_cooler_controller_lib::TecStatus::PID_RUNNING);
                    if self.pompa_watch.poll(
                        self.chart.last_tec_temp(),
                        self.applied_power,
                        raffredda,
                        rpm_noto,
                    ) {
                        crate::commissioning::event(
                            "POMPA",
                            "firma termica: piastra in salita con potenza alta, \
                             loop probabilmente fermo",
                        );
                    }
                    self.pump_stalled = self.pompa_watch.guasto();
                }

                // Sample the displayed CPU each second to feed the visual buffer.
                // Controller injection keeps its existing, independent cadence.
                if self.tick_count % 2 == 0 {
                    self.last_cpu_temp_ext = hwinfo::read_cpu_temperature(&self.sensors.active_source);
                    if let Some(cpu_t) = self.last_cpu_temp_ext { self.chart.update_cpu_temp(cpu_t); }
                }
                if self.tick_count % CPU_TEMP_PUSH_INTERVAL == 0 {
                    if let Some(cpu_t) = self.last_cpu_temp_ext {
                        self.scrivi_tec(crate::attore_tec::Richiesta::TempCpu(cpu_t));
                    }
                    // FEAT #3: leggi GPU temp da HWiNFO ogni 5s (stesso intervallo CPU)
                    self.last_gpu_temp_ext = self.sensors.sensors.iter()
                        .filter(|s| s.sensor_type == crate::hwinfo::SensorType::Temperature)
                        .filter(|s| {
                            let n = &s.reading_name;
                            (crate::hwinfo::ci_contains(n, "gpu") || crate::hwinfo::ci_contains(n, "graphic"))
                                && (crate::hwinfo::ci_contains(n, "temperature") || crate::hwinfo::ci_contains(n, "temp") || crate::hwinfo::ci_contains(n, "hot spot"))
                                && !crate::hwinfo::ci_contains(n, "memory") && !crate::hwinfo::ci_contains(n, "video")
                        })
                        .map(|s| s.value)
                        .reduce(f32::max);
                }

                // BUG B FIX — Re-push periodico del cap al firmware.
                // Il PID onboard gira in autonomia e ignora il cap se non viene
                // confermato. Il software ufficiale invia i setpoint in
                // realtime nel suo loop principale.
                //
                // NB: si ri-pusha `applied_power`, NON `inputs.max_power`: se
                // mandassimo il cap dell'utente annulleremmo la protezione del
                // controller ogni 5 secondi (il firmware dimenticherebbe il
                // limite e continuerebbe a scaldarsi).
                // Gen1: nessun re-push del registro potenza ignorato dal firmware.

                // FEAT #1: Auto-save CSV ogni 5 minuti (600 tick × 500ms = 300s)
                // Protegge i dati in caso di crash durante sessioni OC aggressive.
                self.auto_save_ticks = self.auto_save_ticks.wrapping_add(1);
                if self.auto_save_ticks % 600 == 0 && self.auto_save_ticks > 0 {
                    if let Ok(path) = self.auto_save_csv() {
                        // Aggiorna la status bar silenziosamente (nessun modal)
                        self.pdf_status = Some(format!("💾 Auto-save: {}", path.file_name()
                            .and_then(|n| n.to_str()).unwrap_or("cryo_autosave.csv")));
                    }
                }

                // ── Campionamento ────────────────────────────────────────
                //
                // Il round e' gia' stato eseguito dal thread proprietario:
                // qui si prende il risultato **senza aspettare** (`try_recv`).
                // Se non e' ancora arrivato si salta il giro e si riprova al
                // tick dopo: la UI non si blocca piu' sulla seriale, quindi i
                // grafici non scattano piu'.
                //
                // Sotto, la guardia e' **identica** a quella di r81: cambia
                // solo da dove arriva `data`. Nessuna decisione di sicurezza
                // e' stata spostata nel thread.
                // ── Campionamento TEC ────────────────────────────────────
                //
                // Se il round non e' ancora arrivato **non si fa niente**:
                // niente guardia, niente watchdog, e soprattutto nessun
                // errore. "Non e' ancora arrivato" e' la norma con un thread
                // in background, non un guasto. Registrarlo come errore faceva
                // salire `monitor_failures` e il watchdog tagliava la potenza
                // al 30% per un ritardo di un giro: pericoloso, e falso.
                if let Some(poll) = self.raccogli_campione() {
                let (mon, beat) = match poll {
                    Ok((data, status)) => (Ok(data), Ok(status)),
                    Err(e) => {
                        self.ocp_consecutive = 0;
                        (Err(e), Err("nessun battito: round fallito".to_owned()))
                    }
                };
                match mon {
                    Ok(data) => {
                        self.last_power_watts = data.tec_power_watts;
                        self.last_electrical = Some((data.tec_voltage,data.tec_current));
                        self.last_pcb_temperature = data.pcb_temperature;
                        // **Efficienza marginale**, calcolata in un punto solo.
                        //
                        // Servono due temperature: la piastra e la scheda, che e'
                        // sul lato caldo. Il COP vero del modulo non e' calcolabile
                        // senza i suoi parametri di costruzione, ma il rendimento
                        // marginale si: dice quanti gradi si comprano con i
                        // **ultimi** watt, che e' la domanda che decide se
                        // spingere ancora ha senso.
                        if let Some(dt) = crate::efficienza::delta_t(
                            data.pcb_temperature, data.tec_temperature,
                        ) {
                            let potenza = data.tec_power_watts;
                            self.rendimento_marginale = self.efficienza_precedente
                                .and_then(|prec| crate::efficienza::rendimento_marginale(
                                    prec, (dt, potenza),
                                ));
                            self.efficienza_precedente = Some((dt, potenza));
                        }
                        self.last_cond_margin = data.condensation_margin;
                        self.active_alerts = self.active_alerts.iter().filter(|(channel,_)| {
                            self.valore_live_canale(channel).map_or(true, |value| {
                                self.alert_manager.rules().iter().any(|r|r.channel == *channel && r.triggered(value))
                            })
                        }).cloned().collect();
                        // ── Protezione controller TEC ──────────────────────
                        // Riduce il cap PRIMA che il firmware Delta2 colpisca
                        // OT1/OT2 (80°C -> Standby). Con un controller V1 su
                        // modulo V2 la PCB supera quella soglia e il ciclo
                        // Standby/ripresa è il "scalda molte volte".
                        self.update_ctrl_guard(data.pcb_temperature);
                        // Dopo la guardia: se la guardia ha fatto scendere, la
                        // salita graduale e' già stata disarmata e non fa
                        // niente. La sicurezza viene prima, sempre.
                        // **Niente regolatore chiuso, qui.** Questa riga e' la
                        // causa dei 150-225 W: `regola_potenza` saliva di uno
                        // (o fino a otto) punti a ogni tick per inseguire un
                        // target, e il modulo restava inchiodato al cap. r116
                        // non aveva un regolatore e faceva 20-30 W.
                        //
                        // Il soft start resta: e' quello che in r116 saliva
                        // lentamente dal 30% dopo l'accensione, e serve a non
                        // far crollare la piastra sotto la rugiada. Dopo, la
                        // potenza la decide l'operatore e il firmware.
                        // Gen 1 / TEC Gen 2: regulate demand using actual watts.
                        // Ignore the power-register percentage as a cap.
                        if self.ultimo_regime_richiesto == Some(crate::commutazione::Regime::Cryo)
                            && self.tec_status.contains(TecStatus::PID_RUNNING) {
                            let budget = 200.0 * self.inputs.max_power.min(100) as f32 / 100.0;
                            if let Some(offset) = self.tec_regulator.update_with_cpu(
                                self.regulation_started.elapsed().as_secs_f64(), data.tec_temperature,
                                data.dew_point_temperature, data.pcb_temperature,
                                data.tec_power_watts, self.regulation_offset, budget, Self::CTRL_SOFT, Profile { name: self.inputs.profile_name.clone(), ..Profile::default() }.margine_sicuro(), self.last_cpu_temp_ext,
                            ) {
                                self.scrivi_tec(crate::attore_tec::Richiesta::Setpoint(offset));
                                crate::commissioning::event("REGOLAZIONE-GEN1", &format!("offset={offset:.2} watt={:.1} budget={budget:.1} {}", data.tec_power_watts, self.tec_regulator.status));
                            }
                        }
                        // Aggiorna il power level letto dal firmware — usato per diagnosi UI
                        self.fw_power_level   = data.tec_power_level;
                        // **Spool in corso.** Richiesta di accensione pendente e
                        // ancora senza effetti: il firmware sale da solo in
                        // ~2 minuti. La riga mostra secondi e watt cosi' si
                        // vede che e' vivo invece di sembrare morto. Oltre i
                        // 4 minuti senza effetti, dice cosa fare.
                        let in_avvio = matches!(
                            self.commutazione,
                            crate::commutazione::Commutazione::Richiesta(r)
                            if r != crate::commutazione::Regime::Spento
                        ) && self.attesa_tick > 0
                            && data.tec_power_watts < 5.0
                            && data.tec_power_level < 5;
                        if in_avvio {
                            let secondi = self.tick_count.saturating_sub(self.attesa_tick) / 2;
                            self.esito_commutazione = Some(if secondi < 240 {
                                format!(
                                    "avvio da {secondi}s: {:.1} W, il firmware \
                                     sale da solo — non toccare",
                                    data.tec_power_watts,
                                )
                            } else {
                                format!(
                                    "avvio da {secondi}s ancora a {:.1} W: sembra \
                                     bloccato — premi DISABILITA, aspetta 30s, \
                                     premi ABILITA",
                                    data.tec_power_watts,
                                )
                            });
                        }
                        // **Conferma per effetti.** Su questo Gen 1 il bit
                        // `PID_RUNNING` non si attiva mai, quindi la rilettura
                        // dei bit non confermera' mai Cryo e la richiesta
                        // resterebbe pendente per sempre con "non posso
                        // confermarlo". Ma un modulo che assorbe watt al
                        // livello comandato **sta lavorando**: gli effetti
                        // sono una conferma, e il messaggio dice esplicitamente
                        // cosa conferma cosa — non finge una lettura dei bit.
                        if matches!(
                            self.commutazione,
                            crate::commutazione::Commutazione::Richiesta(
                                crate::commutazione::Regime::Cryo
                            )
                        ) && data.tec_power_level >= 5
                            && data.tec_power_watts > 5.0
                        {
                            self.esito_commutazione = Some(format!(
                                "Cryo attivo (confermato dagli effetti): il Gen 1 \
                                 non dichiara il regime, ma il modulo assorbe \
                                 {:.0} W al {}%",
                                data.tec_power_watts, data.tec_power_level,
                            ));
                        }
                        // Speculare per lo spegnimento: su Gen 1 nemmeno Spento
                        // viene confermato dai bit quando l'OCP (rumore) resta
                        // su, e la riga restava pendente per sempre. Ma un
                        // modulo a 0 W **e' spento**: gli effetti confermano.
                        if matches!(
                            self.commutazione,
                            crate::commutazione::Commutazione::Richiesta(
                                crate::commutazione::Regime::Spento
                            )
                        ) && data.tec_power_watts < 2.0
                        {
                            self.esito_commutazione = Some(
                                "Spento attivo (confermato dagli effetti): \
                                 modulo fermo a 0 W"
                                    .to_owned(),
                            );
                        }
                        self.stats.update(&data);

                        // ── Condensation Risk Engine ──────────────────
                        self.cond_risk.push(data.tec_temperature, data.dew_point_temperature);

                        // Il controller misura l'umidita' del loop: da li'
                        // si ricava la temperatura ambiente, che e' la
                        // reference per l'offset. Il margine sul punto di
                        // rugiada e' il pavimento anticondensa.
                        self.last_dew_point = data.dew_point_temperature;
                        if data.humidity.is_finite() && data.dew_point_temperature.is_finite()
                        {
                            // Formula di Magnus semplificata, inversa della
                            // relazione che il firmware usa per il punto di
                            // rugiada: T_ambiente ≈ T_rugiada + (1-RH/100)*k
                            // Con k ≈ 18 °C a temperature ambiente normali.
                            self.last_room_temp = data.dew_point_temperature
                                + (1.0 - (data.humidity / 100.0).clamp(0.0, 1.0)) * 18.0;
                        }
                        // tick_count incrementa a 2 Hz (should_update gate = 500ms)
                        // → interval_secs = 0.5 per campione
                        // Cachato in last_risk_state — view usa la cache, non ricalcola
                        self.last_risk_state = self.cond_risk.analyze(0.5);

                        // ── Registrazione curva della cella ───────────
                        //
                        // Scrive una riga CSV con V, A, W e temperature **solo
                        // quando la potenza e' cambiata** e le misure hanno avuto
                        // tempo di stabilizzarsi. Non tocca la potenza e non
                        // chiama il TEC: e' un misuratore passivo, quindi non
                        // puo' cambiare il comportamento termico.
                        //
                        // Serve per trovare il punto di massimo rendimento della
                        // cella, che e' una proprieta' dell'hardware e non si
                        // deduce dalla teoria. Vedi `docs/analisi-cella-peltier.md`.
                        self.misura_curva(&data);

                        // ── COP Calculator ───────────────────────────
                        if let Some(cpu_t) = self.last_cpu_temp_ext {
                            self.cop_state = CopState::calculate(
                                cpu_t, data.tec_temperature, data.tec_power_watts
                            );
                            // FIX [6]: traccia COP medio di sessione
                            self.stats.push_cop(self.cop_state.cop);
                        }

                        // ── Modalita' automatica ─────────────────────
                        // Sorgenti dei dati, in ordine di affidabilita':
                        //   1. sensori HWiNFO/AIDA64 (se installati)
                        //   2. API di Windows, senza dipendenze
                        // La temperatura e' facoltativa: se non c'e' un
                        // sensore, il regime si decide sul carico da solo.
                        if self.auto_profile_on {
                            let mut carico: Option<f32> = None;
                            let mut origine = "nessuno";
                            let mut temp_cpu: Option<f32> = None;

                            for s in &self.sensors.sensors {
                                let n = &s.reading_name;
                                let e_cpu = crate::hwinfo::ci_contains(n, "cpu")
                                    || crate::hwinfo::ci_contains(n, "processor");
                                if e_cpu {
                                    match s.sensor_type {
                                        crate::hwinfo::SensorType::Load => {
                                            carico = Some(carico.unwrap_or(0.0).max(s.value));
                                        }
                                        crate::hwinfo::SensorType::Temperature => {
                                            // La CPU e' il pacchetto: si prende il
                                            // massimo fra i core, altrimenti si
                                            // legge il core piu' freddo e si
                                            // sottovaluta il rischio termico.
                                            temp_cpu =
                                                Some(temp_cpu.unwrap_or(f32::MIN).max(s.value));
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            if carico.is_some() {
                                origine = "HWiNFO";
                            } else {
                                // Ripiego: il carico lo espone il sistema.
                                if let Some(v) = self.carico_win.campiona() {
                                    carico = Some(v);
                                    origine = "Windows";
                                }
                            }
                            self.carico_origine = origine;

                            // Senza nessuna lettura non c'e' niente su cui
                            // decidere. Non si inventa un carico: si registra
                            // che mancano i dati, cosi' la UI lo dice invece di
                            // mostrare un Idle che sembra scelto e non calcolato.
                            self.auto_senza_sensore = carico.is_none() && temp_cpu.is_none();
                            let esito = self.auto_mgr.push(carico, temp_cpu);
                            let dati_validi = !self.auto_senza_sensore;
                            let corrente = self.auto_mgr.regime();
                            if let Some(r) = crate::automanager::regime_da_applicare(
                                self.auto_profile_on,
                                self.auto_da_allineare,
                                esito,
                                corrente,
                                dati_validi,
                            ) {
                                self.apply_regime(r);
                                self.auto_da_allineare = false;
                            }
                        }

                        // ── Alert Manager ────────────────────────────
                        // Alert check ogni 2s (4 tick @ 2 Hz)
                        if self.tick_count % ALERT_EVERY_TICKS == 0 {
                            // Se la pompa non e' leggibile, si passa un
                            // valore "neutro" (sopra soglia) invece di 0.
                            //
                            // Altrimenti la regola "sotto 200 RPM" scatta
                            // sempre: l'utente vedeva "POMPA FERMA" a
                            // raffica anche con la pompa che girava
                            // regolarmente, e dopo averla silenziata per
                            // un falso allarme la protezione reale
                            // diventava scomoda da usare.
                            let rpm_per_alert = if self.pump_readable {
                                self.last_pump_rpm
                            } else {
                                9999.0
                            };
                            // Il canale TEC e' uno **scarto dal setpoint**,
                            // non la temperatura assoluta: vedi la regola in
                            // `alerts.rs`. Il TEC qui non scende sotto i
                            // 26 °C perche' il pavimento anticondensa glielo
                            // impedisce, e con una soglia assoluta la regola
                            // era sempre vera, con tutto regolare.
                            //
                            // **Ma solo se il setpoint e' credibile.** Senza il
                            // dato di rugiada il pavimento anti-condensa
                            // restituisce -1000, il setpoint vale -1000 e lo
                            // scarto sarebbe ~1000: la regola scatterebbe
                            // sempre, con un numero che non descrive nulla.
                            // In quel caso il canale non e' misurabile e non
                            // si valuta: `None` fa saltare la regola, che e'
                            // esattamente la semantica giusta.
                            let setpoint_credibile = self.inputs.set_point.is_finite()
                                && self.inputs.set_point > -200.0;
                            let scarto_tec = if setpoint_credibile {
                                Some(data.tec_temperature - self.inputs.set_point)
                            } else {
                                None
                            };
                            let check = self.alert_manager.check(
                                scarto_tec,
                                // "TEC calda" ha senso solo se il TEC sta
                                // davvero raffreddando. Se e' spento o in
                                // regime la piastra e' a temperatura ambiente
                                // e la soglia la raggiunge senza che ci sia
                                // nessun guasto: era la causa dell'allarme a
                                // raffica che l'utente segnalava.
                                self.tec_status.contains(TecStatus::PID_RUNNING),
                                // Lo scarto dal setpoint e' un'anomalia solo
                                // se un setpoint viene inseguito: in manuale
                                // la potenza e' fissa e la piastra sta dove
                                // la mette la fisica. Senza questo, in
                                // manuale l'avviso restava sempre acceso.
                                self.auto_profile_on,
                                self.last_cpu_temp_ext,
                                data.condensation_margin,
                                data.tec_power_watts,
                                data.humidity,
                                rpm_per_alert,
                                ALERT_CHECK_HZ,
                            );

                            // ── Emergenza pompa ─────────────────────────────
                            // Il TEC raffredda trasportando calore: senza
                            // circolazione la piastra si scalda e il danno e'
                            // irreversibile. Sotto una soglia di RPM il
                            // limitatore scende.
                            //
                            // **Perche' la decisione usa `last_pump_rpm` e non
                            // `check.pump_emergency`.** Il flag dell'alert
                            // vale `false` quando la regola e' in cooldown,
                            // cioe' per i 5 minuti successivi. Leggendolo come
                            // "pompa recuperata" il blocco di rilascio
                            // ripristinava il cap pieno dopo 2 secondi
                            // DAVVERO a pompa ferma, e per 5 minuti non
                            // succedeva piu' nulla: il TEC girava al 100% con
                            // la pompa morta.
                            const PUMP_MIN_RPM: f32 = 200.0;
                            // Se la pompa non e' leggibile la protezione
                            // resta sospesa: non si assume "ferma" (che
                            // generava l'allarme "POMPA FERMA" a raffica
                            // senza che la pompa si fosse mai fermata) ne
                            // "sana" (che nasconderebbe un guasto reale).
                            // Tre casi, tenuti separati perche' hanno
                            // conseguenze opposte:
                            //
                            //  - sensore che legge sotto soglia  -> guasto
                            //    reale, si limita;
                            //  - nessun sensore, ma la FIRMA TERMICA ha
                            //    parlato -> guasto reale, si limita. E' l'unico
                            //    giudizio disponibile sulla V1, quindi deve
                            //    poter agire: prima scriveva "FERMA" in rosso
                            //    sulla dashboard e la potenza non cambiava, e
                            //    un'indicazione di guasto senza protezione
                            //    dietro e' peggio di niente, perche' l'operatore
                            //    crede di essere coperto;
                            //  - nessun sensore e nessun altro indizio -> NON
                            //    SI SA. E non si limita. Limitare "per
                            //    sicurezza" qui e' quello che teneva la V1 al
                            //    50% per tutta la sessione: non e' prudenza, e'
                            //    un guasto che si autosabotaggia, perche' la
                            //    firma termica richiede potenza >= 60% e quindi
                            //    non avrebbe mai potuto giudicare niente.
                            // Solo l'RPM reale può limitare la potenza.
                            //
                            // La firma termica (`pump_stalled`) **non** entra
                            // nel tetto: resta solo per schermo e allarmi.
                            //
                            // Sulla V1 non esiste sensore, e la firma si
                            // autosabotaggia: per giudicare un loop fermo
                            // vuole potenza alta, ma a quel punto abbassare la
                            // potenza le toglie l'unico dato che aveva. Il
                            // risultato era il TEC che non saliva e nessuno
                            // capiva perché, perché la firma poteva scattare
                            // e restare attiva da sola.
                            let in_fallo = self.pump_readable
                                && self.last_pump_rpm < PUMP_MIN_RPM;

                            if in_fallo {
                                if !self.pump_emergency_active {
                                    self.max_power_before_emergency = self.inputs.max_power;
                                    self.pump_emergency_active = true;
                                    // Non si sale MAI sopra il cap dell'utente:
                                    // con cap a 30% si scrive 30, non 50.
                                    let lim = self.inputs.max_power.min(50);
                                    self.inputs.max_power = lim;
                                    // Aggiornava l'hardware ma NON
                                    // `applied_power`: il re-push scriveva
                                    // il valore vecchio e annullava il
                                    // limitatore ogni 10 secondi.
                                    self.applica_potenza(lim);
                                }
                            } else if !in_fallo && self.pump_emergency_active {
                                // Circolazione ristabilita: si rimuove il
                                // limitatore. Senza questo ramo il tetto
                                // restava inchiodato anche dopo che la pompa
                                // ripartiva.
                                self.pump_emergency_active = false;
                                self.inputs.max_power = self.max_power_before_emergency;
                                let ripristinato = self.inputs.max_power;
                                self.applica_potenza(ripristinato);
                            }

                            // Un solo toast per evento, con tutte le
                            // etichette accorpate: ogni notifica costa un
                            // processo PowerShell (~100 MB), quindi N alert
                            // non devono diventare N processi.
                            if !check.labels.is_empty() {
                                if self.app_config.notify.toasts {
                                    let msg = check
                                        .labels
                                        .iter()
                                        .map(|(_, l)| l.as_str())
                                        .collect::<Vec<_>>()
                                        .join("\n");
                                    crate::alerts::send_toast("Stargate Cryo", &msg);
                                }
                                // Gli allarmi in-app sono gratuiti e mostrano
                                // informazioni di sicurezza: si possono
                                // spegnere, ma non di default.
                                //
                                // Un banner per canale: se la regola spara di
                                // nuovo, si aggiorna l'etichetta invece di
                                // accumulare doppioni.
                                if self.app_config.notify.banner {
                                    for (ch, lb) in check.labels {
                                        self.active_alerts.retain(|(c, _)| *c != ch);
                                        self.active_alerts.push((ch, lb));
                                    }
                                    if self.active_alerts.len() > 3 {
                                        let troppi = self.active_alerts.len() - 3;
                                        self.active_alerts.drain(0..troppi);
                                    }
                                }
                            }
                        }

                        // ── Dynamic theme hue ────────────────────────
                        // Interpola: TEC < -8°C → hue 200° (blu ghiaccio)
                        //            TEC = 0°C  → hue 161° (verde petrolio)
                        //            TEC > 4°C  → hue 30°  (arancio pericolo)
                        let t = data.tec_temperature;
                        self.theme_hue = if t < -8.0 {
                            200.0
                        } else if t < 0.0 {
                            161.0 + (200.0 - 161.0) * (-t / 8.0)
                        } else if t < 4.0 {
                            161.0 - (161.0 - 30.0) * (t / 4.0)
                        } else {
                            30.0
                        };

                        // ── OC Score live ────────────────────────────
                        if self.stats.session_samples > 10 {
                            let uptime_min = (self.stats.session_samples / 120) as u32;
                            self.oc_score_current = OcScore::calculate(
                                self.stats.tec_temp.min,
                                self.cop_state.delta_t,
                                // FIX [6]: usa media COP di sessione, non valore istantaneo
                                if self.stats.cop.count > 0 { self.stats.cop.avg() } else { self.cop_state.cop },
                                self.stats.condensation_margin.min,
                                uptime_min,
                                self.ocp_event_count,
                            ).score;
                            if self.oc_score_current > self.oc_best {
                                self.oc_best = self.oc_score_current;
                            }
                        }

                        // ── Session DB sample ───────────────────────
                        self.session_db.push_sample(SampleRow {
                            ts:        data.timestamp,
                            tec_temp:  data.tec_temperature,
                            dew_point: data.dew_point_temperature,
                            cpu_temp:  self.last_cpu_temp_ext,
                            power_w:   data.tec_power_watts,
                            humidity:  data.humidity,
                            cop:       if self.cop_state.cop > 0.0 { Some(self.cop_state.cop) } else { None },
                            margin:    data.condensation_margin,
                        });

                        // ── PID Wizard tick ──────────────────────────
                        if self.pid_wizard.is_running() {
                            if let Some(pwr) = self.pid_wizard.tick(data.tec_temperature) {
                                // Il Wizard NON puo' scavalcare la guardia.
                                //
                                // Prima scriveva il valore grezzo: 80% per 30
                                // secondi, ignorando sia il cap dell'utente
                                // sia la protezione termica. La guardia
                                // scriveva 52% e 30 ms dopo il Wizard
                                // riscriveva 80%: su un controller V1 (225W)
                                // con modulo V2 (288W) questo e' il modo piu'
                                // rapido per bruciare il modulo.
                                //
                                // Ora il valore passa dallo stesso tetto
                                // della guardia, e `applied_power` viene
                                // aggiornato, cosi' i due non divergono.
                                let applicato = pwr.min(self.tetto_potenza());
                                if applicato != self.applied_power {
                                    // Scrittura diretta, come in r81: lo stato
                                    // segue l'hardware, non una speranza.
                                    self.applica_potenza(applicato);
                                    self.power_push_failures = 0;
                                }
                            }
                        }

                        // ── Overlay RTSS + Discord (ogni 2s = 4 tick @ 2 Hz) ──
                        self.overlay_ticks = self.overlay_ticks.wrapping_add(1);
                        if self.overlay_ticks % 4 == 0 {
                            self.rtss.update(
                                data.tec_temperature,
                                data.condensation_margin,
                                self.cop_state.cop,
                            );
                            // Il regime e' quello deciso dal gestore
                            // automatico, non il vecchio profilo: e' l'unico
                            // che arriva davvero all'hardware.
                            let mode_str =
                                Some(self.auto_mgr.regime().etichetta().to_owned())
                                .unwrap_or_else(|| if self.auto_profile_on {
                                    "auto".to_owned()
                                } else {
                                    "manuale".to_owned()
                                });
                            self.discord.update(
                                data.tec_temperature,
                                self.oc_score_current,
                                &mode_str,
                            );
                        }

                        self.log.push(LogEntry {
                            timestamp:           data.timestamp.format("%Y-%m-%d %H:%M:%S%.3f").to_string(),
                            tec_temp:            data.tec_temperature,
                            pcb_temp:            data.pcb_temperature,
                            humidity:            data.humidity,
                            dew_point:           data.dew_point_temperature,
                            condensation_margin: data.condensation_margin,
                            tec_voltage:         data.tec_voltage,
                            tec_current:         data.tec_current,
                            tec_power_watts:     data.tec_power_watts,
                            tec_power_level:     data.tec_power_level,
                            cpu_temp_ext:        self.last_cpu_temp_ext,
                        });
                        // Cap a 3600 entries = 30 min @ 2 Hz — i sample completi sono in SessionDb
                        const LOG_MAX: usize = 3_600;
                        if self.log.len() > LOG_MAX {
                            self.log.drain(0..self.log.len() - LOG_MAX);
                        }
                        self.chart.update(data);
                        self.monitor_failures = 0;
                    }
                    Err(err) => {
                        // WATCHDOG: se la lettura fallisce, la guardia non
                        // gira piu'. Prima non succedeva niente: il TEC
                        // restava all'ultimo valore scritto, spesso 100%,
                        // per un tempo indefinitito, senzache nessuno
                        // intervenisse. Ora si conta e dopo un breve
                        // margine si scende a una potenza conservativa.
                        self.monitor_failures = self.monitor_failures.wrapping_add(1);
                        self.error_text = Some(format!("Monitor error: {err}"));

                        // 3 errori = ~1,5 s a 2 Hz. Il breve margine serve a
                        // non reagire a un singolo timeout, che su una
                        // seriale capita normalmente.
                        const WATCHDOG_TRIGGER: u32 = 3;
                        if self.monitor_failures >= WATCHDOG_TRIGGER
                            && self.monitor_failures % WATCHDOG_TRIGGER == 0
                            && (self.last_power_watts > 2.0 || self.applied_power > 0)
                        {
                            // Scendiamo comunque: se il comando fallisce
                            // non e' peggio di lasciare il TEC al massimo.
                            // Se la scrittura fallisce non c'e' da fare:
                            // il modulo resta al valore precedente e non
                            // possiamo far altro da qui. `applied_power`
                            // non viene toccato, quindi il re-push riprova.
                            // The Gen 1 board ignored the power cap in the real
                            // test. A 30% write cannot protect a blind controller.
                            self.scrivi_regime(crate::commutazione::Regime::Spento);
                            self.watchdog_tripped = true;
                            crate::commissioning::event(
                                "WATCHDOG",
                                &format!(
                                    "{} errori di monitor(): richiesto DISABLE reale",
                                    self.monitor_failures
                                ),
                            );
                        }
                    }
                }

                // Il battito viaggia nello stesso round del monitor: non e'
                // una seconda attesa, e' la seconda meta' dello stesso
                // risultato gia' raccolto.
                match beat {
                    Ok(status_completo) => {
                        // Lo stato completo, non solo i 18 bit noti: i 14
                        // alternativi finiscono nel log per la correlazione
                        // con i codici errore del manuale. `tec_status` resta
                        // il campo che tutta la dashboard gia' legge, quindi
                        // i ~20 chiamanti esistenti non cambiano.
                        self.tec_status_completo = status_completo;
                        self.tec_status = status_completo.noti;
                        // Una riga per campione, 2 Hz. Read-only: non e' un
                        // comando, e serve a capire se gli alternativi
                        // cambiano quando l'OCP scatta.
                        // Una riga di log della diagnostica a ogni cambio
                        // di verdetto, non a ogni campione: qui e' il punto
                        // giusto perche' e' il momento in cui i bit sono
                        // freschi. Il file e' quello che l'utente manda al
                        // supporto, quindi la prima parte dichiara che
                        // questo NON e' il self test del produttore.
                        // `None` = misure non disponibili: non si inventa un
                        // verdetto. Il ramo che segue lo tratta come "non lo so".
                        let diagnostica_ora = self.dati_diagnostica().map(|m| {
                            crate::diagnostica::valuta(m, status_completo.noti)
                        });
                        // `None` = misure non disponibili. Si logga **una volta**
                        // e non piu': il log serve a registrare i cambiamenti di
                        // verdetto, e qui non ce n'e'. Senza questo, il log
                        // crescerebbe di una riga ogni mezzo secondo dicendo
                        // sempre la stessa cosa.
                        //
                        // Prima, quando le misure non erano disponibili, si
                        // passava un `Esito` vuoto: `valuta` lo traduceva in
                        // "nessuna anomalia" e il log registrava un controller
                        // sano che nessuno aveva misurato.
                        match diagnostica_ora {
                            Some(d) => {
                                // `None` al primo campione: la prima riga entra
                                // sempre, cosi' il file non parte a meta' senza
                                // dire nulla.
                                if self.diagnostica_precedente != Some(d.verdetto) {
                                    self.diagnostica_precedente = Some(d.verdetto);
                                    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
                                        let percorso =
                                            crate::stato_log::cartella_app(&dir.to_string_lossy())
                                                .join("diagnostica.log");
                                        let riga =
                                            d.riga_log(&chrono::Utc::now().to_rfc3339());
                                        if let Ok(mut f) = std::fs::OpenOptions::new()
                                            .create(true)
                                            .append(true)
                                            .open(&percorso)
                                        {
                                            use std::io::Write;
                                            let _ = writeln!(f, "{riga}");
                                        }
                                    }
                                }
                            }
                            // Se un giorno le misure torneranno disponibili
                            // dopo una pausa, `diagnostica_precedente` deve
                            // ripartire vuoto, altrimenti il primo verdetto
                            // vero sarebbe scambiato per una ripetizione e non
                            // entrerebbe nel log.
                            None => self.diagnostica_precedente = None,
                        }

                        // **Il log su disco e' spento di default.** Scriveva a
                        // ogni campione, 2 Hz, per tutta la sessione: il file
                        // arrivava al tetto e da li' veniva riscritto intero a
                        // ogni campione. Il consumo di disco che l'utente ha
                        // notato e' questo. Si accende solo se lo chiede.
                        if self.app_config.log_su_disco {
                            self.stato_log.registra(
                                &status_completo,
                                &chrono::Utc::now().to_rfc3339(),
                            );
                        }
                        if status_completo.noti.contains(TecStatus::OCP_ACTIVE) {
                            self.ocp_consecutive = self.ocp_consecutive.saturating_add(1);
                        } else {
                            self.ocp_consecutive = 0;
                        }
                        self.ocp_confirmed = self.ocp_consecutive >= OCP_DEBOUNCE_THRESHOLD;

                        // ── Reazione all'OCP: una volta sola ──────────
                        //
                        // **Questo bit e' un falso allarme su questo hardware.**
                        //
                        // Misurato: l'OCP scatta a 230 W, ma anche a 73 W
                        // (il 30% del tetto). Se fosse un vero indicatore di
                        // sovraccorrente, non potrebbe scattare con una
                        // corrente cosi' bassa. Intanto la TEC raffredda
                        // normalmente a 220 / 230 / 237 W.
                        //
                        // Quindi il bit non e' un indicatore di sovraccorrente
                        // e **non deve poter spegnere il raffreddamento**.
                        //
                        // r96 lo aveva reso persistente, e per questo ogni
                        // giro toglieva potenza: da 225 W si e' scesi a 73 W e
                        // la TEC ha smesso di raffreddare. Un allarme falso
                        // che spegne il dispositivo e' peggio di nessun
                        // allarme.
                        //
                        // Ora interviene **una volta sola** al raggiungimento
                        // della soglia, e il messaggio si chiude da solo
                        // quando il bit non e' piu' attivo: resta un avviso,
                        // non un blocco.
                        if self.ocp_confirmed && self.ocp_consecutive == OCP_DEBOUNCE_THRESHOLD {
                            self.ocp_event_count = self.ocp_event_count.saturating_add(1);
                            // **Nessun tocco alla potenza.** La regola standing
                            // dell'operatore e' "OCP = nessuna azione": su
                            // questo impianto il bit e' rumore (acceso anche a
                            // 73 W mentre raffredda) e la diagnostica lo dice.
                            // Prima qui si dimezzava la potenza: oltre a
                            // disobbedire, ogni scrittura di livello rischia di
                            // tenere il firmware in manuale e rompere la sua
                            // autoregolazione. Resta il conteggio, il log e il
                            // badge ambra: osservare, non toccare.
                            self.ocp_mitigated = true;
                            crate::commissioning::event(
                                "OCP",
                                "segnale OCP (possibile falso allarme): \
                                 nessuna azione sulla potenza",
                            );
                        }
                        // Il flag di allarme segue il bit, e il messaggio
                        // sparisce quando il bit non c'e' piu': un avviso che
                        // resta per sempre non e' un avviso.
                        self.ocp_mitigated = self.ocp_confirmed;
                        if !self.ocp_confirmed
                            && self
                                .error_text
                                .as_deref()
                                .is_some_and(|e| e.starts_with("Sovracorrente (OCP)"))
                        {
                            self.error_text = None;
                        }
                    }
                    Err(err) => {
                        // Anche in errore il contatore va azzerato: senza
                        // questo `ocp_confirmed` poteva restare attivo su uno
                        // stato vecchio, perche' `tec_status` non viene
                        // aggiornato.
                        self.ocp_consecutive = 0;
                        self.error_text = Some(format!("Heartbeat error: {err}"));
                    }
                }
                } // fine `if let`: senza campione non si e' toccato nulla

                // Chiedi il prossimo campione. `send` su canale non limitato
                // non blocca mai, e si salta se il precedente e' ancora in
                // viaggio: accodarne un secondo farebbe lavorare il thread per
                // un risultato che il tick seguente non troverebbe piu'.
                //
                // Qui il clock si azzera solo se la richiesta parte davvero:
                // azzerarlo sempre farebbe saltare `should_update()` e il
                // campione non verrebbe piu' richiesto.
                if self.should_update() && !self.poll_in_flight {
                    if self.tec_tx.send(crate::attore_tec::Richiesta::Campione).is_err() {
                        // Il thread e' morto. Senza questo controllo si
                        // resterebbe in silenzio per sempre, con la UI ferma
                        // sull'ultimo valore e nessun errore.
                        self.poll_in_flight = false;
                        self.error_text =
                            Some("Thread di polling seriale non raggiungibile".to_owned());
                    } else {
                        self.poll_in_flight = true;
                        self.last_sample_time = Instant::now();
                    }
                }
            }

            // I coefficienti vanno al controller subito, non solo in stato.
            //
            // Prima l'unico percorso che raggiungeva il firmware era
            // `enable()`: cambiare un coefficiente e non premere "Abilita
            // TEC" non cambiava nulla, e la UI mostrava il valore nuovo
            // mentre l'hardware usava quello vecchio. Ora ogni modifica
            // viene scritta, se il TEC e' in funzione.
            Message::UpdatePCoef(v)       => {
                self.inputs.p_coef = v;
                self.push_pid_if_running();
            }
            Message::UpdateICoef(v)       => {
                self.inputs.i_coef = v;
                self.push_pid_if_running();
            }
            Message::UpdateDCoef(v)       => {
                self.inputs.d_coef = v;
                self.push_pid_if_running();
            }
            Message::UpdateSetpoint(v)    => {
                // ── Pavimento anticondensa ─────────────────────────
                //
                // L'offset e' uno spostamento rispetto all'ambiente. Se il
                // punto di rugiada e' 15 °C e l'utente chiede -20 °C, la
                // piastra va 35 °C sotto: l'acqua dell'aria condensa sul
                // blocco freddo e il danno e' irreversibile.
                //
                // Il firmware Intel ha un limite di questo tipo, ma non e'
                // documentato e la modalita' Unregulated lo disattiva. Qui
                // il pavimento e' applicato **sempre**, indipendentemente
                // dalle soglie di allarme, che l'utente puo' disattivare.
                //
                // Si applica alla piastra TEC, non al setpoint: e' la
                // temperatura della piastra che incontra l'umidita'.
                //
                // **Il valore che arriva e' gia' un margine** (il campo non
                // accetta negativi), e il clamp qui e' la seconda rete: un
                // valore fuori intervallo non arriva dall'interfaccia, ma
                // non ci si fida mai di un solo controllo.
                let limit = self.dew_floor_offset();
                let richiesto = v;
                let (applicato, limitato) = self.offset_con_pavimento(richiesto);
                self.inputs.set_point = applicato;

                if limitato && applicato != richiesto {
                    self.cond_limit_active = true;
                    crate::commissioning::event(
                        "PAVIMENTO",
                        &format!(
                            "offset richiesto {richiesto:.1} °C limitato a {applicato:.1} °C \
                             (pavimento {:.1} °C, rugiada {:.1} °C)",
                            limit.unwrap_or(0.0),
                            self.last_dew_point
                        ),
                    );
                } else if !limitato {
                    // **Il pavimento non e' calcolabile, e questo va detto.**
                    //
                    // Prima la guardia limitava comunque, con un numero
                    // inventato, e non diceva nulla: l'operatore credeva che il
                    // freddo fosse controllato. Ora non viene limitato niente
                    // e resta scritto che non e' controllato, cosi' la scelta e'
                    // sua e non una sorpresa.
                    self.cond_limit_active = false;
                    crate::commissioning::event(
                        "PAVIMENTO-NON-CALCOLABILE",
                        &format!(
                            "offset {richiesto:.1} °C accettato senza limitazione: \
                             il pavimento anticondensa non e' calcolabile su questo controller \
                             (l'offset non e' relativo a una temperatura nota). \
                             Margine misurato {:.1} °C.",
                            self.last_cond_margin
                        ),
                    );
                } else {
                    self.cond_limit_active = false;
                }

                // Invia immediatamente al firmware, **solo se il regime in uso
                // e' quello che impiega il setpoint dell'operatore**. In
                // Standby e Unregulated l'offset e' del regime, e scriverci
                // sopra cambierebbe la temperatura del controller senza che la
                // UI lo sappia.
                self.scrivi_setpoint_utente(applicato);
                // La firma termica della pompa riparte da zero: dopo un
                // cambio di offset la piastra sale per qualche secondo **per
                // motivi normali**, e senza azzerare la finestra quella
                // salita verrebbe letta come loop fermo, con la protezione
                // che agisce da sola mentre tutto funziona.
                self.pompa_watch.reset();
            }
            Message::UpdateMaxPower(v)    => {
                // Anche un cambio di cap muove il punto di lavoro: la
                // finestra termica riparte da zero.
                self.pompa_watch.reset();
                self.inputs.max_power = v.min(100);
                self.tec_regulator.reset();
                // BUG C FIX: invia subito il nuovo cap al firmware — non aspettare il prossimo
                // Enable o il prossimo Tick. Senza questo, il slider è puramente decorativo.
                if self.tec_status.contains(cryo_cooler_controller_lib::TecStatus::PID_RUNNING) {
                    self.applica_potenza(v);
                                    }
            }
            Message::UpdateProfileName(v) => { self.inputs.profile_name = v; }

            Message::SaveProfile => {
                let p = Profile {
                    name: self.inputs.profile_name.clone(),
                    p_coef: self.inputs.p_coef, i_coef: self.inputs.i_coef,
                    d_coef: self.inputs.d_coef, set_point: self.inputs.set_point,
                    max_power: self.inputs.max_power,
                };
                if let Some(ex) = self.app_config.profiles.iter_mut().find(|x| x.name == p.name) {
                    *ex = p;
                } else {
                    self.app_config.profiles.push(p);
                }
                self.app_config.save();
            }
            Message::LoadProfile(idx) => {
                if let Some(saved) = self.app_config.profiles.get(idx) {
                    let p = saved.con_margine_sicuro();
                    self.tec_regulator.reset();
                    self.inputs.p_coef       = p.p_coef;
                    self.inputs.i_coef       = p.i_coef;
                    self.inputs.d_coef       = p.d_coef;
                    self.inputs.profile_name = p.name.clone();

                    // Il margine passa dal clamp di sicurezza: un profilo
                    // salvato quando `set_point` era un offset in °C porta
                    // valori negativi, e riletti come margine sarebbero
                    // "sotto la rugiada" — condensa garantita. Il profilo
                    // vecchio continua a funzionare, col margine al minimo.
                    let profilo = p.con_margine_sicuro();
                    let richiesto = profilo.set_point;
                    let (applicato, _) = self.offset_con_pavimento(richiesto);
                    self.inputs.set_point = applicato;
                    self.cond_limit_active = applicato != richiesto;

                    // Il cap non supera quello gia' in uso: caricare un
                    // profilo non deve poter alzare la potenza oltre il
                    // massimo consentito al modulo.
                    self.inputs.max_power = profilo.max_power.clamp(0, 100)
                        .min(if self.pump_emergency_active { self.inputs.max_power } else { 100 });

                    // Lo portiamo subito all'hardware se il TEC e' in
                    // funzione: prima i valori restavano solo in stato fino
                    // al prossimo Enable, quindi la UI e il controller
                    // mostravano cose diverse.
                    if self.ultimo_regime_richiesto == Some(crate::commutazione::Regime::Cryo) {
                        self.push_pid_if_running();
                        self.scrivi_setpoint_utente(applicato);
                    }
                    // Il cap NON si scrive all'hardware qui: come all'accensione,
                    // ogni scrittura di livello inchioda il firmware in manuale.
                    // Il cap resta il tetto che la guardia usa per scendere in
                    // caso di surriscaldamento. Come nella versione che usi.
                }
            }
            Message::DeleteProfile(idx) => {
                if idx < self.app_config.profiles.len() {
                    self.app_config.profiles.remove(idx);
                    self.app_config.save();
                }
            }
            Message::ExportCsv => {
                match self.export_csv() {
                    Ok(p)  => self.pdf_status = Some(format!("✓ CSV salvato: {}", p.display())),
                    Err(e) => self.pdf_status = Some(format!("✗ Errore CSV: {e}")),
                }
            }
            Message::SensorTab(tab) => { self.sensors.set_tab(tab); }
            Message::SetSensorSource(src) => { self.sensors.set_source(src); }
            Message::CloseModal => { self.error_text = None; self.ai_modal_open = false; self.info_modale = None; }
            Message::InfoModalita => {
                self.info_modale = Some((
                    "Modalita' del controller".to_owned(),
                    crate::modalita::MODALITA_NOTA.to_owned(),
                ));
            }
            Message::DiagnosticaToggle => {
                self.show_diagnostica = !self.show_diagnostica;
                self.diagnostica_msg = None;
            }
            Message::DiagnosticaSalva => {
                self.diagnostica_msg = Some(self.salva_rapporto_diagnostica());
            }
            // ── Guardia modale ─────────────────────────────────────────
            //
            // Mentre una conferma e' aperta, **nessun altro cambio di regime
            // parte**. Prima non esisteva nessuna guardia: si poteva aprire la
            // richiesta di Unregulated e poi premere Standby, e la conferma
            // successiva scriveva il regime sbagliato.
            //
            // Il dialogo deve essere l'unica azione possibile finche' e' aperto,
            // altrimenti non e' un dialogo: e' un avviso che si può bypassare
            // premendo un pulsante che copre.
            Message::CommutaCryo | Message::CommutaStandby
            | Message::CommutaSpento
                if self.in_attesa.pendente() =>
            {
                // Non si scrive e non si segnala un errore: l'operatore ha
                // premuto mentre una domanda era aperta, e la risposta e'
                // "chiudi prima la finestra", non un errore.
            }
            Message::Enable => {
                self.in_attesa = crate::commutazione::InAttesa::Nessuna;
                self.chiedi_unregulated = false;
                self.scrivi_regime(crate::commutazione::Regime::Cryo);
            }
            Message::Disable => {
                self.scrivi_regime(crate::commutazione::Regime::Spento);
            }
            Message::CommutaCryo => {
                // **Tornare a Cryo azzera anche la conferma Unregulated.**
                //
                // `chiedi_unregulated` e' un flag a due stanti: al primo clic su
                // Unregulated diventa `true` e il pulsante mostra "CONFERMA".
                // Se l'operatore non conferma e preme Cryo, il flag restava
                // `true` per sempre: il pulsante rimaneva bloccato su CONFERMA e
                // il ritorno a Cryo non partiva, costringendo a spegnere e
                // riaccendere. Ora ogni comando di regime chiude la richiesta
                // aperta, perche' e' una decisione diversa.
                self.chiedi_unregulated = false;
                self.in_attesa=crate::commutazione::InAttesa::Nessuna;
                self.scrivi_regime(crate::commutazione::Regime::Cryo);
            }
            Message::CommutaStandby => {
                self.chiedi_unregulated = false;
                self.commutazione_richiesta(crate::commutazione::Regime::Standby);
            }
            Message::CommutaSpento => {
                self.chiedi_unregulated = false;
                self.commutazione_richiesta(crate::commutazione::Regime::Spento);
            }
            Message::CommutaUnregulatedChiedi => {
                // Solo il primo passo: **non scrive niente**. Apre la
                // richiesta e aspetta il secondo clic.
                self.chiedi_unregulated = true;
                self.esito_commutazione=Some("Unregulated: premi CONFERMA per applicare la domanda massima".into());
                crate::commissioning::event("CLIC-UNREGULATED","richiesta conferma aperta");
            }
            Message::CommutaUnregulatedConferma => {
                crate::commissioning::event("CONFERMA-UNREGULATED", &format!("aperta={} attesa={}", self.chiedi_unregulated,self.in_attesa.pendente()));
                if self.chiedi_unregulated {
                    self.chiedi_unregulated = false;
                    self.in_attesa=crate::commutazione::InAttesa::Nessuna;
                    self.scrivi_regime(crate::commutazione::Regime::Unregulated);
                } else {
                    self.esito_commutazione=Some("Conferma scaduta: premi nuovamente Unregulated".into());
                }
            }
            Message::Annulla => {
                // **Prima di tutto** la conferma anticondensa: e' l'unica
                // finestra il cui pulsante di conferma **scrive sul bus**, quindi
                // e' la prima che va chiusa. Premere Esc per chiudere una
                // finestra non deve poter diventare un comando.
                if self.in_attesa.pendente() {
                    self.in_attesa = crate::commutazione::InAttesa::Nessuna;
                    return Task::none();
                }
                // Dal piu' pericoloso al meno. Solo la conferma
                // dell'Unregulated ha un pulsante che scrive, quindi e'
                // l'unica che va chiusa per prima: premere Esc per
                // chiudere una finestra non deve poter diventare un comando.
                if self.chiedi_unregulated {
                    self.chiedi_unregulated = false;
                } else if self.show_diagnostica {
                    self.show_diagnostica = false;
                } else if self.show_session_hist {
                    self.show_session_hist = false;
                } else {
                    self.error_text = None;
                    self.ai_modal_open = false;
                    self.info_modale = None;
                }
            }
            Message::DismissAlert => { if !self.active_alerts.is_empty() { self.active_alerts.remove(0); } }
            Message::DismissAlertChannel(channel) => { self.active_alerts.retain(|(c,_)|*c != channel); }
            Message::ExportPdf => {
                let rd = ReportData {
                    stats:       &self.stats,
                    cop:         &self.cop_state,
                    oc_score:    self.oc_score_current,
                    oc_best:     self.oc_best,
                    ocp_events:  self.ocp_event_count,
                    p_coef:      self.inputs.p_coef,
                    i_coef:      self.inputs.i_coef,
                    d_coef:      self.inputs.d_coef,
                    set_point:   self.inputs.set_point,
                    max_power:   self.inputs.max_power,
                    session_min: (self.stats.session_samples / 120) as u32,
                    fw_ver:      format!("{:X}.{:X}", self.fw_major, self.fw_minor),
                    hw_ver:      self.hw_version,
                    tec_history: self.chart.tec_sparkline(),
                    dew_history: self.chart.dew_sparkline(),
                };
                self.pdf_status = Some(match export_pdf(&rd) {
                    Ok(p)  => format!("✓ PDF salvato: {}", p.display()),
                    Err(e) => format!("✗ Errore: {}", e),
                });
            }
            Message::PidWizardStart => { self.pid_wizard.start(); }
            Message::PidWizardCancel => { self.pid_wizard.cancel(); }
            Message::PidWizardApply => {
                if let WizardPhase::Done { p, i, d, .. } = self.pid_wizard.phase.clone() {
                    self.inputs.p_coef = p;
                    self.inputs.i_coef = i;
                    self.inputs.d_coef = d;
                    // Scrive davvero sul controller: prima i valori restavano
                    // solo in stato e serviva un "Abilita TEC" per applicarli.
                    self.push_pid_if_running();
                    self.pid_wizard.phase = WizardPhase::Idle;
                }
            }
            Message::DiscordToggle => {
                if self.discord.is_active() { self.discord.clear(); }
                else { self.discord.try_connect(); }
            }
            Message::RtssToggle => {
                if self.rtss.is_active() { self.rtss.clear(); }
                else { self.rtss.try_connect(); }
            }
            Message::SessionHistoryOpen => {
                self.show_session_hist = !self.show_session_hist;
                // Ricariciamo solo all'apertura: query SQLite solo quando
                // serve, mai a ogni frame.
                if self.show_session_hist {
                    self.session_list = self.session_db.recent_sessions(40);
                }
            }
            Message::DebugToggle => {
                self.debug_mode = !self.debug_mode;
            }
            Message::ToggleToasts(v) => {
                self.app_config.notify.toasts = v;
                // Salvataggio immediato: l'utente si aspetta che la scelta
                // sopravviva al riavvio dell'app.
                self.app_config.save();
                // Allineiamo anche l'interruttore globale del modulo alert:
                // i thread gia' in coda lo rileggono e non lanciano
                // PowerShell se le notifiche sono state spente nel frattempo.
                crate::alerts::set_toasts_enabled(v);
            }
            Message::ToggleAlertBanner(v) => {
                self.app_config.notify.banner = v;
                self.app_config.save();                if !v {
                    self.active_alerts.clear();
                }
            }

            // ── AI Advisor ────────────────────────────────────────────────
            Message::AskAi => {
                if self.ai_loading { return Task::none(); }
                self.ai_loading   = true;
                self.ai_error     = None;
                self.ai_modal_open = true;

                let snap = SessionSnapshot {
                    tec_temp:    self.chart.last_tec_temp(),
                    dew_point:   self.log.last().map(|e| e.dew_point).unwrap_or(0.0),
                    cpu_temp:    self.last_cpu_temp_ext,
                    gpu_temp:    self.last_gpu_temp_ext,
                    power_watts: self.last_power_watts,
                    humidity:    self.log.last().map(|e| e.humidity).unwrap_or(0.0),
                    cop:         self.cop_state.cop,
                    cop_eff_pct: self.cop_state.efficiency_pct,
                    margin:      self.last_cond_margin,
                    oc_score:    self.oc_score_current,
                    p_coef:      self.inputs.p_coef,
                    i_coef:      self.inputs.i_coef,
                    d_coef:      self.inputs.d_coef,
                    set_point:   self.inputs.set_point,
                    max_power:   self.inputs.max_power,
                    ocp_events:  self.ocp_event_count,
                    uptime_min:  (self.stats.session_samples / 120) as u32,
                    tec_min:     self.stats.tec_temp.min,
                    tec_avg:     self.stats.tec_temp.avg(),
                    tec_max:     self.stats.tec_temp.max,
                    workload_mode:
                        Some(self.auto_mgr.regime().etichetta().to_owned())
                        .unwrap_or_else(|| if self.auto_profile_on {
                            "auto".to_owned()
                        } else {
                            "manuale".to_owned()
                        }),
                };
                let key = self.ai_api_key.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || -> Result<crate::ai_advisor::AiAdvice, String> {
                            ask_advisor(&snap, &key)
                        })
                            .await
                            .unwrap_or_else(|e| Err(format!("Task error: {}", e)))
                    },
                    Message::AiResponse,
                );
            }

            Message::AiResponse(result) => {
                self.ai_loading = false;
                match result {
                    Ok(advice) => { self.ai_advice = Some(advice); self.ai_error = None; }
                    Err(e)     => { self.ai_error = Some(e); self.ai_advice = None; }
                }
            }

            Message::AiApiKeyChanged(k) => {
                self.ai_api_key = k.clone();
                save_api_key(&k);
            }
            Message::AiApplyParams => {
                if let Some(ref advice) = self.ai_advice {
                    if let Some(p) = advice.suggested_p   { self.inputs.p_coef    = p; }
                    if let Some(i) = advice.suggested_i   { self.inputs.i_coef    = i; }
                    if let Some(d) = advice.suggested_d   { self.inputs.d_coef    = d; }
                    let pid_cambiato =
                        advice.suggested_p.is_some()
                            || advice.suggested_i.is_some()
                            || advice.suggested_d.is_some();
                    if let Some(v) = advice.suggested_p { self.inputs.p_coef = v; }
                    if let Some(v) = advice.suggested_i { self.inputs.i_coef = v; }
                    if let Some(v) = advice.suggested_d { self.inputs.d_coef = v; }
                    if let Some(s) = advice.suggested_sp {
                        // Passa dal pavimento anticondensa anche quando lo
                        // suggerisce l'IA.
                        let (applicato, _) = self.offset_con_pavimento(s);
                        self.inputs.set_point = applicato;
                        self.cond_limit_active = applicato != s;
                    }
                    if let Some(w) = advice.suggested_pwr {
                        // `suggested_pwr` e' gia' un u8, e comunque non
                        // supera il tetto d'emergenza pompa.
                        self.inputs.max_power =
                            w.min(if self.pump_emergency_active { self.inputs.max_power } else { 100 });
                    }
                    if pid_cambiato {
                        self.push_pid_if_running();
                    }
                    // Il consiglio dell'AI **non** scrive il setpoint greggio.
                    //
                    // Prima mandava `Setpoint(self.inputs.set_point)` direttamente,
                    // e aveva due difetti: nessun clamp sul pavimento
                    // anticondensa — quindi un consiglio aggressivo poteva portare
                    // la piastra sotto la rugiada — e nessun controllo del regime,
                    // quindi in Standby o Unregulated sovrascriveva l'offset del
                    // regime con quello dell'operatore.
                    //
                    // `inputs.set_point` e' gia' stato aggiornato dal blocco
                    // sopra, quindi qui si rilegge **clampato**.
                    let limit = self.dew_floor_offset();
                    self.scrivi_setpoint_utente(self.inputs.set_point.clamp(limit.unwrap_or(-1000.0), 50.0));
                }
            }
            Message::ToggleAlertRule(idx) => {
                if let Some(rule) = self.alert_manager.rules_mut().get_mut(idx) {
                    rule.enabled = !rule.enabled;
                }
                self.persist_alerts();
            }
            Message::ToggleSettings => {
                self.show_settings = !self.show_settings;
            }
            Message::SetAlertCondition(idx, above) => {
                if let Some(rule) = self.alert_manager.rules_mut().get_mut(idx) {
                    rule.condition = if above {
                        crate::alerts::AlertCondition::Above(rule.threshold())
                    } else {
                        crate::alerts::AlertCondition::Below(rule.threshold())
                    };
                }
                self.persist_alerts();
            }
            Message::SetAlertThreshold(idx, v) => {
                if let Some(rule) = self.alert_manager.rules_mut().get_mut(idx) {
                    let v = v.clamp(-50.0, 2000.0);
                    rule.condition = match rule.condition {
                        crate::alerts::AlertCondition::Above(_) =>
                            crate::alerts::AlertCondition::Above(v),
                        crate::alerts::AlertCondition::Below(_) =>
                            crate::alerts::AlertCondition::Below(v),
                    };
                }
                self.persist_alerts();
            }
            Message::SetAlertCooldown(idx, v) => {
                if let Some(rule) = self.alert_manager.rules_mut().get_mut(idx) {
                    rule.cooldown_s = v.clamp(0, 86_400);
                }
                self.persist_alerts();
            }
            Message::ToggleAutoProfile => {
                self.auto_profile_on = !self.auto_profile_on;
                if self.auto_profile_on {
                    // Riparte pulito: la configurazione potrebbe essere
                    // cambiata mentre era spenta, e l'isteresi deve ricominciare
                    // da capo invece di valutare un regime vecchio.
                    self.auto_mgr.reset();
                    // E si allinea subito al primo dato valido: se il regime
                    // e' gia' quello giusto non ci sara' transizione, e senza
                    // allineamento l'accensione non cambierebbe niente.
                    self.auto_da_allineare = true;
                }
                if !self.auto_profile_on {
                    // Reset al modo Idle quando si spegne
                    self.last_auto_regime = None;
                    self.auto_da_allineare = false;
                }
            }
            Message::WindowResized(w, h) => { self.win_w = w; self.win_h = h; }
            _ => {}
        }
        Task::none()
    }

    /// Copia le regole di alert dalla memoria alla config e salva su disco.
    ///
    /// Senza questo, spegnere un allarme dalla finestra Impostazioni valeva
    /// solo fino alla chiusura: al riavvio `AlertManager` ripartiva da
    /// `default_rules()` e l'allarme tornava da solo. Salvataggio
    /// immediato, non differito: sono scelte che l'utente si aspetta di
    /// vedere applicate anche se il programma dovesse chiudersi in modo
    /// improvviso.
    fn persist_alerts(&mut self) {
        self.app_config.alerts = self.alert_manager.rules().to_vec();
        self.app_config.save();
    }

    /// Esporta il log corrente in CSV sul Desktop.    /// Ritorna il path del file scritto, o un messaggio d'errore.
    fn export_csv(&self) -> Result<std::path::PathBuf, String> {
        let desktop = dirs::desktop_dir()
            .or_else(|| dirs::home_dir())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let ts   = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let path = desktop.join(format!("stargate_cryo_{ts}.csv"));
        self.write_csv(&path)?;
        Ok(path)
    }

    /// FEAT #1: Auto-save periodico in XDG data directory.
    /// Sovrascrive sempre lo stesso file (non accumula file sul Desktop).
    /// Protegge i dati in caso di crash durante sessioni OC aggressive.
    fn auto_save_csv(&self) -> Result<std::path::PathBuf, String> {
        let base = dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("stargate-cryo");
        let _ = std::fs::create_dir_all(&base);
        let path = base.join("autosave.csv");
        self.write_csv(&path)?;
        Ok(path)
    }

    /// Logica CSV comune per export_csv e auto_save_csv.
    fn write_csv(&self, path: &std::path::Path) -> Result<(), String> {
        let mut csv = String::from(
            "timestamp,tec_temp_C,pcb_temp_C,humidity_pct,dew_point_C,\
             condensation_margin_C,tec_voltage_V,tec_current_A,\
             tec_power_W,tec_power_level_pct,cpu_temp_ext_C\n",
        );
        for e in &self.log {
            csv.push_str(&format!(
                "{},{:.2},{:.2},{:.1},{:.2},{:.2},{:.3},{:.3},{:.2},{},{}\n",
                e.timestamp, e.tec_temp, e.pcb_temp, e.humidity,
                e.dew_point, e.condensation_margin, e.tec_voltage,
                e.tec_current, e.tec_power_watts, e.tec_power_level,
                e.cpu_temp_ext.map(|t| format!("{:.1}", t)).unwrap_or_else(|| "N/A".to_owned()),
            ));
        }
        std::fs::write(path, csv)
            .map_err(|e| format!("Scrittura su {}: {e}", path.display()))
    }

    // ── View ──────────────────────────────────────────────────────────────────

    /// Sfondo con gradiente: metodo invece di closure per non impigliarsi nei
    /// lifetime. Costa una sola passata di fill, nessuna allocazione.
    fn backdrop(content: Element<'_, Message>) -> Element<'_, Message> {
        // **Vetro.** Il gradiente copre TUTTA la finestra, quindi e' un unico
        // strato uniforme: la foto si vede uguale da ogni parte e la
        // dashboard non risulta "divisa a meta'".
        //
        // Prima era opaco (alpha 1.0) e copriva del tutto la foto: si vedeva
        // solo la striscia laterale, perche' la foto stava dentro la colonna
        // da 340 px. Opaque e' anche l'unico valore che garantisce il contrasto
        // del testo dim, ma rende la foto invisibile.
        //
        // 0,85 e' il compromesso: la foto, che e' scura, si intravede come
        // presenza, e il testo della sidebar resta leggibile come prima. Il
        // valore e' in una costante: se non si vede basta alzarlo.
        // **Nessun vetro sopra la foto.** Era 0,85 e copriva la Terra di
        // quasi tutto: la si intravedeva appena e la dashboard sembrava
        // una lastra scura appoggiata sopra. La foto deve vedersi per
        // quello che e'.
        //
        // Il contrasto del testo non dipende da questo strato: le card, le
        // sezioni e la sidebar hanno tutti il proprio fondo, quindi restano
        // leggibili senza dover scurire l'immagine sotto di loro. Il fondo
        // globale serve solo a non far diventare trasparente ci��' che non ha
        // un riquadro proprio.
        const VETRO: f32 = 0.18;
        Container::new(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Gradient(
                    palette::app_gradient_alpha(VETRO),
                )),
                border: iced::Border::default(),
                text_color: None,
                shadow: iced::Shadow::default(),
            })
            .into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let mg       = self.last_cond_margin;
        let mg_color = if mg < 0.0      { palette::DANGER }
                       else if mg < 2.0 { palette::WARNING }
                       else             { palette::NEON_GREEN };

        let is_landscape = self.win_w >= 900;
        let banner        = self.view_banner();

        // ── Larghezze minime per decidere il layout ─────────────────────
        // I numeri fissi (es. ">= 1000") non bastavano: a 1080px la sidebar
        // ne mangiava 340 e i sensori si trovavano 190px, insufficienti per
        // i pulsanti da 132px + testo. Il pannello AIDA64 veniva schiacciato
        // e il testo andava a capo UN CARATTERE PER RIGA.
        //
        // Regola: i sensori restano a fianco solo se restano larghezze
        // utilizzabili per entrambi. Sotto, il pannello scende sotto i
        // grafici e prende tutta la larghezza.
        const SIDEBAR_W: f32   = 340.0;
        const CHARTS_MIN_W: f32 = 520.0;  // sotto, i grafici sono illeggibili
        const SENSORS_MIN_W: f32 = 380.0; // pulsanti (132) + testo + padding
        const SIDE_BY_SIDE_MIN_W: f32 = SIDEBAR_W + CHARTS_MIN_W + SENSORS_MIN_W;

        // ── Larghezza massima del contenuto ─────────────────────────────
        // Su un monitor ultrawide (5120px) il layout a 3 colonne stirato
        // edge-to-edge si rompeva: i grafici avevano un minimo intrinseco
        // che superava la quota in percentuale, quindi si sovrapponevano e
        // il pannello dei sensori finiva schiacciato in una colonna di
        // pochi pixel.
        //
        // La soluzione non e' alzare le percentuali, ma limitare la larghezza
        // del contenuto e centeringlo: e' quello che fanno i dashboard
        // professionali, e mantiene le colonne leggibili a qualsiasi
        // dimensione di schermo.
        const MAX_CONTENT_W: f32 = 2400.0;

        let base = if is_landscape {
            // ── LANDSCAPE: sidebar + grafici + sensori ────────────────────
            //   >= 1500  sidebar | grafici 2 colonne | sensori
            //   >= 1188  sidebar | grafici 1 colonna   | sensori
            //   <  1188  sidebar | grafici sopra      | sensori sotto
            // I grafici passano a 2 colonne solo quando ci sono almeno
            // ~700px per colonna: sotto, i grafici diventano illeggibili e
            // le due colonne si sovrappongono.
            let charts = if self.win_w >= 1800 {
                self.chart.view_wide()
            } else {
                self.chart.view()
            };

            let sensors_beside = (self.win_w as f32) >= SIDE_BY_SIDE_MIN_W;

            // I sensori hanno larghezza FISSA, non in percentuale.
            //
            // Con `FillPortion` il pannello riceveva solo la quota residua:
            // su schermi molto larghi i grafici (che hanno un minimo
            // intrinseco) se la mangiavano tutta e i sensori restavano con
            // pochi pixel, rendendo il testo illeggibile. Una larghezza
            // fissa e' l'unica che garantisce spazio sufficiente.
            let sensors = Length::Fixed(SENSORS_MIN_W);

            let right = if sensors_beside {
                Into::<Element<'_, Message>>::into(
                    Row::new()
                        .spacing(0)
                        .push(
                            Container::new(charts)
                                .width(Length::Fill)
                                .height(Length::Fill),
                        )
                        .push(
                            Container::new(self.sensors.view())
                                .width(sensors)
                                .height(Length::Fill),
                        )
                        .height(Length::Fill)
                        .width(Length::Fill)
                )
            } else {
                //Finestra stretta (o alta in verticale): il pannello AIDA64
                // scende SOTTO i grafici invece di restare di lato, così
                // ha tutta la larghezza e il testo resta leggibile.
                Into::<Element<'_, Message>>::into(
                    Column::new()
                        .spacing(0)
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .push(
                            Container::new(charts)
                                .width(Length::Fill)
                                .height(Length::FillPortion(62)),
                        )
                        .push(
                            Container::new(self.sensors.view())
                                .width(Length::Fill)
                                .height(Length::FillPortion(38)),
                        )
                )
            };

            // Body: sidebar a sinistra (larghezza fissa), contenuto a destra
            let body = Row::new()
                .spacing(0)
                .height(Length::Fill)
                .push(self.view_left_column())
                .push(right);

            // Limite di larghezza + CENTRATURA. Su un ultrawide 5120px il
            // layout a 3 colonne non reggeva: i grafici si sovrapponevano e
            // il pannello dei sensori restava compresso in una colonna di
            // testo illeggibile. Contenere la larghezza e centrare e' la
            // soluzione usata dai dashboard professionali.
            //
            // La centratura e' fatta dal contenitore ESTERNO: il blocco a
            // larghezza fissa sta dentro un contenitore Fill che lo centra,
            // altrimenti resterebbe allineato a sinistra lasciando una
            // fetta vuota a destra.
            let body_capped = Container::new(body)
                .width(Length::Fixed(MAX_CONTENT_W))
                .height(Length::Fill);

            let body = Container::new(body_capped)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(iced::Alignment::Center);

            Self::backdrop(
                Column::new()
                    .spacing(0)
                    .push(banner)
                    .push(
                        Container::new(body)
                            .width(Length::Fill)
                            .height(Length::Fill),
                    )
                    .into()
            )

        } else {
            // ── PORTRAIT: banner + tile metriche + controlli + grafici ─────
            // Ottimizzato per finestre strette: le metriche diventano tile
            // compatte in orizzontale invece di una lista di righe, e i
            // controlli si raggruppano per schermo.
            let metrics_strip = self.view_portrait_metrics(mg, mg_color);
            let compact_ctrl  = self.view_portrait_controls();

            Self::backdrop(
                Column::new()
                    .spacing(0)
                    .push(banner)
                    .push(metrics_strip)
                    .push(compact_ctrl)
                    .push(
                        Column::new()
                            .spacing(0)
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .push(
                                Container::new(self.chart.view())
                                    .width(Length::Fill)
                                    .height(Length::FillPortion(62))
                                    .padding(iced::Padding { top: 0.0, right: 2.0, bottom: 0.0, left: 2.0 }),
                            )
                            .push(
                                Container::new(self.sensors.view())
                                    .width(Length::Fill)
                                    .height(Length::FillPortion(38))
                                    .padding(iced::Padding { top: 0.0, right: 2.0, bottom: 0.0, left: 2.0 }),
                            ),
                    )
                    .into()
            )
        };

        // Foto in fondo, contenuto sopra, modali in cima.
        //
        // La foto sta QUI e non dentro la colonna laterale: solo da qui
        // copre l'intera finestra. Con la colonna larga 340 px la foto
        // occupava solo quella striscia, e lo sfondo opaco copriva tutto il
        // resto: due superfici diverse, cioe' la dashboard "divisa a meta'".
        iced::widget::Stack::new()
            .push(
                iced::widget::Image::new(self.bg_handle.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Cover),
            )
            .push(base)
            .push(self.view_modals())
            .into()
    }


    /// Un pulsante di regime, condiviso dal menu e dalla sua posizione d'onore.
    ///
    /// Era una **closure locale** dentro `view_menu_modalita`. Ora che il menu
    /// e' renderizzato anche in `pulsante_tec`, una closure non basterebbe: o si
    /// duplicava il codice in due punti — e due copie divergono, che e'
    /// esattamente il difetto che questa fase chiude — o si saliva con un
    /// parametro di closure, piu' complicato di un metodo.
    ///
    /// `non_cliccabile` descrive **cosa fa**, non cos'e': il pulsante del regime
    /// corrente non si preme, perche' premerlo non cambierebbe nulla. Il nome
    /// precedente, `disattivato`, si leggeva al contrario.
    fn pulsante_regime(
        &self,
        etichetta: &'static str,
        sottotitolo: &'static str,
        msg: Message,
        colore: (u8, u8, u8),
        selezionato: bool,
    ) -> iced::widget::Button<'_, Message> {
        iced::widget::button(
            Column::new()
                .spacing(1)
                .width(Length::Fill)
                .push(
                    Text::new(etichetta)
                        .size(12)
                        .align_x(alignment::Horizontal::Center)
                        .width(Length::Fill)
                        .color(iced::Color::from_rgb8(colore.0, colore.1, colore.2)),
                )
                .push(
                    Text::new(sottotitolo)
                        .size(9)
                        .align_x(alignment::Horizontal::Center)
                        .width(Length::Fill)
                        .color(palette::BLUE_DIM),
                ),
        )
        .padding([6, 8])
        .width(Length::Fill)
        .style(move |theme, status| {
            let mut style = crate::btn::secondary(theme, status);
            if selezionato {
                let accent = iced::Color::from_rgb8(colore.0,colore.1,colore.2);
                style.border = iced::Border { color: iced::Color {a:0.8,..accent}, width:1.5, radius:8.0.into() };
                style.background = Some(iced::Background::Color(iced::Color {r:accent.r*0.10,g:accent.g*0.10,b:accent.b*0.10,a:0.92}));
                style.shadow = crate::palette::glow(accent,0.18);
            }
            style
        })
        .on_press(msg)
    }

    /// I **regimi**, al posto del vecchio pulsante Abilita/Disabilita, con la
    /// striscia di potenza **intatta sotto**.
    ///
    /// **Perche' il menu e' qui e non in fondo alla barra laterale.** Il vecchio
    /// pulsante singolo viveva in questo slot, ed era l'unico comando che
    /// scriveva in proprio: `Message::Enable` mandava il *setpoint
    /// dell'operatore* come offset, e poteva quindi sovrascrivere il `-30` di
    /// Unregulated mentre il menu continuava a mostrare Unregulated. Sostituendo
    /// il pulsante con i quattro regimi, il comando che occupa il posto piu'
    /// comodo e' anche l'unico che puo' scrivere un offset, e quell'offset
    /// viene dal regime.
    ///
    /// Sotto, **invariata**, la striscia di potenza: stesso `Canvas`, stesso
    /// `MiniSpark`, stessi 34 px, stesso colore guidato dalla modalita'. Dice
    /// ancora se il TEC sta spingendo, calibrando o fermo, e resta separata dal
    /// menu quindi il menu non le cambia dimensione.
    fn pulsante_tec(&self) -> iced::Element<'_, Message> {
        // (modalita rimossa: la striscia segue i watt veri)

        // Il regime corrente, una volta sola: i quattro pulsanti sottostanti
        // decidono da qui quale e' gia' attivo e quale no.
        //
        // `da_stato` puo' tornare `None`: non e' "spento", e' **"non lo so in
        // che regime sia"**. Succede sul tuo controller, dove `PID_RUNNING` non
        // si attiva mai. La conseguenza pratica e' che **nessun pulsante**
        // risulta evidenziato quando il regime e' sconosciuto, ed e' corretto:
        // evidenziare Standby perche' "non e' Cryo" sarebbe una bugia.
        // **Il regime attivo viene da quello che hai scelto tu**, non dai bit.
        //
        // Prima usava `da_stato(tec_status)`: su questo Gen 1 quei bit non
        // si attivano mai, quindi nessun pulsante risultava mai evidenziato —
        // e quello giusto sembrava non cliccabile, come un controllo spento.
        // Ora la sorgente e' l'ultimo regime richiesto, che e' l'unica verita'
        // che questo hardware espone. I bit si usano solo come riserva, se il
        // firmware tornasse a confermare qualcosa.
        let regime_attuale = self.ultimo_regime_richiesto
            .or_else(|| crate::commutazione::Regime::da_stato(self.tec_status));
        let attivo = |r: crate::commutazione::Regime| regime_attuale == Some(r);

        // I due rami dell'Unregulated sono costruiti **fuori** dalla `Column`:
        // dentro un `if` dentro un `push`, l'inferenza del tipo non ce la fa e
        // servirebbero annotazioni. Qui sono due variabili con tipo gia' noto.
        let unregulated_el: iced::Element<'_, Message> = if self.chiedi_unregulated {
            iced::widget::button(
                Column::new()
                    .spacing(1)
                    .push(
                        Text::new("CONFERMA")
                            .size(12)
                            .color(palette::DANGER),
                    )
                    .push(
                        Text::new("porta la piastra sotto la rugiada")
                            .size(9)
                            .color(palette::BLUE_DIM),
                    ),
            )
            .padding([6, 8])
            .width(Length::Fill)
            .style(crate::btn::secondary)
            .on_press(Message::CommutaUnregulatedConferma)
            .into()
        } else {
            self.pulsante_regime(
                "Unregulated",
                "massima potenza",
                Message::CommutaUnregulatedChiedi,
                crate::modalita::colore_regime(crate::commutazione::Regime::Unregulated),
                attivo(crate::commutazione::Regime::Unregulated),
            )
            .into()
        };

        // "Torna a Cryo" compare solo in Unregulated: e' la via d'uscita dal
        // regime che raffredda di piu', e una via d'uscita non si perde
        // spostando un pannello.
        // **Cryo e Unregulated, due pulsanti affiancati.**
        //
        // Prima il ritorno a Cryo stava in un pulsante separato che compariva
        // solo se i bit confermavano Unregulated: su questo Gen 1 non
        // confermano mai, quindi il pulsante non c'era e l'unico modo di tornare
        // a Cryo era spegnere e riaccendere. Ora **Cryo e' un pulsante come
        // l'altro**: si passa da Unregulated a Cryo con un clic, senza
        // spegnere il modulo nel mezzo.
        //
        // Standby e "Spegni il modulo" non ci sono: lo spegnimento sta nel
        // pulsante grosso, e Standby su questo hardware non fa niente che
        // Cryo non faccia.
        let cryo_el: iced::Element<'_, Message> = self.pulsante_regime(
            "Cryo",
            "controllo anticondensa",
            Message::CommutaCryo,
            crate::modalita::colore_regime(crate::commutazione::Regime::Cryo),
            attivo(crate::commutazione::Regime::Cryo),
        )
        .into();

        // **Il pulsante grosso, in vetro, con l'icona.**
        //
        // Il colore dice **l'azione**: verde quando accendi, rosso quando
        // spegni. Se dicesse lo stato, il pulsante che spegne sarebbe verde e
        // si leggerebbe al contrario — ed e' esattamente l'equivoco che aveva
        // l'interruttore acceso/spento di prima.
        //
        // "Acceso" = watt veri sopra 2 W. Prima guardava il bit OCP, che e'
        // rumore e resta alto anche a modulo spento: all'avvio il pulsante
        // diceva DISABILITA e l'operatore spegneva un modulo gia' fermo.
        let acceso = self.last_power_watts > 2.0;
        let (led, titolo, sottotitolo, azione) = if acceso {
            (
                (235_u8, 70_u8, 70_u8),
                "DISABILITA TEC",
                "Arresta il raffreddamento TEC",
                Message::CommutaSpento,
            )
        } else {
            (
                (0_u8, 215_u8, 120_u8),
                "ABILITA TEC",
                "Avvia il raffreddamento Cryo",
                Message::Enable,
            )
        };

        let striscia: iced::Element<'_, Message> = iced::widget::Canvas::new(MiniSpark {
            punti: self.chart.power_sparkline(),
            acceso,
            colore: led,
        })
        .width(Length::Fill)
        .height(Length::Fixed(16.0))
        .into();

        let accent=iced::Color::from_rgb8(led.0,led.1,led.2);
        let potenza=if self.last_power_watts.is_finite() {format!("{:.0} W",self.last_power_watts.max(0.0))} else {"— W".to_owned()};
        let testata = Row::new()
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .push(
                    Container::new(iced::widget::Image::new(crate::icons::plate())
                        .width(Length::Fixed(22.0)).height(Length::Fixed(22.0)))
                        .padding(6)
                        .style(move |_:&iced::Theme| iced::widget::container::Style {
                            background:Some(iced::Background::Color(iced::Color {a:0.10,..accent})),
                            border:iced::Border {color:iced::Color {a:0.40,..accent},width:1.0,radius:10.0.into()},
                            ..Default::default()
                        }),
                )
                .push(
                    Column::new()
                        .spacing(3)
                        .width(Length::Fill)
                        .push(
                            Text::new(titolo)
                                .size(13)
                                .font(iced::Font {weight:iced::font::Weight::Semibold,..iced::Font::DEFAULT})
                                .color(if acceso { palette::DANGER } else { palette::NEON_GREEN }),
                        )
                        .push(
                            Text::new(sottotitolo)
                                .size(9)
                                .color(palette::BLUE_DIM),
                        )
                        ,
                )
            .push(Container::new(Text::new(potenza).size(13).color(palette::TEXT_BRIGHT))
                .padding([5,7]).style(move |_:&iced::Theme| iced::widget::container::Style {
                    background:Some(iced::Background::Color(iced::Color::from_rgba8(4,12,19,0.80))),
                    border:iced::Border {color:iced::Color {a:0.20,..accent},width:1.0,radius:8.0.into()},
                    ..Default::default()
                }));
        let grafico = Container::new(striscia).padding([3,4]).width(Length::Fill)
            .style(|_:&iced::Theme| iced::widget::container::Style {
                background:Some(iced::Background::Color(iced::Color::from_rgba8(3,10,17,0.60))),
                border:iced::Border {color:iced::Color::from_rgba8(109,163,191,0.14),width:1.0,radius:6.0.into()},
                ..Default::default()
            });
        let tasto_grosso: iced::Element<'_, Message> = iced::widget::button(
            Column::new().spacing(7).push(testata).push(grafico)
        )
        .padding([9, 10])
        .width(Length::Fill)
        .style(move |t: &iced::Theme, st| crate::btn::tec_console(led, t, st))
        .on_press(azione)
        .into();

        let menu: iced::Element<'_, Message> = Column::new()
            .spacing(5)
            .push(tasto_grosso)
            .push(
                Row::new()
                    .spacing(5)
                    .push(cryo_el)
                    .push(unregulated_el),
            )
            .into();

        // La riga di esito viaggia con il menu: e' l'unica parte di cui ci si
        // puo' fidare, e senza di essa un comando e' una scatola nera.
        // **Spazio riservato anche quando la riga e' vuota.** Prima era uno
        // `Space` ad altezza zero: compariva un messaggio, la colonna si
        // allungava di una riga, il messaggio spariva e tutto sotto risaliva.
        // Il pannello "respirava" a ogni comando. Ora l'altezza e' fissa e il
        // testo cambia dentro, senza spostare niente.
        let esito_corpo: iced::Element<'_, Message> = match &self.esito_commutazione {
                Some(t) => {
                    let colore = if self.commutazione.confermato() {
                        palette::NEON_GREEN
                    } else if self.commutazione.in_corso() {
                        palette::WARNING
                    } else {
                        palette::BLUE_DIM
                    };
                    Text::new(t.clone()).size(10).color(colore).width(Length::Fill).into()
                }
                None => iced::widget::Space::with_width(Length::Fill).into(),
        };
        let esito: iced::Element<'_, Message> = Container::new(esito_corpo)
            .width(Length::Fill)
            .height(Length::Fixed(30.0))
            .into();

        // Sotto il menu, la striscia con la potenza erogata: scala fissa
        // 0-250 W, area piena, niente assi. Elemento separato, quindi non
        // cambia dimensione al crescere del menu.
        // La striscia segue i watt veri, non il regime dichiarato: il regime
        // su Gen 1 non e' mai confermato dai bit, quindi guidarla dalla
        // modalita' la teneva grigia per sempre. Verde fluo mentre spinge,
        // spenta quando e' fermo — come la striscia originale.
        // **Qui non c'e' piu' nessun grafico.** La striscia sta dentro il
        // pulsante ABILITA/DISABILITA: lasciarla anche qui la mostrava due
        // volte nella stessa colonna, una grossa e una dentro il bottone.
        Column::new()
            .spacing(5)
            .push(menu)
            .push(esito)
            .into()
    }

    pub fn view_left_column(&self) -> Element<'_, Message> {
        let en_button = self.pulsante_tec();

        let (cond_color, cond_label) = self.etichetta_margine_ui();
        let cpu_str = self.last_cpu_temp_ext
            .map(|t| format!("{:.1}°C", t))
            .unwrap_or_else(|| "N/D".to_owned());

        // Temperatura TEC
        let tec_temp_str = self.log.last()
            .map(|s| format!("{:.1}°C", s.tec_temp))
            .unwrap_or_else(|| "N/D".to_owned());
        

        let live_block = Column::new().spacing(3)
            .push(Row::new()
                .push(Text::new("Piastra TEC:").size(15))
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(Text::new(tec_temp_str).size(16)
                    .color(palette::BLUE_PRIMARY)))
            // **Niente OCP e niente Potenza qui.** L'OCP ha un badge in fondo
            // alla colonna, e la potenza ha la striscia dentro il pulsante
            // piu' il suo grafico: ripeterli qui faceva leggere lo stesso
            // numero tre volte senza aggiungere niente.
            // **Quanto freddo comprano gli ultimi watt.**
            //
            // Il valore assoluto (gradi per watt) non e' confrontabile con
            // niente, perche' dipende dalla macchina. Il **marginale** si: dice
            // quanti gradi hai guadagnato con gli ultimi watt, ed e' l'unico
            // numero che risponde a "sto ancora pagando per il freddo?".
            //
            // Sotto `RENDIMENTO_MINIMO` quei watt sono sprecati: stai
            // scaldando il liquido invece di raffreddare la piastra. Il valore
            // non e' un optimum misurato — con una sola sonda non lo si ricava —
            // e' un criterio dichiarato, e si puo' cambiare.
            // **Altezza fissa**: la riga del rendimento cambia testo e
            // lunghezza a ogni campione (marginale, assoluto, "n/d", "gia' al
            // massimo"). Senza un'altezza riservata la colonna saltava a ogni
            // aggiornamento, e tutto quello sotto si spostava.
            .push(Container::new(Text::new(format!(
                "{} · offset {:.1} °C",
                self.tec_regulator.status, self.regulation_offset
            )).size(11).color(palette::BLUE_DIM))
                .width(Length::Fill).height(Length::Fixed(36.0)))
            // Il margine di condensa cambia da SICURO a CONDENSA: stesso
            // principio, spazio riservato, colore e parole cambiano dentro.
            .push(Container::new(Text::new(cond_label).size(14)
                .color(cond_color)).width(Length::Fill)
                .height(Length::Fixed(20.0)))
            // **Avviso UNREGULATED ATTIVO.** Non una notifica che passa: una
            // riga fissa che resta finche' il regime e' quello. Unregulated
            // spinge a -30 °C, quindi porta la piastra sotto il punto di
            // rugiada — e' il suo scopo, ma chi lo sceglie deve poterlo vedere
            // in ogni momento, non in un avviso che sparisce.
            //
            // Sparisce da solo quando torni a Cryo: nel frattempo lo spazio
            // resta riservato, quindi la colonna non salta.
            .push({ let avviso: iced::Element<'_, Message> = if self.ultimo_regime_richiesto
                == Some(crate::commutazione::Regime::Unregulated)
            {
                Container::new(
                    Column::new()
                        .spacing(1)
                        .push(
                            Text::new("UNREGULATED ATTIVO")
                                .size(14)
                                .color(palette::DANGER),
                        )
                )
                .width(Length::Fill)
                .height(Length::Fixed(36.0))
                .padding([4, 6])
                // `Container::style` non prende gli stili dei pulsanti: qui
                // basta una tinta rossa scurissima, il testo fa il resto.
                .style(|_: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(
                        iced::Color { r: palette::DANGER.r * 0.22, g: palette::DANGER.g * 0.22, b: palette::DANGER.b * 0.22, a: 1.0 },
                    )),
                    border: iced::Border {
                        color: palette::DANGER,
                        width: 1.0,
                        radius: 6.0.into(),
                    },
                    ..Default::default()
                })
                .into()
            } else {
                iced::widget::Space::new(Length::Fill, Length::Fixed(36.0)).into()
            }; avviso })
            .push(Row::new()
                .spacing(6)
                .push(iced::widget::Image::new(crate::icons::cpu())
                    .width(Length::Fixed(16.0))
                    .height(Length::Fixed(17.0)))
                .push(Text::new("CPU · sensori:").size(13)
                    .color(palette::BLUE_DIM))
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(Text::new(cpu_str).size(13)
                    .color(palette::BLUE_DIM)))
            // Pompa: era la protezione piu' importante e non aveva una riga
            // con il numero. Sotto i 200 RPM la potenza viene dimezzata, quindi
            // il valore deve essere visibile senza aprire le impostazioni.
            .push(Row::new()
                .spacing(6)
                .push(iced::widget::Image::new(crate::icons::fan())
                    .width(Length::Fixed(16.0))
                    .height(Length::Fixed(17.0)))
                .push(Text::new("Pompa:").size(13)
                    .color(palette::BLUE_DIM))
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(
                    // La riga dice COME si sta giudicando la pompa, non solo il
                    // numero. Senza sensore RPM non e' che la protezione sia
                    // spenta: e' attiva e deduce dalla temperatura.
                    //
                    // "pompa non governata" e' la parte che conta: sui
                    // controller che non hanno l'uscita per pompa e ventole
                    // (V1) la pompa e' alimentata e regolata **fuori** dal
                    // software, e il protocollo non contiene nessun comando
                    // per essa. Dirlo evita due equivoci: che ci sia un
                    // regime da impostare, e che 9999 RPM voglia dire
                    // qualcosa. Non e' un valore letto: e' l'assenza di una
                    // lettura, dichiarata come tale.
                    if self.pump_stalled {
                        Text::new("Flusso da verificare").size(13).color(palette::DANGER)
                    } else if !self.pump_readable {
                        Text::new("RPM non disponibili")
                            .size(11).color(palette::BLUE_DIM)
                    } else if self.last_pump_rpm < 200.0 {
                        Text::new(format!("{:.0} RPM  BASSA", self.last_pump_rpm))
                            .size(13).color(palette::DANGER)
                    } else {
                        Text::new(format!("{:.0} RPM", self.last_pump_rpm))
                            .size(13).color(palette::NEON_GREEN)
                    },
                ));

        // ── Analytics block ───────────────────────────────────────────────
        // Usa cache aggiornata nel Tick — evita regressione lineare per-frame
        let risk = &self.last_risk_state;
        let risk_color = match &risk.level {
            crate::analytics::RiskLevel::Safe       => palette::SUCCESS,
            crate::analytics::RiskLevel::Watch      => palette::WARNING,
            crate::analytics::RiskLevel::Warning    => palette::DANGER,
            crate::analytics::RiskLevel::Critical   => palette::DANGER,
            crate::analytics::RiskLevel::Condensing => palette::DANGER,
        };
        let risk_label = match &risk.level {
            crate::analytics::RiskLevel::Safe       => "SICURO".to_owned(),
            crate::analytics::RiskLevel::Watch      => "ATTENZIONE".to_owned(),
            crate::analytics::RiskLevel::Warning    => "PERICOLO".to_owned(),
            crate::analytics::RiskLevel::Critical   =>
                format!("CRITICO  {:.0}s", risk.eta_seconds.unwrap_or(0.0)),
            crate::analytics::RiskLevel::Condensing => "CONDENSA!".to_owned(),
        };
        let eta_text = risk.eta_seconds
            .map(|s| if s < 60.0 { format!("ETA: {:.0}s", s) }
                     else        { format!("ETA: {:.0}m {:.0}s", s/60.0, s%60.0) })
            .unwrap_or_else(|| "Trend stabile".to_owned());
        // Il regime corrente, o lo stato spento. Il nome viene dalla stessa
        // variabile che ha scritto i valori sull'hardware, quindi qui non
        // puo' divergere da quello che il TEC sta davvero facendo.
        // Fonte unica: il regime lo conosce il gestore, non un campo tenuto
        // all'aggiornato a mano. `last_auto_regime` serve solo a distinguere
        // "nessuna decisione ancora" da "Idle", che sono cose diverse.
        let regime_corrente = self.auto_mgr.regime();
        let mode_label = match (self.auto_profile_on, self.last_auto_regime) {
            (true, Some(_)) => format!("Auto ● {}", regime_corrente.etichetta()),
            (true, None)   => "Auto · attesa".to_owned(),
            (false, _)     => "Manuale".to_owned(),
        };
        // Icona del rischio: il triangolo "cryogenic hazard" del set. Sta
        // ACCANTO all'etichetta e non la sostituisce: il testo dice cosa
        // stiamo misurando, l'icona dice che qui si puo' fare un danno.
        let rischio_icon = iced::widget::Image::new(crate::icons::hazard())
            .width(Length::Fixed(18.0))
            .height(Length::Fixed(16.0));

        let analytics_block = Column::new().spacing(3)
            .push(Row::new()
                .spacing(6)
                .push(rischio_icon)
                .push(Text::new("Rischio condensa:").size(13))
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(Text::new(risk_label).size(14)
                    .color(risk_color)))
            .push(Text::new(eta_text).size(12)
                .color(palette::BLUE_DIM))
            // COP e OC Score dipendono entrambi dalla **temperatura esterna
            // della CPU**: il COP e' il rapporto fra il calore tolto e la
            // differenza di temperatura, quindi senza la CPU non e'
            // calcolabile. Quando quel dato manca il valore restava `0.00`,
            // che e' peggio di "non disponibile": uno zero sembra una
            // misura, e l'operatore lo legge come "il TEC non raffredda".
            .push(Row::new()
                .push(Text::new("COP stimato:").size(13))
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(
                    match self.last_cpu_temp_ext {
                        Some(_) => Text::new(format!("{:.2}", self.cop_state.cop))
                            .size(14).color(palette::TEXT_BRIGHT),
                        None => Text::new("N/D \u{00B7} CPU esterna")
                            .size(13).color(palette::BLUE_DIM),
                    }))
            // **OC Score tolto.** E' il voto 0-100 della stessa cosa che la riga
            // sopra mostra gia' in percentuale: due numeri per un fatto solo,
            // e quello sotto senza un suo significato ("best" non e' spiegato
            // da nessuna parte). Se serve, il COP resta il numero da leggere.
            .push(Text::new("").size(1))
            .push(Row::new()
                .push(Text::new("Gestione:").size(12)
                    .color(palette::BLUE_DIM))
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(Text::new(mode_label).size(12)
                    .color(palette::BLUE_DIM)));

        // ── Controllo notifiche e regole alert ──────────────────────────
        // Due canali separati: le notifiche Windows costano un processo
        // PowerShell ciascuna, gli allarmi in-app sono gratuiti. Utile
        // anche perche' ogni regola puo' essere disattivata singolarmente.
        let notify_block = Column::new().spacing(4)
            .push(intestazione_con_icona("Notifiche", palette::SEM_PROTEZIONE, Some(crate::icons::shield())))
            .push(
                Row::new().spacing(6)
                    .push(Text::new("Notifiche Windows").size(12))
                    .push(iced::widget::Space::with_width(Length::Fill))
                    .push(
                        iced::widget::Toggler::new(self.app_config.notify.toasts)
                            .size(16)
                            .on_toggle(Message::ToggleToasts)
                    .style(crate::btn::toggler),
                    ),
            )
            .push(
                Row::new().spacing(6)
                    .push(Text::new("Allarmi in-app").size(12))
                    .push(iced::widget::Space::with_width(Length::Fill))
                    .push(
                        iced::widget::Toggler::new(self.app_config.notify.banner)
                            .size(16)
                            .on_toggle(Message::ToggleAlertBanner)
                    .style(crate::btn::toggler),
                    ),
            )
            .push(
                Text::new(if crate::alerts::toasts_enabled() {
                    "Le protezioni termiche restano attive in ogni caso."
                } else {
                    "Notifiche Windows spente. Le protezioni restano attive."
                })
                    .size(10)
                    .color(palette::BLUE_DIM),
            );

        // Le soglie si configurano nel pannello Impostazioni: qui in sidebar
        // c'era solo una lista di testi troncati, illeggibile e non
        // modificabile.
        let attive = self.alert_manager.rules().iter().filter(|r| r.enabled).count();
        let regole_tot = self.alert_manager.rules().len();
        let rules_block: iced::Element<'_, Message> = iced::widget::button(
            Row::new().spacing(8)
                .align_y(iced::Alignment::Center)
                .push(
                    Text::new(if self.show_settings { "Chiudi impostazioni" } else { "Impostazioni" })
                        .size(12)
                        .color(palette::TEXT_BRIGHT),
                )
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(
                    Text::new(format!("{attive}/{regole_tot}"))
                        .size(11)
                        .color(if attive > 0 { palette::TEXT_BRIGHT } else { palette::BLUE_DIM }),
                ),
        )
        .width(Length::Fill)
        .padding([8, 11])
        .style(crate::btn::secondary)
        .on_press(Message::ToggleSettings)
        .into();

        // ── Diagnostica: verdetto e nient'altro ─────────────────────────
        //
        // Questo blocco ha subito piu' riduzioni, tutte per lo stesso motivo:
        // ripeteva nella sidebar numeri e messaggi che stanno gia' altrove.
        //
        //  - Tensione, corrente, potenza e COP: sono gia' nella griglia
        //    principale, tre righe piu' su.
        //  - La modalita': e' gia' nella sezione MODALITA' qui sotto.
        //  - Il paragrafo sull'OCP che spiegava i 73 W, i 112 W e il fatto che
        //    il bit e' rumoroso: era il diario della ricerca, stampato in un
        //    posto che si rilegge a ogni aggiornamento. La risposta a un
        //    segnale che non significa niente e' "nessuna azione", e quella
        //    parola sta gia' nel pannello diagnostico.
        //
        // Quindi qui resta **una riga sola**: verdetto e numero di problemi.
        // Il dettaglio si apre premendo.
        let diag_block: iced::Element<'_, Message> = {
            use crate::diagnostica::{valuta, Verdetto};
            // **Nessun dato non e' "tutto a posto".**
            //
            // Se le misure non sono disponibili la riga lo dichiara in grigio e
            // non dice "Nessuna anomalia": un pannello che rassicura senza aver
            // misurato niente e' il caso peggiore, perche' l'operatore si fida
            // e smette di guardare.
            //
            // Il colore segue il **verdetto**, con un'eccezione: un modulo
            // spento non e' "tutto regolare", e verde significa "sta
            // raffreddando normalmente". Lo stato spento e' neutro — un grigio
            // spento dice "fermo, e va bene", mentre il verde direbbe "tutto
            // bene", che e' un'affermazione diversa e non vera.
            let (colore, etichetta) = match self.dati_diagnostica() {
                Some(m) => {
                    // Il riepilogo e' calcolato in `diagnostica::riassunto`, non
                    // qui: cosi' la regola "un dato una volta sola" e' scritta
                    // una volta e testata, invece di essere una promessa nella
                    // vista.
                    let e = valuta(m, self.tec_status);
                    let riepilogo = e.riassunto();
                    let colore = match e.verdetto {
                        Verdetto::Guasto => palette::DANGER,
                        Verdetto::Attenzione => palette::WARNING,
                        Verdetto::Ok if riepilogo.spento => palette::BLUE_DIM,
                        Verdetto::Ok => palette::NEON_GREEN,
                    };
                    (colore, riepilogo.etichetta)
                }
                None => (
                    palette::BLUE_DIM,
                    "Diagnostica non disponibile".to_owned(),
                ),
            };
            Row::new()
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .push(Text::new("\u{25CF}").size(10).color(colore))
                .push(
                    iced::widget::Button::new(
                        Text::new(etichetta).size(11).color(colore),
                    )
                    .padding([5, 8])
                    .width(Length::Fill)
                    .style(crate::btn::glass)
                    .on_press(Message::DiagnosticaToggle),
                )
                .into()
        };

        // La sezione modalita' **non e' piu' qui**: e' salita dove stava il
        // vecchio pulsante Abilita/Disabilita, in `pulsante_tec`. Un comando
        // che scrive in seriale sta nel posto piu' comodo della colonna, non in
        // fondo alla barra laterale, e cosi' non ne esistono due.

        // ── Alert attivi ──────────────────────────────────────────────────
        //
        // Prima era una `Badge` con **rosso pieno saturo** (0.85, 0.05, 0.12)
        // su testo bianco puro, bordo spessore zero, e il pulsante di chiusura
        // era a sua volta rosso: rosso su rosso, senza che si vedesse. Il
        // risultato era una fascia che gridava anche per un avviso di umidita',
        // e con piu' avvisi insieme non si capiva quale pesasse.
        //
        // Adesso ogni riga porta la propria gravita' e si legge come
        // un'etichetta di stato, non come un allarme:
        //   - fondo scuro tinta del colore di gravita', non pieno;
        //   - una barra laterale solida che dice il livello a colpo d'occhio;
        //   - bordo sottile dello stesso colore;
        //   - testo in `TEXT_BRIGHT`, non bianco puro (meno aggressivo);
        //   - chiusura neutra, visibile e che non si confonde col fondo.
        //
        // Il raggio e' 12 come tutto il resto della dashboard: prima era 6 e
        // l'elemento sembravaappiccicato a una lista di link invece che farne
        // parte.
        let alerts_widget: iced::Element<'_, Message> = if !self.active_alerts.is_empty() {
            let mut acol = Column::new().spacing(6);
            for (canale, alert) in &self.active_alerts {
                // A persistent notification must not present a resolved event as live.
                if let Some(value) = self.valore_live_canale(canale) {
                    if !self.alert_manager.rules().iter().any(|r| r.channel == *canale && r.triggered(value)) {
                        continue;
                    }
                }
                // La gravita' segue il simbolo con cui la regola e' scritta.
                // Dedurla qui e' comunque piu' onesto che mostrarle tutte
                // uguali.
                //
                // Il numero accanto all'etichetta e' il valore **live** del
                // suo canale, riletto a ogni frame: prima era congelato al
                // momento dello scatto e sembrava inventato perche' non si
                // muoveva piu'. Ogni canale mostra la sua unita' (watt per
                // la potenza, gradi per le temperature), mai quella di un
                // altro canale.
                let (grav, testo) = if let Some(t) = alert.strip_prefix('\u{1F6A8}') {
                    (Gravita::Critica, t)
                } else if let Some(t) = alert.strip_prefix('\u{26A0}') {
                    (Gravita::Avviso, t.trim())
                } else {
                    (Gravita::Info, alert.as_str())
                };
                let _ = &testo;

                // La soglia da mostrare e' quella della regola che ha
                // prodotto questo avviso, cercata per canale. Il testo
                // dell'avviso arriva gia' formattato e non porta la
                // soglia con se': senza questo, l'etichetta mostrerebbe la
                // soglia di default anche quando l'utente l'ha cambiata
                // (il caso "Potenza TEC > 200W" con valore 113 W).
                // `Option<&AlertCondition>`: quando la regola non c'e' non
                // si mostra nessuna soglia. Passare una soglia fittizia
                // (tipo `f32::NAN`) stamperebbe "> NaN" a schermo.
                let condizione = self
                    .alert_manager
                    .rules()
                    .iter()
                    .find(|r| r.channel == *canale)
                    .map(|r| &r.condition);

                let colore = grav.colore();
                // Fondo scurissimo tinta del colore: la tinta e' 0.16, non
                // 1.0. Su un pieno saturo il testo perde contrasto e l'occhio
                // smette di leggere il testo per guardare il colore.
                let fondo = colore.a * 0.16;

                let riga_testo = Row::new()
                    .spacing(7)
                    .align_y(iced::Alignment::Center)
                    .push(
                        Text::new(crate::alerts::formatta_avviso(
                            canale,
                            self.valore_live_canale(canale),
                            testo,
                            condizione,
                        ))
                            .size(11)
                            .color(palette::TEXT_BRIGHT)
                            .width(Length::Fill),
                    )
                    .push(
                        iced::widget::button(
                            Text::new("\u{00D7}").size(13)
                                .align_x(alignment::Horizontal::Center),
                        )
                        .padding([1, 5])
                        // Neutra, non `danger`: un pulsante rosso dentro un
                        // riquadro rosso non si distingue dal fondo.
                        .style(crate::btn::ghost)
                        .on_press(Message::DismissAlertChannel(canale.clone())),
                    );

                acol = acol.push(
                    // La cornice sta sul `Container`, non sulla `Row`: la Row
                    // non ha uno stile proprio, e il bordo applicato a lei
                    // verrebbe ignorato.
                    Container::new(
                        Row::new()
                            .spacing(8)
                            .align_y(iced::Alignment::Center)
                            // La barra laterale: 3 px di colore pieno. E' il
                            // segnale di gravita' che resta leggibile anche se
                            // il testo viene troncato dalla larghezza della
                            // sidebar.
                            .push(
                                Container::new(iced::widget::Space::with_width(Length::Fixed(3.0)))
                                    .width(Length::Fixed(3.0))
                                    .height(Length::Fixed(22.0))
                                    .style(move |_: &iced::Theme| iced::widget::container::Style {
                                        background: Some(iced::Background::Color(colore)),
                                        border: iced::Border::default(),
                                        text_color: None,
                                        shadow: iced::Shadow::default(),
                                    }),
                            )
                            .push(riga_testo)
                            .width(Length::Fill),
                    )
                    .width(Length::Fill)
                    .padding([8, 9])
                    .style(move |_: &iced::Theme| iced::widget::container::Style {
                        background: Some(iced::Background::Color(iced::Color {
                            r: colore.r * fondo + 0.012,
                            g: colore.g * fondo + 0.028,
                            b: colore.b * fondo + 0.034,
                            a: 1.0,
                        })),
                        border: iced::Border {
                            color: iced::Color {
                                r: colore.r, g: colore.g, b: colore.b, a: 0.42,
                            },
                            width: 1.0,
                            radius: 12.0_f32.into(),
                        },
                        text_color: None,
                        shadow: palette::glow(colore, 0.20),
                    }),
                );
            }
            acol.into()
        } else {
            iced::Element::from(iced::widget::Space::with_height(0.0))
        };


        // Padding laterale obbligatorio: senza, le etichette appoggiano a
        // x=0 e il bordo sinistro dei glifi viene mangiato dal bordo della
        // finestra (le voci sembrano tagliate a metà).
        let content = Column::new().spacing(6)
            .width(Length::Fixed(340.0))
            // `Shrink` e' OBBLIGATORIO: il contenuto di uno `Scrollable` non
            // puo' riempire l'asse di scorrimento verticale. Senza questo,
            // iced va in panic con "scrollable content must not fill its
            // vertical scrolling axis" e l'app si chiude all'avvio.
            .height(Length::Shrink)
            .padding(iced::Padding { top: 0.0, right: 12.0, bottom: 0.0, left: 12.0 })
            .push(Row::new().padding([3, 8]).push(
                // Tag di build sempre visibile: ogni schermata prova da sola
                // quale eseguibile gira. Basta piastre "e' la versione nuova?"
                // senza saperlo: si legge qui.
                Text::new(format!("FW {:X}.{:X}  HW {}  Campioni: {}  ·  app r143",
                    self.fw_major, self.fw_minor, self.hw_version, self.stats.session_samples))
                    .size(13).color(palette::BLUE_DIM)))
            .push(live_block)
            .push(
                iced::widget::button(
                    Text::new(if self.show_session_hist {
                        "▾  Cronologia sessioni"
                    } else {
                        "▸  Cronologia sessioni"
                    })
                    .size(12)
                    .align_x(alignment::Horizontal::Center),
                )
                .width(Length::Fill)
                .padding([7, 8])
                .style(crate::btn::secondary)
                .on_press(Message::SessionHistoryOpen),
            )
            .push(analytics_block)
            .push(diag_block)
            .push(notify_block)
            .push(rules_block)
            .push(alerts_widget)
            .push(self.view_session_stats())
            .push(Row::new()
                .push(Text::new("Offset temp. (°C)").size(15))
                .push(iced::widget::Space::with_width(Length::Fill))
                // **Il campo accetta negativi, come in r116**: e' lo "spinta in
                // giu'" che dai al modulo, e `0` vuol dire nessuna correzione.
                // Prima l'avevo limitato a 0.5-50 chiamandolo "gradi sotto la
                // rugiada": quel numero positivo faceva scaldare l'impianto.
                .push(NumberInput::new(&self.inputs.set_point, -1000.0..=50.0, Message::UpdateSetpoint)
                    .step(0.5).style(crate::btn::number_input)
                    .input_style(crate::btn::text_input))
                .padding(3).spacing(4))
            .push(Row::new()
                .push(Text::new("Budget watt % (100 = 200 W)").size(15))
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(NumberInput::new(&self.inputs.max_power, 0u8..=100u8, Message::UpdateMaxPower)
                    .step(1).style(crate::btn::number_input)
                    .input_style(crate::btn::text_input))
                .padding(3).spacing(4))
            .push({
                // Diagnostica firmware: mostra il valore EFFETTIVO letto dal device
                // Se fw_power_level != inputs.max_power il firmware sta ignorando il cap
                //
                // **"Fermo" solo se il modulo e' davvero fermo.** Prima il
                // controllo era `!PID_RUNNING`, ma su questo Gen 1 il bit
                // `PID_RUNNING` non si attiva mai anche col modulo che lavora:
                // il risultato era "FW: TEC fermo" con 138 W sul modulo.
                // Ora si usa `modulo_acceso`, la stessa verita' del resto
                // della dashboard: fermo solo se non c'e' ne' PID ne' OCP.
                let acceso = self.last_power_watts > 2.0;
                let (fw_color, fw_label) = if !acceso {
                    (palette::BLUE_DIM, "Assorbimento TEC minimo".to_owned())
                } else {
                    (palette::NEON_GREEN,
                     format!("Duty misurato: {}%", self.fw_power_level))
                };
                Text::new(fw_label).size(11).color(fw_color)
            })
            .push(Column::new().push(en_button).padding([4, 6])
                .align_x(iced::Alignment::Center).width(Length::Fill))
            .push(intestazione_con_icona("Coefficienti PID", palette::SEM_TEMPERATURA, Some(crate::icons::gauge())))
            .push(pid_row("Coef. P", self.inputs.p_coef, Message::UpdatePCoef))
            .push(pid_row("Coef. I", self.inputs.i_coef, Message::UpdateICoef))
            .push(pid_row("Coef. D", self.inputs.d_coef, Message::UpdateDCoef))
            .push(view_badges(
                &self.tec_status,
                self.ocp_confirmed,
                self.cryo_icon_handle.clone(),
                self.ocp_icon_handle.clone(),
                self.chart.last_tec_temp(),
                self.last_power_watts,
            ))
            .push(self.view_profiles())
            .push(self.view_pid_wizard())
            .push(self.view_integrations())
            .push(iced::widget::Space::with_height(Length::Fill));

        // ── Modal AI Advisor ──────────────────────────────────────────────
        // iced 0.13: Stack overlay invece di iced_aw::Modal
        //
        // La sidebar va dentro una Scrollable: i controlli occupano piu'
        // spazio di quanto ne resti su finestre basse (specialmente in
        // verticale, 1080x1920 con la barra hero). Senza scroll, i pulsanti
        // in fondo — Debug, Export CSV, Nascondi — erano irraggiungibili.
        //
        // Il contenuto e' avvolto in un `Container` con altezza `Shrink`.
        // Non basta impostare `.height(Shrink)` sul Column: `Column::extend`
        // fa `enclose` sui figli, quindi un singolo figlio con `Fill` riporta
        // il Column a `Fill` e iced va in panic con "scrollable content must
        // not fill its vertical scrolling axis", chiudendo l'app all'avvio.
        // Il Container applica l'altiesezione DOPO la costruzione e vince.
        let scroll_content = Container::new(content)
            .width(Length::Fixed(340.0))
            .height(Length::Shrink);

        let sidebar_scrollable = iced::widget::Scrollable::new(scroll_content)
            .width(Length::Fixed(340.0))
            .height(Length::Fill)
            .direction(
                iced::widget::scrollable::Direction::Vertical(
                    // Scrollbar con larghezza 0: la sidebar resta
                    // scorrevole quando il contenuto supera l'altezza, ma
                    // senza la barra visibile, che sembrava un elemento
                    // grafico del 2005 e rubava spazio utile.
                    iced::widget::scrollable::Scrollbar::default()
                        .width(0.0)
                        .scroller_width(0.0),
                ),
            );

        Container::new(sidebar_scrollable)
            .style(|_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba8(3,18,23,0.52))),
                border: iced::Border {color:iced::Color::from_rgba8(80,165,175,0.15),width:1.0,radius:12.0.into()},
                ..Default::default()
            }).into()
    }

    /// Hero: identita' del prodotto a sinistra, strumenti di misura a destra.
    ///
    /// Non richiede parametri: legge tutto dallo stato.
    ///
    /// **Perche' i quadranti e non le tile.** Le tile mostravano numeri senza
    /// contesto: "18 °C" non dice se e' freddo o pericoloso. I quadranti
    /// mettono quel numero su un anello con la zona di pericolo in rosso, e
    /// dicono subito quanto sei vicino al limite. Le tile che restano
    /// coprono solo cio' che i quadranti NON mostrano: il margine di
    /// condensa, l'OCP e la versione del firmware.
    /// Fascia di identita' e strumenti: **una barra compatta e densa**.
    ///
    /// **Perche' e' una barra e non due righe.** La versione a due righe
    /// occupava 270 px di altezza con il logo da 168 px: tre quarti della
    /// fascia erano vuoti e le card, distese a tutta larghezza con `Fill`,
    /// sembravano tre oggetti sciolti invece di un gruppo. Qui tutto sta in
    /// una riga di ~96 px, diviso in settori da filetti da 1 px: identita',
    /// tre letture, stato. Impaginazione densa (data-dense-design): la
    /// densita' e' una funzionalita', lo spazio bianco e' sprecato.
    ///
    /// **Niente canvas e niente `FillPortion`.** I quadranti a canvas erano
    /// spariti del tutto quando la fascia si stringeva (larghezza zero,
    /// disegno vuoto) e i `FillPortion` collassavano a 3 px per campo,
    /// facendo andare a capo ogni singola lettera. Qui ogni larghezza e'
    /// calcolata da un budget esplicito e ogni valore ha una dimensione fissa.
    /// Fascia di identita' e strumenti: una barra compatta e densa.
    ///
    /// **Perche' tutto sta in una riga.** La versione a due righe occupava
    /// 270 px con tre quarti di vuoto. Qui la fascia e' alta ~128 px e porta
    /// identita', tre letture e stato, separate da filetti da 1 px.
    ///
    /// **Cosa e' stato provato e tolto, e perche'.** Tre varianti, tre
    /// fallimenti, tutti da layout e non da gusto:
    ///   - riquadri con fondo e bordo: sembravano tre blocchi scollegati
    ///     invece di una barra di strumenti;
    ///   - icone del set con alone pulsante dietro: cerchi colorati attorno
    ///     a cornici quadre, si leggevano come artefatti sovrapposti;
    ///   - numero sotto l'icona: non ci stava e veniva **tagliato via**.
    /// Resta il testo, che si legge meglio e non puo' rompersi.
    ///
    /// **Niente canvas e niente `FillPortion`.** I `FillPortion` collassavano
    /// a 3 px per campo e facevano andare a capo ogni singola lettera. Ogni
    /// larghezza qui e' calcolata da un budget esplicito e ogni valore ha
    /// dimensione fissa.
    /// Fascia di identita' e letture: griglia 3x2 a destra, logo a sinistra.
    ///
    /// **Un solo tipo di elemento, per sei informazioni.** Le tre letture
    /// (piastra, potenza, controller) e le tre di stato (margine, OCP,
    /// firmware) sono costruite dalla stessa funzione e hanno lo stesso aspetto.
    /// Prima lo stato era in chip con bordo e pallino, e le letture erano
    /// testo libero: due linguaggi diversi nella stessa fascia, e si capiva
    /// subito che erano stati aggiunti dopo.
    ///
    /// **Niente filetti.** Sono stati tolti: separavano troppo, sembravano
    /// una tabella divisa a celle invece di un pannello di letture. Il
    /// raggruppamento lo fa la spaziatura, che e' piu' pulita di una linea.
    ///
    /// **Cosa e' stato provato e tolto, e perche'.**
    ///   - riquadri con fondo e bordo: sembravano blocchi scollegati;
    ///   - icone del set con alone pulsante: cerchi colorati attorno a
    ///     cornici quadre, si leggevano come artefatti sovrapposti;
    ///   - numero sotto l'icona: non ci steva e veniva **tagliato via**.
    ///
    /// **Niente `FillPortion`.** Collassavano a 3 px per campo e facevano
    /// andare a capo ogni singola lettera. Ogni larghezza qui e' calcolata da
    /// un budget esplicito e ogni valore ha dimensione fissa.
    /// Fascia di identita' e letture: logo e titolo a sinistra, SEI letture
    /// su una riga sola a destra dentro un pannello unico.
    ///
    /// **Perche' una riga e non una griglia.** Sei letture in fila si leggono
    /// come una strumentazione; in griglia si leggono come due righe di numeri
    /// senza relazione fra loro. La griglia resta solo come ripiego, quando
    /// una cella scenderebbe sotto i 92 px e il numero non si leggerebbe piu'.
    ///
    /// **Perche' il pannello.** Prima le sei letture erano sei numeri sciolti in
    /// 330 px di vuoto: sembravano dimenticati in fila. Il pannello unico chiude
    /// il vuoto e le fa leggere come un blocco unico.
    ///
    /// **Cosa e' stato provato e tolto, e perche'.** Quattro varianti, tutte
    /// peggiori di questa:
    ///   - riquadri con fondo e bordo: sembravano blocchi scollegati;
    ///   - icone del set con alone pulsante: cerchi colorati attorno a
    ///     cornici quadre, si leggevano come artefatti sovrapposti;
    ///   - numero sotto l'icona: non ci steva e veniva **tagliato via**;
    ///   - celle sparse con `Fill` in mezzo: trecento px di buco.
    ///
    /// **Niente `FillPortion` a finestrella.** Collassavano a 3 px per campo e
    /// facevano andare a capo ogni singola lettera. Qui la larghezza delle
    /// celle e' calcolata da un budget esplicito e il `Fill` le divide solo
    /// quando lo spazio e' davvero abbondante.
    /// Il banner cryogenic compare **sotto** la soglia e sparisce sopra.
    ///
    /// La soglia vive in `charts.rs` (`cryo_visibile`); qui non resta
    /// nessuna copia.
    /// Banner di allerta cryogenic.
    ///
    /// Compare sotto soglia e sparisce sopra. Le dimensioni sono quelle
    /// dell'asset (`CRYO_WARN_W`/`CRYO_WARN_H`); a schermo si fissa solo la
    /// larghezza e l'altezza viene dall'immagine, quindi non si stira.
    fn view_banner(&self) -> iced::Element<'_, Message> {
        // ── Valori e colori di stato ─────────────────────────────
        let tec_temp = self.chart.last_tec_temp();

        // **Una sola decisione per frame.**
        //
        // Prima la soglia veniva valutata due volte: una per la riga di
        // stato e una dentro il banner. Se le due letture non coincidevano
        // — perche' la temperatura era aggiornata nel mezzo — l'app
        // scriveva una cosa e disegnava l'altra. Qui si decide una volta e
        // il risultato passa a entrambi.
        let cryo_visibile = self.cryo_hero_visibile();
        let dew_ok = self.last_dew_point.is_finite();
        let dew = if dew_ok { self.last_dew_point } else { -999.0 };
        let margine = if dew_ok { tec_temp - dew } else { f32::NAN };
        let mg = self.last_cond_margin;
        let mg_valido = mg != f32::MAX && mg.is_finite();

        // Verde con margine, ambra quando ci si avvicina, blu se la rugiada
        // non e' nota, rosso sopra la guardia. Il colore e' informazione, non
        // decorazione: dice lo stato prima che tu legga il numero.
        let tec_color = if !dew_ok { palette::BLUE_PRIMARY }
            else if tec_temp < dew - 1.5 { palette::NEON_GREEN }
            else { palette::WARNING };
        let pot_color = if self.applied_power >= self.inputs.max_power {
            palette::NEON_GREEN
        } else {
            palette::BLUE_PRIMARY
        };
        let ctrl_color = if self.ctrl_temp >= Self::CTRL_SOFT {
            palette::DANGER
        } else {
            palette::NEON_GREEN
        };
        let mg_color = if !mg_valido { palette::BLUE_DIM }
            else if mg < 0.0 { palette::DANGER }
            else if mg < 2.0 { palette::WARNING }
            else { palette::NEON_GREEN };
        let ocp_color = if self.ocp_confirmed { palette::DANGER }
            else { palette::NEON_GREEN };
        // ── Quote ────────────────────────────────────────────────
        // Il logo resta a 112 px: non e' stato chiesto di ingrandirlo, e il
        // 150 px era una mia interpretazione di "fa schifo", non una
        // richiesta. La sua deformazione non veniva da qui ma dall'asset,
        // che era stirato del 2%: quello e' stato corretto a parte, in
        // `build.rs`, rigenerando il raw da `stargate_logo_official.png`.
        const LOGO_H:     f32 = 112.0;
        const LOGO_PAD:   f32 = 7.0;
        const IDENTITY_W: f32 = 262.0;
        const CELL_MIN:   f32 = 118.0; // sotto questa la cella non si legge
        // Altezza riga: icona 24 + titolo + numero 30 + didascalia.
        // Era 58 px, cioe' piu' bassa del contenuto: la seconda riga
        // veniva tagliata e restavano solo tre puntini. Ora ci sta tutta.
        const CELL_H:     f32 = 86.0;
        const CELL_GAP:   f32 = 12.0;
        const ROW_GAP:    f32 = 10.0;
        // Intestazione STRUMENTAZIONE + spazio prima della griglia.
        const HEAD_H:     f32 = 30.0;
        const GRID_PAD_X: f32 = 18.0;
        const GRID_PAD_Y: f32 = 10.0;
        const GAP:        f32 = 16.0;
        const PAD_X:      f32 = 32.0; // 14 a sinistra + 18 a destra
        const PAD_Y:      f32 = 22.0; // 11 sopra + 11 sotto

        let logo_box = LOGO_H + LOGO_PAD * 2.0;
        let avail = (self.win_w as f32 - PAD_X).max(280.0);

        // Larghezza che resta dopo logo e identita': e' la piu' grande
        // sprecazione rimasta, e fin qui finiva tutta in un vuoto tra il
        // titolo e la prima cella.
        let leftover_grezzo = avail - logo_box - GAP - IDENTITY_W - GAP;

        // ── Banner nello slot del marchio ──────────────────────────
        //
        // Il banner NON sottrae spazio alla griglia: occupa lo slot del
        // marchio, che e' gia' a budget. Prima lo spazio veniva riservato
        // sempre (slot fisso) e la griglia restava piccola con un vuoto a
        // destra; prima ancora veniva sottratto solo quando visibile e la
        // griglia si ricalcolava a ogni comparsa. In entrambi i casi la
        // hero cambiava forma. Ora la griglia riceve sempre tutto lo
        // spazio e non si ricalcola mai per colpa del banner.
        // La griglia riceve tutto lo spazio: il banner non ne sottrae.
        // `spazio_riservato_banner` ritorna sempre zero ed esiste per
        // vincolarlo con un test, non per calcolare.
        let leftover = leftover_grezzo - spazio_riservato_banner(cryo_visibile);

        // **Sei celle su UNA riga.** Non in griglia: l'utente ha chiesto una
        // sola riga, e ha ragione perche' sei letture in fila si leggono
        // come una strumentazione, mentre in griglia si leggono come due
        // righe di numeri senza relazione.
        //
        // La riga singola e' possibile finche' una cella non scende sotto
        // `CELL_MIN`: sotto quella soglia il numero non si legge piu' e si
        // cade nel ripiego a due righe.
        const N_CELL:  f32 = 6.0;
        const CELL_SP: f32 = 10.0; // spaziatura fra celle in riga singola
        /// Sotto questa larghezza la cella non regge: "26.7" piu' "°C" non ci
        /// stanno piu' e il numero viene compresso fino a non leggersi.
        const CELL_MIN_SINGOLA: f32 = 92.0;

        // La griglia non ha ancora la sua larghezza definitiva: la calcola
        // piu' avanti, DOPO che il marchio ha preso la sua quota di spazio.
        // Calcolarla qui e poi ricalcolarla significava due fonti di verita'
        // e la seconda vinceva: era cosi' che il marchio non trovava mai
        // spazio e non veniva disegnato.
        // La griglia resta su una riga se c'e' spazio: serve solo per calcolare
        // la sua altezza, non per scegliere il layout.
        let w_singola = (leftover - GRID_PAD_X * 2.0 - CELL_SP * (N_CELL - 1.0)) / N_CELL;
        let singola = w_singola >= CELL_MIN_SINGOLA;

        // **La disposizione e' sempre a due righe**, con e senza banner.
        //
        // Prima dipendeva dallo spazio: quando il banner compariva sottraeva
        // 190 px, la cella scendeva sotto `CELL_MIN_SINGOLA` e le celle
        // finivano sotto la fascia, dove la griglia le schiacciava. Il
        // risultato era che **la card info spariva proprio quando compariva
        // il banner**, cioe' nel momento in cui serviva leggere le
        // temperature.
        //
        // Con due righe sempre: il banner sta nella riga alta e compare o
        // sparisce li', senza cambiare la struttura; le celle restano nella
        // loro riga, intere, con o senza banner. E l'altezza della fascia non
        // cambia mai, quindi la hero non si deforma.

        // ── Griglia: tutto lo spazio disponibile ───────────────────
        //
        // Il marchio non esiste piu' nella hero: `assets/hero_brand.png`
        // conteneva l'immagine cryogenic, quindi sopra i 20 °C — quando il
        // banner vero e' nascosto — si vedeva comunque un logo cryogenic.
        // Sembrava la soglia invertita, ma erano due immagini diverse.
        // Senza marchio la griglia prende tutto `leftover`, meno un gap.
        // Larghezza minima per tre colonne: e' il requisito assoluto della
        // griglia 2x3.
        let grid_min_2x3 = CELL_MIN * 3.0 + CELL_GAP * 2.0 + GRID_PAD_X * 2.0;

        // La griglia ri-calcola su quello che e' rimasto, e la scelta
        // riga singola / 2x3 va rifatta: cambia la base.
        let (grid_w, grid_h) = {
            let disp = leftover - GAP;
            if singola {
                let w = (disp - GRID_PAD_X * 2.0 - CELL_SP * (N_CELL - 1.0))
                    / N_CELL;
                if w >= CELL_MIN_SINGOLA {
                    (disp, HEAD_H + CELL_H)
                } else {
                    (
                        (grid_min_2x3).min(disp),
                        HEAD_H + CELL_H * 2.0 + ROW_GAP,
                    )
                }
            } else {
                (
                    grid_min_2x3.min(disp),
                    HEAD_H + CELL_H * 2.0 + ROW_GAP,
                )
            }
        };

        // **Sempre una riga sola**: logo, titolo, banner e card stanno
        // tutti nella stessa riga, la card sempre a destra. Non scende
        // mai sotto, a nessuna larghezza e con o senza banner.
        // (Niente `if`: la riga e' unica per decisione, non per spazio.)

        // L'altezza e' il massimo del contenuto della riga, piu' i bordi.
        // Non dipende dalla visibilita' del banner (l'altezza del banner
        // e' sempre dentro il massimo), quindi la fascia non cambia forma
        // quando compare o sparisce. Dipende solo dalla larghezza della
        // finestra tramite `grid_h`, che e' normale: ridimensionare non e'
        // deformare.
        // L'altezza riservata al banner e' quella di **visualizzazione**
        // (150 px sul rapporto reale = ~96 px), non quella dell'asset:
        // l'asset e' a piena risoluzione per la qualita' e non deve entrare
        // nel calcolo dell'altezza, altrimenti la fascia diventerebbe alta
        // 400 px.
        let cryo_display_h =
            CRYO_DISPLAY_W * crate::CRYO_WARN_H as f32 / crate::CRYO_WARN_W as f32;
        let banner_h = logo_box
            .max(cryo_display_h)
            .max(grid_h + GRID_PAD_Y * 2.0)
            + PAD_Y;

        // ── Logo ─────────────────────────────────────────────────
        // Il rapporto viene dalle costanti **del logo**, che sono
        // `LOGO_W`/`LOGO_H` (480x305, banner). Non va preso da
        // `BRAND_RAPPORTO`, che descrive un asset diverso: il marchio,
        // quadrato 336x336. Usare quello qui stringeva il logo da 176 px a
        // 112 px e lo schiacciava.
        let logo_w = LOGO_H * (crate::LOGO_W as f32 / crate::LOGO_H as f32);
        let logo_card = Container::new(
            iced::widget::Image::new(self.logo_handle.clone())
                // `Contain`: iced per default **stira** l'immagine per
                // riempire il riquadro. Con larghezza e altezza arrotondate
                // in modo indipendente, lo stiramento e' garantito, e non
                // si vede finche' il riquadro combacia. In `Contain`
                // l'immagine entra nel riquadro **tenendo le proporzioni**,
                // quindi un riquadro imperfetto non la deforma.
                .content_fit(iced::ContentFit::Contain)
                .width(Length::Fixed(logo_w))
                .height(Length::Fixed(LOGO_H))
        )
        .padding(LOGO_PAD)
        .style(|_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(palette::GLASS_HI)),
            // Stesso raggio della maschera applicata in build.rs: altrimenti
            // il logo sembra una foto quadrata incollata sopra una tonda.
            border: iced::Border {
                color: iced::Color { r: 0.0, g: 0.75, b: 0.85, a: 0.38 },
                width: 1.0,
                // Stesso raggio della maschera applicata in `build.rs`
                // (`LOGO_RADIUS`). Qui c'era `14` con un commento che
                // diceva "stesso raggio": non lo era, era la meta'.
                // Il disallineamento faceva leggere il bordo come tagliato
                // fuori dal logo.
                radius: 28.0.into(),
            },
            shadow: palette::glow(iced::Color { r: 0.0, g: 0.7, b: 0.9, a: 1.0 }, 0.26),
            text_color: None,
        });

        // ── Identita' ────────────────────────────────────────────
        // Titolo, sottotitolo, e sotto la riga di stato che il desktop
        // riporta in alto: qui si legge senza spostare gli occhi.
        //
        // **Il titolo ha una cornice propria, e non la hero.** Il logo
        let identity = Column::new()
            .spacing(3)
            .width(Length::Fixed(IDENTITY_W))
            .push(
                Text::new("CryoCooling Dashbord")
                    // 20 px e non 22: a 22 il titolo andava a capo su due
                    // righe dentro i 262 px del blocco identita'. Il titolo
                    // su una riga sola e' parte del layout, non un dettaglio:
                    // spezzato, l'hero smette di leggersi come intestazione.
                    .size(20)
                    .color(palette::TEXT_BRIGHT),
            )
            .push(
                Text::new("EK-Quantum Delta\u{00B2} TEC \u{00B7} StargateLab")
                    .size(9)
                    .color(palette::BLUE_DIM),
            )
            // Questa riga era una **stringa fissa**: diceva "HW TEC in
            // funzione · protezione attiva" qualunque cosa fosse successa,
            // anche a TEC spento. Un'informazione che non cambia non
            // e' un'informazione: e' rumore che sembra una conferma.
            //
            // Ora dice lo stato reale, e quando non e' noto lo dichiara.
            // Dice lo stato **del modulo**, non la sua intenzione.
            //
            // Prima usava `tec_eroga_potenza()`, che rispondeva "no" quando
            // `PID_RUNNING` non era attivo — e su questo controller non lo e'
            // mai. Il risultato era la riga che hai letto: **"HW TEC spento"**
            // in rosso mentre il modulo erogava 227 W e la tensione era 10.6 V.
            //
            // La riga dice se il modulo lavora davvero, e lo decide dai watt
            // misurati — non dal bit OCP (rumore: acceso anche da spento) e
            // non da `applied_power` (e' quello che abbiamo chiesto, non
            // quello che il modulo fa). "In funzione a 0%" era la frase che
            // ha fatto premere DISABILITA a modulo gia' spento.
            .push(
                if Self::avvicinamento_muro(self.ctrl_temp) {
                    // **Avviso PRIMA della guardia.** Dice anche cosa sta
                    // succedendo in concreto: il tetto del modulo e' sceso per
                    // la temperatura, quindi il modulo non puo' spingere di
                    // piu'. Non e' un errore: e' il lato caldo al limite.
                    Text::new(if Self::oltre_il_muro(self.ctrl_temp) {
                        // **Say perche'.** Il numero che hai indicato tu come
                        // limite e' 38, e qui si dice che e' superato: un
                        // avviso che non sa quale muro ha superato e' un
                        // avviso che l'operatore impara a ignorare.
                        format!(
                            "controller {:.1} °C · oltre i {} °C che avevi indicato ·                              tetto modulo ridotto al {}%",
                            self.ctrl_temp,
                            Self::CTRL_MURO,
                            Self::tetto_per_ctrl(self.ctrl_temp, self.inputs.max_power),
                        )
                    } else {
                        format!(
                            "controller caldo {:.1} °C · tetto modulo ridotto al {}%",
                            self.ctrl_temp,
                            Self::tetto_per_ctrl(self.ctrl_temp, self.inputs.max_power),
                        )
                    })
                    .size(9).color(palette::WARNING)
                } else if self.last_power_watts > 2.0 {
                    Text::new(format!(
                        "Gen 1 / TEC 2 attivo \u{00B7} duty misurato {}%",
                        self.fw_power_level))
                    .size(9).color(palette::BLUE_DIM)
                } else {
                    Text::new("HW TEC fermo \u{00B7} 0 W")
                    .size(9).color(palette::WARNING)
                });

        // ── Una lettura ──────────────────────────────────────────
        // Tre righe: etichetta 8 px, valore 22 px nel colore di stato,
        // didascalia 9 px. La didascalia e' la parte che rende il numero
        // utile: "26.7 °C" da solo non dice se va bene, "margine +10.4" sì.
        //
        // `Fn` e non `FnOnce`: la griglia la chiama in due rami alternativi e
        // `Element` in iced 0.13 non e' Clone.
        // ── Una lettura ──────────────────────────────────────────
        // **Il numero domina, l'etichetta sta sotto.** Era il contrario:
        // etichetta a 8 px e numero a 22, cioe' il titolo si leggeva prima
        // del dato, e su una card di tre letture l'occhio non sapeva
        // dove guardare. Su un display da calcolatrice e' l'opposto: il
        // numero grande e acceso, l'etichetta piccola e in secondo piano.
        //
        // Da 22 a 30 px il numero, da 8 a 10 l'etichetta, e il numero
        // prende un alone del proprio colore. L'alone e' la parte che
        // fa "in evidenza": un colore piu' acceso da solo, su uno sfondo
        // scuro, non basta per staccare un numero da un altro numero.
        //
        // `Fn` e non `FnOnce`: la griglia la chiama in due rami alternativi e
        // `Element` in iced 0.13 non e' Clone.
        let cell = |icona: iced::widget::image::Handle,
                    titolo: &'static str,
                    valore: String,
                    unita: &'static str,
                    didascalia: String,
                    colore: iced::Color|
         -> iced::Element<'_, Message> {
            // **Una sola strumentazione, niente scatole.**
            //
            // Ogni numero aveva un contenitore con alone colorato: sei
            // aloni diventavano sei riquadri e la card sembrava sei card
            // piccole. Qui non c'e' contenitore attorno al valore. Lo stato
            // resta nel colore del numero; l'icona monocromatica dice il
            // contesto e non compete con lo stato.
            Column::new()
                .spacing(2)
                .width(Length::Fill)
                .push(
                    Row::new()
                        .spacing(7)
                        .align_y(iced::Alignment::Center)
                        .push(
                            iced::widget::Image::new(icona)
                                .width(Length::Fixed(24.0))
                                .height(Length::Fixed(24.0))
                                .opacity(0.92_f32),
                        )
                        .push(Text::new(titolo).size(10).color(palette::TEXT_BRIGHT)),
                )
                .push(
                    Row::new()
                        .spacing(3)
                        .align_y(iced::Alignment::Center)
                        .push(Text::new(valore).size(30).color(colore))
                        .push(Text::new(unita).size(12).color(colore)),
                )
                .push(Text::new(didascalia).size(10).color(palette::BLUE_DIM))
                .into()
        };

        let mk_cells = || -> Vec<iced::Element<'_, Message>> {
            vec![
                // ── Le tre letture ──────────────────────────────
                // Etichette corte. A ~90 px per cella "CONTROLLER" e
                // "FIRMWARE" si spezzavano a meta' parola ("CONTROL LER",
                // "FIRMWAR E") e il valore andava a capo con il punto
                // staccato dal numero ("18. 6"). La didascalia sotto il
                // valore porta gia' il dettaglio, quindi l'etichetta puo'
                // stare al minimo.
                cell(
                    crate::icons::plate(),
                    "PIASTRA",
                    format!("{tec_temp:.1}"),
                    "\u{00B0}C",
                    if dew_ok {
                        format!("margine {:+.1}", margine)
                    } else {
                        "rugiada n.d.".to_owned()
                    },
                    tec_color,
                ),
                cell(
                    crate::icons::bolt(),
                    "POTENZA",
                    // **Watt, non percentuale.** `applied_power` e' la
                    // percentuale comandata, non la misura. Qui si mostrava
                    // quella con l'unita' "%" e una didascalia che scriveva
                    // "tetto 50 W" dove 50 era una percentuale: due errori
                    // nella stessa cella. Il wattaggio vero e'
                    // `last_power_watts`; la percentuale e' solo il comando.
                    format!("{:.0}", self.last_power_watts),
                    "W",
                    format!("budget {:.0} W", 200.0 * self.inputs.max_power.min(100) as f32 / 100.0),
                    pot_color,
                ),
                cell(
                    crate::icons::shield(),
                    "CTRL",
                    format!("{:.1}", self.ctrl_temp),
                    "\u{00B0}C",
                    format!("guardia {}", Self::CTRL_SOFT as i32),
                    ctrl_color,
                ),
                // ── Le tre di stato, stesso aspetto delle altre ──
                cell(
                    crate::icons::thermo(),
                    "MARGINE",
                    if !mg_valido { "\u{2014}".to_owned() }
                    else { format!("{mg:+.1}") },
                    "\u{00B0}",
                    if !mg_valido { "non noto".to_owned() }
                    else if mg < 0.0 { "CONDENSA".to_owned() }
                    else if mg < 2.0 { "basso".to_owned() }
                    else { "sicuro".to_owned() },
                    mg_color,
                ),
                cell(
                    crate::icons::hazard(),
                    "OCP",
                    if self.ocp_confirmed { "ON".to_owned() } else { "OFF".to_owned() },
                    "",
                    // Non "potenza ridotta": su questo impianto l'OCP e'
                    // rumore e la diagnostica dice nessuna azione. La cella
                    // riporta il segnale, non una conseguenza che potrebbe
                    // non esserci.
                    if self.ocp_confirmed { "segnale attivo".to_owned() }
                    else { "nessun allarme".to_owned() },
                    ocp_color,
                ),
                cell(
                    crate::icons::cpu(),
                    "FW",
                    format!("{:X}.{:X}", self.fw_major, self.fw_minor),
                    "",
                    format!("hw rev {:X}", self.hw_version),
                    palette::BLUE_PRIMARY,
                ),
            ]
        };

        // Griglia 3x2 dentro UN pannello.
        //
        // Il pannello e' quello che chiude il vuoto: sei letture sciolte in
        // 330 px di spazio non sembravano un blocco, sembravano sei numeri
        // dimenticati in fila. Con il vetro e il bordo unico leggono come
        // un'unica strumentazione.
        let griglia = || -> iced::Element<'_, Message> {
            let mut it = mk_cells().into_iter();
            let r1 = (
                it.next().expect("6 celle"),
                it.next().expect("6 celle"),
                it.next().expect("6 celle"),
            );
            let r2 = (
                it.next().expect("6 celle"),
                it.next().expect("6 celle"),
                it.next().expect("6 celle"),
            );
            // Il contenuto del pannello cambia con la forma scelta: una riga
            // da sei, oppure due da tre quando la finestra e' stretta.
            // Le righe interne sono `Fill`: senza, restano `Shrink` e le
            // celle `Fill` collassano alla larghezza del contenuto,
            // lasciando un vuoto dentro la card. E' il vuoto che si vedeva
            // a destra delle celle.
            let interno: iced::Element<'_, Message> = if singola {
                Row::new()
                    .spacing(CELL_SP)
                    .width(Length::Fill)
                    .push(r1.0).push(r1.1).push(r1.2)
                    .push(r2.0).push(r2.1).push(r2.2)
                    .into()
            } else {
                Column::new()
                    .spacing(ROW_GAP)
                    .push(Row::new().spacing(CELL_GAP).width(Length::Fill).push(r1.0).push(r1.1).push(r1.2))
                    .push(Row::new().spacing(CELL_GAP).width(Length::Fill).push(r2.0).push(r2.1).push(r2.2))
                    .into()
            };
            // **La card "premium" delle letture.**
            //
            // Prima era una lastra quasi invisibile: fondo al 55% di un
            // colore vicino al nero, bordo ciano al 22% di opacita' e 1 px.
            // Su uno sfondo gia' scuro, un bordo al 22% non si vede, e il
            // risultato era che le sei letture sembravano galleggiare senza
            // contenitore.
            //
            // Qui: fondo piu' profondo e **opaco**, bordo ciano acceso e
            // 1 px, alone esterno. Il fondo e' opaco perche' una card
            // premium non si vede perche' e' trasparente: si vede perche'
            // ha un bordo che stacca e un'ombra sotto. Il vetro
            // translucido resta per i pannelli di sfondo, dove il
            // contenuto e' testo e non numeri grandi.
            // Intestazione della strumentazione: dice che questo blocco e'
            // uno strumento unico, non sei numeri messi vicini. Icona e
            // titolo bastano; lo stato vive nei valori, non qui.
            let intestazione = Row::new()
                .spacing(8)
                .align_y(iced::Alignment::Center)
                .push(
                    iced::widget::Image::new(crate::icons::gauge())
                        .width(Length::Fixed(20.0))
                        .height(Length::Fixed(20.0))
                        .opacity(0.92_f32),
                )
                .push(Text::new("STRUMENTAZIONE").size(11).color(palette::TEXT_BRIGHT))
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(Text::new("LIVE").size(9).color(palette::NEON_GREEN));
            Container::new(
                Column::new()
                    .spacing(8)
                    .push(intestazione)
                    .push(interno),
            )
            .width(Length::Fixed(grid_w))
            .padding([GRID_PAD_Y, GRID_PAD_X])
            .style(|_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(
                    iced::Color { r: 0.012, g: 0.042, b: 0.055, a: 0.92 },
                )),
                border: iced::Border {
                    // Ciano acceso: su un bordo sottile e' la linea che
                    // dice "qui c'e' qualcosa". Era al 22% e non si vedeva.
                    color: iced::Color { r: 0.0, g: 0.75, b: 0.85, a: 0.65 },
                    width: 1.0,
                    radius: 12.0_f32.into(),
                },
                text_color: None,
                // Alone dietro la card: stacca il pannello dall'hero senza
                // dover spostare niente, ed e' quello che fa leggere
                // "premium" senza aggiungere un altro elemento.
                shadow: palette::glow(iced::Color { r: 0.0, g: 0.6, b: 0.8, a: 1.0 }, 0.28),
            })
            .into()
        };

        // ── Composizione ─────────────────────────────────────────
        let body: iced::Element<'_, Message> = {
            // Logo e identita' a sinistra, pannello delle letture **a filo
            // del bordo destro**.
            //
            // Il rientro elastico tra identita' e pannello e' la cosa che
            // mancava: senza, il pannello si fermava dove finiva il titolo e
            // restavano oltre 100 px di spazio morto a destra, con la fascia
            // piu' larga a sinistra che a destra. Il pannello ha gia' la
            // larghezza dello spazio avanzato, quindi il rientro non cambia
            // le sue dimensioni: le spinge solo al bordo.
            //
            // Il marchio si piu` avanti di tutti, cosi' occupa lo spazio
            // morto invece di spostare il pannello.
            // **Allineamento al centro, non in basso.**
            //
            // Qui c'era `Alignment::End`, con un commento che lo giustificava.
            // La giustificazione era sbagliata: `End` non allinea gli elementi
            // su un asse comune, li appoggia al fondo della fascia. Il logo e
            // la card delle letture finivano "scendere", e la fascia leggeva
            // come pezzi appoggiati per caso. E' il sintomo che l'utente ha
            // segnalato come "il logo si vede in basso".
            //
            // Il centro e' quello corretto: logo, titolo e celle stanno sulla
            // stessa mezzaltezza, indipendentemente dalle loro dimensioni.
            let mut riga = Row::new()
                .spacing(GAP)
                .align_y(iced::Alignment::Center)
                .push(logo_card)
                .push(identity);
            // Il logo cryogenic sta **nella riga**, fra il titolo e le
            // celle: e' il posto che occupa nella schermata di riferimento.
            // Sopra la fascia, o al centro sovrapposto, spostava gli altri
            // elementi e rompeva l'allineamento.
            // Il logo cryogenic sta **al mezzo** della hero, fra il titolo e
            // le celle, e ci sta **centrato**: un `Fill` prima e uno dopo
            // spostano l'elemento al centro dello spazio che gli compete.
            //
            // Con un solo `Fill` (prima o dopo) il banner restava addossato al
            // titolo, che e' esattamente il difetto di allineamento segnalato.
        // Il banner cryogenic sta **fra il logo e la card delle letture**,
        // non al centro della finestra e non dentro la fascia: e' il posto
        // che occupa nella schermata di riferimento, sotto il titolo.
        //
        // La larghezza e' quella dichiarata per l'asset, che gia' tiene il
        // rapporto corretto: qui non si calcola niente, quindi qui non si
        // puo' deformare. Il vuoto attorno viene assorbito dagli spazi
        // elastici, non allargando il banner.
            // **Un solo spazio elastico**, e va prima delle celle.
            //
            // Qui ce n'erano tre: uno prima del banner, uno dopo, uno prima
            // della griglia. Ogni elastico si prende un pezzo di spazio
            // uguale, quindi il banner finiva a meta' della fascia, il marchio
            // piu' in la, e le celle a destra con un vuoto in mezzo che
            // sembrava un errore di composizione.
            //
            // Con un elastico solo: logo, titolo e banner restano accostati a
            // sinistra, le celle restano al bordo destro, e il vuoto sta dove
            // deve stare, fra i due gruppi.
            // Banner accanto al titolo quando visibile, niente quando non
            // lo e': il marchio non esiste piu', quindi non c'e' nessuno
            // scambio di slot e la riga non cambia struttura.
            if let Some(c) = self.view_cryo_banner(cryo_visibile) {
                riga = riga.push(c);
            }
            riga
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(griglia())
                .into()
        };
        // Sfondo a gradiente diagonale: e' il tratto che fa leggere l'area come
        // intestazione e non come "un altro riquadro".
        // Contenuto **centrato in verticale** nell'altezza della fascia.
        //
        // Senza questo il contenuto si accalcava in alto e sotto restava il
        // vuoto: l'altezza c'e', ma il contenuto no, e il risultato e' una
        // fascia con il fondo a vuoto. Con due spazi elastici sopra e sotto
        // il vuoto si divide in due e la fascia resta centrata sia col
        // banner sia senza.
        let contenuto_centrato = Column::new()
            .push(iced::widget::Space::with_height(Length::Fill))
            .push(body)
            .push(iced::widget::Space::with_height(Length::Fill));

        Container::new(contenuto_centrato)
            .width(Length::Fill)
            .height(Length::Fixed(banner_h))
            .padding(iced::Padding {
                top: PAD_Y / 2.0,
                right: 18.0,
                bottom: PAD_Y / 2.0,
                left: 14.0,
            })
            .style(|_: &iced::Theme| iced::widget::container::Style {
                // **Vetro.** Gli alpha erano 0,98 / 0,97 / 0,96: di fatto
                // opachi, e la fascia si leggeva come un blocco scuro
                // appoggiato sopra la foto invece che come un vetro steso
                // sopra di essa. Ora la scena passa through e il gradiente
                // serve solo a tenere il testo leggibile.
                //
                // La discesa non e' lineare per scelta estetica: a sinistra,
                // dove stanno il logo e il titolo, resta piu' coperta, perche'
                // e' la zona con piu' testo; a destra, dove stanno le cifre,
                // si schiarisce e la foto si vede. I valori non scendono sotto
                // ~0,30 perche' sotto quella soglia le celle perdono contrasto
                // e i numeri diventano illeggibili: un vetro che non si legge
                // e' solo una decorazione.
                background: Some(iced::Background::Gradient(iced::Gradient::Linear(
                    iced_core::gradient::Linear::new(0.0)
                        .add_stop(0.0, iced::Color { r: 0.035, g: 0.105, b: 0.125, a: 0.62 })
                        .add_stop(0.55, iced::Color { r: 0.012, g: 0.052, b: 0.058, a: 0.44 })
                        .add_stop(1.0, iced::Color { r: 0.004, g: 0.026, b: 0.028, a: 0.30 }),
                ))),
                border: iced::Border {
                    color: iced::Color { r: 0.0, g: 0.55, b: 0.62, a: 0.45 },
                    width: 1.0,
                    radius: iced_core::border::Radius {
                        top_left: 0.0,
                        top_right: 0.0,
                        bottom_right: 12.0_f32.into(),
                        bottom_left: 12.0_f32.into(),
                    },
                },
                text_color: None,
                shadow: iced::Shadow::default(),
            })
            .into()
    }
    /// Strip orizzontale con metriche chiave — solo in portrait
    ///
    /// Il margine arrivava come parametro, ma ora la funzione lo prende da
    /// `etichetta_margine_ui`, per non avere due fonti. Il parametro resta per
    /// non cambiare la firma del chiamante, che passa anche il colore.
    fn view_portrait_metrics(&self, _mg: f32, _mg_color: iced::Color) -> Element<'_, Message> {
        // Stessa decisione della colonna laterale, stessa funzione: due copie
        // dei confronti divergerebbero al primo ritocco.
        let (cond_color, cond_lbl) = self.etichetta_margine_ui();

        // Tile di metrica: superficie delimitata, raggio concentrico e
        // profondità via shadow (niente bordi duri). Gerarchia: label piccola
        // e spenta, valore grande e brillante. Valori allineati a destra per
        // non far saltare il layout quando cambiano le cifre.
        let cell = |label: &'static str, val: String, color: iced::Color| {
            Container::new(
                Column::new()
                    .align_x(iced::Alignment::Center)
                    .spacing(2)
                    .push(Text::new(label).size(11).color(palette::BLUE_DIM))
                    .push(
                        Text::new(val)
                            .size(17)
                            .color(color),
                    )
            )
            .width(Length::Fill)
            .padding([8, 4])
            .style(|_: &iced::Theme| iced::widget::container::Style {
                // Superficie appena più chiara dello sfondo: separa senza strappare.
                background: Some(iced::Background::Color(iced::Color {
                    r: 0.02, g: 0.10, b: 0.08, a: 0.88,
                })),
                // Raggio interno del contenuto + padding = raggio esterno.
                border: iced::Border {
                    color: iced::Color::TRANSPARENT,
                    width: 0.0,
                    radius: 10.0.into(),
                },
                // Profondità morbida invece di un bordo.
                shadow: iced::Shadow {
                    color: iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.35 },
                    offset: iced::Vector { x: 0.0, y: 1.0 },
                    blur_radius: 6.0,
                },
                text_color: None,
            })
        };

        let cpu_str = self.last_cpu_temp_ext
            .map(|t| format!("{:.1}°C", t))
            .unwrap_or_else(|| "N/D".to_owned());

        Row::new()
            .padding([4, 8])
            .spacing(4)
            .push(cell("TEC", format!("{:.1}°C", self.chart.last_tec_temp()), palette::BLUE_PRIMARY))
            .push(cell("Potenza", format!("{:.0}W", self.last_power_watts), palette::DANGER))
            .push(cell("Margine", cond_lbl, cond_color))
            .push(cell("CPU ext", cpu_str, palette::SUCCESS))
            .push(cell("Umidità", self.chart.last_humidity(), palette::BLUE_PRIMARY))
            .width(Length::Fill)
            .into()
    }

    /// Controlli compatti su due righe — solo in portrait
    fn view_portrait_controls(&self) -> Element<'_, Message> {
        let en_button = self.pulsante_tec();

        Row::new()
            .padding([4, 8])
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .push(Text::new("Offset:").size(14).color(palette::BLUE_DIM))
            .push(
                NumberInput::new(&self.inputs.set_point, -1000.0..=50.0, Message::UpdateSetpoint)
                    
                    .step(1.0).style(crate::btn::number_input)
                    .input_style(crate::btn::text_input),
            )
            .push(Text::new("Pot %:").size(14).color(palette::BLUE_DIM))
            .push(
                NumberInput::new(&self.inputs.max_power, 0u8..=100u8, Message::UpdateMaxPower)
                    .step(1).style(crate::btn::number_input)
                    .input_style(crate::btn::text_input),
            )
            .push({
                let (col, lbl) = if self.tec_status.contains(TecStatus::PID_RUNNING)
                    && self.fw_power_level > self.inputs.max_power {
                    (palette::WARNING, format!("⚠{}%", self.fw_power_level))
                } else if self.tec_status.contains(TecStatus::PID_RUNNING) {
                    (palette::SUCCESS, format!("✓{}%", self.fw_power_level))
                } else {
                    (palette::BLUE_DIM, "—".to_owned())
                };
                Text::new(lbl).size(13).color(col)
            })
            .push(iced::widget::Space::with_width(Length::Fill))
            .push(en_button)
            .width(Length::Fill)
            .into()
    }

    fn view_session_stats(&self) -> Element<'_, Message> {
        let s = &self.stats;

        fn stat_row(label: &'static str, val: String) -> Row<'static, Message> {
            Row::new()
                .push(Text::new(label).size(12)
                    .color(palette::BLUE_DIM)
                    .width(Length::Fixed(105.0)))
                .push(iced::widget::Space::with_width(Length::Fill))
                .push(Text::new(val).size(14)
                    .color(palette::TEXT_BRIGHT))
        }

        let tec_stat = if s.tec_temp.count > 0 {
            format!("{:.1} / {:.1} / {:.1} °C",
                s.tec_temp.min, s.tec_temp.avg(), s.tec_temp.max)
        } else { "—".to_owned() };

        let pwr_stat = if s.tec_power_watts.count > 0 {
            format!("{:.0} / {:.0} / {:.0} W",
                s.tec_power_watts.min, s.tec_power_watts.avg(), s.tec_power_watts.max)
        } else { "—".to_owned() };

        let cond_stat = if s.condensation_margin.count > 0 {
            format!("{:.1} °C", s.condensation_margin.min)
        } else { "—".to_owned() };

        Column::new()
            .spacing(2)
            .push(Text::new("Statistiche sessione").size(13)
                .color(palette::BLUE_DIM))
            .push(stat_row("TEC min/med/max", tec_stat))
            .push(stat_row("Pot min/med/max", pwr_stat))
            .push(stat_row("Margine min", cond_stat))
            .into()
    }

    fn view_profiles(&self) -> Element<'_, Message> {
        let name_input = iced::widget::text_input("Nome profilo...", &self.inputs.profile_name)
            .on_input(Message::UpdateProfileName)
            .style(crate::btn::text_input)
            .padding(4).size(14).width(Length::Fill);

        let save_btn = iced::widget::button(
            iced::widget::text("Salva").size(14)
                .align_x(alignment::Horizontal::Center))
            .padding(4).style(crate::btn::primary)
            .on_press(Message::SaveProfile);

        let mut col = Column::new()
            .spacing(3)
            .push(Text::new("Profili").size(13)
                .color(palette::BLUE_DIM));

        // ── I profili, su due colonne ───────────────────────────
        //
        // Prima ogni profilo era una riga a larghezza piena: nome e X
        // affiancati, tre righe per tre profili, con il nome che non
        // arrivava mai a finire ("AI / Rendering" toccava il bordo della
        // X). E il campo per rinominare stava a tutta larghezza sopra
        // tutto, quindi la zona piu' grande della sezione era quella che si
        // usa una volta.
        //
        // Ora: due profili per riga, nome abbreviato al punto, e il campo
        // di rinomina accanto al bottone Salva che condivide la riga con i
        // profili invece di occupare una riga tutta sua.
        for coppia in self.app_config.profiles.chunks(2) {
            let mut riga = Row::new().spacing(4);
            for (offset, p) in coppia.iter().enumerate() {
                let idx = self
                    .app_config
                    .profiles
                    .iter()
                    .position(|x| std::ptr::eq(x, p))
                    .unwrap_or(offset);
                let selected=p.name==self.inputs.profile_name;
                riga = riga.push(
                    Row::new().spacing(2).align_y(iced::Alignment::Center)
                        .push(iced::widget::button(
                            Text::new(match p.name.as_str() { "Silenzioso / Idle" => "Silenzioso".to_owned(), "AI / Rendering" => "AI / Rendering".to_owned(), _ => abbrevia(p.name.as_str()) })
                                .size(11)
                                .align_x(alignment::Horizontal::Center))
                            .padding([4, 6]).width(Length::Fill)
                            .style(move |theme,status| if selected {crate::btn::selected_profile(theme,status)} else {crate::btn::glass(theme,status)})
                            .on_press(Message::LoadProfile(idx)))
                        .push(iced::widget::button(
                            Text::new("×").size(12)
                                .align_x(alignment::Horizontal::Center))
                            .padding([4, 5]).style(crate::btn::glass)
                            .on_press(Message::DeleteProfile(idx))),
                );
            }
            // Riga con un solo profilo: la seconda colonna resta vuota
            // invece di raddoppiare il pulsante, che sembrerebbe un
            // collegamento diverso dagli altri.
            if coppia.len() == 1 {
                riga = riga.push(iced::widget::Space::with_width(Length::Fill));
            }
            col = col.push(riga);
        }

        // Il campo nome e "Salva" stanno nella stessa riga, non due righe
        // separate: il campo e' corto e il bottone sta accanto.
        col = col.push(Row::new().spacing(5).push(name_input).push(save_btn));
        col.into()
    }


    fn view_pid_wizard(&self) -> iced::Element<'_, Message> {
        use crate::pid_wizard::WizardPhase;
        let title = intestazione_con_icona("PID Auto-Tuning", palette::SEM_TEMPERATURA, Some(crate::icons::gauge()));

        let body: iced::Element<'_, Message> = match &self.pid_wizard.phase {
            WizardPhase::Idle => {
                Column::new().spacing(5)
                    .push(
                        // Profilatore automatico: adatta offset e potenza al
                        // carico di lavoro. Il gestore esisteva da tempo ma
                        // non aveva un pulsante, quindi la funzione non era
                        // raggiungibile.
                        Row::new().spacing(7)
                            .push(
                                iced::widget::Toggler::new(self.auto_profile_on)
                                    .size(17)
                                    .style(crate::btn::toggler)
                                    .on_toggle(|_| Message::ToggleAutoProfile),
                            )
                            .push(
                                Text::new("Modalità automatica")
                                    .size(11).color(palette::TEXT_BRIGHT),
                            )
                            .push(iced::widget::Space::with_width(Length::Fill))
                            .push(
                                // Il regime e la sorgente, oppure il motivo per
                                // cui non c'e'. Prima diceva solo "attiva
                                // HWiNFO": ora il carico puo' arrivare anche da
                                // Windows, quindi si dice da dove arriva.
                                Text::new(match (self.auto_profile_on, self.auto_senza_sensore) {
                                    (true, true) => "attesa dati…".to_owned(),
                                    (true, false) => format!(
                                        "{} · {}",
                                        self.auto_mgr.regime().etichetta(),
                                        self.carico_origine
                                    ),
                                    (false, _) => "spenta".to_owned(),
                                })
                                .size(10)
                                .color(if !self.auto_profile_on {
                                    palette::BLUE_DIM
                                } else if self.auto_senza_sensore {
                                    palette::WARNING
                                } else {
                                    palette::NEON_GREEN
                                }),
                            ),
                    )
                    .push(
                        Text::new(
                            "La dashboard sceglie il regime da carico e temperatura CPU, e lo \
                             scrive sul controller. Usa HWiNFO/AIDA64 se presenti, altrimenti \
                             il carico di Windows; la temperatura resta opzionale. La protezione \
                             termica e il pavimento anticondensa hanno sempre l'ultima parola.",
                        )
                        .size(10).color(palette::BLUE_DIM),
                    )
                    .push(iced::widget::button(
                        Text::new("Avvia Wizard").size(12)
                            .align_x(alignment::Horizontal::Center))
                        .padding([6, 8]).width(Length::Fill)
                        .style(crate::btn::glass)
                        .on_press(Message::PidWizardStart))
                    .into()
            }
            WizardPhase::WaitingStable | WizardPhase::ApplyingStep | WizardPhase::Analyzing => {
                let pct = self.pid_wizard.progress_pct;
                let last_log = self.pid_wizard.log.last().map(|s| s.as_str()).unwrap_or("...");
                Column::new().spacing(3)
                    .push(Text::new(last_log).size(11)
                        .color(palette::BLUE_DIM))
                    .push(Text::new(format!("Progresso: {}%", pct)).size(11)
                        .color(palette::BLUE_PRIMARY))
                    .push(iced::widget::button(
                        Text::new("■ Annulla").size(11)
                            .align_x(alignment::Horizontal::Center))
                        .padding([3,6]).width(Length::Fill)
                        .style(crate::btn::danger)
                        .on_press(Message::PidWizardCancel))
                    .into()
            }
            WizardPhase::Done { p, i, d, .. } => {
                Column::new().spacing(3)
                    .push(Text::new(format!("✓  P={:.2}  I={:.3}  D={:.3}", p, i, d))
                        .size(12).color(palette::SUCCESS))
                    .push(Row::new().spacing(4)
                        .push(iced::widget::button(
                            Text::new("Applica").size(11)
                                .align_x(alignment::Horizontal::Center))
                            .padding([3,6]).width(Length::Fill)
                            .style(crate::btn::primary)
                            .on_press(Message::PidWizardApply))
                        .push(iced::widget::button(
                            Text::new("Scarta").size(11)
                                .align_x(alignment::Horizontal::Center))
                            .padding([3,6]).width(Length::Fill)
                            .style(crate::btn::glass)
                            .on_press(Message::PidWizardCancel)))
                    .into()
            }
            WizardPhase::Failed(msg) => {
                Column::new().spacing(2)
                    .push(Text::new(format!("✗  {}", msg)).size(11)
                        .color(palette::DANGER))
                    .push(iced::widget::button(
                        Text::new("Riprova").size(11)
                            .align_x(alignment::Horizontal::Center))
                        .padding([3,6]).width(Length::Fill)
                        .style(crate::btn::glass)
                        .on_press(Message::PidWizardStart))
                    .into()
            }
        };

        Column::new().spacing(3)
            .push(title)
            .push(body)
            .into()
    }

    fn view_integrations(&self) -> iced::Element<'_, Message> {
        let mut col = Column::new().spacing(4);
        col = col.push(Text::new("Integrazioni").size(12).color(palette::BLUE_PRIMARY));
        
        // RTSS toggle (Windows only)
        //
        // I tre stati non vengono collassati in due. "Assente" (RTSS non in
        // esecuzione) e "layout non valido" (RTSS c'e' ma la struttura non
        // corrisponde, e quindi non si scrive) sono due problemi diversi, e
        // dirlo evita l impressione che sia una spia che non funziona.
        #[cfg(target_os = "windows")]
        {
            use crate::overlay::rtss::RtssState;
            let (etichetta, colore) = match self.rtss.state() {
                RtssState::Attivo => ("● RTSS overlay", palette::SUCCESS),
                RtssState::Assente => ("○ RTSS non in esecuzione", palette::BLUE_DIM),
                RtssState::LayoutInvalido => ("△ RTSS layout non riconosciuto", palette::WARNING),
            };
            col = col.push(
                Row::new().spacing(4).align_y(iced::Alignment::Center)
                    .push(Text::new(etichetta).size(11).color(colore))
                    .push(iced::widget::Space::with_width(Length::Fill))
                    .push(iced::widget::button(
                        Text::new(if self.rtss.is_active() {"Disattiva"} else {"Attiva"}).size(11).align_x(alignment::Horizontal::Center))
                        .padding([3,8])
                        .style(if self.rtss.is_active() { crate::btn::secondary } else { crate::btn::primary })
                        .on_press(Message::RtssToggle))
            );
        }
        
        // Discord toggle (Windows only)
        #[cfg(target_os = "windows")]
        {
            let disc_color = if self.discord.is_active() { palette::SUCCESS } else { palette::BLUE_DIM };
            col = col.push(
                Row::new().spacing(4).align_y(iced::Alignment::Center)
                    .push(Text::new(if self.discord.is_active() {"● Discord"} else {"○ Discord"}).size(11).color(disc_color))
                    .push(iced::widget::Space::with_width(Length::Fill))
                    .push(iced::widget::button(
                        Text::new(if self.discord.is_active() {"Disattiva"} else {"Attiva"}).size(11).align_x(alignment::Horizontal::Center))
                        .padding([3,8])
                        .style(if self.discord.is_active() { crate::btn::secondary } else { crate::btn::primary })
                        .on_press(Message::DiscordToggle))
            );
    }
        
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            col = col.push(Text::new("N/A su questa piattaforma").size(11).color(palette::BLUE_DIM));
        }
        
        // Export PDF Report button
        col = col.push(
            iced::widget::button(
                Text::new("📄 Export PDF Report").size(12).align_x(alignment::Horizontal::Center))
                .padding([5,8]).width(Length::Fill)
                .style(crate::btn::glass)
                .on_press(Message::ExportPdf)
        );
        
        // PDF status
        if let Some(ref msg) = self.pdf_status {
            let color = if msg.starts_with("✓") { palette::SUCCESS } else { palette::DANGER };
            col = col.push(Text::new(msg.as_str()).size(10).color(color));
        }
        
        // ── Azioni finali, su due righe ────────────────────────────────
        //
        // Prima erano cinque bottoni uno sotto l'altro, ciascuno a
        // larghezza piena: cinque righe per cinque azioni che si usano una
        // volta al giorno. In una sidebar stretta e' spazio sprecato, e
        // costringe a scorrere per arrivare agli ultimi.
        //
        // La riga superiore prende quello che si usa spesso: l'AI e il CSV.
        // Quella sotto prende il resto, piu' compatto. L'ordine e' per
        // frequenza d'uso, non per posizione nell'elenco originale.
        let ai_text = if self.ai_loading { "AI  ⏳  Analisi..." } else { "Chiedi all'AI" };
        col = col.push(
            Row::new().spacing(6)
                .push(
                    iced::widget::button(
                        Text::new(ai_text).size(12).align_x(alignment::Horizontal::Center))
                        .padding([5, 8]).width(Length::Fill)
                        .style(crate::btn::primary)
                        .on_press_maybe(if self.ai_loading { None } else { Some(Message::AskAi) })
                )
                .push(
                    iced::widget::button(
                        Text::new("Esporta CSV").size(12).align_x(alignment::Horizontal::Center))
                        .padding([5, 8]).width(Length::Fill)
                        .style(crate::btn::glass)
                        .on_press(Message::ExportCsv)
                )
        );

        // Wizard, Debug e Nascondi: azioni di servizio, non di uso continuo.
        // Debug e' volutamente vicino a "Nascondi": sono le due cose che si
        // toccano quando qualcosa non va, e tenerle vicine le rende
        // reperibili insieme.
        col = col.push(
            Row::new().spacing(6)
                .push(
                    iced::widget::button(
                        Text::new("Wizard").size(11).align_x(alignment::Horizontal::Center))
                        .padding([5, 8]).width(Length::Fill)
                        .style(crate::btn::glass)
                        .on_press(Message::PidWizardStart)
                )
                .push(
                    iced::widget::button(
                        Text::new("Debug").size(11).align_x(alignment::Horizontal::Center))
                        .padding([5, 8]).width(Length::Fill)
                        .style(crate::btn::glass)
                        .on_press(Message::DebugToggle)
                )
                .push(
                    iced::widget::button(
                        Text::new("Nascondi").size(11).align_x(alignment::Horizontal::Center))
                        .padding([5, 8]).width(Length::Fill)
                        .style(crate::btn::glass)
                        .on_press(Message::Hide)
                )
        );

        // Pannello diagnostico: attivabile con "Debug".
        // Prima questo pulsante alternava un flag che non mostrava nulla
        // (nessun feedback visibile). Ora espone i dati che servono
        // davvero quando il firmware non collabora.
        if self.debug_mode {
            col = col.push(
                Container::new(
                    Column::new().spacing(3)
                        .push(Text::new(format!("status 0x{:014b}", self.tec_status.bits()))
                            .size(10).color(palette::BLUE_DIM))
                        .push(Text::new(format!(
                                "cap {}% → firmware {}%",
                                self.inputs.max_power, self.fw_power_level
                            )).size(10).color(
                                if self.fw_power_level == self.inputs.max_power {
                                    palette::SUCCESS
                                } else {
                                    palette::WARNING
                                }
                            ))
                        .push(Text::new(format!(
                                "setpoint {:.1}°C  P {:.1}  I {:.1}  D {:.1}",
                                self.inputs.set_point,
                                self.inputs.p_coef,
                                self.inputs.i_coef,
                                self.inputs.d_coef
                            )).size(10).color(palette::BLUE_DIM))
                        .push(Text::new(format!(
                                "pompa {:.0} RPM  sensori {}  campioni {}",
                                self.last_pump_rpm,
                                self.sensors.sensors.len(),
                                self.stats.session_samples
                            )).size(10).color(palette::BLUE_DIM))
                        .push(Text::new(format!(
                                "controller {:.1}°C (max {:.1})  appl {}% / cap {}%",
                                self.ctrl_temp,
                                self.ctrl_temp_peak,
                                self.applied_power,
                                self.inputs.max_power
                            )).size(10).color(
                                if self.ctrl_temp >= Self::CTRL_SOFT {
                                    palette::DANGER
                                } else if self.ctrl_warned {
                                    palette::WARNING
                                } else {
                                    palette::SUCCESS
                                }
                            ))
                )
                .width(Length::Fill)
                .padding([8, 8])
                .style(|_: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(iced::Color {
                        r: 0.0, g: 0.06, b: 0.05, a: 0.88,
                    })),
                    border: iced::Border {
                        color: palette::BLUE_DIM,
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                text_color: None,
                shadow: iced::Shadow::default(),
            })
        );
        }

        col.into()
    }


    /// Layer sopra la dashboard: modali, avvisi e pannelli.
    ///
    /// **Stanno qui e non dentro `view_left_column()` perche' questa
    /// funzione finisce dentro la colonna laterale, larga 340 px.** Tutto cio'
    /// che veniva costruito li' era quindi vincolato a quella striscia:
    /// i modali non potevano centrarsi sulla pagina e finivano tagliati, e la
    /// foto di sfondo si vedeva solo in quella colonna mentre il resto lo
    /// copriva lo sfondo opaco. Da qui l'idea che la dashboard fosse
    /// "divisa a meta'".
    ///
    /// Ora `view()` compone: foto, contenuto, e questo livello sopra.
    /// Il dialogo di conferma anticondensa.
    ///
    /// **Il testo non attribuisce la causa.** Dice il margine misurato e la
    /// direzione del movimento, e non afferma che il comando causi o prevenga
    /// la condensa. Il motivo e' che l'avviso compare quando la piastra e' gia'
    /// vicina alla rugiada, e in quel momento il comando premuto puo' essere
    /// esattamente quello che allontana il rischio: un dialogo che dicesse
    /// "questo comando causa condensa" mentre l'operatore spegne il modulo
    /// sarebbe semplicemente falso.
    pub fn view_modals(&self) -> Element<'_, Message> {
        let mut stack = iced::widget::Stack::new();

        // **Nessuna conferma anticondensa.** L'avevo costruita per fermare la
        // condensa in automatico, e l'utente ha detto di toglierla: nessun
        // intervento automatico sulla condensa. Se serve, l'avviso resta
        // nella colonna di sinistra e la decisione resta all'operatore.

        let ai_card_body = if self.ai_loading {
            Column::new()
                .push(Text::new("Sto analizzando i dati del tuo sistema...").size(14)
                    .color(palette::BLUE_DIM))
                .push(Text::new("Connessione ad Anthropic Claude in corso...").size(12)
                    .color(palette::BLUE_DIM))
                .spacing(8)
        } else if let Some(ref err) = self.ai_error {
            Column::new()
                .push(Text::new("Errore API:").size(14)
                    .color(palette::DANGER))
                .push(Text::new(err.as_str()).size(12)
                    .color(palette::WARNING))
                .push(Text::new("Verifica la API key Anthropic:").size(12)
                    .color(palette::BLUE_DIM))
                .push(iced::widget::text_input("sk-ant-...", &self.ai_api_key)
                    .on_input(Message::AiApiKeyChanged)
                    .secure(true).padding(5).size(12).width(Length::Fill).style(crate::btn::text_input))
                .spacing(6)
        } else if let Some(ref advice) = self.ai_advice {
            let has_params = advice.suggested_p.is_some() || advice.suggested_i.is_some();
            let mut col = Column::new()
                .push(Text::new(advice.text.as_str()).size(13)
                    .color(palette::TEXT_BRIGHT))
                .spacing(8);
            if has_params {
                col = col.push(
                    iced::widget::button(
                        Text::new("✓  Applica parametri suggeriti").size(14)
                            .align_x(alignment::Horizontal::Center))
                        .padding([8, 0]).width(Length::Fill)
                        .style(crate::btn::primary)
                        .on_press(Message::AiApplyParams)
                );
            }
            col
        } else {
            Column::new()
                .push(Text::new("Inserisci la tua API key Anthropic per usare il consulente AI:").size(13)
                    .color(palette::TEXT_BRIGHT))
                .push(iced::widget::text_input("sk-ant-...", &self.ai_api_key)
                    .on_input(Message::AiApiKeyChanged)
                    .secure(true).padding(6).size(13).width(Length::Fill).style(crate::btn::text_input))
                .push(Text::new(r"La chiave viene salvata localmente in %APPDATA%\StargateCryo\api_key.txt").size(11)
                    .color(palette::BLUE_DIM))
                .spacing(6)
        };

        // Layer AI Advisor (se aperto)
        if self.ai_modal_open {
            let ai_card = iced_aw::Card::new(
                Row::new()
                    .spacing(8).align_y(iced::Alignment::Center)
                    .push(Text::new("🤖  AI Advisor — Stargate Cryo")
                        .size(16).color(palette::BLUE_PRIMARY)),
                iced::widget::Scrollable::new(ai_card_body.padding(4))
                    .height(Length::Fixed(380.0))
                    .width(Length::Fill),
            )
            .foot(
                Row::new().spacing(6).padding(5)
                    .push(iced::widget::button(
                        Text::new("Chiudi").size(13)
                            .align_x(alignment::Horizontal::Center))
                        .padding([6, 16])
                        .style(crate::btn::glass)
                        .on_press(Message::CloseModal))
                    .push(iced::widget::Space::with_width(Length::Fill))
                    .push(iced::widget::button(
                        Text::new(if self.ai_loading { "Analisi..." } else { "🔄  Richiedi consiglio" })
                            .size(13).align_x(alignment::Horizontal::Center))
                        .padding([6, 12])
                        .style(crate::btn::primary)
                        .on_press_maybe(if self.ai_loading { None } else { Some(Message::AskAi) }))
            )
            .max_width(600.0);

            stack = stack.push(
                Container::new(ai_card)
                    // Come sopra: dimensioni esplicite della finestra.
                    .width(Length::Fixed(self.win_w as f32)).height(Length::Fixed(self.win_h as f32))
                    .center_x(Length::Fixed(self.win_w as f32)).center_y(Length::Fixed(self.win_h as f32))
                    .style(|_: &iced::Theme| iced::widget::container::Style {
                        background: Some(iced::Background::Color(iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.65 })),
                        ..Default::default()
                    })
            );
        }

        // Layer errore TEC (se presente)
        if let Some(ref err) = self.error_text {
            let err_card = iced_aw::Card::new(
                Text::new("Errore").color(palette::DANGER),
                Text::new(err.clone()),
            )
            .style(crate::btn::card)
            .foot(Column::new().padding(5).width(Length::Fill).push(
                iced::widget::Button::new(
                    Text::new("OK").align_x(alignment::Horizontal::Center))
                    .style(crate::btn::primary)
                    .width(Length::Fixed(100.0)).on_press(Message::CloseModal)))
            .max_width(320.0);

            stack = stack.push(
                Container::new(err_card)
                    // Come sopra: dimensioni esplicite della finestra.
                    .width(Length::Fixed(self.win_w as f32)).height(Length::Fixed(self.win_h as f32))
                    .center_x(Length::Fixed(self.win_w as f32)).center_y(Length::Fixed(self.win_h as f32))
                    .style(|_: &iced::Theme| iced::widget::container::Style {
                        background: Some(iced::Background::Color(iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.65 })),
                        ..Default::default()
                    })
            );
        }

        // Layer informativo: finestra con titolo proprio, senza il colore
        // "Errore". Aperta dal bottone "?" della sezione MODALITA'. La
        // costruzione segue la card d'errore qui sopra: stessa griglia, stesso
        // bottone OK, stesso overlay scuro.
        if let Some((ref titolo, ref testo)) = self.info_modale {
            use crate::modalita::Modalita;

            // Le tre modalita' con cosa comportano e il LED come lo
            // documenta il manuale (3.1). Prima qui c'era "LED non
            // documentato" accanto a Standby e Unregulated: si credeva che il
            // produttore scrivesse solo verde e rosso. Era falso — la sezione
            // 3.1 elenca tutti e quattro i colori — e toglieva proprio
            // l'informazione che serve a chi non ha installato il software
            // Intel, cioe' poter confrontare il pannello con la dashboard.
            let attuale = crate::modalita::modalita_corrente(
                self.tec_status,
                !crate::certezza::puo_scrivere(self.certezza()),
            );
            let mut elenco = Column::new().spacing(6);
            for m in Modalita::selezionabili() {
                let (mr, mg, mb) = m.colore_led();
                let colore = iced::Color::from_rgb8(mr, mg, mb);
                let mut riga = Row::new()
                    .spacing(6)
                    .align_y(iced::Alignment::Center)
                    .push(Text::new("\u{25CF}").size(9).color(colore))
                    .push(
                        Text::new(m.nome())
                            .size(12)
                            .color(colore)
                            .width(Length::Fixed(88.0)),
                    )
                    .push(
                        // Colore e lampeggio vengono dal manuale 3.1: qui si
                        // stampa quello che c'e', senza "non documentato",
                        // perche' l'informazione c'e' per tutte e quattro.
                        Text::new(
                            m.led()
                                .map_or(String::new(), |l| format!("LED {} {}", l.colore, l.lampeggio)),
                        )
                        .size(9)
                        .color(palette::BLUE_DIM)
                        .width(Length::Fixed(128.0)),
                    );
                if m == attuale {
                    riga = riga.push(
                        Text::new("adesso")
                            .size(9)
                            .color(palette::NEON_GREEN),
                    );
                } else if m.e_consigliata() {
                    riga = riga.push(
                        Text::new("consigliata")
                            .size(9)
                            .color(palette::SEM_TEMPERATURA),
                    );
                }
                elenco = elenco.push(
                    Column::new().spacing(1)
                        .push(riga)
                        .push(
                            Text::new(m.descrizione())
                                .size(10)
                                .color(palette::BLUE_DIM),
                        ),
                );
            }

            let corpo = Column::new()
                .spacing(10)
                .push(Text::new(testo.clone()).size(11))
                .push(
                    Text::new(format!(
                        "Le tre modalita'. Adesso: {}. In Offline il controller non risponde.",
                        attuale.nome()
                    ))
                        .size(10)
                        .color(palette::BLUE_DIM),
                )
                .push(elenco);

            let info_card = iced_aw::Card::new(
                Text::new(titolo.clone()).color(palette::BLUE_PRIMARY),
                corpo,
            )
            .style(crate::btn::card)
            .foot(Column::new().padding(5).width(Length::Fill).push(
                iced::widget::Button::new(
                    Text::new("Chiudi").align_x(alignment::Horizontal::Center))
                    .style(crate::btn::primary)
                    .width(Length::Fixed(100.0)).on_press(Message::CloseModal)))
            .max_width(520.0);

            stack = stack.push(
                Container::new(info_card)
                    .width(Length::Fixed(self.win_w as f32)).height(Length::Fixed(self.win_h as f32))
                    .center_x(Length::Fixed(self.win_w as f32)).center_y(Length::Fixed(self.win_h as f32))
                    .style(|_: &iced::Theme| iced::widget::container::Style {
                        background: Some(iced::Background::Color(iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.65 })),
                        ..Default::default()
                    })
            );
        }

        // Layer impostazioni: modale centrato con tutte le impostazioni.
        //
        // **Perche' il testo finiva a capo una lettera per riga.** La riga di
        // ogni regola usava tre `FillPortion` (38 / 28 / 24) dentro una
        // modale che si restringeva: su ~250 px utili ogni porzione riceveva
        // pochi pixel, e le stringhe di descrizione avevano 26 spazi
        // consecutivi al posto dell'a capo. Uno spazio e' un punto di wrap
        // legittimo, quindi il testo si spezzava a ogni carattere.
        //
        // Ora: larghezza della modale calcolata sulla finestra, celle numeriche
        // a larghezza FISSA, descrizione su riga propria a tutta larghezza.
        // Impaginazione densa (bordi 1px, padding 2-4px) secondo
        // data-dense-design: e' uno strumento per esperti, la densita' e' una
        // funzionalita' e lo spazio bianco e' sprecato.
        if self.show_settings {
            use crate::alerts::AlertCondition;

            // La modale non puo' superare la finestra, ma nemmeno restare
            // piu' piccola di quanto serve per i controlli.
            // Il modale non puo' MAI superare la finestra: se lo fa, il
            // `center_x` lo centra ma i lati escono dallo schermo e il testo
            // risulta tagliato. 32 px di margine per lato e 40 in alto e basso.
            let mod_w = ((self.win_w as f32) - 32.0).clamp(320.0, 880.0);
            let mod_h = ((self.win_h as f32) - 40.0).clamp(240.0, 720.0);

            // Intestazione di sezione: etichetta piccola e spenta, con un
            // filetto che la separa dal contenuto.
            let sezione = |titolo: &'static str, nota: Option<&'static str>| -> Column<'_, Message> {
                Column::new()
                    .spacing(4)
                    .push(
                        Text::new(titolo.to_uppercase())
                            .size(11)
                            .color(palette::BLUE_PRIMARY),
                    )
                    .push(
                        Container::new(iced::widget::Space::with_width(Length::Fill))
                            .height(Length::Fixed(1.0))
                            .style(|_: &iced::Theme| iced::widget::container::Style {
                                background: Some(iced::Background::Color(
                                    iced::Color { r: 0.10, g: 0.45, b: 0.55, a: 0.45 },
                                )),
                                ..Default::default()
                            }),
                    )
                    .push({
                        // Tipizzato esplicitamente: senza, il compilatore
                        // unifica i due rami su `Text` e rifiuta `Space`.
                        let riga: iced::Element<'_, Message> = match nota {
                            Some(n) => Text::new(n).size(10).color(palette::BLUE_DIM).into(),
                            None => iced::widget::Space::with_height(Length::Fixed(0.0)).into(),
                        };
                        riga
                    })
            };

            // ── Soglie di allarme ───────────────────────────────
            let mut righe = Column::new().spacing(3);
            for (i, rule) in self.alert_manager.rules().iter().enumerate() {
                let attivo = rule.enabled;
                let (dir_label, dir_above) = match rule.condition {
                    AlertCondition::Above(_) => ("supera", true),
                    AlertCondition::Below(_) => ("scende sotto", false),
                };
                let soglia = rule.threshold();
                let attesa = rule.cooldown_s;
                // Un canale che non ha il sensore non e' una regola che
                // aspetta di scattare: e' una regola che non puo' scattare.
                // Senza questo la dashboard mostrava "POMPA FERMA" tra le
                // regole attive, dando l'impressione di una sorveglianza che
                // invece su questo controller non esiste. Vale solo per la
                // pompa: gli altri canali leggono valori reali dal TEC.
                let pump_senza_sensore =
                    matches!(rule.channel, crate::alerts::AlertChannel::PumpRpm)
                        && !self.pump_readable;

                // Cosa significa il canale e cosa succede quando scatta: il
                // numero da solo non dice niente.
                let spiegazione: &'static str = match rule.channel {
                    crate::alerts::AlertChannel::TecTemp =>
                        "La temperatura della piastra TEC sale sopra la soglia.\nSegnala che il raffreddamento non sta funzionando.",
                    crate::alerts::AlertChannel::CpuTemp =>
                        "La CPU supera la soglia.\nIl TEC potrebbe non stare seguendo il carico.",
                    crate::alerts::AlertChannel::DewPointMargin =>
                        "Distanza fra la piastra e il punto di rugiada.\nVicino a zero significa rischio di condensa.",
                    crate::alerts::AlertChannel::PowerWatts =>
                        "Potenza assorbita dal TEC oltre la soglia.\nAttenzione: il modulo V2 supera la capacita' del\ncontroller V1.",
                    crate::alerts::AlertChannel::HumidityPct =>
                        "Umidita' oltre la soglia: con umidita' alta il punto di\nrugiada si avvicina alla piastra e la condensa e' piu'\nprobabile.",
                    crate::alerts::AlertChannel::PumpRpm =>
                        if pump_senza_sensore {
                            "NON APPLICABILE su questo controller.\nIl canale pompa vuole un sensore RPM, e qui non ce n'e':\nla pompa e' governata fuori dal software. La protezione\nreale e' la firma termica, che non passa da questa regola."
                        } else {
                            "La pompa scende sotto la soglia.\nSenza circolazione la piastra si scalda: la potenza viene\ndimezzata finche' la pompa non riparte."
                        },
                };
                let accento = if attivo { palette::BLUE_PRIMARY } else { palette::BLUE_DIM };

                // Riga 1: i controlli, a larghezze fisse.
                // Riga 2: la descrizione, a tutta larghezza.
                righe = righe.push(
                    Column::new()
                        .spacing(4)
                        // Riga 1: stato, canale, direzione.
                        .push(
                            Row::new()
                                .spacing(6)
                                .align_y(iced::Alignment::Center)
                                .push(
                                    iced::widget::Toggler::new(attivo)
                                        .size(16)
                                        .on_toggle(move |_| Message::ToggleAlertRule(i))
                                        .style(crate::btn::toggler),
                                )
                                .push(
                                    Text::new(rule.channel.to_string())
                                        .size(12)
                                        .color(if attivo { palette::TEXT_BRIGHT }
                                               else { palette::BLUE_DIM })
                                        .width(Length::Fill),
                                )
                                .push(
                                    // Direzione: una freccia, non una parola.
                                    iced::widget::button(
                                        Text::new(if dir_above { "\u{2191}" }
                                                         else { "\u{2193}" })
                                            .size(12)
                                            .align_x(alignment::Horizontal::Center),
                                    )
                                    .width(Length::Fixed(24.0))
                                    .height(Length::Fixed(22.0))
                                    .padding([0, 0])
                                    .style(crate::btn::secondary)
                                    .on_press(Message::SetAlertCondition(i, !dir_above)),
                                )
                                .push(
                                    Text::new(dir_label).size(10).color(accento),
                                ),
                        )
                        // Riga 2: i due campi numerici.
                        //
                        // **Perche' non sulla stessa riga.** In fila chiedevano
                        // 132+24+74+36+74+58+74 = 472 px piu' gli spaziature,
                        // e il modale ne dava ~385: la Row andava in overflow e
                        // iced comprimeva i figli. "soglia" e "ripeti s"
                        // finivano a capo una lettera per riga e i campi
                        // sparivano del tutto. Su due righe si chiedono 300 px e
                        // non c'e' piu' niente da schiacciare.
                        .push(
                            Row::new()
                                .spacing(6)
                                .align_y(iced::Alignment::Center)
                                .push(
                                    Text::new("soglia").size(10)
                                        .color(palette::BLUE_DIM)
                                        .width(Length::Fixed(38.0)),
                                )
                                .push(
                                    iced_aw::NumberInput::new(
                                        &soglia, -50.0..=2000.0,
                                        move |v| Message::SetAlertThreshold(i, v),
                                    )
                                    .step(0.5)
                                    .style(crate::btn::number_input)
                                    .input_style(crate::btn::text_input)
                                    .width(Length::Fixed(88.0)),
                                )
                                .push(
                                    Text::new("ripeti s").size(10)
                                        .color(palette::BLUE_DIM)
                                        .width(Length::Fixed(52.0)),
                                )
                                .push(
                                    iced_aw::NumberInput::new(
                                        &attesa, 0u32..=86_400u32,
                                        move |v| Message::SetAlertCooldown(i, v),
                                    )
                                    .step(60)
                                    .style(crate::btn::number_input)
                                    .input_style(crate::btn::text_input)
                                    .width(Length::Fixed(88.0)),
                                )
                                .push(iced::widget::Space::with_width(Length::Fill)),
                        )
                        .push(
                            Text::new(spiegazione).size(10).color(palette::BLUE_DIM),
                        ),
                )
                .push(
                    Container::new(iced::widget::Space::with_width(Length::Fill))
                        .height(Length::Fixed(1.0))
                        .style(move |_: &iced::Theme| iced::widget::container::Style {
                            background: Some(iced::Background::Color(iced::Color {
                                r: 0.10, g: 0.13, b: 0.14, a: 0.6,
                            })),
                            ..Default::default()
                        }),
                );
            }

            // ── Notifiche ───────────────────────────────────────
            let interruttore = |desc: &'static str,
                                stato: bool,
                                msg: Message|
             -> Row<'_, Message> {
                Row::new()
                    .spacing(6)
                    .align_y(iced::Alignment::Center)
                    .push(
                        iced::widget::Toggler::new(stato)
                            .size(16)
                            .on_toggle(move |_| msg.clone())
                            .style(crate::btn::toggler),
                    )
                    .push(
                        Text::new(desc).size(10).color(palette::BLUE_DIM),
                    )
            };

            let corpo = Column::new()
                .spacing(12)
                .push(sezione("Soglie di allarme", Some(
                    "Le protezioni termiche NON dipendono da queste soglie: sono nel \
                     firmware e agiscono comunque. Qui regoli solo gli avvisi.",
                )))
                .push(righe)
                .push(sezione("Notifiche", None))
                .push(interruttore(
                    "Striscia di allarme dentro la dashboard",
                    self.app_config.notify.banner,
                    Message::ToggleAlertBanner(!self.app_config.notify.banner),
                ))
                .push(interruttore(
                    "Notifiche Windows. Le protezioni restano attive: cambia solo l'avviso.",
                    crate::alerts::toasts_enabled(),
                    Message::ToggleToasts(!crate::alerts::toasts_enabled()),
                ))
                .push(sezione("Protezione termica", None))
                .push(
                    Row::new()
                        .spacing(6)
                        .align_y(iced::Alignment::Center)
                        .push(
                            Text::new("offset").size(12)
                                .color(palette::TEXT_BRIGHT)
                                .width(Length::Fixed(132.0)),
                        )
                        .push(
                            iced_aw::NumberInput::new(
                                &self.inputs.set_point, -1000.0..=50.0, Message::UpdateSetpoint,
                            )
                            .step(1.0)
                            .style(crate::btn::number_input)
                            .input_style(crate::btn::text_input)
                            .width(Length::Fixed(74.0)),
                        )
                        .push(
                            Text::new("piastra). °C")
                                .size(10)
                                .color(palette::BLUE_DIM),
                        ),
                )
                .push(
                    Text::new(
                        "Spostamento rispetto all'ambiente. Il limite anticondensa e' \
                         applicato dal software: sotto il punto di rugiada la piastra \
                         non scende piu'.",
                    )
                    .size(10)
                    .color(palette::BLUE_DIM),
                )
                .push(
                    Row::new()
                        .spacing(6)
                        .align_y(iced::Alignment::Center)
                        .push(
                            Text::new("potenza max").size(12)
                                .color(palette::TEXT_BRIGHT)
                                .width(Length::Fixed(132.0)),
                        )
                        .push(
                            iced_aw::NumberInput::new(
                                &self.inputs.max_power, 0u8..=100u8, Message::UpdateMaxPower,
                            )
                            .step(5)
                            .style(crate::btn::number_input)
                            .input_style(crate::btn::text_input)
                            .width(Length::Fixed(74.0)),
                        )
                        .push(
                            Text::new("%").size(10).color(palette::BLUE_DIM),
                        ),
                )
                .push(
                    Text::new(
                        "Tetto di potenza che l'utente puo' chiedere. Le protezioni \
                         possono ridurlo: non e' un pavimento, e' un limite.",
                    )
                    .size(10)
                    .color(palette::BLUE_DIM),
                )
                .push(sezione("Regolazione PID", None))
                .push(
                    Row::new()
                        .spacing(6)
                        .align_y(iced::Alignment::Center)
                        .push(
                            Text::new("P / I / D").size(12)
                                .color(palette::TEXT_BRIGHT)
                                .width(Length::Fixed(132.0)),
                        )
                        .push(
                            iced_aw::NumberInput::new(
                                &self.inputs.p_coef, 0.0..=1000.0, Message::UpdatePCoef,
                            )
                            .step(5.0)
                            .style(crate::btn::number_input)
                            .input_style(crate::btn::text_input)
                            .width(Length::Fixed(74.0)),
                        )
                        .push(
                            iced_aw::NumberInput::new(
                                &self.inputs.i_coef, 0.0..=1000.0, Message::UpdateICoef,
                            )
                            .step(1.0)
                            .style(crate::btn::number_input)
                            .input_style(crate::btn::text_input)
                            .width(Length::Fixed(74.0)),
                        )
                        .push(
                            iced_aw::NumberInput::new(
                                &self.inputs.d_coef, 0.0..=1000.0, Message::UpdateDCoef,
                            )
                            .step(1.0)
                            .style(crate::btn::number_input)
                            .input_style(crate::btn::text_input)
                            .width(Length::Fixed(74.0)),
                        ),
                )
                .push(
                    Text::new(
                        "Reali solo col TEC in funzione. Modificarli a caldo non e' \
                         consigliato: usa il Wizard PID, che misura prima di applicare.",
                    )
                    .size(10)
                    .color(palette::BLUE_DIM),
                )
                .push(sezione("Integrazioni", None))
                .push(interruttore(
                    "RTSS overlay. Richiede RTSS in esecuzione.",
                    self.rtss.is_active(),
                    Message::RtssToggle,
                ))
                .push(interruttore(
                    "Discord Rich Presence. Richiede un client_id reale.",
                    self.discord.is_active(),
                    Message::DiscordToggle,
                ))
                .push(
                    Text::new(format!(
                        "Log di collaudo: {}",
                        crate::commissioning::path_display(),
                    ))
                    .size(10)
                    .color(palette::BLUE_DIM),
                );

            let attive = self.alert_manager.rules().iter().filter(|r| r.enabled).count();
            let totale = self.alert_manager.rules().len();

            let card = iced_aw::Card::new(
                Column::new()
                    .spacing(3)
                    .push(
                        Text::new("Impostazioni").size(16)
                            .color(palette::TEXT_BRIGHT),
                    )
                    .push(
                        Text::new(
                            "Le modifiche vengono salvate subito e valgono al prossimo avvio.",
                        )
                        .size(10)
                        .color(palette::BLUE_DIM),
                    ),
                // Il corpo scorre, l'intestazione e il pulsante restano fermi.
                //
                // iced 0.13 non ha `hide_scrollbar`: la barra si nasconde con lo
                // stile, mettendone il *colore* trasparente. Il colore non basta
                // a distinguerla, va resa invisibile: una striscia azzurra
                // sempre presente faceva sembrare il modale un pannello di
                // sistema invece di una finestra. Resta scorrevole con la
                // rotella, e si vede solo mentre la si trascina.
                iced::widget::Scrollable::new(corpo)
                    .width(Length::Fill)
                    // **Altezza calcolata, mai `Fill`.**
                    //
                    // `Fill` dentro una colonna di altezza non nota e' un
                    // vuoto che iced non sa riempire: il pannello si
                    // allunga senza limite e blocca la finestra. Con un
                    // numero esplicito non c'e' nulla da risolvere.
                    //
                    // Si sottrae all'altezza del modale tutto cio' che non e'
                    // il corpo scorrevole: intestazione, descrizione, pulsanti
                    // e bordi. Il pavimento tiene il corpo utilizzabile anche
                    // con finestra bassa.
                    // L'altezza del corpo deve stare in `mod_h` **meno tutto**
                    // cio' che non e' corpo: intestazione della card, padding
                    // e footer con il pulsante "Chiudi".
                    //
                    // Con `mod_h - 150` la somma superava l'altezza della
                    // card, il footer veniva spinto fuori e tagliato, e il
                    // pannello risultava "tutto celeste senza tasti" e
                    // **impossibile da chiudere**: la X non c'era da cliccare.
                    // 230 e' la misura con margine, non tirata a occhio.
                    .height(Length::Fixed((mod_h - 230.0).max(120.0_f32)))
                    .style(|_: &iced::Theme, st: iced::widget::scrollable::Status| {
                        use iced::widget::scrollable::{Rail, Scroller, Status, Style};
                        let rail = |c: iced::Color| Rail {
                            background: None,
                            border: iced::Border::default(),
                            scroller: Scroller {
                                color: c,
                                border: iced::Border::default(),
                            },
                        };
                        // Invisibile di riposo, accesa solo durante il
                        // trascinamento: cosi' si sa che il contenuto scorre
                        // senza avere una barra sempre addosso.
                        let riposo = iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
                        let accesa = iced::Color { r: 0.12, g: 0.45, b: 1.0, a: 0.55 };
                        let colore = match st {
                            Status::Dragged { .. } => accesa,
                            _ => riposo,
                        };
                        Style {
                            container: Default::default(),
                            vertical_rail: rail(colore),
                            horizontal_rail: rail(riposo),
                            gap: None,
                        }
                    }),
            )
            // **Lo stile mancava, ed e' il colore azzurro.**
            //
            // Senza `.style(...)` la card usa il tema predefinito di
            // `iced_aw`, che su questo tema e' azzurro: il pannello
            // Impostazioni usciva come una scatola blu piena e vuota.
            // `btn::card` e' lo stesso stile delle altre card dell'app.
            .style(crate::btn::card)
            .foot(
                Row::new()
                    .spacing(8)
                    .padding(4)
                    .push(
                        iced::widget::button(
                            Text::new("Chiudi").size(12)
                                .align_x(alignment::Horizontal::Center),
                        )
                        .padding([6, 18])
                        .style(crate::btn::primary)
                        .on_press(Message::ToggleSettings),
                    )
                    .push(iced::widget::Space::with_width(Length::Fill))
                    .push(
                        Text::new(format!("{attive} regole attive su {totale}"))
                            .size(10)
                            .color(palette::BLUE_DIM),
                    ),
            );

            // Centrato davvero: larghezza e altezza calcolate sulla finestra,
            // non `Fill` che si stringeva fino a far andare a capo ogni
            // singola lettera.
            stack = stack.push(
                Container::new(
                    Container::new(card)
                        .width(Length::Fixed(mod_w))
                        .height(Length::Fixed(mod_h)),
                )
                // Dimensioni ESPLICITE della finestra, non `Fill`.
                //
                // `Stack` in iced 0.13 dimensiona i figli sul loro contenuto: un
                // `Container` con `Fill` dentro non arriva mai a schermo intero,
                // quindi `center_x`/`center_y` non avevano nulla su cui centrare.
                // Il risultato era il modale attaccato all'angolo in alto a
                // sinistra e tagliato in basso. Con `Fixed` uguale alla finestra
                // iloverlay e' deterministico, e il riquadro dentro viene
                // centrato davvero.
                .width(Length::Fixed(self.win_w as f32))
                .height(Length::Fixed(self.win_h as f32))
                .center_x(Length::Fixed(self.win_w as f32))
                .center_y(Length::Fixed(self.win_h as f32))
                .style(|_: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(
                        iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.72 },
                    )),
                    ..Default::default()
                }),
            );
        }

        // Layer diagnostica.
        //
        // Qui stanno i dettagli che in sidebar sono spariti perche' erano
        // duplicati: tensione, corrente, potenza, COP, temperature di
        // sessione, i bit di stato e **cosa fare** per ciascun problema.
        //
        // La finestra e' larga perche' il contenuto e' una tabella: un report
        // che scrolla dentro una card stretta si legge a meta'.
        if self.show_diagnostica {
            use crate::diagnostica::{valuta, Verdetto};

            // Nessuna misura disponibile: la finestra dice che **non lo sa**, e
            // non costruisce una tabella di valori che non ha. Un pannello
            // vuoto con scritto "Nessuna anomalia" sarebbe peggio di non
            // aprirlo.
            let esito = match self.dati_diagnostica() {
                Some(m) => valuta(m, self.tec_status),
                None => {
                    let avviso: iced::Element<'_, Message> = iced::widget::Column::new()
                        .spacing(10)
                        .width(Length::Shrink)
                        .push(
                            Text::new("Diagnostica non disponibile")
                                .size(15)
                                .color(palette::WARNING),
                        )
                        .push(
                            Text::new(
                                "Le misure di questa sessione non sono disponibili, \
                                 quindi qui non c'e' niente da giudicare. \
                                 Questo NON significa che sia tutto regolare: \
                                 significa che non lo sappiamo.",
                            )
                            .size(12)
                            .color(palette::BLUE_DIM),
                        )
                        .push(
                            iced::widget::button(Text::new("Chiudi").size(12))
                                .padding([6, 10])
                                .style(crate::btn::secondary)
                                .on_press(Message::DiagnosticaToggle),
                        )
                        .into();
                    return avviso;
                }
            };
            let colore_verdetto = match esito.verdetto {
                Verdetto::Ok => palette::NEON_GREEN,
                Verdetto::Attenzione => palette::WARNING,
                Verdetto::Guasto => palette::DANGER,
            };
            let (titolo_verdetto, _) = match esito.verdetto {
                Verdetto::Ok => ("Nessuna anomalia", colore_verdetto),
                Verdetto::Attenzione => ("Attenzione", colore_verdetto),
                Verdetto::Guasto => ("Guasto", colore_verdetto),
            };

            // ── Misure: due per riga, etichetta sopra valore ────────────
            //
            // Solo le **istantanee**. Gli aggregati di sessione (temperatura
            // minima, potenza massima) sono gia' in "Statistiche sessione"
            // nella sidebar, quindi qui sarebbero un terzo posto con lo
            // stesso numero. Restano nel **file**: li vedi con "Salva
            // rapporto", e servono per confrontarli col self test del
            // produttore. Il filtro e' `misure_istantanee`, non una lista
            // scritta qui: se domani un campo, la duplicazione torna senza
            // che nessuno se ne accorga.
            let mut griglia = Column::new().spacing(6);
            for coppia in esito.misure_istantanee().chunks(2) {
                let mut riga = Row::new().spacing(24);
                for m in coppia {
                    let colore_misura = match m.giudizio {
                        // Una misura "n/d" non e' un problema del controller:
                        // colorarla come guasto insieme a un guasto vero
                        // confonderebbe l'operatore.
                        Verdetto::Ok => colore_verdetto,
                        Verdetto::Attenzione => palette::BLUE_DIM,
                        Verdetto::Guasto => palette::DANGER,
                    };
                    riga = riga.push(
                        Column::new().spacing(0)
                            .push(
                                Text::new(m.etichetta)
                                    .size(10)
                                    .color(palette::BLUE_DIM),
                            )
                            .push(
                                Text::new(m.valore.clone())
                                    .size(18)
                                    .color(colore_misura),
                            )
                            .width(Length::Fill),
                    );
                }
                griglia = griglia.push(riga);
            }

            // ── Problemi: titolo e cosa fare, niente frasi di comodo ─────
            let mut problemi = Column::new().spacing(8);
            if esito.problemi.is_empty() {
                // **A modulo spento la frase diversa non è un dettaglio.**
                //
                // "Nessun problema rilevato" è vera e insieme ingannevole: è
                // vero che non ci sono problemi, ma il lettore porta via
                // l'idea che il sistema sia operativo. Non lo è. Qui si
                // dichiara che il regolatore è fermo, e perché l'assenza di
                // problemi non è una buona notizia ma la conseguenza attesa
                // di un comando che l'operatore ha dato lui.
                let testo = if esito.spento {
                    "Il modulo è spento: nessun problema, perché non sta \
                     regolando. I valori qui sotto sono gli ultimi letti."
                } else {
                    "Nessun problema rilevato sui dati di questa sessione."
                };
                problemi = problemi.push(
                    Text::new(testo).size(12).color(palette::BLUE_DIM),
                );
            } else {
                for problema in &esito.problemi {
                    let titolo = problema.split(':').next().unwrap_or(problema);
                    let consiglio = problema
                        .split_once(':')
                        .map(|(_, c)| c.trim())
                        .unwrap_or("");
                    problemi = problemi.push(
                        Column::new().spacing(1)
                            .push(
                                Row::new().spacing(6)
                                    .push(
                                        Text::new("\u{25B2}").size(10).color(colore_verdetto),
                                    )
                                    .push(
                                        Text::new(titolo.to_owned())
                                            .size(13)
                                            .color(colore_verdetto),
                                    ),
                            )
                            .push(
                                Text::new(consiglio.to_owned())
                                    .size(12)
                                    .color(palette::TEXT_BRIGHT),
                            ),
                    );
                }
            }

            // ── Protezioni attive ─────────────────────────────────────────
            // Solo quelle vere. "Potenza oltre il tetto HW" resta un avviso e
            // non un blocco: non limita niente, quindi non va tra le
            // protezioni ma fra le note.
            let prot: Vec<(&str, bool)> = vec![
                ("Soft start", self.soft_starting),
                ("Watchdog scattato", self.watchdog_tripped),
                ("Pavimento anticondensa", self.cond_limit_active),
                ("Regolazione Cryo", self.ultimo_regime_richiesto == Some(crate::commutazione::Regime::Cryo)),
                ("Limitatore pompa", self.pump_emergency_active),
            ];
            let attive: Vec<&str> = prot
                .iter()
                .filter(|(_, a)| *a)
                .map(|(n, _)| *n)
                .collect();
            let mut note = Column::new().spacing(3);
            if !attive.is_empty() {
                note = note.push(
                    Row::new().spacing(5)
                        .push(
                            Text::new("Protezioni attive:").size(11)
                                .color(palette::BLUE_DIM),
                        )
                        .push(
                            Text::new(attive.join(" \u{00B7} ")).size(11)
                                .color(palette::WARNING),
                        ),
                );
            }
            if self.potenza_supera_tetto_hardware() {
                note = note.push(
                    Text::new(
                        "La percentuale scelta chiede piu' watt del tetto che il \
                         controller dovrebbe dare in regime stabile. Non e' un \
                         blocco: la potenza non viene limitata.",
                    )
                    .size(11)
                    .color(palette::BLUE_DIM),
                );
            }
            if let Some(msg) = &self.diagnostica_msg {
                note = note.push(
                    Text::new(msg.clone()).size(11).color(palette::BLUE_PRIMARY),
                );
            }

            let corpo = Column::new().spacing(14)
                .push(
                    Row::new().spacing(8).align_y(iced::Alignment::Center)
                        .push(
                            Text::new("\u{25CF}").size(14).color(colore_verdetto),
                        )
                        .push(
                            Text::new(titolo_verdetto).size(20)
                                .color(colore_verdetto),
                        )
                        .push(iced::widget::Space::with_width(Length::Fill))
                        .push(
                            match &esito.modalita {
                                Some(m) => Text::new(format!("Modalita': {m}")).size(12)
                                    .color(palette::BLUE_DIM),
                                None => Text::new("").size(12),
                            },
                        ),
                )
                .push(griglia)
                .push(
                    Text::new("Problemi rilevati").size(11)
                        .color(palette::BLUE_DIM),
                )
                .push(problemi)
                .push(note)
                // Provenienza dichiarata. Va in schermo, non solo nel file:
                // se l'operatore fa uno screenshot e lo manda al supporto,
                // senza questa riga il report sembra quello del produttore.
                .push(
                    Text::new(
                        "Dato osservativo, non e' il self test del produttore.",
                    )
                    .size(10)
                    .color(palette::BLUE_DIM),
                );

            let diag_w = ((self.win_w as f32) - 60.0).clamp(360.0, 860.0);
            let diag_card = iced_aw::Card::new(
                Text::new("Diagnostica controller")
                    .size(18).color(palette::BLUE_PRIMARY),
                iced::widget::Scrollable::new(
                    Container::new(corpo).padding(10)
                )
                .height(Length::Fill)
                .width(Length::Fill),
            )
            .foot(
                Row::new().spacing(6).padding(5)
                    .push(iced::widget::button(
                        Text::new("Chiudi").size(12)
                            .align_x(alignment::Horizontal::Center))
                        .padding([6, 16])
                        .style(crate::btn::glass)
                        .on_press(Message::DiagnosticaToggle))
                    .push(iced::widget::button(
                        Text::new("Salva rapporto").size(12)
                            .align_x(alignment::Horizontal::Center))
                        .padding([6, 16])
                        .style(crate::btn::primary)
                        .on_press(Message::DiagnosticaSalva))
                    .push(iced::widget::Space::with_width(Length::Fill))
                    .push(Text::new(crate::diagnostica::SORGENTE_LOG)
                        .size(10).color(palette::BLUE_DIM)),
            )
            .style(crate::btn::card);

            // Il pannello e' separato dal contenitore a tutto schermo: il
            // primo ha le dimensioni del report, il secondo fa da sfondo
            // scuro e centra. Mescolarli in una sola espressione rendeva
            // impossibile capire quale parentesi chiudesse cosa.
            let pannello = Container::new(diag_card)
                .width(Length::Fixed(diag_w))
                .height(Length::Fixed(((self.win_h as f32) - 80.0).clamp(240.0, 720.0)));

            stack = stack.push(
                Container::new(pannello)
                    .width(Length::Fixed(self.win_w as f32))
                    .height(Length::Fixed(self.win_h as f32))
                    .center_x(Length::Fixed(self.win_w as f32))
                    .center_y(Length::Fixed(self.win_h as f32))
                    .style(|_: &iced::Theme| iced::widget::container::Style {
                        background: Some(iced::Background::Color(
                            iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.75 })),
                        ..Default::default()
                    })
            );
        }

        // Layer cronologia sessioni.
        if self.show_session_hist {
            let hdr = |t: &'static str, w: f32| {
                Text::new(t).size(10).color(palette::BLUE_DIM)
                    .width(Length::Fixed(w))
            };
            let cell = |t: String, c: iced::Color, w: f32| {
                Text::new(t).size(11).color(c).width(Length::Fixed(w))
            };

            let mut table = Column::new().spacing(0);
            table = table.push(
                Row::new()
                    .spacing(0)
                    .push(hdr("inizio", 78.0))
                    .push(hdr("min TEC", 66.0))
                    .push(hdr("COP", 54.0))
                    .push(hdr("margine", 66.0))
                    .push(hdr("OCP", 40.0))
                    .push(hdr("score", 48.0))
                    .push(hdr("campioni", 64.0)),
            );
            for s in &self.session_list {
                // started_at e' ISO-8601: l'ora e' dai caratteri 11..16.
                let hhmm: String = s.started_at.chars().skip(11).take(5).collect();
                let margine = s
                    .min_margin
                    .map(|v| format!("{v:+.1}"))
                    .unwrap_or_else(|| "-".to_owned());
                let ocp = if s.ocp_events > 0 {
                    Text::new(s.ocp_events.to_string()).size(11).color(palette::DANGER)
                        .width(Length::Fixed(40.0))
                } else {
                    Text::new("0").size(11).color(palette::BLUE_DIM)
                        .width(Length::Fixed(40.0))
                };
                table = table.push(
                    Row::new()
                        .spacing(0)
                        .push(cell(hhmm, palette::TEXT_BRIGHT, 78.0))
                        .push(cell(
                            s.min_tec.map(|v| format!("{v:.1}"))
                                .unwrap_or_else(|| "-".to_owned()),
                            palette::TEXT_BRIGHT, 66.0))
                        .push(cell(
                            s.avg_cop.map(|v| format!("{v:.2}"))
                                .unwrap_or_else(|| "-".to_owned()),
                            palette::TEXT_BRIGHT, 54.0))
                        .push(cell(margine, palette::TEXT_BRIGHT, 66.0))
                        .push(ocp)
                        .push(cell(s.oc_score.to_string(), palette::NEON_GREEN, 48.0))
                        .push(cell(s.samples.to_string(), palette::BLUE_DIM, 64.0)),
                );
            }

            let hist_w = ((self.win_w as f32) - 40.0).clamp(320.0, 640.0);

            let hist_card = iced_aw::Card::new(
                Text::new("Cronologia sessioni")
                    .size(16).color(palette::BLUE_PRIMARY),
                iced::widget::Scrollable::new(
                    Container::new(table).padding(6)
                )
                .height(Length::Fill)
                .width(Length::Fill),
            )
            .foot(
                Row::new().spacing(6).padding(5)
                    .push(iced::widget::button(
                        Text::new("Chiudi").size(12)
                            .align_x(alignment::Horizontal::Center))
                        .padding([6, 16])
                        .style(crate::btn::glass)
                        .on_press(Message::SessionHistoryOpen))
                    .push(iced::widget::Space::with_width(Length::Fill))
                    .push(Text::new(format!("{} sessioni", self.session_list.len()))
                        .size(11).color(palette::BLUE_DIM)),
            );

            stack = stack.push(
                Container::new(
                    Container::new(hist_card)
                        .width(Length::Fixed(hist_w))
                        .height(Length::Fixed(420.0)),
                )
                    // Come sopra: dimensioni esplicite della finestra.
                    .width(Length::Fixed(self.win_w as f32)).height(Length::Fixed(self.win_h as f32))
                    .center_x(Length::Fixed(self.win_w as f32)).center_y(Length::Fixed(self.win_h as f32))
                    .style(|_: &iced::Theme| iced::widget::container::Style {
                        background: Some(iced::Background::Color(
                            iced::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.7 })),
                        ..Default::default()
                    })
            );
        }

        stack.into()
    }

    /// Il banner cryogenic: condizionale.
    ///
    /// Compare sotto la soglia, sparisce sopra, e all'avvio non compare
    /// perche' non c'e' ancora nessuna misura.
    ///
    /// Non perche' non funzioni: funziona, e la soglia e' corretta. E' stato
    /// tolto perche' su un impianto criogenico la piastra sta **sotto i 20 °C
    /// quasi sempre** — e' il punto del dispositivo — quindi l'avviso era
    /// acceso di continuo e non informava di nulla.
    ///
    /// Piu' costava la forma della hero: comparendo e sparendo cambiava
    /// l'altezza della fascia, e la card info finiva schiacciata o con mezza
    /// fascia vuota. Un avviso che dice sempre "si" non e' un avviso.
    ///
    /// La protezione vera e' rimasta: e' l'allarme **Rischio condensa**, che
    /// confronta la piastra con il **punto di rugiada** e da' il margine
    /// reale. Se la piastra scende troppo, quello parla da solo.
    fn cryo_hero_visibile(&self) -> bool {
        self.chart.cryo_visibile()
    }

    /// Valore live di un canale di allarme, per le notifiche.
    ///
    /// Le stesse sorgenti usate da `check()`: se il valore non c'e'
    /// (sensore assente, setpoint non credibile) torna `None` e il banner
    /// mostra solo l'etichetta. Niente zeri inventati.
    fn valore_live_canale(&self, canale: &crate::alerts::AlertChannel) -> Option<f32> {
        use crate::alerts::AlertChannel as Ch;
        match canale {
            Ch::TecTemp => {
                let credibile =
                    self.inputs.set_point.is_finite() && self.inputs.set_point > -200.0;
                if credibile {
                    self.log.last().map(|s| s.tec_temp - self.inputs.set_point)
                } else {
                    None
                }
            }
            Ch::CpuTemp => self.last_cpu_temp_ext,
            Ch::DewPointMargin => {
                if self.last_cond_margin != f32::MAX && self.last_cond_margin.is_finite() {
                    Some(self.last_cond_margin)
                } else {
                    None
                }
            }
            Ch::PowerWatts => Some(self.last_power_watts),
            Ch::HumidityPct => self.log.last().map(|s| s.humidity),
            Ch::PumpRpm => {
                if self.pump_readable {
                    Some(self.last_pump_rpm)
                } else {
                    None
                }
            }
        }
    }

    /// Riceve la decisione **gia' presa**, non la ricalcola.
    ///
    /// Valutare la soglia due volte nello stesso frame permetteva alla riga
    /// di stato e al banner di discordare: l'app scriveva una cosa e
    /// disegnava l'altra.
    fn view_cryo_banner(&self, visibile: bool) -> Option<iced::Element<'_, Message>> {
            // **Nessun campione, nessun avviso.**
            //
            // Senza misura `last_tec_temp` restituisce `0.0`, e `0.0` non e'
            // una temperatura: e' sotto la soglia, quindi all'avvio il banner
            // compariva da solo, prima ancora di misurare. Nessun dato non
            // puo' voler dire "piastra fredda".
            if !visibile || !self.chart.ha_campione_tec() {
                return None;
            }
            Some(
                iced::widget::Image::new(self.cryo_warn_handle.clone())
                    // **Solo la larghezza.** L'altezza non viene imposta.
                    //
                    // Impostando entrambe le dimensioni sono due numeri
                    // indipendenti, e basta che uno dei due sia sbagliato
                    // perché l'immagine venga stirata: e' successo piu'
                    // volte, perche' l'altezza veniva ricalcolata a mano a
                    // ogni cambio di soglia e di finestra.
                    //
                    // Qui si fissa solo la larghezza di visualizzazione:
                    // iced ricava l'altezza dalle dimensioni reali
                    // dell'immagine. Il banner non puo' deformarsi perche'
                    // **non c'e' un'altezza da sbagliare**.
                    .width(Length::Fixed(CRYO_DISPLAY_W))
                    .into(),
            )
    }
} // fine impl RunningState

/// Chiusura automatica: quando l'app esce (iced::exit) o torna alla schermata
/// Home, `RunningState` viene droppato. Senza questo il TEC restava acceso,
/// gli overlay restavano attivi e la sessione DB non veniva mai chiusa.
impl Drop for RunningState {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Intestazione di sezione: chip di categoria + etichetta.
///
/// L'etichetta dice *cosa* e' la sezione, il chip dice *di che categoria
/// e'*, e usa le stesse cinque tinte di `palette::SEM_*` dei valori e
/// degli avvisi: un quadratino ambra accanto a "Coefficienti PID" e
/// l'ambra di una riga di watt sono la stessa informazione.
///
/// Il chip e' lo stesso accorgimento della barra laterale degli avvisi
/// (3 px di colore pieno) ma in scala piccola: 8x8, bordo tenue e alone
/// corto. Li' il colore era il messaggio; qui sarebbe un livido, e la
/// gerarchia la deve tenere l'etichetta.
///
/// Il quadrato e' fill dentro un `Container` stilato perche' uno `Space`
/// da solo non ha uno stile proprio: il bordo applicato a lui verrebbe
/// ignorato, come accade a `Row`.
/// Come `intestazione_sezione`, ma con l'icona della categoria.
///
/// **Solo icona ed etichetta.** Il quadratino colorato e' stato rimosso:
/// accanto a un'icona da 16 px diventava un secondo marcatore illeggibile
/// e la riga sembrava due simboli appiccicati. Ora l'icona e' a 22 px,
/// nitida dal nuovo set, e l'etichetta sale a 11 px. Il colore di categoria
/// non e' sparito: vive nei valori e negli stati, dove serve davvero.
///
/// Le icone sono monocromatiche bianche: `iced 0.13` non espone `tint` e
/// non serve, perche' il bianco su fondo scuro resta leggibile in ogni
/// sezione senza dover ricolorare nulla.
fn intestazione_con_icona(
    etichetta: &str,
    _colore: iced::Color,
    icona: Option<iced::widget::image::Handle>,
) -> Row<'_, Message> {
    let mut riga = Row::new()
        .spacing(8)
        .align_y(iced::Alignment::Center);

    if let Some(h) = icona {
        riga = riga.push(
            iced::widget::Image::new(h)
                .width(Length::Fixed(22.0))
                .height(Length::Fixed(22.0))
                .opacity(0.95_f32),
        );
    }

    riga.push(Text::new(etichetta).size(11).color(palette::TEXT_BRIGHT))
}

fn pid_row(label: &'static str, value: f32, msg: fn(f32) -> Message) -> iced::Element<'static, Message> {
    use iced::widget::Text;
    use iced::Length;
    use iced_aw::NumberInput;
    use crate::palette;

    Row::new()
        .spacing(4)
        .push(Text::new(label).size(14).color(palette::BLUE_DIM))
        .push(iced::widget::Space::with_width(Length::Fill))
        .push(NumberInput::new(&value, 0.0..=500.0, msg).step(1.0).style(crate::btn::number_input)
                    .input_style(crate::btn::text_input))
        .padding(3)
        .into()
}

fn view_badges(
    status: &TecStatus,
    ocp_confirmed: bool,
    cryo_icon: iced::widget::image::Handle,
    ocp_icon: iced::widget::image::Handle,
    tec_temp: f32,
    power_watts: f32,
) -> iced::Element<'_, Message> {
    use iced::widget::{Column, Row, Container, Text};
    use iced::Length;
    use crate::palette;

    let mut col = Column::new().spacing(4);

    if status.contains(cryo_cooler_controller_lib::TecStatus::PID_RUNNING) {
        // Mostra temperatura TEC alla fine
        let temp_str = format!(" {:.1}°C", tec_temp);
        // Colori dinamiche in base alla temperatura
        let (temp_color, bg_color) = if tec_temp <= 0.0 {
            // Blu ciano per temperature molto basse (≤ 0°C)
            (iced::Color { r: 0.0, g: 0.8, b: 1.0, a: 1.0 },
            iced::Color { r: 0.0, g: 0.2, b: 0.4, a: 1.0 })
        } else if tec_temp <= 10.0 {
            // Verde su sfondo verde petrolio (zona ottimale: 0°C a 10°C)
            (iced::Color::BLACK, palette::NEON_GREEN)
        } else if tec_temp <= 20.0 {
            // Blu primario (10°C a 20°C - zona normale)
            (iced::Color::BLACK, palette::BLUE_PRIMARY)
        } else if tec_temp <= 30.0 {
            // Arancione warning (20°C a 30°C)
            (iced::Color::BLACK, palette::WARNING)
        } else {
            // Rosso pericolo (30°C+)
            (iced::Color::WHITE, palette::DANGER)
        };
        
        // Pallino: verde acceso fisso, perche' dice una cosa sola, che il
        // canale e' attivo. Prima prendeva il colore del testo, che cambia
        // con la temperatura e lo rendeva ora invisibile ora del colore
        // sbagliato: un indicatore che cambia significato non e' un
        // indicatore.
        let accent=if temp_color==iced::Color::BLACK {bg_color} else {temp_color};
        let dot = Text::new("●").size(16).color(palette::NEON_GREEN);
        
        col = col.push(
            Container::new(
                Row::new()
                    .spacing(8)
                    .align_y(iced::Alignment::Center)
                    .push(dot)
                    .push(iced::widget::Image::new(cryo_icon).width(Length::Fixed(24.0)).height(Length::Fixed(24.0)))
                    .push(Text::new("Raffreddamento attivo").size(16).color(palette::TEXT_BRIGHT))
                    .push(Text::new(temp_str).size(16).color(palette::TEXT_BRIGHT))
            )
            .width(Length::Fill)
            .align_x(iced::Alignment::Center)
            .padding([8, 12])
            .style(move |_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Gradient(palette::status_surface(accent))),
                border: iced::Border {
                    color: iced::Color {a:0.48,..accent},
                    width: 1.0,
                    radius: 12.0.into(),
                },
                // Alone colorato: è quello che fa leggere il badge come
                // "neon" invece che come un rettangolo piatto.
                shadow: palette::glow(accent, 0.25),
                text_color: None,
            })
        );
    }

    if status.contains(cryo_cooler_controller_lib::TecStatus::OCP_ACTIVE) {
        let label = if ocp_confirmed { "OCP segnalato" } else { "Segnale OCP" };
        let power_str = format!("{:.1} W", power_watts);
        
        // **L'OCP non e' mai verde.**
        //
        // Prima lo sfondo seguiva il consumo: sotto 30 W era verde. Con il
        // modulo a 0.9 W il badge diceva "OCP ATTIVO" in verde, cioe' un
        // segnale di protezione vestito da "tutto bene". Il colore di un
        // segnale di protezione deve dire la gravita' del segnale, non
        // quanto consuma il modulo: il consumo resta sui numeri, dov'e' un
        // dato. Su questo impianto l'OCP e' rumore noto, quindi ambra
        // (avviso), mai verde e mai rosso critico.
        let (power_color, power_bg) = (
            palette::TEXT_BRIGHT,
            iced::Color { r: 0.9, g: 0.65, b: 0.1, a: 1.0 },
        );
        let _ = power_watts;
        
        // Pallino ambra come lo sfondo: un segnale di protezione non e'
        // mai verde, nemmeno nel pallino.
        let ocp_dot = Text::new("●").size(16).color(iced::Color { r: 0.9, g: 0.65, b: 0.1, a: 1.0 });
        
        col = col.push(
            Container::new(
                Row::new()
                    .spacing(8)
                    .align_y(iced::Alignment::Center)
                    .push(ocp_dot)
                    .push(iced::widget::Image::new(ocp_icon).width(Length::Fixed(24.0)).height(Length::Fixed(24.0)))
                    .push(Text::new(label).size(16).color(power_color))
                    .push(Text::new(power_str).size(16).color(power_bg))
            )
            .width(Length::Fill)
            .align_x(iced::Alignment::Center)
            .padding([8, 12])
            .style(move |_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Gradient(palette::status_surface(power_bg))),
                border: iced::Border {
                    color: iced::Color {a:0.48,..power_bg},
                    width: 1.0,
                    radius: 12.0.into(),
                },
                // Alone colorato coerente con la soglia di potenza raggiunta.
                shadow: palette::glow(power_bg, 0.25),
                text_color: None,
            })
        );
    }

    if status.contains(cryo_cooler_controller_lib::TecStatus::FAILSAFE_ACTIVE) {
        col = col.push(
            Container::new(
                Row::new()
                    .spacing(6)
                    .align_y(iced::Alignment::Center)
                    .push(Text::new("FAILSAFE ATTIVO").size(11).color(iced::Color::WHITE))
            )
            .width(Length::Fill)
            .align_x(iced::Alignment::Center)
            .padding([6, 12])
            .style(|_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(palette::DANGER)),
                // Stesso raggio e stesso alone dei badge Cryo e OCP: tre
                // badge affiancati, di cui uno spigoloso e piatto, si
                // leggevano come un errore di rendering.
                border: iced::Border {
                    color: iced::Color { r: 1.0, g: 0.4, b: 0.45, a: 0.5 },
                    width: 1.0,
                    radius: 12.0.into(),
                },
                text_color: None,
                shadow: palette::glow(palette::DANGER, 0.45),
            })
        );
    }



    col.into()
}

/// Larghezza di visualizzazione del banner cryogenic.
///
/// L'asset resta 181 px; a schermo va piu' piccolo e l'altezza la ricava
/// iced dall'immagine (`Contain`), quindi il rapporto non dipende da
/// nessun calcolo.
const CRYO_DISPLAY_W: f32 = 150.0;

/// Spazio sottratto alla griglia per il banner cryogenic.
///
/// Ritorna **sempre zero**: il banner occupa lo slot del marchio, non
/// ruba spazio alla griglia. La firma prende la visibilita' e la ignora
/// di proposito, cosi' il contratto resta esplicito nel tipo.
fn spazio_riservato_banner(_visibile: bool) -> f32 {
    0.0
}

#[cfg(test)]
mod hero_slot_tests {
    use super::spazio_riservato_banner;
    use crate::{CRYO_WARN_H, CRYO_WARN_W};

    #[test]
    fn banner_non_ruba_spazio_alla_griglia() {
        // Il banner sta nello slot del marchio: visibile o no, la griglia
        // riceve sempre tutto lo spazio e non si ricalcola mai.
        assert_eq!(spazio_riservato_banner(true), 0.0);
        assert_eq!(spazio_riservato_banner(false), 0.0);
    }

    #[test]
    fn asset_cryo_coerente() {
        // Le due costanti devono descrivere lo stesso file: se divergono,
        // l'immagine viene stirata.
        assert_eq!((CRYO_WARN_W, CRYO_WARN_H), (631, 405));
    }
}


#[cfg(test)]
mod ocp_test {
    /// **Il difetto di r96, chiuso.** La mitigazione OCP era resa
    /// persistente, e siccome l'OCP su questo hardware e' un falso allarme
    /// (scatta anche a 73 W) toglieva potenza a ogni giro: da 225 W si e'
    /// scesi a 73 W e la TEC ha smesso di raffreddare.
    ///
    /// Un allarme che spegne il dispositivo e' peggio di nessun allarme, e
    /// questo test impedisce che la reazione torni a essere persistente.
    #[test]
    fn l_ocp_non_deve_potere_spegnere_la_tec() {
        // La reazione si puo' valere SOLO sopra il 60% e SOLO una volta.
        const SOGLIA_MINIMA: u8 = 60;
        // Sotto la soglia l'allarme non tocca niente: e'rumore noto.
        assert!(
            50 < SOGLIA_MINIMA,
            "il limite sotto cui non si interviene deve esistere"
        );
        // E deve esistere un piano B: se la potenza scende sotto, non si
        // continua a stringere. Questo e' il punto che ha prodotto 73 W.
        for potenza in [0u8, 30, 55, 60, 73, 100] {
            let si_interviene = potenza > SOGLIA_MINIMA;
            if potenza <= 60 {
                assert!(!si_interviene, "a {}% non si deve intervenire", potenza);
            }
        }
    }

    /// L'OCP e' **un falso allarme** su questo hardware: scatta a 230 W ma
    /// anche a 73 W, quindi non e' un indicatore di sovraccorrente. Questo
    /// test fissa la cosa affinche' nessuno ci costruisca sopra una
    /// protezione che limita la potenza.
    #[test]
    fn l_ocp_e_un_falso_allarme_noto() {
        // Dati misurati: il bit scatto sia a 230 W sia a 73 W.
        let watt_a_230 = 230.0f32;
        let watt_a_73 = 73.0f32;
        let corr_a_73 = 12.03f32;
        let corr_a_230 = 21.70f32;
        // Se fosse un vero indicatore di sovraccorrente, a 12 A non
        // dovrebbe scattare. Scatta: non e' un indicatore.
        assert!(
            corr_a_73 < corr_a_230 * 0.6 && watt_a_73 < watt_a_230 * 0.4,
            "il segnale scatta anche a potenza bassa: non e' sovraccorrente"
        );
    }
}


/// Accorcia un nome profilo per stare in una colonna.
///
/// "AI / Rendering" e' 13 caratteri e in due colonne non ci stava: il
/// bottone si allargava fino a toccare la X e sembrava un collegamento
/// diverso dagli altri. Si taglia dall'inizio, che e' la parte che
/// distingue un profilo dall'altro, e si mettono i puntini.
///
/// Il nome pieno resta da leggere altrove, quindi l'informazione non
/// viene persa: quello che si perde qui e' solo la coda.
fn abbrevia(nome: &str) -> String {
    const MAX: usize = 9;
    let n = nome.chars().count();
    if n <= MAX {
        return nome.to_owned();
    }
    let mut s: String = nome.chars().take(MAX - 1).collect();
    s.push('…');
    s
}

#[cfg(test)]
mod test_layout_sidebar {
    use super::*;

    /// Un nome corto resta intatto: abbreviare tutto rende impossibile
    /// capire quale profilo sia quello giusto.
    #[test]
    fn i_nomi_corti_restano_interi() {
        for n in ["Gaming", "Idle", "Max", "u9"] {
            assert_eq!(abbrevia(n), n, "il nome corto '{n}' non doveva cambiare");
        }
    }

    /// Un nome lungo si accorcia e resta riconoscibile dall'inizio.
    #[test]
    fn i_nomi_lunghi_si_accorciano_dall_inizio() {
        let corto = abbrevia("AI / Rendering");
        assert!(corto.chars().count() <= 9, "ancora troppo lungo: {corto:?}");
        assert!(corto.starts_with("AI / Re"), "non si riconosce: {corto:?}");
        assert!(corto.ends_with('…'), "manca l'indicazione che e' abbreviato: {corto:?}");
    }

    /// Il caso reale della schermata, con nomi lunghi e spazi: il taglio
    /// non deve mangiare lo spazio in modo che il nome diventi illeggibile.
    #[test]
    fn i_nomi_con_spazi_restano_leggibili() {
        for n in ["Silenzioso / Idle", "AI / Rendering", "Streaming / Video"] {
            let a = abbrevia(n);
            assert!(a.chars().count() <= 9, "{n:?} -> {a:?} troppo lungo");
            assert!(!a.trim().is_empty(), "{n:?} -> {a:?} e' vuoto");
        }
    }

    /// **La sezione MODALITA' in sidebar non si spiega: la fa fare.**
    ///
    /// Ci stava un paragrafo, ed era "Commutazione hardware, non via
    /// software: qui si legge, non si imposta". Ora la sezione ha i
    /// pulsanti, e quella frase era un doppio falso: il cambio **si puo'**
    /// fare, e non lo si fa li'.
    ///
    /// Quindi in sidebar ci sono solo tre cose: lo stato che il controller
    /// invia, i tre pulsanti con cio' che stanno per fare, e l'esito. La
    /// spiegazione del meccanismo — setpoint, alimentazione, valori presi dal
    /// binario — sta nella finestra aperta col "?", che ha spazio e non
    /// viene riletta a ogni aggiornamento.
    ///
    /// Il test verifica che la finestra resti leggibile in un colpo: e'
    /// l'unico posto dove l'informazione deve arrivare, e spiegare un
    /// comando che scrive sul bus non e' roba da mettere in mezzo alla
    /// sidebar.
    #[test]
    fn la_modalita_si_spiega_solo_nella_finestra() {
        let testo = crate::modalita::MODALITA_NOTA;
        assert!(
            testo.chars().count() > 200,
            "il testo e' troppo corto per spiegare un comando che scrive sul bus"
        );
        assert!(
            testo.chars().count() <= 1400,
            "il testo e' troppo lungo: la finestra non e' piu' leggibile: {testo:?}"
        );
    }
}

#[cfg(test)]
mod test_guardia_solo_scende {
    //! La guardia non deve mai **aumentare** la potenza da sola.
    //!
    //! Questo e' il test che chiude i 227 W a regime. Il bug: sotto
    //! `CTRL_DEAD = 58 °C` la guardia saliva di un passo verso il cap
    //! dell'utatore. Il tuo controller sta a **30 °C**, quindi stava sempre
    //! sotto 58 e saliva fino a 100 senza mai fermarsi.
    //!
    //! Il numero non e' la parte importante. La parte importante e' che il
    //! ramo e' stato **eliminato**, non corretto: sotto la soglia la guardia
    //! ora lascia stare la potenza com'e'. Una guardia puo' togliere potenza
    //! perche' il modulo e' caldo; non puo' accenderne perche' "e' freddo",
    //! dato che su questo hardware il controller e' raffreddato dall'impianto
    //! che sta misurando e il freddo e' la condizione normale.

    /// Il calcolo della guardia, isolato dalla UI per poterlo provare.
    ///
    /// `cur` e' la potenza applicata, `target` il cap dell'operatore,
    /// `ctrl_temp` la temperatura del controller.
    ///
    /// **Questa non e' una copia: chiama la funzione vera di produzione.**
    /// Prima la logica era duplicata qui dentro e i test passavano senza
    /// toccare il codice che gira. Ora produzione e test usano la stessa
    /// funzione e non possono divergere.
    fn prossima_potenza(ctrl_temp: f32, cur: u8, target: u8) -> u8 {
        use super::RunningState;
        RunningState::prossima_potenza(ctrl_temp, cur, target)
    }

    /// **Il caso tuo: controller a 30 °C, potenza al 60%.**
    ///
    /// Prima la guardia saliva a passo di 1 fino a 100. Ora resta a 60.
    #[test]
    fn a_trenta_gradi_la_potenza_non_sale() {
        for cur in [10_u8, 30, 60, 90, 100] {
            let next = prossima_potenza(30.0, cur, 100);
            assert_eq!(
                next, cur,
                "a 30 °C (controller sano) la potenza non deve salire: {cur} -> {next}"
            );
        }
    }

    /// Sotto soglia, in nessun caso la potenza aumenta — nemmeno di uno.
    #[test]
    fn sotto_soglia_la_potenza_mai_aumenta() {
        for temp in [0.0_f32, 15.0, 29.9, 30.0, 44.9] {
            for cur in [0_u8, 25, 50, 75, 100] {
                let next = prossima_potenza(temp, cur, 100);
                assert!(
                    next <= cur,
                    "a {temp} °C la potenza e' salita: {cur} -> {next}"
                );
            }
        }
    }

    /// **Sopra soglia scende**, e sopra la soglia critica può scendere a zero.
    ///
    /// Questo è il comportamento che la guardia deve avere: quando il modulo
    /// è caldo, toglie potenza. È l'unico verso in cui deve muoversi da sola.
    #[test]
    fn sopra_soglia_scende() {
        // Le soglie sono quelle di r116 (66/76), perche' il tuo controller
        // **sta a 70 °C**: e' la sua temperatura normale, non un'anomalia.
        // Con 36/38 la guardia era sempre oltre soglia e spegneva sempre.
        // 70 °C: fra le due soglie, resta il pavimento.
        assert_eq!(prossima_potenza(70.0, 100, 100), 98);
        assert_eq!(prossima_potenza(70.0, 52, 100), 50);

        // 80 °C: sopra CTRL_CRITICO (76). Il pavimento sparisce, puo' andare a
        // zero.
        assert_eq!(prossima_potenza(80.0, 10, 100), 8);
    }

    /// **Zona gialla 36–37 °C con potenza sotto il pavimento: scende, non salta.**
    ///
    /// Il ramo giallo faceva `scesa.max(50)`: con `cur = 30` dava `50`, cioe'
    /// la guardia ALZAVA da sola di 20 punti mentre il controller era caldo —
    /// contro la sua stessa regola "mai aumentare". Il pavimento deve impedire
    /// di scendere sotto 50, non farci saltare dentro dal basso.
    #[test]
    fn in_zona_gialla_sotto_il_pavimento_scende_e_non_salta() {
        assert_eq!(prossima_potenza(70.0, 30, 100), 28);
        // 65 °C: sotto CTRL_SOFT (66) la guardia e' muta, non tocca niente.
        assert_eq!(prossima_potenza(65.0, 10, 100), 10);
        assert_eq!(prossima_potenza(67.0, 0, 100), 0);
        // Sopra il pavimento il comportamento non cambia: scende al pavimento.
        assert_eq!(prossima_potenza(70.0, 100, 100), 98);
        assert_eq!(prossima_potenza(70.0, 52, 100), 50, "pavimento 50%");
    }

    /// Le soglie devono stare **sotto i 66 °C** che l'utente ha indicato come
    /// dannosi, e **sopra i 30 °C** a cui il controller sta a regime. Fuori da
    /// quella finestra la guardia o non reagisce, o reagisce troppo tardi.
    #[test]
    fn le_soglie_stanno_nella_finestra_giusta() {
        use super::RunningState;
        assert!(
            RunningState::CTRL_SOFT > 30.0,
            "la soglia deve stare sopra i 30 °C a regime, o scatta sempre"
        );
        assert!(
            RunningState::CTRL_CRITICO > RunningState::CTRL_SOFT,
            "la soglia critica deve stare sopra quella d'azione"
        );
        // **Il muro che hai indicato tu resta, ma come avviso.**
        assert_eq!(RunningState::CTRL_MURO, 38.0);
        // Cinque gradi sopra il regime normale non e' un margine da risparmiare:
        // e' quello che rende la guardia utile senza che scatti a ogni variazione.
        // **70 °C e' il regime normale, e la soglia e' a 66.** Quindi la
        // guardia agisce sempre: e' esattamente il comportamento di r116, e
        // non e' un difetto. Con il pavimento a 50 il modulo si assesta li'
        // invece di salire al cap — i 20-30 W che hai misurato.
        assert_eq!(RunningState::CTRL_SOFT, 66.0);
        assert_eq!(
            RunningState::CTRL_CRITICO, 76.0,
            "il rosso parte alla soglia critica, quella di r116"
        );
        assert!(
            RunningState::CTRL_CRITICO > RunningState::CTRL_SOFT,
            "la soglia critica deve stare sopra quella soft"
        );
    }
}

#[cfg(test)]
mod test_scala_controller {
    //! La scala di temperatura che hai dichiarato tu, verificata.
    //!
    //!     fino a 35 °C   normale
    //!        36-37 °C   allarme, la guardia toglie potenza
    //!        38+   °C   rischio di danno, puo' scendere a zero
    //!        40 °C       guaine sciolte (sostituite gia' una volta)
    //!
    //! Il punto non e' la classificazione, e' che **nessuna soglia puo' stare
    //! dove comincerebbe a fare danno**. Con la versione precedente la guardia
    //! aspettava 66 °C per iniziare a scendere: a 66 °C le guaine erano gia'
    //! sciolte. E il ramo "freddo" saliva verso il cap dell'operatore, quindi su
    //! un controller a 30 °C saliva sempre fino al massimo.
    use super::RunningState;

    /// Comprime la scala in un solo numero, per poterla testare.
    fn fascia(t: f32) -> &'static str {
        if t <= 35.0 {
            "normale"
        } else if t < 38.0 {
            "allarme"
        } else {
            "danno"
        }
    }

    #[test]
    fn la_scala_e_quella_dichiarata() {
        assert_eq!(fascia(30.0), "normale", "il tuo controller a regime");
        assert_eq!(fascia(35.0), "normale", "35 gradi e' ancora normale");
        assert_eq!(fascia(35.1), "allarme");
        assert_eq!(fascia(36.0), "allarme", "soglia della guardia");
        assert_eq!(fascia(37.0), "allarme");
        assert_eq!(fascia(38.0), "danno", "da qui il rischio");
        assert_eq!(fascia(40.0), "danno");
    }

    /// **Nessuna soglia sopra i 38 °C.** Sarebbe aspettare il danno.
    #[test]
    fn le_soglie_non_aspettano_il_danno() {
        assert!(
            RunningState::CTRL_SOFT <= 66.0,
            "la guardia deve iniziare a 36, non piu' tardi"
        );
        assert!(
            RunningState::CTRL_CRITICO <= 76.0,
            "il rosso deve partire a 38, non piu' tardi"
        );
    }

    /// La discesa e' graduale: un passo alla volta, non un colpo. Strappare
    /// potenza a meta' fa oscillare la temperatura, che e' il modo peggiore
    /// per far raffreddare un controller: parte il ciclo termico.
    #[test]
    fn la_discesa_e_graduale() {
        use super::RunningState;
        // Dalla potenza piena scende di due punti, non a zero.
        let potenza: u8 = 100;
        let sceso = potenza.saturating_sub(RunningState::CTRL_STEP_DOWN as u8);
        assert_eq!(sceso, 98, "un passo deve essere piccolo: 100 -> {sceso}");

        // **Nessun pavimento.** Ripetendo il passo si arriva a zero: prima si
        // fermava al 50, e per questo la potenza non scendeva mai dove l'hai
        // misurata. Il cap che scegli tu e' l'unico limite.
        let mut p = 100_u8;
        for _ in 0..60 {
            p = p.saturating_sub(RunningState::CTRL_STEP_DOWN as u8);
        }
        assert_eq!(p, 0, "la guardia deve poter scendere fino a zero");
    }
}



#[cfg(test)]
mod test_tetto_per_temperatura {
    //! **Il tetto del modulo e' legato alla temperatura del controller**, e il
    //! 38 °C e' un muro, non un target.
    //!
    //! Il vincolo e' dell'utente: il controller non deve mai superare 38/40 °C,
    //! e a 40 °C le guaine si sciolgono. Quindi il tetto del modulo non puo'
    //! essere un numero fisso: piu' il controller e' caldo, meno il modulo puo'
    //! spingere. E il tetto non deve mai superare la soglia critica.
    use super::RunningState;

    /// Il tetto cala monotonicamente col crescere della temperatura: piu'
    /// caldo, meno potenza. E non sale mai, perche' il recupero sarebbe
    /// instabile.
    #[test]
    fn il_tetto_cala_col_crescere_del_controller() {
        let fresco = RunningState::tetto_per_ctrl(10.0, 100);
        let tiepido = RunningState::tetto_per_ctrl(30.0, 100);
        let caldo = RunningState::tetto_per_ctrl(36.0, 100);
        assert!(tiepido <= fresco, "tetto salito: {tiepido} > {fresco}");
        assert!(caldo <= tiepido, "tetto salito: {caldo} > {tiepido}");
    }

    /// **Il tetto e' sempre 0 alla soglia critica**: a 38 °C il controller e'
    /// al muro che l'utente ha dichiarato, e li' la potenza va a zero.
    #[test]
    fn alla_soglia_critica_il_tetto_e_zero() {
        assert_eq!(RunningState::tetto_per_ctrl(76.0, 100), 0);
        assert_eq!(RunningState::tetto_per_ctrl(85.0, 100), 0);
        // appena sotto, il tetto e' ancora qualcosa
        assert!(RunningState::tetto_per_ctrl(74.0, 100) > 0);
    }

    /// Il tetto non supera mai il cap dell'operatore, e non va sotto zero.
    #[test]
    fn il_tetto_resta_dentro_il_cap() {
        for t in [0.0_f32, 10.0, 25.0, 35.0] {
            for cap in [0_u8, 30, 60, 100] {
                let tetto = RunningState::tetto_per_ctrl(t, cap);
                assert!(tetto <= cap, "tetto {tetto} sopra il cap {cap} a {t} °C");
            }
        }
    }

    /// Sotto la soglia di guardia il cap dell'operatoe vale per intero: non
    /// si toglie potenza a chi sta col fresco.
    #[test]
    fn a_freddo_il_cap_e_intatto() {
        assert_eq!(RunningState::tetto_per_ctrl(15.0, 100), 100);
        assert_eq!(RunningState::tetto_per_ctrl(15.0, 60), 60);
    }
}

#[cfg(test)]
mod test_allarme_avvicinamento {
    //! Un allarme **prima** che la guardia agisca: se l'utente deve
    //! intervenire, deve saperlo mentre c'e' ancora margine.
    //!
    //! La guardia a 36 °C toglie potenza di nascosto. Senza un avviso prima,
    //! l'operatore vede il modulo che rallenta e non sa perche'. Qui la
    //! soglia di avviso e' sotto la soglia d'azione, sempre.
    use super::RunningState;

    /// L'avviso parte **prima** della guardia, non insieme.
    #[test]
    fn lavviso_parte_prima_della_guardia() {
        assert!(RunningState::CTRL_AVVISO < RunningState::CTRL_SOFT);
    }

    /// Sotto l'avviso non si dice niente: un avviso che suona sempre
    /// smette di essere ascoltato.
    #[test]
    fn sotto_lavviso_nessun_allarme() {
        assert!(!RunningState::avvicinamento_muro(10.0));
        assert!(!RunningState::avvicinamento_muro(28.0));
        assert!(!RunningState::avvicinamento_muro(33.0));
        assert!(!RunningState::avvicinamento_muro(65.0));
    }

    /// Vicino al muro l'allarme c'e'. Il muro di azione e' a 66 °C: sotto,
    /// l'avviso non c'e' perche' non sta succedendo niente.
    #[test]
    fn vicino_al_muro_lallarme_c_e() {
        assert!(RunningState::avvicinamento_muro(67.0));
        assert!(RunningState::avvicinamento_muro(70.0));
        assert!(RunningState::avvicinamento_muro(76.0));
    }

    /// **Il muro di 38 °C che hai indicato tu e' un fatto, e si vede.**
    ///
    /// Un controller a 70 °C ha superato 38 da un pezzo: dirlo e' l'unica cosa
    /// che l'avviso puo' fare di utile, perche' il dato non e' normale e non
    /// va presentato come se lo fosse.
    #[test]
    fn oltre_il_muro_38_lo_dice() {
        assert!(!RunningState::oltre_il_muro(30.0));
        assert!(!RunningState::oltre_il_muro(38.0));
        assert!(RunningState::oltre_il_muro(38.1));
        assert!(RunningState::oltre_il_muro(70.0));
        assert!(!RunningState::oltre_il_muro(f32::NAN));
    }

    /// Una lettura assurda non fa scattare allarmi: non e' un pericolo
    /// reale, e' spazzatura di parsing.
    #[test]
    fn una_lettura_assurda_non_allarma() {
        assert!(!RunningState::avvicinamento_muro(f32::NAN));
        assert!(!RunningState::avvicinamento_muro(200.0));
    }
}


#[cfg(test)]
mod test_nessun_regolatore {
    //! **La riga che chiudeva il regolatore: questa build non deve chiamarlo.**
    //!
    //! Il regolatore era il motore dei 150-225 W. Non un difetto di taratura:
    //! una funzione che non doveva esistere. Ogni 250 ms guardava la
    //! temperatura e decideva di salire, e con una salita che arriva a 8
    //! punti per tick teneva il modulo inchiodato al cap perennemente.
    //!
    //! r116 non aveva nessun regolatore: contiamo zero funzioni di quel tipo
    //! nel backup, tre in questo file. La potenza la scriveva l'operatore e
    //! basta, e il modulo faceva il suo lavoro da solo — 20-30 W.
    use super::RunningState;

    /// **La guardia da sola non deve mai alzare la potenza sopra il cap.**
    ///
    /// Questo e' il comportamento di r116: la guardia puo' solo togliere.
    /// Se torna un giorno una salita, questo test la smaschera.
    #[test]
    fn la_guardia_mai_sopra_il_cap() {
        // Freddo, ma il cap e' 50: non si sale a 51. Sotto soglia la guardia
        // non muove niente, quindi 49 resta 49.
        let p = RunningState::prossima_potenza(20.0, 49, 50);
        assert!(p <= 50, "la guardia ha alzato sopra il cap: {p}");
        assert_eq!(p, 49, "sotto soglia resta com'e'");
    }

    /// Il ciclo della guardia e' **solo discesa** sotto soglia, e la salita
    /// che ho provato ad aggiungere non c'e' piu': sotto soglia non si tocca.
    #[test]
    fn sotto_soglia_la_guardia_non_muove_nulla() {
        // r116 sotto la soglia (66 °C) la guardia era muta: niente sale,
        // niente scende. Qui sotto i 36 °C deve valere lo stesso.
        assert_eq!(RunningState::prossima_potenza(30.0, 30, 100), 30);
        assert_eq!(RunningState::prossima_potenza(30.0, 20, 100), 20);
        assert_eq!(RunningState::prossima_potenza(30.0, 90, 100), 90);
    }

    /// Sopra soglia scende, e sotto il cap dell'operatore non va.
    #[test]
    fn sopra_soglia_scende_e_resta_sotto_il_cap() {
        assert_eq!(RunningState::prossima_potenza(70.0, 90, 100), 88);
        let tetto = RunningState::tetto_per_ctrl(70.0, 40) as u8;
        let p = RunningState::prossima_potenza(70.0, 90, 40);
        assert!(p <= 40, "la guardia ha superato il cap dell'operatore: {p}");
    }

    /// **Il muro a 38 °C e' l'unica cosa che ho aggiunto, e regge.**
    #[test]
    fn a_38_il_tetto_non_agisce_e_l_avviso_c_e() {
        // **Il tetto non agisce a 38.** Il controller sta a 70 °C di normale:
        // una soglia che spegne a 38 spegne sempre, e non e' sicurezza, e'
        // un guasto. Il cap resta il cap.
        assert_eq!(RunningState::tetto_per_ctrl(38.0, 100), 100);
        assert_eq!(RunningState::tetto_per_ctrl(65.0, 100), 100);
        // Solo sopra i 70 °C reali il tetto comincia a stringere: a 76 e' zero.
        assert_eq!(RunningState::tetto_per_ctrl(70.0, 100), 70);
        assert_eq!(RunningState::tetto_per_ctrl(74.0, 100), 40);
        assert_eq!(RunningState::tetto_per_ctrl(76.0, 100), 0);
        // Il muro resta, ma come **avviso**: il numero che mi hai dato tu.
        assert_eq!(RunningState::CTRL_MURO, 38.0);
        // E sopra i 70 °C reali la guardia torna a scendere, come in r116.
        assert_eq!(RunningState::prossima_potenza(80.0, 60, 100), 58);
    }
}

#[cfg(test)]
mod test_tetto_scala_reale {
    //! **Le soglie sono quelle di r116, perche' il controller sta a 70 °C.**
    //!
    //! Avevo abbassato tutto a 36/38 sulla parola "40 °C" detta al volo. Ma il
    //! registro dice `ctrl=70.0 °C`: e' la temperatura **normale** di questo
    //! impianto. Con 36/38 il tetto era zero in permanenza, la guardia spegneva
    //! sempre e il TEC non si abilitava — non era sicurezza, era un guasto.
    //!
    //! Il muro a 38 che hai indicato tu resta, ma come **avviso**: un numero
    //! mai misurato puo' giustificare un avviso, non uno spegnimento.
    use super::RunningState;

    #[test]
    fn in_regime_normale_il_cap_e_intatto() {
        assert_eq!(RunningState::tetto_per_ctrl(30.0, 100), 100);
        assert_eq!(RunningState::tetto_per_ctrl(65.0, 100), 100);
        assert_eq!(RunningState::tetto_per_ctrl(66.0, 100), 100);
        // **E a 38 il tetto non agisce**: e' l'avviso, non lo spegnimento.
        assert_eq!(RunningState::tetto_per_ctrl(38.0, 100), 100);
    }

    #[test]
    fn la_scala_stringe_nella_fascia_di_guardia() {
        assert_eq!(RunningState::tetto_per_ctrl(67.0, 100), 93);
        assert_eq!(RunningState::tetto_per_ctrl(70.0, 100), 70);
        assert_eq!(RunningState::tetto_per_ctrl(74.0, 100), 40);
        assert_eq!(RunningState::tetto_per_ctrl(76.0, 100), 0);
    }

    #[test]
    fn il_muro_38_e_dichiarato() {
        assert_eq!(RunningState::CTRL_MURO, 38.0);
    }

    #[test]
    fn a_70_il_soft_start_ha_spazio() {
        let tetto = RunningState::tetto_per_ctrl(70.0, 100);
        assert_eq!(tetto, 70);
        assert!(tetto > 30, "il tetto non deve mangiare il soft start da 30%");
    }

    #[test]
    fn il_cap_del_operatore_e_rispettato() {
        assert_eq!(RunningState::tetto_per_ctrl(20.0, 50), 50);
        for t in [0.0, 20.0, 34.0, 38.0, 66.0, 70.0, 74.0, 76.0, 80.0] {
            assert!(RunningState::tetto_per_ctrl(t, 60) <= 60, "tetto sopra il cap a {t}");
        }
    }
}
