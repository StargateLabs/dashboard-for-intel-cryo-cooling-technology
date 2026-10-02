#![cfg_attr(windows, windows_subsystem = "windows")]
// NOTE: forbid(unsafe_code) rimosso per permettere hwinfo.rs (Windows shared memory API)
#![warn(
    clippy::dbg_macro,
    clippy::decimal_literal_representation,
    clippy::panic,
    clippy::panic_in_result_fn,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::todo,
    clippy::unimplemented,
    clippy::unwrap_in_result,
    clippy::unwrap_used,
    clippy::use_debug
)]

extern crate iced;
extern crate plotters;

mod modalita;
mod condensa;
mod efficienza;
mod tec_optimizer;
mod certezza;
mod curva;
mod stato_log;
mod attore_tec;
#[allow(unused)]
mod serial_probe;
mod single_instance;
mod recovery;

#[allow(unused)]
mod autostart;

mod charts;
mod chart_preview;
mod condensation_badge;
mod automanager;
mod cpuload;
mod pumpwatch;
mod commissioning;
mod config;
mod ai_advisor;
mod session_db;
mod report_pdf;
mod pid_wizard;
mod overlay;
mod analytics;
mod alerts;
mod tecdiag;
mod diagnostica;
mod commutazione;
mod hwinfo;
mod running;
mod sensors_panel;
mod session_stats;

use iced::{
    alignment,
    widget::{Column, Container, Row, Text},
    Element, Length, Subscription, Task, Theme,
};

use running::RunningState;
use std::time::Duration;
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIconBuilder,
};

const ICON: &[u8; 0x4000] = include_bytes!(concat!(env!("OUT_DIR"), "/icon.bin"));

pub const LOGO_BANNER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/logo_banner.bin"));
pub const LOGO_W: u32 = 480;
pub const LOGO_H: u32 = 305;

/// Logo cryogenic, mostrato quando la piastra scende sotto i 20 °C.
///
/// Sorgente: `CRYOGENICLOGO.png`, **631x405**, gia' ritagliato dall'utente
/// sulla grafica: non c'e' piu' cornice bianca.
///
/// Le dimensioni sono quelle del file (631x405, risoluzione piena) e
/// devono coincidere con `cryo_warning.png` e con l'assert in `build.rs`:
/// due numeri diversi per lo stesso file stirano l'immagine. A schermo
/// viene ridotto dalla GPU in un solo passaggio, che mantiene i bordi
/// netti; pre-ridurlo a mano significa ricampionare due volte e il logo
/// esce sgranato.
pub const CRYO_WARN_BANNER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cryo_warning.bin"));
pub const CRYO_WARN_W: u32 = 631;
pub const CRYO_WARN_H: u32 = 405;

/// Marchio mostrato nell'hero, nello spazio fra il titolo e la card dei dati.
///
/// Le dimensioni sono quelle prodotte da `build.rs` (336x336, forzato
/// quadrato dal ritaglio del contenuto): devono coincidere con
/// `BRAND_W`/`BRAND_H` e con `BRAND_RAPPORTO` in `running.rs`.
pub const BRAND_BANNER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/hero_brand.bin"));
pub const BRAND_W: u32 = 336;
pub const BRAND_H: u32 = 336;

pub const CRYO_ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cryo_icon.bin"));
pub const OCP_ICON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ocp_icon.bin"));

/// Sfondo della dashboard, con alpha gia' cotta in build.
pub const DASHBOARD_BG: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/dashboard_bg.bin"));
/// Dimensione dello sfondo: deve combaciare con i pixel grezzi.
pub const DASHBOARD_BG_W: u32 = 768;
pub const DASHBOARD_BG_H: u32 = 1376;

/// Set di icone monocromatiche ad alta risoluzione.
///
/// Sorgente: Font Awesome Free 6.5.2, stile Solid, glifi bianchi su sfondo
/// trasparente. Attribuzione richiesta dalla licenza CC BY 4.0:
/// "Font Awesome Free 6.5.2 by Fonticons, Inc. — CC BY 4.0".
/// Le immagini sono quadrate e centrate, quindi si mostrano senza
/// deformazioni a qualsiasi dimensione.
pub mod icons {
    use iced::widget::image::Handle;
    /// Dimensione di ogni icona dopo il build: quadrata, senza deformazioni.
    pub const W: u32 = 128;
    pub const H: u32 = 128;

    macro_rules! icon {
        ($name:literal) => {
            {
                static HANDLE: std::sync::OnceLock<Handle> = std::sync::OnceLock::new();
                HANDLE.get_or_init(|| Handle::from_rgba(W, H, include_bytes!(concat!(env!("OUT_DIR"), $name)).to_vec())).clone()
            }
        };
    }

    /// Termometro: temperatura e margine termico.
    pub fn thermo() -> Handle { icon!("/icon_thermo.bin") }
    /// Fulmine: potenza elettrica istantanea.
    pub fn bolt()   -> Handle { icon!("/icon_bolt.bin") }
    /// Ventola: pompa e flusso del loop.
    pub fn fan()    -> Handle { icon!("/icon_fan.bin") }
    /// Scudo: protezioni, guardia termica e sicurezza.
    pub fn shield() -> Handle { icon!("/icon_shield.bin") }
    /// Microchip: firmware, revisione hardware e sistema.
    pub fn cpu()    -> Handle { icon!("/icon_cpu.bin") }
    /// Tachimetro: carico e percentuale comandata.
    pub fn gauge()  -> Handle { icon!("/icon_gauge.bin") }
    /// Modulo a strati: piastra e cella TEC.
    pub fn plate()  -> Handle { icon!("/icon_plate.bin") }
    /// Triangolo di allarme: OCP e condizioni critiche.
    pub fn hazard() -> Handle { icon!("/icon_hazard.bin") }
    #[cfg(test)]
    mod tests {
        #[test]
        fn repeated_frames_reuse_the_same_image_handles() {
            let icons:[fn()->super::Handle;8]=[super::thermo,super::bolt,super::fan,super::shield,super::cpu,super::gauge,super::plate,super::hazard];
            for icon in icons {
                let first=icon();
                for _ in 0..1000 { assert_eq!(first,icon()); }
            }
            assert_ne!(super::plate(),super::bolt());
        }
    }
}
pub const ICON_64: u32 = 64;



pub mod palette {
    use iced::Color;
    /// Sfondo — verde petrolio scurissimo che sposa il blu elettrico #1A6AFF
    pub const BACKGROUND:   Color = Color { r: 0.010, g: 0.072, b: 0.052, a: 1.0 };
    pub const NEON_GREEN:   Color = Color { r: 0.0,   g: 0.95,  b: 0.28,  a: 1.0 };
    pub const MATRIX_GRN:   Color = Color { r: 0.0,   g: 0.55,  b: 0.28,  a: 1.0 };
    pub const TEXT_BRIGHT:  Color = Color { r: 0.85,  g: 0.97,  b: 0.95,  a: 1.0 };
    pub const BLUE_PRIMARY: Color = Color { r: 0.12,  g: 0.45,  b: 1.0,   a: 1.0 };
    // Testo secondario: leggibile ma **chiaramente subordinato** ai pulsanti.
    //
    // Era (0.08, 0.32, 0.42), illeggibile sul fondo scuro: l'utente lo
    // segnalava come "testo scuro". Portarlo pero' a (0.62, 0.74, 0.80) e'
    // stato l'errore successivo: con la stessa luminosita' dei pulsanti il
    // pannello Impostazioni diventava un blocco celeste uniforme, senza piu'
 // distinzione fra testo e comando, e i pulsanti sparivano.
    //
    // Il livello giusto sta **fra** i due: abbastanza chiaro da leggersi,
    // abbastanza scuro da non competere con un bottone.
    pub const BLUE_DIM:     Color = Color { r: 0.46,  g: 0.58,  b: 0.63,  a: 1.0 };
    pub const DANGER:       Color = Color { r: 1.0,   g: 0.10,  b: 0.22,  a: 1.0 };
    pub const WARNING:      Color = Color { r: 1.0,   g: 0.62,  b: 0.0,   a: 1.0 };
    pub const SUCCESS:      Color = Color { r: 0.0,   g: 0.85,  b: 0.45,  a: 1.0 };

    /// Colori semantici: uno per categoria, mai riutilizzato.
    ///
    /// A 16 px un'illustrazione dettagliata non e' leggibile, quindi il
    /// colore e' l'unico segnale che resta per capire a che categoria
    /// appartiene una riga.
    pub const SEM_TEMPERATURA:  Color = Color { r: 0.20, g: 0.80, b: 0.95, a: 1.0 };
    pub const SEM_POTENZA:      Color = Color { r: 1.00, g: 0.70, b: 0.10, a: 1.0 };
    pub const SEM_CIRCOLAZIONE: Color = Color { r: 0.55, g: 0.45, b: 1.00, a: 1.0 };
    pub const SEM_PROTEZIONE:   Color = Color { r: 0.10, g: 0.90, b: 0.55, a: 1.0 };
    pub const SEM_RISCHIO:      Color = Color { r: 1.00, g: 0.30, b: 0.35, a: 1.0 };

    // ── Superfici "glass" ─────────────────────────────────────────────────
    // Usate per le card: translucide sopra lo sfondo, con bordo luminoso
    // sottile. Danno profondità senza aggiungere elementi al layout.
    /// Superficie card: quasi nera, leggermente azzurrata.
    pub const GLASS:    Color = Color { r: 0.020, g: 0.085, b: 0.075, a: 0.82 };
    /// Superficie card in evidenza (hover/stato attivo).
    pub const GLASS_HI: Color = Color { r: 0.030, g: 0.130, b: 0.110, a: 0.88 };

