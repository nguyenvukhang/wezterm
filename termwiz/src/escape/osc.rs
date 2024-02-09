use crate::color::SrgbaTuple;
pub use crate::hyperlink::Hyperlink;
use crate::{bail, ensure, Result};
use base64::Engine;
use bitflags::bitflags;
use num_derive::*;
use num_traits::FromPrimitive;
use std::collections::HashMap;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::str;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq)]
pub enum ColorOrQuery {
    Color(SrgbaTuple),
    Query,
}

impl Display for ColorOrQuery {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        match self {
            ColorOrQuery::Query => write!(f, "?"),
            ColorOrQuery::Color(c) => write!(f, "{}", c.to_x11_16bit_rgb_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum OperatingSystemCommand {
    SetIconNameAndWindowTitle(String),
    SetWindowTitle(String),
    SetWindowTitleSun(String),
    SetIconName(String),
    SetIconNameSun(String),
    ClearSelection(Selection),
    QuerySelection(Selection),
    SetSelection(Selection, String),
    SystemNotification(String),
    FinalTermSemanticPrompt(FinalTermSemanticPrompt),
    ChangeColorNumber(Vec<ChangeColorPair>),
    ChangeDynamicColors(DynamicColorNumber, Vec<ColorOrQuery>),
    ResetDynamicColor(DynamicColorNumber),
    CurrentWorkingDirectory(String),
    ResetColors(Vec<u8>),
    RxvtExtension(Vec<String>),

    Unspecified(Vec<Vec<u8>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromPrimitive)]
#[repr(u8)]
pub enum DynamicColorNumber {
    TextForegroundColor = 10,
    TextBackgroundColor = 11,
    TextCursorColor = 12,
    MouseForegroundColor = 13,
    MouseBackgroundColor = 14,
    TektronixForegroundColor = 15,
    TektronixBackgroundColor = 16,
    HighlightBackgroundColor = 17,
    TektronixCursorColor = 18,
    HighlightForegroundColor = 19,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChangeColorPair {
    pub palette_index: u8,
    pub color: ColorOrQuery,
}

bitflags! {
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection :u16{
    const NONE = 0;
    const CLIPBOARD = 1<<1;
    const PRIMARY=1<<2;
    const SELECT=1<<3;
    const CUT0=1<<4;
    const CUT1=1<<5;
    const CUT2=1<<6;
    const CUT3=1<<7;
    const CUT4=1<<8;
    const CUT5=1<<9;
    const CUT6=1<<10;
    const CUT7=1<<11;
    const CUT8=1<<12;
    const CUT9=1<<13;
}
}

impl Selection {
    fn try_parse(buf: &[u8]) -> Result<Selection> {
        if buf == b"" {
            Ok(Selection::SELECT | Selection::CUT0)
        } else {
            let mut s = Selection::NONE;
            for c in buf {
                s |= match c {
                    b'c' => Selection::CLIPBOARD,
                    b'p' => Selection::PRIMARY,
                    b's' => Selection::SELECT,
                    b'0' => Selection::CUT0,
                    b'1' => Selection::CUT1,
                    b'2' => Selection::CUT2,
                    b'3' => Selection::CUT3,
                    b'4' => Selection::CUT4,
                    b'5' => Selection::CUT5,
                    b'6' => Selection::CUT6,
                    b'7' => Selection::CUT7,
                    b'8' => Selection::CUT8,
                    b'9' => Selection::CUT9,
                    _ => bail!("invalid selection {:?}", buf),
                }
            }
            Ok(s)
        }
    }
}

impl Display for Selection {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        macro_rules! item {
            ($variant:ident, $s:expr) => {
                if (*self & Selection::$variant) != Selection::NONE {
                    write!(f, $s)?;
                }
            };
        }

        item!(CLIPBOARD, "c");
        item!(PRIMARY, "p");
        item!(SELECT, "s");
        item!(CUT0, "0");
        item!(CUT1, "1");
        item!(CUT2, "2");
        item!(CUT3, "3");
        item!(CUT4, "4");
        item!(CUT5, "5");
        item!(CUT6, "6");
        item!(CUT7, "7");
        item!(CUT8, "8");
        item!(CUT9, "9");
        Ok(())
    }
}

impl OperatingSystemCommand {
    pub fn parse(osc: &[&[u8]]) -> Self {
        Self::internal_parse(osc).unwrap_or_else(|err| {
            let mut vec = Vec::new();
            for slice in osc {
                vec.push(slice.to_vec());
            }
            log::trace!(
                "OSC internal parse err: {}, track as Unspecified {:?}",
                err,
                vec
            );
            OperatingSystemCommand::Unspecified(vec)
        })
    }

    fn parse_selection(osc: &[&[u8]]) -> Result<Self> {
        if osc.len() == 2 {
            Selection::try_parse(osc[1]).map(OperatingSystemCommand::ClearSelection)
        } else if osc.len() == 3 && osc[2] == b"?" {
            Selection::try_parse(osc[1]).map(OperatingSystemCommand::QuerySelection)
        } else if osc.len() == 3 {
            let sel = Selection::try_parse(osc[1])?;
            let bytes = base64_decode(osc[2])?;
            let s = String::from_utf8(bytes)?;
            Ok(OperatingSystemCommand::SetSelection(sel, s))
        } else {
            bail!("unhandled OSC 52: {:?}", osc);
        }
    }

    fn parse_reset_colors(osc: &[&[u8]]) -> Result<Self> {
        let mut colors = vec![];
        let mut iter = osc.iter();
        iter.next(); // skip the command word that we already know is present

        while let Some(index) = iter.next() {
            if index.is_empty() {
                continue;
            }
            let index: u8 = str::from_utf8(index)?.parse()?;
            colors.push(index);
        }

        Ok(OperatingSystemCommand::ResetColors(colors))
    }

    fn parse_change_color_number(osc: &[&[u8]]) -> Result<Self> {
        let mut pairs = vec![];
        let mut iter = osc.iter();
        iter.next(); // skip the command word that we already know is present

        while let (Some(index), Some(spec)) = (iter.next(), iter.next()) {
            let index: u8 = str::from_utf8(index)?.parse()?;
            let spec = str::from_utf8(spec)?;
            let spec = if spec == "?" {
                ColorOrQuery::Query
            } else {
                ColorOrQuery::Color(
                    SrgbaTuple::from_str(spec)
                        .map_err(|()| format!("invalid color spec {:?}", spec))?,
                )
            };

            pairs.push(ChangeColorPair {
                palette_index: index,
                color: spec,
            });
        }

        Ok(OperatingSystemCommand::ChangeColorNumber(pairs))
    }

    fn parse_reset_dynamic_color_number(idx: u8) -> Result<Self> {
        let which_color: DynamicColorNumber = FromPrimitive::from_u8(idx)
            .ok_or_else(|| format!("osc code is not a valid DynamicColorNumber!?"))?;

        Ok(OperatingSystemCommand::ResetDynamicColor(which_color))
    }

    fn parse_change_dynamic_color_number(idx: u8, osc: &[&[u8]]) -> Result<Self> {
        let which_color: DynamicColorNumber = FromPrimitive::from_u8(idx)
            .ok_or_else(|| format!("osc code is not a valid DynamicColorNumber!?"))?;
        let mut colors = vec![];
        for spec in osc.iter().skip(1) {
            if spec == b"?" {
                colors.push(ColorOrQuery::Query);
            } else {
                let spec = str::from_utf8(spec)?;
                colors.push(ColorOrQuery::Color(
                    SrgbaTuple::from_str(spec)
                        .map_err(|()| format!("invalid color spec {:?}", spec))?,
                ));
            }
        }

        Ok(OperatingSystemCommand::ChangeDynamicColors(
            which_color,
            colors,
        ))
    }

    fn internal_parse(osc: &[&[u8]]) -> Result<Self> {
        ensure!(!osc.is_empty(), "no params");
        let p1str = String::from_utf8_lossy(osc[0]);

        if p1str.is_empty() {
            bail!("zero length osc");
        }

        // Ugh, this is to handle "OSC ltitle" which is a legacyish
        // OSC for encoding a window title change request.  These days
        // OSC 2 is preferred for this purpose, but we need to support
        // generating and parsing the legacy form because it is the
        // response for the CSI ReportWindowTitle.
        // So, for non-numeric OSCs, we look up the prefix and use that.
        // This only works if the non-numeric OSC code has length == 1.
        let osc_code = if !p1str.chars().nth(0).unwrap().is_ascii_digit() && osc.len() == 1 {
            let mut p1 = String::new();
            p1.push(p1str.chars().nth(0).unwrap());
            OperatingSystemCommandCode::from_code(&p1)
        } else {
            OperatingSystemCommandCode::from_code(&p1str)
        }
        .ok_or_else(|| format!("unknown code"))?;

        macro_rules! single_string {
            ($variant:ident) => {{
                if osc.len() != 2 {
                    bail!("wrong param count");
                }
                let s = String::from_utf8(osc[1].to_vec())?;
                Ok(OperatingSystemCommand::$variant(s))
            }};
        }

        macro_rules! single_title_string {
            ($variant:ident) => {{
                if osc.len() < 2 {
                    bail!("wrong param count");
                }
                let mut s = String::from_utf8(osc[1].to_vec())?;
                for i in 2..osc.len() {
                    s = [s, String::from_utf8(osc[i].to_vec())?].join(";");
                }

                Ok(OperatingSystemCommand::$variant(s))
            }};
        }

        use self::OperatingSystemCommandCode::*;
        match osc_code {
            SetIconNameAndWindowTitle => single_title_string!(SetIconNameAndWindowTitle),
            SetWindowTitle => single_title_string!(SetWindowTitle),
            SetWindowTitleSun => Ok(OperatingSystemCommand::SetWindowTitleSun(
                p1str[1..].to_owned(),
            )),

            SetIconName => single_title_string!(SetIconName),
            SetIconNameSun => Ok(OperatingSystemCommand::SetIconNameSun(
                p1str[1..].to_owned(),
            )),
            ManipulateSelectionData => Self::parse_selection(osc),
            SystemNotification => single_string!(SystemNotification),
            SetCurrentWorkingDirectory => single_string!(CurrentWorkingDirectory),
            RxvtProprietary => {
                let mut vec = vec![];
                for slice in osc.iter().skip(1) {
                    vec.push(String::from_utf8_lossy(slice).to_string());
                }
                Ok(OperatingSystemCommand::RxvtExtension(vec))
            }
            FinalTermSemanticPrompt => self::FinalTermSemanticPrompt::parse(osc)
                .map(OperatingSystemCommand::FinalTermSemanticPrompt),
            ChangeColorNumber => Self::parse_change_color_number(osc),
            ResetColors => Self::parse_reset_colors(osc),

            ResetSpecialColor
            | ResetTextForegroundColor
            | ResetTextBackgroundColor
            | ResetTextCursorColor
            | ResetMouseForegroundColor
            | ResetMouseBackgroundColor
            | ResetTektronixForegroundColor
            | ResetTektronixBackgroundColor
            | ResetHighlightColor
            | ResetTektronixCursorColor
            | ResetHighlightForegroundColor => Self::parse_reset_dynamic_color_number(
                p1str.parse::<u8>().unwrap().saturating_sub(100),
            ),

            SetTextForegroundColor
            | SetTextBackgroundColor
            | SetTextCursorColor
            | SetMouseForegroundColor
            | SetMouseBackgroundColor
            | SetTektronixForegroundColor
            | SetTektronixBackgroundColor
            | SetHighlightBackgroundColor
            | SetTektronixCursorColor
            | SetHighlightForegroundColor => {
                Self::parse_change_dynamic_color_number(p1str.parse::<u8>().unwrap(), osc)
            }

            osc_code => bail!("{:?} not impl", osc_code),
        }
    }
}

macro_rules! osc_entries {
($(
    $( #[doc=$doc:expr] )*
    $label:ident = $value:expr
),* $(,)?) => {

#[derive(Debug, Clone, PartialEq, Eq, FromPrimitive, Hash, Copy)]
pub enum OperatingSystemCommandCode {
    $(
        $( #[doc=$doc] )*
        $label,
    )*
}

impl OscMap {
    fn new() -> Self {
        let mut code_to_variant = HashMap::new();
        let mut variant_to_code = HashMap::new();

        use OperatingSystemCommandCode::*;

        $(
            code_to_variant.insert($value, $label);
            variant_to_code.insert($label, $value);
        )*

        Self {
            code_to_variant,
            variant_to_code,
        }
    }
}
    };
}

osc_entries!(
    SetIconNameAndWindowTitle = "0",
    SetIconName = "1",
    SetWindowTitle = "2",
    SetXWindowProperty = "3",
    ChangeColorNumber = "4",
    ChangeSpecialColorNumber = "5",
    /// iTerm2
    ChangeTitleTabColor = "6",
    SetCurrentWorkingDirectory = "7",
    /// iTerm2
    SystemNotification = "9",
    SetTextForegroundColor = "10",
    SetTextBackgroundColor = "11",
    SetTextCursorColor = "12",
    SetMouseForegroundColor = "13",
    SetMouseBackgroundColor = "14",
    SetTektronixForegroundColor = "15",
    SetTektronixBackgroundColor = "16",
    SetHighlightBackgroundColor = "17",
    SetTektronixCursorColor = "18",
    SetHighlightForegroundColor = "19",
    SetLogFileName = "46",
    SetFont = "50",
    EmacsShell = "51",
    ManipulateSelectionData = "52",
    ResetColors = "104",
    ResetSpecialColor = "105",
    ResetTextForegroundColor = "110",
    ResetTextBackgroundColor = "111",
    ResetTextCursorColor = "112",
    ResetMouseForegroundColor = "113",
    ResetMouseBackgroundColor = "114",
    ResetTektronixForegroundColor = "115",
    ResetTektronixBackgroundColor = "116",
    ResetHighlightColor = "117",
    ResetTektronixCursorColor = "118",
    ResetHighlightForegroundColor = "119",
    RxvtProprietary = "777",
    FinalTermSemanticPrompt = "133",
    /// Here the "Sun" suffix comes from the table in
    /// <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html#h3-Miscellaneous>
    /// that lays out various window related escape sequences.
    SetWindowTitleSun = "l",
    SetIconNameSun = "L",
);

struct OscMap {
    code_to_variant: HashMap<&'static str, OperatingSystemCommandCode>,
    variant_to_code: HashMap<OperatingSystemCommandCode, &'static str>,
}

lazy_static::lazy_static! {
    static ref OSC_MAP: OscMap = OscMap::new();
}

impl OperatingSystemCommandCode {
    fn from_code(code: &str) -> Option<Self> {
        OSC_MAP.code_to_variant.get(code).copied()
    }

    fn as_code(self) -> &'static str {
        OSC_MAP.variant_to_code.get(&self).unwrap()
    }
}

impl Display for OperatingSystemCommand {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        write!(f, "\x1b]")?;

        macro_rules! single_string {
            ($variant:ident, $s:expr) => {{
                let code = OperatingSystemCommandCode::$variant.as_code();
                match OperatingSystemCommandCode::$variant {
                    OperatingSystemCommandCode::SetWindowTitleSun
                    | OperatingSystemCommandCode::SetIconNameSun => {
                        // For the legacy sun terminals, the `l` and `L` OSCs are
                        // not separated by `;`.
                        write!(f, "{}{}", code, $s)?;
                    }
                    _ => {
                        // In the common case, the OSC is numeric and is separated
                        // from the rest of the string
                        write!(f, "{};{}", code, $s)?;
                    }
                }
            }};
        }

        use self::OperatingSystemCommand::*;
        match self {
            SetIconNameAndWindowTitle(title) => single_string!(SetIconNameAndWindowTitle, title),
            SetWindowTitle(title) => single_string!(SetWindowTitle, title),
            SetWindowTitleSun(title) => single_string!(SetWindowTitleSun, title),
            SetIconName(title) => single_string!(SetIconName, title),
            SetIconNameSun(title) => single_string!(SetIconNameSun, title),
            RxvtExtension(params) => write!(f, "777;{}", params.join(";"))?,
            Unspecified(v) => {
                for (idx, item) in v.iter().enumerate() {
                    if idx > 0 {
                        write!(f, ";")?;
                    }
                    f.write_str(&String::from_utf8_lossy(item))?;
                }
            }
            ClearSelection(s) => write!(f, "52;{}", s)?,
            QuerySelection(s) => write!(f, "52;{};?", s)?,
            SetSelection(s, val) => write!(f, "52;{};{}", s, base64_encode(val))?,
            SystemNotification(s) => write!(f, "9;{}", s)?,
            FinalTermSemanticPrompt(i) => i.fmt(f)?,
            ResetColors(colors) => {
                write!(f, "104")?;
                for c in colors {
                    write!(f, ";{}", c)?;
                }
            }
            ChangeColorNumber(specs) => {
                write!(f, "4;")?;
                for pair in specs {
                    write!(f, "{};{}", pair.palette_index, pair.color)?
                }
            }
            ChangeDynamicColors(first_color, colors) => {
                write!(f, "{}", *first_color as u8)?;
                for color in colors {
                    write!(f, ";{}", color)?
                }
            }
            ResetDynamicColor(color) => {
                write!(f, "{}", 100 + *color as u8)?;
            }
            CurrentWorkingDirectory(s) => write!(f, "7;{}", s)?,
        };
        // Use the longer form ST as neovim doesn't like the BEL version
        write!(f, "\x1b\\")?;
        Ok(())
    }
}

/// https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalTermClick {
    /// Allow motion only within the single input line using left/right arrow keys
    Line,
    /// Allow moving between multiple lines of input using left/right arrow keys
    MultipleLine,
    /// Allow left/right and conservative up/down arrow motion
    ConservativeVertical,
    /// Allow left/right and up/down motion, and the line editor ensures that
    /// there are no spurious trailing spaces at ends of lines and that vertical
    /// motion across shorter lines causes some horizontal cursor motion.
    SmartVertical,
}

impl std::convert::TryFrom<&str> for FinalTermClick {
    type Error = crate::Error;
    fn try_from(s: &str) -> Result<Self> {
        match s {
            "line" => Ok(Self::Line),
            "m" => Ok(Self::MultipleLine),
            "v" => Ok(Self::ConservativeVertical),
            "w" => Ok(Self::SmartVertical),
            _ => bail!("invalid FinalTermClick {}", s),
        }
    }
}

impl Display for FinalTermClick {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        match self {
            Self::Line => write!(f, "line"),
            Self::MultipleLine => write!(f, "m"),
            Self::ConservativeVertical => write!(f, "v"),
            Self::SmartVertical => write!(f, "w"),
        }
    }
}

/// https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalTermPromptKind {
    /// A normal left side primary prompt
    Initial,
    /// A right-aligned prompt
    RightSide,
    /// A continuation prompt for an input that can be edited
    Continuation,
    /// A continuation prompt where the input cannot be edited
    Secondary,
}

