#![forbid(unsafe_code)]
#![warn(
    clippy::dbg_macro,
    clippy::decimal_literal_representation,
    clippy::panic,
    clippy::panic_in_result_fn,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::todo,
    clippy::unimplemented,
    clippy::unwrap_used,
    clippy::use_debug
)]
// Vedi la nota in cryo_cooler_controller/src/main.rs: nei test `unwrap` e
// `eprintln!` verificano un risultato, in produzione restano segnalati.
#![cfg_attr(
    test,
    allow(
        clippy::assertions_on_constants,
        clippy::panic,
        clippy::print_stderr,
        clippy::unwrap_used
    )
)]

use chrono::Utc;
use serial::SerialPort;
use std::{
    convert::TryInto,
    io::{Read, Write},
};

pub mod tecstatus;

const CRC_16_XMODEM: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM);

/// Gli opcode che `probe_opcodes` e' autorizzato a inviare.
///
/// Il nome del modulo `commands::set` dice la verita' sul pericolo: `get::*`
/// sono letture, `set::*` sono scritture. I due gruppi **non** sono
/// disgiunti per numero — 0x14..=0x17 compaiono in `set` (scritture), non in
/// `get`. Percio' l'allowlist qui sotto e' scritta a partire dai valori
/// leggibili nella tabella dei comandi, non da una supposizione sul layout.
const SOLO_LETTURE: &[std::ops::RangeInclusive<u8>] = &[
    // 0x00 HEART_BEAT .. 0x0A FW_VERSION: blocco di letture, tutto `get`.
    commands::HEART_BEAT..=commands::get::FW_VERSION,
    // 0x1B NTC_COEFFICIENT, 0x1F BOARD_TEMP: letture sparse oltre il blocco.
    commands::get::NTC_COEFFICIENT..=commands::get::NTC_COEFFICIENT,
    commands::get::BOARD_TEMP..=commands::get::BOARD_TEMP,
    // 0x22 VOLTAGE_AND_CURRENT .. 0x24 TEC_CURRENT.
    commands::get::VOLTAGE_AND_CURRENT..=commands::get::TEC_CURRENT,
];

/// `true` se l'opcode e' una lettura autorizzata dalla sonda.
fn is_solo_lettura(op: u8) -> bool {
    SOLO_LETTURE.iter().any(|r| r.contains(&op))
}

/// Gli opcode che `probe_opcodes(da, a)` manderebbe davvero, in ordine.
///
/// Separato dal ciclo seriale perche' e' la sola parte con una garanzia di
/// sicurezza: tutto quello che non passa di qui non viene mai scritto sul
/// bus, indipendentemente dagli argomenti ricevuti dal chiamante.
fn probe_plan(da: u8, a: u8) -> Vec<u8> {
    (da..=a).filter(|op| is_solo_lettura(*op)).collect()
}

pub struct Tec {
    port: serial::SystemPort,
    /// Timeout di lavoro, salvato perche' `drain_input` ne abbassa il valore
    /// temporaneamente e deve poterlo ripristinare.
    work_timeout: std::time::Duration,
}

/// Timeout per un singolo round-trip seriale in condizioni normali.
///
/// 150 ms e' ampio per un riscontro a 115200 baud, ma non tanto da
/// congelare la UI: `monitor()` esegue 8 round-trip consecutivi sul thread
/// UI, quindi 150 ms x 8 = 1,2 s al massimo per tick.
const WORK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);
// User's modified Gen 1 controller: this is a configured installation limit,
// not a manufacturer temperature rating for other controllers.
pub const CRITICAL_BOARD_TEMP:f32=38.0;
pub const BOARD_REENABLE_TEMP:f32=37.0;
pub fn board_allows_enable(board:f32)->bool {
    board.is_finite() && (-40.0..BOARD_REENABLE_TEMP).contains(&board)
}

fn validate_finite(value: f32, name: &str) -> Result<(), std::io::Error> {
    if value.is_finite() { Ok(()) } else {
        Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{name} must be finite")))
    }
}

fn validate_pid(p: f32, i: f32, d: f32) -> Result<(), std::io::Error> {
    for value in [p, i, d] {
        validate_finite(value, "PID")?;
        if !(0.0..=1000.0).contains(&value) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "PID outside dashboard range 0..1000"));
        }
    }
    Ok(())
}

fn validate_offset(offset:f32)->Result<(),std::io::Error> {
    validate_finite(offset,"setpoint")?;
    if (-30.0..=50.0).contains(&offset) {Ok(())} else {
        Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,"Setpoint outside dashboard range -30..50 C"))
    }
}

fn validate_power(power: u8) -> Result<(), std::io::Error> {
    if power <= 100 { Ok(()) } else {
        Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Power must be in 0..100%"))
    }
}

// Validate the whole transaction before touching the device.
fn enable_plan(p: f32, i: f32, d: f32, power: u8, offset: f32) -> Result<Vec<Request>, std::io::Error> {
    validate_pid(p, i, d)?;
    validate_power(power)?;
    validate_offset(offset)?;
    Ok(vec![
        Request::new(commands::set::POINT_OFFSET, offset.to_le_bytes()),
        Request::new(commands::set::P_COEFFICIENT, p.to_le_bytes()),
        Request::new(commands::set::I_COEFFICIENT, i.to_le_bytes()),
        Request::new(commands::set::D_COEFFICIENT, d.to_le_bytes()),
        Request::new(commands::set::DISABLE_NOT_ENABLE, payload_alimentazione(true)),
        Request::new(commands::set::TEC_POWER_LEVEL, [power, 0, 0, 0]),
        Request::new(commands::set::TEC_POWER_LEVEL, [power, 0, 0, 0]),
    ])
}

