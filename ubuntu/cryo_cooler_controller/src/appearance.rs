//! Presentation only. The single application shares this selection with canvas renderers.
use std::sync::atomic::{AtomicBool, Ordering};
use iced::{Color, Theme};

static SUBZERO: AtomicBool = AtomicBool::new(false);
static EARTH: AtomicBool = AtomicBool::new(true);
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Preferences { subzero: bool, earth: bool }
impl Default for Preferences {
    fn default() -> Self { Self { subzero: false, earth: true } }
}
pub const LIME: Color = Color { r: 0.57, g: 1.0, b: 0.0, a: 1.0 };
pub const PANEL: Color = Color { r: 0.020, g: 0.075, b: 0.052, a: 1.0 };

pub fn active() -> bool { SUBZERO.load(Ordering::Relaxed) }
pub fn set(active: bool) { SUBZERO.store(active, Ordering::Relaxed); }
pub fn earth() -> bool { EARTH.load(Ordering::Relaxed) }
pub fn set_earth(enabled: bool) { EARTH.store(enabled, Ordering::Relaxed); }
pub fn background(handle: iced::widget::image::Handle) -> iced::Element<'static, crate::Message> {
    use iced::{widget::{Container, Image, Space}, Length};
    if earth() {
        Image::new(handle).width(Length::Fill).height(Length::Fill)
            .content_fit(iced::ContentFit::Cover).into()
    } else {
        Container::new(Space::new(Length::Fill, Length::Fill))
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Gradient(crate::palette::app_gradient())),
                ..Default::default()
            }).into()
    }
}
fn path() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join("stargate-cryo").join("appearance.json"))
}
pub fn load() {
    let preferences = path().and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice::<Preferences>(&bytes).ok()).unwrap_or_default();
    set(preferences.subzero);
    EARTH.store(preferences.earth, Ordering::Relaxed);
}
pub fn toggle() {
    set(!active());
    save();
}
pub fn toggle_earth() {
    EARTH.store(!earth(), Ordering::Relaxed);
    save();
}
fn save() {
    let save = || -> std::io::Result<()> {
        let Some(path) = path() else { return Ok(()); };
        std::fs::create_dir_all(path.parent().unwrap())?;
        let bytes = serde_json::to_vec(&Preferences { subzero: active(), earth: earth() })?;
        std::fs::write(path, bytes)
    };
    if let Err(error) = save() { eprintln!("Salvataggio tema: {error}"); }
}
pub fn earth_switch() -> iced::widget::Button<'static, crate::Message> {
    iced::widget::button(iced::widget::text(if earth() { "Terra: ON" } else { "Terra: OFF" }).size(10))
        .padding([2, 6]).style(crate::btn::secondary).on_press(crate::Message::ToggleEarth)
}
pub fn theme() -> Theme {
    Theme::custom("SubZero".into(), iced::theme::palette::Palette {
        background: Color::from_rgb8(4, 17, 12), text: Color::from_rgb8(232, 242, 235),
        primary: Color::from_rgb8(45, 224, 172), success: crate::palette::SUCCESS, danger: crate::palette::DANGER,
    })
}
pub fn switch() -> iced::widget::Button<'static, crate::Message> {
    iced::widget::button(iced::widget::text(if active() { "Tema: SubZero" } else { "Tema: attuale" }).size(10))
        .padding([2, 6]).style(crate::btn::secondary).on_press(crate::Message::ToggleAppearance)
}
/// Instrument-panel corner marks, with no extra layout or pointer capture.
pub fn frame<'a>(content: iced::Element<'a, crate::Message>, portion: u16) -> iced::Element<'a, crate::Message> {
    if !active() { return content; }
    iced::widget::Stack::new().push(content)
        .push(iced::widget::canvas(Corners).width(iced::Length::Fill).height(iced::Length::Fill))
        .width(iced::Length::Fill).height(iced::Length::FillPortion(portion)).into()
}
struct Corners;
impl iced::widget::canvas::Program<crate::Message> for Corners {
    type State = ();
    fn draw(&self, _: &(), renderer: &iced::Renderer, _: &Theme, bounds: iced::Rectangle,
        _: iced::mouse::Cursor) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, Path, Stroke};
        let mut frame = Frame::new(renderer, bounds.size());
        let w = bounds.width;
        let h = bounds.height;
        if w < 40.0 || h < 20.0 { return vec![frame.into_geometry()]; }
        let upper = Path::new(|p| {
            p.move_to(iced::Point::new(1.0, 14.0));
            p.line_to(iced::Point::new(1.0, 1.0));
            p.line_to(iced::Point::new(29.0, 1.0));
        });
        frame.stroke(&upper, Stroke::default().with_color(LIME).with_width(1.4));
        let lower = Path::new(|p| {
            p.move_to(iced::Point::new(w-18.0, h-1.0));
            p.line_to(iced::Point::new(w-1.0, h-1.0));
            p.line_to(iced::Point::new(w-1.0, h-9.0));
        });
        frame.stroke(&lower, Stroke::default().with_color(Color::from_rgb8(0, 205, 190)).with_width(1.0));
        vec![frame.into_geometry()]
    }
}
// Red/amber surfaces retain their warning meaning; only neutral chrome is recolored.
fn alert(c: Color) -> bool { c.r > 0.45 && c.r > c.g * 1.25 }
pub fn panel(style: iced::widget::container::Style) -> iced::widget::container::Style {
    panel_for(active(), style)
}
fn panel_for(enabled: bool, mut style: iced::widget::container::Style) -> iced::widget::container::Style {
    if enabled {
        if let Some(iced::Background::Color(c)) = style.background {
            if !alert(c) && !alert(style.border.color) { style.background = Some(Color { a: c.a, ..PANEL }.into()); }
        }
        if style.border.width > 0.0 {
            if !alert(style.border.color) { style.border.color = Color::from_rgba8(92, 183, 139, 0.25); }
            style.border.radius = 12.0.into();
        }
        style.shadow = iced::Shadow::default();
    }
    style
}
pub fn button(style: iced::widget::button::Style) -> iced::widget::button::Style {
    button_for(active(), style)
}
fn button_for(enabled: bool, mut style: iced::widget::button::Style) -> iced::widget::button::Style {
    if enabled {
        style.border.radius = 8.0.into();
        if !alert(style.border.color) { style.border.color = Color::from_rgba8(92, 183, 139, 0.35); }
        if let Some(iced::Background::Color(c)) = style.background {
            if !alert(c) {
                let light = c.r.max(c.g).max(c.b);
                style.background = Some(Color { r: 0.020 + light * 0.035,
                    g: 0.075 + light * 0.09, b: 0.052 + light * 0.06, a: c.a }.into());
            }
        }
        style.shadow = iced::Shadow::default();
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_keeps_warning_and_danger_distinct_from_lime() {
        assert!(alert(crate::palette::DANGER));
        assert!(alert(crate::palette::WARNING));
        assert!(!alert(LIME));
        assert_ne!(theme().palette().danger, LIME);
    }
    #[test]
    fn preference_is_separate_from_controller_configuration() {
        if let Some(p) = path() { assert_eq!(p.file_name().unwrap(), "appearance.json"); }
        let old = serde_json::from_str::<Preferences>("{}").unwrap();
        assert!(!old.subzero && old.earth);
        let saved = serde_json::to_vec(&Preferences { subzero: true, earth: false }).unwrap();
        let restored: Preferences = serde_json::from_slice(&saved).unwrap();
        assert!(restored.subzero && !restored.earth);
    }
    #[test]
    fn original_styles_are_unchanged_when_theme_is_off() {
        let panel = iced::widget::container::Style {
            background: Some(crate::palette::GLASS.into()),
            border: iced::Border { color: crate::palette::BLUE_PRIMARY, width: 1.5, radius: 12.0.into() },
            shadow: crate::palette::soft_shadow(), ..Default::default()
        };
        let original = format!("{panel:?}");
        assert_eq!(format!("{:?}", panel_for(false, panel)), original);
        let button = iced::widget::button::Style { text_color: crate::palette::BLUE_DIM,
            background: Some(crate::palette::GLASS_HI.into()), ..Default::default() };
        let original = format!("{button:?}");
        assert_eq!(format!("{:?}", button_for(false, button)), original);
    }
    #[test]
    fn subzero_preserves_alerts_transparency_and_disabled_text() {
        let warning = iced::widget::container::Style {
            background: Some(crate::palette::DANGER.into()),
            border: iced::Border { color: crate::palette::DANGER, width: 1.0, ..Default::default() },
            ..Default::default()
        };
        let updated = panel_for(true, warning);
        assert_eq!(updated.border.color, crate::palette::DANGER);
        assert_eq!(updated.background, Some(crate::palette::DANGER.into()));
        let transparent = panel_for(true, iced::widget::container::Style {
            background: Some(Color::TRANSPARENT.into()), ..Default::default()
        });
        assert!(matches!(transparent.background, Some(iced::Background::Color(c)) if c.a == 0.0));
        let disabled = button_for(true, iced::widget::button::Style {
            text_color: crate::palette::BLUE_DIM, ..Default::default()
        });
        assert_eq!(disabled.text_color, crate::palette::BLUE_DIM);
    }
}