impl Default for FinalTermPromptKind {
    fn default() -> Self {
        Self::Initial
    }
}

impl std::convert::TryFrom<&str> for FinalTermPromptKind {
    type Error = crate::Error;
    fn try_from(s: &str) -> Result<Self> {
        match s {
            "i" => Ok(Self::Initial),
            "r" => Ok(Self::RightSide),
            "c" => Ok(Self::Continuation),
            "s" => Ok(Self::Secondary),
            _ => bail!("invalid FinalTermPromptKind {}", s),
        }
    }
}

impl Display for FinalTermPromptKind {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        match self {
            Self::Initial => write!(f, "i"),
            Self::RightSide => write!(f, "r"),
            Self::Continuation => write!(f, "c"),
            Self::Secondary => write!(f, "s"),
        }
    }
}

/// https://gitlab.freedesktop.org/Per_Bothner/specifications/blob/master/proposals/semantic-prompts.md
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalTermSemanticPrompt {
    /// Do a "fresh line"; if the cursor is at the left margin then
    /// do nothing, otherwise perform the equivalent of "\r\n"
    FreshLine,

    /// Do a "fresh line" as above and then place the terminal into
    /// prompt mode; the output between now and the next marker is
    /// considered part of the prompt.
    FreshLineAndStartPrompt {
        aid: Option<String>,
        cl: Option<FinalTermClick>,
    },

    /// Denote the end of a command output and then perform FreshLine
    MarkEndOfCommandWithFreshLine {
        aid: Option<String>,
        cl: Option<FinalTermClick>,
    },

    /// Start a prompt
    StartPrompt(FinalTermPromptKind),

    /// Mark the end of a prompt and the start of the user input.
    /// The terminal considers all subsequent output to be "user input"
    /// until the next semantic marker.
    MarkEndOfPromptAndStartOfInputUntilNextMarker,

    /// Mark the end of a prompt and the start of the user input.
    /// The terminal considers all subsequent output to be "user input"
    /// until the end of the line.
    MarkEndOfPromptAndStartOfInputUntilEndOfLine,

    MarkEndOfInputAndStartOfOutput {
        aid: Option<String>,
    },

    /// Indicates the result of the command
    CommandStatus {
        status: i32,
        aid: Option<String>,
    },
}