fn execute_enable(plan: &[Request], mut send: impl FnMut(&Request) -> Result<(), std::io::Error>) -> Result<(), std::io::Error> {
    for request in plan {
        if let Err(error) = send(request) {
            // An ACK can be lost after the board applied a command. Always
            // attempt disable, including an error on the enable packet itself.
            let off = Request::new(commands::set::DISABLE_NOT_ENABLE, payload_alimentazione(false));
            return match send(&off) {
                Ok(()) => Err(std::io::Error::new(error.kind(), format!("Enable failed at opcode 0x{:02X}: {error}; disable acknowledged", request.op_code))),
                Err(disable_error) => Err(std::io::Error::new(error.kind(), format!("Enable failed at opcode 0x{:02X}: {error}; DISABLE NOT CONFIRMED: {disable_error}", request.op_code))),
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod transaction_tests {
    use super::*;

    #[test]
    fn invalid_parameters_never_produce_a_transaction() {
        assert!(enable_plan(f32::NAN, 1.0, 0.0, 30, 0.0).is_err());
        assert!(enable_plan(100.0, -1.0, 0.0, 30, 0.0).is_err());
        assert!(enable_plan(100.0, 1.0, 0.0, 101, 0.0).is_err());
        assert!(enable_plan(100.0, 1.0, 0.0, 30, f32::INFINITY).is_err());
        assert!(enable_plan(1001.0, 1.0, 0.0, 30, 0.0).is_err());
        assert!(enable_plan(100.0, 1.0, 0.0, 30, -1000.0).is_err());
    }

    #[test]
    fn each_serial_failure_attempts_disable_and_stops_the_sequence() {
        let plan = enable_plan(100.0, 1.0, 0.0, 30, -3.0).unwrap();
        for failed in 0..plan.len() {
            let mut calls = Vec::new();
            let result = execute_enable(&plan, |r| {
                calls.push((r.op_code, r.data));
                if calls.len() == failed + 1 {
                    Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "lost ACK"))
                } else { Ok(()) }
            });
            assert!(result.is_err());
            assert_eq!(calls.len(), failed + 2);
            assert_eq!(calls.last(), Some(&(commands::set::DISABLE_NOT_ENABLE, payload_alimentazione(false))));
        }
    }

    #[test]
    fn failed_disable_is_reported_explicitly() {
        let plan = enable_plan(100.0, 1.0, 0.0, 30, 0.0).unwrap();
        let error = execute_enable(&plan, |_| Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "offline"))).unwrap_err();
        assert!(error.to_string().contains("DISABLE NOT CONFIRMED"));
    }

    #[test]
    fn successful_enable_preserves_the_wire_sequence() {
        let plan = enable_plan(100.0, 1.0, 0.0, 30, -3.0).unwrap();
        let mut ops = Vec::new();
        execute_enable(&plan, |r| { ops.push(r.op_code); Ok(()) }).unwrap();
        assert_eq!(ops, [0x14, 0x15, 0x16, 0x17, 0x18, 0x1d, 0x1d]);
        assert_eq!(plan[4].data, payload_alimentazione(true));
    }
}

impl Tec {
    /// Scrive un comando e legge la risposta, ripulendo il canale se si
    /// disallinea.
    ///
    /// **Il problema che risolve.** Il protocollo e' half-duplex senza numero
    /// di sequenza: l'unico controllo e' che il byte di start sia 0xAA e che
    /// l'op_code risponda a quello appena inviato. Se una lettura va in
    /// timeout, restano 8 byte (o parte di essi) nel buffer. Il `read_exact`
    /// successivo consuma quei byte sbagliati, l'op_code non combacia e il
    /// comando fallisce: **ogni comando dopo un timeout fallisce, per
    /// sempre**, perche' il buffer non si svuota mai da solo.
    ///
    /// Qui, quando la risposta non e' plausibile, si scarta il buffer di
    /// input e si ritenta una volta. Il controller risponde a ogni comando,
    /// quindi ritentare non e' pericoloso: al massimo si duplica una
    /// scrittura idempotente.
    fn send_cmd(&mut self, request: &Request) -> Result<Response, std::io::Error> {
        self.send_cmd_once(request).or_else(|first| {
            // Si scarta tutto cio' che e' rimasto nel buffer di ricezione e
            // si riprova una volta. Senza questo, un singolo timeout rendeva
            // il controller irraggiungibile per il resto della sessione.
            self.drain_input();
            self.send_cmd_once(request).map_err(|second| std::io::Error::new(
                first.kind(), format!("Serial opcode 0x{:02X}: {first}; retry: {second}", request.op_code)))
        })
    }

    /// Sonde gli opcode **sconosciuti** per cercare letture non ancora mappate
    /// (per esempio l'RPM della pompa, che il manuale Intel dice essere
    /// collegata direttamente alla scheda Cryo Controller ma che nei codici
    /// noti non compare).
    ///
    /// # AVVERTENZA: RISCHIO PER L'HARDWARE
    ///
    /// **Questa funzione deve essere chiamata solo con il TEC spento, il
    /// controller scollegato dal carico termico e nessun altro processo
    /// collegato alla stessa porta seriale.** La funzione **non verifica**
    /// lo stato del TEC, **non verifica** che nessun altro canale stia
    /// pilotando la board, e **non protegge** il modulo: non c'e' nessun
    /// interlock, nessun check di `BOARD_INIT`, nessuna sequenza di
    /// ripristino. La responsabilita' e' interamente del chiamante.
    ///
    /// Non invia "solo letture con dati nulli che non possono scrivere un
    /// valore nuovo": **quella frase era falsa e costava un reset di
    /// fabbrica.** Con `[0, 0, 0, 0]`:
    ///
    ///  - `0x18` = `DISABLE_NOT_ENABLE` -> **abilita** il TEC (il nome dice
    ///    il resto, non "disabilita");
    ///  - `0x1E` = `RESET_BOARD` -> **reset di fabbrica**, perde PID, setpoint
    ///    e tetto di potenza;
    ///  - `0x15`/`0x16`/`0x17` -> **azzera** i guadagni P/I/D, e con PID a
    ///    zero il controllo smette di funzionare;
    ///  - `0x1D` -> porta il tetto di potenza allo **0%**;
    ///  - `0x14` = `POINT_OFFSET` -> azzera il setpoint.
    ///
    /// Per questo gli opcode inviati passano da `SOLO_LETTURE`, un'allowlist
    /// scritta a partire dai soli valori in `commands::get` piu' `HEART_BEAT`:
    /// tutto il resto del range richiesto viene **saltato**, non inviato. Il
    /// filtro vale per qualunque `da`/`a` il chiamante passi, quindi non
    /// basta spostare gli argomenti per aggirarlo. Attenzione: `0x14..=0x17`
    /// **non** e' un intervallo di sole letture, e' `commands::set`.
    ///
    /// Un codice che risponde con un frame CRC-valido *potrebbe* essere una
    /// lettura. Non e' una prova: puo' essere anche un eco. Il valore va
    /// interpretato con criterio, e i dati ottenuti non vanno usati per
    /// comandare l'hardware finche' non sono stati messi in sicurezza.
    ///
    /// Restituisce una riga per opcode **inviato**: ha risposto, e il CRC era
    /// valido. Gli opcode fuori allowlist non compaiono nel risultato.
    pub fn probe_opcodes(&mut self, da: u8, a: u8) -> Vec<(u8, bool, bool)> {
        let mut esiti = Vec::new();
        for op in probe_plan(da, a) {
            // Dati nulli: per un opcode in sola lettura non e' un dato, e'
            // solo il contenitore del frame.
            let req = Request::new(op, [0, 0, 0, 0]);
            match self.send_cmd(&req) {
                Ok(resp) => {
                    // `from_bytes` accetta gia' solo i frame con CRC valido,
                    // quindi una risposta Ok significa anche CRC buono.
                    esiti.push((op, true, resp.op_code == op));
                }
                Err(_) => esiti.push((op, false, false)),
            }
        }
        esiti
    }

