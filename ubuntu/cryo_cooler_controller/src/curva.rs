//! Registratore della curva di funzionamento della cella.
//!
//! # A cosa serve
//!
//! Il punto di massimo rendimento di una cella Peltier **non coincide** con la
//! potenza massima: sopra un certo punto si spendono watt e si scalda il
//! lato caldo senza guadagno. Qual e' quel punto, pero', e' una proprieta'
//! della cella che si possiede: si misura, non si deduce dalla teoria.
//!
//! Questo modulo registra la curva reale. Ogni volta che la potenza cambia,
//! aspetta che le misure si stabilizzino e scrive una riga CSV con
//! tensione, corrente, watt, temperature e COP stimato.
//!
//! # Cosa NON fa
//!
//! Non tocca la potenza, non chiama il TEC, non parte da solo. Registra
//! quello che il programma sta **gia'** facendo, quindi non puo' cambiare
//! il comportamento termico: e' uno strumento di misura, non un regolatore.
//!
//! La logica di protezione e il piano anticondensa non leggono nulla da
//! qui, quindi nessuna protezione dipende da questo file.

use std::io::Write;

/// Una riga della curva, pronta per il CSV.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Punto {
    /// Potenza impostata, 0-100.
    pub power_level: u8,
    /// Tensione ai capi della cella.
    pub volt:         f32,
    /// Corrente assorbita.
    pub amp:          f32,
    /// Tensione x corrente.
    pub watt:         f32,
    /// Lato freddo (piastra).
    pub tec_temp:     f32,
    /// Lato caldo del modulo: e' lui che fissa il limite del COP.
    pub ctrl_temp:    f32,
    /// Calore stimato sottratto, in watt.
    pub q_removed:    f32,
    /// COP stimato: q_removed / watt.
    pub cop:          f32,
    /// Coefficiente termico della CPU usato per stimare `q_removed`.
    ///
    /// **E' una stima, non una misura.** Il valore reale cambia da CPU a CPU
    /// e con la temperature, e per questo il COP qui e' indicativo: serve a
    /// confrontare potenze diverse fra loro, non a dichiarare un rendimento.
    pub k_cpu:        f32,
    /// Differenza fra lato caldo e lato freddo.
    pub delta_hot_cold: f32,
}

impl Punto {
    /// COP stimato, con protezione contro la divisione per zero.
    pub fn cop(&self) -> f32 {
        if self.watt > 1.0 {
            (self.q_removed / self.watt).clamp(0.0, 5.0)
        } else {
            0.0
        }
    }

    /// **Quanti watt servono per ogni watt di calore spostato.** Piu' e'
    /// basso, meglio e' il punto di lavoro.
    ///
    /// E' la metrica che rende confrontabili potenze diverse, perche' il COP
    /// da solo premia i punti con potenza quasi zero, che non raffreddano
    /// nulla ma sembrano i piu' "efficienti".
    ///
    /// Usata dall'analisi offline del CSV: e' uno strumento di lettura, non
    /// fa parte del funzionamento a runtime.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn watt_per_watt(&self) -> f32 {
        if self.q_removed > 0.1 {
            self.watt / self.q_removed
        } else {
            f32::INFINITY
        }
    }
}

