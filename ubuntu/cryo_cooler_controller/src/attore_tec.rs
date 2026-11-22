//! Thread proprietario della porta seriale: legge e scrive, la UI non attende.
//!
//! # Perche' un thread, e cosa non puo' fare
//!
//! Il porting seriale e' bloccante (timeout 150 ms a comando, 7-8 comandi per
//! `monitor()`). Se gira sul thread della UI blocca il ridisegno: da li' gli
//! scatti. Serve quindi un thread dedicato.
//!
//! Ma un thread che *anche* scrive ha un pericolo reale, gia' osservato su
//! hw: mettendo letture e scritture nella stessa coda FIFO, un `Enable`
//! premuto dall'utente restava in coda dietro i campioni e il TEC non si
//! abilitava mai. La correzione non e' "togliere il thread", e' **dare la
//! precedenza alle scritture**.
//!
//! # Il ciclo, in una regola
//!
//! > Prima le scritture, poi le letture.
//!
//! Il ciclo controlla se c'e' una scrittura in attesa *prima* di aprire un
//! round di lettura, e non apre una seconda lettura mentre la prima non ha
//! finito. Ogni scrittura riceve un `Ack` con l'esito reale: la UI non
//! dichiara mai "abilitato" o "potenza applicata" senza quel SIGNO.
//!
//! La logica di sicurezza (guardia, soft start, OCP, anticondensa) resta
//! tutta nel tick della UI: il thread non decide nulla, esegue e risponde.

use cryo_cooler_controller_lib::tecstatus::StatusCompleto;
use cryo_cooler_controller_lib::MonitoringData;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

/// Cosa chiede la UI al thread proprietario.
pub enum Richiesta {
    /// Un round di lettura: `monitor()` + `hear_beat()`.
    Campione,
    // ── Rimosse `Enable` e `Disable` ───────────────────────────────────
    //
    // `Enable` portava `potenza` e `setpoint` propri: poteva quindi scrivere
    // un offset **diverso da quello del regime attivo**, e lo faceva dal
    // pulsante "ABILITA TEC". Con il regime come unica sorgente, quel pulsante
    // non ha piu' niente da dire: due strade che scrivono lo stesso controller
    // sono due strade che divergono.
    //
    // `Disable` non e' sparito per meta': e' diventato `Richiesta::spegnimento`,
    // che e' il piano di `Regime::Spento`. Copre anche lo spegnimento all'uscita,
    // che ora passa da `applica_regime` e quindi **rilascia lo stato** — prima
    // l'ultimo comando dell'app non sapeva se fosse andato.
    /// Scrivi la potenza. Risponde `Ack` con l'esito reale della scrittura.
    Potenza(u8),
    Setpoint(f32),
    Pid { p: f32, i: f32, d: f32 },
    TempCpu(f32),
    /// Commuta regime: scrive l'offset, poi accende o spegne il TEC, poi
    /// **rilascia lo stato** perche' la UI possa verificare.
    ///
    /// Il thread non decide nulla: esegue. Il `Regime` che riceve arriva gia'
    /// calcolato da `commutazione::Regime::piano`, e i numeri che porta sono
    /// presi dal binario del produttore.
    Regime {
        /// `None` = non scrivere il setpoint. Serve allo spegnimento, che
        /// deve toccare solo l'alimentazione.
        offset: Option<f32>,
        tec_acceso: bool,
        /// `(P, I, D)` dell'operatore. Vanno scritti **durante**
        /// l'abilitazione: sono cio' che dà al firmware un regolatore da
        /// usare. Senza, il modulo resta al livello di default.
        pid: Option<(f32, f32, f32)>,
        /// Il modulo era gia' acceso: il firmware ignora un enable in quel
        /// caso, quindi serve un disable prima (Unregulated → Cryo).
        prima_spegni: bool,
    },
}