    /// Svuota il buffer di ricezione.
    ///
    /// `COMPort` su Windows non espone `clear()`, quindi si legge con un
    /// timeout brevissimo finche' non arriva piu' nulla. Il limite di 64
    /// byte evita che un stream continuo (il controller che risponde a
    /// ripetizione) trasformi lo scarico in un loop infinito.
    fn drain_input(&mut self) {
        const MAX_DRAIN: usize = 64;
        let _ = self.port.set_timeout(std::time::Duration::from_millis(5));
        let mut scratch = [0u8; 8];
        let mut drained = 0usize;
        while drained < MAX_DRAIN {
            match self.port.read(&mut scratch) {
                // Andare avanti finche' il buffer e' vuoto e' il caso
                // normale: la fine si riconosce dal timeout.
                Ok(0) => break,
                Ok(n) => drained += n,
                Err(_) => break,
            }
        }
        // Timeout di lavoro ripristinato: con 5 ms ogni lettura successiva
        // fallirebbe.
        let _ = self
            .port
            .set_timeout(self.work_timeout);
    }

    /// Un singolo scambio request/response, senza recupero.
    fn send_cmd_once(
        &mut self,
        request: &Request,
    ) -> Result<Response, std::io::Error> {
        self.port.write_all(&request.as_bytes())?;
        // Il controller puo' avere byte arretrati da una risposta precedente
        // persa: senza flush finirebbero nel flusso di questa.
        let _ = self.port.flush();

        let mut buffer = [0u8; 8];
        self.port.read_exact(&mut buffer)?;
        // Valida il byte di start — deve essere sempre 0xAA.
        // Un valore diverso indica garbage nel buffer (risposta di comando precedente,
        // rumore sul bus UART, o timeout parziale).
        if buffer[0] != 0xAA {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Response start byte invalid: expected 0xAA, got 0x{:02X}", buffer[0]),
            ));
        }
        if buffer[1] != request.op_code.wrapping_add(127) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Response incorrect op code: expected {}, got {}",
                    request.op_code.wrapping_add(127),
                    buffer[1]
                ),
            ));
        }
        let crc = CRC_16_XMODEM.checksum(&buffer[0..6]);
        let response = Response::from_bytes(buffer);
        if response.crc == crc {
            Ok(response)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Response contained incorrect CRC16-XMODEM",
            ))
        }
    }

    // ── NOTA: qui non c'e' piu` nessun `reset()`. ────────────────────────
    //
    // Il metodo mandava `0x1E` (reset di fabbrica) e lo chiamava `Tec::new`
    // quando `BOARD_INIT` non era impostato. E' stato **rimosso**, non solo
    // smesso di chiamare: lasciare il metodo in piedi, privato e non usato,
    // sarebbe una trappola per chi lo riusa in un ramo di avvio o in un
    // percorso di recupero, convinto che "riazzera la board".
    //
    // Non e' una perdita di funzionalita': `0x1E` perde PID, setpoint e power
    // cap, quindi nessun codice legittimo lo vuole. Se in futuro servisse
    // reinizializzare una board, si scrive un'azione di commissioning esplicita,
    // con la conferma di chi la preme — non un effetto collaterale.

    /// Scrive i coefficienti PID al controller.
    ///
    /// Reso pubblico perche' senza di questo ogni modifica di P/I/D
    /// nell'app restava solo in stato: l'unico percorso che raggiungeva il
    /// firmware era `enable()`, cioe' premere "Abilita TEC". Cambiare un
    /// coefficiente e non premere Abilita non cambiava nulla sul controller,
    /// e la UI mostrava il valore nuovo mentre l'hardware usava quello
    /// vecchio.
    pub fn set_pid(&mut self, p: f32, i: f32, d: f32) -> Result<(), std::io::Error> {
        validate_pid(p, i, d)?;
        self.send_cmd(&Request::new(commands::set::P_COEFFICIENT, p.to_le_bytes()))?;
        self.send_cmd(&Request::new(commands::set::I_COEFFICIENT, i.to_le_bytes()))?;
        self.send_cmd(&Request::new(commands::set::D_COEFFICIENT, d.to_le_bytes()))?;
        Ok(())
    }
}

/// I quattro byte di `0x18` per accendere o spegnere il TEC.
///
/// **Perche' esiste una funzione per questo, e non basta guardare `applica_regime`.**
///
/// Il nome dell'opcode e' `DISABLE_NOT_ENABLE` e il suo bit 0 e' *negato*:
/// `[0,0,0,0]` accende, `[1,0,0,0]` spegne. E' la trappola che ha gia' fatto
/// perdere il TEC a un altro, ed e' anche il punto in cui un errore di un
/// carattere e' indistinguibile dal successo: il controller accetta il
/// pacchetto, non si rompe, e semplicemente fa il contrario di quello chiesto.
///
/// Incollando i byte alla mano in tre punti diversi del file — `enable`,
/// `disable`, `applica_regime` — si ha un solo posto dove sbagliare *una
/// volta* e poi correggerlo in un altro, lasciando i due inconsistenti. Qui la
/// polarita' e' scritta una volta sola, e il test la verifica.
///
/// `true` = acceso, `false` = spento, in entrambi i sensi.
pub fn payload_alimentazione(tec_acceso: bool) -> [u8; 4] {
    if tec_acceso {
        [0, 0, 0, 0]
    } else {
        [1, 0, 0, 0]
    }
}

/// Se aprire una connessione debba emettere il reset di fabbrica (`0x1E`).
///
/// **La risposta e' no, e non e' una semplificazione: e' una correzione.**
///
/// La versione precedente rispondeva "si" ogni volta che `BOARD_INIT` non era
/// impostato, e `Tec::new` lo usava per mandare `0x1E`. La conseguenza era che
/// PID, setpoint e power cap sparivano al primo ricollegamento, senza che
/// l'operatore avesse toccato niente. Il power cap in particolare e' il limite
/// di potenza che l'operatore ha impostato per stare tranquillo: azzerarlo di
/// nascosto all'apertura della porta non e' un dettaglio, e' togliere una
/// protezione senza chiederlo.
///
/// "Non e' un TEC, fallisce qui" e' gia' garantito da `hear_beat`: non e' la
/// condizione di `BOARD_INIT` a decidere se la scheda e' viva.
///
/// Se in futuro servisse una procedura di inizializzazione della board, sara' un
/// azione esplicita di commissioning con la sua conferma — non un effetto
/// collaterale dell'apertura della porta.
///
/// La funzione resta anche se non ha piu' casi veri: e' il posto dove la
/// decisione e' dichiarata, e il test che la presidia deve trovare qualcosa su
/// cui agire invece di non trovare niente.
pub fn serve_reset_alla_connessione(_status: TecStatus) -> bool {
    false
}

impl Tec {
    /// Costruisce un `Tec` da una porta **già aperta e configurata**.
    ///
    /// Utile per i test di integrazione e per chi gestisce
    /// l'apertura: evita il doppio `open` sullo stesso handle USB.
    pub fn from_open_port(mut port: serial::SystemPort) -> Self {
        let _ = port.set_timeout(WORK_TIMEOUT);
        Tec { port, work_timeout: WORK_TIMEOUT }
    }

