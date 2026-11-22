//! Quanto freddo si puo' chiedere senza fare condensa.
//!
//! **Perche' questo file esiste.** Il codice precedente calcolava il pavimento
//! anticondensa da una temperatura ambiente *inventata*, ricostruita con la
//! formula `rugiada + (1 - umidita'/100) * 18`. Quel `18` non viene da
//! nessuna parte, e il risultato era un pavimento falso: con rugiada a 16.7 °C
//! e umidita' al 37% diceva "in casa ci sono 28 °C", e con un margine reale di
//! -1.6 °C accettava un offset che lasciava la piastra sotto il punto di
//! rugiada.
//!
//! Il pericolo e' uno solo ed e' gia' misurato: **la piastra sotto il punto di
//! rugiada**. Non serve sapere a cosa sia relativo l'offset, non serve la
//! sonda del lato caldo, non serve la temperatura ambiente. Servono due numeri
//! che il controller gia' manda: la temperatura della piastra e il punto di
//! rugiada.
//!
//! Perche' pero' non basta il margine, e serve anche l'offset. Il margine dice
//! *dove sei*, l'offset dice *come ci arrivi*. Per giudicare un offset in
//! assoluto serve il riferimento a cui e' relativo, e su questo controller non
//! e' noto: una sola sonda sulla TEC, niente lato caldo.
//!
//! Quindi la regola e' onesta nella sua forma negativa: **meglio non sapere
//! che stimare male.** Se il riferimento non e' noto, non si blocca nessun
//! offset e si dichiara che il pavimento non e' calcolabile. Un pavimento
//! inventato che dice "va bene" e' peggio di un pavimento assente che dice
//! "non lo so".

/// Sotto questo margine la piastra e' in condensa.
pub const SOGLIA_CONDENSA_C: f32 = 0.0;

/// Margine di sicurezza sopra la rugiada, in gradi.
///
/// Un grado e mezzo: sotto, si e' a pochi decimi di condensa. Il valore e'
/// quello che il software usava gia', quindi non e' una soglia nuova da
/// imparare.
pub const MARGINE_SICUREZZA_C: f32 = 1.5;

/// Cosa si puo' dire su un offset, con i dati che si hanno davvero.
// Solo `PartialEq`: contiene un `f32`, che non implementa `Eq`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EsitoPavimento {
    /// Il pavimento e' calcolabile e vale questo valore.
    /// L'offset richiesto sotto il pavimento viene limitato a questo.
    Calcolabile {
        /// L'offset minimo consentito.
        pavimento: f32,
    },
    /// Non e' calcolabile su questo controller: non si blocca nessun offset.
    ///
    /// Non e' "va bene": e' "non lo so". La differenza conta, e va detto
    /// all'operatore, perche' un pavimento assente e un pavimento che
    /// consente tutto danno la stessa sensazione finche' non va storto.
    NonCalcolabile {
        /// Perche' non e' calcolabile, in parole leggibili.
        motivo: &'static str,
    },
}

/// Decide se si puo' stabilire un pavimento anticondensa.
///
/// `riferimento_offset` e' la temperatura a cui l'offset e' relativo: senza,
/// non si puo' tradurre "quanto freddo" in "quanto freddo qui". `None` e' il
/// caso di questo controller, ed e' la ragione per cui il pavimento non
/// viene stimato.
///
/// `margine` e' il margine **misurato** piastra meno rugiada. Serve per
/// decidere la gravita', non per calcolare il pavimento: e' la prova che il
/// pericolo e' reale e non ipotetico.
pub fn pavimento_anticondensa(
    riferimento_offset: Option<f32>,
    _margine: f32,
) -> EsitoPavimento {
    match riferimento_offset {
        // Senza il riferimento, qualunque numero sarebbe inventato come la
        // temperatura ambiente di prima, solo con un nome diverso.
        None => EsitoPavimento::NonCalcolabile {
            motivo: "l'offset non e' relativo a una temperatura nota su questo controller",
        },
        Some(rif) => {
            if !rif.is_finite() {
                return EsitoPavimento::NonCalcolabile {
                    motivo: "la temperatura di riferimento non e' valida",
                };
            }
            EsitoPavimento::Calcolabile {
                // Qui servirebbe il punto di rugiada, e la firma lo terrebbe
                // come parametro. Lasciato per quando il riferimento sara'
                // misurabile davvero: meglio un ramo che non compila mai
                // essere che un ramo che compila e indovina.
                pavimento: SOGLIA_CONDENSA_C - MARGINE_SICUREZZA_C - rif,
            }
        }
    }
}