impl Richiesta {
    /// `true` se la richiesta **scrive** sul TEC: quelle precedono le letture.
    /// Lo spegnimento: identico al piano di `Regime::Spento`.
    ///
    /// Un nome invece di `{ offset: None, tec_acceso: false }` ripetito in due
    /// punti: il chiamante non deve poter scrivere `tec_acceso: true` per
    /// sbaglio in un posto che dice "spengo".
    pub fn spegnimento() -> Self {
        Richiesta::Regime { offset: None, tec_acceso: false, pid: None, prima_spegni: false }
    }

    /// L'offset che questa richiesta scrive, se lo scrive.
    ///
    /// Esiste per i test: e' il modo in cui si verifica che lo spegnimento non
    /// scriva un setpoint. Fuori dai test non serve a nessuno, e dichiararlo
    /// `#[cfg(test)]` evita che un avviso "mai usato" di un domani venga
    /// scambiato per un forgotten refactor.
    #[cfg(test)]
    pub fn offset(&self) -> Option<f32> {
        match self {
            Richiesta::Regime { offset, .. } => *offset,
            Richiesta::Setpoint(v) => Some(*v),
            _ => None,
        }
    }

    /// Se questa richiesta accende o spegne il TEC. Solo per i test.
    #[cfg(test)]
    pub fn tec_acceso(&self) -> Option<bool> {
        match self {
            Richiesta::Regime { tec_acceso, .. } => Some(*tec_acceso),
            _ => None,
        }
    }

    fn e_scrittura(&self) -> bool {
        !matches!(self, Richiesta::Campione)
    }
}

/// Cosa risponde il thread. `Campione` e' il dato per i grafici, `Ack` e'
/// l'esito onesto di una scrittura.
///
/// L'`Ack` porta **cosa** e' stato scritto: senza questo la UI non puo'
/// distinguere "abilitato" da "potenza aggiornata" e deve indovinare.
pub enum Risposta {
    /// `(MonitoringData, StatusCompleto)`.
    ///
    /// Trasporta lo **stato completo**, non solo i 18 bit noti: i 14 bit
    /// alternativi servono a correlare l'OCP con i codici errore del
    /// manuale, e qui era il punto in cui andavano persi.
    Campione(Result<(MonitoringData, StatusCompleto), String>),
    /// `Ok(quello)` = il firmware ha confermato quell'azione.
    Ack(Result<Scritto, String>),
    ErroreRegime {offset:Option<f32>,error:String},
}

/// L'azione che il firmware ha confermato.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scritto {
    Potenza(u8),
    Setpoint(f32),
    Pid,
    TempCpu(f32),
    /// La commutazione e' stata eseguita e il controller ha risposto. Il
    /// valore e' lo **stato rilettato dopo** la scrittura: la UI verifica
    /// qui che il regime sia quello richiesto invece di darlo per scontato.
    Regime(StatusCompleto, Option<f32>),
    Spegnimento(StatusCompleto),
}

impl Scritto {
    pub fn matches_mode(&self,enabled:bool,offset:Option<f32>)->bool {
        match self {
            Self::Spegnimento(_)=>!enabled,
            Self::Regime(_,actual)=>enabled && match (offset,*actual) {
                (Some(wanted),Some(actual))=>(wanted-actual).abs()<=0.05,
                (None,None)=>true,
                _=>false,
            },
            _=>false,
        }
    }
}

fn accoda(coda:&mut std::collections::VecDeque<Richiesta>,r:Richiesta) {
    if matches!(r,Richiesta::Regime {..}) {
        // A new mode supersedes pending mode/power/offset updates.
        coda.retain(|old|!matches!(old,Richiesta::Regime {..}|Richiesta::Setpoint(_)|Richiesta::Potenza(_)));
    } else {
        let kind=std::mem::discriminant(&r);
        coda.retain(|old|std::mem::discriminant(old)!=kind);
    }
    coda.push_back(r);
}