    /// Apre la porta e interroga il controller **senza modificarne lo stato**.
    ///
    /// Serve al rilevamento automatico: `new()` è pensato per la connessione
    /// reale e, se la board non è in `BOARD_INIT`, manda un `reset`. Durante
    /// una scansione questo sarebbe dannoso — ogni tentativo riporterebbe la
    /// board allo stato di fabbrica, e il retry automatico (uno ogni 2s)
    /// farebbe reset ripetuti. Qui mandiamo solo `HEART_BEAT` e, se la
    /// risposta è un `TecStatus` valido, restituiamo l'istanza già aperta:
    /// il chiamante non deve riaprire la porta (secondo open sullo stesso
    /// handle USB fallirebbe).
    pub fn probe<T: AsRef<std::ffi::OsStr>>(
        serial_port: &T,
        timeout: std::time::Duration,
    ) -> Result<Self, std::io::Error> {
        let mut port = serial::open(serial_port)?;
        port.reconfigure(&|settings| {
            settings.set_baud_rate(serial::Baud115200)?;
            settings.set_char_size(serial::Bits8);
            settings.set_stop_bits(serial::Stop1);
            settings.set_parity(serial::ParityNone);
            settings.set_flow_control(serial::FlowNone);
            Ok(())
        })?;
        port.set_timeout(timeout)?;
        let mut tec = Tec { port, work_timeout: WORK_TIMEOUT };
        // Se non è un TEC, fallisce qui: nessuna scrittura di stato.
        tec.hear_beat()?;
        Ok(tec)
    }

    pub fn new<T: AsRef<std::ffi::OsStr>>(serial_port: &T) -> Result<Self, std::io::Error> {
        let mut port = serial::open(serial_port)?;
        port.reconfigure(&|settings| {
            settings.set_baud_rate(serial::Baud115200)?;
            settings.set_char_size(serial::Bits8);
            settings.set_stop_bits(serial::Stop1);
            settings.set_parity(serial::ParityNone);
            settings.set_flow_control(serial::FlowNone);
            Ok(())
        })?;
        // Timeout per singolo round-trip seriale.
        //
        // `monitor()` esegue 8 round-trip consecutivi sul thread UI. Con 1000ms
        // ciascuno, un cooler scollegato bloccava la finestra per 8 secondi
        // (= 8s ogni 500ms, quindi appesa all'infinito). 150ms×8 = 1.2s al
        // massimo: abbastanza generoso per rispondere a 115200 baud, senza
        // far congelare la UI quando il device non c'è.
        port.set_timeout(WORK_TIMEOUT)?;
        let mut tec = Tec { port, work_timeout: WORK_TIMEOUT };
        tec.drain_input();
        // Se non e' un TEC, fallisce qui: nessuna scrittura di stato.
        //
        // **Nota: qui non c'e' piu' nessun reset.** La versione precedente
        // mandava `0x1E` quando `BOARD_INIT` non era impostato, e perdi PID,
        // setpoint e power cap a ogni ricollegamento. Il power cap e' la
        // protezione che l'operatore ha configurato: azzerarla di nascosto
        // all'apertura della porta non e' un dettaglio, e' togliere una
        // protezione senza chiederlo.
        tec.hear_beat()?;
        Ok(tec)
    }

    pub fn hear_beat(&mut self) -> Result<TecStatus, std::io::Error> {
        Ok(self.hear_beat_completo()?.noti)
    }