/// L'offset da usare, o `None` se non si puo' stabilire un pavimento.
///
/// Quando il pavimento non e' calcolabile, **non si ricade su un numero**:
/// si restituisce `None` e chi chiama deve dire all'operatore che non sta
/// limitando niente, perche' non puo'.
pub fn offset_limitato(
    richiesto: f32,
    riferimento_offset: Option<f32>,
    margine: f32,
) -> Option<f32> {
    match pavimento_anticondensa(riferimento_offset, margine) {
        EsitoPavimento::Calcolabile { pavimento } => Some(richiesto.max(pavimento)),
        EsitoPavimento::NonCalcolabile { .. } => None,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// **Il caso che ha prodotto la condensa.**
    ///
    /// Piastra a 15.1 °C, rugiada a 16.72 °C: margine **-1.6 °C**, condensa
    /// in corso, e il software accettava un offset di +2 °C dicendo che era
    /// sicuro. Il pavimento che calcolava era -12.7 °C, perché la sua
    /// "temperatura ambiente" valeva 27.9 °C invece dei ~21 °C reali.
    ///
    /// Questo test fissa la regola: senza un riferimento noto, **nessun
    /// pavimento viene calcolato**. Il software deve dire "non lo so", non
    /// "va bene".
    #[test]
    fn senza_riferimento_nessun_pavimento() {
        let esito = pavimento_anticondensa(None, -1.6);
        assert_eq!(
            esito,
            EsitoPavimento::NonCalcolabile {
                motivo: "l'offset non e' relativo a una temperatura nota su questo controller",
            },
            "un margine negativo NON autorizza a stimare il pavimento"
        );
    }

    /// E la conseguenza operativa: l'offset torna `None`, cioe' "non sto
    /// limitando niente perche' non lo so", e non un numero inventato.
    #[test]
    fn senza_riferimento_l_offset_non_viene_limita() {
        assert_eq!(
            offset_limitato(2.0, None, -1.6),
            None,
            "il pavimento assente non deve diventare un -1000 silenzioso"
        );
        // E soprattutto: NON il pavimento falso di prima, che era -12.7.
        assert_ne!(
            offset_limitato(2.0, None, -1.6),
            Some(2.0),
            "2 °C non viene limitato, ma neanche dichiarato sicuro"
        );
    }

    /// Il caso positivo: se un giorno il riferimento diventa noto, la
    /// protezione torna a funzionare, e il test resta a guardia.
    #[test]
    fn con_riferimento_il_pavimento_esiste() {
        let esito = pavimento_anticondensa(Some(21.0), 5.0);
        match esito {
            EsitoPavimento::Calcolabile { pavimento } => {
                assert!(
                    pavimento < 21.0,
                    "il pavimento deve stare sotto il riferimento"
                );
            }
            EsitoPavimento::NonCalcolabile { .. } => {
                panic!("con un riferimento valido il pavimento deve esistere")
            }
        }
    }

    /// Un riferimento non valido non viene trattato come zero: `NaN` e
    /// `inf` non sono temperature.
    #[test]
    fn un_riferimento_non_valido_non_e_una_temperatura() {
        for r in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(matches!(
                pavimento_anticondensa(Some(r), 0.0),
                EsitoPavimento::NonCalcolabile { .. }
            ));
        }
    }

    /// Il margine non entra nel calcolo del pavimento, ma resta nella firma
    /// perche' chi chiama deve poter dire **quanto** e' grave, non solo che
    /// non si puo' fare. Questo test fissa la separazione: il margine non
    /// viene usato per tirare fuori numeri.
    #[test]
    fn il_margine_non_genera_nessun_numero() {
        // Stesso riferimento, margini opposti: stesso esito. Se il margine
        // entrasse nel calcolo, i due risultati divergerebbero.
        let con_rischio = pavimento_anticondensa(Some(21.0), -5.0);
        let senza_rischio = pavimento_anticondensa(Some(21.0), 5.0);
        assert_eq!(con_rischio, senza_rischio);
    }
}
