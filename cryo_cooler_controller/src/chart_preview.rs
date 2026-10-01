//! Isolated visual preview. No serial port, config writes, database or tray.
use crate::{charts::ChartGroup, Message};
use chrono::{Duration, Utc};
use iced::{Element, Subscription, Task, Theme};

struct Preview {
    charts: ChartGroup,
    started: std::time::Instant,
    wide: bool,
}
impl Default for Preview {
    fn default() -> Self {
        let mut charts = ChartGroup::default();
        let now = Utc::now();
        for i in 0..360 {
            sample(&mut charts, now - Duration::milliseconds((360-i)*500), i as f32*0.05);
        }
        Self { charts, started: std::time::Instant::now(), wide: std::env::args().any(|a|a=="--wide") }
    }
}
fn sample(charts: &mut ChartGroup, timestamp: chrono::DateTime<Utc>, phase: f32) {
    let cold=if std::env::args().any(|a|a=="--condensa") || std::env::current_exe().ok().is_some_and(|p|p.to_string_lossy().contains("Condensa-Preview")) {13.0+phase.sin()} else {20.0+phase.sin()*1.5};
    let dew=16.0+(phase*0.3).sin();
    let watts=60.0+(phase*1.3).sin()*25.0;
    charts.update(cryo_cooler_controller_lib::MonitoringData {
        timestamp, tec_temperature:cold, pcb_temperature:28.0+phase.cos(),
        humidity:48.0+phase.sin()*2.0, dew_point_temperature:dew,
        tec_voltage:10.0, tec_current:watts/10.0, tec_power_level:45,
        tec_power_watts:watts, condensation_margin:cold-dew,
    });
    charts.update_cpu_temp(48.0+(phase*0.7).sin()*12.0);
}
impl Preview {
    fn update(&mut self, message: Message) -> Task<Message> {
        if self.started.elapsed().as_secs()>120 { return iced::exit(); }
        if matches!(message,Message::Tick) {
            sample(&mut self.charts,Utc::now(),18.0+self.started.elapsed().as_secs_f32()*0.1);
        }
        Task::none()
    }
    fn view(&self) -> Element<'_, Message> {
        let graphs=if self.wide {self.charts.view_wide()} else {self.charts.view()};
        iced::widget::container(graphs).padding(12).into()
    }
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(std::time::Duration::from_millis(crate::charts::ANIMATION_MS)).map(|_|Message::RidisegnaGrafici),
            iced::time::every(std::time::Duration::from_millis(500)).map(|_|Message::Tick),
        ])
    }
}
pub fn run() {
    let wide=std::env::args().any(|a|a=="--wide");
    let _=iced::application("Anteprima grafici R8 - DATI SIMULATI",Preview::update,Preview::view)
        .theme(|_|Theme::Dark).subscription(Preview::subscription)
        .default_font(iced::Font::with_name("Segoe UI"))
        .antialiasing(false)
        .window_size(if wide {iced::Size::new(1400.0,900.0)} else {iced::Size::new(660.0,1000.0)})
        .run();
}