    /// Heartbeat che **non perde informazione**.
    ///
    /// `hear_beat` mascherava il campo a 32 bit a 18 bit, scartando i 14
    /// alti: e' per questo che `from_bits` non poteva mai restituire `None`
    /// e la validazione era codice morto. Qui il campo viene scomposto e
    /// conservato.
    pub fn hear_beat_completo(&mut self) -> Result<tecstatus::StatusCompleto, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::HEART_BEAT, [0; 4]))?;
        Ok(tecstatus::scompone(u32::from_le_bytes(response.data)))
    }

    pub fn monitor(&mut self) -> Result<MonitoringData, std::io::Error> {
        let tec_voltage = self.tec_voltage()?;
        let tec_current = self.tec_current()?;
        let tec_temperature = self.tec_temperature()?;
        let dew_point_temperature = self.dew_point_temperature()?;
        let tec_power_watts = tec_voltage * tec_current;
        let condensation_margin = tec_temperature - dew_point_temperature;

        let data = MonitoringData {
            timestamp: Utc::now(),
            tec_temperature,
            pcb_temperature: self.board_temperature()?,
            humidity: self.humidity()?,
            dew_point_temperature,
            tec_voltage,
            tec_current,
            tec_power_level: self.tec_power_level()?,
            tec_power_watts,
            condensation_margin,
        };
        data.validate()?;
        Ok(data)
    }

    pub fn humidity(&mut self) -> Result<f32, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::HUMIDITY, [0; 4]))?;
        Ok(f32::from_le_bytes(response.data))
    }

    pub fn tec_temperature(&mut self) -> Result<f32, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::TEC_TEMPERATURE, [0; 4]))?;
        Ok(f32::from_le_bytes(response.data))
    }

    pub fn board_temperature(&mut self) -> Result<f32, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::BOARD_TEMP, [0; 4]))?;
        Ok(f32::from_le_bytes(response.data))
    }

    pub fn dew_point_temperature(&mut self) -> Result<f32, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::DEW_POINT, [0; 4]))?;
        Ok(f32::from_le_bytes(response.data))
    }

    pub fn tec_voltage(&mut self) -> Result<f32, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::TEC_VOLTAGE, [0; 4]))?;
        // Fattore di scala ADC → Volt.
        // 21.1 = ADC_FULL_SCALE / (V_REF × VOLTAGE_DIVIDER_RATIO)
        // Derivato dal circuito PCB del MasterLiquid ML360 SUB-ZERO (resistori R1/R2 sul divisore).
        Ok(u32::from_le_bytes(response.data) as f32 / 21.1)
    }

    pub fn tec_current(&mut self) -> Result<f32, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::TEC_CURRENT, [0; 4]))?;
        // Fattore di scala ADC → Ampere.
        // 4.6545 = ADC_FULL_SCALE / (I_SENSE_GAIN × SHUNT_RESISTANCE_mΩ × ADC_VREF)
        // Shunt ~10 mΩ, gain amplificatore ×50 (INA193 o equivalente).
        Ok(u32::from_le_bytes(response.data) as f32 / 4.6545)
    }

    pub fn tec_power_level(&mut self) -> Result<u8, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::TEC_POWERLEVEL, [0; 4]))?;
        Ok(response.data[0])
    }

    pub fn p_coefficient(&mut self) -> Result<f32, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::P_COEFFICIENT, [0; 4]))?;
        Ok(f32::from_le_bytes(response.data))
    }

    pub fn i_coefficient(&mut self) -> Result<f32, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::I_COEFFICIENT, [0; 4]))?;
        Ok(f32::from_le_bytes(response.data))
    }

    pub fn d_coefficient(&mut self) -> Result<f32, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::D_COEFFICIENT, [0; 4]))?;
        Ok(f32::from_le_bytes(response.data))
    }

    pub fn set_setpoint_offset(&mut self, setpoint: f32) -> Result<(), std::io::Error> {
        validate_offset(setpoint)?;
        self.send_cmd(&Request::new(
            commands::set::POINT_OFFSET,
            setpoint.to_le_bytes(),
        ))?;
        Ok(())
    }

    pub fn setpoint_offset(&mut self) -> Result<f32, std::io::Error> {
        let response=self.send_cmd(&Request::new(commands::get::SET_POINT_OFFSET,[0;4]))?;
        let offset=f32::from_le_bytes(response.data);
        validate_finite(offset,"offset letto")?;
        Ok(offset)
    }

    pub fn hw_version(&mut self) -> Result<u32, std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::HW_VERSION, [0; 4]))?;
        Ok(u32::from_le_bytes(response.data))
    }

    pub fn fw_version(&mut self) -> Result<(u8, u8, u8, u8), std::io::Error> {
        let response = self.send_cmd(&Request::new(commands::get::FW_VERSION, [0; 4]))?;
        Ok((
            response.data[0],
            response.data[1],
            response.data[2],
            response.data[3],
        ))
    }

    /// Set TEC max power level (0–100).
    /// NOTE: Some firmware versions may silently ignore this; verify with get after set.
    pub fn set_power_level(&mut self, power_level: u8) -> Result<(), std::io::Error> {
        validate_power(power_level)?;
        self.send_cmd(&Request::new(
            commands::set::TEC_POWER_LEVEL,
            [power_level, 0, 0, 0],
        ))?;
        Ok(())
    }

    /// Set TEC max power level con read-back di verifica.
    /// Legge il valore attuale dopo il set e ritorna Err se il firmware lo ha ignorato.
    /// Utile per diagnosticare firmware che silenziosamente ignorano il cap di potenza.
    pub fn set_power_level_verified(&mut self, power_level: u8) -> Result<u8, std::io::Error> {
        self.set_power_level(power_level)?;
        // Piccola pausa — il firmware ha bisogno di qualche ms per aggiornare il registro
        std::thread::sleep(std::time::Duration::from_millis(20));
        let actual = self.tec_power_level()?;
        if actual != power_level {
            // Il firmware ha ignorato il set — loggato come InvalidData ma non fatale
            // Il chiamante decide se ritentare
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Power level mismatch: sent {power_level}%, firmware returned {actual}%. \
                     Firmware may be ignoring the cap (known Intel Cryo firmware limitation)."
                ),
            ));
        }
        Ok(actual)
    }

    /// Feed external CPU temperature to device PID controller.
    /// Enables proper PID operation on AMD or non-natively-supported Intel CPUs.
    pub fn set_cpu_temp(&mut self, temp_celsius: f32) -> Result<(), std::io::Error> {
        if !temp_celsius.is_finite() || !(0.0..=150.0).contains(&temp_celsius) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "CPU temperature outside 0..150 C"));
        }
        self.send_cmd(&Request::new(
            commands::set::CPU_TEMP,
            temp_celsius.to_le_bytes(),
        ))?;
        Ok(())
    }

    pub fn enable(
        &mut self,
        p: f32,
        i: f32,
        d: f32,
        power_level: u8,
        setpoint: f32,
    ) -> Result<(), std::io::Error> {
        // ORDINE CRITICO — documentato dal reverse engineering del protocollo Intel:
        // 1. Imposta PID e setpoint PRIMA di abilitare
        // 2. Abilita il TEC (DISABLE_NOT_ENABLE con [0,0,0,0])
        // 3. Invia set_power_level DOPO l'enable — il firmware resetta il cap all'atto
        //    dell'abilitazione, quindi mandarlo prima è inutile.
        let plan = enable_plan(p, i, d, power_level, setpoint)?;
        execute_enable(&plan, |request| self.send_cmd(request).map(|_| ()))
    }

    pub fn disable(&mut self) -> Result<(), std::io::Error> {
        self.send_cmd(&Request::new(
            commands::set::DISABLE_NOT_ENABLE,
            payload_alimentazione(false),
        ))?;
        Ok(())
    }

    

    ///
    /// L'ordine non e' un dettaglio. Il reverse engineering del software Intel
    /// (`IntelCryoCooling.Controller.dll`) mostra che `InitCryoMode`,
    /// `InitUnregulatedMode` e `InitStandbyMode` non sono metodi separati:
    /// chiamano tutti e tre lo stesso worker privato
    /// (`_SuwRS2F8fovSdlJCBMLFuCKsDOk`, RVA `0x96B8`), e quel worker chiama
    /// prima `SetSetPointOffset` e poi `GetBoardStatus`. Non esiste un
    /// opcode "cambia modalita'": il regime e' una **consequenza** di dove
    /// cade il setpoint e se il TEC resta alimentato.
    ///
    /// Per questo qui non si accende il TEC *prima* di scrivere l'offset:
    /// se il setpoint non e' ancora a posto, l'abilitazione parte con il
    /// valore vecchio e per un istante la cella va dove non deve. Prima
    /// l'obiettivo, poi l'alimentazione.
    ///
    /// # Perche' non c'e' un modo "molto piu' breve"
    ///
    /// Sembra che basti `set_setpoint_offset`. Ma `0x14` sta a due byte da
    /// `0x1E`, che e' il **reset di fabbrica**: su questo protocollo
    /// half-duplex senza numero di sequenza, un byte sbagliato azzera il
    /// controller e perde PID, setpoint e tetto di potenza. Il secondo
    /// comando serve a dichiarare esplicitamente l'intenzione, e il
    /// richiamo finale a `hear_beat_completo` serve a **verificare** che il
    /// controller abbia accettato, invece di restare a credere di averlo
    /// fatto.
    ///
    /// Restituisce lo stato **rilevato dopo** la scrittura, non quello
    /// ipotizzato: e' l'unico modo perche' il chiamante possa distinguere
    /// "commutato" da "ho sparato due byte e il controller ha ignorato".
    /// La potenza con cui si parte: 30.
    ///
    /// Non e' il cap dell'operatore. E' il punto da cui la salita graduale
    /// parte: subito al cap la piastra crolla sotto la rugiada prima che
    /// l'operatore possa reagire, e il firmware resta inchiodato al massimo.
    pub const SOFT_START_POWER: u8 = 30;

    pub fn applica_regime(
        &mut self,
        offset: Option<f32>,
        tec_acceso: bool,
    ) -> Result<tecstatus::StatusCompleto, std::io::Error> {
        self.applica_regime_con_pid(offset, tec_acceso, 100.0, 1.0, 0.0)
    }

    /// Come `applica_regime`, ma con i coefficienti PID dell'operatore.
    ///
    /// I PID sono parte della sequenza di accensione, non un parametro
    /// separato: senza coefficienti il firmware non ha un regolatore e resta
    /// fermo al livello di default. Vengono dalla UI perche' l'operatore li
    /// cambia dal pannello, e devono arrivare al momento dell'abilitazione —
    /// non al prossimo campione, perche' a quel punto il modulo e' gia'
    /// partito con i coefficienti sbagliati.
    pub fn applica_regime_con_pid(
        &mut self,
        offset: Option<f32>,
        tec_acceso: bool,
        p: f32,
        i: f32,
        d: f32,
    ) -> Result<tecstatus::StatusCompleto, std::io::Error> {
        self.applica_regime_completo(offset, tec_acceso, p, i, d, false)
    }

    /// Come `applica_regime_con_pid`, ma con la possibility di **spengere
    /// prima**: serve a passare da un regime acceso a un altro regime acceso.
    ///
    /// `prima_spegni` e' il caso di Unregulated → Cryo. Il firmware **ignora
    /// un enable quando e' gia' acceso**: il comando passa, l'offset viene
    /// scritto, ma il modulo non si riaccende e resta nel regime precedente,
    /// mentre la dashboard dichiara quello nuovo. Per questo l'unica via
    /// era spegnere e riaccendere a mano.
    ///
    /// Con `prima_spegni` il passaggio avviene in un colpo solo: disable, e
    /// subito dopo la sequenza di accensione completa.
    pub fn applica_regime_completo(
        &mut self,
        offset: Option<f32>,
        tec_acceso: bool,
        p: f32,
        i: f32,
        d: f32,
        prima_spegni: bool,
    ) -> Result<tecstatus::StatusCompleto, std::io::Error> {
        if tec_acceso {
            if prima_spegni {
                // Il disable non tocca l'offset: il controller lo tiene come
                // ultima impostazione, e subito dopo riscriviamo quello giusto.
                self.disable()?;
                // Pausa brevissima: il firmware deve vedere il modulo fermo
                // prima dell'enable, altrimenti lo ignora come prima.
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            // **Sequenza dell'originale, in quest'ordine esatto.**
            //
            // `enable()` (quello usato dalla r116 che funziona) fa quattro
            // cose: setpoint, **PID**, enable, **potenza due volte**. Il mio
            // `applica_regime` faceva solo setpoint + enable: senza i
            // coefficienti il firmware non ha un regolatore da usare, e
            // senza un livello iniziale resta fermo al 10% di default con
            // 0.7 W — il sintomo "non si abilita". Non era un problema di
            // failsafe ne' di manuale: era la sequenza incompleta.
            //
            // L'ordine conta: il firmware azzera il cap all'abilitazione,
            // quindi il livello va DOPO l'enable, e due volte perche' alcuni
            // firmware ignorano il primo pacchetto subito dopo l'avvio.
            self.enable(
                p,
                i,
                d,
                Self::SOFT_START_POWER,
                offset.unwrap_or(0.0),
            )?;
        } else {
            // Lo spegnimento non tocca l'offset: il controller lo tiene come
            // ultima impostazione e un riavvio successivo riparte da li'.
            self.disable()?;
        }
        self.hear_beat_completo()
    }

    /// `true` se il modulo risulta acceso dai **watt misurati**.
    ///
    /// Non dai bit: `OCP_ACTIVE` e' rumore su questo impianto e resta alto
    /// anche da spento, quindi decidere con quello faceva credere acceso un
    /// modulo fermo — e faceva scattare lo spegnimento "di sicurezza" quando
    /// non serviva.
    pub fn risulta_acceso(&self, watts: f32) -> bool {
        watts > 2.0
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
    pub struct TecStatus: u32 {
        const BOARD_INIT            = 1 << 0;
        const POWER_OK              = 1 << 1;
        const TEMP_SENSE_OK         = 1 << 2;
        const HUM_SENSE_OK          = 1 << 3;
        const LAST_CMD_OK           = 1 << 4;
        const LAST_CMD_BAD_CRC      = 1 << 5;
        const LAST_CMD_INCOMPLETE   = 1 << 6;
        const FAILSAFE_ACTIVE       = 1 << 7;
        const PID_READY             = 1 << 8;
        const PID_INVALID           = 1 << 9;
        const PID_OUT_OF_RANGE      = 1 << 10;
        const PID_DEFAULT           = 1 << 11;
        const PID_RUNNING           = 1 << 12;
        const OCP_ACTIVE            = 1 << 13;
        const BOARD_TEMP_OK         = 1 << 14;
        const TEC_CONN_OK           = 1 << 15;
        const LOW_POWER_MODE_ACTIVE = 1 << 16;
        const TEMP_MODE             = 1 << 17;
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Request {
    fixed: u8,
    op_code: u8,
    data: [u8; 4],
    crc: u16,
}

impl Request {
    pub const fn new(op_code: u8, data: [u8; 4]) -> Self {
        let buffer = [0xAA, op_code, data[0], data[1], data[2], data[3]];
        let crc = CRC_16_XMODEM.checksum(&buffer);
        Request { fixed: 0xAA, op_code, data, crc }
    }

    const fn as_bytes(&self) -> [u8; 8] {
        [
            self.fixed,
            self.op_code,
            self.data[0],
            self.data[1],
            self.data[2],
            self.data[3],
            (self.crc & 0xFF) as u8,
            (self.crc >> 8) as u8,
        ]
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct Response {
    fixed: u8,
    op_code: u8,
    data: [u8; 4],
    crc: u16,
}

impl Response {
    fn from_bytes(bytes: [u8; 8]) -> Self {
        Response {
            fixed: bytes[0],
            op_code: bytes[1],
            data: bytes[2..6].try_into().expect("Constant slice size"),
            crc: u16::from_le_bytes(bytes[6..8].try_into().expect("Constant slice size")),
        }
    }
}

#[allow(dead_code)]
mod commands {
    pub const HEART_BEAT: u8 = 0x00;
    pub mod get {
        pub const TEC_TEMPERATURE:     u8 = 0x01;
        pub const HUMIDITY:            u8 = 0x02;
        pub const DEW_POINT:           u8 = 0x03;
        pub const SET_POINT_OFFSET:    u8 = 0x04;
        pub const P_COEFFICIENT:       u8 = 0x05;
        pub const I_COEFFICIENT:       u8 = 0x06;
        pub const D_COEFFICIENT:       u8 = 0x07;
        pub const TEC_POWERLEVEL:      u8 = 0x08;
        pub const HW_VERSION:          u8 = 0x09;
        pub const FW_VERSION:          u8 = 0x0A;
        pub const NTC_COEFFICIENT:     u8 = 0x1B;
        pub const BOARD_TEMP:          u8 = 0x1F;
        pub const VOLTAGE_AND_CURRENT: u8 = 0x22;
        pub const TEC_VOLTAGE:         u8 = 0x23;
        pub const TEC_CURRENT:         u8 = 0x24;
    }
    pub mod set {
        pub const POINT_OFFSET:        u8 = 0x14;
        pub const P_COEFFICIENT:       u8 = 0x15;
        pub const I_COEFFICIENT:       u8 = 0x16;
        pub const D_COEFFICIENT:       u8 = 0x17;
        pub const DISABLE_NOT_ENABLE:  u8 = 0x18;
        pub const CPU_TEMP:            u8 = 0x19;
        pub const NTC_COEFFICIENT:     u8 = 0x1A;
        pub const TEMP_SENSOR:         u8 = 0x1C;
        pub const TEC_POWER_LEVEL:     u8 = 0x1D;
        pub const RESET_BOARD:         u8 = 0x1E;
    }
}

pub struct MonitoringData {
    pub timestamp:             chrono::DateTime<Utc>,
    pub tec_temperature:       f32,
    pub pcb_temperature:       f32,
    pub humidity:              f32,
    pub dew_point_temperature: f32,
    pub tec_voltage:           f32,
    pub tec_current:           f32,
    pub tec_power_level:       u8,
    /// Real electrical power in Watts (V × I)
    pub tec_power_watts:       f32,
    /// Safety margin above dew point in °C. Positive = safe, negative = CONDENSATION RISK
    pub condensation_margin:   f32,
}

impl MonitoringData {
    fn validate(&self) -> Result<(),std::io::Error> {
        for (name,value,min,max) in [
            ("TEC temperature",self.tec_temperature,-40.0,100.0),
            ("board temperature",self.pcb_temperature,-40.0,150.0),
            ("dew point",self.dew_point_temperature,-40.0,60.0),
            ("humidity",self.humidity,0.0,100.0),
            ("voltage",self.tec_voltage,0.0,60.0),
            ("current",self.tec_current,0.0,100.0),
            ("power",self.tec_power_watts,0.0,1000.0),
            ("condensation margin",self.condensation_margin,-100.0,140.0),
        ] {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,format!("Invalid {name}: {value}")));
            }
        }
        if self.tec_power_level>100 {return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"Invalid duty >100%"));}
        Ok(())
    }
}