/// Esegue una richiesta sul TEC. Separata dal ciclo perche' la stessa logica
/// viene usata dai test, senza hardware.
fn esegui(tec: &mut cryo_cooler_controller_lib::Tec, r: Richiesta) -> Risposta {
    match r {
        Richiesta::Campione => {
            let m = tec.monitor().map_err(|e| e.to_string());
            let b = tec.hear_beat_completo().map_err(|e| e.to_string());
            match (m, b) {
                (Ok(md), Ok(st)) => Risposta::Campione(Ok((md, st))),
                (Err(e), _) | (_, Err(e)) => Risposta::Campione(Err(e)),
            }
        }
        Richiesta::Potenza(v) => {
            Risposta::Ack(tec.set_power_level(v)
                .map(|()| Scritto::Potenza(v))
                .map_err(|e| e.to_string()))
        }
        Richiesta::Setpoint(v) => {
            Risposta::Ack(tec.set_setpoint_offset(v)
                .map(|()| Scritto::Setpoint(v))
                .map_err(|e| e.to_string()))
        }
        Richiesta::Pid { p, i, d } => {
            Risposta::Ack(tec.set_pid(p, i, d).map(|()| Scritto::Pid).map_err(|e| e.to_string()))
        }
        Richiesta::TempCpu(v) => {
            Risposta::Ack(tec.set_cpu_temp(v)
                .map(|()| Scritto::TempCpu(v))
                .map_err(|e| e.to_string()))
        }
        Richiesta::Regime { offset, tec_acceso, pid, prima_spegni } => {
            if tec_acceso {
                match tec.hear_beat_completo() {
                    Ok(st)=>crate::commissioning::event("ENABLE-PREFLIGHT",&format!(
                        "raw=0x{:08X} BOARD_INIT={} PID_RUNNING={} POWER_OK={}",st.noti.bits() | ((st.bit_alternativi as u32)<<18),
                        st.noti.contains(cryo_cooler_controller_lib::TecStatus::BOARD_INIT),
                        st.noti.contains(cryo_cooler_controller_lib::TecStatus::PID_RUNNING),
                        st.noti.contains(cryo_cooler_controller_lib::TecStatus::POWER_OK))),
                    Err(e)=>return Risposta::ErroreRegime {offset,error:format!("Heartbeat pre-abilitazione fallito: {e}")},
                }
                match tec.board_temperature() {
                    Ok(board) if cryo_cooler_controller_lib::board_allows_enable(board)=>{},
                    result=> {
                        let disabled=tec.disable();
                        return Risposta::ErroreRegime {offset,error:format!("Abilitazione bloccata: temperatura PCB non sicura {result:?}; disable={disabled:?}")};
                    }
                }
            }
            crate::commissioning::event("SERIALE-REGIME", &format!("acceso={tec_acceso} offset={offset:?} disable_prima={prima_spegni}"));
            // `applica_regime_con_pid` scrive offset, **PID**, enable e potenza
            // in quest'ordine: e' la sequenza dell'originale, e senza i
            // coefficienti il firmware non regola e resta al 10% di default.
            // L'errore arriva come stringa perche' la UI lo mostra
            // all'operatore: un `io::Error` grezzo ("invalid data") non dice
            // nulla su cosa sia successo al controller.
            let (p, i, d) = pid.unwrap_or((100.0, 1.0, 0.0));
            match tec.applica_regime_completo(offset, tec_acceso, p, i, d, prima_spegni) {
                Ok(st) => {
                    if tec_acceso {
                        if let Some(expected)=offset {
                            match tec.setpoint_offset() {
                                Ok(read) if (read-expected).abs()<=0.05 => {
                                    crate::commissioning::event("OFFSET-VERIFICATO",&format!("richiesto={expected:.2} letto={read:.2}"));
                                }
                                other => {
                                    let error=format!("Offset non verificato: richiesto {expected:.2}, risposta {other:?}");
                                    let disabled=tec.disable();
                                    return Risposta::ErroreRegime {offset,error:format!("{error}; disable={disabled:?}")};
                                }
                            }
                        }
                    }
                    Risposta::Ack(Ok(if tec_acceso { Scritto::Regime(st,offset) } else { Scritto::Spegnimento(st) }))
                },
                Err(e) => Risposta::ErroreRegime {offset,error:e.to_string()},
            }
        }
    }
}

