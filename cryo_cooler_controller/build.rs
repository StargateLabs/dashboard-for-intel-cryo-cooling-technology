fn main() {
    println!("cargo:rerun-if-changed=icon.png");
    println!("cargo:rerun-if-changed=logo_banner.raw");
    println!("cargo:rerun-if-changed=cryo_warning.png");
    println!("cargo:rerun-if-changed=assets/hero_brand.png");
    println!("cargo:rerun-if-changed=cryo_icon.png");
    println!("cargo:rerun-if-changed=ocp_icon.png");

    let out_dir = std::env::var_os("OUT_DIR").unwrap();

    // Icon 64x64 RGBA → icon.bin
    let image = image::open(concat!(env!("CARGO_MANIFEST_DIR"), "/icon.png"))
        .expect("Failed to open icon.png")
        .into_rgba8()
        .into_raw();
    let icon_path = std::path::Path::new(&out_dir).join("icon.bin");
    std::fs::write(icon_path, image).unwrap();

    // Logo banner: applichiamo un maschera ad angoli arrotondati.
    //
    // Il file .raw è un'immagine RGBA grezza, quindi ha angoli squadrati e
    // si appiccicava male sulla card. Arrotondiamo qui, in build, invece che
    // a runtime: costa una volta sola e non una maschera per frame.
    //
    // Il bordo e' ANTIALIASED tramite copertura parziale: un taglio netto
    // would leave a stair-step aliasing very visible on a dark background.
    // Devono corrispondere ESATTAMENTE a `logo_banner.raw`.
    //
    // Sorgente: `stargate_logo_cyan.png`, il logo **ciano** dell'utente
    // (480x305). Non il verde: quello e' stato un errore e l'originale
    // ciano era sparito insieme ai vecchi `.bin`. Non si ritaglia e non si
    // riscala: il file e' gia' alla risoluzione finale con l'alpha dei
    // bordi intatto.
    const LOGO_W: u32 = 480;
    const LOGO_H: u32 = 305;
    const LOGO_RADIUS: f32 = 28.0;

    let mut logo = image::RgbaImage::from_raw(
        LOGO_W,
        LOGO_H,
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/logo_banner.raw"))
            .expect("Failed to read logo_banner.raw"),
    )
    .expect("logo_banner.raw has unexpected dimensions");

    let (w, h) = (LOGO_W as f32, LOGO_H as f32);
    let r = LOGO_RADIUS.min(w / 2.0).min(h / 2.0);
    // 1px di feather: abbastanza da togliere l'aliasing, poco da far sembrare
    // lo sfondo sfocato.
    let feather = 1.0_f32;

    for y in 0..LOGO_H {
        for x in 0..LOGO_W {
            // Distanza con segno dal rettangolo arrotondato, in pixel.
            //
            // Formule: d = length(max(|p| - b, 0)) - r, dove p e' la
            // posizione RELATIVA AL CENTRO e b e' la meta' del "core"
            // (il rettangolo senza gli angoli tondi).
            //
            // Nota: la distanza va dal CENTRO, non dal bordo. Usare la
            // distanza dal bordo faceva fallire la maschera nell'angolo in
            // alto a sinistra, che restava squadrato.
            let px = x as f32 + 0.5 - w / 2.0;
            let py = y as f32 + 0.5 - h / 2.0;
            let bx = w / 2.0 - r;
            let by = h / 2.0 - r;
            let dx = (px.abs() - bx).max(0.0);
            let dy = (py.abs() - by).max(0.0);
            let dist = (dx * dx + dy * dy).sqrt() - r;

            // dist < 0 -> dentro; dist > 0 -> fuori. La copertura sfuma
            // nell'arco di 1px attorno al bordo.
            let coverage = ((0.5 - dist / feather).clamp(0.0, 1.0) * 255.0) as u8;
            let p = logo.get_pixel_mut(x, y);
            // Moltiplico l'alpha: preserva la trasparenza gia' presente
            // nell'immagine originale invece di forzarla a 255.
            p.0[3] = ((p.0[3] as u32 * coverage as u32) / 255) as u8;
        }
    }

    let logo_path = std::path::Path::new(&out_dir).join("logo_banner.bin");
    std::fs::write(logo_path, logo.into_raw()).unwrap();

    // Logo cryogenic: PNG -> RGBA grezzo, senza maschera. Lo sfondo bianco
    // e' gia' stato reso trasparente, e i bordi del riquadro giallo devono
    // restare netti:Applicare qui la maschera arrotondata di `logo_banner`
    // taglierebbe il bordo giallo, che invece e' gia' arrotondato.
    let warning = image::open(concat!(env!("CARGO_MANIFEST_DIR"), "/cryo_warning.png"))
        .expect("Failed to open cryo_warning.png")
        .to_rgba8();
    let (ww, wh) = (warning.width(), warning.height());
    assert!(
        warning.as_raw().len() as u32 == ww * wh * 4,
        "cryo_warning.png deve essere RGBA grezzo, atteso {} byte",
        ww * wh * 4
    );
    // L'altezza reale va in `CRYO_WARN_H` e la larghezza in `CRYO_WARN_W`, al
    //tranquillo: quando l'app ricava l'altezza dal file, se i due numeri non
    // tornavano l'immagine usciva stirata. Con i due valori presi da qui
    // c'e' una fonte sola.
    assert!(
        (ww, wh) == (631, 405),
        "cryo_warning.png e' {ww}x{wh}, ma main.rs dichiara 631x405: \
         il file e l'asset devono avere le stesse dimensioni"
    );
    let warning_path = std::path::Path::new(&out_dir).join("cryo_warning.bin");
    std::fs::write(warning_path, warning.into_raw()).unwrap();

    // CryoCooling icon 64x64 RGBA
    let cryo_icon = image::open(concat!(env!("CARGO_MANIFEST_DIR"), "/cryo_icon.png"))
        .expect("Failed to open cryo_icon.png")
        .resize(64, 64, image::imageops::FilterType::Lanczos3)
        .into_rgba8()
        .into_raw();
    let cryo_path = std::path::Path::new(&out_dir).join("cryo_icon.bin");
    std::fs::write(cryo_path, cryo_icon).unwrap();

    // OCP Active icon 64x64 RGBA  
    let ocp_icon = image::open(concat!(env!("CARGO_MANIFEST_DIR"), "/ocp_icon.png"))
        .expect("Failed to open ocp_icon.png")
        .resize(64, 64, image::imageops::FilterType::Lanczos3)
        .into_rgba8()
        .into_raw();
    let ocp_path = std::path::Path::new(&out_dir).join("ocp_icon.bin");
    std::fs::write(ocp_path, ocp_icon).unwrap();

    // ── Set di icone monocromatiche ad alta risoluzione ───────────────────
    //
    // Sorgente: Font Awesome Free 6.5.2, stile Solid, icone bianche su
    // sfondo trasparente. Attribuzione: "Font Awesome Free 6.5.2 by
    // Fonticons, Inc. — CC BY 4.0". I glifi sono quadrati e centrati con
    // margine uniforme, quindi si riducono a qualsiasi dimensione senza
    // deformare cornici o proporzioni. Risoluzione di build 128x128:
    // nitida anche a 24-32 px, senza dover caricare il font a runtime.
    const ICON_W: u32 = 128;
    const ICON_H: u32 = 128;
    for name in [
        "thermo", "bolt", "fan", "shield", "cpu", "gauge", "plate", "hazard",
    ] {
        let path = format!("{}/icons/{name}.png", env!("CARGO_MANIFEST_DIR"));
        let img = image::open(&path)
            .unwrap_or_else(|e| panic!("Failed to open {path}: {e}"));
        // Le icone devono nascere quadrate: se una sorgente rettangolare
        // finisce qui dentro, il resize quadrato la storpia in silenzio e
        // la forma torna a essere illeggibile. Meglio fallire in build che
        // pubblicare un set deforme.
        assert_eq!(
            img.width(), img.height(),
            "{path} non e' quadrata: {}x{}",
            img.width(), img.height()
        );
        assert!(
            img.width() >= 128,
            "{path} e' troppo piccola ({} px): servono almeno 128 px per restare nitida a 24 px",
            img.width()
        );
        let rgba = img
            .resize(ICON_W, ICON_H, image::imageops::FilterType::Lanczos3)
            .into_rgba8()
            .into_raw();
        let out = std::path::Path::new(&out_dir).join(format!("icon_{name}.bin"));
        std::fs::write(out, rgba).unwrap();
    }

    // ── Sfondo della dashboard ───────────────────────────────
    //
    // **Nessuna trasparenza applicata all'immagine.** La foto viene usata
    // per quello che e', piena e al 100% di qualita': nessun `resize`
    // (che aggiungerebbe sfocatura su un'immensa), nessuna tinta, nessun
    // alpha. Convertire il JPEG in RGBA non perde nulla, non e' una
    // ricompressione.
    //
    // La trasparenza sta **sopra**, nel vetto di `backdrop()`, ed e' un
    // livello di design a se stante. Mescolarle qui dentro era la cosa
    // sbagliata delle prime prove: rendeva l'immagine dipendente da un
    // valore deciso altrove, e ogni ritocco dell'interfaccia richiedeva
    // di toccare anche la foto.
    //
    // La foto e' 768x1376 e il monitor verticale 1096x1936: rapporto 0,558
    // contro 0,566, quindi `Cover` ritaglia di pochi pixel sui bordi e
    // l'immensa si vede quasi intera.
    let bg_src = format!("{}/assets/dashboard_bg.jpg", env!("CARGO_MANIFEST_DIR"));
    let bg = image::open(&bg_src)
        .unwrap_or_else(|e| panic!("Failed to open {bg_src}: {e}"))
        .into_rgba8();
    let bg_path = std::path::Path::new(&out_dir).join("dashboard_bg.bin");
    std::fs::write(bg_path, bg.into_raw()).unwrap();
    // ── Marchio del prodotto nell'hero ────────────
    //
    // L'originale è 1263x848, ridimensionato a 336x226 con Lanczos3: ben
    // sopra la dimensione reale di display (~112 px di altezza, ~167 px di
    // larghezza), quindi nessun dettaglio perso sui bordi, ma senza portare
    // 4 MB di pixel in memoria per un elemento minuscolo.
    //
    // `resize` va PRIMA di `into_rgba8`, come per le icone: è la firma
    // dell'immagine generica che espone il ridimensionamento, non quella
    // del buffer RGBA.
    let brand_src = format!("{}/assets/hero_brand.png", env!("CARGO_MANIFEST_DIR"));
    let brand = image::open(&brand_src)
        .unwrap_or_else(|e| panic!("Failed to open {brand_src}: {e}"))
        .into_rgba8();

    // **Ritaglio automatico del contenuto utile.**
    //
    // Il file originale e' 1263x848 ma il disegno occupa solo 631x626 al
    // centro: 316 px di margine vuoto a sinistra, 316 a destra, 222 in alto.
    // Circa tre quarti dell'immagine era aria.
    //
    // Senza questo ritaglio l'elemento nella UI e' grande ma quasi vuoto, e
    // il logo sembra non comparire: si vedeva solo il vuoto ai lati. Il
    // ritaglio e' la correzione giusta perche' agisce sull'immagine e non
    // sul layout, quindi non ruba spazio a nessun altro componente — che era
    // il modo sbagliato di risolverlo.
    let ritagliata = {
        let (w, h) = (brand.width(), brand.height());
        let (mut x0, mut y0) = (w, h);
        let (mut x1, mut y1) = (0u32, 0u32);
        for (x, y, px) in brand.enumerate_pixels() {
            if px.0[3] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        // Immagine completamente trasparente: non c'e' niente da ritagliare,
        // e un ritaglio a zero pixel produrrebbe un file illeggibile.
        if x1 >= x0 && y1 >= y0 {
            let x0 = x0.saturating_sub(2);
            let y0 = y0.saturating_sub(2);
            let x1 = (x1 + 2).min(w - 1);
            let y1 = (y1 + 2).min(h - 1);
            Some(image::imageops::crop_imm(
                &brand,
                x0,
                y0,
                x1 - x0 + 1,
                y1 - y0 + 1,
            )
            .to_image())
        } else {
            None
        }
    };
    let brand = ritagliata.unwrap_or(brand);

    // Il ritaglio cambia il rapporto: ora e' quasi quadrato (631x626), non
    // piu' 1,486. Il valore va ricalcolato su quello vero, altrimenti
    // l'immagine viene schiacciata per adattarla a un rapporto sbagliato.
    let brand_raw = image::imageops::resize(
        &brand,
        336,
        336,
        image::imageops::FilterType::Lanczos3,
    )
    .into_raw();
    let brand_path = std::path::Path::new(&out_dir).join("hero_brand.bin");
    std::fs::write(brand_path, brand_raw).unwrap();
}