#[cfg(test)]
mod measurement_tests {
    use super::*;
    fn sample()->MonitoringData {MonitoringData {timestamp:Utc::now(),tec_temperature:20.0,pcb_temperature:30.0,humidity:50.0,dew_point_temperature:15.0,tec_voltage:7.0,tec_current:15.0,tec_power_level:50,tec_power_watts:105.0,condensation_margin:5.0}}
    #[test] fn corrupt_sensor_values_are_not_accepted_as_successful_samples() {
        for bad in [f32::NAN,f32::INFINITY,200.0] {let mut s=sample();s.pcb_temperature=bad;assert!(s.validate().is_err());}
        let mut s=sample();s.dew_point_temperature=f32::NAN;assert!(s.validate().is_err());
        let mut s=sample();s.humidity=101.0;assert!(s.validate().is_err());
        let mut s=sample();s.tec_voltage=-1.0;assert!(s.validate().is_err());
    }
    #[test] fn condensation_and_real_high_temperature_still_reach_the_guards() {
        let mut s=sample();s.pcb_temperature=90.0;s.condensation_margin=-3.0;s.tec_temperature=12.0;
        assert!(s.validate().is_ok());
    }
    #[test] fn enable_is_blocked_for_hot_or_invalid_board_sensor() {
        for t in [f32::NAN,f32::INFINITY,-100.0,37.0,38.0,40.0,76.0] {assert!(!board_allows_enable(t));}
        for t in [0.0,30.0,34.0,36.9] {assert!(board_allows_enable(t));}
    }
}

