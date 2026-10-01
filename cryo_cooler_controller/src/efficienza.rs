//! Quanto freddo compra ogni watt in piu'.
//!
//! **Il calcolo che si puo' fare con una sola sonda.** Il COP vero di un
//! modulo TEC vuole tre parametri del costruttore (il fattore Seebeck, la
//! resistenza interna, la conduttanza termica) e non li abbiamo. Ma abbiamo
//! **due temperature**: la piastra e la scheda, che e' sul lato caldo.
//!
//! Quindi non si calcola il COP, si calcola una **efficienza di sistema**:
//!
//! ```text
//!   DT  = T_scheda - T_piastra        [gradi]
//!   E   = DT / P                      [gradi per watt]
//! ```
//!
//! E soprattutto la sua **derivata marginale**: quanto `DT` guadagni per ogni
//! watt in piu'. E' la grandezza che risponde alla domanda vera, cioe'
//! "sto ancora pagando per il freddo, o sto bruciando watt?".
//!
//! Il perche' della derivata, con un esempio. Se da 100 W a 150 W il `DT`
//! passa da 10 a 12 gradi, hai comprato 2 gradi con 50 watt: 0.04 gradi per
//! watt, e quei watt li stai ancora spendendo bene. Se invece da 100 a 150 W
//! il `DT` passa da 10 a 10.1, hai comprato 0.1 gradi con 50 watt: stai
//! bruciando corrente per niente, e il punto giusto era 100 W.
//!
//! Il valore assoluto di `E` non e' confrontabile con nulla, perche' dipende
//! dalla macchina. Il **confronto nel tempo** sulla stessa macchina e' tutto
//! quello che serve, ed e' quello che questo modulo fa.

/// La differenza di temperatura attraverso il complessio, in gradi.
///
/// E' il numeratore dell'efficienza: quanto freddo ha vinto il lato caldo
/// rispetto al lato freddo. Sale quando il modulo sta lavorando bene.
///
/// Se i due valori non sono numeri, o se la scheda e' **piu' fredda** della
/// piastra, il segno e' invertito o il montaggio e' sbagliato: in quel caso si
/// restituisce `None` invece di propagare un numero che non significa niente.
pub fn delta_t(scheda: f32, piastra: f32) -> Option<f32> {
    if !scheda.is_finite() || !piastra.is_finite() {
        return None;
    }
    let d = scheda - piastra;
    // La scheda non puo' stare sotto la piastra in un complessio TEC
    // funzionante: significherebbe che il lato caldo e' quello freddo, e ogni
    // rapporto ricavato da li' sarebbe negativo e senza significato.
    if d < 0.0 {
        return None;
    }
    Some(d)
}

/// L'efficienza di sistema: gradi vinti per watt.
///
/// `None` se il delta non e' utilizzabile o la potenza non e' positiva.
pub fn efficienza(delta_t: Option<f32>, potenza_w: f32) -> Option<f32> {
    let d = delta_t?;
    if !potenza_w.is_finite() || potenza_w <= 0.0 {
        return None;
    }
    Some(d / potenza_w)
}

/// Quanto guadagni in gradi per ogni watt **in piu'**.
///
/// `prima` e `dopo` sono due istantanee della stessa macchina. Serve il
/// **delta di delta**, non il valore assoluto: e' la differenza di efficienza
/// che dice se gli ultimi watt comprati hanno servito.
///
/// `None` se non ci sono abbastanza dati per confrontare, o se la potenza non
/// e' aumentata: diminuire la potenza non e' "rendere di piu' per watt", e
/// trattarlo come tale produrrebbe un numero senza senso.
pub fn rendimento_marginale(
    prima: (f32, f32),
    dopo: (f32, f32),
) -> Option<f32> {
    let (d1, p1) = prima;
    let (d2, p2) = dopo;
    if !d1.is_finite() || !d2.is_finite() || !p1.is_finite() || !p2.is_finite() {
        return None;
    }
    if p1 <= 0.0 || p2 <= 0.0 {
        return None;
    }
    let dp = p2 - p1;
    // Solo aumenti di potenza: e' un "in piu' per watt in piu'".
    if dp <= 0.0 {
        return None;
    }
    Some((d2 - d1) / dp)
}