    /// Sfondo dell'app: gradiente diagonale, dal verde petrolio profondo in
    /// alto a quasi nero in basso. Costo zero per frame (una passata di fill).
    ///
    /// iced 0.13 espone solo `Gradient::Linear` (angolo + stop), quindi il
    /// "flare" centrale si simula con lo stop iniziale.
    pub fn app_gradient() -> iced::Gradient {
        use iced::Gradient;
        Gradient::Linear(
            iced_core::gradient::Linear::new(1.2)   // angolo in radianti: diagonale
                .add_stop(0.0, Color { r: 0.030, g: 0.140, b: 0.128, a: 1.0 })
                .add_stop(1.0, Color { r: 0.004, g: 0.028, b: 0.022, a: 1.0 }),
        )
    }

    /// Stesso gradiente di `app_gradient`, ma con alpha ridotta: e' il
    /// "vetro" che copre la foto di sfondo.
    ///
    /// Serve una funzione a parte e non un parametro, perche' `app_gradient()`
    /// e' usato anche dove serve completamente opaco (schermata iniziale,
    /// superfici a pieno campo) e non deve diventare trasparente per effetto
    /// collaterale.
    pub fn app_gradient_alpha(alpha: f32) -> iced::Gradient {
        use iced::Gradient;
        let a = alpha.clamp(0.0, 1.0);
        Gradient::Linear(
            iced_core::gradient::Linear::new(1.2)
                .add_stop(0.0, Color { r: 0.030, g: 0.140, b: 0.128, a })
                .add_stop(1.0, Color { r: 0.004, g: 0.028, b: 0.022, a }),
        )
    }

    /// Alone colorato: shadow con lo stesso colore della card, sfocata.
    /// È quello che fa sembrare "neon" un pannello piatto.
    pub fn glow(color: Color, intensity: f32) -> iced::Shadow {
        iced::Shadow {
            color: Color { r: color.r, g: color.g, b: color.b, a: intensity },
            offset: iced::Vector { x: 0.0, y: 0.0 },
            blur_radius: 14.0,
        }
    }

    /// Alone discreto per le card normali.
    pub fn soft_shadow() -> iced::Shadow {
        iced::Shadow {
            color: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.45 },
            offset: iced::Vector { x: 0.0, y: 2.0 },
            blur_radius: 8.0,
        }
    }

    /// Tinted glass for status badges; retain each signal's semantic accent.
    pub fn status_surface(accent: Color) -> iced::Gradient {
        iced::Gradient::Linear(iced_core::gradient::Linear::new(0.4)
            .add_stop(0.0,Color {r:0.018+accent.r*0.13,g:0.045+accent.g*0.13,b:0.052+accent.b*0.13,a:0.94})
            .add_stop(1.0,Color {r:0.012,g:0.035,b:0.043,a:0.88}))
    }
}

/// Stili dei pulsanti.
///
/// I pulsanti di default di iced sono rettangolari, con bordi netti e poco
/// contrasto: su una dashboard scura sembrano controlli di una GUI degli
/// anni 2000. Qui si definisce un linguaggio coerente — angoli arrotondati,
/// bordo sottile, alone leggero in hover — applicato a tutti i pulsanti
/// dell'app tramite una sostituzione dei tre stili standard.
pub mod btn {
    use iced::widget::button::{Status, Style};
    use iced::{Border, Color, Shadow, Theme};

    /// Raggio dei pulsanti: arrotondati, non rettangolari.
    const RADIUS: f32 = 9.0;

    pub fn selected_profile(theme: &Theme, status: Status) -> Style {
        let mut style=glass(theme,status);
        style.border.color=Color::from_rgb8(75,210,235);
        style.text_color=Color::from_rgb8(217,250,255);
        if !matches!(status,Status::Hovered|Status::Pressed) {
            style.background=Some(iced::Background::Color(Color::from_rgba8(18,105,122,0.72)));
        }
        style
    }

    fn base(accent: Color, bg: Color, border_a: f32) -> Style {
        Style {
            background: Some(iced::Background::Color(bg)),
            text_color: accent,
            border: Border {
                color: Color { r: accent.r, g: accent.g, b: accent.b, a: border_a },
                width: 1.0,
                radius: RADIUS.into(),
            },
            shadow: Shadow::default(),
        }
    }

    /// Azione principale: accento pieno, alone in hover.
    pub fn primary(_: &Theme, status: Status) -> Style {
        let s = base(Color::WHITE, Color { r: 0.05, g: 0.30, b: 0.40, a: 1.0 }, 0.45);
        match status {
            Status::Hovered => Style {
                shadow: crate::palette::glow(Color { r: 0.10, g: 0.55, b: 0.75, a: 1.0 }, 0.40),
                ..s
            },
            Status::Pressed => Style {
                background: Some(iced::Background::Color(Color { r: 0.04, g: 0.22, b: 0.30, a: 1.0 })),
                ..s
            },
            _ => s,
        }
    }

    /// Controllo neutro e discreto: chiusura di un avviso, azioni di
    /// secondario ordine.
    ///
    /// Serve perche' il pulsante di chiusura degli avvisi era `danger`, cioe'
    /// rosso: dentro un riquadro rosso non si distingue dal fondo, e sembrava
    /// un secondo avviso invece di un controllo. Qui il fondo e' trasparente
    /// e il colore arriva solo all'hover, cosi' l'icona di chiusura non
    /// compete con l'avviso che sta chiudendo.
    pub fn ghost(_: &Theme, status: Status) -> Style {
        let s = Style {
            background: None,
            text_color: Color { r: 0.58, g: 0.72, b: 0.75, a: 0.75 },
            border: Border { color: Color::TRANSPARENT, width: 0.0, radius: RADIUS.into() },
            shadow: Shadow::default(),
        };
        match status {
            Status::Hovered => Style {
                background: Some(iced::Background::Color(Color { r: 0.12, g: 0.22, b: 0.24, a: 0.9 })),
                text_color: Color { r: 0.88, g: 0.97, b: 0.98, a: 1.0 },
                ..s
            },
            Status::Pressed => Style {
                background: Some(iced::Background::Color(Color { r: 0.09, g: 0.17, b: 0.19, a: 0.95 })),
                ..s
            },
            _ => s,
        }
    }

    /// Azione su fondo vetro.
    ///
    /// `secondary` ha un fondo **opaco** (`a: 1.0`), e su uno sfondo che e'
    /// una fotografia quei pulsanti diventavano buchi neri: non sembravano
    /// parte dell'interfaccia, sembravano un riquadro appoggiato sopra.
    ///
    /// Qui il fondo e' traslucido, quindi la foto passa through e il
    /// pulsante appartiene alla scena; il bordo e l'alone rifiniscono il
    /// contorno. Il testo resta ad alto contrasto perche' il fondo e' quello
    /// che si schiarisce, non la scritta.
    pub fn glass(_: &Theme, status: Status) -> Style {
        let s = Style {
            background: Some(iced::Background::Color(
                Color { r: 0.62, g: 0.82, b: 0.88, a: 0.14 },
            )),
            text_color: Color { r: 0.84, g: 0.95, b: 0.97, a: 1.0 },
            border: Border {
                color: Color { r: 0.55, g: 0.85, b: 0.92, a: 0.38 },
                width: 1.0,
                radius: RADIUS.into(),
            },
            shadow: crate::palette::glow(
                Color { r: 0.20, g: 0.70, b: 0.85, a: 1.0 }, 0.22,
            ),
        };
        match status {
            Status::Hovered => Style {
                background: Some(iced::Background::Color(
                    Color { r: 0.70, g: 0.90, b: 0.96, a: 0.24 },
                )),
                border: Border {
                    color: Color { r: 0.70, g: 0.95, b: 1.0, a: 0.65 },
                    width: 1.0,
                    radius: RADIUS.into(),
                },
                text_color: Color::WHITE,
                shadow: crate::palette::glow(
                    Color { r: 0.30, g: 0.80, b: 0.95, a: 1.0 }, 0.40,
                ),
            },
            Status::Pressed => Style {
                background: Some(iced::Background::Color(
                    Color { r: 0.55, g: 0.78, b: 0.85, a: 0.32 },
                )),
                ..s
            },
            _ => s,
        }
    }

    /// Azione secondaria: fondo scuro neutro, bordo tenue.
    pub fn secondary(_: &Theme, status: Status) -> Style {
        let s = base(Color { r: 0.74, g: 0.86, b: 0.88, a: 1.0 },
                     Color { r: 0.045, g: 0.085, b: 0.095, a: 1.0 }, 0.22);
        match status {
            Status::Hovered => Style {
                background: Some(iced::Background::Color(Color { r: 0.08, g: 0.17, b: 0.19, a: 1.0 })),
                border: Border {
                    color: Color { r: 0.20, g: 0.62, b: 0.72, a: 0.55 },
                    width: 1.0,
                    radius: RADIUS.into(),
                },
                ..s
            },
            Status::Pressed => Style {
                background: Some(iced::Background::Color(Color { r: 0.03, g: 0.10, b: 0.11, a: 1.0 })),
                ..s
            },
            _ => s,
        }
    }

