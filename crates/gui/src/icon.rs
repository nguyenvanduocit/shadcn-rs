//! Lucide icons (ISC licence), embedded as SVG and read once into paths
//! ([`crate::svg`]), and icons whose SVG a design system supplies
//! ([`SvgIcon`]). The renderer rasterizes an icon on the GPU, once per
//! device size and stroke width, into the coverage atlas and tints it in
//! the shader; a scaled or turned icon is drawn as its paths (ADR-0017).

use std::sync::OnceLock;

use crate::svg::{self, Drawing};

macro_rules! icons {
    ($($variant:ident => $file:literal,)*) => {
        /// An icon from the embedded lucide set, or one a design system
        /// supplies as SVG.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($variant,)*
            /// An icon read from SVG its owner supplies (`shadcn`'s icon
            /// libraries), drawn like the embedded ones.
            Svg(&'static SvgIcon),
        }

        impl Icon {
            /// Every embedded icon (not the supplied ones).
            pub const ALL: &[Icon] = &[$(Icon::$variant,)*];

            /// Lucide's name for it, or the name a supplied icon was given.
            #[must_use]
            pub fn name(self) -> &'static str {
                match self {
                    $(Icon::$variant => $file,)*
                    Icon::Svg(i) => i.name,
                }
            }

            fn svg(self) -> &'static str {
                match self {
                    $(Icon::$variant => include_str!(concat!("../assets/icons/", $file, ".svg")),)*
                    Icon::Svg(i) => i.svg,
                }
            }
        }
    };
}

icons! {
    Activity => "activity",
    AlignCenter => "align-center",
    AlignLeft => "align-left",
    AlignRight => "align-right",
    Archive => "archive",
    ArrowDown => "arrow-down",
    ArrowLeft => "arrow-left",
    ArrowRight => "arrow-right",
    ArrowUpDown => "arrow-up-down",
    ArrowUpRight => "arrow-up-right",
    ArrowUp => "arrow-up",
    AudioLines => "audio-lines",
    AudioWaveform => "audio-waveform",
    BadgeCheck => "badge-check",
    Bell => "bell",
    Bold => "bold",
    BookOpenCheck => "book-open-check",
    BookOpen => "book-open",
    BookUser => "book-user",
    Bot => "bot",
    Braces => "braces",
    Brain => "brain",
    Calculator => "calculator",
    CalendarDays => "calendar-days",
    CalendarPlus => "calendar-plus",
    Calendar => "calendar",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    ChevronsUpDown => "chevrons-up-down",
    CircleAlert => "circle-alert",
    CircleCheck => "circle-check",
    CircleDashed => "circle-dashed",
    CircleDot => "circle-dot",
    CircleFadingArrowUp => "circle-fading-arrow-up",
    CircleHelp => "circle-help",
    CircleX => "circle-x",
    Circle => "circle",
    Clock => "clock",
    Cloud => "cloud",
    Code2 => "code-2",
    Command => "command",
    Copy => "copy",
    CornerDownLeft => "corner-down-left",
    CreditCard => "credit-card",
    Dot => "dot",
    Download => "download",
    Ellipsis => "ellipsis",
    ExternalLink => "external-link",
    EyeOff => "eye-off",
    Eye => "eye",
    FileCode => "file-code",
    FilePen => "file-pen",
    FilePlus => "file-plus",
    FileSearch => "file-search",
    FileText => "file-text",
    FileWarning => "file-warning",
    File => "file",
    Filter => "filter",
    FlaskConical => "flask-conical",
    FolderCode => "folder-code",
    FolderSearch => "folder-search",
    Folder => "folder",
    Forward => "forward",
    Frame => "frame",
    GalleryVerticalEnd => "gallery-vertical-end",
    GitBranch => "git-branch",
    GitCommitHorizontal => "git-commit-horizontal",
    Globe => "globe",
    GripVertical => "grip-vertical",
    Heart => "heart",
    House => "house",
    Image => "image",
    Inbox => "inbox",
    Info => "info",
    Italic => "italic",
    Layers => "layers",
    Link2 => "link-2",
    ListChecks => "list-checks",
    ListFilterPlus => "list-filter-plus",
    ListTodo => "list-todo",
    LoaderCircle => "loader-circle",
    Loader => "loader",
    LogOut => "log-out",
    MailCheck => "mail-check",
    Mail => "mail",
    Map => "map",
    MessageCircleDashed => "message-circle-dashed",
    Minus => "minus",
    Moon => "moon",
    OctagonX => "octagon-x",
    PanelLeft => "panel-left",
    Paperclip => "paperclip",
    Pencil => "pencil",
    PieChart => "pie-chart",
    PlusCircle => "plus-circle",
    Plus => "plus",
    Popcorn => "popcorn",
    RefreshCcw => "refresh-ccw",
    RefreshCw => "refresh-cw",
    RotateCcw => "rotate-ccw",
    RotateCw => "rotate-cw",
    Route => "route",
    Search => "search",
    Send => "send",
    Settings2 => "settings-2",
    Settings => "settings",
    Share => "share",
    ShieldAlert => "shield-alert",
    Slash => "slash",
    Smile => "smile",
    Sparkles => "sparkles",
    Square => "square",
    SquareTerminal => "square-terminal",
    Star => "star",
    Sun => "sun",
    Table => "table",
    Tag => "tag",
    Telescope => "telescope",
    Terminal => "terminal",
    ThumbsDown => "thumbs-down",
    ThumbsUp => "thumbs-up",
    Trash2 => "trash-2",
    Trash => "trash",
    TrendingUp => "trending-up",
    TriangleAlert => "triangle-alert",
    Truck => "truck",
    Underline => "underline",
    Upload => "upload",
    UserRoundX => "user-round-x",
    User => "user",
    VolumeOff => "volume-off",
    WrapText => "wrap-text",
    X => "x",
}

