//! Vector icons on a 24x24 coordinate grid as SVG path data.
//! Strict Rule: NO Unicode emojis are used anywhere in the UI. All visual indicators
//! use crisp vector SVG paths rendered via Direct2D (Windows) and Core Graphics (macOS).

pub const STROKE: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// ALERT (>= 2s): Eye with warning exclamation indicator
    EyeWarning,
    /// ESCALATED (>= 7s): Security shield with alert exclamation mark
    ShieldAlert,
    /// Monitoring / Active protection eye
    EyeMonitoring,
    /// Paused / Camera off eye-slash
    EyeOff,
    /// Manual calibration target reticle
    Calibrate,
    /// Reset tracking arrow
    Reset,
    /// Checkmark for active toggles
    Check,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    pub d: &'static str,
    pub fill: bool,
}

const fn s(d: &'static str) -> Part {
    Part { d, fill: false }
}

const fn f(d: &'static str) -> Part {
    Part { d, fill: true }
}

macro_rules! p {
    ($($part:expr),* $(,)?) => {{
        const PARTS: &[Part] = &[$($part),*];
        PARTS
    }};
}

pub fn parts(icon: Icon) -> &'static [Part] {
    use Icon::*;
    match icon {
        EyeWarning => p![
            // Upper and lower eyelid contour (stroked)
            s("M2.5 12C4.8 7.2 8.2 5 12 5s7.2 2.2 9.5 7c-2.3 4.8-5.7 7-9.5 7s-7.2-2.2-9.5-7Z"),
            // Central exclamation stem and dot inside the eye
            s("M12 9v3.5"),
            f("M12 14.5a1 1 0 1 0 0 2a1 1 0 1 0 0-2Z"),
        ],
        ShieldAlert => p![
            // Shield outline (stroked)
            s("M12 3l8 3.5v5.5c0 5-3.4 8.8-8 10c-4.6-1.2-8-5-8-10V6.5L12 3Z"),
            // Exclamation mark inside shield
            s("M12 8v4.5"),
            f("M12 15a1.1 1.1 0 1 0 0 2.2a1.1 1.1 0 1 0 0-2.2Z"),
        ],
        EyeMonitoring => p![
            s("M2.5 12C4.8 7.2 8.2 5 12 5s7.2 2.2 9.5 7c-2.3 4.8-5.7 7-9.5 7s-7.2-2.2-9.5-7Z"),
            s("M9 12a3 3 0 1 0 6 0a3 3 0 1 0-6 0"),
        ],
        EyeOff => p![
            s("M2.5 12C4.8 7.2 8.2 5 12 5s7.2 2.2 9.5 7c-2.3 4.8-5.7 7-9.5 7s-7.2-2.2-9.5-7Z"),
            s("M4 4l16 16"),
        ],
        Calibrate => p![
            s("M5 12a7 7 0 1 0 14 0a7 7 0 1 0-14 0"),
            s("M12 2v3M12 19v3M2 12h3M19 12h3"),
            f("M10.5 12a1.5 1.5 0 1 0 3 0a1.5 1.5 0 1 0-3 0"),
        ],
        Reset => p![s("M4 12a8 8 0 1 0 2.3-5.6L4 8.5"), s("M4 4v4.5h4.5"),],
        Check => p![s("M5 12.5l4.5 4.5L19 7.5")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svgpath;

    #[test]
    fn all_icons_parse_valid_svg_paths() {
        let all = [
            Icon::EyeWarning,
            Icon::ShieldAlert,
            Icon::EyeMonitoring,
            Icon::EyeOff,
            Icon::Calibrate,
            Icon::Reset,
            Icon::Check,
        ];
        for icon in all {
            for part in parts(icon) {
                let segs = svgpath::parse(part.d).expect("valid SVG path");
                assert!(!segs.is_empty(), "{icon:?} produced empty segments");
            }
        }
    }
}
