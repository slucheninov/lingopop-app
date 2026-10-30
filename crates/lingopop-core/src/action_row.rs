//! Pure width math for the popup action row (port of `Core/ActionRowMetrics.swift`).
//! The text measurer is injected because the mac app measures with `NSFont`.

/// Text of at least this many characters counts as long and prefers `ActionRowMode::Full`.
pub const LONG_TEXT_CHARACTER_THRESHOLD: usize = 120;

/// Icon + icon/label gap + horizontal bezel padding of a small bordered button.
const BUTTON_CHROME: f64 = 38.0;
/// Extra width of the Retry menu's disclosure chevron.
const MENU_INDICATOR_WIDTH: f64 = 14.0;
/// Icon-only small bordered button: glyph plus bezel padding.
const ICON_BUTTON_WIDTH: f64 = 30.0;
const BUTTON_SPACING: f64 = 8.0;
/// Chevron-only menu button: 14 pt glyph frame plus bezel padding.
const CHEVRON_MENU_WIDTH: f64 = 14.0 + 16.0;
/// Gap between a chevron menu and the button it is attached to.
const ATTACHED_SPACING: f64 = 4.0;
/// 2 x the popup content padding.
const ROW_INSET: f64 = 28.0;

/// How the popup's action row renders its buttons. The first `LABELLED_PREFIX` buttons keep
/// their labels in every mode except `Compact`; the rest follow the mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionRowMode {
    /// Every button shows its text label.
    Full,
    /// The first two buttons keep their labels, the rest are icon-only.
    Mixed,
    /// Last-resort fallback: icons only.
    Compact,
}

impl ActionRowMode {
    /// Number of leading buttons that stay labelled (as long as they fit).
    pub const LABELLED_PREFIX: usize = 2;

    /// Whether the button at `index` (0-based, row order, Retry menu last) is icon-only.
    pub fn is_icon_only(self, index: usize) -> bool {
        match self {
            ActionRowMode::Full => false,
            ActionRowMode::Mixed => index >= Self::LABELLED_PREFIX,
            ActionRowMode::Compact => true,
        }
    }

    /// The mode to try first: long text (at least `threshold` characters) prefers `Full`,
    /// shorter text `Mixed`. With no button beyond the labelled prefix the two rows are
    /// identical, so `Full` is returned.
    pub fn preferred(character_count: usize, threshold: usize, button_count: usize) -> Self {
        if button_count <= Self::LABELLED_PREFIX || character_count >= threshold {
            ActionRowMode::Full
        } else {
            ActionRowMode::Mixed
        }
    }

    /// Fallback chain, widest first.
    pub fn candidates(self) -> &'static [ActionRowMode] {
        match self {
            ActionRowMode::Full => &[
                ActionRowMode::Full,
                ActionRowMode::Mixed,
                ActionRowMode::Compact,
            ],
            ActionRowMode::Mixed => &[ActionRowMode::Mixed, ActionRowMode::Compact],
            ActionRowMode::Compact => &[ActionRowMode::Compact],
        }
    }
}