/// An icon whose SVG its owner supplies, in the subset [`crate::svg`] reads.
/// It lives in a `static`, and is read into paths once, the first time it is
/// drawn. Two are the same icon when they are the same `static`.
pub struct SvgIcon {
    name: &'static str,
    svg: &'static str,
    drawing: OnceLock<Option<Drawing>>,
}

impl SvgIcon {
    /// The icon `svg` (its `currentColor` is the colour it is drawn in),
    /// named `name` in the inspector and devtools.
    #[must_use]
    pub const fn new(name: &'static str, svg: &'static str) -> Self {
        Self {
            name,
            svg,
            drawing: OnceLock::new(),
        }
    }

    /// Its name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Whether its SVG reads into paths that draw something.
    #[must_use]
    pub fn draws(&'static self) -> bool {
        Icon::Svg(self)
            .drawing()
            .is_some_and(|d| d.parts.iter().any(|p| !p.path.is_empty() && (p.fill || p.stroke)))
    }
}

impl PartialEq for SvgIcon {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for SvgIcon {}

impl std::hash::Hash for SvgIcon {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::ptr::hash(self, state);
    }
}

impl std::fmt::Debug for SvgIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SvgIcon").field(&self.name).finish()
    }
}

impl Icon {
    /// The icon read into paths, once per process, the first time it is
    /// drawn: each icon on its own, so the first icon a page shows does not
    /// read the whole set (about 4 000 allocations, in the frame that shows it).
    pub(crate) fn drawing(self) -> Option<&'static Drawing> {
        if let Icon::Svg(i) = self {
            return i.drawing.get_or_init(|| svg::parse(i.svg).ok()).as_ref();
        }
        static DRAWINGS: [OnceLock<Option<Drawing>>; Icon::ALL.len()] = [const { OnceLock::new() }; Icon::ALL.len()];
        let index = Icon::ALL.iter().position(|i| *i == self)?;
        DRAWINGS
            .get(index)?
            .get_or_init(|| svg::parse(self.svg()).ok())
            .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_icon_reads_into_paths() {
        for &icon in Icon::ALL {
            let d = icon
                .drawing()
                .unwrap_or_else(|| panic!("{} does not read", icon.name()));
            assert!(
                d.parts.iter().any(|p| !p.path.is_empty() && (p.fill || p.stroke)),
                "{} draws nothing",
                icon.name()
            );
        }
    }

    static TICK: SvgIcon = SvgIcon::new(
        "test/tick",
        r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor"><path d="M5 14l3.5 3.5L19 6.5" /></svg>"#,
    );
    static SAME_SVG: SvgIcon = SvgIcon::new("test/same", TICK.svg);
    static BROKEN: SvgIcon = SvgIcon::new("test/broken", "<svg");

    #[test]
    fn a_supplied_icon_reads_once_and_is_itself() {
        assert!(TICK.draws());
        let first = Icon::Svg(&TICK).drawing().map(std::ptr::from_ref);
        assert_eq!(Icon::Svg(&TICK).drawing().map(std::ptr::from_ref), first);
        assert_eq!(Icon::Svg(&TICK).name(), "test/tick");
        assert_eq!(Icon::Svg(&TICK), Icon::Svg(&TICK));
        assert_ne!(Icon::Svg(&TICK), Icon::Svg(&SAME_SVG));
        assert!(!BROKEN.draws());
    }
}