    /// Azione distruttiva / pericolo.
    pub fn danger(_: &Theme, status: Status) -> Style {
        // Rosso vivo, non marrone: il fondo precedente (0.36, 0.05, 0.09)
        // era spento e fangoso e il tasto sembrava disabilitato invece che
        // pericoloso. Un'azione distruttiva deve leggersi da lontano.
        let s = base(Color::WHITE, Color { r: 0.58, g: 0.09, b: 0.13, a: 1.0 }, 0.65);
        match status {
            Status::Hovered => Style {
                shadow: crate::palette::glow(Color { r: 0.90, g: 0.12, b: 0.18, a: 1.0 }, 0.35),
                ..s
            },
            Status::Pressed => Style {
                background: Some(iced::Background::Color(Color { r: 0.26, g: 0.03, b: 0.06, a: 1.0 })),
                ..s
            },
            _ => s,
        }
    }

    /// **Pulsante Abilita TEC, colorato come il LED del controller.**
    ///
    /// Il colore viene dal modulo `modalita`, che lo deriva a sua volta dal
    /// colore del LED scurito. Quindi il pulsante e la modalita' parlano la
    /// stessa lingua: blu in Standby, verde in Cryo, viola in Unregulated,
    /// rosso se il controller non risponde. Un colore, un significato.
    ///
    /// Non e' il colore del LED esatto perche' sul pulsante c'e' testo
    /// bianco: il verde saturo del LED non darebbe contrasto.
    pub fn tec_modalita(led: (u8, u8, u8), _: &Theme, status: Status) -> Style {
        let (r, g, b) = (led.0 as f32 / 255.0, led.1 as f32 / 255.0, led.2 as f32 / 255.0);
        // Alone in hover piu' saturo del fondo: da' il senso di "premilo".
        let alone = Color { r, g, b, a: 1.0 };
        let s = base(
            Color::WHITE,
            Color { r, g, b, a: 1.0 },
            0.75,
        );
        match status {
            Status::Hovered => Style {
                shadow: crate::palette::glow(alone, 0.38),
                ..s
            },
            Status::Pressed => Style {
                // Premuto: si scurisce, non cambia tinta.
                background: Some(iced::Background::Color(
                    Color { r: r * 0.7, g: g * 0.7, b: b * 0.7, a: 1.0 },
                )),
                ..s
            },
            _ => s,
        }
    }

    /// **Pulsante in vetro**, col colore che dice l'azione.
    ///
    /// Il vetro e' quello giusto qui perche' lo sfondo e' una foto: il vetro
    /// sfuma quello che c'e' sotto e mette solo un bordo chiaro sottile a
    /// far leggere il bordo del vetro. Un fondo pieno, su una foto, e' un
    /// tappo; il vetro no.
    ///
    /// `accent` e' il colore che dice **l'azione**, non lo stato: verde
    /// quando accendi, rosso quando spegni. Se il colore dicesse lo stato,
    /// un pulsante verde che spegne si leggerebbe al contrario.
    ///
    /// L'alone e' piu' forte in hover, e in pressed il vetro si scurisce senza
    /// cambiare tinta: la pressione non e' un colore diverso, e' la stessa
    /// azione con un grado in piu' di luce.
    pub fn tec_console(accent: (u8,u8,u8), theme:&Theme, status:Status) -> Style {
        let mut style=vetro(accent,theme,status);
        let color=Color::from_rgb8(accent.0,accent.1,accent.2);
        let pressed=matches!(status,Status::Pressed);
        style.background=Some(iced::Background::Gradient(iced::Gradient::Linear(
            iced_core::gradient::Linear::new(0.7)
                .add_stop(0.0,Color::from_rgba8(21,42,55,if pressed {0.96} else {0.90}))
                .add_stop(0.52,Color::from_rgba8(8,21,30,0.94))
                .add_stop(1.0,Color {r:color.r*0.09,g:color.g*0.09,b:color.b*0.09,a:0.96})
        )));
        style.border.radius=14.0.into();
        style.border.color=Color {a:if matches!(status,Status::Hovered) {0.85} else {0.45},..color};
        style.shadow=if matches!(status,Status::Hovered) {crate::palette::glow(color,0.25)} else {Shadow::default()};
        style
    }

    pub fn vetro(accent: (u8, u8, u8), _: &Theme, status: Status) -> Style {
        let (r, g, b) = (
            accent.0 as f32 / 255.0,
            accent.1 as f32 / 255.0,
            accent.2 as f32 / 255.0,
        );
        let pieno = Color { r, g, b, a: 1.0 };
        // Fondo scuro semitrasparente: si vede la foto, ma il testo resta
        // leggibile. Il vetro e' al 18%: abbastanza da non confondere i
        // numeri, poco da nascondere l'immagine.
        let s = base(
            Color { r: 0.94, g: 0.98, b: 1.0, a: 1.0 },
            Color { r: 0.04, g: 0.07, b: 0.10, a: 0.82 },
            0.30,
        );
        // Bordo colorato: e' il bordo del vetto che prende la luce.
        let s = Style {
            border: Border {
                color: Color { r, g, b, a: 0.55 },
                width: 1.0,
                radius: RADIUS.into(),
            },
            ..s
        };
        match status {
            Status::Hovered => Style {
                shadow: crate::palette::glow(pieno, 0.50),
                // In hover il vetro si illumina leggermente: dice "premi qui".
                background: Some(iced::Background::Color(
                    Color { r: 0.06, g: 0.11, b: 0.15, a: 0.90 },
                )),
                ..s
            },
            Status::Pressed => Style {
                background: Some(iced::Background::Color(
                    Color { r: 0.02, g: 0.04, b: 0.06, a: 0.92 },
                )),
                shadow: crate::palette::glow(pieno, 0.28),
                ..s
            },
            Status::Disabled => Style {
                background: Some(iced::Background::Color(
                    Color { r: 0.05, g: 0.06, b: 0.07, a: 0.55 },
                )),
                text_color: Color { r: 0.45, g: 0.50, b: 0.52, a: 0.7 },
                border: Border {
                    color: Color { r: 0.35, g: 0.38, b: 0.40, a: 0.25 },
                    width: 1.0,
                    radius: RADIUS.into(),
                },
                ..s
            },
            _ => s,
        }
    }

    /// Stile dei campi numerici (offset, potenza massima, coefficienti PID).
    ///
    /// Il `NumberInput` di default mostra il campo come un input di testo
    /// grigio con due pulsanti grossi a freccia: su sfondo scuro sembrava
    /// un controllo di una utility Windows degli anni 2000. Qui il campo
    /// diventa un pill arrotondato coerente con le card, con frecce
    /// sottili e visibili.
    pub fn number_input(theme: &Theme, status: iced_aw::style::Status) -> iced_aw::style::number_input::Style {
        use iced_aw::style::number_input::Style as NiStyle;
        let _ = theme;
        match status {
            iced_aw::style::Status::Disabled => NiStyle {
                // I pulsanti +/- spariscono quando disabilitato: niente
                // forme grigie vuote che sembrano errori.
                button_background: None,
                icon_color: Color { r: 0.30, g: 0.34, b: 0.36, a: 0.8 },
            },
            _ => NiStyle {
                // Sfondo dei +/- appena percettibile: il campo deve
                // dominare, non i bottoni.
                button_background: Some(iced::Background::Color(
                    Color { r: 0.10, g: 0.16, b: 0.18, a: 1.0 })),
                icon_color: Color { r: 0.55, g: 0.85, b: 0.90, a: 1.0 },
            },
        }
    }

    /// Stile delle Card dei pannelli.
    ///
    /// Il default di iced_aw colora l'intestazione di **dodgerblue**
    /// (rgb 30,144,255): su questa dashboard scura era una barra azzurra
    /// piena sopra un corpo nero, l'aspetto peggiore possibile. Qui
    /// l'intestazione segue il tema e il bordo ciano mette il pannello in
    /// linea con il resto della UI.
    pub fn card(_: &Theme, _status: iced_aw::style::Status) -> iced_aw::style::card::Style {
        iced_aw::style::card::Style {
            background: iced::Background::Color(Color { r: 0.030, g: 0.065, b: 0.072, a: 0.99 }),
            border_radius: 12.0,
            border_width: 1.0,
            border_color: Color { r: 0.10, g: 0.32, b: 0.38, a: 0.9 },
            head_background: iced::Background::Color(Color { r: 0.045, g: 0.115, b: 0.130, a: 1.0 }),
            head_text_color: Color { r: 0.80, g: 0.95, b: 0.98, a: 1.0 },
            body_background: iced::Background::Color(Color { r: 0.020, g: 0.045, b: 0.050, a: 1.0 }),
            body_text_color: Color { r: 0.80, g: 0.92, b: 0.94, a: 1.0 },
            foot_background: iced::Background::Color(Color { r: 0.045, g: 0.115, b: 0.130, a: 1.0 }),
            foot_text_color: Color { r: 0.70, g: 0.85, b: 0.88, a: 1.0 },
            close_color: Color { r: 0.60, g: 0.75, b: 0.80, a: 1.0 },
        }
    }

