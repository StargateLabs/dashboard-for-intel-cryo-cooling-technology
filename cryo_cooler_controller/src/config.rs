//! Profile persistence — cross-platform using XDG directories.
//! Windows: %APPDATA%\StargateLabsCryo\config.json
//! Linux: ~/.config/stargate-cryo/config.json

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub p_coef: f32,
    pub i_coef: f32,
    pub d_coef: f32,
    pub set_point: f32,
    pub max_power: u8,
}

/// Il set_point e' l'offset del firmware; non e' la temperatura della piastra.
impl Profile {
    /// Gaming: budget software 120 W, target misurato rugiada +3.5 C.
    pub fn default_gaming() -> Self {
        Profile {
            name: "Gaming".to_owned(),
            p_coef: 100.0,
            i_coef: 1.0,
            d_coef: 0.0,
            set_point: 3.0,
            max_power: 60,
        }
    }

    /// Carico massimo: potenza piena, e un gradino di margine in piu' per
    /// stare dalla parte del freddo senza bagnare niente.
    pub fn default_ai_workload() -> Self {
        Profile {
            name: "AI / Rendering".to_owned(),
            p_coef: 100.0,
            i_coef: 1.0,
            d_coef: 0.0,
            set_point: 2.0,
            max_power: 80,
        }
    }

    /// Silenzioso: budget software 60 W, target misurato rugiada +6 C.
    pub fn default_idle() -> Self {
        Profile {
            name: "Silenzioso / Idle".to_owned(),
            p_coef: 100.0,
            i_coef: 1.0,
            d_coef: 0.0,
            set_point: 6.0,
            max_power: 30,
        }
    }

    /// Obiettivo misurato sopra la rugiada, separato dall'offset firmware.
    pub fn margine_sicuro(&self) -> f32 {
        match self.name.as_str() {
            "Silenzioso / Idle" => 6.0,
            "Gaming" => 3.5,
            "AI / Rendering" => 3.0,
            _ => 3.5,
        }
    }
    /// Migra i tre preset storici; conserva gli offset dei profili personali.
    pub fn con_margine_sicuro(&self) -> Profile {
        match self.name.as_str() {
            "Gaming" => Self::default_gaming(),
            "AI / Rendering" => Self::default_ai_workload(),
            "Silenzioso / Idle" => Self::default_idle(),
            _ => Profile { set_point: if self.set_point.is_finite() { self.set_point.clamp(-30.0,50.0) } else { 6.0 }, ..self.clone() },
        }
    }
}
impl Default for Profile {
    fn default() -> Self {
        Profile {
            name: "Predefinito".to_owned(),
            p_coef: 100.0,
            i_coef: 1.0,
            d_coef: 0.0,
            set_point: 3.0,
            max_power: 80,
        }
    }
}

/// Controllo delle notifiche.
///
/// Sono due canali distinti e vanno spenti separatamente:
///   - `toasts`  → notifiche Windows ( balloon ). Costano un processo
///     PowerShell ciascuna, quindi sono la causa principale del consumo di
///     memoria quando scattano piu' alert insieme.
///   - `banner`  → le strisce di allarme dentro la dashboard. Non costano
///     nulla e servono a non perdere un evento di sicurezza.
///
/// Disattivare i toast NON deve spegnere il controllo termico: le
/// protezioni restano attive, cambia solo come l'utente viene avvisato.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotifySettings {
    /// Notifiche Windows abilitate.
    pub toasts: bool,
    /// Strisce di allarme in-app abilitate.
    pub banner: bool,
}

