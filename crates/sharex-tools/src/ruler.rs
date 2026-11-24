use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RulerMeasurement {
    pub start: (i32, i32),
    pub end: (i32, i32),
    pub dx: i32,
    pub dy: i32,
    pub width: u32,
    pub height: u32,
    pub distance: f64,
    pub angle_deg: f64,
    pub area: u64,
}

impl RulerMeasurement {
    pub fn between(start: (i32, i32), end: (i32, i32)) -> Self {
        let dx_i64 = i64::from(end.0) - i64::from(start.0);
        let dy_i64 = i64::from(end.1) - i64::from(start.1);
        let dx = end.0.saturating_sub(start.0);
        let dy = end.1.saturating_sub(start.1);
        let width = u32::try_from(dx_i64.unsigned_abs()).unwrap_or(u32::MAX);
        let height = u32::try_from(dy_i64.unsigned_abs()).unwrap_or(u32::MAX);
        let distance = (dx_i64 as f64).hypot(dy_i64 as f64);
        let angle_deg = if dx_i64 == 0 && dy_i64 == 0 {
            0.0
        } else {
            (dy_i64 as f64).atan2(dx_i64 as f64).to_degrees()
        };
        let area = u64::from(width) * u64::from(height);

        Self {
            start,
            end,
            dx,
            dy,
            width,
            height,
            distance,
            angle_deg,
            area,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_three_four_five_right_triangle_accurately() {
        let m = RulerMeasurement::between((10, 20), (13, 24));
        assert_eq!(m.start, (10, 20));
        assert_eq!(m.end, (13, 24));
        assert_eq!(m.dx, 3);
        assert_eq!(m.dy, 4);
        assert_eq!(m.width, 3);
        assert_eq!(m.height, 4);
        assert!((m.distance - 5.0).abs() < 1e-12);
        let expected_angle = (4.0_f64).atan2(3.0).to_degrees();
        assert!((m.angle_deg - expected_angle).abs() < 1e-12);
        assert_eq!(m.area, 12);
    }

    #[test]
    fn handles_negative_deltas_and_cardinal_angles() {
        let reverse = RulerMeasurement::between((10, 10), (7, 6));
        assert_eq!(reverse.dx, -3);
        assert_eq!(reverse.dy, -4);
        assert_eq!(reverse.width, 3);
        assert_eq!(reverse.height, 4);
        assert!((reverse.distance - 5.0).abs() < 1e-12);
        assert_eq!(reverse.area, 12);

        let left = RulerMeasurement::between((5, 5), (0, 5));
        assert!((left.angle_deg - 180.0).abs() < 1e-12);
        assert_eq!(left.area, 0);

        let up = RulerMeasurement::between((5, 5), (5, 0));
        assert!((up.angle_deg - (-90.0)).abs() < 1e-12);

        let zero = RulerMeasurement::between((4, 4), (4, 4));
        assert_eq!(zero.width, 0);
        assert_eq!(zero.height, 0);
        assert_eq!(zero.distance, 0.0);
        assert_eq!(zero.angle_deg, 0.0);
        assert_eq!(zero.area, 0);
    }
}