    /// Stile dei Toggler.
    ///
    /// Il default e' una capsula grigio-verdosa (rgb 89,110,105) senza
    /// bordo, piu' chiara dello sfondo della sidebar: le interruttori
    /// sembravano pulsanti galleggianti. Qui la capsula spenta e' scura con
    /// bordo, quella attiva e' ciano con alone.
    pub fn toggler(_: &Theme, status: iced::widget::toggler::Status) -> iced::widget::toggler::Style {
        use iced::widget::toggler::Status;
        // Lo stato "acceso" e' dentro le varianti, non e' una variante a se.
        let acceso = matches!(
            status,
            Status::Active { is_toggled: true }
                | Status::Hovered { is_toggled: true }
        );
        let spento = iced::widget::toggler::Style {
            background: Color { r: 0.06, g: 0.11, b: 0.12, a: 1.0 },
            background_border_width: 1.0,
            background_border_color: Color { r: 0.16, g: 0.26, b: 0.28, a: 1.0 },
            foreground: Color { r: 0.30, g: 0.36, b: 0.38, a: 1.0 },
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
        };
        let stile_acceso = iced::widget::toggler::Style {
            background: Color { r: 0.08, g: 0.45, b: 0.56, a: 1.0 },
            background_border_width: 1.0,
            background_border_color: Color { r: 0.20, g: 0.75, b: 0.88, a: 1.0 },
            foreground: Color::WHITE,
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
        };
        if acceso { stile_acceso } else { spento }
    }

    /// Stile dei campi di testo.
    ///
    /// Il default di iced e' un rettangolo grigio chiaro: su questa dashboard
    /// scura risultava una patch chiara che spezzava il ritmo. Qui il campo
    /// diventa un pill arrotondato, coerente con le card, che si illumina
    /// appena al focus.
    pub fn text_input(_: &Theme, status: iced::widget::text_input::Status) -> iced::widget::text_input::Style {
        use iced::widget::text_input::Status;
        let base = iced::widget::text_input::Style {
            background: iced::Background::Color(Color { r: 0.035, g: 0.070, b: 0.080, a: 1.0 }),
            border: iced::Border {
                color: Color { r: 0.10, g: 0.16, b: 0.18, a: 1.0 },
                width: 1.0,
                radius: 8.0.into(),
            },
            icon: Color { r: 0.45, g: 0.80, b: 0.90, a: 1.0 },
            placeholder: Color { r: 0.35, g: 0.45, b: 0.48, a: 1.0 },
            value: Color { r: 0.88, g: 0.97, b: 0.96, a: 1.0 },
            selection: Color { r: 0.10, g: 0.45, b: 0.55, a: 1.0 },
        };
        match status {
            Status::Focused => iced::widget::text_input::Style {
                border: iced::Border {
                    color: Color { r: 0.15, g: 0.65, b: 0.78, a: 0.9 },
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..base
            },
            Status::Disabled => iced::widget::text_input::Style {
                background: iced::Background::Color(Color { r: 0.02, g: 0.035, b: 0.04, a: 1.0 }),
                value: Color { r: 0.40, g: 0.45, b: 0.47, a: 1.0 },
                ..base
            },
            _ => base,
        }
    }
}

fn main() {
    if std::env::args().any(|arg| arg == "--preview-grafici")
        || std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s|s.to_string_lossy().into_owned()))
            .is_some_and(|name|name.ends_with("-Preview")) {
        chart_preview::run();
        return;
    }
    if recovery::supervise() { return; }
    let previous_hook=std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {recovery::log(&format!("PANIC: {info}"));previous_hook(info);}));
    // ── Guardia sugli offset di regime ────────────────────────────────
    //
    // Il cambio di regime scrive `0x14` (setpoint offset). Nello stesso
    // intervallo di opcode c'è `0x1E`, che è il **reset di fabbrica**: su
    // questo protocollo half-duplex senza numero di sequenza, un byte
    // sbagliato non produce un errore, produce un controller azzerato che
    // perde PID, setpoint e tetto di potenza.
    //
    // Per questo gli offset dei tre regimi non sono numeri scelti qui: sono
    // presi dal binario del produttore (`Intel.CryoCooling.Configuration.dll`).
    // `non_sono_tutti_verificati()` è `const`, quindi se qualcuno cambiasse un
    // valore il fallimento arriva **alla compilazione**, non alla prima
    // esecuzione su una macchina con la piastra sotto la rugiada.
    if commutazione::non_sono_tutti_verificati() {
        eprintln!(
            "Avvio rifiutato: un regime ha un offset non verificato nel binario Intel.\n\
             Scrivere un offset inventato su 0x14 puo' azzerare il controller.\n\
             Non avvio: correggi l'offset in src/commutazione.rs prima di usarlo."
        );
        std::process::exit(2);
    }

    // ── Istanza singola ──────────────────────────────────────────────
    // Due istanze non possono condividere la porta COM: la seconda prende
    // "Accesso negato" e finisce per mostrare la schermata di selezione
    // portando l'utente a credere che il rilevamento automatico sia rotto.
    // Il mutex viene rilasciato dal SO quando il processo muore, quindi
    // non lascia lock residui dopo un crash.
    let _instance_guard = match single_instance::SingleInstance::acquire() {
        Ok(g) => Some(g),
        Err(()) => {
            eprintln!(
                "Stargate CryoCooling Controller e' gia' in esecuzione.\n\
                 Due istanze non possono condividere la porta COM del controller TEC:\n\
                 chiudi quella gia' aperta (icona nella barra dei task o menu tray -> Quit)\n\
                 prima di riavviare."
            );
            std::process::exit(1);
        }
    };

    // ── Ripara l'avvio automatico ─────────────────────────────────────
    // Se il registro punta a una copia vecchia dell'eseguibile, Windows
    // avvierebbe quella all'accensione e le due copie si contenderebbero la
    // porta COM del controller. Va fatto PRIMA di scansionare le porte.
    #[cfg(target_os = "windows")]
    let _ = autostart::repair_autostart();

    let icon =
        tray_icon::icon::Icon::from_rgba(ICON.to_vec(), 64, 64).expect("Failed to open icon");

    let tray_menu = Menu::new();
    // tray_icon assegna gli ID in ordine di creazione a partire da 1000.
    // Li catturiamo dai MenuItem per non dipendere da quel fragile implicito.
    let quit_i  = MenuItem::new("Esci e arresta il controllo", true, None);
    let show_i  = MenuItem::new("Mostra dashboard", true, None);
    let quit_id = quit_i.id();
    let show_id = show_i.id();
    tray_menu.append_items(&[
        &show_i,
        &PredefinedMenuItem::separator(),
        &quit_i,
    ]);

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("Stargate Labs · Cryo Cooler Controller")
        .with_icon(icon)
        .build();

    // BUG: prima era `if let Err(e) = tray_icon` — in caso Ok la TrayIcon veniva
    // droppata a fine statement e l'icona spariva dal system tray.
    // Ora la handle viene conservata per tutta la durata del `run_with`.
    let _tray_icon = match tray_icon {
        Ok(t)  => t,
        Err(tray_icon::Error::OsError(err)) => {
            std::process::exit(err.raw_os_error().unwrap_or(-1))
        }
        Err(_) => std::process::exit(-1),
    };

    let window_settings = iced::window::Settings {
        size: iced::Size::new(720.0, 560.0),  // abbastanza grande per logo + picker senza resize
        min_size: Some(iced::Size::new(660.0, 500.0)), // impedisce di rimpicciolirla troppo
        resizable: true,
        exit_on_close_request: false,
        decorations: true,
        icon: Some(
            iced::window::icon::from_rgba(ICON.to_vec(), 64, 64)
                .expect("icon.bin contains valid rgba"),
        ),
        ..iced::window::Settings::default()
    };

    // Gli ID del menu tray servono al gestore eventi.
    TRAY_IDS.with(|c| *c.borrow_mut() = (show_id, quit_id));

    let result = iced::application(
        CryoCoolerController::title,
        CryoCoolerController::update,
        CryoCoolerController::view,
    )
    .theme(CryoCoolerController::theme)
    .subscription(CryoCoolerController::subscription)
    // Font di sistema, non il monospace integrato di iced.
    //
    // iced 0.13 usa di default `Font::MONOSPACE`, un font minimale che non
    // contiene i simboli usati nella dashboard: `⚠`, `🚨`, `▸`, `✓`, `▲`
    // venivano resi come quadratini vuoti. Segoe UI e' gia' presente su
    // ogni Windows, ha copertura Unicode ampia e — a differenza del
    // monospace — e' un font proporzionale, che per una dashboard con
    // tabelle e cifre affiancate si legge meglio.
    //
    // Se per qualche motivo il font non e' disponibile, il renderer cade
    // sul default: peggio, ma non si rompe nulla.
    .default_font(iced::Font::with_name("Segoe UI"))
    // Text and image filtering remain antialiased; avoid full-window 4x MSAA.
    .antialiasing(false)
    .window(window_settings)
    .run_with(CryoCoolerController::new);
    if let Err(e)=result {recovery::log(&format!("UI error: {e}"));std::process::exit(70);}
}

