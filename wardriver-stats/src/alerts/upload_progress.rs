//! Upload lifecycle types shared by stats counters and the control uploader.

pub const DEST_WIGLE: &str = "wigle";
pub const DEST_WDGWARS: &str = "wdgwars";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadDest {
    Wigle,
    Wdgwars,
}

impl UploadDest {
    pub fn oled_label(self) -> &'static str {
        match self {
            Self::Wigle => "WiGLE",
            Self::Wdgwars => "WDG",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            DEST_WIGLE => Some(Self::Wigle),
            DEST_WDGWARS => Some(Self::Wdgwars),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadProgressKind {
    Connecting,
    Scanning,
    Uploading {
        file_index: u32,
        file_total: u32,
        dest: UploadDest,
        /// HTTP body bytes sent (0-100); 255 when not in the read/write loop.
        byte_pct: u8,
    },
    NothingPending,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadProgress {
    pub kind: UploadProgressKind,
    pub wigle_ok: u32,
    pub wigle_fail: u32,
    pub wdgwars_ok: u32,
    pub wdgwars_fail: u32,
}

impl UploadProgress {
    pub fn emit<F: FnMut(UploadProgress)>(on_progress: &mut Option<F>, p: UploadProgress) {
        if let Some(f) = on_progress.as_mut() {
            f(p);
        }
    }
}