/// Soglia sotto la quale i watt in piu' non comprano freddo.
///
/// Un centesimo di grado per watt, cioe' **100 watt in piu' che non comprano un
/// grado**. E' un criterio che si spiega in una frase e si ricorda: se ti
/// serve una regola che puoi ricordare mentre guardi la dashboard, questa e'
/// quella.
///
/// Il valore **non e' un optimum ricavato da misure**: con una sola sonda non si
/// puo' ricavare, e falsarlo sarebbeSTATE peggio che non averlo. E' una
/// soglia dichiarata, e cambiarla e' una scelta, non un calcolo. Il primo
/// valore che avevo messo, 0.1, era sbagliato per un motivo preciso: rendeva
/// "inutile" anche l'acquisto di 2 gradi con 50 watt, che è chiaramente un
/// buon affare. La regola contraddiceva il proprio esempio.
pub const RENDIMENTO_MINIMO: f32 = 0.01;

/// `true` se gli ultimi watt comprati non hanno prodotto freddo utile.
///
/// E' la domanda che l'auto-manager deve poter fare: "sto ancora comprando
/// qualcosa?". Se la risposta e' no, la potenza va ridotta.
pub fn watt_inutili(rendimento: Option<f32>) -> bool {
    match rendimento {
        // Nessun dato: **non si assume che siano inutili.** Il default e'
        // "non lo so", e in un sistema di raffreddamento "non lo so" non
        // autorizza a tagliare potenza perche' si rischia di spegnere il
        // raffreddamento mentre serve.
        None => false,
        Some(r) => r < RENDIMENTO_MINIMO,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// **I numeri del tuo controller, ricalcolati.**
    ///
    /// Scheda 33.08 °C, piastra 15.1 °C, 10.57 V e 21.48 A. Il delta e' 17.98
    /// e la potenza 227 W, quindi 0.079 gradi per watt.
    #[test]
    fn i_numeri_del_tuo_controller() {
        let d = delta_t(33.08, 15.1).expect("entrambi validi");
        assert!((d - 17.98).abs() < 0.01, "delta {d}");
        let p = 10.57 * 21.48;
        let e = efficienza(Some(d), p).expect("potenza valida");
        assert!(
            (e - 0.0792).abs() < 0.001,
            "efficienza {e:.4} gradi per watt"
        );
    }

    /// Il caso del margine di potenza comprato bene: da 100 a 150 W il delta
    /// sale di 2 gradi, quindi 0.04 gradi per watt in piu'. La soglia e' 0.1,
    /// quindi quei watt **non** sono inutili.
    #[test]
    fn watt_comprati_bene_non_sono_inutili() {
        let r = rendimento_marginale((10.0, 100.0), (12.0, 150.0)).expect("confronto valido");
        assert!((r - 0.04).abs() < 0.0001, "rendimento marginale {r}");
        assert!(!watt_inutili(Some(r)), "2 gradi per 50 watt: servo a qualcosa");
    }

    /// E il caso opposto, che e' quello che ti costava 227 W: da 100 a 150 W
    /// il delta passa da 10 a 10.1. Compri 0.1 gradi con 50 watt: stai
    /// bruciando corrente.
    #[test]
    fn watt_comprati_a_vano_sono_inutili() {
        let r = rendimento_marginale((10.0, 100.0), (10.1, 150.0)).expect("confronto valido");
        assert!((r - 0.002).abs() < 0.0001, "rendimento marginale {r}");
        assert!(watt_inutili(Some(r)), "0.1 gradi per 50 watt: spreco");
    }

    /// **Senza dati non si taglia potenza.** E' la regola che evita di
    /// spegnere il raffreddamento perche' un sensore ha letto male.
    #[test]
    fn senza_dati_non_si_taglia_potenza() {
        assert!(!watt_inutili(None));
        assert!(!watt_inutili(rendimento_marginale((10.0, 100.0), (11.0, 100.0))));
        // Potenza non aumentata: non e' "watt in piu'".
        assert_eq!(rendimento_marginale((10.0, 150.0), (10.0, 100.0)), None);
    }

    /// Un valore non numerico non diventa uno zero.
    #[test]
    fn i_valori_non_finiti_non_producono_numeri() {
        assert_eq!(delta_t(f32::NAN, 15.1), None);
        assert_eq!(delta_t(33.0, f32::NAN), None);
        assert_eq!(efficienza(None, 100.0), None);
        assert_eq!(efficienza(Some(10.0), 0.0), None);
    }

    /// Scheda piu' fredda della piastra: il montaggio e' invertito o il
    /// sensore e' rotto, e il rapporto sarebbe negativo.
    #[test]
    fn scheda_piu_fredda_della_piastra_non_e_un_dato() {
        assert_eq!(delta_t(10.0, 20.0), None);
    }
}