// ID del menu tray, impostati in `main()` e letti dal gestore eventi.
// Evita i numeri magici 1000/1001, che dipendono dall'ordine di creazione.
// `MenuItem::id()` restituisce `u32` in tray-icon 0.4 / muda 0.4.
thread_local! {
    static TRAY_IDS: std::cell::RefCell<(u32, u32)> =
        const { std::cell::RefCell::new((0, 0)) };
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    /// Ridisegna **solo** i grafici, senza toccare nient'altro.
    ///
    /// Il tick da 250 ms fa due cose insieme: raccoglie il campione seriale
    /// e ridisegna. Il ridisegno, pero', resta a 4 Hz, e a 4 fps ogni
    /// animazione si vede a scatti: perche' l'immagine viene ricalcolata
    /// solo quattro volte al secondo, non perche' manchino i dati (il
    /// campione arriva 2 volte al secondo e basta).
    ///
    /// Questo messaggio stacca le due cose: arriva 60 volte al secondo e
    /// serve **solo** a far avanzare l'orologio dei grafici, cosi' la linea
    /// scorre. Non legge la seriale, non scrive sul TEC, non esegue la
    /// guardia: solo ridisegno.
    ///
    /// Non si e' potuto semplicemente accorciare `Tick`: prima la seriale
    /// era bloccante sul thread della UI, quindi un tick veloce faceva
    /// bloccare tutto 20 volte al secondo. Ora la seriale e' su un thread
    /// dedicato, ma accorciare `Tick` farebbe girare a 60 Hz anche guardia,
    /// allarmi e overlay, che non serve e costa.
    RidisegnaGrafici,
    /// Tasto premuto: al momento serve solo F11 per lo schermo intero.
    Key(iced::keyboard::Key),
    /// Passa da finestra a schermo intero e viceversa.
    ToggleFullscreen,
    /// Il watchdog ha riportato la finestra in modalità finestra.
    ///
    /// Serve perché lo stato di fullscreen vive sul livello superiore e
    /// questa azione nasce dentro `RunningState`. Senza il messaggio di
    /// ritorno il flag continuerebbe a dire "schermo intero" mentre la
    /// finestra è appena stata riportata, e il F11 successivo sembrerebbe
    /// non funzionare.
    FinestraRiportataInFinestra,
    CloseModal,
    ReconnectController(bool),
    /// Apre la spiegazione delle modalita' del controller.
    ///
    /// Non parte nessun comando TEC: il cambio di modalita' non e' un comando
    /// seriale, e indovinare un opcode su una seriale funzionante e' rischioso
    /// perche' lo stesso protocollo contiene il reset di fabbrica. Il messaggio
    /// apre la finestra che spiega dove si e' e perche' da qui non si commuta.
    InfoModalita,
    /// Il TEC spento non e' un errore, e' la modalita' richiesta: premere
    /// "Abilita TEC" deve portare a **Cryo**, non lasciare il controller nel
    /// riposo con la dashboard che dice Standby. Il messaggio porta con se'
    /// l'offset del pannello, perche' il Cryo non ha un offset proprio.
    CommutaCryo,
    /// Passa al regime di riposo: offset basso e TEC spento. Nessun rischio,
    /// e' il regime da cui si parte per ogni prova.
    CommutaStandby,
    /// Spegne il modulo.
    ///
    /// Non e' un regime: e' l'assenza di un regime, e per questo ha un
    /// pulsante tutto suo. Con il modulo spento il controller continua a
    /// rispondere — la dashboard resta collegata e continua a mostrare lo
    /// stato — ma non eroga potenza e non raffredda. Da qui si torna con
    /// "Abilita TEC" o con un regime.
    CommutaSpento,
    /// Passa a massima potenza non regolata.
    ///
    /// **Non parte nessun comando con il primo clic.** Il primo clic apre
    /// una richiesta esplicita; solo il secondo invia. Il motivo e' nel
    /// manuale: l'Unregulated porta la piastra sotto il punto di rugiada e
    /// puo' danneggiare la scheda, quindi non deve poter succedere per
    /// un gesto distratto su un pannello che si aggiorna a ogni secondo.
    CommutaUnregulatedChiedi,
    /// L'operatore ha confermato: adesso si scrive sul bus.
    CommutaUnregulatedConferma,
    /// L'operatore ha confermato la **conferma anticondensa**.
    ///
    /// Non e' la stessa cosa di `CommutaUnregulatedConferma`: questo scrive il
    /// regime che era gia' in attesa dentro `InAttesa`, non quello che l'ha
    /// aperta. La finestra puo' restare aperta qualche secondo, e in quel
    /// tempo l'operatore puo' premere altri pulsanti: se la conferma rileggesse
    /// "qualunque cosa sia premuta adesso", scriverebbe il regime sbagliato.
    ConfermaAnticondensa,
    /// Apre e chiude il pannello diagnostico.
    ///
    /// Il pannello e' il posto dove vivono i dettagli: tensione, corrente,
    /// potenza, COP, margine di condensa, i bit di stato e cosa fare per
    /// ciascun problema. In sidebar resta solo il verdetto, perche' gli stessi
    /// numeri sono gia' nella griglia principale e un dato ripetuto due volte
    /// non rende l'informazione piu' affidabile, solo piu' rumorosa.
    DiagnosticaToggle,
    /// Scrive il rapporto diagnostico su file.
    ///
    /// Il file dichiara gia' per iscritto che non e' il self test del
    /// produttore: e' il documento da allegare al supporto, e serve che la
    /// persona che lo manda sappia cosa sta mandando.
    DiagnosticaSalva,
    /// Esc: chiude la richiesta aperta **senza scrivere niente sul bus**.
    ///
    /// Priorità: conferma dell'Unregulated, poi pannello diagnostica, poi
    /// cronologia, poi le finestre informative. L'ordine è dal più pericoloso
    /// al meno: chiudere la conferma dell'Unregulated è l'unico caso in cui
    /// un tasto premuto per abitudine (chiudere una finestra) potrebbe
    /// scrivere sul controller, e qui non scrive.
    Annulla,
    Enable,
    Disable,
    UpdatePCoef(f32),
    UpdateICoef(f32),
    UpdateDCoef(f32),
    UpdateSetpoint(f32),
    UpdateMaxPower(u8),
    UpdateProfileName(String),
    SaveProfile,
    LoadProfile(usize),
    DeleteProfile(usize),
    ExportCsv,
    AskAi,
    AiResponse(Result<crate::ai_advisor::AiAdvice, String>),
    AiApiKeyChanged(String),
    AiApplyParams,
    DismissAlert,
    DismissAlertChannel(crate::alerts::AlertChannel),
    ToggleAlertRule(usize),
    /// Accende/spegne il profilatore automatico del carico di lavoro.
    ToggleAutoProfile,
    /// Pannello impostazioni: apre/chiude la configurazione delle soglie.
    ToggleSettings,
    /// Direzione della soglia: `true` = "supera", `false` = "scende sotto".
    SetAlertCondition(usize, bool),
    /// Nuovo valore di soglia.
    SetAlertThreshold(usize, f32),
    /// Attesa prima di ripetere l'allarme (secondi).
    SetAlertCooldown(usize, u32),
    ExportPdf,
    PidWizardStart,
    PidWizardCancel,
    PidWizardApply,
    DiscordToggle,
    RtssToggle,
    DebugToggle,
    /// Accende/spegne le notifiche Windows (toast). Gli allarmi in-app e le
    /// protezioni termiche restano attivi: cambia solo l'avviso esterno.
    ToggleToasts(bool),
    /// Accende/spegge le strisce di allarme dentro la dashboard.
    ToggleAlertBanner(bool),
    AutostartToggled(bool),
    AutoDetectPort,
    /// L'utente chiede di riprovare: azzera i contatori e riavvia subito
    /// la scansione, senza attendere la pausa programmata.
    RetryAutoConnect,
    /// Fine scansione: porta trovata (se presente) + elenco delle porte
    /// effettivamente esaminate, per diagnosticare i fallimenti.
    ScanFinished { found: Option<PortIdent>, scanned: String },
    SessionHistoryOpen,
    SensorTab(sensors_panel::SensorTab),
    SetSensorSource(hwinfo::SensorSource),
    WindowResized(u32, u32),
    WindowFocused(bool),
    PortSelected(PortIdent),
    Open,
    Hide,
    FontLoaded(Result<(), iced::font::Error>),
    FontLoadingFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortIdent {
    path: std::path::PathBuf,
}

impl std::fmt::Display for PortIdent {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

#[derive(Default)]
struct HomeState {
    selected_port: Option<PortIdent>,
    error_text:    Option<String>,
    /// Handle immagine logo precalcolato — evita LOGO_BANNER.to_vec() (586 KB) ad ogni frame.
    logo_handle:   Option<iced::widget::image::Handle>,
    /// Stato avvio automatico con Windows (letto dal registro)
    autostart_enabled: bool,
    /// Stato scansione porte
    scanning: bool,
    /// Contatore per animazione puntini scansione
    scan_dots: u8,
    /// Cache porte disponibili — rigenerata nel Tick, NON in view().
    /// Evita 20 enumerazioni SetupAPI/secondo dentro view().
    available_ports: Vec<PortIdent>,
    /// Porta rilevata automaticamente
    detected_port: Option<PortIdent>,
    /// Tentativi di connessione automatica COMPLETATI (non i tick).
    /// Dopo 3 tentativi falliti si mostra la schermata Home con il bottone.
    auto_connect_tries: u32,
    /// Quando è iniziata l'ultima scansione. Serve a non lanciare scansioni
    /// sovrapposte e a dare tempo a Windows di enumerare le porte seriali
    /// (il device spesso non è pronto nei primi secondi dopo il boot).
    last_scan: Option<std::time::Instant>,
    /// Ultima scansione: porte effettivamente esaminate. Serve a distinguere
    /// "nessuna porta COM" da "porte presenti ma nessuna TEC".
    last_scan_info: String,
}

/// Pausa fra due tentativi di connessione automatica.
const AUTO_CONNECT_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(2000);
/// Quante volte riprovare prima di mostrare la schermata Home.
const AUTO_CONNECT_MAX_TRIES: u32 = 20;

#[cfg(test)]
mod connection_tests {
    #[test]
    fn repeated_detection_requests_do_not_start_parallel_scans() {
        let mut home=super::HomeState::new();
        home.scanning=true;
        home.error_text=Some("scan already in progress".into());
        for _ in 0..20 {
            let _=home.update(super::Message::AutoDetectPort);
            assert!(home.scanning);
            assert_eq!(home.error_text.as_deref(),Some("scan already in progress"));
        }
    }
}

impl HomeState {
    /// Filtro porte seriali: prioritario su Linux usa ttyUSB/ttyACM, su Windows COM
    fn filter_ports(names: Vec<String>) -> Vec<String> {
        #[cfg(target_os = "linux")]
        {
            let mut filtered: Vec<String> = names
                .into_iter()
                .filter(|n| {
                    n.contains("ttyUSB") || n.contains("ttyACM") || n.contains("ttyS")
                })
                .collect();
            // Ordina: USB* > ACM* > S*
            filtered.sort_by(|a, b| {
                let order = |s: &str| {
                    if s.contains("ttyUSB") { 0 }
                    else if s.contains("ttyACM") { 1 }
                    else { 2 }
                };
                order(a).cmp(&order(b))
            });
            filtered
        }
        #[cfg(not(target_os = "linux"))]
        {
            names
        }
    }

    pub fn new() -> Self {
        let available_ports = Self::probe_ports();
        // Alloca il logo buffer una volta sola — clonare Handle è O(1) (Arc interno)
        let logo_handle = Some(iced::widget::image::Handle::from_rgba(
            LOGO_W, LOGO_H, LOGO_BANNER.to_vec(),
        ));
        // Leggi stato avvio automatico da registro Windows
        #[cfg(target_os = "windows")]
        let autostart_enabled = autostart::is_autostart_enabled();
        #[cfg(not(target_os = "windows"))]
        let autostart_enabled = false;
        
        Self { 
            selected_port: available_ports.last().cloned(),
            error_text: None, 
            logo_handle,
            autostart_enabled,
            scanning: false,
            scan_dots: 0,
            available_ports,
            detected_port: None,
            auto_connect_tries: 0,
            last_scan: None,
            last_scan_info: String::new(),
        }
    }

    /// Enumera le porte seriali e le converte in `PortIdent`.
    /// Chiamata una volta per tick (~2 Hz), non per frame.
    fn probe_ports() -> Vec<PortIdent> {
        Self::filter_ports(serial_probe::SerialProbe::available_com_ports())
            .into_iter()
            .map(|name| PortIdent { path: std::path::PathBuf::from(name) })
            .collect()
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        // `Message::ToggleFullscreen` non e' gestito qui: lo stato del
        // video e' di `CryoCoolerController`, non della schermata di
        // connessione, e F11 deve funzionare anche a dashboard avviata.
        //
        // Qui ogni braccio che produce una `Task` la restituisce con
        // `return` diretto: non c'e' niente da accumulare.
        match message {
            Message::Key(_) => {}
            // Ridisegno veloce: **non fa niente**, serve solo a far
            // ricostruire l'albero dei widget (e quindi i grafici) a 60 Hz.
            //
            // Non raccoglie campioni, non scrive sul TEC, non esegue la
            // guardia: quelle cose stanno in `Message::Tick`. Qui il corpo e'
            // vuoto di proposito, e i grafici si muovono lo stesso perche'
            // il loro orologio e' l'ora di sistema, letta al ridisegno.
            Message::RidisegnaGrafici => {}
            Message::Tick => {
                if self.scanning {
                    // Scansione in corso: solo l'animazione, niente altro.
                    self.scan_dots = (self.scan_dots + 1) % 4;
                } else {
                    // Rinfresca la lista porte (una volta per tick, leggero)
                    // invece che per frame dentro view().
                    let fresh = Self::probe_ports();
                    if fresh != self.available_ports {
                        self.available_ports = fresh;
                        if self.selected_port.is_none() {
                            self.selected_port = self.available_ports.first().cloned();
                        }
                    }

                    // ── Connessione automatica ────────────────────────────
                    // Una scansione per volta, distanziate di 2 secondi.
                    // Il pacing è sul TEMPO, non sui tick: i tick arrivano
                    // 20 volte al secondo, quindi contarne consumerebbe i
                    // tentativi in 150ms, prima ancora che la prima
                    // scansione sia finita.
                    if self.auto_connect_tries < AUTO_CONNECT_MAX_TRIES {
                        let pronto = match self.last_scan {
                            None => true,
                            Some(t) => t.elapsed() >= AUTO_CONNECT_RETRY_DELAY,
                        };
                        if pronto {
                            self.last_scan = Some(std::time::Instant::now());
                            return Task::done(Message::AutoDetectPort);
                        }
                    }
                }
            }
            Message::PortSelected(p) => {
                self.selected_port = Some(p);
                // Scelta manuale: azzera i tentativi automatici così, se la
                // connessione fallisce, l'utente vede subito l'errore invece
                // di aspettare i retry. Ripartiamo subito da una scansione.
                self.auto_connect_tries = AUTO_CONNECT_MAX_TRIES;
                self.last_scan = None;
                self.scanning = false;
            }
            Message::CloseModal      => { self.error_text = None; }
            Message::AutostartToggled(enabled) => {
                if let Err(e) = autostart::toggle_autostart(enabled) {
                    self.error_text = Some(format!("Errore autostart: {e}"));
                } else {
                    self.autostart_enabled = enabled;
                }
            }
            Message::RetryAutoConnect => {
                // Azzera tutto e riavvia: utile se il controller è stato
                // collegato dopo l'ultimo tentativo fallito.
                self.auto_connect_tries = 0;
                self.last_scan = None;
                self.scanning = false;
                self.error_text = None;
                return Task::done(Message::AutoDetectPort);
            }
            Message::AutoDetectPort => {
                if self.scanning { return Task::none(); }
                self.scanning = true;
                self.error_text = None;
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(|| {
                            serial_probe::SerialProbe::find_tec_port_with_report()
                        })
                        .await
                        .unwrap_or_else(|_e| serial_probe::ScanReport {
                            found: None,
                            attempted: Vec::new(),
                        })
                    },
                    |report| {
                        // Teniamo la diagnostica: senza, un fallimento di
                        // rilevamento è indistinguibile da "nessuna porta".
                        let scanned = report
                            .attempted
                            .iter()
                            .map(|(p, _)| p.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        Message::ScanFinished {
                            found: report.found.map(|name| PortIdent {
                                path: std::path::PathBuf::from(&name),
                            }),
                            scanned,
                        }
                    },
                );
            }
            Message::ScanFinished { found, scanned } => {
                self.scanning = false;
                self.last_scan = Some(std::time::Instant::now());
                self.last_scan_info = scanned;
                match found {
                    Some(p) => {
                        self.detected_port = Some(p.clone());
                        self.selected_port = Some(p);
                        // Delega la connessione: HomeState non possiede lo
                        // stato dell'app, quindi chiediamo al livello
                        // superiore di aprire la porta.
                        return Task::done(Message::Open);
                    }
                    None => {
                        // Scansione conclusa senza esito: conta come tentativo.
                        // Il prossimo riprova dopo la pausa di 2 secondi.
                        self.auto_connect_tries += 1;
                    }
                }
            }
            _ => {}
        }
        Task::none()
    }

    pub fn view(&self) -> Element<'_, Message> {
        // ── Logo — usa l'handle precalcolato (clonare Handle è O(1), Arc interno) ──
        let logo_img = if let Some(ref handle) = self.logo_handle {
            iced::widget::Image::new(handle.clone())
                .width(Length::Fixed(340.0))
                .height(Length::Fixed(151.0))
        } else {
            // Fallback se handle non disponibile (non dovrebbe mai accadere)
            iced::widget::Image::new(iced::widget::image::Handle::from_rgba(
                LOGO_W, LOGO_H, LOGO_BANNER.to_vec(),
            ))
            .width(Length::Fixed(340.0))
            .height(Length::Fixed(151.0))
        };

        let title_cryo = Text::new("CryoCooling Dashboard")
            .size(30)
            .color(palette::BLUE_PRIMARY);

        let title_by = Text::new("by StargateLab")
            .size(15)
            .color(palette::BLUE_DIM);

        let brand_tag = Text::new(format!("Intel TEC Controller  ·  v{}", env!("CARGO_PKG_VERSION")))
            .size(13)
            .color(
                iced::Color { r: 0.05, g: 0.30, b: 0.25, a: 1.0 }
            );

        let header = Column::new()
            .align_x(iced::Alignment::Center)
            .spacing(8)
            .push(logo_img)
            .push(title_cryo)
            .push(title_by)
            .push(brand_tag);

        #[cfg(target_os = "windows")]
        let autostart_toggle: Element<'_, Message> = {
            let toggler = iced::widget::Toggler::new(self.autostart_enabled)
                    .style(crate::btn::toggler);
            let toggler = toggler.label("Avvio automatico con Windows");
            let toggler = toggler.on_toggle(|enabled| Message::AutostartToggled(enabled));
            toggler.into()
        };

        #[cfg(not(target_os = "windows"))]
        let autostart_toggle = Text::new("").size(1);

        // Pannello di scansione. Prima era una stringa tipo "[==   ]" dentro
        // un bottone: non communicate nulla e sembrava un bottone che non
        // risponde. Ora e' un box con testo, puntini animati e barra reale.
        let auto_detect_btn: Element<'_, Message> = if self.scanning {
            let dots = ".".repeat(self.scan_dots as usize + 1);
            Container::new(
                Column::new().spacing(4)
                    .push(
                        Row::new().spacing(6)
                            .push(Text::new(format!("Ricerca del controller TEC{dots}"))
                                .size(12).color(palette::TEXT_BRIGHT))
                            .push(iced::widget::Space::with_width(Length::Fill)),
                    )
                    .push(
                        // Barra reale: la parte colorata cresce a ogni tick.
                        Container::new(
                            iced::widget::Space::with_height(Length::Fixed(4.0))
                                .width(Length::FillPortion(
                                    (((self.scan_dots as u32 + 1) * 25).min(100)) as u16,
                                )),
                        )
                        .width(Length::Fill)
                        .height(Length::Fixed(4.0))
                        .style(|_: &iced::Theme| iced::widget::container::Style {
                            background: Some(iced::Background::Color(
                                iced::Color { r: 0.20, g: 0.75, b: 0.95, a: 1.0 })),
                            border: iced::Border::default(),
                            text_color: None,
                            shadow: iced::Shadow::default(),
                        }),
                    )
                    .push(
                        Text::new("Connessione automatica in corso — non serve fare nulla.")
                            .size(10).color(palette::BLUE_DIM),
                    ),
            )
            .padding([10, 12])
            .width(Length::Fill)
            .style(|_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(
                    iced::Color { r: 0.05, g: 0.10, b: 0.10, a: 1.0 })),
                border: iced::Border {
                    color: palette::BLUE_PRIMARY,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                text_color: None,
                shadow: palette::glow(
                    iced::Color { r: 0.05, g: 0.20, b: 0.25, a: 1.0 }, 0.35),
            })
            .into()
        } else {
            iced::widget::button(
                iced::widget::text("Auto-rileva")
                    .size(11)
                    .align_x(alignment::Horizontal::Center)
                    .align_y(alignment::Vertical::Center),
            )
            .padding([6, 8])
            .style(crate::btn::secondary)
            .on_press(Message::AutoDetectPort)
            .into()
        };

        let version_line = Text::new(format!(
            "v{}  ·  Stargate Labs Edition",
            env!("CARGO_PKG_VERSION")
        ))
        .size(14)
        .color(palette::MATRIX_GRN);

        // Diagnostica: quante porte abbiamo provato e quante rispondono.
        // Distingue "nessuna COM" da "COM presenti ma il TEC è spento".
        let scan_line: Element<'_, Message> = if self.last_scan_info.is_empty() {
            iced::widget::Space::with_height(0.0).into()
        } else {
            Text::new(format!(
                "Porte rilevate: {}",
                self.last_scan_info
            ))
            .size(11)
            .color(palette::BLUE_DIM)
            .into()
        };

        // "Riprova" ha senso solo quando i tentativi automatici sono finiti:
        // durante la scansione automatica mostrerebbe una UI che si riavvia
        // da sola senza che l'utente possa fare nulla.
        //
        // Il placeholder è uno `Space`, non un `Text::new("")`: quest'ultimo
        // occupa comunque un'area e veniva reso come un rettangolo colorato
        // nel layout.
        let retry_btn: Element<'_, Message> = if self.scanning
            || self.auto_connect_tries < AUTO_CONNECT_MAX_TRIES
        {
            iced::widget::Space::with_height(0.0).into()
        } else {
            iced::widget::button(
                iced::widget::text("Riprova")
                    .size(12)
                    .align_x(alignment::Horizontal::Center)
                    .align_y(alignment::Vertical::Center),
            )
            .padding([8, 14])
            .style(crate::btn::primary)
            .on_press(Message::RetryAutoConnect)
            .into()
        };

        // ── Schermata di connessione ────────────────────────────────────
        // Non esiste piu' la selezione della porta COM: l'app rileva e
        // collega da sola. Qui c'e' solo l'attesa, e in caso di fallimento
        // un pulsante "Riprova" — niente dropdown, niente liste di porte:
        // per l'utente sono rumore, e la scelta non serve a nulla.
        let fallito = self.error_text.is_some()
            && self.auto_connect_tries >= AUTO_CONNECT_MAX_TRIES
            && !self.scanning;

        let status_col = Column::new()
            .spacing(10)
            .align_x(iced::Alignment::Center)
            .push(
                Text::new(if fallito {
                    "Controller TEC non raggiungibile"
                } else {
                    "Connessione al controller TEC"
                })
                .size(19)
                .color(if fallito { palette::DANGER } else { palette::TEXT_BRIGHT }),
            )
            .push(
                Text::new(match (self.scanning, fallito) {
                    (true, _)  => "Ricerca della porta in corso…",
                    (_, true)  => "Nessun controller risponde. Controlla il cavo USB e riprova.",
                    _          => "Preparazione…",
                })
                .size(13)
                .color(palette::BLUE_DIM),
            );

        let mut content = Column::new()
            .align_x(iced::Alignment::Center)
            .spacing(16)
            .push(header)
            .push(iced::widget::horizontal_rule(1))
            .push(iced::widget::Space::with_height(6.0))
            .push(status_col)
            .push(auto_detect_btn)
            .push(retry_btn)
            .push(scan_line)
            .push(autostart_toggle)
            .push(version_line);

        // Dettaglio tecnico solo se qualcosa e' andato storto: la diagnostica
        // resta disponibile senza occupare la schermata nel caso normale.
        if let Some(ref err) = self.error_text {
            if fallito {
                content = content.push(
                    Container::new(
                        Text::new(err.clone()).size(11).color(palette::BLUE_DIM),
                    )
                    .padding([8, 12])
                    .max_width(560.0)
                    .style(|_: &iced::Theme| iced::widget::container::Style {
                        background: Some(iced::Background::Color(
                            iced::Color { r: 0.10, g: 0.03, b: 0.04, a: 0.9 })),
                        border: iced::Border {
                            color: palette::DANGER,
                            width: 1.0,
                            radius: 8.0.into(),
                        },
                        text_color: None,
                        shadow: iced::Shadow::default(),
                    }),
                );
            }
        }

        // L'errore ora e' mostrato dentro la schermata di connessione ( sopra ),
        // non come overlay: una modale che richiede "OK" costringeva l'utente
        // a premere un pulsante per proseguire verso una schermata che non
        // esiste piu'.
        Container::new(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(24)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }
}