impl FinalTermSemanticPrompt {
    fn parse(osc: &[&[u8]]) -> Result<Self> {
        ensure!(osc.len() > 1, "not enough args");
        let param = String::from_utf8_lossy(osc[1]);

        macro_rules! single {
            ($variant:ident, $text:expr) => {
                if osc.len() == 2 && param == $text {
                    return Ok(FinalTermSemanticPrompt::$variant);
                }
            };
        }

        single!(FreshLine, "L");
        single!(MarkEndOfPromptAndStartOfInputUntilNextMarker, "B");
        single!(MarkEndOfPromptAndStartOfInputUntilEndOfLine, "I");

        let mut params = HashMap::new();
        use std::convert::TryInto;

        for s in osc.iter().skip(if param == "D" { 3 } else { 2 }) {
            if let Some(equal) = s.iter().position(|c| *c == b'=') {
                let key = &s[..equal];
                let value = &s[equal + 1..];
                params.insert(str::from_utf8(key)?, str::from_utf8(value)?);
            } else if !s.is_empty() {
                bail!("malformed FinalTermSemanticPrompt");
            }
        }

        if param == "A" {
            return Ok(Self::FreshLineAndStartPrompt {
                aid: params.get("aid").map(|&s| s.to_owned()),
                cl: match params.get("cl") {
                    Some(&cl) => Some(cl.try_into()?),
                    None => None,
                },
            });
        }

        if param == "C" {
            return Ok(Self::MarkEndOfInputAndStartOfOutput {
                aid: params.get("aid").map(|&s| s.to_owned()),
            });
        }

        if param == "D" {
            let status = match osc.get(2).map(|&p| p) {
                Some(s) => match str::from_utf8(s) {
                    Ok(s) => s.parse().unwrap_or(0),
                    _ => 0,
                },
                _ => 0,
            };

            return Ok(Self::CommandStatus {
                status,
                aid: params.get("aid").map(|&s| s.to_owned()),
            });
        }

        if param == "N" {
            return Ok(Self::MarkEndOfCommandWithFreshLine {
                aid: params.get("aid").map(|&s| s.to_owned()),
                cl: match params.get("cl") {
                    Some(&cl) => Some(cl.try_into()?),
                    None => None,
                },
            });
        }

        if param == "P" {
            return Ok(Self::StartPrompt(match params.get("k") {
                Some(&cl) => cl.try_into()?,
                None => FinalTermPromptKind::default(),
            }));
        }

        bail!(
            "invalid FinalTermSemanticPrompt p1:{:?}, params:{:?}",
            param,
            params
        );
    }
}

