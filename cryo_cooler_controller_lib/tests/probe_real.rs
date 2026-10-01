//! Test di integrazione REALE: enumera le porte COM presenti sulla macchina e
//! interroga ciascuna con `HEART_BEAT` per capire se risponde come TEC.
//!
//! Non è un test automatico (richiede l'hardware collegato): serve come
//! strumento di diagnostica. Eseguire con:
//!
//! ```text
//! cargo test --test probe_real -- --nocapture --ignored
//! ```

use cryo_cooler_controller_lib::Tec;
use serial::SerialPort;

#[test]
#[ignore = "richiede il controller TEC collegato"]
fn elenca_e_interroga_tutte_le_com() {
    // L'enumerazione la fa `serialport` (non `serial`, che non la espone).
    // Lo usiamo solo per i nomi: l'apertura resta su `serial`, che è il
    // driver di produzione, così il test prov davvero lo stesso percorso.
    let nomi: Vec<String> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .filter(|n| n.to_uppercase().starts_with("COM"))
        .collect();
    println!(" Porte COM trovate: {}", nomi.len());

    if nomi.is_empty() {
        println!(" Nessuna porta seriale presente.");
        return;
    }

    let mut trovata = None;
    for nome in &nomi {
        let nome = nome.as_str();
        let apertura = serial::open(nome);
        match apertura {
            Err(e) => {
                println!("  {nome:<8} -> apertura FALLITA: {e}");
            }
            Ok(mut port) => {
                if let Err(e) = port.reconfigure(&|s: &mut dyn serial::SerialPortSettings| {
                    s.set_baud_rate(serial::Baud115200)?;
                    s.set_char_size(serial::Bits8);
                    s.set_stop_bits(serial::StopBits::Stop1);
                    s.set_parity(serial::ParityNone);
                    s.set_flow_control(serial::FlowControl::FlowNone);
                    Ok(())
                }) {
                    println!("  {nome:<8} -> reconfig FALLITO: {e}");
                    continue;
                }
                // Timeout generoso: qui non c'è UI da congelare, e il device
                // può essere lento al primo risveglio.
                let _ = port.set_timeout(std::time::Duration::from_millis(500));

                let mut tec = Tec::from_open_port(port);
                match tec.hear_beat() {
                    Ok(status) => {
                        println!("  {nome:<8} -> *** RISPONDE come TEC *** status={status:?}");
                        trovata = Some(nome.clone());
                    }
                    Err(e) => println!("  {nome:<8} -> non TEC ({e})"),
                }
            }
        }
    }

    match trovata {
        Some(n) => println!("\nPORTA TEC: {n}"),
        None => println!("\nNessuna porta ha risposto come TEC."),
    }
}

#[test]
#[ignore = "read-only controller diagnosis"]
fn read_connected_gen1() {
    let mut tec = Tec::new(&"COM5").expect("open COM5");
    println!("FW={:?} HW={:?}", tec.fw_version(), tec.hw_version());
    println!("STATUS={:?}", tec.hear_beat_completo());
    println!("PID={:?}/{:?}/{:?}", tec.p_coefficient(), tec.i_coefficient(), tec.d_coefficient());
    match tec.monitor() {
        Ok(d) => println!("cold={} board={} dew={} RH={} V={} A={} W={} level={}%", d.tec_temperature,d.pcb_temperature,d.dew_point_temperature,d.humidity,d.tec_voltage,d.tec_current,d.tec_power_watts,d.tec_power_level),
        Err(e) => println!("MONITOR ERROR={e}"),
    }
}

#[test]
#[ignore = "controlled activation check; controller Gen1 on COM5"]
fn controlled_enable_gen1() {
    struct Guard(Tec);
    impl Drop for Guard { fn drop(&mut self) { println!("FINAL DISABLE={:?}", self.0.disable()); } }
    let mut g = Guard(Tec::new(&"COM5").expect("COM5"));
    println!("ENABLE={:?}", g.0.enable(100.0, 1.0, 0.0, 30, -3.0));
    println!("PID READBACK={:?}/{:?}/{:?}", g.0.p_coefficient(),g.0.i_coefficient(),g.0.d_coefficient());
    for _ in 0..12 {
        println!("STATUS={:?}", g.0.hear_beat_completo());
        match g.0.monitor() {
            Ok(d) => {
                println!("cold={} board={} dew={} V={} A={} W={} level={}",d.tec_temperature,d.pcb_temperature,d.dew_point_temperature,d.tec_voltage,d.tec_current,d.tec_power_watts,d.tec_power_level);
                if !d.pcb_temperature.is_finite() || d.pcb_temperature>=36.0 || !d.condensation_margin.is_finite() || d.condensation_margin<2.0 { break; }
            }
            Err(e) => { println!("MONITOR={e}"); break; }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

#[test]
#[ignore = "Gen1 bounded setpoint response check on COM5"]
fn verify_gen1_setpoint_response() {
    struct Guard(Tec);
    impl Drop for Guard { fn drop(&mut self) { println!("DISABLE={:?}",self.0.disable()); } }
    let mut g=Guard(Tec::new(&"COM5").expect("COM5"));
    println!("ENABLE={:?}",g.0.enable(100.0,1.0,0.0,30,2.0));
    for offset in [2.0, 10.0, 20.0] {
        println!("SETPOINT {offset}: {:?}",g.0.set_setpoint_offset(offset));
        for _ in 0..3 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _=g.0.hear_beat();
            let d=g.0.monitor().expect("monitor");
            println!("offset={offset} cold={} board={} watts={} duty={}",d.tec_temperature,d.pcb_temperature,d.tec_power_watts,d.tec_power_level);
            if d.pcb_temperature>=36.0 || d.condensation_margin<2.0 { return; }
        }
    }
}