/// Stato della finestra che sopravvive al cambio di schermata.
///
/// Sta qui, e non dentro `HomeState`, perche' `State` viene rimpiazzato da
/// `State::Running` quando la connessione riesce: un flag tenuto nella
/// schermata di partenza verrebbe perso proprio nel caso in cui serve di
/// piu', cioe' quando l'utente preme F11 *prima* del rilevamento
/// automatico. Con il flag a schermo intero e l'handler sparito, dalla
/// dashboard non si tornava piu' indietro: solo Alt+F4.
///
/// `RunningState` non riceve `&mut Ui` (la sua firma non e' nostra da
/// cambiare), quindi `Message::ToggleFullscreen` e' intercettato da
/// `CryoCoolerController::update` prima di inoltrarlo allo stato corrente:
/// cosi' un solo gestore vale per schermata di connessione e dashboard.
pub struct Ui {
    /// Schermo intero attivo: fa da memoria per F11, cosi' il tasto
    /// alterna invece di ripetere l'ultima richiesta.
    pub fullscreen: bool,
}

struct CryoCoolerController {
    state: State,
    ui: Ui,
    window_focused: bool,
    resume_after_connect: bool,
}

enum State {
    Home(HomeState),
    Running(RunningState),
}

impl CryoCoolerController {

