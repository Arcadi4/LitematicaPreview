use rune::Icon as Glyph;

macro_rules! fluent_icons {
    ($($variant:ident => $asset:literal),+ $(,)?) => {
        pub(crate) enum Icon {
            $($variant),+
        }

        impl Icon {
            pub(crate) fn glyph(self) -> &'static Glyph {
                match self {
                    $(Self::$variant => &Glyph {
                        name: concat!("fluent-", $asset),
                        view: 20.0,
                        body: include_str!(concat!("../icons/", $asset, ".svg")),
                    }),+
                }
            }
        }
    };
}

fluent_icons! {
    OpenFiles => "document_add",
    OpenFolder => "folder_open",
    Dismiss => "dismiss",
    ChevronDown => "chevron_down",
    ArrowUp => "arrow_up",
    Settings => "settings",
    FullScreen => "full_screen_maximize",
    Copy => "copy",
    Refresh => "arrow_sync",
    Home => "home",
    Info => "info",
    Checkmark => "checkmark",
    ArrowDownload => "arrow_download",
    BoxMultiple => "box_multiple",
    ZoomIn => "zoom_in",
    ZoomOut => "zoom_out",
    Grid => "grid",
    Person => "person",
    Save => "save",
    Back => "arrow_left",
    Delete => "delete",
    Cube => "cube",
    Keyboard => "keyboard",
    Gauge => "gauge",
    Color => "color",
    Sun => "weather_sunny",
}
