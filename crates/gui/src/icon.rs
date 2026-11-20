//! Lucide icons (ISC licence), embedded as SVG and rasterised with resvg at
//! the size and colour they are drawn in, cached per combination.

use tiny_skia::{Pixmap, Transform};

use crate::cache::FrameCache;
use crate::color::Color;
use crate::convert::f32_to_u32;
use crate::inspect::Raster;

macro_rules! icons {
    ($($variant:ident => $file:literal,)*) => {
        /// An icon from the embedded lucide set.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($variant,)*
        }

        impl Icon {
            /// Every embedded icon.
            pub const ALL: &[Icon] = &[$(Icon::$variant,)*];

            /// Lucide's name for it.
            #[must_use]
            pub fn name(self) -> &'static str {
                match self {
                    $(Icon::$variant => $file,)*
                }
            }

            fn svg(self) -> &'static str {
                match self {
                    $(Icon::$variant => include_str!(concat!("../assets/icons/", $file, ".svg")),)*
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

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct IconKey {
    icon: Icon,
    size: u32,
    rgba: [u8; 4],
    stroke_milli: u32,
}

/// Rasterised icons, kept while the last frames draw them.
#[derive(Default)]
pub struct IconCache {
    images: FrameCache<IconKey, Option<Pixmap>>,
}

impl IconCache {
    /// `icon` as a `size`×`size` physical-pixel image in `color`; `None` when
    /// the SVG cannot be drawn.
    pub fn get(&mut self, icon: Icon, size: u32, color: Color, stroke_width: f32) -> Option<&Pixmap> {
        let key = IconKey {
            icon,
            size,
            rgba: color.to_rgba8(),
            stroke_milli: f32_to_u32(stroke_width * 1000.0),
        };
        self.images
            .get_or_insert_with(key, || rasterise(icon, size, color, stroke_width))
            .as_ref()
    }

    /// Forget the icons the last frames did not draw; call once per frame.
    pub fn end_frame(&mut self) {
        self.images.end_frame();
    }

    /// The icon images held.
    pub(crate) fn footprint(&self) -> Raster {
        Raster::of(self.images.values().flatten())
    }
}

fn rasterise(icon: Icon, size: u32, color: Color, stroke_width: f32) -> Option<Pixmap> {
    let [r, g, b, _] = color.to_rgba8();
    let svg = icon
        .svg()
        .replace("currentColor", &format!("#{r:02x}{g:02x}{b:02x}"))
        .replace("stroke-width=\"2\"", &format!("stroke-width=\"{stroke_width}\""));
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = Pixmap::new(size.max(1), size.max(1))?;
    let scale = crate::convert::u32_to_f32(size) / tree.size().width().max(1.0);
    resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    if color.a < 1.0 {
        for px in pixmap.pixels_mut() {
            let c = px.demultiply();
            let a = crate::convert::unit_to_u8(f32::from(c.alpha()) / 255.0 * color.a);
            *px = tiny_skia::ColorU8::from_rgba(c.red(), c.green(), c.blue(), a).premultiply();
        }
    }
    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_icon_draws() {
        let mut cache = IconCache::default();
        for &icon in Icon::ALL {
            let image = cache.get(icon, 24, Color::BLACK, 2.0);
            let inked = image.is_some_and(|p| p.pixels().iter().any(|px| px.alpha() > 0));
            assert!(inked, "{} draws nothing", icon.name());
        }
    }
}