    pub fn theme(&self) -> Theme {
        // Tema dinamico: il colore di sfondo vira con la temperatura TEC
        // Idle/default: verde petrolio H=161°
        // Cryo attivo < -8°C: blu ghiaccio H=200°
        // Pericolo > 4°C: arancio H=30°
        let hue = if let State::Running(s) = &self.state {
            s.theme_hue
        } else {
            161.0_f32
        };
        // Converti HSV → RGB (S=86%, V=7% come il verde petrolio base)
        let h = hue / 60.0;
        let s = 0.82_f32; let v = 0.075_f32;
        let i = h.floor() as i32 % 6;
        let f = h - h.floor();
        let (p, q, t2) = (v*(1.0-s), v*(1.0-s*f), v*(1.0-s*(1.0-f)));
        let (r, g, b) = match i {
            0 => (v, t2, p), 1 => (q, v, p), 2 => (p, v, t2),
            3 => (p, q, v),  4 => (t2, p, v), _ => (v, p, q),
        };
        let bg = iced::Color { r, g, b, a: 1.0 };
        Theme::custom(String::from("StargateCryo"), iced::theme::palette::Palette {
            background: bg,
            text:       palette::TEXT_BRIGHT,
            primary:    palette::BLUE_PRIMARY,
            success:    palette::SUCCESS,
            danger:     palette::DANGER,
        })
    }

    fn new() -> (Self, Task<Message>) {
        (
            CryoCoolerController {
                state: State::Home(HomeState::new()),
                ui: Ui { fullscreen: false },
                window_focused: true,
                resume_after_connect: false,
            },
            // Font delle icone +/- dei campi numerici, poi la scansione.
            //
            // iced_aw dichiara `REQUIRED_FONT` ma non lo registra da solo:
            // senza questo le frecce dei NumberInput vengono disegnate con un
            // glifo mancante e appaiono come quadratini vuoti. Va caricato
            // esplicitamente, quindi la `Task` e' restituita da `new()`.
            Task::batch([
                iced::font::load(iced_fonts::REQUIRED_FONT_BYTES)
                    .map(Message::FontLoaded),
                Task::done(Message::AutoDetectPort),
            ]),
        )
    }