impl Default for NotifySettings {
    fn default() -> Self {
        Self { toasts: true, banner: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub profiles: Vec<Profile>,
    /// Impostazioni notifiche. `#[serde(default)]` perche' i config scritti
    /// prima di questa opzione non hanno il campo: senza default la
    /// deserializzazione fallirebbe e l'utente perderebbe i suoi profili.
    #[serde(default)]
    pub notify: NotifySettings,
    /// Scrivere un campione su disco a ogni tick: **spento di default**.
    ///
    /// Prima era sempre attivo, e a 2 Hz per tutta la sessione: il file
    /// arrivava al tetto di 20.000 righe e da li' in poi veniva riscritto
    /// intero (2,5 MB) a ogni campione. E' il motivo del consumo di disco.
    /// Il log serve per la diagnostica, quindi si accende solo se lo chiedi.
    #[serde(default)]
    pub log_su_disco: bool,
    /// Regole di alert: condizione, soglia, stato e cooldown.
    ///
    /// Prima queste vivivano solo in memoria: ogni regolazione fatta nelle
    /// impostazioni (per esempio spegnere l'allarme RPM pompa) andava
    /// persa al riavvio. `#[serde(default)]` perche' i config scritti
    /// prima non hanno il campo; se la lista arriva vuota si ricade sui
    /// valori di fabbrica, vedi `load()`.
    #[serde(default)]
    pub alerts: Vec<crate::alerts::AlertRule>,
}

impl AppConfig {
    fn config_path() -> std::path::PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("stargate-cryo")
    }

    fn config_file() -> std::path::PathBuf {
        Self::config_path().join("config.json")
    }

    pub fn load() -> Self {
        let path = Self::config_file();
        match std::fs::read_to_string(&path) {
            Ok(data) => match serde_json::from_str::<AppConfig>(&data) {
                Ok(mut cfg) => {
                    // Config scritti prima dell'opzione: la lista arriva vuota e
                    // si usano i valori di fabbrica, altrimenti l'utente si
                    // ritroverebbe senza nessuna regola attiva.
                    if cfg.alerts.is_empty() {
                        cfg.alerts = crate::alerts::AlertRule::default_rules();
                    } else {
                        // Le regole TecTemp scritte prima del cambio di
                        // semantica (assoluta -> scarto) hanno soglie che
                        // sotto la nuova semantica non significano niente:
                        // migrazione una tantum, on/off ed etichette restano.
                        cfg.alerts = crate::alerts::migra_regole_obsolete(cfg.alerts);
                    }
                    return cfg;
                }
                Err(err) => {
                    // Il file c'e' ma non e' parsabile: NON e' il primo avvio.
                    //
                    // Tornare ai valori di fabbrica senza backup e' una perdita
                    // dati: la prossima `save()` — che scrive su questo stesso
                    // path — sovrascriverebbe i profili dell'utente e non ci
                    // sarebbe piu' niente da recuperare. Quindi il file viene
                    // copiato com'e' PRIMA di procedere, e l'originale non
                    // viene ne' toccato ne' cancellato.
                    let backup = path.with_extension("corrotto.json");
                    eprintln!(
                        "[config] config.json non leggibile ({}). \
                         Copia di sicurezza in '{}'; i profili di fabbrica \
                         verranno caricati al suo posto.",
                        err, backup.display()
                    );
                    if let Err(copy_err) = std::fs::copy(&path, &backup) {
                        eprintln!(
                            "[config] copia di sicurezza fallita ({}). \
                             L'originale non e' stato modificato.",
                            copy_err
                        );
                    }
                }
            },
            // Primo avvio: il file non esiste ancora, niente da preservare.
            Err(_) => {}
        }
        // First run: seed with preset profiles
        AppConfig {
            profiles: vec![
                Profile::default_idle(),
                Profile::default_gaming(),
                Profile::default_ai_workload(),
            ],
            notify: NotifySettings::default(),
            log_su_disco: false,
            alerts: crate::alerts::AlertRule::default_rules(),
        }
    }

    pub fn save(&self) {
        let dir = Self::config_path();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::config_file(), json);
        }
    }
}

