//! A static vector condensation indicator; never alters TEC control.
use chrono::{DateTime, Utc};
use iced::{widget::canvas::{self, Cache, Geometry, Path, Stroke}, Color, Point};

pub fn active(tec: Option<(DateTime<Utc>, f32)>, dew: Option<(DateTime<Utc>, f32)>, now: DateTime<Utc>) -> bool {
    match (tec,dew) {
        (Some((t,cold)),Some((d,dew))) => t==d && cold.is_finite() && dew.is_finite()
            && cold<dew && (0..=5000).contains(&(now-t).num_milliseconds()),
        _ => false,
    }
}

pub struct Droplets<'a> { pub cache: &'a Cache }
impl canvas::Program<crate::Message> for Droplets<'_> {
    type State=();
    fn draw(&self, _: &(), renderer: &iced::Renderer, _: &iced::Theme,
        bounds: iced::Rectangle, _: iced::mouse::Cursor) -> Vec<Geometry> {
        vec![self.cache.draw(renderer,bounds.size(),|frame| {
            // Three hand-drawn teardrops with sapphire gradients and reflections.
            for (x,y,w,h) in [(4.0,8.0,10.0,14.0),(22.0,1.0,15.0,21.0),(46.0,7.0,11.0,15.0)] {
                let p=|a:f32,b:f32|Point::new(x+a*w,y+b*h);
                let shape=Path::new(|b| {
                    b.move_to(p(0.5,0.0));
                    b.bezier_curve_to(p(0.60,0.25),p(1.0,0.48),p(1.0,0.66));
                    b.bezier_curve_to(p(1.0,1.11),p(0.0,1.11),p(0.0,0.66));
                    b.bezier_curve_to(p(0.0,0.48),p(0.40,0.25),p(0.5,0.0));
                    b.close();
                });
                let gradient=canvas::gradient::Linear::new(p(0.1,0.15),p(0.85,1.0))
                    .add_stop(0.0,Color::from_rgb8(162,235,255))
                    .add_stop(0.38,Color::from_rgb8(38,164,255))
                    .add_stop(0.76,Color::from_rgb8(12,92,215))
                    .add_stop(1.0,Color::from_rgb8(9,47,137));
                frame.fill(&shape,gradient);
                frame.stroke(&shape,Stroke::default().with_color(Color::from_rgba8(121,211,255,0.65)).with_width(0.7));
                let reflection=Path::new(|b| {
                    b.move_to(p(0.40,0.34));
                    b.bezier_curve_to(p(0.29,0.48),p(0.19,0.60),p(0.24,0.71));
                });
                frame.stroke(&reflection,Stroke::default().with_color(Color::from_rgba8(232,251,255,0.9))
                    .with_width(1.4).with_line_cap(canvas::LineCap::Round));
                frame.fill(&Path::circle(p(0.69,0.84),w*0.055),Color::from_rgba8(99,195,255,0.85));
            }
        })]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn strict_threshold_and_valid_sample_required() {
        let now=Utc::now();
        assert!(active(Some((now,15.0)),Some((now,16.0)),now));
        assert!(!active(Some((now,16.0)),Some((now,16.0)),now));
        assert!(!active(Some((now,17.0)),Some((now,16.0)),now));
        assert!(!active(None,Some((now,16.0)),now));
        assert!(!active(Some((now,f32::NAN)),Some((now,16.0)),now));
    }
    #[test] fn stale_or_unpaired_readings_do_not_show_condensation() {
        let now=Utc::now(); let old=now-chrono::Duration::seconds(6);
        assert!(!active(Some((old,15.0)),Some((old,16.0)),now));
        assert!(!active(Some((now,15.0)),Some((old,16.0)),now));
    }
}