/// Width needed to show the row in `mode`. `titles` are the plain buttons in row order,
/// `menu_title` is the Retry menu if shown (after the plain buttons), `chevron_menus` counts
/// chevron-only menus attached to the last plain button. `text_width` measures a label;
/// each result is rounded up like Swift's `ceil`.
pub fn row_width(
    titles: &[&str],
    menu_title: Option<&str>,
    chevron_menus: usize,
    mode: ActionRowMode,
    text_width: &dyn Fn(&str) -> f64,
) -> f64 {
    let labelled = |title: &str| text_width(title).ceil() + BUTTON_CHROME;
    let mut widths: Vec<f64> = titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            if mode.is_icon_only(index) {
                ICON_BUTTON_WIDTH
            } else {
                labelled(title)
            }
        })
        .collect();
    if chevron_menus > 0 {
        if let Some(last) = widths.last_mut() {
            *last += chevron_menus as f64 * (CHEVRON_MENU_WIDTH + ATTACHED_SPACING);
        }
    }
    if let Some(menu_title) = menu_title {
        let base = if mode.is_icon_only(titles.len()) {
            ICON_BUTTON_WIDTH
        } else {
            labelled(menu_title)
        };
        widths.push(base + MENU_INDICATOR_WIDTH);
    }
    if widths.is_empty() {
        return 0.0;
    }
    // The rows put a spacer before Retry, and the stack spaces it like any other child, so
    // there is one gap more than between the buttons alone.
    widths.iter().sum::<f64>() + BUTTON_SPACING * widths.len() as f64 + ROW_INSET
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(s: &str) -> f64 {
        s.chars().count() as f64 * 6.0
    }

    fn width(titles: &[&str], menu: Option<&str>, chevrons: usize, mode: ActionRowMode) -> f64 {
        row_width(titles, menu, chevrons, mode, &measure)
    }

    const EN: [&str; 4] = ["Copy", "Replace", "Rewrite", "Explain"];
    const UK: [&str; 4] = ["Копіювати", "Замінити", "Переписати", "Пояснити"];

    #[test]
    fn longer_titles_need_more_width() {
        let en = width(&EN, Some("Retry"), 0, ActionRowMode::Full);
        let uk = width(&UK, Some("Спробувати ще раз"), 0, ActionRowMode::Full);
        assert!(uk > en);
    }

    #[test]
    fn exact_full_width_arithmetic() {
        // ["Copy","Replace"] + menu "Retry": (24+38) + (42+38) + (30+14 -> 5*6+38+14=82)
        // = 62 + 80 + 82 = 224, plus 8 * 3 spacing and 28 inset.
        assert_eq!(
            width(&["Copy", "Replace"], Some("Retry"), 0, ActionRowMode::Full),
            224.0 + 24.0 + 28.0
        );
    }

    #[test]
    fn text_width_is_rounded_up_like_swift_ceil() {
        let fractional = |s: &str| s.chars().count() as f64 * 5.5;
        // "Copy" = 22.0 exactly; "Rep" = 16.5 -> 17.
        let w = row_width(&["Rep"], None, 0, ActionRowMode::Full, &fractional);
        assert_eq!(w, 17.0 + 38.0 + 8.0 + 28.0);
    }

    #[test]
    fn width_shrinks_from_full_to_mixed_to_compact() {
        for (titles, retry) in [(EN, "Retry"), (UK, "Спробувати ще раз")] {
            let full = width(&titles, Some(retry), 0, ActionRowMode::Full);
            let mixed = width(&titles, Some(retry), 0, ActionRowMode::Mixed);
            let compact = width(&titles, Some(retry), 0, ActionRowMode::Compact);
            assert!(mixed < full);
            assert!(compact < mixed);
        }
    }

    #[test]
    fn mixed_width_uses_labels_only_for_first_two_buttons() {
        let a = width(&EN, Some("Retry"), 0, ActionRowMode::Mixed);
        let b = width(
            &[
                "Copy",
                "Replace",
                "A much longer rewrite",
                "Explain everything",
            ],
            Some("Retry again later"),
            0,
            ActionRowMode::Mixed,
        );
        assert_eq!(a, b);
        let c = width(
            &["Copy", "Replace it all", "Rewrite", "Explain"],
            Some("Retry"),
            0,
            ActionRowMode::Mixed,
        );
        assert!(c > a);
    }

    #[test]
    fn compact_width_ignores_titles() {
        let a = width(&["a", "b", "c"], Some("x"), 0, ActionRowMode::Compact);
        let b = width(
            &["long title", "another long", "third long"],
            Some("retry long"),
            0,
            ActionRowMode::Compact,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn chevron_menu_widens_row() {
        let titles = ["Copy", "Translate"];
        let without = width(&titles, Some("Retry"), 0, ActionRowMode::Full);
        let with = width(&titles, Some("Retry"), 1, ActionRowMode::Full);
        assert_eq!(with - without, 14.0 + 16.0 + 4.0);
    }

    #[test]
    fn empty_row_is_zero() {
        assert_eq!(width(&[], None, 0, ActionRowMode::Full), 0.0);
    }

    #[test]
    fn mixed_equals_full_when_only_the_labelled_prefix_exists() {
        let titles = ["Copy", "Translate"];
        assert_eq!(
            width(&titles, None, 1, ActionRowMode::Mixed),
            width(&titles, None, 1, ActionRowMode::Full)
        );
    }

    #[test]
    fn mixed_ocr_row_keeps_chevron_and_icon_retry() {
        let titles = ["Copy", "Translate"];
        let mixed = width(&titles, Some("Retry"), 1, ActionRowMode::Mixed);
        let full = width(&titles, Some("Retry"), 1, ActionRowMode::Full);
        let no_chevron = width(&titles, Some("Retry"), 0, ActionRowMode::Mixed);
        assert!(mixed < full);
        assert!(mixed > no_chevron);
    }

    #[test]
    fn icon_only_index_rules() {
        assert!(!ActionRowMode::Full.is_icon_only(0));
        assert!(!ActionRowMode::Full.is_icon_only(5));
        assert!(!ActionRowMode::Mixed.is_icon_only(0));
        assert!(!ActionRowMode::Mixed.is_icon_only(1));
        assert!(ActionRowMode::Mixed.is_icon_only(2));
        assert!(ActionRowMode::Mixed.is_icon_only(5));
        assert!(ActionRowMode::Compact.is_icon_only(0));
    }

    #[test]
    fn preferred_mode_follows_threshold() {
        let t = LONG_TEXT_CHARACTER_THRESHOLD;
        assert_eq!(t, 120);
        assert_eq!(ActionRowMode::preferred(0, t, 5), ActionRowMode::Mixed);
        assert_eq!(ActionRowMode::preferred(119, t, 5), ActionRowMode::Mixed);
        assert_eq!(ActionRowMode::preferred(120, t, 5), ActionRowMode::Full);
        assert_eq!(ActionRowMode::preferred(5000, t, 3), ActionRowMode::Full);
    }

    #[test]
    fn preferred_is_full_when_no_button_beyond_prefix() {
        assert_eq!(ActionRowMode::preferred(10, 120, 2), ActionRowMode::Full);
        assert_eq!(ActionRowMode::preferred(10, 120, 1), ActionRowMode::Full);
    }

    #[test]
    fn fallback_chains() {
        use ActionRowMode::*;
        assert_eq!(Full.candidates(), &[Full, Mixed, Compact]);
        assert_eq!(Mixed.candidates(), &[Mixed, Compact]);
        assert_eq!(Compact.candidates(), &[Compact]);
    }

    #[test]
    fn labelled_prefix_is_two() {
        assert_eq!(ActionRowMode::LABELLED_PREFIX, 2);
    }
}