/// Sceglie il **miglior** punto di funzionamento fra quelli registrati.
///
/// Il criterio non e' il COP piu' alto in assoluto, ma il **minimo watt per
/// watt di calore utile**, fra i punti che effettivamente raffreddano.
///
/// Il vincolo `cop_min` e' quello che rende la scelta utile: un punto con
/// COP bassissimo ma ottimo COP e potenza quasi zero "vince" la classifica e
/// non raffredda niente. Sopra la soglia, il punto da preferire e' quello che
/// consuma meno a parita' di efficienza.
#[cfg_attr(not(test), allow(dead_code))]
pub fn migliore_punto(punti: &[Punto], cop_min: f32) -> Option<&Punto> {
    punti
        .iter()
        .filter(|p| p.cop() >= cop_min)
        .min_by(|a, b| {
            a.watt_per_watt()
                .partial_cmp(&b.watt_per_watt())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Rende il punto leggibile in una riga CSV.
pub fn riga_csv(p: &Punto) -> String {
    format!(
        "{},{:.2},{:.2},{:.1},{:.2},{:.2},{:.1},{:.3},{:.2},{:.2}",
        p.power_level,
        p.volt,
        p.amp,
        p.watt,
        p.tec_temp,
        p.ctrl_temp,
        p.q_removed,
        p.cop(),
        p.k_cpu,
        p.delta_hot_cold,
    )
}

/// Intestazione del CSV, allineata a `riga_csv`.
pub fn intestazione_csv() -> &'static str {
    "power_level,volt,amp,watt,tec_temp,ctrl_temp,q_removed,cop,k_cpu,delta_hot_cold"
}

/// Scrive una riga nel file della curva, creando l'intestazione se serve.
///
/// Ritorna `false` senza panico se il file non e' scrivibile: la diagnostica
/// non deve mai far cadere l'applicazione, perche' siamo qui per capire
/// il comportamento del TEC e non per impedirgli di funzionare.
pub fn registra(path: &std::path::Path, p: &Punto) -> bool {
    let nuovo = !path.exists();
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return false;
    };
    if nuovo {
        let _ = writeln!(f, "{}", intestazione_csv());
    }
    let _ = writeln!(f, "{}", riga_csv(p));
    let _ = f.flush();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn punto(power: u8, watt: f32, tec: f32, ctrl: f32) -> Punto {
        // q_removed: delta_t tra CPU (40) e TEC, con k = 12 W/C
        let cpu = 40.0;
        let q = ((cpu - tec) * 12.0).max(0.0);
        Punto {
            power_level: power,
            volt: watt / 3.0,
            amp: 3.0,
            watt,
            tec_temp: tec,
            ctrl_temp: ctrl,
            q_removed: q,
            cop: 0.0,
            k_cpu: 12.0,
            delta_hot_cold: ctrl - tec,
        }
    }

    #[test]
    fn il_cop_e_il_rapporto_giusto() {
        let p = punto(50, 60.0, 10.0, 40.0);
        // q = (40-10)*12 = 360 W, watt = 60 -> cop = 6, ma limitato a 5.
        assert_eq!(p.cop(), 5.0);
        assert!((p.watt_per_watt() - 60.0 / 360.0).abs() < 0.001);
    }

    #[test]
    fn watt_basso_non_dividi_per_zero() {
        // Potenza zero: il COP e' 0 perche' non si sposta calore in assoluto,
        // e watt_per_watt e' infinito perche' il rapporto non e' definito.
        let spento = punto(0, 0.0, 40.0, 40.0);
        assert_eq!(spento.cop(), 0.0);
        assert!(spento.watt_per_watt().is_infinite());

        // Potenza bassa ma calore spostato positivo: il rapporto e' un numero
        // piccolissimo, non un errore.
        let piano = punto(5, 2.0, 10.0, 40.0);
        assert!(piano.watt_per_watt() < 0.01);
    }

    /// Il punto migliore e' quello che sposta piu' calore per watt consumato,
    /// non quello con il COP piu' alto in assoluto.
    #[test]
    fn il_migliore_e_quello_piu_efficiente() {
        // 30%: poco watt, COP medio. 70%: COP alto ma molti watt.
        let a = punto(30, 40.0, 18.0, 38.0);   // q = 22*12 = 264, cop ~6.6 -> 5
        let b = punto(70, 90.0, 5.0, 40.0);    // q = 35*12 = 420, cop ~4.6
        let punti = [a, b];
        let migliore = migliore_punto(&punti, 0.5).expect("deve esserci un punto");
        // watt/watt: a = 40/264 = 0.15, b = 90/420 = 0.21 -> vince il 30%.
        assert_eq!(migliore.power_level, 30);
    }

    /// Un punto che non raffredda (COP sotto soglia) **non** deve vincere
    /// neanche se consuma pochissimo.
    #[test]
    fn un_punto_inutile_non_vince() {
        // Potenza quasi zero, COP 0.1: inefficientissimo ma "economico".
        let inutile = punto(2, 1.0, 39.0, 45.0);  // q = 12, watt 1 -> cop 5 (cap)
        let utile = punto(60, 70.0, 8.0, 40.0);    // q = 384, cop ~5
        // Con soglia alta, quello a COP 0.1 e' escluso.
        let scarso = Punto { cop: 0.0, ..inutile };
        let punti = [scarso, utile];
        let migliore = migliore_punto(&punti, 1.0).expect("deve esserci un punto");
        assert_eq!(migliore.power_level, 60);
    }

    #[test]
    fn nessun_punto_sopra_la_soglia() {
        let punti = [punto(10, 100.0, 30.0, 45.0)];  // q = 120, cop ~1.2
        assert!(migliore_punto(&punti, 4.0).is_none());
    }

    #[test]
    fn lista_vuota_non_panica() {
        assert!(migliore_punto(&[], 1.0).is_none());
    }

    /// Il CSV deve avere tanti campi quanti l'intestazione, altrimenti la
    /// tabella non e' leggibile con un foglio di calcolo.
    #[test]
    fn il_csv_e_allineato_alla_intestazione() {
        let p = punto(50, 60.0, 10.0, 40.0);
        let campi = riga_csv(&p).split(',').count();
        let attesi = intestazione_csv().split(',').count();
        assert_eq!(campi, attesi, "riga e intestazione non hanno gli stessi campi");
    }
}
