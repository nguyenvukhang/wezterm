use crate::terminal::Alert;
use crate::terminalstate::{default_color_map, CharSet, MouseEncoding, TabStop};
use crate::{Position, TerminalState, VisibleRowIndex, DCS, ST};
use finl_unicode::grapheme_clusters::Graphemes;
use log::{debug, error};
use std::io::Write as _;
use std::ops::{Deref, DerefMut};
use termwiz::cell::{grapheme_column_width, Cell, CellAttributes};
use termwiz::escape::csi::{
    CharacterPath, EraseInDisplay, Keyboard, KittyKeyboardFlags, KittyKeyboardMode,
};
use termwiz::escape::{Action, ControlCode, DeviceControlMode, Esc, EscCode, CSI};
use termwiz::input::KeyboardEncoding;
use unicode_normalization::{is_nfc_quick, IsNormalized, UnicodeNormalization};
use wezterm_bidi::ParagraphDirectionHint;

/// A helper struct for implementing `vtparse::VTActor` while compartmentalizing
/// the terminal state and the embedding/host terminal interface
pub(crate) struct Performer<'a> {
    pub state: &'a mut TerminalState,
    print: String,
}

impl<'a> Deref for Performer<'a> {
    type Target = TerminalState;

    fn deref(&self) -> &TerminalState {
        self.state
    }
}

impl<'a> DerefMut for Performer<'a> {
    fn deref_mut(&mut self) -> &mut TerminalState {
        &mut self.state
    }
}

impl<'a> Drop for Performer<'a> {
    fn drop(&mut self) {
        self.flush_print();
    }
}

impl<'a> Performer<'a> {
    pub fn new(state: &'a mut TerminalState) -> Self {
        Self {
            state,
            print: String::new(),
        }
    }