impl Display for FinalTermSemanticPrompt {
    fn fmt(&self, f: &mut Formatter) -> FmtResult {
        write!(f, "133;")?;
        match self {
            Self::FreshLine => write!(f, "L")?,
            Self::FreshLineAndStartPrompt { aid, cl } => {
                write!(f, "A")?;
                if let Some(aid) = aid {
                    write!(f, ";aid={}", aid)?;
                }
                if let Some(cl) = cl {
                    write!(f, ";cl={}", cl)?;
                }
            }
            Self::MarkEndOfCommandWithFreshLine { aid, cl } => {
                write!(f, "N")?;
                if let Some(aid) = aid {
                    write!(f, ";aid={}", aid)?;
                }
                if let Some(cl) = cl {
                    write!(f, ";cl={}", cl)?;
                }
            }
            Self::StartPrompt(kind) => {
                write!(f, "P;k={}", kind)?;
            }
            Self::MarkEndOfPromptAndStartOfInputUntilNextMarker => write!(f, "B")?,
            Self::MarkEndOfPromptAndStartOfInputUntilEndOfLine => write!(f, "I")?,
            Self::MarkEndOfInputAndStartOfOutput { aid } => {
                write!(f, "C")?;
                if let Some(aid) = aid {
                    write!(f, ";aid={}", aid)?;
                }
            }
            Self::CommandStatus {
                status,
                aid: Some(aid),
            } => {
                write!(f, "D;{};err={};aid={}", status, status, aid)?;
            }
            Self::CommandStatus { status, aid: None } => {
                write!(f, "D;{}", status)?;
            }
        }
        Ok(())
    }
}

/// base64::encode is deprecated, so make a less frustrating helper
pub(crate) fn base64_encode<T: AsRef<[u8]>>(s: T) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

/// base64::decode is deprecated, so make a less frustrating helper
pub(crate) fn base64_decode<T: AsRef<[u8]>>(
    s: T,
) -> std::result::Result<Vec<u8>, base64::DecodeError> {
    use base64::engine::{GeneralPurpose, GeneralPurposeConfig};
    GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
    )
    .decode(s)
}
