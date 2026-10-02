//! Gen 1 setpoint feedback. Opcode 0x1d is not a verified power limiter on
//! this board: measured 250 W after requesting 30%. PCB is not hot-side TEC.
#[derive(Default)]
pub struct Regulator {
    last_action: Option<f64>,
    pub status: &'static str,
    baseline: Option<(f32, f32, f32, f32)>, // cold, watts, board, previous offset
    settle_until: f64,
    hold_until: f64,
}
impl Regulator {
    pub fn reset(&mut self) { *self = Self::default(); }
    #[cfg(test)]
    pub fn update(&mut self, now: f64, cold: f32, dew: f32, board: f32,
        watts: f32, offset: f32, budget: f32, board_limit: f32) -> Option<f32> {
        self.update_with_margin(now,cold,dew,board,watts,offset,budget,board_limit,3.5)
    }
    pub fn update_with_margin(&mut self, now:f64,cold:f32,dew:f32,board:f32,
        watts:f32,offset:f32,budget:f32,board_limit:f32,target_margin:f32)->Option<f32> {
        self.update_with_cpu(now,cold,dew,board,watts,offset,budget,board_limit,target_margin,None)
    }
    pub fn update_with_cpu(&mut self, now:f64,cold:f32,dew:f32,board:f32,
        watts:f32,offset:f32,budget:f32,board_limit:f32,target_margin:f32,cpu:Option<f32>)->Option<f32> {
        // Cooling priority hint, not a processor-specific shutdown threshold.
        let hot_cpu=cpu.is_some_and(|t| t.is_finite() && (85.0..=150.0).contains(&t));
        if ![now, cold as f64, dew as f64, board as f64, watts as f64, offset as f64, budget as f64, board_limit as f64, target_margin as f64].iter().all(|v| v.is_finite())
            || !(0.0..=200.0).contains(&budget) || !(2.0..=20.0).contains(&target_margin)
            || !(-40.0..=100.0).contains(&cold) || !(-40.0..=60.0).contains(&dew)
            || !(-40.0..=150.0).contains(&board) || !(0.0..=1000.0).contains(&watts) {
            self.reset(); self.status = "Sensori non validi: nessun aumento"; return None;
        }
        if self.last_action.is_some_and(|t| now < t) { self.reset(); }
        let margin = cold - dew;
        let over_budget = watts > budget + 5.0;
        let protection = margin < 1.5 || board >= board_limit || over_budget;
        let physical_protection = margin < 1.5 || board >= board_limit;
        let interval = if physical_protection { 2.0 } else if over_budget { 30.0 } else { 10.0 };
        if self.last_action.is_some_and(|t| now-t < interval) { return None; }
        let mut next = offset;
        if protection {
            // Larger offset requests less cooling. Feedback measures the result;
            // there is no assumption about the absolute reference temperature.
            next += if board >= board_limit { 2.0 } else { 1.0 };
            self.baseline = None; self.settle_until = now + 30.0; self.hold_until = now + 60.0;
            self.status = "Riduce domanda: watt / controller / condensa";
        } else if now < self.settle_until && !(hot_cpu && watts < budget - 10.0 && margin > target_margin + 0.75) { return None; }
        else {
            if let Some((before_cold, before_watts, before_board, previous_offset)) = self.baseline.take() {
                let added_watts = watts - before_watts;
                let improvement = before_cold - cold;
                if !hot_cpu && added_watts > 5.0 && (0.0..0.15).contains(&improvement) && board >= before_board {
                    next = previous_offset;
                    self.hold_until = now + 120.0;
                    self.status = "Annulla aumento: watt senza freddo misurabile";
                }
            }
            if next == offset {
                if margin < target_margin - 0.75 {
                    next += 0.5;
                    self.status = "Risparmio: piastra sotto obiettivo del profilo";
                } else if margin > target_margin + 0.75 && (hot_cpu || now >= self.hold_until || watts < budget * 0.5) && watts < budget - 10.0 {
                    next -= if hot_cpu { 1.0 } else { 0.5 };
                    self.baseline = Some((cold, watts, board, offset));
                    self.settle_until = now + 30.0;
                    self.status = "Aumenta freddo: verifica risposta per 30 s";
                } else { self.status = "Mantiene equilibrio termico e consumo"; }
            }
        }
        next = next.clamp(-30.0, 50.0);
        if next != offset { self.last_action = Some(now); self.settle_until = now + if hot_cpu && !protection { 10.0 } else { 30.0 }; Some(next) } else { None }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn measured_watts_override_fake_percentage() {
        assert_eq!(Regulator::default().update(0.0,30.0,15.0,28.0,250.0,-3.0,200.0,36.0),Some(-2.0));
    }
    #[test] fn cooling_demand_increases_gradually() {
        let mut r=Regulator::default();
        assert_eq!(r.update(0.0,25.0,15.0,28.0,50.0,-3.0,200.0,36.0),Some(-3.5));
        assert_eq!(r.update(5.0,25.0,15.0,28.0,55.0,-3.5,200.0,36.0),None);
    }
    #[test] fn board_heating_does_not_count_as_cooling() {
        let mut r=Regulator::default();
        r.update(0.0,25.0,15.0,28.0,50.0,-3.0,200.0,36.0);
        assert_eq!(r.update(31.0,25.0,15.0,29.0,80.0,-3.5,200.0,36.0),Some(-3.0));
    }
    #[test] fn condensation_wins_over_cooling() {
        assert_eq!(Regulator::default().update(0.0,15.5,15.0,28.0,30.0,-3.0,200.0,36.0),Some(-2.0));
    }
    #[test] fn invalid_data_never_increases_demand() {
        assert_eq!(Regulator::default().update(0.0,f32::NAN,15.0,28.0,30.0,-3.0,200.0,36.0),None);
    }
    #[test] fn temperature_limit_wins() {
        assert_eq!(Regulator::default().update(0.0,30.0,15.0,36.0,30.0,-3.0,200.0,36.0),Some(-1.0));
    }
    #[test] fn profiles_choose_different_demand_for_same_measurement() {
        let mut silent=Regulator::default(); let mut ai=Regulator::default();
        assert_eq!(silent.update_with_margin(0.0,20.0,15.0,28.0,30.0,3.0,60.0,66.0,6.0),Some(3.5));
        assert_eq!(ai.update_with_margin(0.0,20.0,15.0,28.0,30.0,3.0,160.0,66.0,3.0),Some(2.5));
    }
    #[test] fn hysteresis_holds_through_small_temperature_changes() {
        let mut r=Regulator::default();
        for (t,margin) in [(0.0,3.0),(20.0,3.5),(40.0,4.0),(60.0,3.2)] {
            assert_eq!(r.update_with_margin(t,15.0+margin,15.0,28.0,40.0,3.0,120.0,66.0,3.5),None);
        }
    }
    #[test] fn budget_reduction_does_not_immediately_reverse() {
        let mut r=Regulator::default();
        assert_eq!(r.update_with_margin(0.0,25.0,15.0,28.0,150.0,3.0,120.0,66.0,3.5),Some(4.0));
        assert_eq!(r.update_with_margin(31.0,25.0,15.0,28.0,80.0,4.0,120.0,66.0,3.5),None);
    }
    #[test] fn budget_correction_waits_for_response() {
        let mut r=Regulator::default();
        assert_eq!(r.update(0.0,34.0,15.0,30.0,230.0,18.0,200.0,66.0),Some(19.0));
        assert_eq!(r.update(2.0,34.0,15.0,30.0,230.0,19.0,200.0,66.0),None);
    }
    #[test] fn hot_cpu_recovers_during_hold_but_respects_condensation() {
        let mut r=Regulator::default();
        r.update(0.0,34.0,15.0,30.0,230.0,18.0,200.0,66.0);
        assert_eq!(r.update_with_cpu(10.0,34.0,15.0,30.0,36.0,19.0,200.0,66.0,3.5,Some(90.0)),Some(18.0));
        assert_eq!(r.update_with_cpu(12.0,15.5,15.0,30.0,36.0,18.0,200.0,66.0,3.5,Some(90.0)),Some(19.0));
    }
    #[test] fn low_power_recovers_after_settling_instead_of_long_hold() {
        let mut r=Regulator::default();
        r.update(0.0,34.0,15.0,30.0,230.0,18.0,200.0,66.0);
        assert_eq!(r.update(31.0,34.0,15.0,30.0,36.0,19.0,200.0,66.0),Some(18.5));
    }
}