#[cfg(test)]
mod test_reset_alla_connessione {
    use super::{serve_reset_alla_connessione, TecStatus};

    /// **Ricollegarsi non deve mai azzerare il controller.**
    ///
    /// `0x1E` e' il reset di fabbrica: perde PID, setpoint e **power cap**.
    /// Perdere il power cap significa che il limite di potenza che l'operatore
    /// aveva impostato sparisce al primo ricollegamento — e quel limite e' la
    /// protezione che l'operatore ha messo per stare tranquillo.
    ///
    /// Il test enumera stati anche assurdi (`empty`, `all`) perche' la regola
    /// non ha eccezioni: nessuna combinazione di bit autorizza un reset.
    #[test]
    fn il_reset_di_fabbrica_non_e_mai_una_conseguenza_della_connessione() {
        for stato in [
            TecStatus::empty(),
            TecStatus::all(),
            TecStatus::BOARD_INIT,
            TecStatus::POWER_OK,
            TecStatus::BOARD_INIT | TecStatus::POWER_OK,
        ] {
            assert!(
                !serve_reset_alla_connessione(stato),
                "lo stato {stato:?} non deve mai autorizzare un reset di fabbrica"
            );
        }
    }

    /// Il caso pericoloso e' `BOARD_INIT` assente: era la condizione che prima
    /// faceva scattare `0x1E` a ogni connessione. Il test la nomina per nome,
    /// cosi' nessuno puo' reintrodurla scambiandola per un caso particolare
    /// legittimo.
    #[test]
    fn board_init_assente_non_e_un_eccezione() {
        let senza_init = TecStatus::POWER_OK | TecStatus::TEC_CONN_OK;
        assert!(!senza_init.contains(TecStatus::BOARD_INIT));
        assert!(
            !serve_reset_alla_connessione(senza_init),
            "una board non inizializzata NON autorizza il reset: e' il caso che perdeva il power cap"
        );
    }
}

#[cfg(test)]
mod test_polarita_alimentazione {
    use super::payload_alimentazione;

    /// **Il byte piu' pericoloso di tutto il protocollo.**
    ///
    /// L'opcode e' `DISABLE_NOT_ENABLE` e il suo bit 0 e' *negato*, quindi
    /// l'interpretazione e' opposta al nome: `[0,0,0,0]` accende,
    /// `[1,0,0,0]` spegne. Un errore qui non produce un errore, una
    /// eccezione, o un controller che si blocca: produce un controller che
    /// obbedisce e fa il contrario, in silenzio. Il modulo si accende quando
    /// l'operatore ha premuto "Spegni", consuma corrente, e la dashboard
    /// segnala "spento" — l'operatore non ha nessun modo di accorgersene
    /// guardando i numeri, perche' guardano il modulo che ha acceso.
    ///
    /// Il test fissa i due valori per nome, cosi' un refactor che li
    /// inverte non puo' passare.
    #[test]
    fn la_polarita_e_quella_documentata() {
        assert_eq!(
            payload_alimentazione(true),
            [0, 0, 0, 0],
            "true = acceso, e l'opcode e' negato: [0,0,0,0]"
        );
        assert_eq!(
            payload_alimentazione(false),
            [1, 0, 0, 0],
            "false = spento, e l'opcode e' negato: [1,0,0,0]"
        );
    }

