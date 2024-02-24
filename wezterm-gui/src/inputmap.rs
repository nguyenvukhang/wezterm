use crate::commands::CommandDef;
use config::keyassignment::{
    ClipboardCopyDestination, ClipboardPasteSource, KeyAssignment, KeyTableEntry, KeyTables,
    MouseEventTrigger, SelectionMode,
};
use config::{ConfigHandle, MouseEventAltScreen, MouseEventTriggerMods};
use std::collections::HashMap;
use std::time::Duration;
use wezterm_term::input::MouseButton;
use window::{KeyCode, Modifiers};

pub struct InputMap {
    pub keys: KeyTables,
    pub mouse: HashMap<(MouseEventTrigger, MouseEventTriggerMods), KeyAssignment>,
    leader: Option<(KeyCode, Modifiers, Duration)>,
}

impl InputMap {
    pub fn new(config: &ConfigHandle) -> Self {
        let mut mouse = config.mouse_bindings();

        let mut keys = config.key_bindings();

        let leader = config.leader.as_ref().map(|leader| {
            (
                leader.key.key.resolve(config.key_map_preference).clone(),
                leader.key.mods,
                Duration::from_millis(leader.timeout_milliseconds),
            )
        });

        let ctrl_shift = Modifiers::CTRL | Modifiers::SHIFT;

        macro_rules! m {
            ($([$mod:expr, $code:expr, $action:expr]),* $(,)?) => {
                $(
                mouse.entry(($code, $mod)).or_insert($action);
                )*
            };
        }

        use KeyAssignment::*;

        if !config.disable_default_key_bindings {
            for (mods, code, action) in CommandDef::default_key_assignments(config) {
                // If the user configures {key='p', mods='CTRL|SHIFT'} that gets
                // normalized into {key='P', mods='CTRL'} in Config::key_bindings(),
                // and that value exists in `keys.default` when we reach this point.
                //
                // When we get here with the default assignments for ActivateCommandPalette
                // we are going to register un-normalized entries that don't match
                // the existing normalized entry.
                //
                // Ideally we'd unconditionally normalize_shift
                // here and register the result if it isn't already in the map.
                //
                // Our default set of assignments deliberately and explicitly emits
                // variations on SHIFT as a workaround for an issue with
                // normalization under X11: <https://github.com/wez/wezterm/issues/1906>.
                // Until that is resolved, we need to keep emitting both variants.
                //
                // In order for the DisableDefaultAssignment behavior to work with the
                // least surprises, and for these normalization related workarounds
                // to continue? to work, the approach we take here is to lookup the
                // normalized version of what we're about to register, and if we get
                // a match, skip this key.  Otherwise register the non-normalized
                // version from default_key_assignments().
                //
                // See: <https://github.com/wez/wezterm/issues/3262>
                let (disable_code, disable_mods) = code.normalize_shift(mods);
                if keys
                    .default
                    .contains_key(&(disable_code.clone(), disable_mods))
                {
                    continue;
                }
                keys.default
                    .entry((code, mods))
                    .or_insert(KeyTableEntry { action });
            }
        }

        if !config.disable_default_mouse_bindings {
            m!(
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::False,
                    },
                    MouseEventTrigger::Down {
                        streak: 1,
                        button: MouseButton::WheelUp(1),
                    },
                    ScrollByCurrentEventWheelDelta
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::False,
                    },
                    MouseEventTrigger::Down {
                        streak: 1,
                        button: MouseButton::WheelDown(1),
                    },
                    ScrollByCurrentEventWheelDelta
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Down {
                        streak: 3,
                        button: MouseButton::Left
                    },
                    SelectTextAtMouseCursor(SelectionMode::Line)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Down {
                        streak: 2,
                        button: MouseButton::Left
                    },
                    SelectTextAtMouseCursor(SelectionMode::Word)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Down {
                        streak: 1,
                        button: MouseButton::Left
                    },
                    SelectTextAtMouseCursor(SelectionMode::Cell)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::ALT,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Down {
                        streak: 1,
                        button: MouseButton::Left
                    },
                    SelectTextAtMouseCursor(SelectionMode::Block)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::SHIFT,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Down {
                        streak: 1,
                        button: MouseButton::Left
                    },
                    ExtendSelectionToMouseCursor(SelectionMode::Cell)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Up {
                        streak: 1,
                        button: MouseButton::Left
                    },
                    CompleteSelection(ClipboardCopyDestination::ClipboardAndPrimarySelection)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::ALT,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Up {
                        streak: 1,
                        button: MouseButton::Left
                    },
                    CompleteSelection(ClipboardCopyDestination::ClipboardAndPrimarySelection)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::ALT | Modifiers::SHIFT,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Down {
                        streak: 1,
                        button: MouseButton::Left
                    },
                    ExtendSelectionToMouseCursor(SelectionMode::Block)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Up {
                        streak: 2,
                        button: MouseButton::Left
                    },
                    CompleteSelection(ClipboardCopyDestination::ClipboardAndPrimarySelection)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Up {
                        streak: 3,
                        button: MouseButton::Left
                    },
                    CompleteSelection(ClipboardCopyDestination::ClipboardAndPrimarySelection)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Drag {
                        streak: 1,
                        button: MouseButton::Left
                    },
                    ExtendSelectionToMouseCursor(SelectionMode::Cell)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::ALT,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Drag {
                        streak: 1,
                        button: MouseButton::Left
                    },
                    ExtendSelectionToMouseCursor(SelectionMode::Block)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Drag {
                        streak: 2,
                        button: MouseButton::Left
                    },
                    ExtendSelectionToMouseCursor(SelectionMode::Word)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Drag {
                        streak: 3,
                        button: MouseButton::Left
                    },
                    ExtendSelectionToMouseCursor(SelectionMode::Line)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::NONE,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Down {
                        streak: 1,
                        button: MouseButton::Middle
                    },
                    PasteFrom(ClipboardPasteSource::PrimarySelection)
                ],
                [
                    MouseEventTriggerMods {
                        mods: Modifiers::SUPER,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Drag {
                        streak: 1,
                        button: MouseButton::Left,
                    },
                    StartWindowDrag
                ],
                [
                    MouseEventTriggerMods {
                        mods: ctrl_shift,
                        mouse_reporting: false,
                        alt_screen: MouseEventAltScreen::Any,
                    },
                    MouseEventTrigger::Drag {
                        streak: 1,
                        button: MouseButton::Left,
                    },
                    StartWindowDrag
                ],
            );
        }

        keys.default
            .retain(|_, v| v.action != KeyAssignment::DisableDefaultAssignment);

        mouse.retain(|_, v| *v != KeyAssignment::DisableDefaultAssignment);
        // Expand MouseEventAltScreen::Any to individual True/False entries
        let mut expanded_mouse = vec![];
        for ((code, mods), v) in &mouse {
            if mods.alt_screen == MouseEventAltScreen::Any {
                let mods_true = MouseEventTriggerMods {
                    alt_screen: MouseEventAltScreen::True,
                    ..*mods
                };
                let mods_false = MouseEventTriggerMods {
                    alt_screen: MouseEventAltScreen::False,
                    ..*mods
                };
                expanded_mouse.push((code.clone(), mods_true, v.clone()));
                expanded_mouse.push((code.clone(), mods_false, v.clone()));
            }
        }
        // Eliminate ::Any
        mouse.retain(|(_, mods), _| mods.alt_screen != MouseEventAltScreen::Any);
        for (code, mods, v) in expanded_mouse {
            mouse.insert((code, mods), v);
        }

        Self {
            keys,
            leader,
            mouse,
        }
    }

    /// Given an action, return the corresponding set of application-wide key assignments that are
    /// mapped to it.
    /// If any key_tables reference a given combination, then that combination
    /// is removed from the list.
    /// This is used to figure out whether an application-wide keyboard shortcut
    /// can be safely configured for this action, without interfering with any
    /// transient key_table mappings.
    #[allow(dead_code)]
    pub fn locate_app_wide_key_assignment(
        &self,
        action: &KeyAssignment,
    ) -> Vec<(KeyCode, Modifiers)> {
        let mut candidates = vec![];

        for ((key, mods), entry) in &self.keys.default {
            if mods.contains(Modifiers::LEADER) {
                continue;
            }
            if entry.action == *action {
                candidates.push((key.clone(), mods.clone()));
            }
        }

        // Now ensure that this combination is not part of a key table
        candidates.retain(|tuple| {
            for table in self.keys.by_name.values() {
                if table.contains_key(tuple) {
                    return false;
                }
            }
            true
        });

        candidates
    }

    pub fn is_leader(&self, key: &KeyCode, mods: Modifiers) -> Option<std::time::Duration> {
        if let Some((leader_key, leader_mods, timeout)) = self.leader.as_ref() {
            if *leader_key == *key && *leader_mods == mods.remove_positional_mods() {
                return Some(timeout.clone());
            }
        }
        None
    }

    pub fn has_table(&self, name: &str) -> bool {
        self.keys.by_name.contains_key(name)
    }

    pub fn lookup_key(
        &self,
        key: &KeyCode,
        mods: Modifiers,
        table_name: Option<&str>,
    ) -> Option<KeyTableEntry> {
        let table = match table_name {
            Some(name) => self.keys.by_name.get(name)?,
            None => &self.keys.default,
        };

        table
            .get(&key.normalize_shift(mods.remove_positional_mods()))
            .cloned()
    }

    pub fn lookup_mouse(
        &self,
        event: MouseEventTrigger,
        mut mods: MouseEventTriggerMods,
    ) -> Option<KeyAssignment> {
        mods.mods = mods.mods.remove_positional_mods();
        self.mouse.get(&(event, mods)).cloned()
    }
}