/// Ritorna la prossima richiesta da eseguire, **scritture prima**.
///
/// Logica separata dal ciclo cosi' e' testabile senza hardware: e' la regola
/// che decide l'abilitazione, quindi non puo' stare implicita in un `if`.
fn prossima_richiesta(
    coda: &mut std::collections::VecDeque<Richiesta>,
    attesa: &Receiver<Richiesta>,
) -> Option<Richiesta> {
    // Stop before other writes, then mode changes before telemetry updates.
    if let Some(i)=coda.iter().position(|r| matches!(r,Richiesta::Regime {tec_acceso:false,..})) {return coda.remove(i);}
    if let Some(i)=coda.iter().position(|r| matches!(r,Richiesta::Regime {..})) {return coda.remove(i);}
    // 1) Una scrittura gia' in coda ha la precedenza assoluta.
    if let Some(i) = coda.iter().position(|r| r.e_scrittura()) {
        return coda.remove(i);
    }
    // 2) Poi un eventuale campione pendente.
    if let Some(r) = coda.pop_front() {
        return Some(r);
    }
    // 3) Coda vuota: si dorme in attesa di qualcosa di nuovo.
    attesa.recv().ok()
}

/// Ciclo del thread proprietario.
///
/// Termina quando la UI chiude il canale delle richieste.
pub fn ciclo_attore(
    mut tec: cryo_cooler_controller_lib::Tec,
    richieste: Receiver<Richiesta>,
    risposte: Sender<Risposta>,
) {
    // Riconcilia la coda locale con quella del canale. `recv` e' l'unico
    // punto che dorme, e si raggiunge solo a coda vuota: quindi il ciclo non
    // parte mai un round di lettura se ha gia' una scrittura da fare.
    let mut coda: std::collections::VecDeque<Richiesta> = std::collections::VecDeque::new();
    loop {
        // Non bloccare se c'e' roba da fare: si svuota il canale in non
        // bloccante e si lavora dalla coda locale, che sa distinguere
        // letture da scritture.
        match richieste.try_recv() {
            Ok(r) => {
                accoda(&mut coda,r);
                // Drena tutto il disponibile, cosi' la prossima scrittura in
                // coda viene vista subito e non dopo un round di lettura.
                while let Ok(r2) = richieste.try_recv() {
                    accoda(&mut coda,r2);
                }
            }
            Err(TryRecvError::Disconnected) if coda.is_empty() => break,
            Err(TryRecvError::Disconnected) => {},
            Err(TryRecvError::Empty) => {}
        }

        let r = match prossima_richiesta(&mut coda, &richieste) {
            Some(r) => r,
            // Canale chiuso e coda svuotata: la UI e' andata via.
            None => break,
        };

        if risposte.send(esegui(&mut tec, r)).is_err() {
            break;
        }
    }
    // La UI puo' sparire senza consumare l'ACK finale. La porta e' ancora
    // nostra: tentare disable prima di rilasciarla evita un TEC orfano.
    if let Err(e) = tec.disable() {
        crate::commissioning::event("ATTORE-STOP", &format!("disable NON confermato: {e}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::mpsc;
    #[test] fn rapid_updates_stay_bounded_and_stop_precedes_telemetry() {
        let mut q=VecDeque::new();
        for n in 0..1000 {accoda(&mut q,Richiesta::TempCpu(n as f32));accoda(&mut q,Richiesta::Setpoint(2.0));accoda(&mut q,Richiesta::Campione);}
        assert_eq!(q.len(),3);
        accoda(&mut q,Richiesta::spegnimento());
        let (_tx,rx)=mpsc::channel();
        assert!(matches!(prossima_richiesta(&mut q,&rx),Some(Richiesta::Regime {tec_acceso:false,..})));
        assert!(!q.iter().any(|r|matches!(r,Richiesta::Setpoint(_))));
    }
    #[test] fn cryo_ack_cannot_confirm_a_later_unregulated_request() {
        let st=StatusCompleto {noti:cryo_cooler_controller_lib::TecStatus::POWER_OK,bit_alternativi:0};
        let cryo=Scritto::Regime(st,Some(2.0));
        assert!(cryo.matches_mode(true,Some(2.0)));
        assert!(!cryo.matches_mode(true,Some(-30.0)));
        assert!(!cryo.matches_mode(false,None));
    }

    fn canale() -> (Sender<Richiesta>, Receiver<Richiesta>) {
        mpsc::channel()
    }

    #[test]
    fn campione_e_ack_distinguibili() {
        let r: Risposta = Risposta::Ack(Ok(Scritto::Pid));
        assert!(matches!(r, Risposta::Ack(Ok(Scritto::Pid))));
        let r: Risposta = Risposta::Campione(Err("x".to_owned()));
        assert!(matches!(r, Risposta::Campione(_)));
    }

    /// **Il test che copre il blackout.** Una scrittura accodata dietro un
    /// campione deve uscire per prima: e' il caso che in hardware non
    /// abilitava mai il TEC. Da quando `Enable` e' diventato `Regime`, e' la
    /// commutazione a doverlo dimostrare.
    #[test]
    fn la_scrittura_precede_il_campione() {
        let (tx, rx) = std::sync::mpsc::channel();
        let _tx = tx;
        let mut coda: VecDeque<Richiesta> = VecDeque::new();
        coda.push_back(Richiesta::Campione);
        coda.push_back(Richiesta::Regime {
            offset: Some(0.0),
            tec_acceso: true,
            pid: Some((100.0, 1.0, 0.0)),
            prima_spegni: false,
        });
        let prima = prossima_richiesta(&mut coda, &rx).expect("deve restituire qualcosa");
        assert!(
            prima.e_scrittura(),
            "la scrittura deve precedere il campione"
        );
    }

    /// Un campione non viene perso: se non c'e' scrittura, esce il campione.
    #[test]
    fn il_campione_esce_se_non_ci_sono_scritture() {
        let (tx, rx) = std::sync::mpsc::channel();
        let _tx = tx;
        let mut coda: VecDeque<Richiesta> = VecDeque::new();
        coda.push_back(Richiesta::Campione);
        let r = prossima_richiesta(&mut coda, &rx).expect("deve restituire il campione");
        assert!(!r.e_scrittura());
    }

    /// Con piu' scritture in coda, escono tutte prime, e il campione
    /// resta per ultimo. Il campione non viene perso: e' lui l'ultimo.
    #[test]
    fn tutte_le_scritture_prima_del_campione() {
        let (tx, rx) = std::sync::mpsc::channel();
        let _tx = tx;
        let mut coda: VecDeque<Richiesta> = VecDeque::new();
        coda.push_back(Richiesta::Campione);
        coda.push_back(Richiesta::Potenza(40));
        coda.push_back(Richiesta::spegnimento());
        let a = prossima_richiesta(&mut coda, &rx).unwrap();
        let b = prossima_richiesta(&mut coda, &rx).unwrap();
        let c = prossima_richiesta(&mut coda, &rx).unwrap();
        assert!(a.e_scrittura(), "prima deve essere una scrittura");
        assert!(b.e_scrittura(), "seconda deve essere una scrittura");
        assert!(!c.e_scrittura(), "il campione esce per ultimo");
        assert!(coda.is_empty(), "la coda e' stata svuotata");
    }
}

#[cfg(test)]
mod test_spegnimento {
    use super::{Richiesta, Scritto};
    use crate::commutazione::Regime;
    use std::collections::VecDeque;

    use super::prossima_richiesta;

    /// **Lo spegnimento e' il regime `Spento`, non un comando a parte.**
    ///
    /// Prima l'uscita dall'app mandava `Richiesta::Disable`, che chiamava
    /// `Tec::disable()` e basta: stessa opcode `0x18`, ma un percorso diverso da
    /// `applica_regime`, quindi **senza la rilettura finale**. Cioe': l'ultimo
    /// comando dell'app non sapeva se fosse andato, e non poteva dirlo.
    ///
    /// Ora lo spegnimento finale passa da `applica_regime(None, false)`, che e'
    /// esattamente il piano di `Spento`: un solo percorso di scrittura sul
    /// controller, e l'ack di uscita porta lo stato letto.
    #[test]
    fn lo_spegnimento_e_il_piano_di_spento() {
        let piano = Regime::Spento.piano(-12.0);
        let r = Richiesta::spegnimento();
        assert_eq!(r.offset(), piano.offset, "stesso setpoint: nessuno dei due");
        assert_eq!(r.tec_acceso(), Some(piano.tec_acceso));
        assert_eq!(r.tec_acceso(), Some(false), "lo spegnimento spegne");
    }

    /// Lo spegnimento **non scrive il setpoint**. Un byte in piu' su questo bus
    /// puo' essere il reset di fabbrica, e un modulo spento non applica comunque
    /// un setpoint: le impostazioni dell'operatore devono restare.
    #[test]
    fn lo_spegnimento_non_scrive_il_setpoint() {
        assert_eq!(
            Richiesta::spegnimento().offset(),
            None,
            "lo spegnimento non deve toccare il setpoint"
        );
    }

    /// **Il blackout hardware, sul percorso che resta.**
    ///
    /// Il test originale verificava la priorita' con `Richiesta::Enable`, che
    /// questa fase elimina. L'invariante pero' e' reale e va conservato: una
    /// scrittura accodata dietro un campione non usciva mai, e in hardware questo
    /// significava che il TEC non veniva mai abilitato.
    ///
    /// Nota onesta: questo test **passa gia'** senza modifiche, perche'
    /// `e_scrittura()` e' `!matches!(self, Campione)` e `Regime` non e' un
    /// campione. Non e' una prova di una correzione, e' la verifica che
    /// l'invariante sopravviva al cambio di variante.
    #[test]
    fn la_commutazione_precede_il_campione() {
        let (tx, rx) = std::sync::mpsc::channel();
        let _tx = tx;
        let mut coda: VecDeque<Richiesta> = VecDeque::new();
        coda.push_back(Richiesta::Campione);
        coda.push_back(Richiesta::Regime { offset: Some(-12.0), tec_acceso: true, pid: None, prima_spegni: false });
        let prima = prossima_richiesta(&mut coda, &rx).expect("deve restituire qualcosa");
        assert!(prima.e_scrittura(), "la scrittura deve precedere il campione");
    }

    /// Anche lo spegnimento e' una scrittura, e deve uscire prima di un
    /// campione: e' l'ultimo comando dell'app, e non può restare in coda.
    #[test]
    fn anche_lo_spegnimento_precede_il_campione() {
        let (tx, rx) = std::sync::mpsc::channel();
        let _tx = tx;
        let mut coda: VecDeque<Richiesta> = VecDeque::new();
        coda.push_back(Richiesta::Campione);
        coda.push_back(Richiesta::spegnimento());
        let prima = prossima_richiesta(&mut coda, &rx).expect("deve restituire qualcosa");
        assert!(prima.e_scrittura(), "lo spegnimento non puo' restare in coda");
    }

    /// L'ack dello spegnimento porta lo stato letto, come ogni altra
    /// commutazione: e' cio' che permette all'uscita di sapere se il comando
    /// e' andato, invece di sperarlo.
    #[test]
    fn l_ack_dello_spegnimento_porta_lo_stato() {
        let s = Scritto::Regime(
            cryo_cooler_controller_lib::tecstatus::StatusCompleto {
                noti: cryo_cooler_controller_lib::TecStatus::POWER_OK,
                bit_alternativi: 0,
            }, Some(2.0),
        );
        assert!(
            matches!(s, Scritto::Regime(_, _)),
            "l'ack deve portare lo stato, non un semplice 'fatto'"
        );
    }
}