    pub fn title(&self) -> String {
        format!("STARGATE labs  ·  Cryo Cooler Controller v{}", env!("CARGO_PKG_VERSION"))
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        // Svuota TUTTI gli eventi del menu tray pendenti (non solo uno):
        // altrimenti un burst di click richiederebbe un update per evento.
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let (show_id, quit_id) = TRAY_IDS.with(|c| *c.borrow());
            if event.id == quit_id {
                recovery::intentional_quit();
                return iced::exit();
            }
            if event.id == show_id {
                // Ripristina la finestra dal tray (era nascosta con "Nascondi").
                // Si azzera il watchdog: la finestra e' di nuovo visibile.
                if let State::Running(s) = &mut self.state {
                    s.hidden_at = None;
                }
                // Qui si forza la finestra a `Windowed`, quindi il flag
                // va azzerato con lei: senza, il flag direbbe ancora
                // "fullscreen" e il prossimo F11 chiederebbe `Fullscreen`
                // su una finestra gia' non a schermo intero, quindi non
                // succederebbe nulla e servirebbero due pressioni.
                self.ui.fullscreen = false;
                return iced::window::get_latest().then(|opt_id| {
                    if let Some(id) = opt_id {
                        iced::window::change_mode(id, iced::window::Mode::Windowed)
                    } else {
                        Task::none()
                    }
                });
            }
        }

        match message {
            Message::ReconnectController(resume) => {
                self.resume_after_connect = resume;
                let mut home = HomeState::new();
                home.last_scan = Some(std::time::Instant::now());
                self.state = State::Home(home);
                recovery::log("Serial connection lost: releasing port and rescanning after 2s");
                return Task::perform(async {tokio::time::sleep(Duration::from_secs(2)).await}, |_| Message::AutoDetectPort);
            }
            Message::WindowFocused(focused) => {
                self.window_focused = focused;
                return Task::none();
            }
            Message::FinestraRiportataInFinestra => {
                // Lo stato di finestra e' questo, non quello che il flag
                // ricorda: il watchdog ha appena forzato la finestra.
                //
                // Nessun `return`: questo `match` non produce il valore di
                // ritorno, come gli altri bracci di questa funzione.
                self.ui.fullscreen = false;
            }
            Message::ToggleFullscreen => {
                // Gestito qui, prima di inoltrare il messaggio allo stato
                // corrente: `RunningState::update` non ha un braccio per
                // questo messaggio, quindi inoltrandolo F11 sulla dashboard
                // non farebbe nulla. Intercettarlo qui vale per entrambe le
                // schermate, senza toccare la firma di `RunningState`.
                //
                // Si ricorda da che parte si sta andando: `change_mode` con
                // `Windowed` da solo rimetterebbe la finestra piccola di
                // quando e' stata aperta, non le sue dimensioni attuali.
                self.ui.fullscreen = !self.ui.fullscreen;
                let modalita = if self.ui.fullscreen {
                    iced::window::Mode::Fullscreen
                } else {
                    iced::window::Mode::Windowed
                };
                // `Id` non espone un identificatore "finestra principale":
                // l'unico costruttore pubblico e' `unique()`, e la finestra
                // principale se lo e' creata da sola. `get_latest` restituisce
                // l'id dell'ultima finestra con il focus, che qui e' sempre
                // la dashboard.
                // `then` e non `map`: `get_latest` produce un `Task<Option<Id>>`
                // e serve accodare un *altro* Task al suo risultato, non
                // trasformare il risultato in un messaggio.
                return iced::window::get_latest().then(move |id| match id {
                    Some(id) => iced::window::change_mode(id, modalita),
                    None => Task::none(),
                });
            }
            Message::Open => {
                if let State::Home(ref mut home) = &mut self.state {
                    let Some(port) = home.selected_port.clone() else {
                        return Task::none();
                    };
                    match RunningState::new(&port.path) {
                        Ok(rs) => {
                            self.state = State::Running(rs);
                            let resume = std::mem::take(&mut self.resume_after_connect) || recovery::resume_once();
                            return if resume {
                                recovery::log("Controller reconnected; recovering enabled cooling in Cryo");
                                Task::done(Message::Enable)
                            } else { Task::none() };
                        }
                        Err(_e) => {
                            // Connessione fallita: conta come tentativo e
                            // riprova dopo la pausa. Solo quando i tentativi
                            // sono esauriti mostriamo la schermata Home.
                            home.auto_connect_tries += 1;
                            if home.auto_connect_tries < AUTO_CONNECT_MAX_TRIES {
                                return Task::none();
                            }
                            // Tentativi esauriti: diamo il quadro completo,
                            // così si distingue "nessuna porta" da "porte
                            // presenti ma il TEC non risponde".
                            let causa = if home.last_scan_info.is_empty() {
                                "nessuna porta COM trovata".to_owned()
                            } else {
                                format!(
                                    "porte esaminate: {} — nessuna risponde come TEC",
                                    home.last_scan_info
                                )
                            };
                            home.error_text = Some(format!(
                                "Connessione automatica non riuscita ({causa}).\n\
                                 Verifica il cavo USB del controller, poi scegli la porta o premi \"Riprova\"."
                            ));
                            return Task::none();
                        }
                    }
                }
            }
            Message::Hide => {
                // Nascondi la finestra e segna l'ora: il watchdog la
                // riprende se nessuno lo fa dal tray.
                if let State::Running(s) = &mut self.state {
                    s.hidden_at = Some(std::time::Instant::now());
                }
                return iced::window::get_latest().then(|opt_id| {
                    if let Some(id) = opt_id {
                        iced::window::change_mode(id, iced::window::Mode::Hidden)
                    } else {
                        Task::none()
                    }
                });
            }
            Message::FontLoadingFailed => {
                if let State::Home(ref mut home) = &mut self.state {
                    home.error_text = Some(
                        "Font icone non caricato — alcuni simboli potrebbero mancare.".to_owned(),
                    );
                    return Task::none();
                }
            }
            _ => {}
        }

        match &mut self.state {
            State::Home(s)    => s.update(message),
            State::Running(s) => s.update(message),
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        match &self.state {
            State::Home(s)    => s.view(),
            State::Running(s) => s.view(),
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        // **Causa del 50% di GPU: questo intervallo.**
        //
        // In iced ogni messaggio ricostruisce l'albero dei widget e ridisegna
        // la finestra. Qui il tick era ogni 50 ms, cioe' **20 ridisegni
        // completi al secondo, per sempre**: non solo quando cambia qualcosa.
        // Ogni ridisegno ricompatta ~40 widget di testo e 10 texture dei
        // grafici, con 4x MSAA su una superficie 1096x1936.
        //
        // Il commento diceva "50ms per evitare freeze da operazioni HID/IO",
        // ma un intervallo più corto NON previene i freeze: anzi, dato che la
        // lettura seriale è bloccante e sta sul thread della UI (150 ms di
        // timeout per comando, piu' fino a 320 ms di drenaggio nel caso peggiore),
        // un tick ogni 50 ms fa bloccare il thread 20 volte al secondo invece
        // che 4. Il rate veloce non aiutava: peggiorava sia la GPU sia gli
        // scatti.
        //
        // 4 Hz: i grafici restano fluidi (il dato arriva a 2 Hz, quindi si
        // ridisegna comunque piu' spesso di quanto cambi) e i ridisegni
        // passano da 20 a 4 al secondo, una riduzione di 5 volte.
        const TICK_MS: u64 = 250;
        let tick = iced::time::every(Duration::from_millis(TICK_MS)).map(|_| Message::Tick);

        // Presentation animation is independent of serial polling and guards.
        // Stop animation in Home and while hidden in the tray.
        // Keep visible graphs smooth on a second monitor, even without focus.
        let ridisegna = if matches!(&self.state, State::Running(s) if s.hidden_at.is_none() && s.telemetry_healthy()) {
            iced::time::every(Duration::from_millis(crate::charts::ANIMATION_MS)).map(|_| Message::RidisegnaGrafici)
        } else {
            Subscription::none()
        };        let resize = iced::event::listen_with(|event, _, _| {
            if let iced::Event::Window(iced::window::Event::Resized(size)) = event {
                Some(Message::WindowResized(size.width as u32, size.height as u32))
            } else if let iced::Event::Window(iced::window::Event::CloseRequested) = event {
                Some(Message::Hide)
            } else if let iced::Event::Window(iced::window::Event::Focused) = event {
                Some(Message::WindowFocused(true))
            } else if let iced::Event::Window(iced::window::Event::Unfocused) = event {
                Some(Message::WindowFocused(false))
            } else {
                None
            }
        });
        // F11: schermo intero senza bordo. Il bordo della finestra sparisce
        // perche' e' la modalita` fullscreen a prendere il posto di quella
        // decorata, non perche' venga nascosto un controllo.
        //
        // Solo F11 e non un tasto generico: F11 e' il tasto che gia' fa
        // schermo intero in ogni altra applicazione, quindi non si scopre
        // nulla e basta ricordarlo.
        //
        // Esc chiude **senza scrivere niente**. È la via d'uscita della
        // conferma dell'Unregulated: quel pulsante è a due passaggi proprio
        // perché non deve partire un comando per ghiaccio da un tocco
        // distratto, e serve un modo per tornare indietro guardando il
        // pannello senza doverlo cercare. Chiude anche il pannello
        // diagnostica e la cronologia, che per lo stesso motivo non devono
        // stare aperti mentre si guarda altrove.
        let tasti = iced::keyboard::on_key_press(|key, _| match key {
            iced::keyboard::Key::Named(iced::keyboard::key::Named::F11) =>
                Some(Message::ToggleFullscreen),
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) =>
                Some(Message::Annulla),
            _ => None,
        });
        Subscription::batch([tick, ridisegna, resize, tasti])
    }
}