    /// Apply character set related remapping to the input glyph if required
    fn remap_grapheme<'b>(&self, g: &'b str) -> &'b str {
        if (self.shift_out && self.g1_charset == CharSet::DecLineDrawing)
            || (!self.shift_out && self.g0_charset == CharSet::DecLineDrawing)
        {
            match g {
                "`" => "◆",
                "a" => "▒",
                "b" => "␉",
                "c" => "␌",
                "d" => "␍",
                "e" => "␊",
                "f" => "°",
                "g" => "±",
                "h" => "␤",
                "i" => "␋",
                "j" => "┘",
                "k" => "┐",
                "l" => "┌",
                "m" => "└",
                "n" => "┼",
                "o" => "⎺",
                "p" => "⎻",
                "q" => "─",
                "r" => "⎼",
                "s" => "⎽",
                "t" => "├",
                "u" => "┤",
                "v" => "┴",
                "w" => "┬",
                "x" => "│",
                "y" => "≤",
                "z" => "≥",
                "{" => "π",
                "|" => "≠",
                "}" => "£",
                "~" => "·",
                _ => g,
            }
        } else if (self.shift_out && self.g1_charset == CharSet::Uk)
            || (!self.shift_out && self.g0_charset == CharSet::Uk)
        {
            match g {
                "#" => "£",
                _ => g,
            }
        } else {
            g
        }
    }

    fn flush_print(&mut self) {
        if self.print.is_empty() {
            return;
        }

        let seqno = self.seqno;
        let mut p = std::mem::take(&mut self.print);
        let normalized: String;
        let text = if self.config.normalize_output_to_unicode_nfc()
            && is_nfc_quick(p.chars()) != IsNormalized::Yes
        {
            normalized = p.as_str().nfc().collect();
            normalized.as_str()
        } else {
            p.as_str()
        };

        for g in Graphemes::new(text) {
            let g = self.remap_grapheme(g);

            let print_width = grapheme_column_width(g, Some(self.unicode_version));
            if print_width == 0 {
                // We got a zero-width grapheme.
                // We used to force them into a cell to guarantee that we
                // preserved them in the model, but it introduces presentation
                // problems, such as <https://github.com/wez/wezterm/issues/1422>
                log::trace!("Eliding zero-width grapheme {:?}", g);
                continue;
            }

            if self.wrap_next {
                // Since we're implicitly moving the cursor to the next
                // line, we need to tag the current position as wrapped
                // so that we can correctly reflow it if the window is
                // resized.
                {
                    let y = self.cursor.y;
                    let is_conpty = self.state.enable_conpty_quirks;
                    let is_alt = self.state.screen.alt_screen_is_active;
                    let screen = self.screen_mut();
                    let y = screen.phys_row(y);

                    fn makes_sense_to_wrap(s: &str) -> bool {
                        let len = s.len();
                        match (len, s.chars().next()) {
                            (1, Some(c)) => c.is_alphanumeric() || c.is_ascii_punctuation(),
                            _ => true,
                        }
                    }

                    let should_mark_wrapped = !is_alt
                        && (!is_conpty
                            || screen
                                .line_mut(y)
                                .visible_cells()
                                .last()
                                .map(|cell| makes_sense_to_wrap(cell.str()))
                                .unwrap_or(false));
                    if should_mark_wrapped {
                        screen.line_mut(y).set_last_cell_was_wrapped(true, seqno);
                    }
                }
                self.new_line(true);
            }

            let x = self.cursor.x;
            let y = self.cursor.y;
            let width = self.left_and_right_margins.end;

            let pen = self.pen.clone();

            let wrappable = x + print_width >= width;

            if self.insert {
                let margin = self.left_and_right_margins.end;
                let screen = self.screen_mut();
                for _ in x..x + print_width as usize {
                    screen.insert_cell(x, y, margin, seqno);
                }
            }

            // Assign the cell
            log::trace!(
                "print x={} y={} print_width={} width={} cell={} {:?}",
                x,
                y,
                print_width,
                width,
                g,
                self.pen
            );
            self.screen_mut()
                .set_cell_grapheme(x, y, g, print_width, pen, seqno);

            if !wrappable {
                self.cursor.x += print_width;
                self.wrap_next = false;
            } else {
                self.wrap_next = self.dec_auto_wrap;
            }
        }

        std::mem::swap(&mut self.print, &mut p);
        self.print.clear();
    }

    pub fn perform(&mut self, action: Action) {
        debug!("perform {:?}", action);
        match action {
            Action::Print(c) => self.print(c),
            Action::PrintString(s) => {
                for c in s.chars() {
                    self.print(c)
                }
            }
            Action::Control(code) => self.control(code),
            Action::DeviceControl(ctrl) => self.device_control(ctrl),
            Action::Esc(esc) => self.esc_dispatch(esc),
            Action::CSI(csi) => self.csi_dispatch(csi),
            Action::XtGetTcap(names) => self.xt_get_tcap(names),
        }
    }

    fn device_control(&mut self, ctrl: DeviceControlMode) {
        match &ctrl {
            DeviceControlMode::ShortDeviceControl(s) => {
                match (s.byte, s.intermediates.as_slice()) {
                    (b'q', &[b'$']) => {
                        // DECRQSS - Request Status String
                        // https://vt100.net/docs/vt510-rm/DECRQSS.html
                        // The response is described here:
                        // https://vt100.net/docs/vt510-rm/DECRPSS.html
                        // but note that *that* text has the validity value
                        // inverted; there's a note about this in the xterm
                        // ctlseqs docs.
                        match s.data.as_slice() {
                            &[b'"', b'p'] => {
                                // DECSCL - select conformance level
                                write!(self.writer, "{}1$r65;1\"p{}", DCS, ST).ok();
                                self.writer.flush().ok();
                            }
                            &[b'r'] => {
                                // DECSTBM - top and bottom margins
                                let margins = self.top_and_bottom_margins.clone();
                                write!(
                                    self.writer,
                                    "{}1$r{};{}r{}",
                                    DCS,
                                    margins.start + 1,
                                    margins.end,
                                    ST
                                )
                                .ok();
                                self.writer.flush().ok();
                            }
                            &[b's'] => {
                                // DECSLRM - left and right margins
                                let margins = self.left_and_right_margins.clone();
                                write!(
                                    self.writer,
                                    "{}1$r{};{}s{}",
                                    DCS,
                                    margins.start + 1,
                                    margins.end,
                                    ST
                                )
                                .ok();
                                self.writer.flush().ok();
                            }
                            _ => {
                                if self.config.log_unknown_escape_sequences() {
                                    log::warn!("unhandled DECRQSS {:?}", s);
                                }
                                // Reply that the request is invalid
                                write!(self.writer, "{}0$r{}", DCS, ST).ok();
                                self.writer.flush().ok();
                            }
                        }
                    }
                    _ => {
                        if self.config.log_unknown_escape_sequences() {
                            log::warn!("unhandled {:?}", s);
                        }
                    }
                }
            }
            _ => match self.device_control_handler.as_mut() {
                Some(handler) => handler.handle_device_control(ctrl),
                None => {
                    if self.config.log_unknown_escape_sequences() {
                        log::warn!("unhandled {:?}", ctrl);
                    }
                }
            },
        }
    }

    /// Draw a character to the screen
    fn print(&mut self, c: char) {
        // We buffer up the chars to increase the chances of correctly grouping graphemes into cells
        if let Some(title) = self.accumulating_title.as_mut() {
            title.push(c);
        } else {
            self.print.push(c);
        }
    }

    fn control(&mut self, control: ControlCode) {
        let seqno = self.seqno;
        self.flush_print();
        match control {
            ControlCode::LineFeed | ControlCode::VerticalTab | ControlCode::FormFeed => {
                if self.left_and_right_margins.contains(&self.cursor.x) {
                    self.new_line(false);
                } else {
                    // Do move down, but don't trigger a scroll when we're
                    // outside of the left/right margins
                    let old_y = self.cursor.y;
                    let y = if old_y == self.top_and_bottom_margins.end - 1 {
                        old_y
                    } else {
                        (old_y + 1).min(self.screen().physical_rows as i64 - 1)
                    };
                    self.screen_mut().dirty_line(old_y, seqno);
                    self.screen_mut().dirty_line(y, seqno);
                    self.cursor.y = y;
                    self.wrap_next = false;
                }
                if self.newline_mode {
                    self.cursor.x = 0;
                    self.clear_semantic_attribute_due_to_movement();
                }
            }
            ControlCode::CarriageReturn => {
                if self.cursor.x >= self.left_and_right_margins.start {
                    self.cursor.x = self.left_and_right_margins.start;
                } else {
                    self.cursor.x = 0;
                }
                let y = self.cursor.y;
                self.wrap_next = false;
                self.clear_semantic_attribute_due_to_movement();
                self.screen_mut().dirty_line(y, seqno);
            }

            ControlCode::Backspace => {
                if self.reverse_wraparound_mode
                    && self.dec_auto_wrap
                    && self.cursor.x == self.left_and_right_margins.start
                    && self.cursor.y == self.top_and_bottom_margins.start
                {
                    // Backspace off the top-left wraps around to the bottom right
                    let x_pos = Position::Absolute(self.left_and_right_margins.end as i64 - 1);
                    let y_pos = Position::Absolute(self.top_and_bottom_margins.end - 1);
                    self.set_cursor_pos(&x_pos, &y_pos);
                } else if self.reverse_wraparound_mode
                    && self.dec_auto_wrap
                    && self.cursor.x <= self.left_and_right_margins.start
                {
                    // Backspace off the left wraps around to the prior line on the right
                    let x_pos = Position::Absolute(self.left_and_right_margins.end as i64 - 1);
                    let y_pos = Position::Relative(-1);
                    self.set_cursor_pos(&x_pos, &y_pos);
                } else if self.reverse_wraparound_mode
                    && self.dec_auto_wrap
                    && self.cursor.x == self.left_and_right_margins.end - 1
                    && self.wrap_next
                {
                    // If the cursor is in the last column and a character was
                    // just output and reverse-wraparound is on then backspace
                    // by 1 cancels the pending wrap.
                    self.wrap_next = false;
                } else if self.cursor.x == self.left_and_right_margins.start {
                    // Respect the left margin and don't BS outside it
                } else {
                    self.set_cursor_pos(&Position::Relative(-1), &Position::Relative(0));
                }
            }
            ControlCode::HorizontalTab => self.c0_horizontal_tab(),
            ControlCode::HTS => self.c1_hts(),
            ControlCode::IND => self.c1_index(),
            ControlCode::NEL => self.c1_nel(),
            ControlCode::Bell => {
                if let Some(handler) = self.alert_handler.as_mut() {
                    handler.alert(Alert::Bell);
                } else {
                    log::info!("Ding! (this is the bell)");
                }
            }
            ControlCode::RI => self.c1_reverse_index(),

            // wezterm only supports UTF-8, so does not support the
            // DEC National Replacement Character Sets.  However, it does
            // support the DEC Special Graphics character set used by
            // numerous ncurses applications.  DEC Special Graphics can be
            // selected by ASCII Shift Out (0x0E, ^N) or by setting G0
            // via ESC ( 0 .
            ControlCode::ShiftIn => {
                self.shift_out = false;
            }
            ControlCode::ShiftOut => {
                self.shift_out = true;
            }

            ControlCode::Enquiry => {
                let response = self.config.enq_answerback();
                if response.len() > 0 {
                    write!(self.writer, "{}", response).ok();
                    self.writer.flush().ok();
                }
            }

            ControlCode::Null => {}

            _ => {
                if self.config.log_unknown_escape_sequences() {
                    log::warn!("unhandled ControlCode {:?}", control);
                }
            }
        }
    }

    fn csi_dispatch(&mut self, csi: CSI) {
        self.flush_print();
        match csi {
            CSI::Sgr(sgr) => self.state.perform_csi_sgr(sgr),
            CSI::Cursor(termwiz::escape::csi::Cursor::Left(n)) => {
                // We treat CUB (Cursor::Left) the same as Backspace as
                // that is what xterm does.
                // <https://github.com/wez/wezterm/issues/1273>
                for _ in 0..n {
                    self.control(ControlCode::Backspace);
                }
            }
            CSI::Cursor(cursor) => self.state.perform_csi_cursor(cursor),
            CSI::Edit(edit) => self.state.perform_csi_edit(edit),
            CSI::Mode(mode) => self.state.perform_csi_mode(mode),
            CSI::Device(dev) => self.state.perform_device(*dev),
            CSI::Mouse(mouse) => error!("mouse report sent by app? {:?}", mouse),
            CSI::Window(window) => self.state.perform_csi_window(*window),
            CSI::SelectCharacterPath(CharacterPath::ImplementationDefault, _) => {
                self.state.bidi_hint.take();
            }
            CSI::SelectCharacterPath(CharacterPath::LeftToRightOrTopToBottom, _) => {
                self.state
                    .bidi_hint
                    .replace(ParagraphDirectionHint::LeftToRight);
            }
            CSI::SelectCharacterPath(CharacterPath::RightToLeftOrBottomToTop, _) => {
                self.state
                    .bidi_hint
                    .replace(ParagraphDirectionHint::RightToLeft);
            }
            CSI::Keyboard(Keyboard::SetKittyState { flags, mode }) => {
                if self.config.enable_kitty_keyboard() {
                    let current_flags = match self.screen().keyboard_stack.last() {
                        Some(KeyboardEncoding::Kitty(flags)) => *flags,
                        _ => KittyKeyboardFlags::NONE,
                    };
                    let flags = match mode {
                        KittyKeyboardMode::AssignAll => flags,
                        KittyKeyboardMode::SetSpecified => current_flags | flags,
                        KittyKeyboardMode::ClearSpecified => current_flags - flags,
                    };
                    self.screen_mut().keyboard_stack.pop();
                    self.screen_mut()
                        .keyboard_stack
                        .push(KeyboardEncoding::Kitty(flags));
                }
            }
            CSI::Keyboard(Keyboard::PushKittyState { flags, mode }) => {
                if self.config.enable_kitty_keyboard() {
                    let current_flags = match self.screen().keyboard_stack.last() {
                        Some(KeyboardEncoding::Kitty(flags)) => *flags,
                        _ => KittyKeyboardFlags::NONE,
                    };
                    let flags = match mode {
                        KittyKeyboardMode::AssignAll => flags,
                        KittyKeyboardMode::SetSpecified => current_flags | flags,
                        KittyKeyboardMode::ClearSpecified => current_flags - flags,
                    };
                    let screen = self.screen_mut();
                    screen.keyboard_stack.push(KeyboardEncoding::Kitty(flags));
                    if screen.keyboard_stack.len() > 128 {
                        screen.keyboard_stack.remove(0);
                    }
                }
            }
            CSI::Keyboard(Keyboard::PopKittyState(n)) => {
                for _ in 0..n {
                    self.screen_mut().keyboard_stack.pop();
                }
            }
            CSI::Keyboard(Keyboard::QueryKittySupport) => {
                if self.config.enable_kitty_keyboard() {
                    let flags = match self.screen().keyboard_stack.last() {
                        Some(KeyboardEncoding::Kitty(flags)) => *flags,
                        _ => KittyKeyboardFlags::NONE,
                    };
                    write!(self.writer, "\x1b[?{}u", flags.bits()).ok();
                    self.writer.flush().ok();
                }
            }
            CSI::Keyboard(Keyboard::ReportKittyState(_)) => {
                // This is a response to QueryKittySupport and it is invalid for us
                // to receive it. Just ignore it.
            }
            CSI::Unspecified(unspec) => {
                if self.config.log_unknown_escape_sequences() {
                    log::warn!("unknown unspecified CSI: {:?}", format!("{}", unspec));
                }
            }
        };
    }

    fn esc_dispatch(&mut self, esc: Esc) {
        let seqno = self.seqno;
        self.flush_print();
        match esc {
            Esc::Code(EscCode::StringTerminator) => {}
            Esc::Code(EscCode::TmuxTitle) => {
                self.accumulating_title.replace(String::new());
            }
            Esc::Code(EscCode::DecApplicationKeyPad) => {
                debug!("DECKPAM on");
                self.application_keypad = true;
            }
            Esc::Code(EscCode::DecNormalKeyPad) => {
                debug!("DECKPAM off");
                self.application_keypad = false;
            }
            Esc::Code(EscCode::ReverseIndex) => self.c1_reverse_index(),
            Esc::Code(EscCode::Index) => self.c1_index(),
            Esc::Code(EscCode::NextLine) => self.c1_nel(),
            Esc::Code(EscCode::HorizontalTabSet) => self.c1_hts(),
            Esc::Code(EscCode::DecLineDrawingG0) => {
                self.g0_charset = CharSet::DecLineDrawing;
            }
            Esc::Code(EscCode::AsciiCharacterSetG0) => {
                self.g0_charset = CharSet::Ascii;
            }
            Esc::Code(EscCode::UkCharacterSetG0) => {
                self.g0_charset = CharSet::Uk;
            }
            Esc::Code(EscCode::DecLineDrawingG1) => {
                self.g1_charset = CharSet::DecLineDrawing;
            }
            Esc::Code(EscCode::AsciiCharacterSetG1) => {
                self.g1_charset = CharSet::Ascii;
            }
            Esc::Code(EscCode::UkCharacterSetG1) => {
                self.g1_charset = CharSet::Uk;
            }
            Esc::Code(EscCode::DecSaveCursorPosition) => self.dec_save_cursor(),
            Esc::Code(EscCode::DecRestoreCursorPosition) => self.dec_restore_cursor(),

            Esc::Code(EscCode::DecDoubleHeightTopHalfLine) => {
                let idx = self.screen.phys_row(self.cursor.y);
                self.screen.line_mut(idx).set_double_height_top(seqno);
            }
            Esc::Code(EscCode::DecDoubleHeightBottomHalfLine) => {
                let idx = self.screen.phys_row(self.cursor.y);
                self.screen.line_mut(idx).set_double_height_bottom(seqno);
            }
            Esc::Code(EscCode::DecDoubleWidthLine) => {
                let idx = self.screen.phys_row(self.cursor.y);
                self.screen.line_mut(idx).set_double_width(seqno);
            }
            Esc::Code(EscCode::DecSingleWidthLine) => {
                let idx = self.screen.phys_row(self.cursor.y);
                self.screen.line_mut(idx).set_single_width(seqno);
            }

            Esc::Code(EscCode::DecScreenAlignmentDisplay) => {
                // This one is just to make vttest happy;
                // its original purpose was for aligning the CRT.
                // https://vt100.net/docs/vt510-rm/DECALN.html

                let screen = self.screen_mut();
                let col_range = 0..screen.physical_cols;
                for y in 0..screen.physical_rows as VisibleRowIndex {
                    let line_idx = screen.phys_row(y);
                    let line = screen.line_mut(line_idx);
                    line.resize(col_range.end, seqno);
                    line.fill_range(
                        col_range.clone(),
                        &Cell::new('E', CellAttributes::default()),
                        seqno,
                    );
                }

                self.top_and_bottom_margins = 0..self.screen().physical_rows as VisibleRowIndex;
                self.left_and_right_margins = 0..self.screen().physical_cols;
                self.cursor = Default::default();
            }

            // RIS resets a device to its initial state, i.e. the state it has after it is switched
            // on. This may imply, if applicable: remove tabulation stops, remove qualified areas,
            // reset graphic rendition, erase all positions, move active position to first
            // character position of first line.
            Esc::Code(EscCode::FullReset) => {
                let seqno = self.seqno;
                self.pen = Default::default();
                self.cursor = Default::default();
                self.wrap_next = false;
                self.clear_semantic_attribute_on_newline = false;
                self.insert = false;
                self.dec_auto_wrap = true;
                self.reverse_wraparound_mode = false;
                self.reverse_video_mode = false;
                self.dec_origin_mode = false;
                self.use_private_color_registers_for_each_graphic = false;
                self.color_map = default_color_map();
                self.application_cursor_keys = false;
                self.sixel_display_mode = false;
                self.dec_ansi_mode = false;
                self.application_keypad = false;
                self.bracketed_paste = false;
                self.focus_tracking = false;
                self.mouse_tracking = false;
                self.mouse_encoding = MouseEncoding::X10;
                self.keyboard_encoding = KeyboardEncoding::Xterm;
                self.sixel_scrolls_right = false;
                self.any_event_mouse = false;
                self.button_event_mouse = false;
                self.current_mouse_buttons.clear();
                self.cursor_visible = true;
                self.g0_charset = CharSet::Ascii;
                self.g1_charset = CharSet::Ascii;
                self.shift_out = false;
                self.newline_mode = false;
                self.tabs = TabStop::new(self.screen().physical_cols, 8);
                self.palette.take();
                self.top_and_bottom_margins = 0..self.screen().physical_rows as VisibleRowIndex;
                self.left_and_right_margins = 0..self.screen().physical_cols;
                self.unicode_version = self.config.unicode_version();
                self.suppress_initial_title_change = false;
                self.accumulating_title.take();

                self.screen.full_reset();
                self.screen.activate_alt_screen(seqno);
                self.erase_in_display(EraseInDisplay::EraseDisplay);
                self.screen.activate_primary_screen(seqno);
                self.erase_in_display(EraseInDisplay::EraseScrollback);
                self.erase_in_display(EraseInDisplay::EraseDisplay);
                if let Some(handler) = self.alert_handler.as_mut() {
                    handler.alert(Alert::PaletteChanged);
                }
            }

            _ => {
                if self.config.log_unknown_escape_sequences() {
                    log::warn!("ESC: unhandled {:?}", esc);
                }
            }
        }
    }
}