    /// I due payload devono restare distinti in almeno un byte. E' una prova
    /// piu' debole di proposito: non ripete i valori assoluti, ma nota che se
    /// un domani i due casi producessero la stessa sequenza, il comando
    /// sarebbe indistinguibile e il difetto tornerebbe senza che nessun
    /// test di polarita' lo noti.
    #[test]
    fn acceso_e_spento_non_sono_la_stessa_sequenza() {
        assert_ne!(payload_alimentazione(true), payload_alimentazione(false));
    }
}

#[cfg(test)]
mod tests {
    use super::{is_solo_lettura, probe_plan, commands};

    /// Il test che chiude il pericolo: 0x18 con `[0,0,0,0]` **abilita** il
    /// TEC, 0x1E con `[0,0,0,0]` e' il reset di fabbrica. Nessuno dei due puo'
    /// essere raggiunto dalla sonda, per qualunque coppia di argomenti.
    #[test]
    fn la_sonda_non_manda_mai_enable_ne_reset() {
        assert!(
            !is_solo_lettura(commands::set::DISABLE_NOT_ENABLE),
            "0x18 con dati nulli ABILITA il TEC: non puo' essere in sola lettura"
        );
        assert!(
            !is_solo_lettura(commands::set::RESET_BOARD),
            "0x1E con dati nulli fa il RESET DI FABBRICA: perde PID, setpoint e power cap"
        );
    }

    /// Nessun opcode di `commands::set` deve essere nella allowlist: sono
    /// tutti scritture, e con `[0,0,0,0]` i peggiori sono quelli che azzerano
    /// i guadagni PID (0x15/0x16/0x17) e il tetto di potenza (0x1D).
    #[test]
    fn nessun_opcode_di_scrittura_e_in_allowlist() {
        let scritture = [
            ("POINT_OFFSET", commands::set::POINT_OFFSET),
            ("P_COEFFICIENT", commands::set::P_COEFFICIENT),
            ("I_COEFFICIENT", commands::set::I_COEFFICIENT),
            ("D_COEFFICIENT", commands::set::D_COEFFICIENT),
            ("DISABLE_NOT_ENABLE", commands::set::DISABLE_NOT_ENABLE),
            ("CPU_TEMP", commands::set::CPU_TEMP),
            ("NTC_COEFFICIENT", commands::set::NTC_COEFFICIENT),
            ("TEMP_SENSOR", commands::set::TEMP_SENSOR),
            ("TEC_POWER_LEVEL", commands::set::TEC_POWER_LEVEL),
            ("RESET_BOARD", commands::set::RESET_BOARD),
        ];
        for (nome, op) in scritture {
            assert!(!is_solo_lettura(op), "{nome} (0x{op:02X}) e' una scrittura");
        }
    }

    /// Il caso che ha motivato il fix: chiedere l'intero range 0x14..=0x1E
    /// non deve produrre 0x18 fra gli opcode inviati.
    #[test]
    fn il_range_chiesto_dal_chiamante_viene_filtrato() {
        let piano = probe_plan(0x14, 0x1E);
        assert!(
            !piano.contains(&0x18),
            "il piano della sonda contiene 0x18 (enable): {piano:?}"
        );
        assert!(
            !piano.contains(&0x1E),
            "il piano della sonda contiene 0x1E (factory reset): {piano:?}"
        );
        // 0x14..=0x1E e' `commands::set` tranne 0x1B, che e' una lettura.
        assert_eq!(
            piano,
            vec![0x1B],
            "dal range di sole scritture deve sopravvivere solo la lettura 0x1B"
        );
    }

    /// L'allowlist deve coprire le letture documentate, altrimenti la sonda
    /// serve a niente: senza questo test un filtro troppo stretto passerebbe
    /// comunque il test sugli opcode pericolosi.
    #[test]
    fn le_letture_documentate_restano_raggiungibili() {
        let letture = [
            ("HEART_BEAT", commands::HEART_BEAT),
            ("TEC_TEMPERATURE", commands::get::TEC_TEMPERATURE),
            ("HUMIDITY", commands::get::HUMIDITY),
            ("DEW_POINT", commands::get::DEW_POINT),
            ("SET_POINT_OFFSET", commands::get::SET_POINT_OFFSET),
            ("P_COEFFICIENT", commands::get::P_COEFFICIENT),
            ("I_COEFFICIENT", commands::get::I_COEFFICIENT),
            ("D_COEFFICIENT", commands::get::D_COEFFICIENT),
            ("TEC_POWERLEVEL", commands::get::TEC_POWERLEVEL),
            ("HW_VERSION", commands::get::HW_VERSION),
            ("FW_VERSION", commands::get::FW_VERSION),
            ("NTC_COEFFICIENT", commands::get::NTC_COEFFICIENT),
            ("BOARD_TEMP", commands::get::BOARD_TEMP),
            ("VOLTAGE_AND_CURRENT", commands::get::VOLTAGE_AND_CURRENT),
            ("TEC_VOLTAGE", commands::get::TEC_VOLTAGE),
            ("TEC_CURRENT", commands::get::TEC_CURRENT),
        ];
        for (nome, op) in letture {
            assert!(is_solo_lettura(op), "{nome} (0x{op:02X}) deve restare leggibile");
        }
    }

    /// Gli opcode non mappati (0x0B..=0x13) non sono "forse letture": sono
    /// sconosciuti, e il firmware puo' assegnargli un significato di
    /// scrittura. La sonda deve lasciarli fuori.
    #[test]
    fn gli_opcode_non_mappati_non_vengono_inviati() {
        for op in 0x0B..=0x13u8 {
            assert!(!is_solo_lettura(op), "0x{op:02X} non e' mappato: non inviabile");
        }
    }

    /// Il piano e' quello annunciato dal nome: solo letture, in ordine,
    /// senza buchi introdotti dal filtro.
    #[test]
    fn il_piano_contiene_solo_letture_ed_e_ordinato() {
        let piano = probe_plan(0x00, 0x24);
        for op in &piano {
            assert!(is_solo_lettura(*op), "0x{op:02X} nel piano ma non in sola lettura");
        }
        let mut ordinato = piano.clone();
        ordinato.sort_unstable();
        assert_eq!(piano, ordinato, "il piano deve restare in ordine crescente");
        assert_eq!(piano.len(), 16, "0x00..=0x0A (11) + 0x1B + 0x1F + 0x22..=0x24 (3)");
    }

    /// Un range vuoto o invertito non deve mandare nulla: `0x1E..=0x14`
    /// produce una lista vuota e non deve "avvolgere" fino a 0xFF.
    #[test]
    fn un_range_invertito_non_manda_nulla() {
        assert!(probe_plan(0x1E, 0x14).is_empty());
        assert!(probe_plan(0x20, 0x20).is_empty());
    }
}