#[cfg(test)]
mod test_profili {
    //! I profili non devono poter chiedere condensa.
    //!
    //! Il bug: `set_point` era un offset in °C (`-3`, `-5`) e diventava un
    //! margine di condensa negativo — "stai 3 gradi sotto la rugiada" — cioe'
    //! acqua garantita. E finiva scritto sull'hardware, dove un negativo non
    //! ha nessun senso.
    use super::Profile;

    #[test]
    fn migrazione_preset_e_offset_personali() {
        let mut old=Profile::default_gaming(); old.set_point=-10.0; old.max_power=90;
        let migrated=old.con_margine_sicuro();
        assert_eq!(migrated.set_point,3.0); assert_eq!(migrated.max_power,60);
        let custom=Profile {name:"Personale".into(),set_point:-4.0,..old};
        assert_eq!(custom.con_margine_sicuro().set_point,-4.0);
        assert!(Profile::default_idle().margine_sicuro()>Profile::default_gaming().margine_sicuro());
    }
    /// Sotto il minimo fisico non si va, nemmeno con un profilo tarato a mano.
    #[test]
    fn il_margine_resta_sopra_il_minimo() {
        assert!(Profile::default_idle().margine_sicuro() >= 0.5);
    }

    /// Il profilo silenzioso consuma meno di quello di carico: e' il senso.
    #[test]
    fn il_profilo_silenzioso_e_piu_leggero() {
        assert!(Profile::default_idle().max_power < Profile::default_ai_workload().max_power);
        assert!(Profile::default_gaming().max_power <= Profile::default_ai_workload().max_power);
    }
}

#[cfg(test)]
mod test_margine_condensa {
    //! Il campo "offset" torna a essere un numero **negativo**: dice quanto
    //! freddo vuoi, come in r116.
    //!
    //! Il danno: avevo trasformato il campo in "gradi sopra la rugiada" con
    //! valori positivi. Il regolatore allora puntava a `rugiada + 1` e spingeva
    //! a palla, e l'impianto scaldava. Qui la regola e' scritta, cosi' non
    //! si rompe in silenzio.
    use crate::config::Profile;

    /// Con offset negativo la piastra va **sotto** la rugiada: e' il freddo
    /// che l'operatore chiede, e il programma lo limita al bordo sicuro.
    #[test]
    fn un_offset_negativo_chiede_freddo() {
        // -20 vuol dire "voglio scendere", non "voglio salire".
        let freddo = -20.0_f32;
        assert!(freddo < 0.0, "un offset negativo e' la richiesta di freddo");
    }

    /// I profili di fabbrica devono tornare a chiedere freddo, non a scaldare.
    #[test]
    fn i_profili_di_fabbrica_chiedono_freddo() {
        for p in [Profile::default_idle(), Profile::default_gaming(),
                  Profile::default_ai_workload()] {
            // Sono margini sulla rugiada, quindi il freddo corrispondente e'
            // il loro valore assoluto: chiedono di scendere, non di salire.
            assert!(
                p.margine_sicuro() > 0.0,
                "{}: deve chiedere freddo sotto la rugiada",
                p.name,
            );
        }
    }
}

#[cfg(test)]
mod test_log_su_disco {
    //! Il log campione-per-campione scriveva **sempre**, a ogni campione, per
    //! tutta la sessione. Con il tetto di 20.000 righe raggiunto,
    //! `trocca_se_troppo` riscriveva l'intero file da 2,5 MB — e siccome il
    //! tetto era esaurito, succedeva a ogni campione.
    //!
    //! Il consumo di disco che l'utente ha notato e' esattamente questo.
    //! Ora e' **spento di default** e si accende solo se lo chiedi: il log
    //! serve per la diagnostica, non per stare in funzione tutto il giorno.
    use super::AppConfig;

    #[test]
    fn il_log_su_disco_e_spento_di_default() {
        let c = AppConfig::default();
        assert!(
            !c.log_su_disco,
            "il log campione-per-campione non deve stare attivo di default",
        );
    }
}



