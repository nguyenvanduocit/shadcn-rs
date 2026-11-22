//! Hugeicons, the free stroke set (@hugeicons/core-free-icons 4.4.0, MIT). The package ships its glyphs as data, not
//! SVG; each file under `assets/icons/hugeicons/` is that data written as SVG, at stroke width 2 as shadcn-vue's icon
//! loader draws it.

library! {
    "hugeicons";
    glyphs {
        TICK_02 => "tick-02",
        CANCEL_01 => "cancel-01",
        ARROW_DOWN_01 => "arrow-down-01",
        ARROW_UP_01 => "arrow-up-01",
        ARROW_LEFT_01 => "arrow-left-01",
        ARROW_RIGHT_01 => "arrow-right-01",
        ARROW_LEFT_DOUBLE => "arrow-left-double",
        ARROW_RIGHT_DOUBLE => "arrow-right-double",
        UNFOLD_MORE => "unfold-more",
        CHECKMARK_CIRCLE_01 => "checkmark-circle-01",
        INFORMATION_CIRCLE => "information-circle",
        ALERT_02 => "alert-02",
        CANCEL_CIRCLE => "cancel-circle",
        LOADING_03 => "loading-03",
        CIRCLE => "circle",
        COPY_01 => "copy-01",
        MINUS_SIGN => "minus-sign",
        ADD_01 => "add-01",
        SEARCH_01 => "search-01",
        MORE_HORIZONTAL_CIRCLE_01 => "more-horizontal-circle-01",
        SIDEBAR_LEFT => "sidebar-left",
    }
    roles {
        Check => TICK_02,
        Close => CANCEL_01,
        ChevronDown => ARROW_DOWN_01,
        ChevronUp => ARROW_UP_01,
        ChevronLeft => ARROW_LEFT_01,
        ChevronRight => ARROW_RIGHT_01,
        ChevronsLeft => ARROW_LEFT_DOUBLE,
        ChevronsRight => ARROW_RIGHT_DOUBLE,
        Selector => UNFOLD_MORE,
        Success => CHECKMARK_CIRCLE_01,
        Info => INFORMATION_CIRCLE,
        Warning => ALERT_02,
        Error => CANCEL_CIRCLE,
        Loading => LOADING_03,
        Circle => CIRCLE,
        Copy => COPY_01,
        Minus => MINUS_SIGN,
        Plus => ADD_01,
        Search => SEARCH_01,
        More => MORE_HORIZONTAL_CIRCLE_01,
        Sidebar => SIDEBAR_LEFT,
        ArrowDown => super::ARROW_DOWN,
    }
}
